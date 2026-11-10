//! Plan de importación: qué pasará con cada épica e historia del paquete. Lo calculan igual
//! la vista previa (lectura) y la aplicación (dentro de su transacción). La huella del plan
//! (`plan_fingerprint`) garantiza que lo que se aplica es exactamente lo que se previsualizó o
//! falla pidiendo previsualizar de nuevo.
//!
//! Cruce a tres bandas por contenido: el manifiesto guarda el hash *base* de cada entidad al
//! exportar. Si solo cambió el paquete → actualizar; si solo cambió lo local → mantener lo
//! local; si cambiaron ambos → conflicto que la persona resuelve explícitamente.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use sqlx::SqliteConnection;
use ts_rs::TS;

use super::package::{
    EpicContent, ManifestColumn, Package, StoryContent, StoryDoc, content_hash, invalid,
};
use super::snapshot::{LocalProject, load_local};
use crate::error::CoreError;
use crate::features::columns::{
    Column, ColumnCategory, MAX_WIP, columns_of_project, ensure_capacity,
};
use crate::features::epics::{validate_epic_description, validate_epic_title};
use crate::features::projects::{load_project, validate_project_name};
use crate::features::relations::MAX_PREREQUISITES;
use crate::features::stories::{
    MAX_CRITERIA, validate_criterion, validate_story_description, validate_story_title,
};
use crate::features::team::load_agent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ImportAction {
    /// No existe aquí: se crea.
    Create,
    /// Solo cambió el paquete: se actualiza.
    Update,
    /// Igual en ambos lados.
    Unchanged,
    /// Solo cambió lo local desde la exportación: se conserva lo local.
    KeepLocal,
    /// Cambiaron ambos lados: hay que elegir.
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ImportEntityKind {
    Epic,
    Story,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportItem {
    pub id: String,
    pub kind: ImportEntityKind,
    /// «#3 Título» o «E-1 Título» tal como vendrán del paquete.
    pub label: String,
    pub action: ImportAction,
    /// En conflictos: título local, si difiere, para ayudar a decidir.
    pub local_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportPreview {
    /// Huella del paquete leído: la aplicación exige la misma.
    pub fingerprint: String,
    pub project_id: String,
    pub project_name: String,
    /// `false`: se creará un proyecto nuevo con sus columnas.
    pub project_exists: bool,
    pub items: Vec<ImportItem>,
    /// Avisos legibles (renumeraciones, columnas, aceptación no importada…).
    pub warnings: Vec<String>,
}

/// Historia validada del paquete.
pub(super) struct PackageStory<'a> {
    pub doc: &'a StoryDoc,
    pub content: StoryContent,
}

pub(super) struct Plan<'a> {
    pub preview: ImportPreview,
    pub project_exists: bool,
    pub project_name: String,
    pub epics: Vec<(&'a super::package::EpicDoc, EpicContent)>,
    pub stories: Vec<PackageStory<'a>>,
    /// Número final de cada épica/historia que se crea.
    pub numbers: HashMap<String, i64>,
    /// Columna local de cada historia que se crea.
    pub columns: HashMap<String, String>,
    /// Agentes existentes a los que asignar historias creadas.
    pub agents: HashMap<String, String>,
    /// Límites WIP de las columnas de un proyecto nuevo (ajustados para que quepan).
    pub column_limits: HashMap<String, Option<i64>>,
}

pub(super) async fn build_plan<'a>(
    conn: &mut SqliteConnection,
    package: &'a Package,
) -> Result<Plan<'a>, CoreError> {
    let manifest = &package.manifest;
    let mut warnings = Vec::new();

    let project = load_project(conn, &manifest.project.id).await?;
    let project_exists = project.is_some();
    let project_name = match &project {
        Some(p) => {
            if p.name != manifest.project.name.trim() {
                warnings.push(format!(
                    "El proyecto ya existe como «{}»: su nombre no se cambia.",
                    p.name
                ));
            }
            p.name.clone()
        }
        None => validate_project_name(&manifest.project.name).map_err(prefixed("manifest.json"))?,
    };
    let local = if project_exists {
        load_local(conn, &manifest.project.id).await?
    } else {
        LocalProject {
            epics: Vec::new(),
            stories: Vec::new(),
        }
    };
    let local_columns = if project_exists {
        columns_of_project(conn, &manifest.project.id).await?
    } else {
        Vec::new()
    };
    if !project_exists {
        validate_new_columns(conn, &manifest.columns).await?;
    }

    ensure_ids_free(conn, package, &manifest.project.id).await?;

    // Validar y normalizar contenido.
    let mut epics = Vec::with_capacity(package.epics.len());
    for doc in &package.epics {
        let content = EpicContent {
            title: validate_epic_title(&doc.header.title).map_err(prefixed(&doc.entry.file))?,
            description: validate_epic_description(&doc.description)
                .map_err(prefixed(&doc.entry.file))?,
        };
        epics.push((doc, content));
    }
    let epic_ids: HashSet<&str> = package
        .epics
        .iter()
        .map(|e| e.entry.id.as_str())
        .chain(local.epics.iter().map(|e| e.id.as_str()))
        .collect();
    let local_index = local.story_index();
    let story_ids: HashSet<&str> = package
        .stories
        .iter()
        .map(|s| s.entry.id.as_str())
        .chain(local_index.keys().copied())
        .collect();

    let mut stories = Vec::with_capacity(package.stories.len());
    for doc in &package.stories {
        let file = &doc.entry.file;
        let h = &doc.header;
        if h.criteria.len() > MAX_CRITERIA {
            return Err(invalid(format!("{file}: más de {MAX_CRITERIA} criterios.")));
        }
        if i64::try_from(h.depends_on.len()).unwrap_or(i64::MAX) > MAX_PREREQUISITES {
            return Err(invalid(format!(
                "{file}: más de {MAX_PREREQUISITES} dependencias."
            )));
        }
        if let Some(epic) = &h.epic
            && !epic_ids.contains(epic.as_str())
        {
            return Err(invalid(format!("{file}: la épica relacionada no existe.")));
        }
        for related in h.parent.iter().chain(&h.depends_on) {
            if !story_ids.contains(related.as_str()) {
                return Err(invalid(format!(
                    "{file}: una historia relacionada no existe."
                )));
            }
        }
        let mut depends_on = h.depends_on.clone();
        depends_on.sort();
        let content = StoryContent {
            title: validate_story_title(&h.title).map_err(prefixed(file))?,
            description: validate_story_description(&doc.description).map_err(prefixed(file))?,
            priority: h.priority,
            criteria: h
                .criteria
                .iter()
                .map(|c| {
                    Ok((
                        c.id.clone(),
                        validate_criterion(&c.text).map_err(prefixed(file))?,
                    ))
                })
                .collect::<Result<_, CoreError>>()?,
            epic: h.epic.clone(),
            parent: h.parent.clone(),
            depends_on,
        };
        stories.push(PackageStory { doc, content });
    }

    // Acciones. `state` resume el estado local que decide cada acción: entra en la huella, así
    // que si cambia entre la vista previa y la aplicación hay que volver a previsualizar.
    let mut actions = HashMap::new();
    let mut state: Vec<String> = Vec::new();
    let mut items = Vec::new();
    for (doc, content) in &epics {
        let label = format!("E-{} {}", doc.header.number, content.title);
        let (action, local_label) = match local.epic(&doc.entry.id) {
            None => (ImportAction::Create, None),
            Some(l) => (
                decide(
                    &record(&mut state, &l.id, content_hash(&l.content())?),
                    &content_hash(content)?,
                    &doc.entry.content_hash,
                ),
                (l.title != content.title).then(|| format!("E-{} {}", l.number, l.title)),
            ),
        };
        actions.insert(doc.entry.id.clone(), action);
        items.push(ImportItem {
            id: doc.entry.id.clone(),
            kind: ImportEntityKind::Epic,
            label,
            action,
            local_label,
        });
    }
    for story in &stories {
        let entry = &story.doc.entry;
        let label = format!("#{} {}", story.doc.header.number, story.content.title);
        let (mut action, local_label) = match local_index.get(entry.id.as_str()) {
            None => (ImportAction::Create, None),
            Some(l) => {
                let local_hash = l.content().hash()?;
                state.push(format!("{}:{local_hash}", l.id));
                (
                    decide(&local_hash, &story.content.hash()?, &entry.content_hash),
                    (l.title != story.content.title).then(|| format!("#{} {}", l.number, l.title)),
                )
            }
        };
        // Como en el editor: una historia archivada aquí no se modifica (restaurarla antes).
        if let Some(l) = local_index.get(entry.id.as_str())
            && l.archived
            && matches!(action, ImportAction::Update | ImportAction::Conflict)
        {
            action = ImportAction::KeepLocal;
            warnings.push(format!(
                "#{} está archivada aquí: sus cambios del paquete no se importan (restáurala antes).",
                l.number
            ));
        }
        actions.insert(entry.id.clone(), action);
        items.push(ImportItem {
            id: entry.id.clone(),
            kind: ImportEntityKind::Story,
            label,
            action,
            local_label,
        });
    }

    // Relaciones nuevas hacia historias archivadas aquí: el editor no las permite, así que la
    // importación tampoco (las que ya existían se conservan).
    for story in &stories {
        let id = story.doc.entry.id.as_str();
        if !matches!(
            actions.get(id),
            Some(ImportAction::Create | ImportAction::Update)
        ) {
            continue;
        }
        let existing = local_index.get(id);
        for target in story.content.parent.iter().chain(&story.content.depends_on) {
            let archived_here = local_index.get(target.as_str()).is_some_and(|t| t.archived);
            let already = existing.is_some_and(|e| {
                e.parent.as_deref() == Some(target.as_str()) || e.depends_on.contains(target)
            });
            if archived_here && !already {
                return Err(invalid(format!(
                    "{}: relaciona con una historia archivada aquí; restáurala antes de importar.",
                    story.doc.entry.file
                )));
            }
        }
    }

    // Números de lo que se crea: se conservan si están libres.
    let mut numbers = HashMap::new();
    let epic_creates: Vec<(&str, i64, &str)> = epics
        .iter()
        .filter(|(d, _)| actions.get(&d.entry.id) == Some(&ImportAction::Create))
        .map(|(d, c)| (d.entry.id.as_str(), d.header.number, c.title.as_str()))
        .collect();
    assign_numbers(
        &epic_creates,
        local.epics.iter().map(|e| e.number).collect(),
        &mut numbers,
        &mut warnings,
        "E-",
    );
    let story_creates: Vec<(&str, i64, &str)> = stories
        .iter()
        .filter(|s| actions.get(&s.doc.entry.id) == Some(&ImportAction::Create))
        .map(|s| {
            (
                s.doc.entry.id.as_str(),
                s.doc.header.number,
                s.content.title.as_str(),
            )
        })
        .collect();
    assign_numbers(
        &story_creates,
        local.stories.iter().map(|s| s.number).collect(),
        &mut numbers,
        &mut warnings,
        "#",
    );

    // Columna de cada historia creada.
    let target_columns: Vec<(String, ColumnCategory)> = if project_exists {
        local_columns
            .iter()
            .map(|c| (c.id.clone(), c.category))
            .collect()
    } else {
        let mut sorted: Vec<&ManifestColumn> = manifest.columns.iter().collect();
        sorted.sort_by_key(|c| c.position);
        sorted.iter().map(|c| (c.id.clone(), c.category)).collect()
    };
    let mut columns = HashMap::new();
    let mut accepted_skipped = 0;
    let mut remapped = 0;
    let mut agents = HashMap::new();
    for story in &stories {
        let id = &story.doc.entry.id;
        if actions.get(id) != Some(&ImportAction::Create) {
            continue;
        }
        let h = &story.doc.header;
        let package_category = manifest
            .columns
            .iter()
            .find(|c| c.id == h.column)
            .map_or(ColumnCategory::Backlog, |c| c.category);
        let exact = target_columns
            .iter()
            .find(|(cid, cat)| *cid == h.column && *cat != ColumnCategory::Done);
        let column = match exact {
            Some((cid, _)) => cid.clone(),
            None => {
                if package_category == ColumnCategory::Done {
                    accepted_skipped += 1;
                } else {
                    remapped += 1;
                }
                fallback_column(&target_columns, package_category).ok_or_else(|| {
                    invalid("El proyecto no tiene ninguna columna que no sea «hecho».")
                })?
            }
        };
        columns.insert(id.clone(), column);
        if let Some(review) = &h.review
            && let Some(agent_id) = &review.agent_id
            && !h.archived
        {
            if let Some(agent) = load_agent(conn, agent_id).await? {
                // Visible en la vista previa: una asignación puede importar (M2).
                warnings.push(format!("#{} se asignará a «{}».", h.number, agent.name));
                agents.insert(id.clone(), agent_id.clone());
            } else {
                warnings.push(format!(
                    "#{}: el agente «{}» no existe aquí; queda sin asignar.",
                    h.number,
                    review.agent_name.as_deref().unwrap_or("?")
                ));
            }
        }
    }
    // WIP: toda entrada pasa por `ensure_capacity`. En un proyecto existente, la vista previa
    // ya informa del error. En uno nuevo (restauración) el límite se sube lo justo, con aviso:
    // el tablero exportado pudo quedar por encima de su límite.
    let mut incoming: HashMap<&str, i64> = HashMap::new();
    for story in &stories {
        if !story.doc.header.archived
            && let Some(column) = columns.get(&story.doc.entry.id)
        {
            *incoming.entry(column.as_str()).or_default() += 1;
        }
    }
    let mut column_limits = HashMap::new();
    if project_exists {
        for (column_id, count) in &incoming {
            if let Some(column) = local_columns.iter().find(|c| c.id == *column_id) {
                ensure_capacity(conn, column, *count).await?;
            }
        }
    } else {
        for column in &manifest.columns {
            let count = incoming.get(column.id.as_str()).copied().unwrap_or(0);
            let limit = match column.wip_limit {
                Some(limit) if count > limit => {
                    let raised = (count <= MAX_WIP).then_some(count);
                    warnings.push(format!(
                        "El límite WIP de «{}» ({limit}) no admite sus {count} tarjetas: se {}.",
                        column.name.trim(),
                        raised.map_or_else(|| "quita".to_owned(), |r| format!("sube a {r}"))
                    ));
                    raised
                }
                other => other,
            };
            column_limits.insert(column.id.clone(), limit);
        }
    }

    if accepted_skipped > 0 {
        warnings.push(format!(
            "{accepted_skipped} historia(s) estaban en «hecho»: la aceptación no se importa y \
             entrarán en revisión para aceptarlas aquí."
        ));
    }
    if remapped > 0 {
        warnings.push(format!(
            "{remapped} historia(s) nuevas irán a la primera columna de su misma categoría \
             (su columna original no existe aquí)."
        ));
    }
    if package.stories.iter().any(|s| s.header.review.is_some()) {
        warnings.push(
            "Aceptaciones, entregas y bloqueos del paquete son informativos: no se importan."
                .into(),
        );
    }
    if project_exists {
        warnings.push(
            "Las historias existentes conservan su columna y posición; la importación nunca \
             borra datos locales."
                .into(),
        );
    }

    let fingerprint = plan_fingerprint(
        &package.fingerprint,
        &items,
        &state,
        &numbers,
        &columns,
        &agents,
    )?;
    Ok(Plan {
        preview: ImportPreview {
            fingerprint,
            project_id: manifest.project.id.clone(),
            project_name: project_name.clone(),
            project_exists,
            items,
            warnings,
        },
        project_exists,
        project_name,
        epics,
        stories,
        numbers,
        columns,
        agents,
        column_limits,
    })
}

