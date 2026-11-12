-- Reclamos de historias por agentes (HU-014).
-- Un reclamo es exclusivo por historia y caduca: el agente lo renueva o deja de valer. Las
-- horas se guardan en milisegundos Unix (UTC) que da el reloj del núcleo, no `strftime('now')`,
-- para comparar sin redondeos y poder controlar el tiempo en las pruebas.
-- Un reclamo vencido sigue abierto (`ended_at` nulo) hasta que alguien lo cierra al tocar la
-- historia; las reglas lo tratan como inexistente comparando `expires_at_ms` con la hora.

CREATE TABLE story_claims (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    story_id TEXT NOT NULL,
    agent_id TEXT NOT NULL REFERENCES agents (id),
    -- Credencial con la que se reclamó (trazabilidad; renovar exige el mismo agente).
    credential_id TEXT NOT NULL REFERENCES agent_credentials (id),
    claimed_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    renewed_at_ms INTEGER,
    ended_at_ms INTEGER,
    -- `expired`: venció sin renovar; `released`: lo liberó una persona.
    end_reason TEXT CHECK (end_reason IN ('expired', 'released')),
    CHECK (expires_at_ms > claimed_at_ms),
    CHECK ((ended_at_ms IS NULL) = (end_reason IS NULL)),
    FOREIGN KEY (project_id, story_id) REFERENCES stories (project_id, id)
) STRICT;

-- Como mucho un reclamo abierto por historia: dos reclamaciones simultáneas no pueden ganar
-- ambas aunque fallara la comprobación previa.
CREATE UNIQUE INDEX story_claims_one_open ON story_claims (story_id) WHERE ended_at_ms IS NULL;

CREATE INDEX story_claims_open_by_project ON story_claims (project_id) WHERE ended_at_ms IS NULL;
