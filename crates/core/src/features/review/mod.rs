//! Slice `review`: entregas, aceptación humana, reapertura, bloqueos y bandeja de atención.
//!
//! - Aceptar, reabrir y devolver entregas exigen `Actor::Human` (un agente recibe
//!   `forbidden`), sea cual sea el transporte.
//! - Aceptar exige al menos un criterio y que todos estén marcados como satisfechos; deja la
//!   tarjeta en la primera columna «hecho» y registra el evento con los criterios.
//! - El historial (`acceptance_events`) nunca se borra: reabrir añade un evento.

mod accept;
mod attention;
mod decisions;
mod delivery;
mod get;
mod submit;

pub use accept::{
    AcceptStoryInput, ReopenStoryInput, SatisfiedCriterion, accept_story, reopen_story,
};
pub use attention::{AttentionItem, list_attention};
pub use decisions::{ReturnDeliveryInput, SetBlockerInput, return_delivery, set_story_blocker};
pub use delivery::{
    Delivery, DeliveryReference, DeliverySource, DeliveryStatus, ReferenceInput, ReferenceKind,
    ReferenceStatus,
};
pub use get::{AcceptanceEvent, AcceptanceEventKind, Blocker, StoryReview, get_story_review};
pub use submit::{SubmitDeliveryInput, submit_delivery};

use sqlx::SqliteConnection;

use crate::error::CoreError;
use crate::features::columns::{Column, ColumnCategory, columns_of_project};
use crate::features::team::load_agent;
use crate::platform::actor::Actor;

/// Caracteres de formato invisibles (bidi, ancho cero) que podrían disfrazar un texto ante
/// quien revisa. Se rechazan en resúmenes y motivos.
pub(crate) fn has_invisible_formatting(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c,
            '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
    })
}

/// Origen de lo que registra un actor: persona → manual; agente existente → reportado.
async fn source_of(
    conn: &mut SqliteConnection,
    actor: &Actor,
) -> Result<(DeliverySource, Option<String>), CoreError> {
    match actor {
        Actor::Human => Ok((DeliverySource::Manual, None)),
        Actor::Agent { agent_id } => {
            load_agent(conn, agent_id)
                .await?
                .ok_or(CoreError::NotFound { entity: "Agente" })?;
            Ok((DeliverySource::Reported, Some(agent_id.clone())))
        }
    }
}

/// Primera columna (por posición) de una categoría.
async fn first_column_of(
    conn: &mut SqliteConnection,
    project_id: &str,
    category: ColumnCategory,
) -> Result<Option<Column>, CoreError> {
    Ok(columns_of_project(conn, project_id)
        .await?
        .into_iter()
        .find(|c| c.category == category))
}
