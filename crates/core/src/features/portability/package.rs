//! Formato del paquete (carpeta) y su lectura segura.
//!
//! ```text
//! <carpeta>/
//!   manifest.json            formato, versión, proyecto, columnas y entradas con hash base
//!   epics/E-1-acceso.md      cabecera TOML entre `+++` + descripción Markdown
//!   stories/0001-login.md
//! ```
//!
//! Solo se leen los archivos que lista el manifiesto, con nombres de una lista blanca estricta
//! (`stories/` o `epics/` + `[A-Za-z0-9._-]+.md`), sin enlaces simbólicos y dentro de la
//! carpeta elegida. Todo lo demás se ignora.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::CoreError;
use crate::features::columns::ColumnCategory;
use crate::features::stories::Priority;
use crate::platform::idempotency::payload_hash;

pub(super) const FORMAT: &str = "kanbania-package";
pub(super) const VERSION: u32 = 1;
pub(super) const MANIFEST: &str = "manifest.json";
const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// Tope de bytes leídos en total (evita agotar memoria con miles de archivos grandes).
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// Tope de números #N, E-n y CA-n: muy por encima del uso real y lejos del desbordamiento.
pub(super) const MAX_NUMBER: i64 = 1_000_000;
pub(super) const MAX_STORIES: usize = 5_000;
pub(super) const MAX_EPICS: usize = 1_000;
const MAX_COLUMNS: usize = 50;
const MAX_ID_CHARS: usize = 64;
const MAX_FILE_NAME_CHARS: usize = 120;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Manifest {
    pub format: String,
    pub version: u32,
    pub exported_at: String,
    pub app_version: String,
    pub project: ManifestProject,
    pub columns: Vec<ManifestColumn>,
    pub epics: Vec<ManifestEntry>,
    pub stories: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ManifestProject {
    pub id: String,
    pub name: String,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ManifestColumn {
    pub id: String,
    pub name: String,
    pub category: ColumnCategory,
    pub position: i64,
    pub wip_limit: Option<i64>,
}

/// Entrada del manifiesto. `content_hash` es el hash del contenido *al exportar*: la base del
/// cruce a tres bandas (paquete, local y base) que detecta qué lado cambió.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ManifestEntry {
    pub id: String,
    pub file: String,
    pub revision: i64,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct EpicHeader {
    pub id: String,
    pub number: i64,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct StoryHeader {
    pub id: String,
    pub number: i64,
    pub title: String,
    pub priority: Priority,
    /// Columna al exportar (id del manifiesto).
    pub column: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub next_criterion_number: i64,
    #[serde(default)]
    pub criteria: Vec<CriterionDoc>,
    /// Estado de revisión exportado como dato informativo: **nunca se importa** (aceptar es
    /// una decisión humana en la app). Solo se reutiliza `agent_id` si ese agente existe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ReviewInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CriterionDoc {
    /// Sin id: criterio nuevo escrito a mano.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<i64>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ReviewInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
}

/// Contenido que se compara y se importa de una historia (el resto es estado del tablero).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoryContent {
    pub title: String,
    pub description: String,
    pub priority: Priority,
    /// (id, texto) en orden; id `None` = criterio nuevo.
    pub criteria: Vec<(Option<String>, String)>,
    pub epic: Option<String>,
    pub parent: Option<String>,
    /// Ordenadas.
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct EpicContent {
    pub title: String,
    pub description: String,
}

/// Hash de contenido para el cruce a tres bandas. **Forma parte del formato `VERSION`**: si
/// cambian los campos hasheados o el algoritmo, hay que subir la versión (una prueba fija el
/// valor). Las revisiones del manifiesto son informativas: la decisión usa estos hashes.
pub(super) fn content_hash<T: Serialize>(content: &T) -> Result<String, CoreError> {
    payload_hash(content)
}

impl StoryContent {
    /// Hash para comparar versiones. Los criterios cuentan por su texto y orden, no por su id:
    /// un criterio escrito a mano (sin id) que ya se importó no vuelve a contar como cambio.
    pub fn hash(&self) -> Result<String, CoreError> {
        #[derive(Serialize)]
        struct View<'a> {
            title: &'a str,
            description: &'a str,
            priority: Priority,
            criteria: Vec<&'a str>,
            epic: Option<&'a str>,
            parent: Option<&'a str>,
            depends_on: &'a [String],
        }
        payload_hash(&View {
            title: &self.title,
            description: &self.description,
            priority: self.priority,
            criteria: self
                .criteria
                .iter()
                .map(|(_, text)| text.as_str())
                .collect(),
            epic: self.epic.as_deref(),
            parent: self.parent.as_deref(),
            depends_on: &self.depends_on,
        })
    }
}

pub(super) struct EpicDoc {
    pub entry: ManifestEntry,
    pub header: EpicHeader,
    pub description: String,
}

pub(super) struct StoryDoc {
    pub entry: ManifestEntry,
    pub header: StoryHeader,
    pub description: String,
}

pub(super) struct Package {
    pub manifest: Manifest,
    pub epics: Vec<EpicDoc>,
    pub stories: Vec<StoryDoc>,
    /// Hash de todos los bytes leídos: si el paquete cambia entre la vista previa y la
    /// aplicación, se exige volver a previsualizar.
    pub fingerprint: String,
}

pub(super) fn invalid(reason: impl Into<String>) -> CoreError {
    CoreError::Validation {
        field: "package",
        reason: reason.into(),
    }
}

/// Documento Markdown con cabecera TOML entre líneas `+++`. La descripción termina con un
/// salto de línea que la lectura retira (los editores suelen añadirlo al guardar).
pub(super) fn write_document<H: Serialize>(
    header: &H,
    description: &str,
) -> Result<String, CoreError> {
    let header = toml::to_string(header)
        .map_err(|_| invalid("No se pudo serializar la cabecera del documento."))?;
    Ok(format!("+++\n{header}+++\n{description}\n"))
}

fn read_document<H: for<'de> Deserialize<'de>>(
    file: &str,
    text: &str,
) -> Result<(H, String), CoreError> {
    // BOM de editores de Windows; el cuerpo se conserva byte a byte (sin normalizar CRLF).
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("+++\n")
        .or_else(|| text.strip_prefix("+++\r\n"))
        .ok_or_else(|| invalid(format!("{file}: falta la cabecera `+++` al principio.")))?;
    let (header, body) = match (rest.find("\n+++\n"), rest.find("\n+++\r\n")) {
        (Some(a), Some(b)) if b < a => (&rest[..=b], &rest[b + 6..]),
        (Some(a), _) => (&rest[..=a], &rest[a + 5..]),
        (None, Some(b)) => (&rest[..=b], &rest[b + 6..]),
        (None, None) => match rest
            .strip_suffix("\n+++")
            .or_else(|| rest.strip_suffix("\n+++\r\n"))
        {
            Some(header) => (header, ""),
            None => {
                return Err(invalid(format!(
                    "{file}: la cabecera no termina con `+++`."
                )));
            }
        },
    };
    let header: H = toml::from_str(header).map_err(|e| {
        invalid(format!(
            "{file}: cabecera no válida ({}).",
            first_line(&e.to_string())
        ))
    })?;
    let description = body
        .strip_suffix("\r\n")
        .or_else(|| body.strip_suffix('\n'))
        .unwrap_or(body)
        .to_owned();
    Ok((header, description))
}

fn first_line(message: &str) -> &str {
    message.lines().next().unwrap_or("")
}

pub(super) fn validate_id(kind: &str, id: &str) -> Result<(), CoreError> {
    let ok = !id.is_empty()
        && id.len() <= MAX_ID_CHARS
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(invalid(format!(
            "Identificador de {kind} no válido: «{}».",
            truncate(id)
        )))
    }
}

fn truncate(text: &str) -> String {
    text.chars().take(40).collect()
}

/// Nombre de archivo para exportar: solo ASCII alfanumérico y guiones, sin Unicode, para
/// que funcione igual en macOS y Windows.
pub(super) fn slug(text: &str, fallback: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 40 {
            break;
        }
    }
    let out = out.trim_matches('-').to_owned();
    if out.is_empty() {
        fallback.to_owned()
    } else {
        out
    }
}

