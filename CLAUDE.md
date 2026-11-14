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
  - Deuda conocida: `mutation_receipts` no tiene retención. Las migraciones corren en el arranque, antes de mostrar la ventana.
  - Copias (HU-011, `platform/storage/backup.rs`): carpeta `backups/<id>/` con base + `manifest.json` (sha256, versión de esquema, exclusiones), creada con `VACUUM INTO`. Restaurar exige el almacén cerrado (`StorageSlot::restore` lo cierra, espera a que se suelten los manejadores, sustituye y reabre; si no abre, deshace). Verifica hashes, integridad, claves foráneas y que el esquema sea idéntico al de las migraciones. Retención: 10 manuales y 10 automáticas. Restaurar revierte también `mutation_receipts` (pero no las revocaciones de credenciales, ver HU-012). En M2, nunca expongas copias ni restauración a agentes (rutas arbitrarias, sustituyen la base).
  - En M2, no expongas `databasePath` ni rutas locales por MCP o el servicio local: los agentes son lectores no confiables.
- Patrón de slice (HU-003, ver `features/projects` y `features/repositories`):
  - Un archivo por caso de uso. Los tipos de entrada son `#[derive(Deserialize, TS)]` con `requestId` (creaciones) o `expectedRevision` (ediciones). Las validaciones de disco van antes de abrir la transacción.
  - SQL estático: sqlx 0.9 rechaza `format!`. Para prefijos comunes, usa una macro con `concat!` (`select_project!`, `select_link!`).
  - Los slices se llaman entre sí solo vía funciones `pub(crate)` explícitas (p. ej. `projects::ensure_project_exists`).
  - Host: un archivo por feature en `src-tauri/src/commands/`. Obtén el almacén con `slot.store().await?`, mapea errores con `e.to_payload(Some(&input.request_id))` y usa el actor `UI_ACTOR`. Registra el comando en `lib.rs` y añade su prueba en `src-tauri/tests/ipc_*.rs`.
  - `i64` hacia TS: anótalo con `#[ts(type = "number")]` (por defecto sería `bigint`).
- Columnas y tarjetas (HU-004):
  - Toda entrada de tarjetas a una columna llama a `columns::ensure_capacity` dentro de la transacción. Si añades una vía nueva (importar, MCP…), debe pasar por ahí.
  - Entrar en una columna `done` solo ocurre al aceptar (`review::accept_story`). `move_story` devuelve `AcceptanceRequired`, y el tablero (HU-007) debe abrir el flujo de aceptación al soltar ahí una tarjeta.
  - Posiciones densas 0..n en columnas y tarjetas.
  - Historias (HU-005): número `#N` por proyecto y criterios `CA-n` estables, que nunca se reutilizan. La edición es de documento completo con una sola revisión. Archivar pone `position = NULL`: toda consulta de posición o recuento debe excluir las archivadas (`archived_at IS NULL`), y restaurar es una entrada (WIP + «hecho»).
- Relaciones (HU-006, `features/relations`):
  - Son tablas de enlace (`story_epics`, `story_parents`, `story_dependencies`) con claves foráneas compuestas `(project_id, …)`.
  - No cambian la revisión de la historia y son idempotentes.
  - Ciclos: CTE recursivos en `relations/graph.rs` dentro de la transacción.
  - Para «archivada», mira `StoryRef.archived`, no `state` (que prioriza «aceptada»).
  - «Aceptada» = `accepted_at IS NOT NULL`. Solo la escriben `review::accept_story` y `reopen_story`.
  - `pendingPrerequisites` se calcula en SQL (`stories/story.rs`) y en Rust (`relations/get.rs`); una prueba vigila que coincidan.
