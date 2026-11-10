-- Proyectos y repositorios Git locales vinculados (HU-003).
-- Las rutas son canónicas (enlaces simbólicos resueltos). Los duplicados se detectan en Rust
-- por identidad de archivo; UNIQUE(project_id, path) es una segunda barrera.
CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_opened_at TEXT
) STRICT;

CREATE INDEX projects_last_opened ON projects (last_opened_at);

CREATE TABLE repository_links (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects (id),
    path TEXT NOT NULL CHECK (length(path) BETWEEN 1 AND 4096),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (project_id, path)
) STRICT;