/// Ruta relativa del manifiesto → ruta dentro de `root`. Rechaza todo lo que no sea
/// `<dir>/<nombre>.md` con caracteres seguros: así no hay `..`, rutas absolutas, unidades de
/// Windows ni separadores `\`. Un nombre reservado de Windows (`CON.md`) no es un archivo
/// normal y lo rechaza la lectura.
fn safe_path(root: &Path, dir: &str, file: &str) -> Result<PathBuf, CoreError> {
    let reject = || {
        invalid(format!(
            "Ruta no permitida en el paquete: «{}».",
            truncate(file)
        ))
    };
    let name = file
        .strip_prefix(dir)
        .and_then(|r| r.strip_prefix('/'))
        .ok_or_else(reject)?;
    let valid = !name.is_empty()
        && name.len() <= MAX_FILE_NAME_CHARS
        && !name.starts_with('.')
        && name.ends_with(".md")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !valid {
        return Err(reject());
    }
    Ok(root.join(dir).join(name))
}

fn read_limited(
    root: &Path,
    path: &Path,
    limit: u64,
    label: &str,
    budget: &mut u64,
) -> Result<Vec<u8>, CoreError> {
    use std::io::Read;
    let missing = || invalid(format!("Falta {label} en el paquete."));
    let meta = std::fs::symlink_metadata(path).map_err(|_| missing())?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(invalid(format!(
            "{label} no es un archivo normal (¿enlace simbólico?)."
        )));
    }
    // Defensa en profundidad: ni siquiera una carpeta intermedia enlazada puede sacar la
    // lectura de la carpeta elegida.
    let real = dunce::canonicalize(path).map_err(|_| missing())?;
    if !real.starts_with(root) {
        return Err(invalid(format!(
            "{label} está fuera de la carpeta del paquete."
        )));
    }
    // Una sola apertura y lectura acotada: aunque el archivo crezca entre comprobar y leer,
    // nunca se lee más del límite.
    let file = std::fs::File::open(&real).map_err(|_| missing())?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid(format!("No se pudo leer {label}.")))?;
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if len > limit {
        return Err(invalid(format!(
            "{label} supera el tamaño máximo ({} KiB).",
            limit / 1024
        )));
    }
    *budget = budget.saturating_sub(len);
    if *budget == 0 {
        return Err(invalid(format!(
            "El paquete supera el tamaño total máximo ({} MiB).",
            MAX_TOTAL_BYTES / 1024 / 1024
        )));
    }
    Ok(bytes)
}

