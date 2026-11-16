//! Agente ACP v1 **falso, solo para pruebas** de contrato (HU-019). No es un agente ni hace
//! trabajo: responde lo justo para ejercitar el cliente. Lo ejecuta el bin `kanbania-fake-acp`
//! (`kanbania-fake-acp <escenario> [--pids <archivo>] [--log <archivo>] [--prompt-file
//! <archivo>]`); la app nunca lo lanza. Con `--log` anota cada método recibido (una línea
//! por mensaje; `respuesta` para las respuestas) y sus comprobaciones (`ok:`/`mal:`); con
//! `--prompt-file`, guarda el texto exacto del prompt recibido.
//!
//! Comprueba lo que debe recibir y, si no es así, termina con [`WRONG_INPUT_EXIT`] (o escribe
//! en stderr) para que la prueba falle de forma visible: `cwd` absoluto y existente,
//! `mcpServers` vacío, el `_meta` que aísla la configuración de Claude Code
//! ([`kanbania_runner::acp::claude_code_isolation_meta`]), modo `default`, un bloque `text` no vacío, permisos respondidos con
//! `reject_once` (o `cancelled` sin esa opción) y `-32601` a peticiones que el cliente no
//! ofrece. Con una variable `KANBANIA_*`, `ANTHROPIC_*` o `CLAUDE_*` en su entorno termina con
//! [`FORBIDDEN_ENV_EXIT`].
//!
//! Escenarios: `ok`, `permission`, `disconnect`, `malformed`, `oversize`, `bad_response`,
//! `no_modes`, `version2`, `auth`, `set_mode_error`, `cancel`, `ignore_cancel`, `refusal`,
//! `max_tokens`, `cancelled_unasked`, `mode_change`, `config_mode_change`, `flood`, `child`,
//! `slow_init`, `hang_init`, `permission_on_cancel`, `unknown_id`, `other_session`,
//! `mode_no_session`, `auth_other_error`, `approval`, `approval_withdraw`.
//!
//! `approval` (HU-020) pide permiso para `touch nota.txt` y espera la respuesta tanto como
//! haga falta (también durante una cancelación); anota el resultado en `--log`
//! (`permiso: <optionId>` o `permiso: cancelled`) y, solo si se permitió, crea `nota.txt` en
//! su carpeta (el efecto de la acción). `approval_withdraw` pide permiso y lo retira en seguida
//! con `$/cancel_request`; espera el error -32800.

use std::io::{BufRead, Write};
use std::time::Duration;

use serde_json::{Value, json};

/// Correo ficticio que envía `_auth/status_update` (como el adaptador real): nunca debe
/// aparecer en la salida del intento.
const FAKE_EMAIL: &str = "persona.ficticia@example.invalid";
// Textos que nunca deben llegar a la salida (pensamiento, entrada y contenido de herramientas,
// `_meta`): PENSAMIENTO-SECRETO, ENTRADA-SECRETA, CONTENIDO-SECRETO, META-SECRETO (la prueba
// `tests/acp.rs` los busca; si cambian aquí, cámbialos allí).
/// Identificador de la sesión falsa.
const SESSION_ID: &str = "sesion-falsa-1";
/// Salida cuando el cliente envió algo inesperado.
const WRONG_INPUT_EXIT: i32 = 7;
/// Salida cuando el entorno trae una variable prohibida.
const FORBIDDEN_ENV_EXIT: i32 = 8;
/// Argumento interno del hijo del escenario `child`.
const SLEEP_CHILD: &str = "sleep-child";

const USAGE: &str = "uso: kanbania-fake-acp <escenario> [--pids <archivo>] [--log <archivo>] [--prompt-file <archivo>]";
const FORBIDDEN_PREFIXES: [&str; 3] = ["KANBANIA_", "ANTHROPIC_", "CLAUDE_"];

