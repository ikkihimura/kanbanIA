-- Reportes y entregas de agentes (HU-015).
--
-- 1. Nuevos motivos de cierre de reclamos. SQLite no cambia un CHECK en sitio: se reconstruye
--    `story_claims` (nada la referencia) conservando filas e índices.
--    `delivered`: el agente entregó; `revoked`: se revocó la credencial con la que se reclamó;
--    `closed`: la historia se aceptó o archivó.
-- 2. `agent_activity`: lo que los agentes reportan por MCP. Son afirmaciones, no hechos
--    observados, y no indican que el proceso del agente siga en marcha.

CREATE TABLE story_claims_new (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    story_id TEXT NOT NULL,
    agent_id TEXT NOT NULL REFERENCES agents (id),
    credential_id TEXT NOT NULL REFERENCES agent_credentials (id),
    claimed_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    renewed_at_ms INTEGER,
    ended_at_ms INTEGER,
    end_reason TEXT CHECK (end_reason IN ('expired', 'released', 'delivered', 'revoked', 'closed')),
    CHECK (expires_at_ms > claimed_at_ms),
    CHECK ((ended_at_ms IS NULL) = (end_reason IS NULL)),
    FOREIGN KEY (project_id, story_id) REFERENCES stories (project_id, id)
) STRICT;

INSERT INTO story_claims_new (
    id, project_id, story_id, agent_id, credential_id, claimed_at_ms, expires_at_ms,
    renewed_at_ms, ended_at_ms, end_reason
)
SELECT
    id, project_id, story_id, agent_id, credential_id, claimed_at_ms, expires_at_ms,
    renewed_at_ms, ended_at_ms, end_reason
FROM story_claims;

DROP TABLE story_claims;
ALTER TABLE story_claims_new RENAME TO story_claims;

CREATE UNIQUE INDEX story_claims_one_open ON story_claims (story_id) WHERE ended_at_ms IS NULL;
CREATE INDEX story_claims_open_by_project ON story_claims (project_id) WHERE ended_at_ms IS NULL;

-- Actividad reportada por agentes sobre una historia. `detail` es una referencia corta
-- generada por el núcleo (p. ej. «#12» o «E2»), nunca texto del agente.
CREATE TABLE agent_activity (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    story_id TEXT NOT NULL,
    agent_id TEXT NOT NULL REFERENCES agents (id),
    kind TEXT NOT NULL CHECK (kind IN (
        'claimed', 'renewed', 'blocker_reported', 'blocker_cleared', 'subtask_created',
        'delivered'
    )),
    detail TEXT CHECK (detail IS NULL OR length(detail) BETWEEN 1 AND 40),
    at_ms INTEGER NOT NULL,
    FOREIGN KEY (project_id, story_id) REFERENCES stories (project_id, id)
) STRICT;

CREATE INDEX agent_activity_by_story ON agent_activity (story_id, at_ms);

-- Historias creadas por agentes (subtareas, HU-015): procedencia para los topes (pendientes por
-- proyecto, un solo nivel) y para mostrarlas como texto de un agente.
CREATE TABLE agent_created_stories (
    story_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    agent_id TEXT NOT NULL REFERENCES agents (id),
    credential_id TEXT NOT NULL REFERENCES agent_credentials (id),
    created_at_ms INTEGER NOT NULL,
    FOREIGN KEY (project_id, story_id) REFERENCES stories (project_id, id)
) STRICT;

CREATE INDEX agent_created_stories_by_project ON agent_created_stories (project_id);