/// La huella incluye nombre y longitud de cada archivo: mover bytes entre archivos la cambia.
fn add_to_fingerprint(hasher: &mut Sha256, name: &str, bytes: &[u8]) {
    hasher.update(name.as_bytes());
    hasher.update([0]);
    hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
    hasher.update(bytes);
}

fn utf8(bytes: Vec<u8>, label: &str) -> Result<String, CoreError> {
    String::from_utf8(bytes).map_err(|_| invalid(format!("{label} no es texto UTF-8.")))
}

/// Lee y valida la estructura del paquete (sin tocar la base). Operación de disco bloqueante:
/// llamarla fuera del runtime asíncrono.
pub(super) fn read_package(dir: &Path) -> Result<Package, CoreError> {
    let meta = std::fs::metadata(dir)
        .map_err(|_| invalid("La carpeta del paquete no existe o no se puede leer."))?;
    if !meta.is_dir() {
        return Err(invalid("Elige la carpeta del paquete, no un archivo."));
    }
    let root = dunce::canonicalize(dir)
        .map_err(|_| invalid("La carpeta del paquete no existe o no se puede leer."))?;
    let mut fingerprint = Sha256::new();
    let mut budget = MAX_TOTAL_BYTES;

    let bytes = read_limited(
        &root,
        &root.join(MANIFEST),
        MAX_MANIFEST_BYTES,
        MANIFEST,
        &mut budget,
    )?;
    add_to_fingerprint(&mut fingerprint, MANIFEST, &bytes);
    // Primero el formato y la versión: así un paquete de otra versión da un error claro.
    let probe: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| invalid("manifest.json no es JSON válido."))?;
    if probe.get("format").and_then(serde_json::Value::as_str) != Some(FORMAT) {
        return Err(invalid(
            "Formato incompatible: no es un paquete de Kanban IA.",
        ));
    }
    match probe.get("version").and_then(serde_json::Value::as_u64) {
        Some(v) if v == u64::from(VERSION) => {}
        Some(v) if v > u64::from(VERSION) => {
            return Err(invalid(format!(
                "El paquete usa la versión {v} del formato, más reciente que esta app ({VERSION})."
            )));
        }
        _ => {
            return Err(invalid(
                "Formato incompatible: versión del paquete no válida.",
            ));
        }
    }
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| {
        invalid(format!(
            "manifest.json no cumple el esquema ({}).",
            first_line(&e.to_string())
        ))
    })?;

    validate_manifest(&manifest)?;

    let mut epics = Vec::with_capacity(manifest.epics.len());
    for entry in &manifest.epics {
        let path = safe_path(&root, "epics", &entry.file)?;
        let bytes = read_limited(&root, &path, MAX_FILE_BYTES, &entry.file, &mut budget)?;
        add_to_fingerprint(&mut fingerprint, &entry.file, &bytes);
        let (header, description): (EpicHeader, String) =
            read_document(&entry.file, &utf8(bytes, &entry.file)?)?;
        if header.id != entry.id {
            return Err(invalid(format!(
                "{}: el id no coincide con el manifiesto.",
                entry.file
            )));
        }
        epics.push(EpicDoc {
            entry: entry.clone(),
            header,
            description,
        });
    }
    let mut stories = Vec::with_capacity(manifest.stories.len());
    for entry in &manifest.stories {
        let path = safe_path(&root, "stories", &entry.file)?;
        let bytes = read_limited(&root, &path, MAX_FILE_BYTES, &entry.file, &mut budget)?;
        add_to_fingerprint(&mut fingerprint, &entry.file, &bytes);
        let (header, description): (StoryHeader, String) =
            read_document(&entry.file, &utf8(bytes, &entry.file)?)?;
        if header.id != entry.id {
            return Err(invalid(format!(
                "{}: el id no coincide con el manifiesto.",
                entry.file
            )));
        }
        stories.push(StoryDoc {
            entry: entry.clone(),
            header,
            description,
        });
    }
    validate_documents(&manifest, &epics, &stories)?;

    let fingerprint = fingerprint
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(Package {
        manifest,
        epics,
        stories,
        fingerprint,
    })
}

