//! Conexión ACP v1 con un agente lanzado en modo protocolo. Un solo hilo lógico: cada fase
//! envía su petición y lee mensajes hasta su respuesta, atendiendo por el camino las peticiones
//! y notificaciones del agente. Ver la documentación del módulo `acp`.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::time::{Instant, sleep_until, timeout_at};

use super::framing::{LineReader, ReadLine, write_message};
use super::jsonrpc::{
    self, AUTH_REQUIRED, Incoming, METHOD_NOT_FOUND, REQUEST_CANCELLED, RpcError,
};
use super::normalize::{Effect, Normalizer, Out, ascii_name, single_line};
use super::{
    AcpFailure, AcpLimits, AcpProfile, AcpStop, ISOLATION_NOTE, PROTOCOL_VERSION,
    claude_code_isolation_meta,
};
use crate::output::AgentKind;
use crate::process::{CancelToken, OutputSink, ProtocolIo};

/// Longitud máxima que se acepta de un texto de identidad del agente antes de validarlo (el
/// núcleo aplica después sus propios límites).
const MAX_IDENTITY_BYTES: usize = 512;
/// Longitud máxima de un `sessionId`.
const MAX_SESSION_ID_BYTES: usize = 512;
/// Caracteres del mensaje de error del agente que se copian a `sys`.
const MAX_ERROR_CHARS: usize = 200;

/// Lo que el agente declaró en `initialize`. Datos reportados, no verificados; sin `_meta`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDeclaration {
    pub protocol_version: u16,
    pub name: Option<String>,
    pub title: Option<String>,
    pub version: Option<String>,
    /// Claves conocidas (`loadSession`, `image`, `audio`, `embeddedContext`, `mcpHttp`,
    /// `mcpSse`) que el agente declaró como `true`.
    pub capabilities: Vec<&'static str>,
    /// Métodos de autenticación anunciados (solo cuántos: la app nunca autentica).
    pub auth_methods: usize,
}

/// Sesión abierta con el modo del perfil ya fijado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedSession {
    pub session_id: String,
    /// Modo fijado (el del perfil).
    pub mode: String,
    /// Las de `initialize` más `modes`.
    pub capabilities: Vec<&'static str>,
}

/// Por qué terminó una espera.
enum Wait {
    Cancel,
    Deadline,
    PhaseTimeout,
    Failed(AcpFailure),
}

/// Cliente ACP sobre las tuberías de un proceso.
pub struct AcpClient {
    stdin: Option<ChildStdin>,
    reader: LineReader<ChildStdout>,
    sink: OutputSink,
    limits: AcpLimits,
    /// Si está a `true` al cancelar (la app se cierra), se espera menos al agente.
    fast_cancel: Option<Arc<AtomicBool>>,
    next_id: u64,
    session_id: Option<String>,
    /// Modo que el agente no puede abandonar (tras fijarlo).
    required_mode: Option<String>,
    /// Se envió `session/cancel`: los permisos se responden `cancelled`.
    cancelling: bool,
    normalizer: Normalizer,
    /// Cancelación y plazo de la fase en curso: una escritura bloqueada compite con ellos.
    write_cancel: Option<CancelToken>,
    write_deadline: Option<Instant>,
}

impl std::fmt::Debug for AcpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AcpClient")
    }
}

impl AcpClient {
    pub fn new(io: ProtocolIo, limits: AcpLimits, fast_cancel: Option<Arc<AtomicBool>>) -> Self {
        Self {
            stdin: Some(io.stdin),
            reader: LineReader::new(io.stdout),
            sink: io.sink,
            limits,
            fast_cancel,
            next_id: 1,
            session_id: None,
            required_mode: None,
            cancelling: false,
            normalizer: Normalizer::default(),
            write_cancel: None,
            write_deadline: None,
        }
    }

    /// Escritura en el registro del intento (líneas `sys`).
    pub fn sink(&self) -> OutputSink {
        self.sink.clone()
    }

    /// Cierra el stdin del agente (fin del protocolo). Después, quien llama cierra el proceso.
    pub fn close_input(&mut self) {
        self.stdin = None;
    }

