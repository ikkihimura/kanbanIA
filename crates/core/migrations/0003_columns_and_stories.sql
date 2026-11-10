-- Columnas con categoría semántica y tarjetas (historias) mínimas (HU-004).
-- HU-005 amplía `stories` con contenido; HU-007 añade el tablero.
CREATE TABLE columns (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects (id),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 60),
    category TEXT NOT NULL CHECK (category IN ('backlog', 'ready', 'active', 'review', 'done')),
    position INTEGER NOT NULL CHECK (position >= 0),
    wip_limit INTEGER CHECK (wip_limit IS NULL OR wip_limit BETWEEN 1 AND 999),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- Destino de la clave foránea compuesta de `stories`.
    UNIQUE (project_id, id)
) STRICT;

CREATE INDEX columns_by_project ON columns (project_id, position);

CREATE TABLE stories (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects (id),
    column_id TEXT NOT NULL,
    position INTEGER NOT NULL CHECK (position >= 0),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- Una tarjeta solo puede estar en una columna de su mismo proyecto.
    FOREIGN KEY (project_id, column_id) REFERENCES columns (project_id, id)
) STRICT;

CREATE INDEX stories_by_column ON stories (column_id, position);

-- Flujo inicial para los proyectos existentes (los nuevos lo reciben al crearse).
INSERT INTO columns (id, project_id, name, category, position)
SELECT lower(hex(randomblob(16))), p.id, d.name, d.category, d.position
FROM projects p
CROSS JOIN (
    SELECT 'Backlog' AS name, 'backlog' AS category, 0 AS position
    UNION ALL SELECT 'Listo', 'ready', 1
    UNION ALL SELECT 'En curso', 'active', 2
    UNION ALL SELECT 'Revisión', 'review', 3
    UNION ALL SELECT 'Hecho', 'done', 4
) d;
