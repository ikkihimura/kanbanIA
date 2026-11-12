# Kanban IA

Kanban desktop local para historias, épicas y agentes de IA. Estado: **M0 / HU-001**, base ejecutable sin funcionalidades de producto todavía. Planificación en [`docs/`](docs/) y [`docs/BACKLOG.md`](docs/BACKLOG.md).

## Requisitos

| Herramienta | Versión verificada | Notas |
|---|---|---|
| Rust | 1.99.1 | Fijado en `rust-toolchain.toml`; rustup lo instala automáticamente. Instalar rustup desde https://rustup.rs |
| Node.js | 22.16.0 (`.nvmrc`) | Vite 8 exige `^20.19.0 \|\| >=22.12.0` |
| npm | 11.4.2 | Gestor del monorepo (workspaces); usar `npm ci` con el lockfile |

Requisitos de sistema de Tauri 2 (https://v2.tauri.app/start/prerequisites/):

- **macOS**: Xcode Command Line Tools (`xcode-select --install`).
- **Windows**: Microsoft C++ Build Tools ("Desktop development with C++") y WebView2 (preinstalado en Windows 10 1803+ y 11).

## Primeros pasos

```sh
npm ci
npm run dev          # abre la ventana de Kanban IA (tauri dev con recarga en caliente)
```

La primera compilación de Rust tarda varios minutos. Abrir `http://localhost:1420` en un navegador muestra la interfaz sin núcleo: el panel indica el error de forma explícita.

## Comandos

| Comando | Qué hace |
|---|---|
| `npm run dev` | App de escritorio en modo desarrollo |
| `npm run build:web` | Typecheck (`tsc`) y build del frontend en `apps/desktop/dist` |
| `npm run typecheck` | Solo typecheck del frontend |
| `npm run lint` | ESLint (type-checked, react-hooks), 0 warnings |
| `npm run test:web` | Pruebas de UI (Vitest + Testing Library + jsdom) |
| `npm run lint:rust` | Clippy con `-D warnings` |
| `npm run test:rust` | Pruebas Rust; también regenera los tipos TS del contrato |
| `npm run format` / `format:check` | Prettier + rustfmt |
| `npm run check` | Todo lo anterior, en el orden de CI |
| `npm run tauri -- build --debug --no-bundle` | Binario de escritorio sin instalador (`target/debug/kanbania-desktop`) |

Pruebas puntuales:

```sh
cargo test -p kanbania-core                                  # un crate
cargo test -p kanbania-desktop --test ipc_app_info           # un archivo de pruebas de integración
cargo test -p kanbania-core combines_host_data               # por nombre de prueba
```

El crate del host (`kanbania-desktop`) necesita `apps/desktop/dist` para compilar: ejecuta `npm run build:web` antes de `cargo test`/`clippy` en un clon limpio.

## Estructura

```text
apps/desktop/                 React 19 + Vite 8 + TypeScript 6 (estricto)
  src/features/<feature>/     vistas y hooks por funcionalidad
  src/shared/ipc/commands.ts  envoltorios tipados de comandos Tauri
  src/contracts/generated/    tipos generados desde Rust (ts-rs). No editar.
  src-tauri/                  host Tauri 2: adapta casos de uso del núcleo a comandos IPC
crates/core/                  casos de uso y reglas, sin dependencia de Tauri
  src/features/<slice>/       vertical slices con entrada pública
crates/local-service/         API local autenticada (solo 127.0.0.1) y su cliente
crates/mcp/                   servidor MCP por stdio (`kanbania-mcp`) para agentes externos
```

Contrato de frontera: los tipos se definen en Rust con `#[derive(TS)] #[ts(export)]` y se generan en `apps/desktop/src/contracts/generated/` al ejecutar `cargo test` (ruta configurada en `.cargo/config.toml`). Se versionan en Git y CI falla si están desactualizados. Los nombres de comando se registran en `apps/desktop/src-tauri/src/lib.rs` (`register_commands`).

## Versiones principales

Tauri 2.12.5 (CLI/API 2.12.4), React 19.3.1, Vite 8.3.7, TypeScript 6.0.3, ts-rs 12.0.1, axum 0.8.9 + hyper 1.12 (servicio local), rmcp 3.9.0 (MCP), ESLint 10.14.0. Versiones exactas en `package.json` y lockfiles (`package-lock.json`, `Cargo.lock`). Se eligió TypeScript 6.0 (como la plantilla oficial de Tauri) porque typescript-eslint 8.72 soporta `<6.1`.

## CI

`.github/workflows/ci.yml` ejecuta en `macos-26` y `windows-2025`: formato, lint, typecheck/build web, clippy, pruebas, verificación de tipos generados y build debug del host. Acciones fijadas por SHA. No requiere credenciales.

## Datos locales

La aplicación guarda todo en SQLite dentro del directorio de datos del sistema:

- macOS: `~/Library/Application Support/dev.ikkidev.kanbania/`
- Windows: `%APPDATA%\dev.ikkidev.kanbania\`

Para usar otro perfil (pruebas, depuración): `KANBANIA_DATA_DIR=/ruta npm run dev`.

| Archivo | Uso |
|---|---|
| `kanbania.sqlite3` (+ `-wal`, `-shm`) | Base de datos en modo WAL. No copies solo el `.sqlite3` con la app abierta |
| `kanbania.lock` | Bloqueo exclusivo: un solo proceso escritor por directorio. El sistema lo libera si la app termina |
| `service.json` | Mientras la app está abierta: puerto del servicio local y token de sesión (solo legible por el usuario; se borra al salir) |
| `backups/<fecha>-<tipo>/` | Copias de seguridad: `kanbania.sqlite3` + `manifest.json`. Manuales, antes de migrar y antes de restaurar (ver «Copias de seguridad») |

Si otra instancia ya usa los datos, la nueva ventana muestra el error `storage_locked` con opción de reintentar. Una migración fallida se revierte por completo y la app muestra `storage_migration_failed` sin modificar los datos. Una base creada por una versión más reciente muestra `storage_newer_schema` y no se toca. La base debe estar en disco local: no se admite una carpeta de red.

## Proyectos y repositorios

Un proyecto agrupa trabajo y puede vincular varios repositorios Git locales. Al iniciar, la app reabre el último proyecto abierto.

Política de rutas de repositorio (`crates/core/src/features/repositories/git_path.rs`):

- Ruta absoluta a la **raíz del working tree**: carpeta con `.git/` (que contiene `HEAD`) o con un archivo `.git` que empieza por `gitdir:` (worktrees y submódulos). No se admiten subcarpetas ni repositorios bare.
- Se resuelven los enlaces simbólicos y se guarda la ruta real.
- Un repositorio no puede vincularse dos veces al mismo proyecto. La comparación es por identidad de carpeta, así que otra grafía (mayúsculas, Unicode, `..`, enlace simbólico) también cuenta como duplicado. Sí puede pertenecer a varios proyectos.
- Kanban IA **nunca escribe** en el repositorio: solo lee metadatos y el inicio del archivo `.git`.
- El estado (`available`, `missing`, `not_a_repository`, `inaccessible`) se calcula al mostrarlo. Si se movió, **Reubicar** lo apunta a la nueva carpeta; **Desvincular** borra solo el enlace.

## Columnas y límites WIP

Cada proyecto empieza con el flujo Backlog · Listo · En curso · Revisión · Hecho. Las columnas se pueden crear, renombrar, reordenar y eliminar.

- **Tipo de etapa** (`backlog`, `ready`, `active`, `review`, `done`): las reglas dependen del tipo, nunca del nombre visible.
- **Límite WIP** opcional (1–999). Se aplica en el núcleo a toda entrada de tarjetas: crear, mover y destino al eliminar otra columna. Bajarlo por debajo de las tarjetas actuales está permitido: solo bloquea nuevas entradas, y la UI marca la columna como excedida.
- **Columnas «hecho»**: solo reciben trabajo aceptado. Solo se entra en ellas aceptando la historia (sección «Entregas, aceptación y atención»). Una columna con tarjetas no puede cambiar a ese tipo ni dejar de serlo.
- **Eliminar** una columna con tarjetas exige elegir destino. Las tarjetas se mueven al final de ese destino, en el mismo orden y en una sola transacción. Siempre queda al menos una columna que no es «hecho».

## Historias

Cada historia tiene un número por proyecto (`#N`) que no se reutiliza, título obligatorio (≤200), descripción en Markdown (≤20 000 caracteres), prioridad (urgente, alta, media o baja) y criterios de aceptación (≤50, de ≤500 caracteres cada uno).

- **Criterios `CA-n`**: el número es estable. Borrar CA-2 no renumera CA-3, y los números no se reutilizan.
- **Edición**: título, descripción, prioridad y criterios se guardan juntos con una sola revisión. Si otra edición guardó antes, el editor muestra la versión guardada y **conserva tu borrador**. Puedes conservar tu versión o descartarla.
- **Vista previa segura**: no se interpreta HTML crudo (`<script>`, `onerror`…), los enlaces no navegan y las imágenes remotas no se cargan.
- **Archivar** saca la historia del tablero y del límite WIP sin borrar nada. **Restaurar** la devuelve al final de la columna elegida y aplica el WIP.
- **Cambios sin guardar**: cerrar la ventana (botón, ⌘W, Alt+F4) o salir (⌘Q) con un borrador pendiente muestra un aviso para seguir editando o salir sin guardar.

## Épicas, subtareas y dependencias

- **Épicas** (`E-n`): agrupan historias. Su **avance** es aceptadas ÷ historias no archivadas de la épica. Sin historias se muestra «Sin historias», nunca 0 % ni 100 %. Las historias se aceptan desde el panel «Revisión» del editor.
- **Subtareas**: una historia puede colgar de otra del mismo proyecto.
- **Dependencias**: «A depende de B». Los prerequisitos sin aceptar, incluidos los archivados, se muestran como bloqueo. Por ahora no impiden mover la tarjeta; un agente no puede reclamar una historia con prerequisitos pendientes (HU-014).
- Se rechazan los **ciclos** directos e indirectos (subtareas y dependencias) y las relaciones entre proyectos distintos.
- Las relaciones se guardan al momento desde el editor, sin interferir con el borrador del texto. «Guardar y volver» guarda el contenido y vuelve al proyecto.

## Equipo: usuario y agentes

- **Tú** (usuario local) y los **agentes** son entidades distintas. Los agentes son del equipo, no de un proyecto: el acotamiento por proyecto de sus sesiones llegará con MCP (M2).
- Cada agente (`A-n`) tiene un nombre único (su identidad), rol, proveedor y modelo (texto informativo, no conecta con ningún servicio) y capacidades de un conjunto fijo: planificar, implementar, revisar, probar y documentar. **Cambiar de modelo conserva al mismo agente y su avatar.**
- **Avatar por piezas**: sprites originales de 16×16 dibujados por código (cabeza humana o robot, ojos, boca, pelo o tocado, accesorio) con una paleta limitada de colores con nombre. Se edita con teclado.
- **Asignar** un agente a una historia solo registra la asignación: no inicia ejecuciones ni muestra actividad.

## Entregas, aceptación y atención

- **Entregas**: resumen y referencias. Las referencias son URL http(s), que la app **nunca visita** y muestra como «no verificada», o rutas locales, cuyo acceso se comprueba al mostrarlas: accesible / no encontrada / sin acceso. Una entrega es *manual* si la registra la persona y *reportada* si la envía un agente (M2). «Enviar a revisión» mueve la tarjeta a la columna de revisión **sin marcarla como hecha**. Una entrega pendiente se puede devolver con un motivo.
- **Aceptar** solo lo hace una persona: los agentes reciben `forbidden`. Exige al menos un criterio y que todos se marquen como cumplidos, y queda registrado en el historial junto con los criterios. La tarjeta pasa a la primera columna «hecho», o a la columna «hecho» y la posición donde se soltó en el tablero, en la misma operación y con el límite WIP de esa columna. Sin columna «hecho» no se puede aceptar.
- **Reabrir** pide un motivo y devuelve la tarjeta a revisión, o a la columna elegida. El historial se conserva. Una tarjeta aceptada solo sale de «hecho» reabriéndola.
- **Bloqueo**: es un motivo marcado en la historia, no una columna.
- **Atención (N)**, en cada proyecto: una fila por historia con todos sus motivos: bloqueo, entregas pendientes y prerequisitos sin aceptar. Los prerequisitos solo cuentan en las columnas listo, en curso y revisión.

## Tablero

- Pestañas **Tablero** y **Configuración** en cada proyecto (Atención queda encima de ambas). Las tarjetas muestran número, título, prioridad en texto, épica, motivo de bloqueo o prerequisitos pendientes, agente asignado y «Aceptada».
- **Filtros** por texto (título o `#número`) y épica. **Carriles** por épica o agente: solo agrupan, y mover cambia la columna, nunca la épica ni el agente.
- **Mover**: arrastrar y soltar, o con el foco en una tarjeta **Alt + flechas** (←/→ columna contigua, ↑/↓ junto a la vecina visible). El movimiento se ve al instante y vuelve atrás si el núcleo lo rechaza (WIP, conflicto…); ante un conflicto el tablero se recarga.
- Soltar en una columna «hecho» abre la **aceptación**: la tarjeta no se mueve hasta aceptarla; cancelar la deja donde estaba. Una aceptada solo sale de «hecho» reabriéndola.
- Soltar archivos o enlaces externos en la ventana no hace nada (evita perder borradores navegando fuera de la app).
- Medición (macOS arm64, sin SLA): núcleo con 300 tarjetas, crear 123 ms, listar 3,4 ms, 50 movimientos 54 ms (`crates/core/tests/board_load.rs`); render de 300 tarjetas en jsdom ≈ 400 ms.

## Exportar e importar (Markdown)

- **Exportar** (Configuración del proyecto): crea `kanbania-<proyecto>-<fecha>/` con `manifest.json` (formato `kanbania-package` v1), `stories/NNNN-titulo.md` y `epics/E-N-titulo.md`. Cada archivo es Markdown con una cabecera TOML entre líneas `+++` (id, número, prioridad, criterios `CA-n`, relaciones) y la descripción como cuerpo. Incluye archivadas. El estado de revisión (aceptación, bloqueo, agente asignado) va como dato informativo. No incluye rutas de repositorios, entregas ni recibos. Nunca sobrescribe una carpeta existente.
- **Importar** (lista de proyectos): elige la carpeta y revisa la vista previa antes de aplicar. Si el proyecto existe se actualiza emparejando por id (reimportar no duplica); si no, se crea con sus columnas.
  - Cada historia/épica: se crea, se actualiza (solo cambió el paquete), sin cambios, se conserva lo local (solo cambió lo local) o **conflicto** (cambiaron ambos: eliges cuál conservar).
  - Nunca se borra nada ni se importa una aceptación: lo que estaba en «hecho» entra en revisión. Las historias existentes conservan columna y posición; las archivadas aquí no se modifican.
  - El límite WIP se respeta; al restaurar un proyecto nuevo cuyo tablero ya lo superaba, se sube lo justo y se avisa.
  - Se rechazan rutas fuera de la carpeta, enlaces simbólicos, ids duplicados o de otro proyecto, ciclos y formatos de otra versión. Un paquete inválido no cambia nada.
- No hay sincronización automática (ni con Git): exportar e importar son acciones explícitas.

## Copias de seguridad

- En «Sistema» → «Copias de seguridad»: **Crear copia ahora**, **Restaurar…** (con confirmación) y **Guardar en carpeta…** (p. ej. un disco externo); también **Restaurar desde una carpeta…**. El panel aparece aunque la base no se pueda abrir, para recuperarla.
- Cada copia es una carpeta con la base completa, coherente aunque haya escrituras en curso (`VACUUM INTO`), y un `manifest.json` con la versión de esquema, sha256 y lo que **no** incluye: el contenido de los repositorios (solo sus rutas), los registros de ejecución de agentes (llegarán en M3) y las exportaciones Markdown (que no son una copia de seguridad).
- Restaurar verifica la copia (hashes, integridad, claves foráneas, esquema idéntico al de esta versión o anterior; una copia antigua se migra al abrir), guarda antes una copia automática del estado actual para deshacer y sustituye la base sin que nada escriba mientras tanto. Si la base restaurada no abre, se vuelve sola al estado anterior.
- Se conservan las 10 copias manuales y las 10 automáticas más recientes en `backups/`.

## Servicio local y acceso de agentes

- Con la app abierta, el host escucha solo en `127.0.0.1` con un puerto dinámico. Nada fuera de este equipo puede conectarse; no hay CORS y se rechazan peticiones de navegadores (`Origin`) y hosts inesperados.
- Para que un agente (la integración MCP llega en HU-013) use un proyecto: en la configuración del proyecto, «Acceso de agentes» → elige el agente → **Crear credencial**. El token se muestra **una sola vez**; guárdalo solo en la configuración de la integración. Revocar corta el acceso en la siguiente petición.
- Cada petición necesita el token de sesión de `service.json` y la credencial; el agente y el proyecto salen de la credencial. Restaurar una copia no reactiva credenciales revocadas (en otro perfil, las de la copia llegan revocadas).
- Con la app cerrada, las integraciones fallan con «Kanban IA no está abierto» y nunca abren la base. Una segunda instancia no arranca otro servicio.
- Rutas de la API v1 (todas autenticadas y acotadas al proyecto de la credencial): `GET /v1/session`, `GET /v1/stories/ready?cursor=&limit=` y `GET /v1/stories/{id}/context` o `/v1/stories/by-number/{n}/context`.
- Limitación: en Windows `service.json` hereda los permisos del directorio de datos; no uses `KANBANIA_DATA_DIR` en una carpeta compartida con otros usuarios.

## Agentes por MCP (`kanbania-mcp`)

Servidor MCP por stdio (SDK oficial `rmcp` 3.9.0) con cuatro herramientas:

- `list_ready_stories` (`cursor?`, `limit?` 1-50): historias en «Backlog» o «Listo», sin aceptar ni archivar, sin bloqueo, sin prerequisitos pendientes, sin agente o asignadas al propio agente, y sin reclamo vigente de otro agente. Primero las de «Listo», después por prioridad y posición. Paginada con `nextCursor`.
- `get_story_context` (`storyId` o `number`): descripción, criterios CA-n, épica, padre, subtareas, prerequisitos, asignación, si está lista y el reclamo vigente del agente (`yourClaim`) o si la tiene otro (`claimedByOther`).
- `claim_story` (`requestId`, `storyId` o `number`): reclama una historia lista para que ningún otro agente la tome. Dura 30 minutos y la asigna al agente si no tenía a nadie. Como mucho 5 reclamos vigentes por agente y proyecto. Errores: `[claimed]` (la tiene otro), `[not_ready]` y `[claim_limit]`.
- `renew_claim` (`claimId`, `requestId`): extiende el reclamo 30 minutos desde ahora. `[claim_ended]` si venció, una persona lo liberó o reasignó la historia, o la historia se archivó o aceptó.

`requestId` lo elige el agente (p. ej. un UUID). Reintentar con el mismo devuelve el resultado original sin repetir el efecto, y con otros datos responde `[idempotency_conflict]`.

En la app, la tarjeta muestra «Reclamada por…» y el editor de la historia permite liberar el reclamo; liberar quita también la asignación y la historia queda libre para cualquier agente (para impedir que la retomen, bloquéala o muévela fuera de Backlog/Listo). Quitar o cambiar la asignación también cierra el reclamo vigente. Si el reclamo vence sin más, la asignación se queda: solo ese agente puede volver a reclamarla. Restaurar una copia cierra los reclamos que contenga.

El agente solo ve el proyecto de su credencial. Necesita la app abierta: si está cerrada, cada llamada responde «Kanban IA no está abierto…». El proceso MCP nunca abre la base.

Configuración (requiere `cargo build --release -p kanbania-mcp`; el binario queda en `target/release/kanbania-mcp`):

1. En la app: configuración del proyecto → «Acceso de agentes» → crea una credencial para el agente y cópiala.
2. Regístralo en el cliente MCP con la credencial en `env` (nunca como argumento de la orden). En Claude Code, con alcance local o de usuario (fuera del repositorio):

```sh
claude mcp add kanbania --scope local -e KANBANIA_AGENT_TOKEN=kbia_… -- /ruta/a/target/release/kanbania-mcp
```

Si prefieres un `.mcp.json` de proyecto (que se versiona), **no pongas el token en él**: usa la expansión de variables y define `KANBANIA_AGENT_TOKEN` en tu entorno.

```json
{
  "mcpServers": {
    "kanbania": {
      "command": "/ruta/a/target/release/kanbania-mcp",
      "env": { "KANBANIA_AGENT_TOKEN": "${KANBANIA_AGENT_TOKEN}" }
    }
  }
}
```

`KANBANIA_DATA_DIR` es opcional (perfiles de prueba; debe ser una ruta absoluta); por defecto se usa el directorio de datos de la app. stdout es solo del protocolo; los diagnósticos van a stderr sin tokens.

- Compila el binario que vayas a usar con `-p kanbania-mcp`: así no enlaza el núcleo ni SQLite (`cargo tree -p kanbania-mcp -e normal -i libsqlite3-sys` debe salir vacío). En una compilación de todo el workspace, las features se unifican y podría enlazarlos, aunque el código del MCP nunca abre la base.
- El contenido de las historias (títulos, descripciones, criterios, motivos de bloqueo, que pueden venir de otros agentes) se sirve tal cual: la integración no lo filtra. Es dato, no instrucciones; el cliente MCP debe seguir pidiendo permiso para herramientas con efectos.
- `list_ready_stories` pagina hasta 10 000 historias; si hay más, `truncated` es `true`.

## Notas

- Identificador de la app: `dev.ikkidev.kanbania`.
- Iconos actuales: los de la plantilla de Tauri, provisionales hasta definir la marca.
- CSP estricta en producción (`tauri.conf.json`); en desarrollo se desactiva (`devCsp: null`) para la recarga en caliente de Vite.
- `npm audit` reporta avisos moderados de `minimatch` en dependencias de ESLint (solo desarrollo, sin impacto en la app).
- Licencia: pendiente de decisión del propietario.
