# Guía local del MVP (M2)

Cómo completar un ciclo real de trabajo con Kanban IA en tu equipo: planificar en el tablero, dejar que un agente de IA tome una historia por MCP, revisar su entrega y aceptarla. Es una **validación local del MVP**, no una versión alfa distribuible: no hay instalador, firma ni actualizaciones (eso es HU-022).

Detalle de cada función en el [README](../README.md); estado y evidencia en [BACKLOG](BACKLOG.md).

## 1. Compilar y abrir

Requisitos y versiones: README, «Requisitos». Desde la raíz del repositorio:

```sh
npm ci
npm run tauri -- build --debug --no-bundle   # app: target/debug/kanbania-desktop
cargo build --release -p kanbania-mcp        # servidor MCP: target/release/kanbania-mcp
```

Abre la app con `target/debug/kanbania-desktop` (o `npm run dev` para desarrollo). Los datos van al directorio de datos del sistema (README, «Datos locales»).

## 2. Perfiles: datos reales y demostración

Usa un **perfil aparte** para probar o enseñar la app, para no mezclar ejemplos con tus proyectos:

```sh
KANBANIA_DATA_DIR=/ruta/absoluta/perfil-demo target/debug/kanbania-desktop
```

### Paquete de demostración

`examples/demo/demo-recetario/` contiene un proyecto de ejemplo, **«Demo · Recetario»**, en el formato de exportación Markdown (HU-010). Para cargarlo:

1. Abre la app con un perfil de demostración (arriba).
2. En la lista de proyectos, «Importar paquete» → **Elegir paquete…** → elige la carpeta `examples/demo/demo-recetario` → revisa la vista previa (11 elementos nuevos) → **Aplicar importación**.

El paquete solo trae planificación: columnas, 2 épicas, 9 historias con criterios (en Backlog, Listo y En curso), una subtarea y dependencias. Cada descripción indica que es contenido de ejemplo. **No incluye agentes, asignaciones, reclamos, entregas ni actividad de agentes**: nada de lo que verás sobre agentes es simulado; aparece solo cuando un agente real se conecta. Importar no crea nada en tus proyectos reales salvo que lo importes en tu perfil principal (no lo hagas). Para regenerarlo: `rm -rf examples/demo/demo-recetario && cargo run -p kanbania-core --example demo_package -- examples/demo`. Los ids cambian: importar el paquete regenerado en un perfil que ya tenía la demo crea un segundo proyecto.

## 3. Ciclo completo

1. **Proyecto**: crea un proyecto (o usa el de demostración). Opcional: vincula un repositorio.
2. **Historia**: crea una historia y escribe sus **criterios CA-n** (aceptar exige al menos uno). Muévela a «Listo».
3. **Agente**: en «Equipo», crea el perfil del agente (nombre, avatar). En el editor de la historia, **asígnala** a ese agente (opcional: un agente también puede tomar historias sin asignar).
4. **Credencial**: en el proyecto, «Configuración» → «Acceso de agentes» → elige el agente → **Crear credencial**. Copia el token: se muestra una sola vez. Comprueba que el servicio local aparece como activo.
5. **Cliente MCP**: registra `kanbania-mcp` en tu cliente con el token **en una variable de entorno, nunca como argumento** (README, «Agentes por MCP»). Con Claude Code:

   ```sh
   claude mcp add kanbania --scope local -e KANBANIA_AGENT_TOKEN=kbia_… -- /ruta/a/target/release/kanbania-mcp
   ```

   Con un perfil de demostración añade `-e KANBANIA_DATA_DIR=/ruta/absoluta/perfil-demo`.

6. **Trabajo del agente**: pide al agente algo como «Consulta las historias listas en Kanban IA, reclama la #N, haz el trabajo y entrega el resultado con un resumen y referencias». El agente usa `list_ready_stories`, `get_story_context`, `claim_story`, y puede usar `renew_claim`, `report_blocker` y `create_subtask` antes de `submit_result`. Mientras tanto, en la app:
   - la tarjeta muestra «Reclamada por…»;
   - el panel «Actividad del agente» de la historia muestra lo que el agente **reporta** y su última interacción reportada (no significa que siga trabajando);
   - el tablero y «Atención» se actualizan solos.
7. **Revisión**: con la entrega, la tarjeta pasa a «Revisión» y «Atención» muestra la entrega pendiente. Las referencias de un agente son afirmaciones: las URL nunca se visitan y sus rutas aparecen como «Ruta no comprobada (reportada por el agente)».
8. **Aceptar o devolver**: solo una persona. Marca los criterios cumplidos y **Acepta** (la tarjeta va a «Hecho») o **Devuelve** con un motivo (la historia vuelve a estar disponible para agentes). Un agente no tiene ninguna herramienta para aceptar.
9. **Reinicio**: cierra la app y ábrela de nuevo con el mismo perfil. Todo sigue ahí: la historia aceptada, la entrega, la actividad y las credenciales. El proceso MCP no hace falta reiniciarlo: vuelve a funcionar en cuanto la app está abierta.
10. **Al terminar**: revoca la credencial si ya no la usas («Acceso de agentes» → **Revocar**).

## 4. Si algo falla: mensajes y qué hacer

