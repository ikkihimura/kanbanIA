//! HU-016 (CA-2): el paquete de demostración versionado (`examples/demo/demo-recetario`, generado
//! con `cargo run -p kanbania-core --example demo_package -- examples/demo`) se importa en un
//! perfil nuevo, está claramente separado de los datos reales y no simula estados de IA.
//! Si cambia el formato de portabilidad y el paquete deja de importarse, esta prueba falla:
//! regenéralo con el ejemplo. Base SQLite temporal real.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use kanbania_core::features::columns::{ColumnCategory, list_columns};
use kanbania_core::features::epics::list_epics;
use kanbania_core::features::portability::{
    ApplyImportInput, ImportAction, apply_import, preview_import,
};
use kanbania_core::features::projects::list_projects;
use kanbania_core::features::stories::{get_story, list_stories};
use kanbania_core::features::team::list_agents;
use kanbania_core::platform::actor::Actor;
use kanbania_core::platform::storage::Store;

const STORIES: usize = 9;
const EPICS: usize = 2;

fn package() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/demo/demo-recetario")
        .canonicalize()
        .expect("falta el paquete de demostración versionado")
        .to_string_lossy()
        .into_owned()
}

async fn import(store: &Store) -> (kanbania_core::features::portability::ImportPreview, i64) {
    let path = package();
    let preview = preview_import(store, &Actor::Human, &path).await.unwrap();
    let result = apply_import(
        store,
        &Actor::Human,
        &ApplyImportInput {
            path,
            fingerprint: preview.fingerprint.clone(),
            resolutions: vec![],
        },
    )
    .await
    .unwrap();
    assert_eq!(result.project_id, preview.project_id);
    (preview, result.created)
}

async fn count(store: &Store, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(store.reader())
        .await
        .unwrap()
}

#[tokio::test]
async fn the_demo_package_imports_cleanly_without_simulated_ai_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).await.unwrap();

    let (preview, created) = import(&store).await;
    assert!(!preview.project_exists);
    assert!(preview.warnings.is_empty(), "{:?}", preview.warnings);
    assert!(
        preview
            .items
            .iter()
            .all(|i| i.action == ImportAction::Create)
    );
    assert_eq!(preview.items.len(), STORIES + EPICS);
    assert_eq!(created, i64::try_from(STORIES + EPICS).unwrap());

    // Separado de los datos reales: un proyecto propio cuyo nombre lo delata.
    let projects = list_projects(&store).await.unwrap();
    assert_eq!(projects.len(), 1);
    assert!(
        projects[0].name.starts_with("Demo · "),
        "{}",
        projects[0].name
    );
    let project_id = &preview.project_id;

    let stories = list_stories(&store, project_id, true).await.unwrap();
    assert_eq!(stories.len(), STORIES);
    assert_eq!(list_epics(&store, project_id).await.unwrap().len(), EPICS);
    let columns = list_columns(&store, project_id).await.unwrap();
    let category = |column_id: &str| {
        columns
            .iter()
            .find(|c| c.id == column_id)
            .map(|c| c.category)
            .unwrap()
    };
    for story in &stories {
        assert!(story.archived_at.is_none(), "#{}", story.number);
        assert!(story.accepted_at.is_none(), "#{}", story.number);
        assert!(story.agent_id.is_none(), "#{}", story.number);
        assert!(story.blocker_reason.is_none(), "#{}", story.number);
        assert!(story.criteria_count > 0, "#{} sin criterios", story.number);
        assert!(
            matches!(
                category(&story.column_id),
                ColumnCategory::Backlog | ColumnCategory::Ready | ColumnCategory::Active
            ),
            "#{}",
            story.number
        );
        let detail = get_story(&store, project_id, &story.id).await.unwrap();
        assert!(
            detail.description.contains("paquete de demostración"),
            "#{}: debe decir que es contenido de ejemplo",
            story.number
        );
    }
    // Varias columnas, una subtarea y prerequisitos (los pendientes cuentan).
    for wanted in [
        ColumnCategory::Backlog,
        ColumnCategory::Ready,
        ColumnCategory::Active,
    ] {
        assert!(stories.iter().any(|s| category(&s.column_id) == wanted));
    }
    assert_eq!(stories.iter().filter(|s| s.parent_id.is_some()).count(), 1);
    assert!(stories.iter().any(|s| s.pending_prerequisites > 0));

    // Sin estados de IA: ni agentes ni nada que los simule.
    assert!(list_agents(&store).await.unwrap().is_empty());
    for table in [
        "SELECT COUNT(*) FROM agents",
        "SELECT COUNT(*) FROM story_agents",
        "SELECT COUNT(*) FROM agent_credentials",
        "SELECT COUNT(*) FROM story_claims",
        "SELECT COUNT(*) FROM deliveries",
        "SELECT COUNT(*) FROM story_blockers",
        "SELECT COUNT(*) FROM acceptance_events",
        "SELECT COUNT(*) FROM agent_activity",
        "SELECT COUNT(*) FROM agent_created_stories",
        "SELECT COUNT(*) FROM repository_links",
    ] {
        assert_eq!(count(&store, table).await, 0, "{table}");
    }

    // Reimportar no duplica nada.
    let (again, created) = import(&store).await;
    assert!(again.project_exists);
    assert_eq!(created, 0);
    assert!(
        again
            .items
            .iter()
            .all(|i| i.action == ImportAction::Unchanged)
    );
    assert_eq!(list_projects(&store).await.unwrap().len(), 1);
    assert_eq!(
        list_stories(&store, project_id, true).await.unwrap().len(),
        STORIES
    );
}
