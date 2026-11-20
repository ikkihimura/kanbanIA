-- Ruta de `node` para los adaptadores npm (HU-026): al abrir la app desde Finder o el menú
-- Inicio, el `PATH` no incluye el Node de nvm y los scripts `#!/usr/bin/env node` no arrancan.
-- Ruta absoluta del ejecutable `node` tal como la escribió la persona, validada en disco por el
-- núcleo antes de guardarla; su carpeta se antepone al `PATH` de los adaptadores. Nula = usar el
-- `PATH` con el que se abrió la app. La app nunca ejecuta una shell de login para averiguarla.
ALTER TABLE run_settings ADD COLUMN node_path TEXT
    CHECK (node_path IS NULL OR length(node_path) BETWEEN 1 AND 4096);