    /// `initialize`: versión 1, sin capacidades de cliente. Otra versión →
    /// [`AcpFailure::VersionUnsupported`].
    pub async fn initialize(
        &mut self,
        cancel: &mut CancelToken,
        deadline: Instant,
    ) -> Result<AgentDeclaration, AcpStop> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "clientCapabilities": {},
            "clientInfo": {
                "name": "kanbania",
                "title": "Kanban IA",
                "version": env!("CARGO_PKG_VERSION"),
            },
        });
        let result = match self
            .negotiate("initialize", params, cancel, deadline)
            .await?
        {
            Ok(result) => result,
            Err(error) => {
                self.note_rpc_error("initialize", &error).await;
                return Err(AcpStop::Failed(AcpFailure::RequestFailed));
            }
        };
        let Some(version) = result.get("protocolVersion").and_then(Value::as_u64) else {
            self.system("La respuesta a «initialize» no trae una versión de protocolo válida.")
                .await;
            return Err(AcpStop::Failed(AcpFailure::Protocol));
        };
        if version != u64::from(PROTOCOL_VERSION) {
            self.system(&format!(
                "El agente respondió con la versión {version} de ACP; la app solo habla la {PROTOCOL_VERSION}."
            ))
            .await;
            return Err(AcpStop::Failed(AcpFailure::VersionUnsupported));
        }
        let info = result.get("agentInfo");
        let text = |key: &str| {
            info.and_then(|i| i.get(key))
                .and_then(Value::as_str)
                .filter(|t| t.len() <= MAX_IDENTITY_BYTES)
                .map(str::to_owned)
        };
        let declaration = AgentDeclaration {
            protocol_version: PROTOCOL_VERSION,
            name: text("name"),
            title: text("title"),
            version: text("version"),
            capabilities: declared_capabilities(result.get("agentCapabilities")),
            auth_methods: result
                .get("authMethods")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
        };
        Ok(declaration)
    }

    /// `session/new` (cwd = worktree, sin servidores MCP) y `session/set_mode` al modo del
    /// perfil. Sin modos o sin el del perfil, o si fijarlo falla →
    /// [`AcpFailure::ModeUnavailable`]. Error al crear la sesión con métodos de autenticación
    /// anunciados (o con el código `-32000`) → [`AcpFailure::AuthRequired`].
    pub async fn open_session(
        &mut self,
        cwd: &Path,
        declared: &AgentDeclaration,
        profile: &AcpProfile,
        cancel: &mut CancelToken,
        deadline: Instant,
    ) -> Result<OpenedSession, AcpStop> {
        let Some(cwd) = cwd.to_str() else {
            self.system("La carpeta de trabajo no es texto Unicode: no se puede enviar por ACP.")
                .await;
            return Err(AcpStop::Failed(AcpFailure::RequestFailed));
        };
        let mut params = json!({ "cwd": cwd, "mcpServers": [] });
        if profile.isolate_claude_code_settings {
            params["_meta"] = claude_code_isolation_meta();
            self.system(ISOLATION_NOTE).await;
        }
        let result = match self
            .negotiate("session/new", params, cancel, deadline)
            .await?
        {
            Ok(result) => result,
            Err(error) => {
                self.note_rpc_error("session/new", &error).await;
                // Solo el código `-32000` (auth_required, schema v1) significa «inicia sesión»;
                // anunciar `authMethods` no basta.
                if error.code == AUTH_REQUIRED {
                    return Err(AcpStop::Failed(AcpFailure::AuthRequired));
                }
                if declared.auth_methods > 0 {
                    self.system(
                        "El agente anuncia métodos de autenticación, pero el error no es «autenticación requerida» (-32000).",
                    )
                    .await;
                }
                return Err(AcpStop::Failed(AcpFailure::RequestFailed));
            }
        };
        let Some(session_id) = result
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= MAX_SESSION_ID_BYTES)
            .map(str::to_owned)
        else {
            self.system("La respuesta a «session/new» no trae un sessionId válido.")
                .await;
            return Err(AcpStop::Failed(AcpFailure::Protocol));
        };
        self.session_id = Some(session_id.clone());

        let required = profile.required_mode;
        match result.get("modes") {
            None | Some(Value::Null) => {
                self.system(&format!(
                    "El agente no ofrece modos de sesión: no se puede fijar «{required}»."
                ))
                .await;
                return Err(AcpStop::Failed(AcpFailure::ModeUnavailable));
            }
            Some(modes) => {
                let Some(available) = modes.get("availableModes").and_then(Value::as_array) else {
                    self.system("Los modos de sesión del agente no tienen la forma de ACP v1.")
                        .await;
                    return Err(AcpStop::Failed(AcpFailure::Protocol));
                };
                let offered = available
                    .iter()
                    .any(|mode| mode.get("id").and_then(Value::as_str) == Some(required));
                if !offered {
                    self.system(&format!(
                        "El agente no ofrece el modo «{required}», que pide permiso antes de actuar."
                    ))
                    .await;
                    return Err(AcpStop::Failed(AcpFailure::ModeUnavailable));
                }
                if let Some(current) = modes.get("currentModeId").and_then(Value::as_str) {
                    self.system(&format!(
                        "Modo inicial que informa el agente: «{}». Se fija «{required}».",
                        ascii_name(current)
                    ))
                    .await;
                }
            }
        }
        let params = json!({ "sessionId": session_id, "modeId": required });
        match self
            .negotiate("session/set_mode", params, cancel, deadline)
            .await?
        {
            Ok(Value::Object(_) | Value::Null) => {}
            Ok(_) => {
                self.system("La respuesta a «session/set_mode» no tiene la forma de ACP v1.")
                    .await;
                return Err(AcpStop::Failed(AcpFailure::Protocol));
            }
            Err(error) => {
                self.note_rpc_error("session/set_mode", &error).await;
                return Err(AcpStop::Failed(AcpFailure::ModeUnavailable));
            }
        }
        self.required_mode = Some(required.to_owned());
        let mut capabilities = declared.capabilities.clone();
        capabilities.push("modes");
        Ok(OpenedSession {
            session_id,
            mode: required.to_owned(),
            capabilities,
        })
    }

    /// `session/prompt` con un único bloque `text`. `Ok` = `stopReason: end_turn`. La
    /// cancelación y el tiempo máximo envían `session/cancel` y esperan la respuesta un tiempo
    /// acotado; después devuelven [`AcpStop::Cancelled`] o [`AcpStop::TimedOut`] (quien llama
    /// mata el árbol).
    pub async fn prompt(
        &mut self,
        text: &str,
        cancel: &mut CancelToken,
        deadline: Instant,
    ) -> Result<(), AcpStop> {
        let Some(session_id) = self.session_id.clone() else {
            return Err(AcpStop::Failed(AcpFailure::Protocol));
        };
        let id = self.take_id();
        let params = json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": text }],
        });
        self.write_cancel = Some(cancel.clone());
        self.write_deadline = Some(deadline);
        if let Err(failure) = self
            .send(&jsonrpc::request(id, "session/prompt", params))
            .await
        {
            return Err(interrupted_write(cancel, deadline, failure));
        }
        let waited = self.await_response(id, None, deadline, Some(cancel)).await;
        let result = match waited {
            Ok(Ok(result)) => self.stop_reason(&result, false).await,
            Ok(Err(error)) => {
                self.note_rpc_error("session/prompt", &error).await;
                Err(AcpStop::Failed(if error.code == AUTH_REQUIRED {
                    AcpFailure::AuthRequired
                } else {
                    AcpFailure::TurnFailed
                }))
            }
            Err(Wait::Cancel) => Err(self.cancel_turn(id, &session_id, AcpStop::Cancelled).await),
            Err(Wait::Deadline) => Err(self.cancel_turn(id, &session_id, AcpStop::TimedOut).await),
            Err(Wait::PhaseTimeout) => Err(AcpStop::Failed(AcpFailure::RequestFailed)),
            Err(Wait::Failed(failure)) => Err(AcpStop::Failed(failure)),
        };
        let mut out = Vec::new();
        self.normalizer.flush(&mut out);
        self.emit(out).await;
        result
    }

    /// `session/cancel` y espera acotada de la respuesta a `session/prompt`. Devuelve `cause`.
    async fn cancel_turn(&mut self, id: u64, session_id: &str, cause: AcpStop) -> AcpStop {
        self.cancelling = true;
        // `session/cancel` se escribe aunque la cancelación ya esté pedida.
        self.write_cancel = None;
        self.write_deadline = None;
        let mut out = Vec::new();
        self.normalizer.flush(&mut out);
        self.emit(out).await;
        let notification =
            jsonrpc::notification("session/cancel", json!({ "sessionId": session_id }));
        if self.send(&notification).await.is_err() {
            self.system("No se pudo pedir la cancelación al agente: se termina el proceso.")
                .await;
            return cause;
        }
        let fast = self
            .fast_cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst));
        let wait = if fast {
            self.limits.shutdown_cancel_wait
        } else {
            self.limits.cancel_wait
        };
        self.system(&format!(
            "Se pidió al agente cancelar el turno (session/cancel); se espera su respuesta como mucho {} s.",
            wait.as_secs()
        ))
        .await;
        match self
            .await_response(id, None, Instant::now() + wait, None)
            .await
        {
            Ok(Ok(result)) => {
                let reason = result
                    .get("stopReason")
                    .and_then(Value::as_str)
                    .map_or_else(|| "(sin motivo)".to_owned(), ascii_name);
                self.system(&format!(
                    "El agente respondió a la cancelación (stopReason: {reason})."
                ))
                .await;
            }
            Ok(Err(error)) => {
                if error.code != REQUEST_CANCELLED {
                    self.note_rpc_error("session/prompt", &error).await;
                } else {
                    self.system("El agente confirmó la cancelación.").await;
                }
            }
            Err(Wait::Deadline | Wait::PhaseTimeout | Wait::Cancel) => {
                self.system(
                    "El agente no respondió a la cancelación a tiempo: se termina el proceso.",
                )
                .await;
            }
            Err(Wait::Failed(_)) => {
                self.system("La conexión con el agente terminó durante la cancelación.")
                    .await;
            }
        }
        cause
    }

    async fn stop_reason(&mut self, result: &Value, cancelled: bool) -> Result<(), AcpStop> {
        let reason = result.get("stopReason").and_then(Value::as_str);
        let failure = match reason {
            Some("end_turn") => return Ok(()),
            Some("cancelled") if cancelled => return Err(AcpStop::Cancelled),
            Some("refusal") => AcpFailure::Refused,
            Some("max_tokens" | "max_turn_requests") => AcpFailure::LimitReached,
            _ => AcpFailure::Protocol,
        };
        let reason = reason.map_or_else(|| "(ausente)".to_owned(), ascii_name);
        self.system(&format!(
            "El agente terminó el turno con stopReason: {reason}."
        ))
        .await;
        Err(AcpStop::Failed(failure))
    }

    /// Envía una petición de negociación y espera su respuesta (≤ `request_timeout`).
    async fn negotiate(
        &mut self,
        method: &str,
        params: Value,
        cancel: &mut CancelToken,
        deadline: Instant,
    ) -> Result<Result<Value, RpcError>, AcpStop> {
        if cancel.is_cancelled() {
            return Err(AcpStop::Cancelled);
        }
        let id = self.take_id();
        self.write_cancel = Some(cancel.clone());
        self.write_deadline = Some(deadline);
        self.send(&jsonrpc::request(id, method, params))
            .await
            .map_err(|failure| interrupted_write(cancel, deadline, failure))?;
        let phase = Instant::now() + self.limits.request_timeout;
        match self
            .await_response(id, Some(phase), deadline, Some(cancel))
            .await
        {
            Ok(result) => Ok(result),
            Err(Wait::Cancel) => Err(AcpStop::Cancelled),
            Err(Wait::Deadline) => Err(AcpStop::TimedOut),
            Err(Wait::PhaseTimeout) => {
                self.system(&format!(
                    "El agente no respondió a «{method}» en {} s.",
                    self.limits.request_timeout.as_secs()
                ))
                .await;
                Err(AcpStop::Failed(AcpFailure::RequestFailed))
            }
            Err(Wait::Failed(failure)) => Err(AcpStop::Failed(failure)),
        }
    }

    /// Lee mensajes hasta la respuesta a `id`, atendiendo peticiones y notificaciones.
    async fn await_response(
        &mut self,
        id: u64,
        phase: Option<Instant>,
        deadline: Instant,
        mut cancel: Option<&mut CancelToken>,
    ) -> Result<Result<Value, RpcError>, Wait> {
        loop {
            let line = {
                let cancelled = async {
                    match cancel.as_deref_mut() {
                        Some(token) => token.cancelled().await,
                        None => std::future::pending().await,
                    }
                };
                let phase_over = async {
                    match phase {
                        Some(at) => sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    biased;
                    () = cancelled => return Err(Wait::Cancel),
                    () = sleep_until(deadline) => return Err(Wait::Deadline),
                    () = phase_over => return Err(Wait::PhaseTimeout),
                    line = self.reader.next_line() => line,
                }
            };
            let line = match line {
                ReadLine::Line(line) => line,
                ReadLine::Eof => {
                    self.system("El agente cerró su salida (o terminó) antes de responder.")
                        .await;
                    return Err(Wait::Failed(AcpFailure::Disconnected));
                }
                ReadLine::TooLong => {
                    self.system("El agente envió una línea de más de 1 MiB: no es ACP válido.")
                        .await;
                    return Err(Wait::Failed(AcpFailure::Protocol));
                }
            };
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let Some(message) = jsonrpc::parse(&line) else {
                self.system("El agente envió una línea que no es un mensaje JSON-RPC 2.0 válido.")
                    .await;
                return Err(Wait::Failed(AcpFailure::Protocol));
            };
            match message {
                Incoming::Response { id: got, result } if got == id => return Ok(result),
                Incoming::Response { .. } => {
                    self.system("El agente respondió a una petición que la app no hizo.")
                        .await;
                    return Err(Wait::Failed(AcpFailure::Protocol));
                }
                Incoming::Request { id, method, params } => {
                    self.on_request(&id, &method, &params).await;
                }
                Incoming::Notification { method, params } => {
                    if let Some(failure) = self.on_notification(&method, &params).await {
                        return Err(Wait::Failed(failure));
                    }
                }
            }
        }
    }

    async fn on_request(&mut self, id: &Value, method: &str, params: &Value) {
        if method == "session/request_permission" {
            self.on_permission(id, params).await;
            return;
        }
        let name = ascii_name(method);
        let mut out = Vec::new();
        self.normalizer.note(
            format!("request:{name}"),
            format!(
                "El agente pidió «{name}», que la app no ofrece: se responde «método no disponible»."
            ),
            &mut out,
        );
        self.emit(out).await;
        let _ = self
            .send(&jsonrpc::error(id, METHOD_NOT_FOUND, "Method not found"))
            .await;
    }

    /// Política hasta HU-020: rechazar siempre con `reject_once`; sin esa opción, `cancelled`.
    /// Nunca `allow_*` ni `reject_always` (no se guardan reglas en el agente).
    async fn on_permission(&mut self, id: &Value, params: &Value) {
        let mut out = Vec::new();
        self.normalizer.flush(&mut out);
        let title = params
            .get("toolCall")
            .and_then(|t| t.get("title"))
            .and_then(Value::as_str)
            .map_or_else(|| "(sin título)".to_owned(), |t| single_line(t, 300));
        let reject_once = params
            .get("options")
            .and_then(Value::as_array)
            .and_then(|options| {
                options.iter().find_map(|option| {
                    (option.get("kind").and_then(Value::as_str) == Some("reject_once"))
                        .then(|| option.get("optionId").and_then(Value::as_str))
                        .flatten()
                })
            })
            .map(str::to_owned);
        let (response, line) = match (self.cancelling, reject_once) {
            (true, _) => (
                json!({ "outcome": { "outcome": "cancelled" } }),
                format!(
                    "Permiso pedido para «{title}»: respondido como cancelado (se estaba cancelando el turno)."
                ),
            ),
            (false, Some(option)) => (
                json!({ "outcome": { "outcome": "selected", "optionId": option } }),
                format!(
                    "Permiso pedido para «{title}»: rechazado una vez por la política de la app (hasta HU-020 se rechazan todos)."
                ),
            ),
            (false, None) => {
                out.push(Out::System(
                    "El agente pidió un permiso sin la opción «rechazar una vez»: se responde como cancelado."
                        .into(),
                ));
                (
                    json!({ "outcome": { "outcome": "cancelled" } }),
                    format!(
                        "Permiso pedido para «{title}»: sin opción «rechazar una vez», respondido como cancelado (política hasta HU-020)."
                    ),
                )
            }
        };
        out.push(Out::Agent(AgentKind::Permission, line));
        self.emit(out).await;
        let _ = self.send(&jsonrpc::result(id, response)).await;
    }

    /// Devuelve un fallo si la notificación obliga a detener el turno.
    async fn on_notification(&mut self, method: &str, params: &Value) -> Option<AcpFailure> {
        let mut out = Vec::new();
        let mut failure = None;
        if method == "session/update" {
            let session = params.get("sessionId").and_then(Value::as_str);
            let ours = match (&self.session_id, session) {
                (Some(current), Some(got)) => current == got,
                (None, _) => true,
                (Some(_), None) => false,
            };
            let mode_related = params
                .get("update")
                .and_then(|u| u.get("sessionUpdate"))
                .and_then(Value::as_str)
                .is_some_and(|kind| matches!(kind, "current_mode_update" | "config_option_update"));
            if self.session_id.is_some() && session.is_none() && mode_related {
                // `sessionId` es obligatorio: un cambio de modo sin él no se puede atribuir a
                // otra sesión. Falla cerrado.
                out.push(Out::System(
                    "El agente envió un cambio de modo sin sessionId: no es ACP válido.".into(),
                ));
                failure = Some(AcpFailure::Protocol);
            } else if !ours {
                self.normalizer.note(
                    "update:other-session".into(),
                    "Se ignoran actualizaciones de una sesión que no es la del intento.".into(),
                    &mut out,
                );
            } else if let Some(update) = params.get("update")
                && let Effect::ModeChanged(mode) = self.normalizer.update(update, &mut out)
                && let Some(required) = &self.required_mode
                && &mode != required
            {
                out.push(Out::System(format!(
                    "El agente cambió por su cuenta al modo «{mode}»: se detiene el turno (la política exige «{required}»)."
                )));
                failure = Some(AcpFailure::ModeUnavailable);
            }
        } else {
            let name = ascii_name(method);
            self.normalizer.note(
                format!("notification:{name}"),
                format!(
                    "Notificación del agente ignorada: «{name}» (no se registra su contenido)."
                ),
                &mut out,
            );
        }
        self.emit(out).await;
        failure
    }

    async fn note_rpc_error(&mut self, method: &str, error: &RpcError) {
        let message = single_line(&error.message, MAX_ERROR_CHARS);
        self.system(&format!(
            "El agente respondió con un error a «{method}» (código {}): {message}",
            error.code
        ))
        .await;
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    async fn send(&mut self, message: &Value) -> Result<(), AcpFailure> {
        let mut limit = Instant::now() + self.limits.write_timeout;
        if let Some(deadline) = self.write_deadline {
            limit = limit.min(deadline);
        }
        let mut cancel = self.write_cancel.clone();
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(AcpFailure::Disconnected);
        };
        let cancelled = async {
            match cancel.as_mut() {
                Some(token) => token.cancelled().await,
                None => std::future::pending().await,
            }
        };
        let written = tokio::select! {
            biased;
            () = cancelled => false,
            result = timeout_at(limit, write_message(stdin, message)) => matches!(result, Ok(Ok(()))),
        };
        if written {
            Ok(())
        } else {
            // Una escritura a medias deja el canal inservible.
            self.stdin = None;
            Err(AcpFailure::Disconnected)
        }
    }

    /// Línea `sys`, después de volcar el mensaje pendiente (orden de la salida).
    async fn system(&mut self, text: &str) {
        let mut out = Vec::new();
        self.normalizer.flush(&mut out);
        out.push(Out::System(text.to_owned()));
        self.emit(out).await;
    }

    async fn emit(&self, out: Vec<Out>) {
        for item in out {
            match item {
                Out::Agent(kind, text) => self.sink.agent(kind, &text).await,
                Out::System(text) => self.sink.system(&text).await,
            }
        }
    }
}

