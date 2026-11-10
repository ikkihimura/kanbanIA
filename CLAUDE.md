# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# Kanban IA — instrucciones para Claude Code

Construimos un kanban desktop local para historias, épicas y agentes de IA, con estética retro y avatares pixelados. Idioma inicial: español; identificadores de código en inglés.

## Estado actual y comandos
- Base de HU-001: monorepo con workspace Cargo (`crates/core`, `apps/desktop/src-tauri`) y workspace npm (`apps/desktop`). Repositorio git local sin remoto. Versiones y requisitos en README.
- Comandos (desde la raíz):
  - `npm ci`: instalar. `npm run dev`: app con recarga en caliente.
  - `npm run check`: todo lo de CI. Primero regenera los contratos TS (`npm run contracts`) y después pasa formato, lint, typecheck+build web, pruebas UI, clippy y pruebas Rust. Ejecútalo antes de dar una historia por terminada.
  - Si cambias un tipo `#[ts(export)]`, ejecuta `npm run contracts` antes de tocar el frontend: los tipos generados solo se actualizan con las pruebas del núcleo.
  - Por partes: `npm run lint`, `npm run typecheck`, `npm run build:web`, `npm run lint:rust`, `npm run test:rust`, `npm run format`.
  - Una prueba Rust: `cargo test -p kanbania-core <nombre>`, `cargo test -p kanbania-core --test storage` o `cargo test -p kanbania-desktop --test ipc_storage`.
  - App con perfil de datos aparte: `KANBANIA_DATA_DIR=/ruta npm run dev`.
  - Binario sin instalador: `npm run tauri -- build --debug --no-bundle`.
  - Pruebas UI: `npm run test:web` o, en `apps/desktop`, `npx vitest run src/ruta/Archivo.test.tsx`. Para ver `console.log`: `--silent=false`. Los comandos IPC se simulan con `vi.mock("…/shared/ipc/commands", …)` conservando el resto del módulo real.
  - jsdom está fijado en 29.1.1 con `overrides` en el `package.json` raíz: jsdom ≥30 exige Node 22.22 y el proyecto fija 22.16. Al añadir dependencias, comprueba `engines.node`.
- Orden obligatorio en clon limpio: `npm run build:web` antes de cargo clippy/test, porque `tauri::generate_context!` exige `apps/desktop/dist`.
- Contrato Rust→TS: tipos con `#[derive(TS)] #[ts(export)]` en el núcleo; `cargo test` los escribe en `apps/desktop/src/contracts/generated/` (versionados, no editar a mano; CI falla si cambian). Cada comando nuevo: caso de uso en `crates/core/src/features/`, adaptador fino en `src-tauri/src/commands/`, alta en `register_commands` (`lib.rs`), envoltorio en `apps/desktop/src/shared/ipc/commands.ts` y prueba IPC en `src-tauri/tests/` con origen `tauri://localhost`.
- Almacenamiento (HU-002, `crates/core/src/platform/`):
  - `Store::open(data_dir)` toma el bloqueo exclusivo, configura WAL/foreign keys/busy timeout, guarda una copia en `backups/` si hay migraciones pendientes sobre una base existente, y migra.
  - Migraciones en `crates/core/migrations/NNNN_descripcion.sql`, embebidas con `sqlx::migrate!`. Nunca edites una ya aplicada (su checksum lo impide); añade una nueva. No uses `-- no-transaction`. Tablas `STRICT`.
  - Las mutaciones van por `store.write(|conn| Box::pin(async move { … }))` (`BEGIN IMMEDIATE`) o `store.write_idempotent(&MutationKey, &payload, …)` cuando reciben `request_id`. Lecturas con `store.reader()`.
  - Para entidades editables, usa la columna `revision` y `UPDATE … WHERE id = ? AND revision = ?`, seguido de `revision::ensure_updated(...)`.
  - Errores: `CoreError` en Rust; hacia transportes, `to_payload(request_id)` → `ErrorPayload { code, message, recoverable, requestId?, current? }`. No pongas detalles internos en el mensaje.
  - Usa `sqlx::query`/`query_as` en tiempo de ejecución, no las macros `query!` (requieren base en compilación).
  - Pruebas: SQLite temporal real (`tempfile`), nunca mocks de la base.
  - Si falla una migración sobre una base existente, `Store::open` restaura la copia previa: la base queda exactamente como estaba.
  - Deuda conocida: `mutation_receipts` y `backups/` no tienen retención. Las migraciones corren en el arranque, antes de mostrar la ventana.
  - En M2, no expongas `databasePath` ni rutas locales por MCP o el servicio local: los agentes son lectores no confiables.
