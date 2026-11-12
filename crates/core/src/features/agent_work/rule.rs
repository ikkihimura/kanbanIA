//! Regla única de «lista para un agente». La usan la lista (`ready.rs`, como filtro) y el
//! contexto (`context.rs`, como campo `ready`); HU-014 (`claim_story`) debe reutilizarla dentro
//! de su transacción en vez de reescribirla.
//!
//! Expresión SQL booleana sobre los alias `s` (stories) y `c` (su columna). Necesita un
//! parámetro: el id del agente de la sesión.

/// SQL estático (sqlx 0.9 rechaza cadenas dinámicas): úsalo dentro de `concat!`.
macro_rules! ready_for_agent {
    () => {
        "(c.category IN ('backlog', 'ready') \
          AND s.archived_at IS NULL AND s.accepted_at IS NULL \
          AND NOT EXISTS (SELECT 1 FROM story_blockers b WHERE b.story_id = s.id) \
          AND NOT EXISTS (SELECT 1 FROM story_dependencies d \
                          JOIN stories p ON p.id = d.depends_on_id \
                          WHERE d.story_id = s.id AND p.accepted_at IS NULL) \
          AND NOT EXISTS (SELECT 1 FROM story_agents x \
                          WHERE x.story_id = s.id AND x.agent_id <> ?))"
    };
}
pub(super) use ready_for_agent;
