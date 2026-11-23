-- Retención de datos de ejecuciones (HU-025). Decisiones del propietario: limpieza **manual**
-- por antigüedad (la app propone y la persona confirma; nada se borra solo), ramas opcionales
-- una a una y recibos de idempotencia de 30 días.
--
-- - Antigüedad a partir de la cual un intento terminado se propone para limpiar (días).
ALTER TABLE run_settings ADD COLUMN retention_days INTEGER NOT NULL DEFAULT 30
    CHECK (retention_days BETWEEN 1 AND 3650);

-- - Cuándo se borró la salida (`<data_dir>/runs/<id>/`) del intento. Los intentos y sus eventos
--   se conservan siempre (triggers de 0011).
ALTER TABLE runs ADD COLUMN output_removed_at_ms INTEGER;

-- - Cuándo se borró la rama del intento (siempre con confirmación explícita de la persona).
ALTER TABLE run_worktrees ADD COLUMN branch_deleted_at_ms INTEGER
    CHECK (branch_deleted_at_ms IS NULL OR removed_at_ms IS NOT NULL);

-- - Retención de los recibos: se borran por antigüedad al abrir el almacén.
CREATE INDEX mutation_receipts_created_at ON mutation_receipts (created_at);
