//! Normalización de `session/update` (ACP v1) a líneas `agent` del registro del intento.
//!
//! Todo es dato **reportado** por el agente y no confiable: se guarda como texto plano y la UI
//! lo muestra así. Reglas (diseño de HU-019):
//! - `agent_message_chunk`: texto agrupado por `messageId` (o hasta que llega otra cosa o se
//!   acaba el turno), en líneas de como mucho 4 KiB; el contenido no textual se indica sin
//!   copiarlo;
//! - `agent_thought_chunk`: solo «(pensando…)» con la longitud, nunca el contenido;
//! - `tool_call`/`tool_call_update`: título, tipo y estado (sin `content`, `rawInput`,
//!   `rawOutput` ni `locations`); una línea solo si algo de eso cambió;
//! - `plan`: una línea de texto por entrada con su estado;
//! - `usage_update`: `used`/`size` y `cost` tal como los informa (no son tokens del intento);
//! - `current_mode_update` y `config_option_update` de la opción de modo: lo decide quien llama
//!   (política de modo);
//! - en todo texto del agente se sustituyen por un espacio los controles y el formato invisible
//!   (bidi, ancho cero, etiquetas Unicode…), igual que detecta el núcleo;
//! - el resto de tipos se anota una vez en `sys` (solo el tipo).

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::output::{AgentKind, MAX_LINE_BYTES, truncate_at_char};

/// Caracteres máximos de un título de herramienta o de una entrada del plan.
const MAX_TITLE_CHARS: usize = 300;
/// Entradas del plan que se muestran como mucho.
const MAX_PLAN_ENTRIES: usize = 50;
/// Herramientas recordadas como mucho (para los `tool_call_update` sin título).
const MAX_TOOLS: usize = 1024;
/// Tipos de actualización o métodos distintos que se anotan como mucho.
const MAX_NOTES: usize = 32;
/// Tipos de contenido no textual que se indican como mucho en un mensaje.
const MAX_SKIPPED_KINDS: usize = 8;
/// Valor que se informa cuando la opción de modo no se entiende (nunca coincide con un modo).
const UNREADABLE_MODE: &str = "(modo no reconocible)";

/// Lo que produce una actualización.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Out {
    Agent(AgentKind, String),
    System(String),
}