fn record(state: &mut Vec<String>, id: &str, hash: String) -> String {
    state.push(format!("{id}:{hash}"));
    hash
}

/// Huella de lo que se aplicaría: paquete + acción de cada elemento + estado local que la
/// decide + números, columnas y agentes asignados. La aplicación la recalcula en su
/// transacción y exige que coincida: se aplica exactamente lo previsualizado.
fn plan_fingerprint(
    package: &str,
    items: &[ImportItem],
    state: &[String],
    numbers: &HashMap<String, i64>,
    columns: &HashMap<String, String>,
    agents: &HashMap<String, String>,
) -> Result<String, CoreError> {
    let sorted = |map: Vec<(String, String)>| {
        let mut map = map;
        map.sort();
        map
    };
    content_hash(&(
        package,
        items.iter().map(|i| (&i.id, i.action)).collect::<Vec<_>>(),
        state,
        sorted(
            numbers
                .iter()
                .map(|(k, v)| (k.clone(), v.to_string()))
                .collect(),
        ),
        sorted(
            columns
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
        sorted(agents.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
    ))
}

fn decide(local: &str, package: &str, base: &str) -> ImportAction {
    if local == package {
        ImportAction::Unchanged
    } else if package == base {
        ImportAction::KeepLocal
    } else if local == base {
        ImportAction::Update
    } else {
        ImportAction::Conflict
    }
}

fn assign_numbers(
    creates: &[(&str, i64, &str)],
    mut taken: HashSet<i64>,
    numbers: &mut HashMap<String, i64>,
    warnings: &mut Vec<String>,
    prefix: &str,
) {
    let mut pending = Vec::new();
    for &(id, number, title) in creates {
        if taken.insert(number) {
            numbers.insert(id.to_owned(), number);
        } else {
            pending.push((id, number, title));
        }
    }
    let first_free = taken.iter().max().copied().unwrap_or(0) + 1;
    for (next, (id, number, title)) in (first_free..).zip(pending) {
        numbers.insert(id.to_owned(), next);
        warnings.push(format!(
            "{prefix}{number} «{title}» se importará como {prefix}{next}."
        ));
    }
}

fn fallback_column(columns: &[(String, ColumnCategory)], wanted: ColumnCategory) -> Option<String> {
    let first = |cat: ColumnCategory| {
        columns
            .iter()
            .find(|(_, c)| *c == cat)
            .map(|(id, _)| id.clone())
    };
    let same = if wanted == ColumnCategory::Done {
        None
    } else {
        first(wanted)
    };
    same.or_else(|| first(ColumnCategory::Review)).or_else(|| {
        columns
            .iter()
            .find(|(_, c)| *c != ColumnCategory::Done)
            .map(|(id, _)| id.clone())
    })
}

/// Un id del paquete no puede pertenecer ya a *otro* proyecto ni a otra historia.
async fn ensure_ids_free(
    conn: &mut SqliteConnection,
    package: &Package,
    project_id: &str,
) -> Result<(), CoreError> {
    let collision = |what: &str| {
        invalid(format!(
            "El identificador de {what} ya se usa en otro proyecto."
        ))
    };
    for doc in &package.epics {
        let owner: Option<String> = sqlx::query_scalar("SELECT project_id FROM epics WHERE id = ?")
            .bind(&doc.entry.id)
            .fetch_optional(&mut *conn)
            .await?;
        if owner.is_some_and(|p| p != project_id) {
            return Err(collision(&format!("la épica E-{}", doc.header.number)));
        }
    }
    for doc in &package.stories {
        let owner: Option<String> =
            sqlx::query_scalar("SELECT project_id FROM stories WHERE id = ?")
                .bind(&doc.entry.id)
                .fetch_optional(&mut *conn)
                .await?;
        if owner.is_some_and(|p| p != project_id) {
            return Err(collision(&format!("la historia #{}", doc.header.number)));
        }
        for criterion in &doc.header.criteria {
            let Some(id) = &criterion.id else { continue };
            let story: Option<String> =
                sqlx::query_scalar("SELECT story_id FROM acceptance_criteria WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&mut *conn)
                    .await?;
            if story.is_some_and(|s| s != doc.entry.id) {
                return Err(invalid(format!(
                    "{}: un criterio usa un identificador de otra historia.",
                    doc.entry.file
                )));
            }
        }
    }
    Ok(())
}

async fn validate_new_columns(
    conn: &mut SqliteConnection,
    columns: &[ManifestColumn],
) -> Result<(), CoreError> {
    use crate::features::columns::{validate_column_name, validate_wip};
    if !columns.iter().any(|c| c.category != ColumnCategory::Done) {
        return Err(invalid(
            "El paquete necesita al menos una columna que no sea «hecho».",
        ));
    }
    for column in columns {
        validate_column_name(&column.name).map_err(prefixed("manifest.json"))?;
        validate_wip(column.wip_limit).map_err(prefixed("manifest.json"))?;
        let used: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM columns WHERE id = ?)")
            .bind(&column.id)
            .fetch_one(&mut *conn)
            .await?;
        if used {
            return Err(invalid(
                "El identificador de una columna ya se usa en otro proyecto.",
            ));
        }
    }
    Ok(())
}

/// Errores de validación con el archivo de origen delante.
pub(super) fn prefixed(file: &str) -> impl Fn(CoreError) -> CoreError + '_ {
    move |error| match error {
        CoreError::Validation { reason, .. } => invalid(format!("{file}: {reason}")),
        other => other,
    }
}

/// Columnas locales del proyecto ya existente (para aplicar WIP al crear).
pub(super) fn find_column<'c>(columns: &'c [Column], id: &str) -> Option<&'c Column> {
    columns.iter().find(|c| c.id == id)
}
