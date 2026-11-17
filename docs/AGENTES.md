# Agentes de IA en Kanban IA

Kanban IA trabaja con agentes de dos formas, independientes entre sí:

- **MCP**: un agente que ya usas (por ejemplo, Claude Code) se conecta a la app, lee historias listas, reclama una y entrega su resultado. La app no lanza el agente.
- **ACP**: la app lanza el agente (Claude Code o Codex) en un worktree propio para un intento de una historia, y tú decides cada permiso.

En ambos casos el agente **nunca acepta** su trabajo: solo una persona acepta o devuelve una entrega. Lo que reporta un agente (bloqueos, subtareas, entregas, salida) son afirmaciones suyas, no hechos comprobados por la app.

Detalle técnico en el [README](../README.md) («Agentes por MCP», «Claude Code por ACP», «Aprobaciones», «Codex por ACP»).

## MCP (`kanbania-mcp`)

### Requisitos

- La app **abierta** con el mismo directorio de datos. Con la app cerrada, cada llamada responde `[app_closed] Kanban IA no está abierto…`; el proceso MCP nunca abre la base.
- El binario `kanbania-mcp` de la **misma versión** que la app. Viene aparte, junto al instalador, en el mismo artefacto del workflow «Instaladores» (`kanbania-mcp` en macOS, `kanbania-mcp.exe` en Windows; comprueba su SHA-256 en `SHA256SUMS-*.txt`, que detecta una descarga dañada, no una manipulación: las huellas viajan en el mismo artefacto). Guárdalo en una carpeta tuya. Tampoco está firmado: en macOS, antes de usarlo, `chmod +x kanbania-mcp` y quítale la cuarentena con `xattr -d com.apple.quarantine kanbania-mcp` (solo si confías en su origen). También puedes compilarlo desde el código fuente:

  ```sh
  cargo build --release -p kanbania-mcp   # queda en target/release/kanbania-mcp
  ```

### Configuración

1. En la app, crea el agente en «Equipo».
2. En el proyecto: «Configuración» → «Acceso de agentes» → elige el agente → **Crear credencial**. El token (`kbia_…`) se muestra **una sola vez**. Cada credencial liga un agente a un proyecto: el agente solo ve ese proyecto.
3. Registra el servidor en tu cliente MCP con el token en la variable de entorno `KANBANIA_AGENT_TOKEN`, **nunca como argumento**. En Claude Code, con alcance local o de usuario:

   ```sh
   claude mcp add kanbania --scope local -e KANBANIA_AGENT_TOKEN=kbia_… -- /ruta/a/target/release/kanbania-mcp
   ```

   Este comando deja el token en el historial de tu shell y Claude Code lo guarda en claro en su configuración (con `--scope local`, `~/.claude.json`). Empieza la línea con un espacio (con `HIST_IGNORE_SPACE` en zsh o `HISTCONTROL=ignorespace` en bash) o bórrala del historial, y revoca la credencial si compartes ese archivo.

   Si usas un `.mcp.json` versionado, no pongas el token en él: usa `"KANBANIA_AGENT_TOKEN": "${KANBANIA_AGENT_TOKEN}"` y define la variable en tu entorno.

4. `KANBANIA_DATA_DIR` (opcional, ruta absoluta) apunta a otro perfil de datos; por defecto se usa el de la app.

Revocar la credencial en «Acceso de agentes» corta el acceso en la siguiente petición.

### Herramientas

`list_ready_stories`, `get_story_context`, `claim_story`, `renew_claim`, `report_blocker`, `create_subtask` y `submit_result`. Un reclamo dura 30 minutos (máximo 5 vigentes por agente y proyecto). Entregar crea una entrega pendiente de revisión humana y mueve la tarjeta a «Revisión» si cabe. Descripción completa en el README, «Agentes por MCP». Mensajes de error y qué hacer: [guía local](GUIA-LOCAL.md), sección 4.

El contenido de las historias se sirve tal cual (puede venir de otros agentes): es dato, no instrucciones. Tu cliente MCP debe seguir pidiendo permiso para herramientas con efectos.

## ACP: intentos ejecutados por la app

Cada historia puede tener un **intento**: una instantánea inmutable de la historia que se ejecuta en un worktree propio (`<datos>/worktrees/<proyecto>/<intento>`, rama `kanbania/…`). El repositorio original no se toca. Requiere un repositorio vinculado al proyecto y **Git ≥ 2.36**.

La app no incluye ni instala los adaptadores. Instala cada uno en una carpeta propia con npm (necesitan **Node ≥ 22** en `PATH`).

### Claude Code

| Herramienta | Versión verificada |
| --- | --- |
| `@agentclientprotocol/claude-agent-acp` | 0.107.0 |
| Claude Code | con la sesión ya iniciada (la app no autentica ni pasa claves) |