/// Una escritura interrumpida: por cancelación o plazo si ya se cumplieron; si no, el fallo.
fn interrupted_write(cancel: &CancelToken, deadline: Instant, failure: AcpFailure) -> AcpStop {
    if cancel.is_cancelled() {
        AcpStop::Cancelled
    } else if Instant::now() >= deadline {
        AcpStop::TimedOut
    } else {
        AcpStop::Failed(failure)
    }
}

/// Capacidades conocidas declaradas como `true` (sin `_meta` ni extensiones).
pub(crate) fn declared_capabilities(capabilities: Option<&Value>) -> Vec<&'static str> {
    let Some(capabilities) = capabilities else {
        return Vec::new();
    };
    let flag = |value: Option<&Value>| value.and_then(Value::as_bool).unwrap_or(false);
    let prompt = capabilities.get("promptCapabilities");
    let mcp = capabilities.get("mcpCapabilities");
    [
        ("loadSession", flag(capabilities.get("loadSession"))),
        ("image", flag(prompt.and_then(|p| p.get("image")))),
        ("audio", flag(prompt.and_then(|p| p.get("audio")))),
        (
            "embeddedContext",
            flag(prompt.and_then(|p| p.get("embeddedContext"))),
        ),
        ("mcpHttp", flag(mcp.and_then(|m| m.get("http")))),
        ("mcpSse", flag(mcp.and_then(|m| m.get("sse")))),
    ]
    .into_iter()
    .filter_map(|(name, on)| on.then_some(name))
    .collect()
}
