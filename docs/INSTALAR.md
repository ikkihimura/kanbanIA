# Instalar y actualizar Kanban IA (alfa 0.2.0)

Kanban IA es una aplicación de escritorio local: tus proyectos se guardan en tu equipo, sin cuentas ni servidores. Esta versión es una **alfa sin firmar**: puede tener errores y los instaladores no están firmados ni notarizados.

Otras guías: [agentes de IA](AGENTES.md), [copias y exportación](RESPALDO.md), [guía local del ciclo completo](GUIA-LOCAL.md).

## Requisitos

| Sistema | Estado en esta alfa |
| --- | --- |
| macOS 26, Apple Silicon (arm64) | Comprobado (instalación limpia y actualización desde 0.1.0). **Solo Apple Silicon**: no hay instalador para Mac con Intel. Versiones anteriores de macOS sin probar. |
| Windows 10/11 x64 | **Experimental, sin soporte**: la CI compila y pasa todas las pruebas automáticas en Windows y genera sus instaladores, pero nadie los ha instalado todavía y por ahora Windows queda fuera del alcance del proyecto. Requiere WebView2 (ver abajo). |
| Linux | No incluido. |

Solo para usar agentes de IA (opcional, ver [AGENTES.md](AGENTES.md)):

- **Node.js ≥ 22** en `PATH`, para los adaptadores ACP que se instalan con npm.
- **Git ≥ 2.36**, para los worktrees de los intentos. La app no comprueba la versión.

## Descargar