1. `npm install @agentclientprotocol/claude-agent-acp@0.107.0` (o descarga el binario autónomo de sus releases).
2. En «Sistema» → «Ejecuciones», pega la ruta absoluta de `node_modules/.bin/claude-agent-acp`, pulsa **Comprobar adaptador** (no abre sesión ni gasta) y guarda.
3. En el detalle de un intento: **Iniciar…** → «Claude Code (ACP)».

La app fija el modo `default` (pedir permiso) y falla si no puede o si el agente lo cambia. Pide al adaptador no cargar tu configuración de Claude Code ni la del repositorio (reglas `allow`, hooks, `.mcp.json`).

### Codex

| Herramienta | Versión verificada |
| --- | --- |
| `@agentclientprotocol/codex-acp` | 2.5.0 (incluye `@openai/codex` 0.165) |
| Codex CLI (`codex`) | 0.160.0, solo para iniciar sesión |

1. `npm install @agentclientprotocol/codex-acp@2.5.0`.
2. En «Sistema» → «Ejecuciones», pega la ruta absoluta de `node_modules/.bin/codex-acp`, pulsa **Comprobar adaptador** y guarda.
3. Inicia sesión **una vez** en la carpeta de Codex de la app con el comando que muestra esa pantalla: `CODEX_HOME='<datos>/codex-home' codex login` (o su variante de PowerShell). La app nunca usa tu `~/.codex`.
4. En el detalle de un intento: **Iniciar…** → «Codex (ACP)».

La app usa el modo `read-only` de Codex, con una carpeta de Codex y una carpeta personal propias: no carga tu configuración, servidores MCP, reglas ni historial. Se niega a lanzarlo si su carpeta contiene configuración o si el repositorio contiene `.codex/`.

### Entorno

El agente se lanza sin shell y con un entorno limpio: nunca recibe `KANBANIA_*`, ni claves como `ANTHROPIC_*`, `CLAUDE_*`, `OPENAI_*` o `CODEX_API_KEY`. Usa tu sesión iniciada (suscripción o saldo).

## Permisos y aprobaciones

Cuando el agente pide permiso (escribir un archivo, ejecutar un comando…), el intento pasa a **«Espera tu permiso»** y la petición aparece en el detalle del intento, en el tablero y en «Atención».

- Solo **Permitir una vez** o **Rechazar**; nunca reglas permanentes.
- Caducan a los **10 minutos**: sin decisión, la app responde «rechazar» y el agente sigue. El tiempo máximo del intento sigue contando.
- Cancelar el intento invalida las peticiones abiertas y las decididas que aún no se habían entregado.
- **Permitir un comando lo ejecuta de verdad en tu equipo** (con el worktree como carpeta de trabajo). Solo los datos de Kanban IA quedan protegidos (ver «Limitaciones»). Permite solo lo que entiendas.
- Notificación del sistema **opcional y desactivada por defecto**: «Sistema» → «Ejecuciones» → «Avisarme…». Solo dice «Un agente pide permiso en #N».

## Limitaciones

- **Claude Code** ejecuta sin pedir permiso los comandos de solo lectura dentro del worktree (`cat`, `git ls-files`…). Para leer fuera del worktree con su herramienta Read sí pide permiso.
- **Codex lee archivos sin pedir permiso, también fuera del worktree.** Al pedir escribir solo ofrece aceptar o cancelar: **rechazar una escritura detiene su turno**.
- **Datos de Kanban IA** (base, copias, `service.json`, otros intentos): ningún agente debe leerlos.
  - Claude Code en **macOS** corre en un sandbox del sistema que le impide leerlos o modificarlos directamente, aunque apruebes el permiso (comprobado). Un comando que apruebes podría lanzar procesos fuera del sandbox (la app bloquea `launchctl`, `open` y `osascript`, pero no es una garantía completa). En **Windows no hay sandbox**: podría leerlos.
  - Codex recibe un perfil de permisos que su propio sandbox aplica a sus comandos (comprobado con `codex sandbox`; en la app, el modelo no llegó a intentar leerlos). Si apruebas que un comando salga del sandbox, ese comando ya no queda limitado. Windows sin comprobar.
- Los tokens y el coste que informa el agente se ven en la salida, pero **no se guardan** en el intento: se muestran como «no disponible». Codex no informa del coste.
- Los adaptadores npm necesitan `node` en `PATH`. Si usas nvm y abres la app desde Finder, no lo encuentran: abre la app desde una terminal donde `node` funcione, por ejemplo `"/Applications/Kanban IA.app/Contents/MacOS/kanbania-desktop"`, o usa el binario autónomo de Claude Code.
- El aislamiento de cada agente depende de opciones propias de su adaptador: vuelve a comprobarlo al cambiar de versión.
- Un proceso con permisos de tu usuario puede tocar lo que tu usuario puede tocar: los permisos de la app reducen el riesgo, no lo eliminan.
- Si la app se cierra de golpe, los procesos de agentes pueden seguir vivos en macOS. Al reiniciar, los intentos en marcha quedan «Interrumpido» y nada arranca solo.
- Windows: los adaptadores y la supervisión de procesos no se han ejecutado en Windows.