- Equipo (HU-008, `features/team`):
  - El avatar es `AvatarSpec` (índices de piezas) validado en Rust. Los límites se exportan a `contracts/generated/avatarLimits.json` y una prueba de UI los compara con `avatarCatalog.ts`: al añadir una pieza, cambia ambos.
  - Los colores de la UI se aplican con atributos SVG o CSSOM, nunca con `style` en línea (CSP estricta).
  - La asignación de agente vive en `relations::set_story_agent` y no crea actividad.
  - `src/app/contrast.test.ts` comprueba el contraste WCAG de los tokens de color: si cambias la paleta, debe seguir pasando.
- Revisión (HU-009, `features/review`):
  - Las operaciones que dependen de la identidad reciben `platform::actor::Actor`: `Human` desde la app y `Agent { agent_id }` desde MCP (M2).
  - Aceptar, reabrir y devolver llaman a `actor.require_human(..)` en el núcleo; no confíes nunca en que el transporte lo filtre.
  - Una tarjeta aceptada solo vive en columnas `done` (mover, eliminar columna y restaurar lo respetan).
  - Las entregas reportadas por agentes son afirmaciones, no hechos observados.
  - Las URL de referencias no se visitan nunca. Las rutas de red se rechazan y las locales se comprueban fuera del runtime, con límite de tiempo.
  - Para M2 (MCP):
    - El `Actor::Agent` sale de la sesión autenticada, nunca de un campo de la petición.
    - No expongas a agentes operaciones de columnas ni de mover tarjetas, que no reciben `Actor`.
    - No devuelvas a agentes el estado de rutas (`submit_delivery` ya devuelve `Unverified` a un agente).
    - Los resúmenes de agentes son datos no confiables.
    - Los agentes son globales: decide su acotamiento por proyecto.
- Servicio local (HU-012, `features/access` + `crates/local-service`):
  - Dos capas. `service.json` (0600, en el directorio de datos) lleva puerto + token de sesión por arranque. Cada credencial liga agente+proyecto; solo se guarda su sha256. `authenticate_agent` → `AgentSession` (campos privados, solo la construye el núcleo): identidad y proyecto salen de ahí, nunca de la petición. Ninguna ruta acepta `projectId`/`agentId`; cada ruta nueva necesita su prueba de proyecto ajeno.
  - Crear credenciales no usa `requestId` a propósito: el recibo de idempotencia guardaría el token en claro. Crear/revocar exigen `Actor::Human` y nunca se exponen a agentes.
  - El host arranca el servicio solo si posee el almacén (setup, reintentar, restaurar); `HostStore` pide el almacén al `StorageSlot` en cada petición. Guardas: `Host` exacto, cualquier `Origin` → 403, `Content-Length` > 64 KB → 413, sin CORS. Rutas nuevas con cuerpo: usa extractores (`Json`), que aplican `DefaultBodyLimit` también a `chunked`; nunca leas el cuerpo a mano.
  - El cliente (`ServiceClient`) verifica con un reto que el servidor conoce el token antes de enviar la credencial y comprueba `apiVersion`. Con `default-features = false` no enlaza núcleo ni SQLite: así debe usarlo `crates/mcp`.
  - Restaurar copias arrastra las revocaciones vigentes a la base restaurada; si no se pueden leer (perfil nuevo), revoca todas las de la copia (`access/restore.rs`, llamado desde `backup.rs`).
  - Rutas v1: `/v1/session`, `/v1/stories/ready`, `/v1/stories/{id}/context`, `/v1/stories/by-number/{n}/context`, `POST /v1/claims`, `POST /v1/claims/{id}/renew`, `POST /v1/claims/{id}/{blocker,subtasks,result}` (la ruta de verificación `/v1/stories/{id}` de HU-012 se retiró en HU-013 sin subir `API_VERSION`: no se había distribuido ningún cliente).
  - Subir `API_VERSION` (`protocol.rs`) si cambian de forma incompatible las rutas o los tipos que sirven.
  - Los tipos TS del host (p. ej. `LocalServiceStatus`) viven en el núcleo porque solo `kanbania-core --lib` genera contratos.
  - Para M2: clave de idempotencia de agentes por credencial o proyecto (un agente puede tener credenciales en varios proyectos y `agent:<id>` mezclaría recibos); token del agente por variable de entorno o archivo 0600, nunca como argumento; valida ids antes de componer rutas; historias servidas = datos no confiables; no devuelvas `data_dir` ni el `Debug` de `ServiceClient` al agente.
  - Deuda: en Windows el descriptor hereda la ACL del directorio (con `%APPDATA%` es del usuario; con `KANBANIA_DATA_DIR` compartido, no); sin límite de conexiones; `last_used_at` es informativo (si falla, la petición sigue).
