//! JSON-RPC 2.0 mínimo para ACP v1: clasifica los mensajes entrantes y construye los
//! salientes. Sin lotes (un array es un error). Tolera campos desconocidos.

use serde_json::{Map, Value, json};

/// Código JSON-RPC «método no encontrado».
pub(crate) const METHOD_NOT_FOUND: i64 = -32601;
/// ACP: «autenticación requerida».
pub(crate) const AUTH_REQUIRED: i64 = -32000;
/// ACP: «petición cancelada».
pub(crate) const REQUEST_CANCELLED: i64 = -32800;

/// Error JSON-RPC recibido. `message` es texto del agente (no confiable).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RpcError {
    pub code: i64,
    pub message: String,
}

/// Mensaje entrante.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Incoming {
    /// Respuesta a una petición nuestra (nuestros ids son enteros no negativos).
    Response {
        id: u64,
        result: Result<Value, RpcError>,
    },
    /// Petición del agente (id número o texto, que se devuelve tal cual).
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
}

/// Interpreta una línea. `None` = no es JSON-RPC 2.0 válido para ACP.
pub(crate) fn parse(line: &[u8]) -> Option<Incoming> {
    let value: Value = serde_json::from_slice(line).ok()?;
    let object = value.as_object()?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return None;
    }
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    match (object.get("method"), object.get("id")) {
        (Some(method), Some(id)) => {
            let method = method.as_str()?.to_owned();
            if !(id.is_string() || id.is_i64() || id.is_u64()) {
                return None;
            }
            Some(Incoming::Request {
                id: id.clone(),
                method,
                params,
            })
        }
        (Some(method), None) => Some(Incoming::Notification {
            method: method.as_str()?.to_owned(),
            params,
        }),
        (None, Some(id)) => {
            let id = id.as_u64()?;
            let result = match (object.get("result"), object.get("error")) {
                (Some(result), None) => Ok(result.clone()),
                (None, Some(error)) => Err(parse_error(error)?),
                _ => return None,
            };
            Some(Incoming::Response { id, result })
        }
        (None, None) => None,
    }
}

fn parse_error(error: &Value) -> Option<RpcError> {
    let object = error.as_object()?;
    Some(RpcError {
        code: object.get("code")?.as_i64()?,
        message: object
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

pub(crate) fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

pub(crate) fn notification(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

pub(crate) fn result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub(crate) fn error(id: &Value, code: i64, message: &str) -> Value {
    let mut error = Map::new();
    error.insert("code".into(), code.into());
    error.insert("message".into(), message.into());
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_messages() {
        assert_eq!(
            parse(br#"{"jsonrpc":"2.0","id":3,"result":{"a":1}}"#),
            Some(Incoming::Response {
                id: 3,
                result: Ok(json!({"a":1}))
            })
        );
        assert_eq!(
            parse(br#"{"jsonrpc":"2.0","id":3,"error":{"code":-32000,"message":"x"}}"#),
            Some(Incoming::Response {
                id: 3,
                result: Err(RpcError {
                    code: -32000,
                    message: "x".into()
                })
            })
        );
        assert!(matches!(
            parse(br#"{"jsonrpc":"2.0","id":"p","method":"session/request_permission"}"#),
            Some(Incoming::Request { .. })
        ));
        assert!(matches!(
            parse(br#"{"jsonrpc":"2.0","method":"session/update","params":{}}"#),
            Some(Incoming::Notification { .. })
        ));
    }

    #[test]
    fn rejects_invalid_messages() {
        for bad in [
            &b"no es json"[..],
            br#"[{"jsonrpc":"2.0","id":1,"result":{}}]"#,
            br#"{"id":1,"result":{}}"#,
            br#"{"jsonrpc":"2.0","id":1}"#,
            br#"{"jsonrpc":"2.0","id":1,"result":{},"error":{"code":1}}"#,
            br#"{"jsonrpc":"2.0","id":-1,"result":{}}"#,
            br#"{"jsonrpc":"2.0","id":{},"method":"x"}"#,
            br#"{"jsonrpc":"2.0","method":5}"#,
        ] {
            assert_eq!(parse(bad), None, "{}", String::from_utf8_lossy(bad));
        }
    }
}