- Patrón de slice (HU-003, ver `features/projects` y `features/repositories`):
  - Un archivo por caso de uso. Los tipos de entrada son `#[derive(Deserialize, TS)]` con `requestId` (creaciones) o `expectedRevision` (ediciones). Las validaciones de disco van antes de abrir la transacción.
  - SQL estático: sqlx 0.9 rechaza `format!`. Para prefijos comunes, usa una macro con `concat!` (`select_project!`, `select_link!`).
  - Los slices se llaman entre sí solo vía funciones `pub(crate)` explícitas (p. ej. `projects::ensure_project_exists`).
  - Host: un archivo por feature en `src-tauri/src/commands/`. Obtén el almacén con `slot.store().await?`, mapea errores con `e.to_payload(Some(&input.request_id))` y usa el actor `UI_ACTOR`. Registra el comando en `lib.rs` y añade su prueba en `src-tauri/tests/ipc_*.rs`.
  - `i64` hacia TS: anótalo con `#[ts(type = "number")]` (por defecto sería `bigint`).
- Columnas y tarjetas (HU-004):
  - Toda entrada de tarjetas a una columna llama a `columns::ensure_capacity` dentro de la transacción. Si añades una vía nueva (importar, MCP…), debe pasar por ahí.
  - Entrar en una columna `done` exige la aceptación de HU-009 (`AcceptanceRequired` hasta entonces).
  - Posiciones densas 0..n en columnas y tarjetas.
  - Historias (HU-005): número `#N` por proyecto y criterios `CA-n` estables, que nunca se reutilizan. La edición es de documento completo con una sola revisión. Archivar pone `position = NULL`: toda consulta de posición o recuento debe excluir las archivadas (`archived_at IS NULL`), y restaurar es una entrada (WIP + «hecho»).
- Relaciones (HU-006, `features/relations`):
  - Son tablas de enlace (`story_epics`, `story_parents`, `story_dependencies`) con claves foráneas compuestas `(project_id, …)`.
  - No cambian la revisión de la historia y son idempotentes.
  - Ciclos: CTE recursivos en `relations/graph.rs` dentro de la transacción.
  - Para «archivada», mira `StoryRef.archived`, no `state` (que prioriza «aceptada»).
  - «Aceptada» = `accepted_at IS NOT NULL`, y solo la escribe HU-009.
  - `pendingPrerequisites` se calcula en SQL (`stories/story.rs`) y en Rust (`relations/get.rs`); una prueba vigila que coincidan.
