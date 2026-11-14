use sqlx::SqliteConnection;

use crate::error::CoreError;
use crate::features::agent_work::{claimed_by_other, pending_delivery};
use crate::features::stories::StorySummary;

/// Precondiciones para que una historia admita un intento del agente `agent_id` a la hora
/// `now`. Las evalúa `create_run` dentro de su transacción y **HU-018 debe volver a llamarla
/// al despachar** un intento en cola (las condiciones pueden cambiar mientras espera).
///
/// - sin archivar, sin aceptar, sin bloqueo y sin prerequisitos pendientes (las dependencias
///   bloquean la ejecución);
/// - sin entrega pendiente de revisión (una persona debe decidir antes);
/// - sin reclamo vigente de otro agente (decisión del propietario).
///
/// El reclamo y la entrega usan las mismas expresiones SQL que la regla de «lista»
/// (`agent_work/rule.rs`); el intento activo lo comprueba quien llama.
pub(crate) async fn ensure_runnable(
    conn: &mut SqliteConnection,
    story: &StorySummary,
    agent_id: &str,
    now: i64,
) -> Result<(), CoreError> {
    let n = story.number;
    if story.archived_at.is_some() {
        return Err(invalid(format!(
            "La historia #{n} está archivada: restáurala antes de preparar un intento."
        )));
    }
    if story.accepted_at.is_some() {
        return Err(invalid(format!(
            "La historia #{n} ya está aceptada: reábrela para preparar otro intento."
        )));
    }
    if story.blocker_reason.is_some() {
        return Err(invalid(format!(
            "La historia #{n} está bloqueada: quita el bloqueo antes de preparar un intento."
        )));
    }
    if story.pending_prerequisites > 0 {
        return Err(invalid(format!(
            "La historia #{n} tiene prerequisitos sin aceptar ({}): las dependencias bloquean \
             la ejecución.",
            story.pending_prerequisites
        )));
    }
    let (claimed, pending): (bool, bool) = sqlx::query_as(concat!(
        "SELECT ",
        claimed_by_other!(),
        ", ",
        pending_delivery!(),
        " FROM stories s WHERE s.id = ?"
    ))
    .bind(now)
    .bind(agent_id)
    .bind(&story.id)
    .fetch_one(&mut *conn)
    .await?;
    if pending {
        return Err(invalid(format!(
            "La historia #{n} tiene una entrega pendiente de revisión: revisa la entrega \
             (acéptala o devuélvela) antes de preparar un intento."
        )));
    }
    if claimed {
        return Err(CoreError::ClaimedForRun);
    }
    Ok(())
}

fn invalid(reason: String) -> CoreError {
    CoreError::Validation {
        field: "storyId",
        reason,
    }
}
