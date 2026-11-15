-- Primer adaptador ACP real (HU-019): ejecutor elegido al pedir el inicio y ruta del adaptador
-- de Claude Code en la configuración.
--
-- - `run_start_requests` se reconstruye para añadir `executor` y hacer `scenario` nulo salvo
--   para el proceso de prueba. Ninguna tabla, trigger ni vista la referencia, así que crear la
--   tabla nueva, copiar, borrar la vieja y renombrar es seguro con las claves foráneas activas
--   (la migración es transaccional). Se conservan los ids.
-- - `runs` no se toca (saltaría sus triggers de inmutabilidad).

CREATE TABLE run_start_requests_next (
    id INTEGER PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs (id),
    -- Repositorio de la instantánea (no de `repository_links`: el vínculo puede cambiar).
    repository_id TEXT NOT NULL CHECK (length(repository_id) BETWEEN 1 AND 64),
    -- `test_process`: proceso de prueba incluido (sin IA); `claude_acp`: Claude Code vía ACP.
    -- El núcleo valida los valores (`RunExecutor::from_columns`), como `run_events.kind`: así
    -- un ejecutor nuevo no obliga a reconstruir la tabla.
    executor TEXT NOT NULL CHECK (length(executor) BETWEEN 1 AND 32),
    -- Escenario del proceso de prueba (sin IA); nulo para cualquier otro ejecutor.
    scenario TEXT CHECK (
        scenario IS NULL OR scenario IN ('success', 'fail', 'slow', 'write', 'flood', 'hang')
    ),
    requested_at_ms INTEGER NOT NULL,
    dispatched_at_ms INTEGER,
    voided_at_ms INTEGER,
    CHECK (dispatched_at_ms IS NULL OR voided_at_ms IS NULL),
    CHECK ((executor = 'test_process') = (scenario IS NOT NULL))
) STRICT;

-- Las peticiones anteriores eran todas del proceso de prueba.
INSERT INTO run_start_requests_next (
    id, run_id, repository_id, executor, scenario, requested_at_ms, dispatched_at_ms,
    voided_at_ms
)
SELECT id, run_id, repository_id, 'test_process', scenario, requested_at_ms, dispatched_at_ms,
    voided_at_ms
FROM run_start_requests;

DROP TABLE run_start_requests;

ALTER TABLE run_start_requests_next RENAME TO run_start_requests;

CREATE UNIQUE INDEX run_start_requests_one_live ON run_start_requests (run_id)
    WHERE voided_at_ms IS NULL;

-- Cola FIFO por hora de la petición.
CREATE INDEX run_start_requests_pending ON run_start_requests (requested_at_ms, id)
    WHERE voided_at_ms IS NULL AND dispatched_at_ms IS NULL;

-- Ruta del ejecutable del adaptador de Claude Code (`claude-agent-acp`) tal como la escribió la
-- persona: absoluta y validada en disco por el núcleo antes de guardarla. Nula = no configurado.
-- La app no incluye ni instala el adaptador.
ALTER TABLE run_settings ADD COLUMN claude_acp_path TEXT
    CHECK (claude_acp_path IS NULL OR length(claude_acp_path) BETWEEN 1 AND 4096);