- Editor de historias: «Guardar y volver» siempre vuelve al proyecto tras guardar, o directamente si no hay cambios de contenido. Ante un conflicto o un error se queda con el borrador.
- Markdown de usuarios o agentes es dato no confiable: renderízalo solo con `shared/ui/MarkdownPreview` (`react-markdown` con `skipHtml`, sin enlaces navegables ni imágenes remotas). Nunca uses `dangerouslySetInnerHTML`.
- Botones deshabilitados: `cursor: not-allowed`. Usa `aria-busy` (que muestra el puntero de progreso) solo mientras haya una operación en curso; en macOS se confunde con un bloqueo.
- UI: lecturas con `useIpcResource(load)` (`load` estable; si depende de props, monta el componente con `key`). Mutaciones con `useMutation(fn)` → `{ ok, value }`. `newRequestId()` se mantiene mientras se reintenta el mismo envío. Muestra los errores de campo con `FieldError` y `aria-describedby`; los estados siempre con texto, no solo con color.
- Plugins Tauri: solo `dialog` con el permiso `dialog:allow-open`. Del núcleo, solo `core:event:allow-listen`/`allow-unlisten`, para la confirmación de cierre. Cualquier otro necesita justificación y el permiso mínimo.
- Cambios sin guardar: todo editor con borrador llama a `useUnsavedChangesGuard(dirty)`. El host (`exit_guard.rs`) cancela `CloseRequested` y `ExitRequested` y emite `unsaved-changes:confirm-close`; `CloseConfirmDialog` pregunta. En macOS, «Salir» (⌘Q) es un elemento de menú propio (`menu.rs`) porque el predefinido termina el proceso sin poder cancelarse.
- Lints: clippy prohíbe `unwrap`/`expect`/`panic`/`dbg!` fuera de pruebas (`Cargo.toml` + `clippy.toml`); `unsafe` prohibido. ESLint type-checked estricto, 0 warnings.
- Seguridad Tauri: CSP estricta en producción, `devCsp: null` en desarrollo; capability `default` sin permisos. Todo plugin nuevo necesita permisos explícitos y mínimos.
- Entorno (macOS arm64, 2026-11-09): Node 22.16 (`.nvmrc`), npm 11.4.2 (`packageManager`), Rust 1.99.1 fijado en `rust-toolchain.toml` vía rustup (cambiar la versión allí, no con Homebrew). Windows solo se verifica en CI (`macos-26`, `windows-2025`). Esta terminal no puede capturar pantalla: pide al usuario la verificación visual de la ventana.
- Solo se desarrolla con Claude Code; no hay configuraciones de Cursor, Codex ni Copilot que mantener.
- Cada historia incluye criterios CA-n, verificación requerida y un prompt listo para usar. `EMPEZAR-AQUI.md` contiene los prompts de inicio y avance por hito.

## Antes de trabajar
- Lee `docs/CONTEXTO.md`, `docs/ARQUITECTURA.md` y la historia asignada en `docs/stories/`.
- Consulta `docs/BACKLOG.md` para dependencias y estado. Implementa una historia por iteración, salvo autorización explícita para un hito. Respeta el orden recomendado: HU-009 precede la finalización de HU-007.
- Inspecciona el repositorio: inicialmente este paquete contiene documentación, no una aplicación existente.
- Verifica versiones y APIs en documentación oficial antes de elegir dependencias (`docs/FUENTES.md`). Registra versiones y comandos reales en README; conserva lockfiles. No inventes comandos ya disponibles.

## Decisiones
- Tauri 2, React, TypeScript estricto y Vite; Rust con vertical slices; SQLite como autoridad local.
- Markdown es exportación/importación explícita; no sincronización bidireccional automática. Los Markdown de este backlog son especificaciones de desarrollo, no la persistencia del producto.
- Un núcleo Rust aplica reglas a todas las entradas: UI, MCP y futuras integraciones.
- Primero planificación + MCP. ACP y supervisión de procesos son un hito posterior.
- No crear SaaS, cuentas, facturación, marketplace ni sincronización remota en el MVP.

