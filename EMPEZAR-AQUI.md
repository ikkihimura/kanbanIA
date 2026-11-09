# Paquete de ejecución para Claude Code

Este paquete contiene planificación e instrucciones listas para implementar, no código de aplicación. Se preparó el 9 de noviembre de 2026.

## Archivos
- `CLAUDE.md`: instrucciones breves que Claude Code carga al trabajar en esta carpeta.
- `docs/CONTEXTO.md`: visión, alcance y decisiones de producto.
- `docs/ARQUITECTURA.md`: estructura, reglas, contratos y almacenamiento.
- `docs/BACKLOG.md`: 22 historias por hitos, dependencias, estado y evidencia.
- `docs/stories/HU-001.md` a `HU-022.md`: historias con criterios verificables y prompt individual.
- `.claude/agents/`: seis perfiles especializados para construir y revisar la aplicación.
- `docs/FUENTES.md`: documentación consultada y referencias a verificar al implementar.

## Uso
1. Abre Claude Code en esta carpeta. Si lo copias a otro proyecto, conserva `.claude/`, que es una carpeta oculta, y revisa cualquier CLAUDE.md existente antes de combinarlo.
2. Inicia una sesión nueva para que cargue los perfiles. Consulta `/agents` para comprobar que aparecen.
3. Pega el prompt de inicio de abajo. Trabaja primero por historia para ajustar el producto con evidencia.
4. Al finalizar, verifica los criterios y la evidencia en BACKLOG. Continúa con la siguiente dependencia satisfecha.

## Prompt de inicio recomendado
```text
Lee CLAUDE.md, docs/CONTEXTO.md, docs/ARQUITECTURA.md y docs/BACKLOG.md. Este repositorio contiene el paquete de planificación de Kanban IA; debes construir la aplicación desde cero respetando las decisiones existentes.

Implementa únicamente HU-001. Antes de editar, resume el alcance y verifica versiones compatibles en documentación oficial. Usa architecture-reviewer para revisar la estructura y los especialistas rust-engineer y desktop-ui-engineer para tareas acotadas cuando aporten valor. Evita que editen los mismos archivos simultáneamente. Tú eres responsable de integrar y comprobar el resultado.

No implementes aún MCP, ACP ni historias posteriores. Ejecuta las comprobaciones disponibles, pide a quality-reviewer que revise la evidencia, corrige hallazgos relevantes y actualiza docs/BACKLOG.md. Entrega comandos reales para arrancar la aplicación, pruebas realizadas y limitaciones. No hagas push ni publiques.
```

## Prompt para avanzar por hito
Usar después de revisar HU-001 y confirmar que la dirección técnica y visual es correcta.
```text
Continúa el hito M1 de docs/BACKLOG.md, siempre que M0 esté completado. Procesa las historias pendientes en el orden recomendado y respeta dependencias. Completa y verifica una historia antes de pasar a la siguiente; usa subagentes solo para tareas acotadas y con propiedad clara de archivos. No cambies el stack, no adelantes M2/M3/M4 y no publiques. Si una historia queda bloqueada, registra la causa y continúa únicamente con historias independientes. Mantén evidencia real y entrega un reporte final del hito.
```
Para M2, M3 o M4 sustituye el hito y su prerrequisito explícitamente. M3 requiere elegir integraciones reales y disponer de su entorno; no autoriza gastos ni acceso a cuentas por sí solo.

## Responsabilidades
| Perfil | Uso |
|---|---|
| Claude principal | Coordinar, integrar, ejecutar comprobaciones y mantener backlog |
| architecture-reviewer | Revisión estructural, solo lectura |
| rust-engineer | Casos de uso, SQLite y migraciones |
| desktop-ui-engineer | Interfaz React/Tauri y accesibilidad |
| agent-integration-engineer | MCP, ACP y procesos |
| quality-reviewer | Revisar criterios y evidencia, solo lectura |
| security-reviewer | Revisar permisos y fronteras, solo lectura |

No es necesario lanzar los seis perfiles por historia. Heredan el modelo elegido en la sesión; este paquete no fuerza un modelo ni configura ejecución automática, permisos amplios o servicios MCP externos. Los revisores sin herramientas de ejecución solicitan evidencia al coordinador, no afirman haber corrido pruebas.

## Punto de corte útil
HU-016 entrega el MVP local con integración MCP. HU-017–021 añaden orquestación real. HU-022 prepara distribución; firma, licencia y publicación requieren resolver las decisiones del propietario. Mantén esos límites para evitar construir toda la plataforma antes de validar su utilidad.
