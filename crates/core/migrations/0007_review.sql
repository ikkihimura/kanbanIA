-- Entregas, aceptación, bloqueos y atención (HU-009).

-- Entrega de trabajo para revisión: manual (persona) o reportada (agente).
CREATE TABLE deliveries (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    story_id TEXT NOT NULL,
    -- Número dentro de la historia (E1, E2…); no se reutiliza.
    number INTEGER NOT NULL CHECK (number >= 1),
    source TEXT NOT NULL CHECK (source IN ('manual', 'reported')),
    agent_id TEXT REFERENCES agents (id),
    summary TEXT NOT NULL CHECK (length(summary) BETWEEN 1 AND 2000),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'accepted', 'returned')),
    return_reason TEXT CHECK (return_reason IS NULL OR length(return_reason) BETWEEN 1 AND 500),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    decided_at TEXT,
    -- Reportada ⇔ tiene agente.
    CHECK ((source = 'reported') = (agent_id IS NOT NULL)),
    CHECK ((status = 'returned') = (return_reason IS NOT NULL)),
    UNIQUE (story_id, number),
    FOREIGN KEY (project_id, story_id) REFERENCES stories (project_id, id)
) STRICT;

CREATE INDEX deliveries_by_project_status ON deliveries (project_id, status);

-- Referencias de una entrega. Las URL no se visitan; las rutas se comprueban al mostrarlas.
CREATE TABLE delivery_references (
    delivery_id TEXT NOT NULL REFERENCES deliveries (id),
    position INTEGER NOT NULL CHECK (position >= 0),
    kind TEXT NOT NULL CHECK (kind IN ('url', 'path')),
    value TEXT NOT NULL CHECK (length(value) BETWEEN 1 AND 2000),
    PRIMARY KEY (delivery_id, position)
) STRICT, WITHOUT ROWID;

-- Historial de aceptación: nunca se borra. Reabrir añade un evento, no elimina el anterior.
CREATE TABLE acceptance_events (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    story_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('accepted', 'reopened')),
    -- Solo personas aceptan o reabren.
    actor TEXT NOT NULL CHECK (actor = 'human'),
    -- Instantánea JSON de los criterios satisfechos al aceptar ([] al reabrir).
    criteria TEXT NOT NULL DEFAULT '[]',
    reason TEXT CHECK (reason IS NULL OR length(reason) BETWEEN 1 AND 500),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK ((kind = 'reopened') = (reason IS NOT NULL)),
    FOREIGN KEY (project_id, story_id) REFERENCES stories (project_id, id)
) STRICT;

CREATE INDEX acceptance_events_by_story ON acceptance_events (story_id, created_at);

-- Bloqueo con motivo (atributo de la historia, no una columna del tablero).
CREATE TABLE story_blockers (
    story_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
    source TEXT NOT NULL CHECK (source IN ('manual', 'reported')),
    agent_id TEXT REFERENCES agents (id),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK ((source = 'reported') = (agent_id IS NOT NULL)),
    FOREIGN KEY (project_id, story_id) REFERENCES stories (project_id, id)
) STRICT;
