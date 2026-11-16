-- HU-020: peticiones de permiso de un agente durante un intento y la decisión de una persona.
--
-- Cada fila es una petición `session/request_permission` del agente (ACP). Solo una persona
-- decide (`allowed`/`rejected`), una sola vez, antes de `expires_at_ms` y con el intento activo
-- y sin petición de cancelación. El supervisor entrega la decisión al agente (`delivered_at_ms`)
-- en una transacción que vuelve a comprobarlo: una decisión nunca se aplica tras cancelar.
-- `scope` (comando o rutas) y `tool_title` son datos del agente, recortados y con las formas de
-- secreto conocidas ocultas: texto plano en la UI, nunca para agentes.
CREATE TABLE run_approvals (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs (id),
    -- «Permiso #n» dentro del intento; nunca se reutiliza.
    sequence INTEGER NOT NULL CHECK (sequence >= 1),
    tool_title TEXT NOT NULL CHECK (length(tool_title) BETWEEN 1 AND 300),
    tool_kind TEXT CHECK (tool_kind IS NULL OR length(tool_kind) BETWEEN 1 AND 32),
    scope TEXT CHECK (scope IS NULL OR length(scope) BETWEEN 1 AND 2000),
    -- El agente ofreció «permitir una vez» (si no, solo se puede rechazar).
    can_allow INTEGER NOT NULL CHECK (can_allow IN (0, 1)),
    state TEXT NOT NULL CHECK (state IN (
        'pending', 'allowed', 'rejected', 'expired', 'cancelled', 'withdrawn'
    )),
    requested_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms > requested_at_ms),
    -- Hora de la decisión o del cierre (persona, caducidad, cancelación o retirada).
    decided_at_ms INTEGER,
    -- Hora en que el supervisor respondió al agente.
    delivered_at_ms INTEGER,
    end_reason TEXT CHECK (end_reason IS NULL OR length(end_reason) BETWEEN 1 AND 200),
    UNIQUE (run_id, sequence),
    CHECK ((state = 'pending') = (decided_at_ms IS NULL)),
    CHECK (delivered_at_ms IS NULL OR state IN ('allowed', 'rejected', 'expired')),
    CHECK (state <> 'allowed' OR can_allow = 1)
) STRICT;

CREATE INDEX run_approvals_pending ON run_approvals (run_id) WHERE state = 'pending';

-- Notificaciones del sistema al pedir permiso (decisión del propietario: desactivadas por
-- defecto).
ALTER TABLE run_settings ADD COLUMN approval_notifications INTEGER NOT NULL DEFAULT 0
    CHECK (approval_notifications IN (0, 1));