/// Efecto que debe decidir quien llama.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Effect {
    None,
    /// `current_mode_update` con este modo (ya saneado).
    ModeChanged(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChunkKind {
    Message,
    Thought,
}

struct Pending {
    kind: ChunkKind,
    message_id: Option<String>,
    /// Texto (solo mensajes).
    text: String,
    /// Caracteres recibidos (pensamiento).
    chars: usize,
    /// Tipos de contenido no textual ya indicados en este mensaje.
    skipped: Vec<String>,
}

#[derive(Default, Clone)]
struct Tool {
    title: String,
    kind: String,
    status: String,
}

#[derive(Default)]
pub(crate) struct Normalizer {
    pending: Option<Pending>,
    tools: HashMap<String, Tool>,
    noted: HashSet<String>,
}

impl Normalizer {
    /// Procesa el campo `update` de una notificación `session/update`.
    pub(crate) fn update(&mut self, update: &Value, out: &mut Vec<Out>) -> Effect {
        let kind = update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match kind {
            "agent_message_chunk" => {
                self.chunk(ChunkKind::Message, update, out);
                return Effect::None;
            }
            "agent_thought_chunk" => {
                self.chunk(ChunkKind::Thought, update, out);
                return Effect::None;
            }
            _ => {}
        }
        self.flush(out);
        match kind {
            "tool_call" | "tool_call_update" => self.tool(kind == "tool_call", update, out),
            "plan" => plan(update, out),
            "usage_update" => usage(update, out),
            "current_mode_update" => {
                let mode = update
                    .get("currentModeId")
                    .and_then(Value::as_str)
                    .map_or_else(|| "(sin modo)".to_owned(), ascii_name);
                return Effect::ModeChanged(mode);
            }
            "config_option_update" => {
                if let Some(mode) = config_mode(update) {
                    return Effect::ModeChanged(mode);
                }
                self.note(
                    "update:config_option_update".into(),
                    "Actualización de sesión ignorada: «config_option_update» sin opción de modo \
                     (no se registra su contenido)."
                        .into(),
                    out,
                );
            }
            other => self.note(
                format!("update:{other}"),
                format!(
                    "Actualización de sesión ignorada: «{}» (no se registra su contenido).",
                    ascii_name(other)
                ),
                out,
            ),
        }
        Effect::None
    }

    /// Anota una vez (por clave) un mensaje del runner.
    /// Vuelca antes el mensaje pendiente, para que la salida conserve el orden.
    pub(crate) fn note(&mut self, key: String, text: String, out: &mut Vec<Out>) {
        if self.noted.len() < MAX_NOTES && self.noted.insert(key) {
            self.flush(out);
            out.push(Out::System(text));
        }
    }

    /// Emite lo pendiente (fin de turno, otro tipo de actualización…).
    pub(crate) fn flush(&mut self, out: &mut Vec<Out>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        match pending.kind {
            ChunkKind::Message => {
                if !pending.text.is_empty() {
                    out.push(Out::Agent(AgentKind::Message, pending.text));
                }
            }
            ChunkKind::Thought => {
                if pending.chars > 0 {
                    out.push(Out::Agent(
                        AgentKind::Thought,
                        format!("(pensando…, {} caracteres)", pending.chars),
                    ));
                }
            }
        }
    }

    fn chunk(&mut self, kind: ChunkKind, update: &Value, out: &mut Vec<Out>) {
        let message_id = update
            .get("messageId")
            .and_then(Value::as_str)
            .map(|id| truncate_at_char(id, 128).0.to_owned());
        let continues = self
            .pending
            .as_ref()
            .is_some_and(|p| p.kind == kind && p.message_id == message_id);
        if !continues {
            self.flush(out);
            self.pending = Some(Pending {
                kind,
                message_id,
                text: String::new(),
                chars: 0,
                skipped: Vec::new(),
            });
        }
        let Some(pending) = self.pending.as_mut() else {
            return;
        };
        let content = update.get("content");
        let content_type = content
            .and_then(|c| c.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("desconocido");
        match (
            content_type,
            content.and_then(|c| c.get("text")).and_then(Value::as_str),
        ) {
            ("text", Some(text)) => match kind {
                ChunkKind::Thought => pending.chars += text.chars().count(),
                ChunkKind::Message => pending.text.push_str(&clean_multiline(text)),
            },
            (other, _) => {
                let other = ascii_name(other);
                if kind == ChunkKind::Message
                    && pending.skipped.len() < MAX_SKIPPED_KINDS
                    && !pending.skipped.contains(&other)
                {
                    pending
                        .text
                        .push_str(&format!("[contenido «{other}» no mostrado]"));
                    pending.skipped.push(other);
                }
            }
        }
        // Trozos de como mucho 4 KiB sin partir caracteres.
        while pending.text.len() >= MAX_LINE_BYTES {
            let (head, _) = truncate_at_char(&pending.text, MAX_LINE_BYTES);
            let head = head.to_owned();
            pending.text.drain(..head.len());
            out.push(Out::Agent(AgentKind::Message, head));
        }
    }

    fn tool(&mut self, created: bool, update: &Value, out: &mut Vec<Out>) {
        let Some(id) = update.get("toolCallId").and_then(Value::as_str) else {
            self.note(
                "tool:no-id".into(),
                "Una actualización de herramienta llegó sin identificador: se ignora.".into(),
                out,
            );
            return;
        };
        let id = truncate_at_char(id, 256).0.to_owned();
        let previous = self.tools.get(&id).cloned();
        let mut tool = previous.clone().unwrap_or_else(|| Tool {
            title: "(herramienta sin título)".into(),
            kind: "other".into(),
            status: "pending".into(),
        });
        if let Some(title) = update.get("title").and_then(Value::as_str) {
            tool.title = single_line(title, MAX_TITLE_CHARS);
        }
        if let Some(kind) = update.get("kind").and_then(Value::as_str) {
            tool.kind = tool_kind(kind).to_owned();
        }
        if let Some(status) = update.get("status").and_then(Value::as_str) {
            tool.status = status_code(status).to_owned();
        } else if created {
            tool.status = "pending".into();
        }
        let changed = previous.as_ref().is_none_or(|p| {
            p.title != tool.title || p.kind != tool.kind || p.status != tool.status
        });
        if changed {
            out.push(Out::Agent(
                AgentKind::Tool,
                format!(
                    "«{}» · tipo: {} · estado: {}",
                    tool.title,
                    tool.kind,
                    status_label(&tool.status)
                ),
            ));
        }
        if self.tools.len() >= MAX_TOOLS && !self.tools.contains_key(&id) {
            self.tools.clear();
        }
        self.tools.insert(id, tool);
    }
}

/// Valor de la opción de modo de un `config_option_update` (`id == "mode"` o
/// `category == "mode"`), si la trae. Un valor que no es texto (o una lista que no se entiende)
/// cuenta como un modo no reconocible: nunca como el modo exigido.
fn config_mode(update: &Value) -> Option<String> {
    let Some(options) = update.get("configOptions").and_then(Value::as_array) else {
        return Some(UNREADABLE_MODE.to_owned());
    };
    let option = options.iter().find(|option| {
        option.get("id").and_then(Value::as_str) == Some("mode")
            || option.get("category").and_then(Value::as_str) == Some("mode")
    })?;
    Some(
        option
            .get("currentValue")
            .and_then(Value::as_str)
            .map_or_else(|| UNREADABLE_MODE.to_owned(), ascii_name),
    )
}

fn plan(update: &Value, out: &mut Vec<Out>) {
    let Some(entries) = update.get("entries").and_then(Value::as_array) else {
        return;
    };
    let mut lines: Vec<String> = entries
        .iter()
        .take(MAX_PLAN_ENTRIES)
        .map(|entry| {
            let content = entry.get("content").and_then(Value::as_str).map_or_else(
                || "(sin texto)".to_owned(),
                |c| single_line(c, MAX_TITLE_CHARS),
            );
            let status = entry
                .get("status")
                .and_then(Value::as_str)
                .map_or("desconocido", |s| status_label(status_code(s)));
            format!("[{status}] {content}")
        })
        .collect();
    if entries.len() > MAX_PLAN_ENTRIES {
        lines.push(format!(
            "(y {} entradas más)",
            entries.len() - MAX_PLAN_ENTRIES
        ));
    }
    if lines.is_empty() {
        lines.push("(plan vacío)".into());
    }
    out.push(Out::Agent(AgentKind::Plan, lines.join("\n")));
}

fn usage(update: &Value, out: &mut Vec<Out>) {
    let used = update.get("used").and_then(Value::as_u64);
    let size = update.get("size").and_then(Value::as_u64);
    let mut text = match (used, size) {
        (Some(used), Some(size)) => format!("Contexto: {used} de {size} tokens."),
        _ => "Contexto: no disponible.".to_owned(),
    };
    let cost = update.get("cost").filter(|c| !c.is_null());
    match cost.map(|c| (c.get("amount").and_then(Value::as_f64), c.get("currency"))) {
        Some((Some(amount), currency)) => {
            let currency = currency.and_then(Value::as_str).map_or_else(
                || "(sin unidad)".to_owned(),
                |c| ascii_name(truncate_at_char(c, 16).0),
            );
            text.push_str(&format!(" Coste acumulado: {amount} {currency}."));
        }
        _ => text.push_str(" Coste: no disponible."),
    }
    out.push(Out::Agent(AgentKind::Usage, text));
}

fn tool_kind(kind: &str) -> &'static str {
    match kind {
        "read" => "read",
        "edit" => "edit",
        "delete" => "delete",
        "move" => "move",
        "search" => "search",
        "execute" => "execute",
        "think" => "think",
        "fetch" => "fetch",
        "switch_mode" => "switch_mode",
        _ => "other",
    }
}

fn status_code(status: &str) -> &'static str {
    match status {
        "pending" => "pending",
        "in_progress" => "in_progress",
        "completed" => "completed",
        "failed" => "failed",
        _ => "unknown",
    }
}