/// Ejecuta el escenario y devuelve el código de salida.
pub fn run(args: &[String]) -> i32 {
    let Some((scenario, rest)) = args.split_first() else {
        eprintln!("{USAGE}");
        return 2;
    };
    if scenario == SLEEP_CHILD {
        std::thread::sleep(Duration::from_secs(600));
        return 0;
    }
    if let Some((name, _)) = std::env::vars_os().find(|(name, _)| {
        let name = name.to_string_lossy().to_ascii_uppercase();
        FORBIDDEN_PREFIXES.iter().any(|p| name.starts_with(p))
    }) {
        eprintln!(
            "FAKE: variable prohibida en el entorno: {}",
            name.to_string_lossy()
        );
        return FORBIDDEN_ENV_EXIT;
    }
    let option = |name: &str| {
        rest.windows(2)
            .find(|pair| pair[0] == name)
            .map(|pair| pair[1].clone())
    };
    let mut fake = Fake {
        scenario: scenario.clone(),
        lines: std::io::stdin().lock().lines(),
        pids: option("--pids"),
        log: option("--log"),
        prompt_file: option("--prompt-file"),
    };
    fake.serve()
}

struct Fake {
    scenario: String,
    lines: std::io::Lines<std::io::StdinLock<'static>>,
    pids: Option<String>,
    log: Option<String>,
    prompt_file: Option<String>,
}

impl Fake {
    fn is(&self, scenario: &str) -> bool {
        self.scenario == scenario
    }

