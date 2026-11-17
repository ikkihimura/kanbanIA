# Seguridad

## Versiones con soporte

Kanban IA está en alfa. Solo se corrige la última versión publicada.

## Informar de una vulnerabilidad

**No abras un issue público.** Usa «Report a vulnerability» en la pestaña **Security** del repositorio (aviso privado de GitHub).

Incluye la versión, el sistema operativo, los pasos para reproducirlo y el impacto que esperas. **Nunca pegues**:

- tokens de agentes (`kbia_…`) ni claves de proveedores;
- el archivo `service.json` (contiene el token de sesión del servicio local);
- bases de datos (`kanbania.sqlite3`), copias de seguridad ni la carpeta `codex-home`;
- registros o salida de intentos sin quitar rutas personales, correos y secretos.

Si un token se ha expuesto, revócalo en la app («Acceso de agentes») y crea otro.

No hay plazos de respuesta garantizados: es un proyecto mantenido por una persona.

## Alcance y modelo de amenazas

Para orientar los informes:

- **Todo es local.** No hay cuentas, servidores ni sincronización remota. Los datos viven en el directorio de datos de la app.
- **Servicio local**: con la app abierta, el host escucha solo en `127.0.0.1` con puerto dinámico. Exige el token de sesión de `service.json` (solo legible por el usuario en macOS) y una credencial por agente y proyecto, de la que solo se guarda el sha256. Rechaza peticiones con `Origin` y hosts inesperados, sin CORS. Es de interés cualquier forma de saltarse estas comprobaciones o de que un agente acceda a otro proyecto.
- **Los agentes son no confiables**: lo que reportan (texto, entregas, peticiones de permiso) es dato, no instrucciones. Un agente nunca debe poder aceptar trabajo, ver rutas locales, copias de seguridad, intentos o credenciales, ni actuar como otro agente.
- **Ejecución de agentes (ACP)**: el agente corre como tu usuario, en un worktree, con entorno limpio y permisos que decides tú. El modelo de amenaza es de mismo usuario: el agente puede leer o modificar lo que tu usuario puede. Los datos de Kanban IA tienen una protección parcial (HU-023): en macOS, Claude Code corre en un sandbox del sistema que le impide acceder directamente al directorio de datos salvo su worktree, aunque un comando aprobado podría lanzar procesos fuera; en Windows no hay sandbox. Codex recibe un perfil de permisos que su propio sandbox aplica a sus comandos (comprobado con `codex sandbox`; Windows sin comprobar; un comando que apruebes para salir del sandbox ya no queda limitado). Además, la app ejecuta `git` sin sandbox sobre el worktree y respeta la configuración del repositorio (filtros incluidos), que un agente con una escritura aprobada podría plantar. Codex lee sin pedir permiso el resto de archivos, también fuera del worktree. Estas limitaciones están documentadas; los informes que muestren cómo saltarse una decisión de permiso o el aislamiento declarado sí son relevantes.
- **Windows**: `service.json` hereda los permisos del directorio de datos; no uses `KANBANIA_DATA_DIR` en una carpeta compartida.
- **Instaladores sin firmar** en esta alfa: descárgalos solo de los artefactos del flujo del repositorio.

Fuera de alcance: ataques que ya requieren control de tu cuenta de usuario o de la máquina, y el comportamiento propio de Claude Code, Codex o sus adaptadores (infórmalo a sus autores).
