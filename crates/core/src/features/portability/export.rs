use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::package::{
    CriterionDoc, EpicHeader, FORMAT, MANIFEST, Manifest, ManifestColumn, ManifestEntry,
    ManifestProject, ReviewInfo, StoryHeader, VERSION, content_hash, invalid, slug, write_document,
};
use super::snapshot::load_local;
use crate::error::CoreError;
use crate::features::columns::columns_of_project;
use crate::features::projects::load_project;
use crate::platform::actor::Actor;
use crate::platform::storage::Store;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportProjectInput {
    pub project_id: String,
    /// Carpeta existente donde se crea la carpeta del paquete.
    pub destination: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportResult {
    /// Nombre de la carpeta creada dentro del destino.
    pub folder_name: String,
    #[ts(type = "number")]
    pub stories: i64,
    #[ts(type = "number")]
    pub epics: i64,
}

/// Exporta el proyecto a una carpeta nueva `kanbania-<nombre>-<fecha>` dentro de
/// `destination`. Incluye historias (también archivadas), épicas, criterios, relaciones y
/// columnas. **No** incluye repositorios (rutas absolutas), entregas ni sus referencias,
/// recibos de mutación, el perfil ni datos de agentes salvo id y nombre del asignado.
/// Se escribe en una carpeta temporal que se renombra al final: nunca queda un paquete a medias.
pub async fn export_project(
    store: &Store,
    actor: &Actor,
    input: &ExportProjectInput,
    app_version: &str,
) -> Result<ExportResult, CoreError> {
    actor.require_human("exportar un proyecto")?;
    let destination = PathBuf::from(&input.destination);
    if !destination.is_absolute() {
        return Err(destination_error());
    }

    // Una sola transacción de lectura: instantánea coherente aunque otra escritura llegue a
    // mitad de la exportación. Al soltarse, sqlx hace rollback.
    let mut tx = store.reader().begin().await?;
    let project = load_project(&mut tx, &input.project_id)
        .await?
        .ok_or(CoreError::NotFound { entity: "Proyecto" })?;
    let columns = columns_of_project(&mut tx, &project.id).await?;
    let local = load_local(&mut tx, &project.id).await?;
    let (exported_at, stamp): (String, String) = sqlx::query_as(
        "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y%m%d-%H%M%S', 'now')",
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.rollback().await?;

    let mut files: Vec<(String, String)> = Vec::new();
    let mut epic_entries = Vec::new();
    for epic in &local.epics {
        let file = format!("epics/E-{}-{}.md", epic.number, slug(&epic.title, "epica"));
        let header = EpicHeader {
            id: epic.id.clone(),
            number: epic.number,
            title: epic.title.clone(),
        };
        files.push((file.clone(), write_document(&header, &epic.description)?));
        epic_entries.push(ManifestEntry {
            id: epic.id.clone(),
            file,
            revision: epic.revision,
            content_hash: content_hash(&epic.content())?,
        });
    }
    let mut story_entries = Vec::new();
    for story in &local.stories {
        let file = format!(
            "stories/{:04}-{}.md",
            story.number,
            slug(&story.title, "historia")
        );
        let review = (story.accepted_at.is_some()
            || story.agent.is_some()
            || story.blocker.is_some())
        .then(|| ReviewInfo {
            accepted_at: story.accepted_at.clone(),
            agent_id: story.agent.as_ref().map(|(id, _)| id.clone()),
            agent_name: story.agent.as_ref().map(|(_, name)| name.clone()),
            blocker: story.blocker.clone(),
        });
        let header = StoryHeader {
            id: story.id.clone(),
            number: story.number,
            title: story.title.clone(),
            priority: story.priority,
            column: story.column_id.clone(),
            position: story.position,
            archived: story.archived,
            epic: story.epic.clone(),
            parent: story.parent.clone(),
            depends_on: story.depends_on.clone(),
            next_criterion_number: story.next_criterion_number,
            criteria: story
                .criteria
                .iter()
                .map(|c| CriterionDoc {
                    id: Some(c.id.clone()),
                    number: Some(c.number),
                    text: c.text.clone(),
                })
                .collect(),
            review,
        };
        files.push((file.clone(), write_document(&header, &story.description)?));
        story_entries.push(ManifestEntry {
            id: story.id.clone(),
            file,
            revision: story.revision,
            content_hash: story.content().hash()?,
        });
    }
    let manifest = Manifest {
        format: FORMAT.into(),
        version: VERSION,
        exported_at,
        app_version: app_version.into(),
        project: ManifestProject {
            id: project.id.clone(),
            name: project.name.clone(),
            revision: project.revision,
        },
        columns: columns
            .iter()
            .map(|c| ManifestColumn {
                id: c.id.clone(),
                name: c.name.clone(),
                category: c.category,
                position: c.position,
                wip_limit: c.wip_limit,
            })
            .collect(),
        epics: epic_entries,
        stories: story_entries,
    };
    files.push((
        MANIFEST.into(),
        serde_json::to_string_pretty(&manifest)? + "\n",
    ));

    let folder_name = format!("kanbania-{}-{stamp}", slug(&project.name, "proyecto"));
    let result = ExportResult {
        folder_name: folder_name.clone(),
        stories: i64::try_from(local.stories.len()).unwrap_or(i64::MAX),
        epics: i64::try_from(local.epics.len()).unwrap_or(i64::MAX),
    };
    tokio::task::spawn_blocking(move || write_package(&destination, &folder_name, &files))
        .await
        .map_err(|_| invalid("No se pudo escribir el paquete."))??;
    Ok(result)
}

fn destination_error() -> CoreError {
    CoreError::Validation {
        field: "destination",
        reason: "Elige una carpeta existente en la que se pueda escribir.".into(),
    }
}

fn write_package(
    destination: &Path,
    folder: &str,
    files: &[(String, String)],
) -> Result<(), CoreError> {
    if !std::fs::metadata(destination).is_ok_and(|m| m.is_dir()) {
        return Err(destination_error());
    }
    let target = destination.join(folder);
    if target.exists() {
        return Err(CoreError::Validation {
            field: "destination",
            reason: format!("Ya existe una carpeta «{folder}» en el destino."),
        });
    }
    let temp = destination.join(format!(".{folder}.partial"));
    // Si la carpeta temporal ya existía no es nuestra: no se toca.
    if std::fs::create_dir(&temp).is_err() {
        return Err(destination_error());
    }
    let written = (|| -> std::io::Result<()> {
        std::fs::create_dir(temp.join("stories"))?;
        std::fs::create_dir(temp.join("epics"))?;
        for (name, content) in files {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(temp.join(name))?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
        }
        std::fs::rename(&temp, &target)
    })();
    if written.is_err() {
        // Sin restos: o el paquete completo o nada.
        if std::fs::remove_dir_all(&temp).is_err() {
            return Err(CoreError::Validation {
                field: "destination",
                reason: format!(
                    "La exportación falló y quedó una carpeta temporal «.{folder}.partial»."
                ),
            });
        }
        return Err(destination_error());
    }
    Ok(())
}