- MCP (HU-013, `features/agent_work` + `crates/mcp`):
  - `agent_work` recibe `&AgentSession` y es solo lectura. «Lista» = columnas backlog/ready, sin aceptar ni archivar, sin bloqueo, sin prerequisitos pendientes, sin agente o asignada a quien pregunta (decisión del propietario). La regla es una macro SQL (`agent_work/rule.rs`: `ready_for_agent!`, `claimed_by_other!`) que usan la lista, el contexto (campo `ready`) y `claim_story`; una prueba vigila que lista y contexto coincidan. Desde HU-014, la regla también excluye los reclamos vigentes de otros agentes y `claim_story` la reevalúa en su transacción.
  - `kanbania-mcp` (rmcp 3.9.0): stdout solo del protocolo (`#![deny(clippy::print_stdout)]`), diagnósticos en stderr sin tokens ni rutas. Config por entorno: `KANBANIA_AGENT_TOKEN` (obligatoria, nunca argumento) y `KANBANIA_DATA_DIR` (absoluta, opcional; por defecto replica `app_data_dir` de Tauri con el identificador `dev.ikkidev.kanbania`: si cambias el identificador, cambia `config.rs`).
  - Los tipos de salida de `tools.rs` son espejo del contrato del servicio (el núcleo no deriva `JsonSchema`): al cambiar `ReadyPage`/`StoryContext` en el núcleo, cámbialos también; la prueba e2e de `crates/mcp/tests/mcp.rs` lo detecta. Re-deserializar en tipos propios es además una lista de permitidos: un campo nuevo del servicio no llega al agente sin añadirlo aquí.
  - Valida a mano cada argumento antes de componer rutas (`storyId` hex/`-`, `cursor` `[A-Za-z0-9._-]`): los límites de `schemars` son informativos, rmcp no valida el esquema.
- Reclamos (HU-014, `features/claims`, migración 0009):
  - `story_claims` guarda horas en ms Unix del reloj del núcleo (`platform::clock`, `Store::now_ms`; en pruebas `Store::with_clock(ManualClock)`), nunca `strftime('now')`: toda regla con vencimiento toma la hora de ahí. Para mostrarlas, usa `iso_from_ms!`.
  - Un índice único parcial garantiza un solo reclamo abierto por historia. Un reclamo vencido sigue abierto hasta que algo lo cierra (`claim_story`, liberar, `set_story_agent`, restaurar, aceptar, archivar, entregar o revocar; si ya había vencido, con `end_reason = 'expired'`), así que las consultas comparan `expires_at_ms` con la hora.
  - `ready_for_agent!` (en `agent_work/rule.rs`, pública en el crate) recibe 3 parámetros: agente, ahora, agente. Excluye los reclamos vigentes de otros. `claim_story` la evalúa dentro de su transacción.
  - Reclamar asigna la historia si no tenía agente; vencer no la desasigna, liberar (persona) sí (decisiones del propietario). Quitar o cambiar la asignación (`set_story_agent`) sí cierra el reclamo vigente de otro agente (`claims::end_open_claims`, misma transacción).
  - Tope de `MAX_OPEN_CLAIMS` (5) reclamos vigentes por agente y proyecto → `claim_limit`.
  - Recibos de idempotencia de agentes: `AgentSession::receipt_key` (actor `credential:<id>`). Toda mutación de agentes lo usa; nunca `Actor::receipt_key` (`agent:<id>` mezclaría proyectos).
  - Renovar exige el mismo agente y proyecto, y no resucita un reclamo vencido ni renueva el de una historia archivada o aceptada. Liberar es `Actor::Human`, idempotente y quita la asignación de ese agente. `list_active_claims` y el tope ignoran historias archivadas o aceptadas. Restaurar una copia cierra todos sus reclamos (`close_restored_claims`, junto a la reconciliación de credenciales).
  - La hora se lee dentro de la transacción (clon del `Store` en el cierre), no antes de esperar el bloqueo. Las columnas de texto con `strftime('now')` y las de ms no se comparan entre sí.
  - Rutas `POST /v1/claims` y `POST /v1/claims/{id}/renew` (`Json` con `deny_unknown_fields`; un cuerpo rechazado → 400 o 413). El cliente tiene `post_json`.
  - Motivos de cierre (`end_reason`, migración 0010): `expired`, `released`, `delivered` (entregar), `revoked` (revocar la credencial con la que se reclamó) y `closed` (aceptar o archivar). Ninguno quita la asignación salvo liberar (persona). Uno ya vencido se registra siempre como `expired`.
