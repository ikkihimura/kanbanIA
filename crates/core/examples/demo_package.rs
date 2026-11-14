//! Genera el paquete de demostración (HU-016, CA-2): un proyecto «Demo · Recetario» exportado
//! como carpeta Markdown importable desde la app (Importar proyecto).
//!
//! ```text
//! cargo run -p kanbania-core --example demo_package -- <carpeta destino>
//! ```
//!
//! Crea un perfil de datos temporal (nunca toca los datos reales), construye el proyecto con los
//! casos de uso públicos del núcleo y lo exporta a `<carpeta destino>/demo-recetario`.
//!
//! Solo contiene planificación hecha por personas: historias, criterios, épicas, una subtarea y
//! dependencias. **Sin** agentes, asignaciones, credenciales, reclamos, entregas, bloqueos,
//! aceptaciones ni actividad de agentes: el paquete no simula estados de IA. Los ids (y la fecha
//! del manifiesto) cambian en cada ejecución; el contenido es siempre el mismo.

use std::error::Error;
use std::path::PathBuf;

use kanbania_core::features::columns::{ColumnCategory, list_columns};
use kanbania_core::features::epics::{CreateEpicInput, create_epic};
use kanbania_core::features::portability::{ExportProjectInput, export_project};
use kanbania_core::features::projects::{CreateProjectInput, create_project};
use kanbania_core::features::relations::{
    DependencyInput, SetStoryEpicInput, SetStoryParentInput, add_story_dependency, set_story_epic,
    set_story_parent,
};
use kanbania_core::features::stories::{
    CreateStoryInput, CriterionDraft, Priority, UpdateStoryInput, create_story, update_story,
};
use kanbania_core::platform::actor::Actor;
use kanbania_core::platform::storage::Store;

/// Nombre del proyecto: el prefijo «Demo · » lo distingue de los proyectos reales.
const PROJECT_NAME: &str = "Demo · Recetario";
/// Carpeta del paquete dentro del destino (estable, para versionarla).
const FOLDER: &str = "demo-recetario";
const ACTOR: &str = "demo";
const NOTE: &str =
    "_Historia de ejemplo del paquete de demostración de Kanban IA: no es trabajo real._";

#[derive(Clone, Copy)]
enum Col {
    Backlog,
    Ready,
    Active,
}

#[derive(Clone, Copy)]
enum Epic {
    Recipes,
    Shopping,
}

struct StorySpec {
    title: &'static str,
    description: &'static str,
    priority: Priority,
    column: Col,
    epic: Option<Epic>,
    criteria: &'static [&'static str],
}