fn validate_manifest(manifest: &Manifest) -> Result<(), CoreError> {
    if manifest.stories.len() > MAX_STORIES
        || manifest.epics.len() > MAX_EPICS
        || manifest.columns.len() > MAX_COLUMNS
    {
        return Err(invalid(format!(
            "El paquete supera los límites ({MAX_STORIES} historias, {MAX_EPICS} épicas, \
             {MAX_COLUMNS} columnas)."
        )));
    }
    validate_id("proyecto", &manifest.project.id)?;
    let mut ids = HashSet::new();
    ids.insert(manifest.project.id.as_str());
    let mut files = HashSet::new();
    for column in &manifest.columns {
        validate_id("columna", &column.id)?;
        if !ids.insert(column.id.as_str()) {
            return Err(duplicate(&column.id));
        }
    }
    if manifest.columns.is_empty() {
        return Err(invalid("El paquete no tiene columnas."));
    }
    for (kind, entries) in [("épica", &manifest.epics), ("historia", &manifest.stories)] {
        for entry in entries {
            validate_id(kind, &entry.id)?;
            if !ids.insert(entry.id.as_str()) {
                return Err(duplicate(&entry.id));
            }
            if !files.insert(entry.file.to_ascii_lowercase()) {
                return Err(invalid(format!(
                    "El archivo «{}» aparece dos veces.",
                    truncate(&entry.file)
                )));
            }
        }
    }
    Ok(())
}

fn duplicate(id: &str) -> CoreError {
    invalid(format!(
        "Identificador duplicado en el paquete: «{}».",
        truncate(id)
    ))
}

/// Coherencia interna: números únicos, criterios con ids únicos y relaciones sin
/// autorreferencias. Las referencias a historias/épicas se resuelven en el plan, porque
/// pueden apuntar a datos que ya existen en el proyecto local.
fn validate_documents(
    manifest: &Manifest,
    epics: &[EpicDoc],
    stories: &[StoryDoc],
) -> Result<(), CoreError> {
    let columns: HashSet<&str> = manifest.columns.iter().map(|c| c.id.as_str()).collect();
    let mut numbers = HashSet::new();
    for epic in epics {
        if !(1..=MAX_NUMBER).contains(&epic.header.number) || !numbers.insert(epic.header.number) {
            return Err(invalid(format!(
                "{}: número de épica repetido o no válido.",
                epic.entry.file
            )));
        }
    }
    let mut numbers = HashSet::new();
    let mut criteria = HashSet::new();
    for story in stories {
        let h = &story.header;
        let file = &story.entry.file;
        if !(1..=MAX_NUMBER).contains(&h.number) || !numbers.insert(h.number) {
            return Err(invalid(format!(
                "{file}: número de historia repetido o no válido."
            )));
        }
        if !columns.contains(h.column.as_str()) {
            return Err(invalid(format!(
                "{file}: la columna no está en el manifiesto."
            )));
        }
        if !(1..=MAX_NUMBER).contains(&h.next_criterion_number) {
            return Err(invalid(format!(
                "{file}: nextCriterionNumber fuera de rango."
            )));
        }
        let mut criterion_numbers = HashSet::new();
        for c in &h.criteria {
            if let Some(id) = &c.id {
                validate_id("criterio", id)?;
                if !criteria.insert(id.as_str()) {
                    return Err(duplicate(id));
                }
            }
            if let Some(n) = c.number
                && (!(1..=MAX_NUMBER).contains(&n) || !criterion_numbers.insert(n))
            {
                return Err(invalid(format!(
                    "{file}: número de criterio repetido o no válido."
                )));
            }
        }
        for id in h.epic.iter().chain(&h.parent).chain(&h.depends_on) {
            validate_id("relación", id)?;
        }
        if h.parent.as_deref() == Some(h.id.as_str()) || h.depends_on.contains(&h.id) {
            return Err(invalid(format!(
                "{file}: una historia no puede relacionarse consigo misma."
            )));
        }
        let unique: HashSet<&String> = h.depends_on.iter().collect();
        if unique.len() != h.depends_on.len() {
            return Err(invalid(format!("{file}: dependencia repetida.")));
        }
    }
    Ok(())
}