- Reportes de agentes (HU-015, `features/agent_reports` + slice hoja `features/agent_activity`, migración 0010):
  - `report_blocker`, `create_subtask` y `submit_result` reciben `&AgentSession` y un `claimId`, y pasan por `claims::require_active_claim` (decisión del propietario: reclamo vigente). Reclamo ajeno o de otro proyecto → `not_found`; vencido o cerrado → `claim_ended`. Toda operación nueva de agentes sobre una historia pasa por ahí, usa `AgentSession::receipt_key`, registra su actividad (`agent_activity::record_activity`) y expone `story_ids()` en su resultado para el aviso.
  - `submit_delivery` y `set_story_blocker` (`review`) son solo para personas (`require_human`): los agentes nunca tienen una vía sin reclamo.
  - Entregar crea una entrega `reported` pendiente (nunca acepta), mueve a «revisión» solo si cabe (un WIP lleno no pierde la entrega: `inReview = false`), quita el bloqueo que reportó el propio agente y cierra el reclamo. Con una entrega pendiente la historia no está «lista» (`ready_for_agent!`); devolverla la deja lista otra vez.
  - Subtareas de agentes (decisiones del propietario): columna por defecto, sin agente, prioridad del padre; `MAX_SUBTASKS_PER_STORY` por historia, `MAX_PENDING_AGENT_SUBTASKS` creadas por agentes sin aceptar ni archivar por proyecto, y un solo nivel (procedencia en `agent_created_stories`). Un agente solo cambia o quita su propio bloqueo; si su reclamo termina sin entregar con el bloqueo puesto, lo quita una persona.
  - Las piezas en transacción se comparten con los slices dueños: `stories::insert_story`, `review::{insert_delivery, move_to_review, write_blocker}`, `relations::link_new_child`. Si cambias una regla de crear historias, entregar, bloquear o parentesco, cambia el helper, no una copia.
  - Texto de agentes: `platform::text::has_invisible_formatting` (bidi, ancho cero, etiquetas Unicode…) se aplica a títulos, descripciones, resúmenes, motivos y referencias. Las rutas de entregas `reported` nunca se comprueban en disco (`Unverified`); las rutas con `.`/`..` se rechazan.
  - `agent_activity` guarda lo que reportan (incluidos reclamar y renovar; de renovar, solo la última; como mucho 200 filas por historia; repetir el mismo bloqueo no añade filas). `detail` lo genera el núcleo, nunca texto del agente. La UI lo muestra como «reportado» y «última interacción reportada»: nunca afirmes que el agente sigue trabajando. No se exporta (portabilidad) y al restaurar una copia retrocede con ella.
  - Eventos: el núcleo decide el contenido (`AgentChange` con `story_ids()` de cada resultado); tras confirmar, el servidor llama a `StoreSource::agent_changed` y el host emite `agent-change` a la ventana `main` (`service.rs`). La UI agrupa ráfagas y vuelve a leer del núcleo.
  - Rutas `POST /v1/claims/{id}/blocker`, `/subtasks` y `/result`. `claim_story` comprueba dentro de la transacción que la credencial no se revocó tras autenticar.
  - Deuda: un agente no puede volver a reclamar su historia si una persona la mueve a `active` (la persona la devuelve a backlog/ready); `ClaimEnded` no dice el motivo exacto; restaurar cierra los reclamos de credenciales revocadas como `released`, no `revoked`; ciclos de módulo `access` ↔ `claims` (por `AgentSession`); el cuerpo de una entrega está acotado a 64 KB por el servicio aunque el núcleo admita más.