fn status_label(status: &str) -> &'static str {
    match status {
        "pending" => "pendiente",
        "in_progress" => "en curso",
        "completed" => "completada",
        "failed" => "fallida",
        _ => "desconocido",
    }
}

/// Nombre de método, tipo o modo: ASCII visible, como mucho 64 caracteres.
pub(crate) fn ascii_name(name: &str) -> String {
    if !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_graphic()) {
        name.to_owned()
    } else {
        "(nombre no válido)".to_owned()
    }
}

/// Formato invisible: el mismo conjunto que `has_invisible_formatting` del núcleo
/// (`crates/core/src/platform/text.rs`), copiado porque el runner sin la feature `supervisor` no
/// enlaza el núcleo. Si cambia allí, cámbialo aquí.
pub(crate) fn is_invisible_formatting(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{180E}'
        | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{2069}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}'
        | '\u{E0000}'..='\u{E007F}')
}

/// Texto de una línea: controles (incluidos saltos) y formato invisible → espacio; recortado a
/// `max` caracteres.
pub(crate) fn single_line(text: &str, max: usize) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| {
            if c.is_control() || is_invisible_formatting(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.chars().count() > max {
        let mut cut: String = trimmed.chars().take(max).collect();
        cut.push('…');
        cut
    } else {
        trimmed.to_owned()
    }
}

/// Texto de varias líneas: conserva `\n` y `\t`; el resto de controles (p. ej. ESC) y el
/// formato invisible → espacio.
fn clean_multiline(text: &str) -> String {
    text.chars()
        .map(|c| {
            if (c.is_control() && c != '\n' && c != '\t') || is_invisible_formatting(c) {
                ' '
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(updates: &[Value]) -> Vec<Out> {
        let mut normalizer = Normalizer::default();
        let mut out = Vec::new();
        for update in updates {
            normalizer.update(update, &mut out);
        }
        normalizer.flush(&mut out);
        out
    }

    #[test]
    fn groups_messages_by_id_and_hides_thoughts() {
        let out = run(&[
            json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"SECRETO"}}),
            json!({"sessionUpdate":"agent_message_chunk","messageId":"a","content":{"type":"text","text":"Hola, "}}),
            json!({"sessionUpdate":"agent_message_chunk","messageId":"a","content":{"type":"text","text":"mundo\u{1b}[31m"}}),
            json!({"sessionUpdate":"agent_message_chunk","messageId":"b","content":{"type":"image","data":"…"}}),
        ]);
        assert_eq!(
            out,
            vec![
                Out::Agent(AgentKind::Thought, "(pensando…, 7 caracteres)".into()),
                Out::Agent(AgentKind::Message, "Hola, mundo [31m".into()),
                Out::Agent(AgentKind::Message, "[contenido «image» no mostrado]".into()),
            ]
        );
    }

    #[test]
    fn invisible_formatting_is_replaced() {
        assert_eq!(single_line("a\u{202E}b\u{200B}c\u{E0041}", 50), "a b c");
        let out = run(&[
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"x\u{2066}y\nz"}}),
        ]);
        assert_eq!(out, vec![Out::Agent(AgentKind::Message, "x y\nz".into())]);
    }

    #[test]
    fn skipped_content_kinds_are_capped() {
        let updates: Vec<Value> = (0..20)
            .map(|n| json!({"sessionUpdate":"agent_message_chunk","messageId":"m","content":{"type":format!("tipo{n}")}}))
            .collect();
        let out = run(&updates);
        let [Out::Agent(AgentKind::Message, text)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert_eq!(text.matches("no mostrado").count(), MAX_SKIPPED_KINDS);
    }

    #[test]
    fn config_option_mode_is_reported() {
        let mut normalizer = Normalizer::default();
        let mut out = Vec::new();
        let update = |options: Value| json!({"sessionUpdate":"config_option_update","configOptions":options});
        assert_eq!(
            normalizer.update(&update(json!([{"id":"model","currentValue":"x"},{"id":"mode","currentValue":"default"}])), &mut out),
            Effect::ModeChanged("default".into())
        );
        assert_eq!(
            normalizer.update(
                &update(json!([{"id":"permisos","category":"mode","currentValue":"acceptEdits"}])),
                &mut out
            ),
            Effect::ModeChanged("acceptEdits".into())
        );
        assert_eq!(
            normalizer.update(
                &update(json!([{"id":"mode","currentValue":true}])),
                &mut out
            ),
            Effect::ModeChanged(UNREADABLE_MODE.into())
        );
        assert_eq!(
            normalizer.update(&json!({"sessionUpdate":"config_option_update"}), &mut out),
            Effect::ModeChanged(UNREADABLE_MODE.into())
        );
        assert_eq!(
            normalizer.update(
                &update(json!([{"id":"model","currentValue":"x"}])),
                &mut out
            ),
            Effect::None
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn long_messages_are_split_at_4_kib() {
        let text = "é".repeat(5000); // 10 000 bytes
        let out = run(&[
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":text}}),
        ]);
        assert_eq!(out.len(), 3);
        for line in &out {
            let Out::Agent(AgentKind::Message, text) = line else {
                panic!("{line:?}")
            };
            assert!(text.len() <= MAX_LINE_BYTES);
        }
    }

    #[test]
    fn tools_without_content_and_only_on_change() {
        let out = run(&[
            json!({"sessionUpdate":"tool_call","toolCallId":"t1","title":"Leer\nREADME","kind":"read","status":"pending","rawInput":{"path":"/secreto"},"content":[{"type":"content","content":{"type":"text","text":"SECRETO"}}]}),
            json!({"sessionUpdate":"tool_call_update","toolCallId":"t1","content":[{"type":"content","content":{"type":"text","text":"SECRETO"}}]}),
            json!({"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed"}),
            json!({"sessionUpdate":"tool_call_update","toolCallId":"t2","kind":"rara","status":"raro"}),
        ]);
        assert_eq!(
            out,
            vec![
                Out::Agent(
                    AgentKind::Tool,
                    "«Leer README» · tipo: read · estado: pendiente".into()
                ),
                Out::Agent(
                    AgentKind::Tool,
                    "«Leer README» · tipo: read · estado: completada".into()
                ),
                Out::Agent(
                    AgentKind::Tool,
                    "«(herramienta sin título)» · tipo: other · estado: desconocido".into()
                ),
            ]
        );
    }

    #[test]
    fn plan_usage_mode_and_unknown_updates() {
        let mut normalizer = Normalizer::default();
        let mut out = Vec::new();
        normalizer.update(
            &json!({"sessionUpdate":"plan","entries":[{"content":"Leer","priority":"high","status":"completed"},{"content":"Resumir","priority":"low","status":"in_progress"}]}),
            &mut out,
        );
        normalizer.update(
            &json!({"sessionUpdate":"usage_update","used":1200,"size":200000,"cost":{"amount":0.25,"currency":"USD"}}),
            &mut out,
        );
        normalizer.update(
            &json!({"sessionUpdate":"usage_update","used":1,"size":2}),
            &mut out,
        );
        for _ in 0..2 {
            normalizer.update(
                &json!({"sessionUpdate":"available_commands_update","availableCommands":[]}),
                &mut out,
            );
        }
        assert_eq!(
            normalizer.update(
                &json!({"sessionUpdate":"current_mode_update","currentModeId":"auto"}),
                &mut out
            ),
            Effect::ModeChanged("auto".into())
        );
        assert_eq!(
            out,
            vec![
                Out::Agent(AgentKind::Plan, "[completada] Leer\n[en curso] Resumir".into()),
                Out::Agent(
                    AgentKind::Usage,
                    "Contexto: 1200 de 200000 tokens. Coste acumulado: 0.25 USD.".into()
                ),
                Out::Agent(AgentKind::Usage, "Contexto: 1 de 2 tokens. Coste: no disponible.".into()),
                Out::System(
                    "Actualización de sesión ignorada: «available_commands_update» (no se registra su contenido)."
                        .into()
                ),
            ]
        );
    }
}
