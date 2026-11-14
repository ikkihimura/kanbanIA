//! Slice `claims`: reclamos exclusivos de historias por agentes (HU-014).
//!
//! - `claim_story` y `renew_claim` reciben la `AgentSession`: identidad y proyecto salen de
//!   ahí, nunca de la petición. Son reintentables con `requestId`; el recibo se liga a la
//!   credencial (un agente puede tener credenciales en varios proyectos).
//! - Reclamar vuelve a evaluar la regla de «lista» (`agent_work::ready_for_agent!`) dentro de
//!   la transacción de escritura: nunca se fía de un `ready` leído antes. Un índice único
//!   impide dos reclamos abiertos de la misma historia.
//! - Reclamar asigna la historia al agente si no tenía a nadie (decisión del propietario). Al
//!   vencer o liberarse, la asignación se queda: la historia sigue lista para ese agente; para
//!   que la tome otro, una persona quita o cambia la asignación, y eso cierra también el
//!   reclamo vigente del agente anterior (`end_open_claims`, desde `relations`).
//! - Como mucho `MAX_OPEN_CLAIMS` reclamos vigentes por agente y proyecto.
//! - Solo una persona libera un reclamo (`release_claim`, `Actor::Human`). Restaurar una copia
//!   cierra todos los reclamos de la copia.
//! - La hora se lee dentro de la transacción de escritura (tras esperar el bloqueo).
//! - Toda operación de agentes sobre una historia (renovar y, desde HU-015, bloqueos,
//!   subtareas y entregas) pasa por `require_active_claim`. Entregar cierra el reclamo
//!   (`delivered`); aceptar o archivar lo cierran (`closed`); revocar la credencial con la que
//!   se reclamó, también (`revoked`). En todos los casos la asignación se queda.

mod active;
mod claim;
mod end;
mod list;
mod release;
mod renew;
mod story_claim;

pub(crate) use active::require_active_claim;
pub use claim::{ClaimStoryInput, claim_story};
pub(crate) use end::{
    EndReason, close_credential_claims, close_restored_claims, close_story_claims, end_claim,
    end_open_claims,
};
pub use list::list_active_claims;
pub use release::{ReleaseClaimInput, release_claim};
pub use renew::{RenewClaimInput, renew_claim};
pub use story_claim::{CLAIM_DURATION_MINUTES, ClaimOwner, MAX_OPEN_CLAIMS, StoryClaim};