- Tablero (HU-007, `features/board`): modelo puro en `boardModel.ts`; la vista optimista se liga a la identidad de la lista cargada y el núcleo es la autoridad. Soltar en «hecho» abre `AcceptDialog`, que reutiliza `AcceptForm` y pasa `columnId`/`toIndex` a `accept_story` (aceptar y colocar en una transacción). `dragDropEnabled: false` es necesario para HTML5 DnD en Windows; `useBlockExternalDrops` evita que soltar archivos navegue la ventana.
- Portabilidad (HU-010, `features/portability`): carpeta con `manifest.json` + Markdown con cabecera TOML (`+++`). Importar = vista previa + aplicar en una transacción con la huella del plan; cruce a tres bandas por hash de contenido (fijado por prueba: cambiarlo exige subir `VERSION`). Escribe tablas de otros slices: si añades una regla a crear/editar historias, épicas, columnas o relaciones, aplícala también en `portability/apply.rs` (o en el helper compartido, p. ej. `stories::replace_document`). Exportar/importar exigen `Actor::Human` y nunca se exponen por MCP (rutas arbitrarias).
- Editor de historias: «Guardar y volver» siempre vuelve al proyecto tras guardar, o directamente si no hay cambios de contenido. Ante un conflicto o un error se queda con el borrador.
- Markdown de usuarios o agentes es dato no confiable: renderízalo solo con `shared/ui/MarkdownPreview` (`react-markdown` con `skipHtml`, sin enlaces navegables ni imágenes remotas). Nunca uses `dangerouslySetInnerHTML`.
- Botones deshabilitados: `cursor: not-allowed`. Usa `aria-busy` (que muestra el puntero de progreso) solo mientras haya una operación en curso; en macOS se confunde con un bloqueo.
- UI: lecturas con `useIpcResource(load)` (`load` estable; si depende de props, monta el componente con `key`). Mutaciones con `useMutation(fn)` → `{ ok, value }`. `newRequestId()` se mantiene mientras se reintenta el mismo envío. Muestra los errores de campo con `FieldError` y `aria-describedby`; los estados siempre con texto, no solo con color.
- Plugins Tauri: solo `dialog` con el permiso `dialog:allow-open`. Del núcleo, solo `core:event:allow-listen`/`allow-unlisten`, para la confirmación de cierre y los avisos `agent-change`. Cualquier otro necesita justificación y el permiso mínimo.
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
- `apps/desktop/` (React/Vite + host Tauri) → `crates/core/` (casos de uso en `src/features/<entidad>/<caso>`, infraestructura en `src/platform/`; sin dependencia de Tauri). M2 añade `crates/local-service/` (API loopback autenticada con axum; el propietario de los datos sigue siendo el host) y `crates/mcp/` (adaptador stdio que llama a ese servicio, nunca abre su propia conexión SQLite). M3 añade `crates/runner/`. No crees crates de hitos futuros por adelantado.
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
