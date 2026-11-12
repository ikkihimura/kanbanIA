-- Credenciales de agentes para el servicio local (HU-012).
-- Cada credencial liga un agente a un proyecto: de ella salen la identidad (Actor::Agent) y
-- el proyecto de toda petición. Solo se guarda el hash SHA-256 del token, nunca el token.

CREATE TABLE agent_credentials (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects (id),
    agent_id TEXT NOT NULL REFERENCES agents (id),
    -- SHA-256 en hexadecimal del token completo.
    token_hash TEXT NOT NULL UNIQUE CHECK (length(token_hash) = 64),
    -- Últimos caracteres del token, para reconocerlo en la lista.
    token_hint TEXT NOT NULL CHECK (length(token_hint) = 4),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_used_at TEXT,
    revoked_at TEXT
) STRICT;

CREATE INDEX agent_credentials_by_project ON agent_credentials (project_id, created_at);
