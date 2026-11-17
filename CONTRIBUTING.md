# Contribuir a Kanban IA

Gracias por tu interés. Kanban IA está en **alfa**: las contribuciones son bienvenidas, pero el alcance de cada versión lo decide el propietario según `docs/BACKLOG.md`.

## Antes de empezar

- Lee `CLAUDE.md` (convenciones y decisiones), `docs/ARQUITECTURA.md` y `docs/CONTEXTO.md`.
- Para cambios grandes, abre antes un issue para acordar el enfoque.
- Vulnerabilidades: no abras un issue público; sigue [SECURITY.md](SECURITY.md).

## Entorno de desarrollo

| Herramienta | Versión |
| --- | --- |
| Node.js | 22.16.0 (`.nvmrc`) |
| npm | 11.4.2 (`packageManager` en `package.json`) |
| Rust | la de `rust-toolchain.toml` (rustup la instala sola) |
| Git | ≥ 2.36 (solo para intentos con worktrees) |

Más los requisitos de sistema de Tauri 2 (Xcode Command Line Tools en macOS; Microsoft C++ Build Tools y WebView2 en Windows). Detalle en el [README](README.md#requisitos).

```sh
npm ci
npm run dev      # app con recarga en caliente
```

En un clon limpio, ejecuta `npm run build:web` antes de `cargo clippy` o `cargo test`: el host Tauri necesita `apps/desktop/dist`.

Para no mezclar pruebas con tus datos: `KANBANIA_DATA_DIR=/ruta/absoluta npm run dev`.

## Antes de abrir un pull request

```sh
npm run check
```

Ejecuta lo mismo que CI: contratos TS, formato, lint, typecheck y build web, pruebas de UI, clippy y pruebas Rust. Debe pasar sin errores ni avisos. Si cambias un tipo Rust con `#[ts(export)]`, ejecuta `npm run contracts` y versiona los archivos generados en `apps/desktop/src/contracts/generated/` (no los edites a mano).

## Convenciones

- **Idioma**: interfaz, mensajes, comentarios y documentación en español; identificadores de código en inglés.
- **Vertical slices**: un caso de uso por archivo en `crates/core/src/features/<slice>/`; infraestructura común en `crates/core/src/platform/`. Sin capas service/repository genéricas. Las reglas viven en el núcleo, no en la UI ni en el transporte.
- **Migraciones**: nunca edites una migración existente; añade una nueva en `crates/core/migrations/`.
- **Pruebas**: con SQLite temporal real, nunca con mocks de la base. Prueba comportamiento y errores a través de la entrada pública del slice.
- **Rust**: sin `unwrap`/`expect`/`panic` fuera de pruebas y sin `unsafe` (clippy lo impide).
- **UI**: accesible por teclado, estados con texto (no solo color), sin `style` en línea ni `dangerouslySetInnerHTML`. El Markdown se muestra solo con `MarkdownPreview`.
- **Dependencias**: versión exacta y verificada; conserva los lockfiles. Si añades, quitas o actualizas una dependencia que llega a la app (crates no dev o paquetes npm de producción de `apps/desktop`), regenera los avisos con `npm run licenses` (necesita `cargo install cargo-about --locked --version 0.9.3 --features cli`) y versiona `THIRD-PARTY-LICENSES.md`. Solo se aceptan las licencias de `about.toml` (y su equivalente npm en `scripts/third-party-licenses.mjs`); añadir otra es decisión del propietario.

## Herramientas y secretos

El equipo del proyecto usa Claude Code para el desarrollo asistido por IA, y el repositorio solo mantiene esa configuración (`CLAUDE.md`, `.claude/`). Puedes usar las herramientas que prefieras, pero no añadas configuraciones de otras herramientas al repositorio.

**Nunca subas secretos**: tokens de agentes (`kbia_…`), claves de proveedores, `service.json`, bases de datos, copias de seguridad ni registros con rutas o correos.

## Licencia

Kanban IA se distribuye bajo **MIT OR Apache-2.0** (archivos `LICENSE-MIT` y `LICENSE-APACHE`). Salvo que indiques expresamente lo contrario, toda contribución que envíes para su inclusión se licencia bajo esos mismos términos, sin condiciones adicionales. No se exige DCO ni CLA.

## Informar de errores

Usa la plantilla «Informe de error» de los issues. Antes de pegar registros, quita tokens, rutas personales y correos.
