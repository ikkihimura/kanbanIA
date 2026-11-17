# Copias de seguridad, restauración y exportación

Kanban IA ofrece dos mecanismos distintos:

- **Copias de seguridad**: la base completa, para recuperar el estado de la app.
- **Exportar/importar Markdown**: la planificación de un proyecto en archivos legibles, para compartirla o moverla. **No es una copia de seguridad.**

## Copias de seguridad

En «Sistema» → «Copias de seguridad»:

- **Crear copia ahora**: copia manual.
- **Restaurar…**: vuelve a una copia de la lista (con confirmación).
- **Guardar en carpeta…**: guarda una copia en otra carpeta (por ejemplo, un disco externo).
- **Restaurar desde una carpeta…**: restaura una copia guardada fuera.

El panel aparece aunque la base no se pueda abrir, para poder recuperarla.

Cada copia es una carpeta en `<datos>/backups/<fecha>-<tipo>/` con la base completa (`kanbania.sqlite3`, coherente aunque haya escrituras en curso) y un `manifest.json` con la versión de esquema, el sha256 y la lista de lo que no incluye. No copies a mano solo el `.sqlite3` con la app abierta: usa el panel.

### Copias automáticas

- **Antes de migrar**: al abrir una versión nueva con cambios de esquema pendientes, la app guarda una copia (`pre-migration`) y después migra. Si una migración falla, la base vuelve a quedar como estaba.
- **Antes de restaurar**: restaurar guarda antes una copia (`pre-restore`) del estado actual para poder deshacerlo.

### Retención

Se conservan las **10 copias manuales** y las **10 automáticas** más recientes en `backups/`. Para conservar una copia más tiempo, usa **Guardar en carpeta…**.

### Restaurar

Restaurar verifica la copia (hashes, integridad, claves foráneas y esquema de esta versión o anterior; una copia antigua se migra al abrir) y sustituye la base sin que nada escriba mientras tanto. Si la base restaurada no abre, la app vuelve sola al estado anterior. No se puede restaurar una copia hecha por una versión más reciente de la app.

Al restaurar:

- Se cierran los reclamos de agentes que contenga la copia.
- Las credenciales de agentes revocadas siguen revocadas (en otro perfil, las de la copia llegan revocadas).
- Los intentos en marcha de la copia quedan como «Interrumpido».

### Qué no incluye una copia

- El contenido de los repositorios vinculados (solo sus rutas).
- La salida de los intentos (registros JSONL en `<datos>/runs/`).
- Los worktrees de los intentos (`<datos>/worktrees/`; sus ramas están en el repositorio).
- Las carpetas exportadas en Markdown.
- La carpeta de Codex de la app (`<datos>/codex-home/`, con tu inicio de sesión de Codex): la copia solo contiene la base. Tras restaurar en otro equipo, vuelve a iniciar sesión (ver [AGENTES.md](AGENTES.md)).

Restaurar una copia anterior puede dejar worktrees en disco sin intento asociado.

## Exportar e importar (Markdown)

### Exportar

En la configuración del proyecto, **Exportar a una carpeta…** crea `kanbania-<proyecto>-<fecha>/` con:

- `manifest.json` (formato `kanbania-package` v1);
- `stories/NNNN-titulo.md` y `epics/E-N-titulo.md`: Markdown con una cabecera TOML entre líneas `+++` (id, número, prioridad, criterios `CA-n`, relaciones) y la descripción como cuerpo.

Incluye historias archivadas. Aceptación, bloqueo y agente asignado van como dato informativo. **No incluye** rutas de repositorios, entregas, actividad de agentes, intentos ni credenciales. Nunca sobrescribe una carpeta existente.

### Importar

En la lista de proyectos: **Importar paquete** → **Elegir paquete…** → revisa la vista previa → **Aplicar importación**.

- Si el proyecto ya existe, se actualiza emparejando por id (reimportar no duplica); si no, se crea con sus columnas.
- Ante cambios en ambos lados, la vista previa muestra un **conflicto** y eliges qué conservar.
- Nunca se borra nada ni se importa una aceptación: lo que estaba en «hecho» entra en revisión. Los bloqueos no se restauran.
- Un paquete inválido no cambia nada.

No hay sincronización automática (ni con Git): exportar e importar son acciones explícitas.

## Paquete de demostración

`examples/demo/demo-recetario/` es un paquete de exportación listo para importar, sin agentes ni estados de IA simulados. Impórtalo en un perfil aparte (`KANBANIA_DATA_DIR`), como explica [INSTALAR.md](INSTALAR.md#primeros-pasos-y-demostración-sin-claves).

## Recomendaciones

- Crea una copia manual antes de actualizar la app o de probar algo con datos que te importen.
- Guarda periódicamente una copia fuera del equipo con **Guardar en carpeta…**.
- Las copias contienen todo tu trabajo: guárdalas donde solo tú tengas acceso y no las adjuntes a informes de error.
