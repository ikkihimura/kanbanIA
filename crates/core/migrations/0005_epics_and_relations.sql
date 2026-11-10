-- Épicas, subtareas y dependencias (HU-006).
--
-- Las relaciones son tablas de enlace con claves foráneas compuestas (project_id, …): la base
-- impide relacionar historias o épicas de proyectos distintos. (Añadirlas como columnas de
-- `stories` exigiría reconstruir la tabla, que ya tiene tablas hijas.) Los ciclos se impiden
-- en el núcleo, dentro de la transacción de escritura.

-- Aceptación vigente. Solo la establece la operación de aceptación de HU-009.
ALTER TABLE stories ADD COLUMN accepted_at TEXT;

-- Destino de las claves foráneas compuestas hacia historias.
CREATE UNIQUE INDEX stories_by_project_and_id ON stories (project_id, id);

CREATE TABLE epics (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects (id),
    -- Número legible por proyecto (E-n); las épicas no se borran en este hito.
    number INTEGER NOT NULL CHECK (number >= 1),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 120),
    description TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 5000),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (project_id, id),
    UNIQUE (project_id, number)
) STRICT;

-- Una historia pertenece como mucho a una épica.
CREATE TABLE story_epics (
    story_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    epic_id TEXT NOT NULL,
    FOREIGN KEY (project_id, story_id) REFERENCES stories (project_id, id),
    FOREIGN KEY (project_id, epic_id) REFERENCES epics (project_id, id)
) STRICT;

CREATE INDEX story_epics_by_epic ON story_epics (epic_id);

-- Subtareas: una historia tiene como mucho un padre.
CREATE TABLE story_parents (
    story_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL,
    CHECK (story_id <> parent_id),
    FOREIGN KEY (project_id, story_id) REFERENCES stories (project_id, id),
    FOREIGN KEY (project_id, parent_id) REFERENCES stories (project_id, id)
) STRICT;

CREATE INDEX story_parents_by_parent ON story_parents (parent_id);

-- `story_id` depende de `depends_on_id` (su prerequisito).
CREATE TABLE story_dependencies (
    story_id TEXT NOT NULL,
    depends_on_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (story_id, depends_on_id),
    CHECK (story_id <> depends_on_id),
    FOREIGN KEY (project_id, story_id) REFERENCES stories (project_id, id),
    FOREIGN KEY (project_id, depends_on_id) REFERENCES stories (project_id, id)
) STRICT, WITHOUT ROWID;

CREATE INDEX story_dependencies_by_prerequisite ON story_dependencies (depends_on_id);