Descarga los instaladores de la [página de releases](https://github.com/ikkihimura/kanbanIA/releases) del repositorio (la alfa 0.2.0 está marcada como *pre-release*). Se generan con GitHub Actions (flujo `installers.yml`, ejecución manual); también están como artefactos de esa ejecución durante 30 días (hace falta una sesión de GitHub y llegan como `.zip`). Comprueba el SHA-256 con `SHA256SUMS.txt` de la release (o `SHA256SUMS-*.txt` del artefacto); detecta una descarga dañada, no una manipulación. En macOS, en la carpeta de la descarga: `shasum -a 256 -c --ignore-missing SHA256SUMS.txt`; en Windows, compara la salida de `Get-FileHash <archivo>` (PowerShell) con la línea del archivo.

| Sistema | Archivo |
| --- | --- |
| macOS (Apple Silicon) | `.dmg` |
| Windows x64 | `.msi` o `-setup.exe` (NSIS). Usa uno de los dos, no ambos. |

## Instalar en macOS

1. Abre el `.dmg` y arrastra **Kanban IA** a «Aplicaciones».
2. La primera vez, macOS lo bloquea porque no está firmado con un certificado de Apple ni notarizado (solo lleva una firma ad-hoc). Para abrirlo: intenta abrirlo, cierra el aviso, ve a **Ajustes del Sistema → Privacidad y seguridad** y pulsa **Abrir igualmente** (desde macOS 15, Control + clic → «Abrir» ya no sirve para esto).
3. Si macOS dice que la app «está dañada», o prefieres la terminal, quita la marca de cuarentena de la app:

   ```sh
   xattr -dr com.apple.quarantine "/Applications/Kanban IA.app"
   ```

   Riesgo: desactiva para esa app la comprobación de Gatekeeper. Hazlo solo con un archivo que hayas descargado tú de la página de releases o de la ejecución del flujo del repositorio, nunca con uno que te hayan pasado por otra vía.

## Instalar en Windows

1. Ejecuta el `.msi` o el `-setup.exe`.
2. SmartScreen avisa porque el instalador no está firmado: pulsa **Más información** → **Ejecutar de todas formas**.
3. WebView2 viene preinstalado en Windows 10 (1803 o posterior) y 11. Si falta, el instalador intenta descargarlo (necesita conexión).

Windows es **experimental y sin soporte** en esta alfa (no se ha probado a mano): si algo falla, puedes abrir un informe de error (ver [CONTRIBUTING](../CONTRIBUTING.md)).

## Dónde están tus datos

Todo se guarda en una base SQLite dentro del directorio de datos de la app:

- macOS: `~/Library/Application Support/dev.ikkidev.kanbania/`
- Windows: `%APPDATA%\dev.ikkidev.kanbania\`

Para usar otro perfil (por ejemplo, para la demostración), define `KANBANIA_DATA_DIR` con una **ruta absoluta** antes de abrir la app. En macOS:

```sh
KANBANIA_DATA_DIR=/ruta/absoluta/perfil-demo "/Applications/Kanban IA.app/Contents/MacOS/kanbania-desktop"
```

La base debe estar en un disco local, no en una carpeta de red. Solo puede haber una instancia abierta por directorio de datos.

## Primeros pasos y demostración sin claves

Puedes probar la app sin agentes ni claves con el paquete de demostración `examples/demo/demo-recetario/` del repositorio (no va en el instalador: descarga el código del repositorio o solo esa carpeta):

1. Abre la app con un perfil aparte (arriba), para no mezclar la demo con tus proyectos.
2. En la lista de proyectos: **Importar paquete** → **Elegir paquete…** → carpeta `demo-recetario` → revisa la vista previa → **Aplicar importación**.

El paquete solo trae planificación (columnas, 2 épicas, 9 historias con criterios, una subtarea y dependencias). No incluye agentes, asignaciones, reclamos, entregas ni actividad: nada de agentes se simula. Detalle en la [guía local](GUIA-LOCAL.md).

## Actualizar

1. **Antes**: crea una copia manual en «Sistema» → «Copias de seguridad» → **Crear copia ahora** (ver [RESPALDO.md](RESPALDO.md)). Si quieres guardarla fuera del equipo, usa **Guardar en carpeta…**.
2. Cierra Kanban IA.
3. Instala la versión nueva encima:
   - macOS: sustituye la app en «Aplicaciones» por la nueva (y vuelve a autorizarla, ver «Instalar en macOS»).
   - Windows: ejecuta el instalador nuevo; actualiza la instalación existente.
4. Abre la app. Tus datos siguen en el directorio de datos.

Al abrir una versión con cambios de esquema pendientes, la app guarda antes una copia automática en `<datos>/backups/` y después migra. Si una migración falla, la base vuelve a quedar exactamente como estaba y la app muestra `storage_migration_failed`.

**No se puede volver a una versión anterior**: una base migrada por una versión más reciente no la abre una versión antigua (error `storage_newer_schema`; la base no se toca). Para volver atrás, reinstala la versión anterior y restaura una copia hecha con ella, perdiendo los cambios posteriores. La actualización entre versiones instaladas no se ha probado todavía en Windows.

## Desinstalar

Antes, si quieres conservar tus proyectos, crea una copia y guárdala en otra carpeta (ver [RESPALDO.md](RESPALDO.md)).

- macOS: arrastra **Kanban IA** de «Aplicaciones» a la papelera. El directorio de datos no se borra; si quieres borrarlo, elimina `~/Library/Application Support/dev.ikkidev.kanbania/` a mano.
- Windows: «Configuración» → «Aplicaciones» → Kanban IA → **Desinstalar**. El desinstalador NSIS (`-setup.exe`) ofrece una casilla para **borrar los datos de la aplicación**: si la marcas, se borra la base con todos tus proyectos. Sin marcarla (y con el `.msi`) los datos se conservan. Sin comprobar en esta alfa.

## Limitaciones conocidas de la alfa

- Instaladores sin firma ni notarización: distribución pública como **alfa sin firmar**.
- macOS solo Apple Silicon; Windows experimental y sin soporte (sin probar a mano).
- Sin actualización automática: cada versión se instala a mano.
- El servidor MCP (`kanbania-mcp`) no va dentro del instalador: es un binario aparte en la release y en el artefacto (o se compila desde el código fuente; ver [AGENTES.md](AGENTES.md)).
- Los adaptadores npm de los agentes necesitan `node`: si usas nvm y abres la app desde Finder, indica la «Ruta de node» en «Sistema» → «Ejecuciones» (ver [AGENTES.md](AGENTES.md)).
- En Windows, `service.json` hereda los permisos del directorio de datos: no uses `KANBANIA_DATA_DIR` en una carpeta compartida con otros usuarios.
- Iconos provisionales.
