-- Perfil del usuario local, agentes y asignación de agente a historia (HU-008).
-- El usuario y los agentes son entidades distintas. Los agentes son del equipo (no de un
-- proyecto); el acotamiento por proyecto de sus sesiones llega con MCP (M2).

-- Un único usuario local. `avatar` es un AvatarSpec en JSON, validado en el núcleo.
CREATE TABLE user_profile (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 60),
    avatar TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

INSERT INTO user_profile (id, name, avatar) VALUES (
    1,
    'Yo',
    '{"head":0,"skin":1,"eyes":0,"mouth":0,"hair":1,"hairColor":2,"accessory":0,"background":5}'
);

CREATE TABLE agents (
    id TEXT PRIMARY KEY,
    -- Número legible (A-n); los agentes no se borran en este hito.
    number INTEGER NOT NULL UNIQUE CHECK (number >= 1),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 40),
    -- Clave de unicidad del nombre (minúsculas), calculada en el núcleo.
    name_key TEXT NOT NULL UNIQUE,
    role TEXT NOT NULL DEFAULT '' CHECK (length(role) <= 60),
    provider TEXT NOT NULL DEFAULT '' CHECK (length(provider) <= 60),
    model TEXT NOT NULL DEFAULT '' CHECK (length(model) <= 80),
    -- Lista JSON de capacidades del conjunto fijo, validada en el núcleo.
    capabilities TEXT NOT NULL DEFAULT '[]',
    avatar TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- Un agente asignado como mucho por historia. Asignar no crea ejecuciones ni actividad.
CREATE TABLE story_agents (
    story_id TEXT PRIMARY KEY REFERENCES stories (id),
    agent_id TEXT NOT NULL REFERENCES agents (id),
    assigned_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX story_agents_by_agent ON story_agents (agent_id);