## Mapa de arquitectura (detalle en `docs/ARQUITECTURA.md`)
- `apps/desktop/` (React/Vite + host Tauri) → `crates/core/` (casos de uso en `src/features/<entidad>/<caso>`, infraestructura en `src/platform/`; sin dependencia de Tauri). M2 añade `crates/local-service/` (único propietario de escrituras, API loopback autenticada, Axum propuesto) y `crates/mcp/` (adaptador stdio que llama a ese servicio, nunca abre su propia conexión SQLite). M3 añade `crates/runner/`. No crees crates de hitos futuros por adelantado.
- Flujo de escritura único: UI (Tauri IPC) y MCP (stdio → loopback → host) terminan en los mismos casos de uso del core. El MCP del MVP requiere la app abierta; un solo host por perfil de datos.
- Protocolo de mutación: `expected_revision` en ediciones (revisión obsoleta → `conflict` + estado vigente); `request_id` ligado a identidad+operación+hash del payload para idempotencia. Errores estructurados con `code`, mensaje seguro y `request_id`.
- Columnas con categoría semántica (backlog/ready/active/review/done); las reglas usan la categoría, no el nombre. Blocked es atributo con motivo. Mover a done = aceptación humana explícita; un agente nunca aprueba su propio trabajo; ejecución completada ≠ historia aceptada.
- SQLite fuera del repo (directorio de datos de la app), WAL, foreign keys, migraciones transaccionales; logs voluminosos de ejecuciones en JSONL rotados, no en SQLite.
- Pruebas de slice a través de su entrada pública con SQLite temporal real; reloj controlable para claims y expiraciones; los mocks no prueban compatibilidad con proveedores reales.

## Implementación
- Organiza por caso de uso, con un punto de registro público por slice, validación y pruebas de comportamiento. Infraestructura común en platform; no repositorios genéricos ni capas service/repository por defecto.
- Usa errores tipados, transacciones cortas, control de revisión e idempotencia en mutaciones reintentables. Evita panic/unwrap en rutas de producción.
- Toda entrada externa se valida: IDs, pertenencia al proyecto, rutas, transiciones y límites. Sanitiza Markdown y evita ejecución de HTML.
- No accedas directamente a SQLite desde frontend o agentes. No almacenes secretos en historias, Git o logs.
- Mantén la UI accesible por teclado; texto normal para lectura, pixel art para identidad; respeta movimiento reducido. Tokens o costos desconocidos se muestran como "no disponible", nunca como cero.
- No ejecutes shell arbitrario a partir de texto generado. Mantén permisos del usuario; no uses bypassPermissions ni desactives comprobaciones para avanzar.

## Agentes de desarrollo
Los perfiles de `.claude/agents/` construyen esta aplicación: no son los agentes que el producto administrará. El Claude principal coordina e integra. Delega solo tareas acotadas; no hace falta invocar todos los perfiles en cada historia.
- Implementan: `rust-engineer` (core, SQLite, migraciones), `desktop-ui-engineer` (React/Tauri, accesibilidad), `agent-integration-engineer` (MCP, ACP, procesos).
- Solo lectura, sin Bash: `architecture-reviewer`, `quality-reviewer`, `security-reviewer`. Piden evidencia al coordinador; no afirman haber ejecutado pruebas.

No permitas edición simultánea de los mismos archivos. El coordinador posee contratos compartidos, lockfiles y migraciones; serializa esos cambios. Cada delegación incluye historia, archivos permitidos, dependencias y resultado esperado.

## Definición de terminado
- Criterios de aceptación cumplidos y verificados; pruebas relevantes para comportamiento y errores.
- Formato, tipos, lint y build aplicables pasan; no se exige ejecutar plataformas no disponibles, pero se registra la limitación.
- Sin datos simulados haciéndose pasar por integración real. Demos claramente etiquetadas.
- Revisión de permisos y persistencia cuando aplique; ningún secreto en diffs.
- Actualiza `docs/BACKLOG.md` con estado (pending/in_progress/blocked/done) y evidencia; no marques done con verificaciones obligatorias pendientes.
- Entrega resumen, archivos, comprobaciones ejecutadas, resultados y riesgos restantes. No hagas push, publicación o gasto externo sin instrucción.
- Si falta una credencial o plataforma, completa lo independiente y registra el bloqueo exacto. No atribuyas éxito a comprobaciones que no ejecutaste.