| Dónde | Mensaje | Qué hacer |
|---|---|---|
| Agente (MCP) | `[app_closed] Kanban IA no está abierto con este directorio de datos…` | Abre la app con el mismo perfil y repite la llamada con el mismo `requestId` (no se duplica). Si usas un perfil aparte, comprueba `KANBANIA_DATA_DIR`. |
| Agente (MCP) | Al arrancar, en stderr: falta `KANBANIA_AGENT_TOKEN` o no tiene el formato `kbia_…` | Configura la variable de entorno del cliente MCP con el token completo. Un token bien formado pero revocado arranca y falla en cada llamada con `[unauthorized]`. |
| Agente (MCP) | «No se pudo hablar con Kanban IA. Comprueba que la app está abierta.» | La app no respondió a tiempo o dio una respuesta ilegible: comprueba que está abierta y repite con el mismo `requestId`. |
| Agente (MCP) | `[storage_…] … (reintentable en unos segundos)` | La base está ocupada o restaurándose: reintenta con el mismo `requestId`. |
| Agente (MCP) | `[unauthorized] Credencial no válida o revocada…` | La credencial se revocó o no existe: crea otra en la app. |
| Agente (MCP) | `[claim_ended]` | El reclamo terminó (venció, entregó, una persona lo liberó o reasignó, se archivó o aceptó la historia, o se revocó la credencial). El agente debe dejar esa historia. |
| Agente (MCP) | Mensaje sobre permisos del descriptor del servicio | Cierra y vuelve a abrir la app: se regenera `service.json` con permisos solo del usuario. |
| Agente (MCP) | Versión de API incompatible | Compila `kanbania-mcp` desde el mismo código que la app. |
| App | «Servicio local: no disponible…» en «Acceso de agentes» | Los agentes no pueden conectarse: cierra Kanban IA y vuelve a abrirlo. |
| App | «Kanban IA ya está abierto con estos datos en otra ventana o proceso…» | Cierra la otra ventana y pulsa «Reintentar». |
| App | Errores de migración o esquema más reciente | Los datos no se han tocado; ver README, «Datos locales», y «Sistema» → «Copias de seguridad». |

## 5. Copias de seguridad

Antes de probar con datos que te importen: «Sistema» → «Copias de seguridad» → **Crear copia ahora** (README, «Copias de seguridad»). Restaurar cierra los reclamos de agentes y no reactiva credenciales revocadas.

## 6. Plataformas comprobadas

| Plataforma | Estado |
|---|---|
| macOS 26 arm64 | Comprobada el 2026-11-14: `npm run check` (UI 151 pruebas; Rust 353 + 1 ignorada), prueba integrada automatizada y ciclo manual real con Claude Code 2.1.321 (sección 8). |
| Windows 11 | **Sin comprobar.** Hay CI configurada (`.github/workflows/ci.yml`) pero nunca se ha ejecutado: el repositorio no tiene remoto. |
| Linux | Fuera del compromiso inicial. |

## 7. Limitaciones y errores conocidos

- Sin instalador, firma ni actualizaciones; el binario MCP se compila a mano y su ruta no es estable (HU-022).
- El MCP necesita la app abierta; un solo host por perfil de datos.
- Un agente no puede volver a reclamar su historia si una persona la mueve a «En curso»: devuélvela a Backlog o Listo.
- `[claim_ended]` no distingue el motivo exacto del cierre.
- Restaurar una copia cierra como «liberados» los reclamos de credenciales revocadas (no como «revocados»).
- Una entrega por MCP está limitada a 64 KB en total.
- `mutation_receipts` (recibos de idempotencia) no tiene retención.
- En Windows, `service.json` hereda los permisos del directorio de datos: no uses `KANBANIA_DATA_DIR` en una carpeta compartida.
- No hay ejecución ni supervisión de procesos de agentes (M3): la app solo registra lo que el agente reporta.
- Importar no restaura bloqueos: el paquete los lleva como dato informativo y la historia llega sin bloquear.
- Los nombres de archivo de una exportación sustituyen tildes y eñes por `-` (p. ej. `0005-planificar-el-men-semanal.md`); el contenido no cambia.
- En «Acceso de agentes», el estado del servicio local se lee al abrir la sección: no se actualiza solo.
- Licencia pendiente de decisión del propietario.

## 8. Evidencia de pruebas

- `npm run check`: formato, lint, typecheck y build web, pruebas UI (Vitest), clippy y pruebas Rust del workspace.
- Prueba integrada del ciclo con un cliente MCP real (SDK `rmcp`) y el binario `kanbania-mcp`: `crates/mcp/tests/mvp.rs`.
- Paquete de demostración importable y sin estados de agentes: `crates/core/tests/demo_package.rs`.
- **Ciclo manual real (2026-11-14, macOS 26 arm64, perfil nuevo):**
  - importar `examples/demo/demo-recetario` desde la app: 9 historias, sin agentes, asignaciones, reclamos ni actividad;
  - crear el agente y su credencial, y asignarle la #2;
  - Claude Code 2.1.321 con `kanbania-mcp` hizo `list_ready_stories` (#2 con `assignedToYou`), `get_story_context`, `claim_story` y `submit_result` (E1 pendiente, `inReview`);
  - en la app, sin recargar, se vieron el reclamo, la revisión, la ruta no comprobada y «Atención»; la persona aceptó la #2;
  - con la app cerrada, el agente recibió `[app_closed] … Ábrelo y vuelve a intentarlo` y `service.json` ya no existía;
  - tras reabrir (dos veces), sin reiniciar el MCP: #2 aceptada en «Hecho», E1 `accepted`, reclamo `delivered`, actividad conservada, y `claim_story` → `[not_ready]`;
  - tras revocar la credencial: `[unauthorized]`.
- Errores encontrados en la prueba manual: ninguno nuevo. Limitaciones: sección 7.
- Resultados de cada historia: [BACKLOG](BACKLOG.md).
