-- Ejecuciones (intentos) de una historia por un agente (HU-017). Solo el modelo: todavía no
-- hay ejecutor (HU-018), así que un intento nace en `queued` y se queda ahí hasta cancelarse.
--
-- - Cada intento guarda una instantánea JSON inmutable de lo que recibiría el agente
--   (historia, criterios, perfil del agente, referencias de contexto e instrucciones) y su
--   SHA-256. Un trigger aborta cualquier UPDATE de esas columnas: ni el núcleo puede cambiarlas.
-- - Como mucho un intento activo (no terminal) por historia: índice único parcial.
-- - `run_events` es el registro de eventos de estado y uso, con secuencia 1..n sin huecos por
--   intento; solo se añaden filas. La salida voluminosa de procesos irá a JSONL (HU-018).
-- - Horas en milisegundos Unix del reloj del núcleo, como `story_claims`.
-- - Uso y coste son nulos mientras el proveedor no los informe: nulo = no disponible, nunca 0.

CREATE TABLE runs (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    story_id TEXT NOT NULL,
    -- «Intento #n» dentro de la historia; nunca se reutiliza (los intentos no se borran).
    number INTEGER NOT NULL CHECK (number >= 1),
    agent_id TEXT NOT NULL REFERENCES agents (id),
    state TEXT NOT NULL CHECK (state IN (
        'queued', 'preparing', 'running', 'waiting_approval',
        'completed', 'failed', 'cancelled', 'interrupted'
    )),
    snapshot TEXT NOT NULL CHECK (json_valid(snapshot)),
    snapshot_sha256 TEXT NOT NULL CHECK (length(snapshot_sha256) = 64),
    -- Secuencia del último evento aplicado (el primero, `queued`, es el 1).
    last_sequence INTEGER NOT NULL CHECK (last_sequence >= 1),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER,
    -- Motivo corto del final, generado por el núcleo (o por el supervisor, validado).
    end_reason TEXT CHECK (end_reason IS NULL OR length(end_reason) BETWEEN 1 AND 200),
    input_tokens INTEGER CHECK (input_tokens IS NULL OR input_tokens >= 0),
    output_tokens INTEGER CHECK (output_tokens IS NULL OR output_tokens >= 0),
    -- Millonésimas de dólar estadounidense.
    cost_usd_micros INTEGER CHECK (cost_usd_micros IS NULL OR cost_usd_micros >= 0),
    -- Terminado ⇔ estado terminal.
    CHECK ((ended_at_ms IS NULL) = (state IN ('queued', 'preparing', 'running', 'waiting_approval'))),
    CHECK (end_reason IS NULL OR ended_at_ms IS NOT NULL),
    UNIQUE (story_id, number),
    FOREIGN KEY (project_id, story_id) REFERENCES stories (project_id, id)
) STRICT;

-- Un solo intento activo por historia, aunque fallara la comprobación previa.
CREATE UNIQUE INDEX runs_one_active_per_story ON runs (story_id)
    WHERE state IN ('queued', 'preparing', 'running', 'waiting_approval');

CREATE INDEX runs_by_project ON runs (project_id, story_id);

-- La instantánea es inmutable: cualquier UPDATE que la nombre se aborta.
CREATE TRIGGER runs_snapshot_immutable
BEFORE UPDATE OF snapshot, snapshot_sha256 ON runs
BEGIN
    SELECT RAISE(ABORT, 'run snapshot is immutable');
END;

-- Los intentos se conservan.
CREATE TRIGGER runs_never_deleted
BEFORE DELETE ON runs
BEGIN
    SELECT RAISE(ABORT, 'runs are never deleted');
END;

CREATE TABLE run_events (
    run_id TEXT NOT NULL REFERENCES runs (id),
    sequence INTEGER NOT NULL CHECK (sequence >= 1),
    -- Tipo de evento; el núcleo valida los valores (`state_changed`, `usage_reported`…).
    kind TEXT NOT NULL CHECK (length(kind) BETWEEN 1 AND 40),
    -- JSON generado por el núcleo a partir de un evento validado; como mucho 8 KB.
    payload TEXT NOT NULL CHECK (json_valid(payload) AND length(CAST(payload AS BLOB)) <= 8192),
    at_ms INTEGER NOT NULL,
    PRIMARY KEY (run_id, sequence)
) STRICT, WITHOUT ROWID;

-- Registro solo de adición.
CREATE TRIGGER run_events_append_only_update
BEFORE UPDATE ON run_events
BEGIN
    SELECT RAISE(ABORT, 'run events are append-only');
END;

CREATE TRIGGER run_events_append_only_delete
BEFORE DELETE ON run_events
BEGIN
    SELECT RAISE(ABORT, 'run events are append-only');
END;