const STORIES: &[StorySpec] = &[
    StorySpec {
        title: "Crear una receta con ingredientes y pasos",
        description: "Como cocinero aficionado quiero guardar mis recetas con su lista de \
                      ingredientes (cantidad y unidad) y los pasos numerados, para no depender \
                      de notas sueltas.",
        priority: Priority::High,
        column: Col::Active,
        epic: Some(Epic::Recipes),
        criteria: &[
            "Se puede guardar una receta con título, raciones, ingredientes y pasos.",
            "Un ingrediente sin cantidad se guarda como «al gusto».",
            "Al reabrir la app, la receta sigue ahí sin cambios.",
        ],
    },
    StorySpec {
        title: "Buscar recetas por ingrediente",
        description: "Como usuario quiero escribir un ingrediente que tengo en la nevera y ver \
                      las recetas que lo usan.",
        priority: Priority::Medium,
        column: Col::Ready,
        epic: Some(Epic::Recipes),
        criteria: &[
            "La búsqueda no distingue mayúsculas ni tildes («limon» encuentra «limón»).",
            "Sin resultados, se muestra un mensaje que sugiere otra búsqueda.",
        ],
    },
    StorySpec {
        title: "Escalar las raciones de una receta",
        description: "Como usuario quiero cambiar el número de raciones y que las cantidades \
                      se recalculen solas.",
        priority: Priority::Medium,
        column: Col::Ready,
        epic: Some(Epic::Recipes),
        criteria: &[
            "Duplicar las raciones duplica cada cantidad.",
            "Las cantidades se redondean a una precisión legible (p. ej. 1,5 huevos → 2).",
        ],
    },
    StorySpec {
        title: "Etiquetar recetas (vegetariana, rápida, sin gluten)",
        description: "Como usuario quiero marcar recetas con etiquetas y filtrar por ellas.",
        priority: Priority::Low,
        column: Col::Backlog,
        epic: Some(Epic::Recipes),
        criteria: &[
            "Una receta puede tener varias etiquetas.",
            "El filtro por etiqueta se combina con la búsqueda por ingrediente.",
        ],
    },
    StorySpec {
        title: "Planificar el menú semanal",
        description: "Como usuario quiero asignar recetas a cada día de la semana (comida y \
                      cena).",
        priority: Priority::High,
        column: Col::Ready,
        epic: Some(Epic::Shopping),
        criteria: &[
            "Se puede asignar una receta a cada comida de cada día.",
            "El menú de la semana anterior se puede copiar a la siguiente.",
        ],
    },
    StorySpec {
        title: "Generar la lista de la compra desde el menú",
        description: "Como usuario quiero obtener la lista de la compra de la semana a partir \
                      del menú, sumando los ingredientes repetidos.",
        priority: Priority::High,
        column: Col::Backlog,
        epic: Some(Epic::Shopping),
        criteria: &[
            "Los ingredientes repetidos se suman si tienen la misma unidad.",
            "Se pueden tachar los ingredientes que ya hay en casa.",
        ],
    },
    StorySpec {
        title: "Agrupar la lista de la compra por pasillo",
        description: "Subtarea de la lista de la compra: ordenar los ingredientes por sección \
                      del supermercado (fruta, lácteos, despensa…).",
        priority: Priority::Medium,
        column: Col::Backlog,
        epic: Some(Epic::Shopping),
        criteria: &[
            "Cada ingrediente aparece bajo una sección.",
            "Un ingrediente sin sección conocida va a «Otros».",
        ],
    },
    StorySpec {
        title: "Modo cocina con la pantalla siempre encendida",
        description: "Como usuario quiero una vista de pasos en letra grande que no se apague \
                      mientras cocino.",
        priority: Priority::Medium,
        column: Col::Active,
        epic: None,
        criteria: &[
            "Los pasos se muestran de uno en uno y se avanza con el teclado.",
            "Al salir del modo cocina, la pantalla vuelve a su comportamiento normal.",
        ],
    },
    StorySpec {
        title: "Importar recetas desde un archivo de texto",
        description: "Como usuario quiero pegar o abrir una receta en texto plano y que se \
                      separen ingredientes y pasos.",
        priority: Priority::Low,
        column: Col::Backlog,
        epic: None,
        criteria: &[
            "Una línea que empieza por cantidad se reconoce como ingrediente.",
            "Antes de guardar se muestra una vista previa editable.",
        ],
    },
];

/// Índices en `STORIES`: subtarea → padre y dependiente → prerequisito.
const PARENT: (usize, usize) = (6, 5);
const DEPENDENCIES: &[(usize, usize)] = &[(2, 0), (5, 4)];

