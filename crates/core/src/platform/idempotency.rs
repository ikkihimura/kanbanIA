//! Recibos de idempotencia para mutaciones reintentables.
//!
//! Un `request_id` queda ligado a actor + operación + hash SHA-256 del payload:
//! - mismo actor, operación y payload → devuelve el resultado original sin volver a ejecutar;
//! - mismo `request_id` con otra operación o payload → `IdempotencyConflict`.
//!
//! Comprobación, mutación y recibo ocurren en la misma transacción `BEGIN IMMEDIATE`.
//! Si la mutación falla no se guarda recibo, así que un reintento vuelve a ejecutarla.

use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use sqlx::SqliteConnection;

use super::storage::{Store, TxFuture};
use crate::error::CoreError;

const MAX_ACTOR_LEN: usize = 64;
const MAX_REQUEST_ID_LEN: usize = 128;

/// Identidad de una mutación reintentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationKey {
    actor: String,
    operation: &'static str,
    request_id: String,
}

impl MutationKey {
    /// `operation` es un nombre fijo del caso de uso (p. ej. `"stories.create"`), nunca entrada externa.
    pub fn new(
        actor: impl Into<String>,
        operation: &'static str,
        request_id: impl Into<String>,
    ) -> Result<Self, CoreError> {
        let actor = actor.into();
        let request_id = request_id.into();
        validate_token("actor", &actor, MAX_ACTOR_LEN)?;
        validate_token("request_id", &request_id, MAX_REQUEST_ID_LEN)?;
        Ok(Self {
            actor,
            operation,
            request_id,
        })
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }
}

fn validate_token(field: &'static str, value: &str, max_len: usize) -> Result<(), CoreError> {
    let valid_chars = value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'));
    if value.is_empty() || value.len() > max_len || !valid_chars {
        return Err(CoreError::Validation {
            field,
            reason: format!("se esperan 1–{max_len} caracteres ASCII [A-Za-z0-9-_.:]"),
        });
    }
    Ok(())
}

/// Hash sobre JSON canónico (claves ordenadas a cualquier profundidad), para que el orden
/// de iteración de un `HashMap` en el payload no cambie el hash entre procesos.
fn payload_hash<P: Serialize + ?Sized>(payload: &P) -> Result<String, CoreError> {
    let canonical = canonicalize(serde_json::to_value(payload)?);
    let bytes = serde_json::to_vec(&canonical)?;
    let digest = Sha256::digest(&bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn canonicalize(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonicalize(value)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonicalize).collect()),
        other => other,
    }
}

impl Store {
    /// Ejecuta una mutación reintentable. Ver la documentación del módulo.
    pub async fn write_idempotent<P, T, F>(
        &self,
        key: &MutationKey,
        payload: &P,
        mutation: F,
    ) -> Result<T, CoreError>
    where
        P: Serialize + ?Sized,
        T: Serialize + DeserializeOwned + Send,
        F: for<'c> FnOnce(&'c mut SqliteConnection) -> TxFuture<'c, T> + Send,
    {
        let hash = payload_hash(payload)?;
        let mut tx = self.begin_write().await?;

        let previous: Option<(String, String, String)> = sqlx::query_as(
            "SELECT operation, payload_hash, response_json FROM mutation_receipts \
             WHERE actor = ? AND request_id = ?",
        )
        .bind(&key.actor)
        .bind(&key.request_id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some((operation, stored_hash, response)) = previous {
            if operation == key.operation && stored_hash == hash {
                return Ok(serde_json::from_str(&response)?);
            }
            return Err(CoreError::IdempotencyConflict);
        }

        let result = mutation(&mut tx).await?;

        sqlx::query(
            "INSERT INTO mutation_receipts \
             (actor, request_id, operation, payload_hash, response_json) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&key.actor)
        .bind(&key.request_id)
        .bind(key.operation)
        .bind(&hash)
        .bind(serde_json::to_string(&result)?)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_or_empty_identifiers() {
        for bad in ["", "con espacio", "a/b", "ñ", &"x".repeat(129)] {
            assert!(MutationKey::new("ui", "op", bad).is_err(), "aceptó {bad:?}");
        }
        assert!(MutationKey::new("", "op", "req-1").is_err());
        assert!(MutationKey::new("ui", "op", "0b7c-AF_1.2:3").is_ok());
    }

    #[test]
    fn hash_depends_on_payload_content() {
        let a = payload_hash(&serde_json::json!({ "title": "A" })).unwrap();
        let b = payload_hash(&serde_json::json!({ "title": "B" })).unwrap();

        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_eq!(
            a,
            payload_hash(&serde_json::json!({ "title": "A" })).unwrap()
        );
    }

    #[test]
    fn hash_ignores_map_key_order() {
        let mut first = std::collections::HashMap::new();
        let mut second = std::collections::HashMap::new();
        for i in 0..32 {
            first.insert(format!("k{i}"), i);
        }
        for i in (0..32).rev() {
            second.insert(format!("k{i}"), i);
        }
        assert_eq!(
            payload_hash(&first).unwrap(),
            payload_hash(&second).unwrap()
        );
    }
}