    fn serve(&mut self) -> i32 {
        loop {
            let Some(message) = self.recv() else {
                return self.on_eof();
            };
            let method = message.get("method").and_then(Value::as_str);
            let id = message.get("id").cloned();
            let exit = match (method, id) {
                (Some("initialize"), Some(id)) => self.initialize(&id),
                (Some("session/new"), Some(id)) => self.session_new(&id, &message),
                (Some("session/set_mode"), Some(id)) => self.set_mode(&id, &message),
                (Some("session/prompt"), Some(id)) => self.prompt(&id, &message),
                (Some(_), Some(id)) => {
                    send(
                        &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}}),
                    );
                    None
                }
                _ => None,
            };
            if let Some(code) = exit {
                return code;
            }
        }
    }

    fn on_eof(&self) -> i32 {
        if self.is("ignore_cancel") {
            // No responde nunca: la app debe terminar el árbol.
            std::thread::sleep(Duration::from_secs(600));
        }
        0
    }

    /// Añade una línea al registro de `--log` (sin él, nada).
    fn note(&self, text: &str) {
        if let Some(path) = &self.log
            && let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
        {
            let _ = writeln!(file, "{text}");
        }
    }

    fn recv(&mut self) -> Option<Value> {
        loop {
            let line = self.lines.next()?.ok()?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(&line) {
                Ok(value) => {
                    self.note(value["method"].as_str().unwrap_or("respuesta"));
                    return Some(value);
                }
                Err(_) => {
                    eprintln!("FAKE: el cliente envió una línea que no es JSON");
                    return None;
                }
            }
        }
    }

    fn initialize(&mut self, id: &Value) -> Option<i32> {
        if self.is("hang_init") {
            // No responde nunca a `initialize`.
            return None;
        }
        if self.is("slow_init") {
            std::thread::sleep(Duration::from_secs(30));
        }
        let version = if self.is("version2") { 2 } else { 1 };
        let auth = if self.is("auth") || self.is("auth_other_error") {
            json!([{ "id": "claude-login", "name": "Iniciar sesión" }])
        } else {
            json!([])
        };
        send(&json!({"jsonrpc":"2.0","id":id,"result":{
            "protocolVersion": version,
            "agentInfo": {"name":"kanbania-fake-acp","title":"Agente ACP falso (pruebas)","version":"0.0.1-prueba"},
            "authMethods": auth,
            "agentCapabilities": {
                "loadSession": true,
                "promptCapabilities": {"image": true, "audio": false, "embeddedContext": true},
                "mcpCapabilities": {"http": true, "sse": false},
                "_meta": {"cuenta": FAKE_EMAIL, "nota": "META-SECRETO"}
            }
        }}));
        None
    }

    fn session_new(&mut self, id: &Value, message: &Value) -> Option<i32> {
        let params = &message["params"];
        let cwd_ok = params["cwd"].as_str().is_some_and(|cwd| {
            std::path::Path::new(cwd).is_absolute() && std::path::Path::new(cwd).is_dir()
        });
        if !cwd_ok
            || params["mcpServers"] != json!([])
            || params["_meta"] != kanbania_runner::acp::claude_code_isolation_meta()
        {
            eprintln!("FAKE: session/new inesperado");
            return Some(WRONG_INPUT_EXIT);
        }
        // Como el adaptador real: notificaciones no estándar con el correo de la cuenta y un
        // diagnóstico en stderr.
        send(
            &json!({"jsonrpc":"2.0","method":"_auth/status_update","params":{"email":FAKE_EMAIL,"status":"ok"}}),
        );
        send(&json!({"jsonrpc":"2.0","method":"auth/status_update","params":{"email":FAKE_EMAIL}}));
        eprintln!("[session/create] sesión de prueba");
        // Diagnóstico con datos de la cuenta, como el adaptador real: el runner los oculta.
        eprintln!("[auth] cuenta {FAKE_EMAIL} clave sk-ant-api03-PRUEBA0000");
        if self.is("auth") {
            send(
                &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":"Authentication required"}}),
            );
            return None;
        }
        if self.is("auth_other_error") {
            send(
                &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":"Internal error"}}),
            );
            return None;
        }
        if self.is("bad_response") {
            send(&json!({"jsonrpc":"2.0","id":id}));
            return None;
        }
        if self.is("no_modes") {
            send(&json!({"jsonrpc":"2.0","id":id,"result":{"sessionId":SESSION_ID}}));
            return None;
        }
        send(&json!({"jsonrpc":"2.0","id":id,"result":{
            "sessionId": SESSION_ID,
            "modes": {
                "currentModeId": "auto",
                "availableModes": [
                    {"id":"default","name":"Default","description":"Always ask before making changes"},
                    {"id":"acceptEdits","name":"Accept Edits"},
                    {"id":"auto","name":"Auto"},
                    {"id":"bypassPermissions","name":"Bypass Permissions"}
                ]
            }
        }}));
        None
    }

    fn set_mode(&mut self, id: &Value, message: &Value) -> Option<i32> {
        let params = &message["params"];
        if params["sessionId"] != SESSION_ID || params["modeId"] != "default" {
            eprintln!("FAKE: session/set_mode inesperado");
            return Some(WRONG_INPUT_EXIT);
        }
        if self.is("set_mode_error") {
            send(
                &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"Invalid params"}}),
            );
            return None;
        }
        send(&json!({"jsonrpc":"2.0","id":id,"result":{}}));
        update(json!({"sessionUpdate":"current_mode_update","currentModeId":"default"}));
        None
    }

    fn prompt(&mut self, id: &Value, message: &Value) -> Option<i32> {
        let params = &message["params"];
        let text = params["prompt"][0]["text"].as_str().unwrap_or_default();
        if params["sessionId"] != SESSION_ID
            || params["prompt"][0]["type"] != "text"
            || text.is_empty()
            || params["prompt"].as_array().map(Vec::len) != Some(1)
        {
            eprintln!("FAKE: session/prompt inesperado");
            return Some(WRONG_INPUT_EXIT);
        }
        eprintln!("prompt recibido: {} caracteres", text.chars().count());
        if let Some(path) = &self.prompt_file
            && std::fs::write(path, text).is_err()
        {
            eprintln!("FAKE: no se pudo guardar el prompt");
            return Some(1);
        }
        match self.scenario.as_str() {
            "ok" => self.ok(id),
            "permission" => self.permission(id),
            "disconnect" => {
                message_chunk("m1", "Respuesta a medias");
                Some(0)
            }
            "malformed" => {
                raw("esto no es JSON");
                None
            }
            "oversize" => {
                let filler = "x".repeat(kanbania_runner::acp::MAX_LINE_BYTES + 100);
                raw(&format!(
                    "{{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{{\"relleno\":\"{filler}\"}}}}"
                ));
                None
            }
            "cancel" => self.wait_cancel(id),
            "ignore_cancel" => {
                message_chunk("m1", "Trabajando sin parar");
                None
            }
            "refusal" => end(id, "refusal"),
            "max_tokens" => end(id, "max_tokens"),
            "cancelled_unasked" => end(id, "cancelled"),
            "config_mode_change" => {
                update(
                    json!({"sessionUpdate":"config_option_update","configOptions":[
                    {"id":"mode","name":"Modo","category":"mode","type":"select","currentValue":"acceptEdits","options":[]}]}),
                );
                None
            }
            "mode_change" => {
                update(
                    json!({"sessionUpdate":"current_mode_update","currentModeId":"bypassPermissions"}),
                );
                None
            }
            "flood" => {
                for n in 0..8000 {
                    message_chunk(&format!("m{}", n / 10), &"a".repeat(1000));
                }
                end(id, "end_turn")
            }
            "child" => self.child(id),
            "permission_on_cancel" => self.permission_on_cancel(id),
            "approval" => self.approval(id),
            "approval_withdraw" => self.approval_withdraw(id),
            "unknown_id" => {
                send(&json!({"jsonrpc":"2.0","id":4242,"result":{}}));
                None
            }
            "other_session" => {
                let other = |update: Value| {
                    send(
                        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"otra-sesion","update":update}}),
                    );
                };
                other(
                    json!({"sessionUpdate":"agent_message_chunk","messageId":"x","content":{"type":"text","text":"INTRUSO"}}),
                );
                other(
                    json!({"sessionUpdate":"current_mode_update","currentModeId":"bypassPermissions"}),
                );
                other(
                    json!({"sessionUpdate":"agent_message_chunk","messageId":"x","content":{"type":"text","text":"INTRUSO"}}),
                );
                message_chunk("m1", "Hecho.");
                end(id, "end_turn")
            }
            "mode_no_session" => {
                send(
                    &json!({"jsonrpc":"2.0","method":"session/update","params":{"update":
                    {"sessionUpdate":"current_mode_update","currentModeId":"bypassPermissions"}}}),
                );
                None
            }
            _ => {
                eprintln!("{USAGE}");
                Some(2)
            }
        }
    }

    fn ok(&mut self, id: &Value) -> Option<i32> {
        // Una petición que el cliente no ofrece: debe responder -32601.
        send(
            &json!({"jsonrpc":"2.0","id":900,"method":"fs/read_text_file","params":{"sessionId":SESSION_ID,"path":"/etc/hosts"}}),
        );
        let reply = self.recv()?;
        if reply["id"] != 900 || reply["error"]["code"] != -32601 {
            eprintln!("FAKE: respuesta inesperada a fs/read_text_file");
            return Some(WRONG_INPUT_EXIT);
        }
        update(
            json!({"sessionUpdate":"agent_thought_chunk","messageId":"t1","content":{"type":"text","text":"PENSAMIENTO-SECRETO"}}),
        );
        message_chunk("m1", "He leído ");
        message_chunk("m1", "el README.");
        update(
            json!({"sessionUpdate":"tool_call","toolCallId":"h1","title":"Leer README.md","kind":"read","status":"pending",
            "rawInput":{"path":"ENTRADA-SECRETA"},"content":[{"type":"content","content":{"type":"text","text":"CONTENIDO-SECRETO"}}]}),
        );
        update(
            json!({"sessionUpdate":"tool_call_update","toolCallId":"h1","status":"completed",
            "content":[{"type":"content","content":{"type":"text","text":"CONTENIDO-SECRETO"}}]}),
        );
        update(json!({"sessionUpdate":"plan","entries":[
            {"content":"Leer el README","priority":"high","status":"completed"},
            {"content":"Resumirlo","priority":"medium","status":"in_progress"}]}));
        update(
            json!({"sessionUpdate":"usage_update","used":1200,"size":200000,"cost":{"amount":0.01,"currency":"USD"}}),
        );
        update(json!({"sessionUpdate":"available_commands_update","availableCommands":[]}));
        // Opción de modo que sigue en `default`: no detiene el turno.
        update(
            json!({"sessionUpdate":"config_option_update","configOptions":[
            {"id":"mode","name":"Modo","category":"mode","type":"select","currentValue":"default","options":[]},
            {"id":"model","name":"Modelo","type":"select","currentValue":"PRUEBA","options":[]}]}),
        );
        send(
            &json!({"jsonrpc":"2.0","method":"_auth/status_update","params":{"email":FAKE_EMAIL}}),
        );
        message_chunk("m2", "Fin.");
        end(id, "end_turn")
    }

    fn permission(&mut self, id: &Value) -> Option<i32> {
        send(
            &json!({"jsonrpc":"2.0","id":"permiso-1","method":"session/request_permission","params":{
                "sessionId": SESSION_ID,
                "toolCall": {"toolCallId":"h1","title":"Editar README.md","kind":"edit","status":"pending"},
                "options": [
                    {"optionId":"permitir-siempre","name":"Permitir siempre","kind":"allow_always"},
                    {"optionId":"permitir-1","name":"Permitir","kind":"allow_once"},
                    {"optionId":"rechazar-1","name":"Rechazar","kind":"reject_once"},
                    {"optionId":"rechazar-siempre","name":"Rechazar siempre","kind":"reject_always"}
                ]
            }}),
        );
        let reply = self.recv()?;
        if reply["id"] != "permiso-1"
            || reply["result"]["outcome"] != json!({"outcome":"selected","optionId":"rechazar-1"})
        {
            eprintln!("FAKE: el permiso no se rechazó con reject_once");
            return Some(WRONG_INPUT_EXIT);
        }
        // Sin opción `reject_once`: debe responder `cancelled`.
        send(
            &json!({"jsonrpc":"2.0","id":2,"method":"session/request_permission","params":{
                "sessionId": SESSION_ID,
                "toolCall": {"toolCallId":"h2","title":"Ejecutar\u{202E} pruebas"},
                "options": [
                    {"optionId":"permitir-1","name":"Permitir","kind":"allow_once"},
                    {"optionId":"rechazar-siempre","name":"Rechazar siempre","kind":"reject_always"}
                ]
            }}),
        );
        let reply = self.recv()?;
        if reply["id"] != 2 || reply["result"]["outcome"] != json!({"outcome":"cancelled"}) {
            eprintln!("FAKE: el permiso sin reject_once no se respondió como cancelado");
            return Some(WRONG_INPUT_EXIT);
        }
        message_chunk("m1", "No tengo permiso: termino sin cambios.");
        end(id, "end_turn")
    }

    /// Pide permiso y espera la respuesta (ver la documentación del módulo).
    fn approval(&mut self, id: &Value) -> Option<i32> {
        send(
            &json!({"jsonrpc":"2.0","id":"permiso-nota","method":"session/request_permission","params":{
                "sessionId": SESSION_ID,
                "toolCall": {
                    "toolCallId":"h3","title":"Crear nota.txt","kind":"execute","status":"pending",
                    "rawInput": {"command":"touch nota.txt"}
                },
                "options": [
                    {"optionId":"permitir-siempre","name":"Permitir siempre","kind":"allow_always"},
                    {"optionId":"permitir-1","name":"Permitir","kind":"allow_once"},
                    {"optionId":"rechazar-1","name":"Rechazar","kind":"reject_once"}
                ]
            }}),
        );
        let mut cancelled = false;
        loop {
            let message = self.recv()?;
            if message["method"] == "session/cancel" {
                cancelled = true;
                continue;
            }
            if message["id"] != "permiso-nota" {
                continue;
            }
            let outcome = &message["result"]["outcome"];
            let choice = if outcome["outcome"] == "cancelled" {
                "cancelled".to_owned()
            } else {
                outcome["optionId"].as_str().unwrap_or("?").to_owned()
            };
            self.note(&format!("permiso: {choice}"));
            if choice == "permitir-1" {
                if std::fs::write(
                    "nota.txt", "hola
",
                )
                .is_err()
                {
                    eprintln!("FAKE: no se pudo crear nota.txt");
                }
                message_chunk("m1", "Creé nota.txt.");
            } else {
                message_chunk("m1", "No tengo permiso: termino sin cambios.");
            }
            return end(id, if cancelled { "cancelled" } else { "end_turn" });
        }
    }

    /// Pide permiso y lo retira con `$/cancel_request`.
    fn approval_withdraw(&mut self, id: &Value) -> Option<i32> {
        send(
            &json!({"jsonrpc":"2.0","id":"permiso-retirado","method":"session/request_permission","params":{
                "sessionId": SESSION_ID,
                "toolCall": {"toolCallId":"h4","title":"Editar main.rs","kind":"edit","locations":[{"path":"src/main.rs"}]},
                "options": [
                    {"optionId":"permitir-1","name":"Permitir","kind":"allow_once"},
                    {"optionId":"rechazar-1","name":"Rechazar","kind":"reject_once"}
                ]
            }}),
        );
        // Deja que el cliente la registre antes de retirarla.
        std::thread::sleep(Duration::from_millis(500));
        send(
            &json!({"jsonrpc":"2.0","method":"$/cancel_request","params":{"requestId":"permiso-retirado"}}),
        );
        loop {
            let message = self.recv()?;
            if message["id"] == "permiso-retirado" {
                if message["error"]["code"] == -32800 {
                    self.note("ok: retirada respondida con -32800");
                } else {
                    self.note("mal: la retirada no se respondió con -32800");
                }
                message_chunk("m1", "Ya no necesito ese permiso.");
                return end(id, "end_turn");
            }
        }
    }

    /// Espera `session/cancel` y después pide un permiso: la app debe responder `cancelled`.
    fn permission_on_cancel(&mut self, id: &Value) -> Option<i32> {
        message_chunk("m1", "Trabajando…");
        loop {
            let message = self.recv()?;
            if message["method"] == "session/cancel" {
                break;
            }
        }
        send(
            &json!({"jsonrpc":"2.0","id":"permiso-tardio","method":"session/request_permission","params":{
                "sessionId": SESSION_ID,
                "toolCall": {"toolCallId":"h9","title":"Borrar todo"},
                "options": [
                    {"optionId":"permitir-1","name":"Permitir","kind":"allow_once"},
                    {"optionId":"rechazar-1","name":"Rechazar","kind":"reject_once"}
                ]
            }}),
        );
        let reply = self.recv()?;
        if reply["id"] == "permiso-tardio"
            && reply["result"]["outcome"] == json!({"outcome":"cancelled"})
        {
            self.note("ok: permiso cancelado");
        } else {
            self.note("mal: permiso durante la cancelación");
        }
        end(id, "cancelled")
    }

    fn wait_cancel(&mut self, id: &Value) -> Option<i32> {
        message_chunk("m1", "Trabajando…");
        loop {
            let message = self.recv()?;
            if message["method"] == "session/cancel" && message["params"]["sessionId"] == SESSION_ID
            {
                return end(id, "cancelled");
            }
        }
    }

    fn child(&mut self, id: &Value) -> Option<i32> {
        let spawned = std::env::current_exe().and_then(|exe| {
            std::process::Command::new(exe)
                .arg(SLEEP_CHILD)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
        });
        let Ok(spawned) = spawned else {
            eprintln!("FAKE: no se pudo lanzar el hijo");
            return Some(1);
        };
        if let Some(path) = &self.pids {
            let tmp = format!("{path}.tmp");
            if std::fs::write(&tmp, format!("{}\n", spawned.id())).is_err()
                || std::fs::rename(&tmp, path).is_err()
            {
                eprintln!("FAKE: no se pudo escribir el pid del hijo");
                return Some(1);
            }
        }
        message_chunk("m1", "Lancé un proceso hijo.");
        end(id, "end_turn")
    }
}

fn raw(line: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}

fn send(message: &Value) {
    raw(&message.to_string());
}

fn update(update: Value) {
    send(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":SESSION_ID,"update":update}}),
    );
}

fn message_chunk(message_id: &str, text: &str) {
    update(
        json!({"sessionUpdate":"agent_message_chunk","messageId":message_id,"content":{"type":"text","text":text}}),
    );
}

fn end(id: &Value, reason: &str) -> Option<i32> {
    send(&json!({"jsonrpc":"2.0","id":id,"result":{"stopReason":reason}}));
    None
}