fn rid() -> String {
    uuid::Uuid::now_v7().to_string()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let destination = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("Uso: cargo run -p kanbania-core --example demo_package -- <carpeta destino>")?;
    std::fs::create_dir_all(&destination)?;
    let destination = std::fs::canonicalize(&destination)?;
    let target = destination.join(FOLDER);
    if target.exists() {
        return Err(format!(
            "Ya existe {}: bórrala antes de regenerar.",
            target.display()
        )
        .into());
    }

    // Perfil de datos temporal: el paquete nunca sale de los datos reales del usuario.
    let profile = tempfile::tempdir()?;
    let store = Store::open(profile.path()).await?;
    let project = create_project(
        &store,
        ACTOR,
        &CreateProjectInput {
            request_id: rid(),
            name: PROJECT_NAME.into(),
        },
    )
    .await?;
    let columns = list_columns(&store, &project.id).await?;
    let column_id = |category: ColumnCategory| {
        columns
            .iter()
            .find(|c| c.category == category)
            .map(|c| c.id.clone())
            .ok_or("Faltan las columnas por defecto")
    };
    let backlog = column_id(ColumnCategory::Backlog)?;
    let ready = column_id(ColumnCategory::Ready)?;
    let active = column_id(ColumnCategory::Active)?;

    let mut epics = Vec::new();
    for (title, description) in [
        (
            "Recetas",
            "Guardar, buscar y adaptar recetas.\n\n_Épica de ejemplo del paquete de demostración._",
        ),
        (
            "Lista de la compra",
            "Del menú semanal a la compra.\n\n_Épica de ejemplo del paquete de demostración._",
        ),
    ] {
        let epic = create_epic(
            &store,
            ACTOR,
            &CreateEpicInput {
                request_id: rid(),
                project_id: project.id.clone(),
                title: title.into(),
                description: Some(description.into()),
            },
        )
        .await?;
        epics.push(epic.id);
    }

    let mut ids = Vec::new();
    for spec in STORIES {
        let column = match spec.column {
            Col::Backlog => &backlog,
            Col::Ready => &ready,
            Col::Active => &active,
        };
        let created = create_story(
            &store,
            ACTOR,
            &CreateStoryInput {
                request_id: rid(),
                project_id: project.id.clone(),
                title: spec.title.into(),
                column_id: Some(column.clone()),
                priority: Some(spec.priority),
            },
        )
        .await?;
        update_story(
            &store,
            &UpdateStoryInput {
                project_id: project.id.clone(),
                id: created.id.clone(),
                expected_revision: created.revision,
                title: spec.title.into(),
                description: format!("{}\n\n{NOTE}\n", spec.description),
                priority: spec.priority,
                criteria: spec
                    .criteria
                    .iter()
                    .map(|text| CriterionDraft {
                        id: None,
                        text: (*text).into(),
                    })
                    .collect(),
            },
        )
        .await?;
        if let Some(epic) = spec.epic {
            let epic_id = match epic {
                Epic::Recipes => &epics[0],
                Epic::Shopping => &epics[1],
            };
            set_story_epic(
                &store,
                &SetStoryEpicInput {
                    project_id: project.id.clone(),
                    story_id: created.id.clone(),
                    epic_id: Some(epic_id.clone()),
                },
            )
            .await?;
        }
        ids.push(created.id);
    }

    set_story_parent(
        &store,
        &SetStoryParentInput {
            project_id: project.id.clone(),
            story_id: ids[PARENT.0].clone(),
            parent_id: Some(ids[PARENT.1].clone()),
        },
    )
    .await?;
    for &(story, prerequisite) in DEPENDENCIES {
        add_story_dependency(
            &store,
            &DependencyInput {
                project_id: project.id.clone(),
                story_id: ids[story].clone(),
                depends_on_id: ids[prerequisite].clone(),
            },
        )
        .await?;
    }

    // Se exporta a una carpeta temporal del mismo destino y se renombra a un nombre estable.
    let staging = tempfile::tempdir_in(&destination)?;
    let exported = export_project(
        &store,
        &Actor::Human,
        &ExportProjectInput {
            project_id: project.id.clone(),
            destination: staging.path().to_string_lossy().into_owned(),
        },
        env!("CARGO_PKG_VERSION"),
    )
    .await?;
    std::fs::rename(staging.path().join(&exported.folder_name), &target)?;
    store.close().await;

    println!(
        "Paquete de demostración: {} ({} historias, {} épicas)",
        target.display(),
        exported.stories,
        exported.epics
    );
    Ok(())
}
