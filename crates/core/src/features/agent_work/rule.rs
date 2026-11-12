//! Regla única de «lista para un agente». La usan la lista (`ready.rs`, como filtro), el
//! contexto (`context.rs`, como campo `ready`) y `claims::claim_story`, que la vuelve a evaluar
//! dentro de su transacción de escritura.
//!
//! Expresión SQL booleana sobre los alias `s` (stories) y `c` (su columna). Parámetros, en
//! este orden: id del agente de la sesión, hora en ms (`Store::now_ms`), id del agente otra vez.
//! Un reclamo abierto de otro agente cuenta mientras no haya vencido (`claimed_by_other!`).

/// SQL estático (sqlx 0.9 rechaza cadenas dinámicas): úsalo dentro de `concat!`.
macro_rules! ready_for_agent {
    () => {
        concat!(
            "(c.category IN ('backlog', 'ready') \
          AND s.archived_at IS NULL AND s.accepted_at IS NULL \
          AND NOT EXISTS (SELECT 1 FROM story_blockers b WHERE b.story_id = s.id) \
          AND NOT EXISTS (SELECT 1 FROM story_dependencies d \
                          JOIN stories p ON p.id = d.depends_on_id \
                          WHERE d.story_id = s.id AND p.accepted_at IS NULL) \
          AND NOT EXISTS (SELECT 1 FROM story_agents x \
                          WHERE x.story_id = s.id AND x.agent_id <> ?) \
          AND NOT ",
            $crate::features::agent_work::claimed_by_other!(),
            ")"
        )
    };
}
pub(crate) use ready_for_agent;

/// Otro agente tiene un reclamo vigente sobre `s`. Parámetros: hora en ms, id del agente.
/// Vigente = abierto y sin vencer: la única definición, usada también por el contexto.
macro_rules! claimed_by_other {
    () => {
        "EXISTS (SELECT 1 FROM story_claims k \
                 WHERE k.story_id = s.id AND k.ended_at_ms IS NULL \
                   AND k.expires_at_ms > ? AND k.agent_id <> ?)"
    };
}
pub(crate) use claimed_by_other;
