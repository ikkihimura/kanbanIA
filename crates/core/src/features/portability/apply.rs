use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use ts_rs::TS;

use super::package::{Package, invalid, read_package};
use super::plan::{ImportAction, ImportPreview, PackageStory, Plan, build_plan, find_column};
use crate::error::CoreError;
use crate::features::columns::{
    ColumnCategory, columns_of_project, ensure_capacity, validate_column_name,
};
use crate::features::relations::{is_self_or_ancestor, reaches_through_dependencies};
use crate::features::stories::{Document, count_in_column, replace_document};
use crate::platform::actor::Actor;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ResolutionChoice {
    /// Conservar la versión local.
    Local,
    /// Usar la versión del paquete.
    Package,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportResolution {
    pub id: String,
    pub choice: ResolutionChoice,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApplyImportInput {
    pub path: String,
    /// Huella de la vista previa: si el paquete cambió desde entonces, no se aplica.
    pub fingerprint: String,
    /// Una por cada conflicto de la vista previa.
    pub resolutions: Vec<ImportResolution>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportResult {
    pub project_id: String,
    #[ts(type = "number")]
    pub created: i64,
    #[ts(type = "number")]
    pub updated: i64,
    #[ts(type = "number")]
    pub unchanged: i64,
    #[ts(type = "number")]
    pub kept_local: i64,
}

async fn load_package(path: &str) -> Result<Package, CoreError> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(invalid("Elige la carpeta del paquete."));
    }
    tokio::task::spawn_blocking(move || read_package(&path))
        .await
        .map_err(|_| invalid("No se pudo leer el paquete."))?
}

/// Lee y valida el paquete y calcula qué pasaría, sin cambiar nada.
pub async fn preview_import(
    store: &Store,
    actor: &Actor,
    path: &str,
) -> Result<ImportPreview, CoreError> {
    actor.require_human("importar un paquete")?;
    let package = load_package(path).await?;
    let mut conn = store.reader().acquire().await?;
    Ok(build_plan(&mut conn, &package).await?.preview)
}

/// Aplica el paquete en **una** transacción: o todo o nada. Recalcula el plan dentro de la
/// transacción y exige la huella de la vista previa: si el paquete o los datos locales que
/// deciden alguna acción cambiaron, o falta resolver un conflicto, falla sin tocar nada.
pub async fn apply_import(
    store: &Store,
    actor: &Actor,
    input: &ApplyImportInput,
) -> Result<ImportResult, CoreError> {
    actor.require_human("importar un paquete")?;
    let package = load_package(&input.path).await?;
    let fingerprint = input.fingerprint.clone();
    let resolutions: HashMap<String, ResolutionChoice> = input
        .resolutions
        .iter()
        .map(|r| (r.id.clone(), r.choice))
        .collect();

    store
        .write(move |conn| {
            Box::pin(async move {
                let plan = build_plan(conn, &package).await?;
                if plan.preview.fingerprint != fingerprint {
                    return Err(stale());
                }
                apply_plan(conn, &package, &plan, &resolutions).await
            })
        })
        .await
}

fn stale() -> CoreError {
    CoreError::Validation {
        field: "fingerprint",
        reason: "El paquete o los datos cambiaron desde la vista previa, o falta resolver algún \
                 conflicto. Vuelve a previsualizar."
            .into(),
    }
}

async fn apply_plan(
    conn: &mut SqliteConnection,
    package: &Package,
    plan: &Plan<'_>,
    resolutions: &HashMap<String, ResolutionChoice>,
) -> Result<ImportResult, CoreError> {
    let project_id = package.manifest.project.id.as_str();
    // Acción efectiva: los conflictos se resuelven con la elección explícita.
    let mut effective: HashMap<&str, ImportAction> = HashMap::new();
    for item in &plan.preview.items {
        let action = match item.action {
            ImportAction::Conflict => match resolutions.get(&item.id) {
                Some(ResolutionChoice::Package) => ImportAction::Update,
                Some(ResolutionChoice::Local) => ImportAction::KeepLocal,
                None => return Err(stale()),
            },
            other => other,
        };
        effective.insert(item.id.as_str(), action);
    }
    let action_of = |id: &str| {
        effective
            .get(id)
            .copied()
            .unwrap_or(ImportAction::Unchanged)
    };

    if !plan.project_exists {
        sqlx::query("INSERT INTO projects (id, name) VALUES (?, ?)")
            .bind(project_id)
            .bind(&plan.project_name)
            .execute(&mut *conn)
            .await?;
        let mut columns: Vec<_> = package.manifest.columns.iter().collect();
        columns.sort_by_key(|c| c.position);
        for (position, column) in (0_i64..).zip(columns) {
            sqlx::query(
                "INSERT INTO columns (id, project_id, name, category, position, wip_limit) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&column.id)
            .bind(project_id)
            .bind(validate_column_name(&column.name)?)
            .bind(column.category.as_str())
            .bind(position)
            .bind(plan.column_limits.get(&column.id).copied().flatten())
            .execute(&mut *conn)
            .await?;
        }
    }

    for (doc, content) in &plan.epics {
        match action_of(&doc.entry.id) {
            ImportAction::Create => {
                sqlx::query(
                    "INSERT INTO epics (id, project_id, number, title, description) \
                     VALUES (?, ?, ?, ?, ?)",
                )
                .bind(&doc.entry.id)
                .bind(project_id)
                .bind(
                    plan.numbers
                        .get(&doc.entry.id)
                        .copied()
                        .unwrap_or(doc.header.number),
                )
                .bind(&content.title)
                .bind(&content.description)
                .execute(&mut *conn)
                .await?;
            }
            ImportAction::Update => {
                sqlx::query(
                    "UPDATE epics SET title = ?, description = ?, revision = revision + 1, \
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE project_id = ? AND id = ?",
                )
                .bind(&content.title)
                .bind(&content.description)
                .bind(project_id)
                .bind(&doc.entry.id)
                .execute(&mut *conn)
                .await?;
            }
            _ => {}
        }
    }

    // Historias nuevas: primero el WIP de cada columna destino (entradas activas).
    let columns = columns_of_project(conn, project_id).await?;
    let creates: Vec<&PackageStory<'_>> = plan
        .stories
        .iter()
        .filter(|s| action_of(&s.doc.entry.id) == ImportAction::Create)
        .collect();
    let mut incoming: HashMap<&str, i64> = HashMap::new();
    for story in &creates {
        if !story.doc.header.archived
            && let Some(column) = plan.columns.get(&story.doc.entry.id)
        {
            *incoming.entry(column.as_str()).or_default() += 1;
        }
    }
    for (column_id, count) in &incoming {
        let column =
            find_column(&columns, column_id).ok_or(CoreError::NotFound { entity: "Columna" })?;
        if column.category == ColumnCategory::Done {
            return Err(invalid(
                "Una historia importada no puede entrar en «hecho» sin aceptarla.",
            ));
        }
        ensure_capacity(conn, column, *count).await?;
    }
    // Orden del tablero del paquete dentro de cada columna.
    let mut ordered = creates.clone();
    ordered.sort_by_key(|s| {
        (
            s.doc.header.position.unwrap_or(i64::MAX),
            s.doc.header.number,
        )
    });
    for story in ordered {
        insert_story(conn, project_id, plan, story).await?;
    }

    for story in &plan.stories {
        if action_of(&story.doc.entry.id) == ImportAction::Update {
            update_story_content(conn, project_id, story).await?;
        }
    }

    // Relaciones de lo creado o actualizado: primero se quitan las aristas viejas y luego se
    // añaden las nuevas comprobando ciclos. Como el grafo final no tiene ciclos si y solo si
    // ninguna inserción los crea, la comprobación arista a arista basta.
    let touched: Vec<&PackageStory<'_>> = plan
        .stories
        .iter()
        .filter(|s| {
            matches!(
                action_of(&s.doc.entry.id),
                ImportAction::Create | ImportAction::Update
            )
        })
        .collect();
    for story in &touched {
        let id = &story.doc.entry.id;
        for sql in [
            "DELETE FROM story_epics WHERE story_id = ?",
            "DELETE FROM story_parents WHERE story_id = ?",
            "DELETE FROM story_dependencies WHERE story_id = ?",
        ] {
            sqlx::query(sql).bind(id).execute(&mut *conn).await?;
        }
    }
    for story in &touched {
        let id = &story.doc.entry.id;
        let label = format!("#{}", story.doc.header.number);
        if let Some(epic) = &story.content.epic {
            sqlx::query("INSERT INTO story_epics (story_id, project_id, epic_id) VALUES (?, ?, ?)")
                .bind(id)
                .bind(project_id)
                .bind(epic)
                .execute(&mut *conn)
                .await?;
        }
        if let Some(parent) = &story.content.parent {
            if is_self_or_ancestor(conn, parent, id).await? {
                return Err(cycle("parentId", &label, "subtareas"));
            }
            sqlx::query(
                "INSERT INTO story_parents (story_id, project_id, parent_id) VALUES (?, ?, ?)",
            )
            .bind(id)
            .bind(project_id)
            .bind(parent)
            .execute(&mut *conn)
            .await?;
        }
        for dependency in &story.content.depends_on {
            if reaches_through_dependencies(conn, dependency, id).await? {
                return Err(cycle("dependsOnId", &label, "dependencias"));
            }
            sqlx::query(
                "INSERT INTO story_dependencies (story_id, depends_on_id, project_id) \
                 VALUES (?, ?, ?)",
            )
            .bind(id)
            .bind(dependency)
            .bind(project_id)
            .execute(&mut *conn)
            .await?;
        }
    }

    let count = |wanted: ImportAction| {
        i64::try_from(effective.values().filter(|a| **a == wanted).count()).unwrap_or(i64::MAX)
    };
    Ok(ImportResult {
        project_id: project_id.to_owned(),
        created: count(ImportAction::Create),
        updated: count(ImportAction::Update),
        unchanged: count(ImportAction::Unchanged),
        kept_local: count(ImportAction::KeepLocal),
    })
}

fn cycle(field: &'static str, label: &str, what: &str) -> CoreError {
    CoreError::Cycle {
        field,
        reason: format!("Las {what} de {label} en el paquete formarían un ciclo."),
    }
}

async fn insert_story(
    conn: &mut SqliteConnection,
    project_id: &str,
    plan: &Plan<'_>,
    story: &PackageStory<'_>,
) -> Result<(), CoreError> {
    let h = &story.doc.header;
    let id = &story.doc.entry.id;
    let column = plan
        .columns
        .get(id)
        .ok_or(CoreError::NotFound { entity: "Columna" })?;
    let position: Option<i64> = if h.archived {
        None
    } else {
        Some(count_in_column(conn, column).await?)
    };

    // Números CA-n: se conservan los del paquete y los que faltan siguen al mayor.
    let mut used: HashSet<i64> = h.criteria.iter().filter_map(|c| c.number).collect();
    let mut next = h
        .next_criterion_number
        .max(used.iter().max().copied().unwrap_or(0) + 1)
        .max(1);
    let mut criteria = Vec::with_capacity(h.criteria.len());
    for (doc, (_, text)) in h.criteria.iter().zip(&story.content.criteria) {
        let number = match doc.number {
            Some(n) => n,
            None => {
                while used.contains(&next) {
                    next += 1;
                }
                used.insert(next);
                next += 1;
                next - 1
            }
        };
        let criterion_id = doc
            .id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
        criteria.push((criterion_id, number, text));
    }

    sqlx::query(
        "INSERT INTO stories (id, project_id, number, column_id, position, title, description, \
         priority, next_criterion_number, archived_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, \
         CASE WHEN ? THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') END)",
    )
    .bind(id)
    .bind(project_id)
    .bind(plan.numbers.get(id).copied().unwrap_or(h.number))
    .bind(column)
    .bind(position)
    .bind(&story.content.title)
    .bind(&story.content.description)
    .bind(story.content.priority.as_str())
    .bind(next)
    .bind(h.archived)
    .execute(&mut *conn)
    .await?;
    for (position, (criterion_id, number, text)) in (0_i64..).zip(criteria) {
        sqlx::query(
            "INSERT INTO acceptance_criteria (id, story_id, number, position, text) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(criterion_id)
        .bind(id)
        .bind(number)
        .bind(position)
        .bind(text)
        .execute(&mut *conn)
        .await?;
    }
    if let Some(agent) = plan.agents.get(id) {
        sqlx::query("INSERT INTO story_agents (story_id, agent_id) VALUES (?, ?)")
            .bind(id)
            .bind(agent)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Sustituye el contenido de una historia existente por el del paquete con la misma función
/// que el editor. Columna, posición, archivo y aceptación no cambian.
async fn update_story_content(
    conn: &mut SqliteConnection,
    project_id: &str,
    story: &PackageStory<'_>,
) -> Result<(), CoreError> {
    replace_document(
        conn,
        project_id,
        &story.doc.entry.id,
        &Document {
            title: &story.content.title,
            description: &story.content.description,
            priority: story.content.priority,
            criteria: &story.content.criteria,
        },
        None,
    )
    .await?;
    Ok(())
}
