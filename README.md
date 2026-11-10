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
```

Contrato de frontera: los tipos se definen en Rust con `#[derive(TS)] #[ts(export)]` y se generan en `apps/desktop/src/contracts/generated/` al ejecutar `cargo test` (ruta configurada en `.cargo/config.toml`). Se versionan en Git y CI falla si están desactualizados. Los nombres de comando se registran en `apps/desktop/src-tauri/src/lib.rs` (`register_commands`).

## Versiones principales

Tauri 2.12.5 (CLI/API 2.12.4), React 19.3.1, Vite 8.3.7, TypeScript 6.0.3, ts-rs 12.0.1, ESLint 10.14.0. Versiones exactas en `package.json` y lockfiles (`package-lock.json`, `Cargo.lock`). Se eligió TypeScript 6.0 (como la plantilla oficial de Tauri) porque typescript-eslint 8.72 soporta `<6.1`.

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
| `backups/pre-migration-vN.sqlite3` | Copia consistente previa a aplicar migraciones sobre una base existente |

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
- **Dependencias**: «A depende de B». Los prerequisitos sin aceptar, incluidos los archivados, se muestran como bloqueo. Por ahora no impiden mover la tarjeta; el bloqueo de ejecución llega con las reclamaciones de agentes (HU-014).
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

## Notas

- Identificador de la app: `dev.ikkidev.kanbania`.
- Iconos actuales: los de la plantilla de Tauri, provisionales hasta definir la marca.
- CSP estricta en producción (`tauri.conf.json`); en desarrollo se desactiva (`devCsp: null`) para la recarga en caliente de Vite.
- `npm audit` reporta avisos moderados de `minimatch` en dependencias de ESLint (solo desarrollo, sin impacto en la app).
- Licencia: pendiente de decisión del propietario.
