-- Contenido de historias y criterios de aceptación (HU-005).
-- SQLite no permite cambiar restricciones en sitio: se reconstruye `stories`.
-- `position` es NULL solo cuando la historia está archivada (fuera del orden del tablero).
CREATE TABLE stories_new (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects (id),
    -- Número legible por proyecto (#N); nunca se reutiliza (las historias se archivan, no se borran).
    number INTEGER NOT NULL CHECK (number >= 1),
    column_id TEXT NOT NULL,
    position INTEGER CHECK (position >= 0),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    description TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 20000),
    priority TEXT NOT NULL DEFAULT 'medium'
        CHECK (priority IN ('urgent', 'high', 'medium', 'low')),
    -- Siguiente número CA-n: los criterios eliminados no liberan su número.
    next_criterion_number INTEGER NOT NULL DEFAULT 1 CHECK (next_criterion_number >= 1),
    archived_at TEXT,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (project_id, column_id) REFERENCES columns (project_id, id),
    UNIQUE (project_id, number),
    CHECK ((archived_at IS NULL) = (position IS NOT NULL))
) STRICT;

INSERT INTO stories_new (
    id, project_id, number, column_id, position, title, revision, created_at, updated_at
)
SELECT
    s.id,
    s.project_id,
    (SELECT COUNT(*) FROM stories o
     WHERE o.project_id = s.project_id
       AND (o.created_at < s.created_at OR (o.created_at = s.created_at AND o.id <= s.id))),
    s.column_id,
    s.position,
    s.title,
    s.revision,
    s.created_at,
    s.updated_at
FROM stories s;

DROP TABLE stories;
ALTER TABLE stories_new RENAME TO stories;
CREATE INDEX stories_by_column ON stories (column_id, position);

CREATE TABLE acceptance_criteria (
    id TEXT PRIMARY KEY,
    story_id TEXT NOT NULL REFERENCES stories (id),
    -- Identificador visible CA-n, estable dentro de la historia.
    number INTEGER NOT NULL CHECK (number >= 1),
    position INTEGER NOT NULL CHECK (position >= 0),
    text TEXT NOT NULL CHECK (length(text) BETWEEN 1 AND 500),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (story_id, number)
) STRICT;

CREATE INDEX acceptance_criteria_by_story ON acceptance_criteria (story_id, position);
