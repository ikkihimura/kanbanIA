# Arquitectura y contratos

## Estructura propuesta
```text
apps/desktop/             # React/Vite y host Tauri
crates/core/              # Casos de uso y reglas, sin depender de Tauri
  src/features/          # stories/create, stories/move, runs/submit_result...
  src/platform/          # SQLite, reloj, IDs, eventos, archivos
crates/local-service/     # Propietario único de escrituras y API local para M2
crates/mcp/               # Adaptador stdio hacia servicio local, M2
crates/runner/            # Supervisión e integraciones, M3
docs/stories/
```
No crear crates vacíos de hitos futuros solo para cumplir el árbol. Cada slice ofrece una función de registro para el transporte, recibe dependencias explícitas y mantiene sus detalles privados. Las reglas no dependen del transporte. Evitar abstracciones antes de tener usos reales.

## Decisiones técnicas
- Rust estable, Tokio cuando se necesite asincronía; SQLite con SQLx y migraciones explícitas como opción inicial. Verificar compatibilidad al implementar HU-001.
- Tauri IPC para UI. En M2, host desktop mantiene el servicio local que invoca los mismos casos de uso; MCP no abre una segunda conexión con autoridad independiente.
- MVP MCP requiere aplicación abierta. El proceso MCP usa stdio y conecta a una API loopback autenticada del host. Axum es la opción propuesta para esa API. No hay servidor en red ni daemon automático.
- Puerto dinámico y descriptor local protegido por permisos del usuario; token local fuera de Git y logs. Enlazar solo loopback; autenticar cada petición, validar Host/Origin cuando corresponda, limitar payloads. No habilitar CORS global.
- Un solo host por perfil de datos; segunda ventana/instancia se conecta o falla claramente. Prueba de competencia entre dos arranques.
- SQLite fuera del repo, en directorio de datos de aplicación; WAL en disco local, foreign keys, busy timeout acotado, migraciones transaccionales y copias consistentes. Nunca copiar solo el .db abierto ignorando WAL.
- Eventos relevantes y resultado en SQLite; stdout/stderr voluminoso en archivos JSONL acotados por ejecución, con retención y rotación. No inventar event sourcing completo.
- Tipos de frontera serializables y contrato versionado. Validar en Rust incluso si TypeScript valida. Reutilizar contratos generados si la herramienta elegida lo soporta con poco costo.

## Modelo mínimo
Project, RepositoryLink, Epic, Story, AcceptanceCriterion, Column, StoryDependency, AgentProfile, Run, RunEvent, Approval, ArtifactReference, Claim, MutationReceipt.
IDs estables; timestamps UTC; presentación en zona local. Entidades editables tienen revision entero. Toda relación valida pertenencia a proyecto. Story guarda column_id y orden; cada columna tiene categoría semántica backlog/ready/active/review/done para reglas independientes del nombre visible. Blocked es atributo con motivo, no una categoría obligatoria.
Story incluye parent_story_id opcional; prohibir ciclos y parentesco cruzado. Dependencias bloquean ejecución hasta aceptación de prerequisitos. Prohibir ciclos al editar.
Done requiere aceptación humana explícita en MVP; mover a una columna done invoca la misma operación de aceptación, verificando criterios. Reabrir elimina la condición actual de aceptada conservando historial.

## Concurrencia y protocolo de errores
Mutaciones reciben expected_revision cuando editan entidad y request_id en operaciones reintentables. Una revisión obsoleta devuelve conflict y estado vigente; no sobrescribe silenciosamente. request_id se vincula a identidad+operación+hash de payload: mismo payload devuelve resultado original, payload distinto devuelve conflicto. Error estructurado con code, mensaje seguro y request_id; no stack traces al usuario.
Claim atómico y exclusivo por historia, con propietario y expiración. Renovación explícita; al expirar queda disponible de nuevo. Entrega desde claim vencido requiere reconciliación, no cambia estado automáticamente. Probar dos reclamaciones simultáneas con reloj controlable.

## Ejecución y observación
Estados Run: queued, preparing, running, waiting_approval, completed, failed, cancelled, interrupted. Recuperar ejecuciones no verificables como interrupted/unknown; nunca reiniciarlas sin política explícita. Persistir snapshot de historia y referencia a versión/commit del contexto antes de ejecutar.
Agente externo MCP reporta entregas y bloqueos, pero no puede aprobar su propio trabajo. No inferir salud del proceso a partir de reportes. Fuente observada/reported visible. Costos y consumo solo si proveedor los informa o si una estimación está claramente etiquetada.
M3 usa worktree por ejecución, proceso con argumentos explícitos y directorio controlado; worktree no es sandbox. Aprobaciones ligadas a run y acción exacta; rechazar duplicados o aprobaciones caducadas. Cancelación y cierre deben contemplar procesos hijos en ambos sistemas.

## MCP y ACP
MCP: este producto es servidor; agentes externos son clientes. Herramientas iniciales: list_ready_stories, get_story_context, claim_story, renew_claim, create_subtask, report_blocker, submit_result. Contexto paginado/acotado. Cada sesión/capacidad restringe proyecto; el agente no elige ampliar sus permisos cambiando project_id.
ACP: el producto es cliente de agentes compatibles para sesiones, actividad, permisos y cancelación según capacidades negociadas. No asumir compatibilidad nativa de un proveedor; registrar versiones y prueba real del adaptador elegido. Agentes sin ACP necesitan adaptador específico o quedan fuera del hito.
Ambos transportes invocan reglas centrales. MCP no concede acceso SQL, shell genérico ni aceptación humana. No exponer start_run hasta existir supervisor, políticas y límites de concurrencia.

## Verificación
Pruebas de slices a través de su entrada con SQLite temporal real para persistencia y transacciones. Pruebas de contrato para transportes, pruebas UI para operaciones críticas y proceso falso determinista para errores/cancelación. Integraciones reales se verifican por separado; mocks no demuestran compatibilidad con proveedores. CI macOS/Windows para builds y pruebas disponibles; documentar cobertura real de UI nativa en cada plataforma.
