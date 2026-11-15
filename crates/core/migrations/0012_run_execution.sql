-- Ejecución de intentos (HU-018): configuración, peticiones de inicio, worktrees y peticiones
-- de cancelación. El ejecutor es un proceso de prueba (sin IA) hasta HU-019.
--
-- - `runs` no se reconstruye nunca (saltaría sus triggers de inmutabilidad): solo se le añaden
--   columnas.
-- - Horas en milisegundos Unix del reloj del núcleo, como `runs` y `story_claims`.
-- - Las rutas de worktrees son absolutas y locales: nunca se exponen a agentes.

-- Configuración de ejecución de todo el perfil: una sola fila, editable por una persona con
-- control de revisión.
CREATE TABLE run_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    -- Procesos a la vez; los que no caben esperan en cola.
    max_concurrency INTEGER NOT NULL DEFAULT 2 CHECK (max_concurrency BETWEEN 1 AND 8),
    -- Tiempo máximo de un proceso; al superarlo se mata su árbol y el intento falla.
    timeout_minutes INTEGER NOT NULL DEFAULT 30 CHECK (timeout_minutes BETWEEN 1 AND 240),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1)
) STRICT;

INSERT INTO run_settings (id) VALUES (1);

-- Petición de inicio de una persona. El intento sigue `queued` hasta que el supervisor tiene
-- hueco y lo despacha (`dispatched_at_ms`). Al arrancar la app, las peticiones sin despachar se
-- anulan (`voided_at_ms`): nada arranca sin una acción en la sesión actual. Como mucho una
-- petición vigente (sin anular) por intento; una anulada se conserva y se puede pedir otra.
CREATE TABLE run_start_requests (
    id INTEGER PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs (id),
    -- Repositorio de la instantánea (no de `repository_links`: el vínculo puede cambiar).
    repository_id TEXT NOT NULL CHECK (length(repository_id) BETWEEN 1 AND 64),
    -- Escenario del proceso de prueba (sin IA).
    scenario TEXT NOT NULL CHECK (scenario IN ('success', 'fail', 'slow', 'write', 'flood', 'hang')),
    requested_at_ms INTEGER NOT NULL,
    dispatched_at_ms INTEGER,
    voided_at_ms INTEGER,
    CHECK (dispatched_at_ms IS NULL OR voided_at_ms IS NULL)
) STRICT;

CREATE UNIQUE INDEX run_start_requests_one_live ON run_start_requests (run_id)
    WHERE voided_at_ms IS NULL;

-- Cola FIFO por hora de la petición.
CREATE INDEX run_start_requests_pending ON run_start_requests (requested_at_ms, id)
    WHERE voided_at_ms IS NULL AND dispatched_at_ms IS NULL;

-- Worktree y rama creados para un intento desde el commit congelado en la instantánea. La rama
-- se conserva siempre; el worktree solo se elimina por una acción explícita (`removal`).
CREATE TABLE run_worktrees (
    run_id TEXT PRIMARY KEY REFERENCES runs (id),
    repository_id TEXT NOT NULL CHECK (length(repository_id) BETWEEN 1 AND 64),
    -- Ruta absoluta bajo `<data_dir>/worktrees/`, calculada por el núcleo.
    path TEXT NOT NULL UNIQUE CHECK (length(path) BETWEEN 1 AND 4096),
    branch TEXT NOT NULL CHECK (length(branch) BETWEEN 1 AND 200),
    base_commit TEXT NOT NULL CHECK (
        length(base_commit) IN (40, 64) AND base_commit NOT GLOB '*[^0-9a-f]*'
    ),
    created_at_ms INTEGER NOT NULL,
    removed_at_ms INTEGER,
    -- `clean`: estaba limpio; `discarded`: la persona descartó sus cambios; `missing`: la carpeta
    -- ya no existía.
    removal TEXT CHECK (removal IN ('clean', 'discarded', 'missing')),
    CHECK ((removed_at_ms IS NULL) = (removal IS NULL)),
    UNIQUE (repository_id, branch)
) STRICT;

-- Petición de cancelación de un intento con proceso: el intento sigue en su estado hasta que el
-- supervisor mata el árbol de procesos y registra `cancelled` con este motivo.
ALTER TABLE runs ADD COLUMN cancel_requested_at_ms INTEGER;
ALTER TABLE runs ADD COLUMN cancel_reason TEXT
    CHECK (cancel_reason IS NULL OR length(cancel_reason) BETWEEN 1 AND 200);
