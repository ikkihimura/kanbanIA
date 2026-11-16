-- Segundo adaptador ACP (HU-021): Codex vía `@agentclientprotocol/codex-acp`.
--
-- - `run_start_requests.executor` ya admite cualquier valor corto validado por el núcleo
--   (`RunExecutor::from_columns`): `codex_acp` no necesita reconstruir la tabla.
-- - Ruta del ejecutable del adaptador de Codex (`codex-acp`) tal como la escribió la persona:
--   absoluta y validada en disco por el núcleo antes de guardarla. Nula = no configurado. La app
--   no incluye ni instala el adaptador.
ALTER TABLE run_settings ADD COLUMN codex_acp_path TEXT
    CHECK (codex_acp_path IS NULL OR length(codex_acp_path) BETWEEN 1 AND 4096);
