//! Pruebas de HU-005 a través de la API pública del núcleo, con SQLite temporal real.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use std::path::Path;

use kanbania_core::error::ErrorCode;
use kanbania_core::features::columns::{
    Column, DeleteColumnInput, UpdateColumnInput, delete_column, list_columns, update_column,
};
use kanbania_core::features::projects::{CreateProjectInput, Project, create_project};
use kanbania_core::features::stories::{
    ArchiveStoryInput, CreateStoryInput, CriterionDraft, MoveStoryInput, Priority,
    RestoreStoryInput, StoryDetail, StorySummary, UpdateStoryInput, archive_story, create_story,
    get_story, list_stories, move_story, restore_story, update_story,
};
use kanbania_core::platform::storage::Store;

const ACTOR: &str = "ui";

fn rid() -> String {
    uuid::Uuid::now_v7().to_string()
}

async fn project(store: &Store) -> Project {
    create_project(
        store,
        ACTOR,
        &CreateProjectInput {
            request_id: rid(),
            name: "Historias".into(),
        },
    )
    .await
    .unwrap()
}

async fn column(store: &Store, project: &Project, name: &str) -> Column {
    list_columns(store, &project.id)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name == name)
        .unwrap()
}

async fn story(store: &Store, project: &Project, title: &str) -> StorySummary {
    create_story(
        store,
        ACTOR,
        &CreateStoryInput {
            request_id: rid(),
            project_id: project.id.clone(),
            title: title.into(),
            column_id: None,
            priority: None,
        },
    )
    .await
    .unwrap()
}

fn edit(detail: &StoryDetail) -> UpdateStoryInput {
    UpdateStoryInput {
        project_id: detail.project_id.clone(),
        id: detail.id.clone(),
        expected_revision: detail.revision,
        title: detail.title.clone(),
        description: detail.description.clone(),
        priority: detail.priority,
        criteria: detail
            .criteria
            .iter()
            .map(|c| CriterionDraft {
                id: Some(c.id.clone()),
                text: c.text.clone(),
            })
            .collect(),
    }
}

fn new_criterion(text: &str) -> CriterionDraft {
    CriterionDraft {
        id: None,
        text: text.into(),
    }
}

fn labels(detail: &StoryDetail) -> Vec<String> {
    detail
        .criteria
        .iter()
        .map(|c| format!("CA-{}: {}", c.number, c.text))
        .collect()
}

async fn detail(store: &Store, s: &StorySummary) -> StoryDetail {
    get_story(store, &s.project_id, &s.id).await.unwrap()
}

// ---------- CA-1: crear, editar, archivar y consultar con ID estable y criterios ----------

#[tokio::test]
async fn full_story_survives_restart_with_stable_ids() {
    let data = tempfile::tempdir().unwrap();
    let (project_id, story_id) = {
        let store = Store::open(data.path()).await.unwrap();
        let p = project(&store).await;
        let first = story(&store, &p, "Primera").await;
        let second = story(&store, &p, "Segunda").await;
        assert_eq!((first.number, second.number), (1, 2));
        assert_eq!(second.priority, Priority::Medium);

        let mut input = edit(&detail(&store, &second).await);
        input.title = "Exportar informe".into();
        input.description = "## Contexto\n\nEl **cliente** necesita _CSV_.\n\n- [ ] tarea".into();
        input.priority = Priority::High;
        input.criteria = vec![
            new_criterion("Descarga un CSV válido"),
            new_criterion("Incluye cabeceras en español"),
        ];
        let saved = update_story(&store, &input).await.unwrap();
        assert_eq!(saved.revision, 2);
        store.close().await;
        (p.id, second.id)
    };

    let store = Store::open(data.path()).await.unwrap();
    let reopened = get_story(&store, &project_id, &story_id).await.unwrap();
    assert_eq!(reopened.number, 2);
    assert_eq!(reopened.title, "Exportar informe");
    assert!(reopened.description.contains("**cliente**"));
    assert_eq!(reopened.priority, Priority::High);
    assert_eq!(
        labels(&reopened),
        [
            "CA-1: Descarga un CSV válido",
            "CA-2: Incluye cabeceras en español"
        ]
    );
}

#[tokio::test]
async fn criterion_numbers_are_never_renumbered_or_reused() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store).await;
    let s = story(&store, &p, "Criterios").await;

    let mut input = edit(&detail(&store, &s).await);
    input.criteria = vec![
        new_criterion("uno"),
        new_criterion("dos"),
        new_criterion("tres"),
    ];
    let saved = update_story(&store, &input).await.unwrap();

    // Quitar CA-1, reordenar (CA-3 primero) y añadir uno nuevo.
    let mut input = edit(&saved);
    let ca2 = saved.criteria[1].clone();
    let ca3 = saved.criteria[2].clone();
    input.criteria = vec![
        CriterionDraft {
            id: Some(ca3.id.clone()),
            text: "tres (editado)".into(),
        },
        CriterionDraft {
            id: Some(ca2.id.clone()),
            text: ca2.text.clone(),
        },
        new_criterion("cuatro"),
    ];
    let saved = update_story(&store, &input).await.unwrap();
    assert_eq!(
        labels(&saved),
        ["CA-3: tres (editado)", "CA-2: dos", "CA-4: cuatro"]
    );

    // Borrar el último no libera su número.
    let mut input = edit(&saved);
    input.criteria.pop();
    let saved = update_story(&store, &input).await.unwrap();
    let mut input = edit(&saved);
    input.criteria.push(new_criterion("cinco"));
    let saved = update_story(&store, &input).await.unwrap();
    assert_eq!(saved.criteria.last().unwrap().number, 5);
}

#[tokio::test]
async fn story_inputs_are_validated() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store).await;
    let s = story(&store, &p, "Validar").await;
    let other = story(&store, &p, "Otra").await;
    let mut other_input = edit(&detail(&store, &other).await);
    other_input.criteria = vec![new_criterion("ajeno")];
    let foreign = update_story(&store, &other_input).await.unwrap().criteria[0].clone();
    let base = edit(&detail(&store, &s).await);

    let cases: Vec<(UpdateStoryInput, &str)> = vec![
        (
            UpdateStoryInput {
                title: "   ".into(),
                ..base.clone()
            },
            "title",
        ),
        (
            UpdateStoryInput {
                description: "x".repeat(20_001),
                ..base.clone()
            },
            "description",
        ),
        (
            UpdateStoryInput {
                criteria: (0..51).map(|i| new_criterion(&format!("c{i}"))).collect(),
                ..base.clone()
            },
            "criteria",
        ),
        (
            UpdateStoryInput {
                criteria: vec![CriterionDraft {
                    id: Some(foreign.id.clone()),
                    text: "robado".into(),
                }],
                ..base.clone()
            },
            "criteria",
        ),
        (
            UpdateStoryInput {
                criteria: vec![new_criterion("")],
                ..base.clone()
            },
            "criteria",
        ),
    ];
    for (input, field) in cases {
        let payload = update_story(&store, &input)
            .await
            .unwrap_err()
            .to_payload(None);
        assert_eq!(payload.code, ErrorCode::Validation, "{field}");
        assert_eq!(payload.field.as_deref(), Some(field));
    }
    assert_eq!(
        detail(&store, &s).await.revision,
        1,
        "ningún intento guardó cambios"
    );

    let missing_title = create_story(
        &store,
        ACTOR,
        &CreateStoryInput {
            request_id: rid(),
            project_id: p.id.clone(),
            title: "".into(),
            column_id: None,
            priority: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(missing_title.code(), ErrorCode::Validation);
}

#[tokio::test]
async fn markdown_is_stored_verbatim_as_data() {
    // La sanitización ocurre al renderizar en la UI; el núcleo guarda el texto sin interpretarlo.
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store).await;
    let s = story(&store, &p, "Hostil").await;
    let hostile =
        "<script>alert(1)</script>\n<img src=x onerror=alert(2)>\n[x](javascript:alert(3))";

    let mut input = edit(&detail(&store, &s).await);
    input.description = hostile.into();
    let saved = update_story(&store, &input).await.unwrap();

    assert_eq!(saved.description, hostile);
}

// ---------- CA-2: conflicto de dos editores sin perder texto ----------

#[tokio::test]
async fn second_editor_gets_conflict_with_current_version_and_can_overwrite() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store).await;
    let s = story(&store, &p, "Compartida").await;
    let opened_by_a = detail(&store, &s).await;
    let opened_by_b = detail(&store, &s).await;

    let mut a = edit(&opened_by_a);
    a.description = "Texto de A".into();
    update_story(&store, &a).await.unwrap();

    let mut b = edit(&opened_by_b);
    b.description = "Borrador largo de B".into();
    let conflict = update_story(&store, &b).await.unwrap_err().to_payload(None);
    assert_eq!(conflict.code, ErrorCode::Conflict);
    let current: StoryDetail = serde_json::from_value(conflict.current.unwrap()).unwrap();
    assert_eq!(current.description, "Texto de A");
    assert_eq!(current.revision, 2);

    // B decide conservar su versión: reenvía su borrador sobre la revisión vigente.
    b.expected_revision = current.revision;
    let saved = update_story(&store, &b).await.unwrap();
    assert_eq!(saved.description, "Borrador largo de B");
    assert_eq!(saved.revision, 3);
}

// ---------- CA-3: archivar no destruye; límites ----------

#[tokio::test]
async fn archive_keeps_content_frees_wip_and_restore_reenters() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store).await;
    let backlog = column(&store, &p, "Backlog").await;
    story(&store, &p, "A").await;
    let b = story(&store, &p, "B").await;
    story(&store, &p, "C").await;
    let mut input = edit(&detail(&store, &b).await);
    input.criteria = vec![new_criterion("se conserva")];
    update_story(&store, &input).await.unwrap();

    // Archivar B: sale del orden de la columna y deja de ocupar sitio.
    let b = list_stories(&store, &p.id, false)
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.id == b.id)
        .unwrap();
    let archived = archive_story(
        &store,
        &ArchiveStoryInput {
            project_id: p.id.clone(),
            id: b.id.clone(),
            expected_revision: b.revision,
        },
    )
    .await
    .unwrap();
    assert!(archived.archived_at.is_some());
    assert_eq!(archived.position, None);

    let active = list_stories(&store, &p.id, false).await.unwrap();
    let positions: Vec<_> = active
        .iter()
        .map(|s| (s.title.clone(), s.position))
        .collect();
    assert_eq!(
        positions,
        [("A".to_string(), Some(0)), ("C".to_string(), Some(1))],
        "posiciones compactadas"
    );
    assert_eq!(column(&store, &p, "Backlog").await.card_count, 2);
    let all = list_stories(&store, &p.id, true).await.unwrap();
    assert_eq!(
        all.last().unwrap().id,
        b.id,
        "archivadas al final y solo si se piden"
    );

    // Consultar sigue funcionando y el contenido se conserva.
    let kept = get_story(&store, &p.id, &b.id).await.unwrap();
    assert_eq!(labels(&kept), ["CA-1: se conserva"]);

    // Editar o mover una archivada no está permitido.
    let error = update_story(&store, &edit(&kept)).await.unwrap_err();
    assert_eq!(error.code(), ErrorCode::Validation);
    let error = move_story(
        &store,
        &MoveStoryInput {
            project_id: p.id.clone(),
            id: b.id.clone(),
            expected_revision: kept.revision,
            to_column_id: backlog.id.clone(),
            to_index: 0,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Validation);

    // Restaurar es una entrada: con la columna llena (WIP 2) falla y no cambia nada.
    let backlog = column(&store, &p, "Backlog").await;
    update_column(
        &store,
        &UpdateColumnInput {
            project_id: p.id.clone(),
            id: backlog.id.clone(),
            expected_revision: backlog.revision,
            name: backlog.name.clone(),
            category: backlog.category,
            wip_limit: Some(2),
        },
    )
    .await
    .unwrap();
    let restore = RestoreStoryInput {
        project_id: p.id.clone(),
        id: b.id.clone(),
        expected_revision: kept.revision,
        column_id: None,
    };
    let full = restore_story(&store, &restore).await.unwrap_err();
    assert_eq!(full.code(), ErrorCode::WipLimitExceeded);

    // A una columna elegida, al final.
    let ready = column(&store, &p, "Listo").await;
    let restored = restore_story(
        &store,
        &RestoreStoryInput {
            column_id: Some(ready.id.clone()),
            ..restore
        },
    )
    .await
    .unwrap();
    assert_eq!(restored.archived_at, None);
    assert_eq!(
        (restored.column_id.as_str(), restored.position),
        (ready.id.as_str(), Some(0))
    );
}

#[tokio::test]
async fn restore_into_done_requires_acceptance() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store).await;
    let s = story(&store, &p, "Hecho").await;
    let archived = archive_story(
        &store,
        &ArchiveStoryInput {
            project_id: p.id.clone(),
            id: s.id.clone(),
            expected_revision: s.revision,
        },
    )
    .await
    .unwrap();
    let done = column(&store, &p, "Hecho").await;

    let error = restore_story(
        &store,
        &RestoreStoryInput {
            project_id: p.id.clone(),
            id: s.id.clone(),
            expected_revision: archived.revision,
            column_id: Some(done.id),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::AcceptanceRequired);
}

#[tokio::test]
async fn deleting_a_column_with_only_archived_stories_keeps_them() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store).await;
    let s = story(&store, &p, "Archivada en Backlog").await;
    archive_story(
        &store,
        &ArchiveStoryInput {
            project_id: p.id.clone(),
            id: s.id.clone(),
            expected_revision: s.revision,
        },
    )
    .await
    .unwrap();
    let backlog = column(&store, &p, "Backlog").await;
    assert_eq!(backlog.card_count, 0);

    // Sin tarjetas activas no se pide destino; la archivada se reasigna, no bloquea ni se pierde.
    delete_column(
        &store,
        &DeleteColumnInput {
            project_id: p.id.clone(),
            id: backlog.id.clone(),
            expected_revision: backlog.revision,
            destination_column_id: None,
        },
    )
    .await
    .unwrap();

    let kept = get_story(&store, &p.id, &s.id).await.unwrap();
    assert!(kept.archived_at.is_some());
    assert_ne!(kept.column_id, backlog.id);
    let restored = restore_story(
        &store,
        &RestoreStoryInput {
            project_id: p.id.clone(),
            id: s.id.clone(),
            expected_revision: kept.revision,
            column_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(restored.position, Some(0));
}

#[tokio::test]
async fn migration_numbers_existing_stories_in_creation_order() {
    let data = tempfile::tempdir().unwrap();
    {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(data.path().join("kanbania.sqlite3"))
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
        let migrator =
            sqlx::migrate::Migrator::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"))
                .await
                .unwrap();
        migrator.run_to(3, &pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, name) VALUES ('p', 'Antiguo')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO columns (id, project_id, name, category, position) \
             VALUES ('c', 'p', 'Backlog', 'backlog', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (id, title, position, created) in [
            ("s-b", "Segunda", 1, "2026-01-02T00:00:00.000Z"),
            ("s-a", "Primera", 0, "2026-01-01T00:00:00.000Z"),
        ] {
            sqlx::query(
                "INSERT INTO stories (id, project_id, column_id, position, title, created_at) \
                 VALUES (?, 'p', 'c', ?, ?, ?)",
            )
            .bind(id)
            .bind(position)
            .bind(title)
            .bind(created)
            .execute(&pool)
            .await
            .unwrap();
        }
        pool.close().await;
    }

    let store = Store::open(data.path()).await.unwrap();
    let stories = list_stories(&store, "p", false).await.unwrap();
    let numbered: Vec<_> = stories
        .iter()
        .map(|s| (s.title.as_str(), s.number, s.position))
        .collect();
    assert_eq!(numbered, [("Primera", 1, Some(0)), ("Segunda", 2, Some(1))]);
    let detail = get_story(&store, "p", "s-a").await.unwrap();
    assert_eq!(
        (detail.description.as_str(), detail.priority),
        ("", Priority::Medium)
    );

    // La numeración continúa tras las existentes.
    let p = Project {
        id: "p".into(),
        name: "Antiguo".into(),
        revision: 1,
        created_at: String::new(),
        updated_at: String::new(),
        last_opened_at: None,
    };
    assert_eq!(story(&store, &p, "Tercera").await.number, 3);
}

#[tokio::test]
async fn column_with_archived_stories_cannot_become_done_and_delete_validates_destination() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store).await;
    let s = story(&store, &p, "Archivada").await;
    archive_story(
        &store,
        &ArchiveStoryInput {
            project_id: p.id.clone(),
            id: s.id.clone(),
            expected_revision: s.revision,
        },
    )
    .await
    .unwrap();
    let backlog = column(&store, &p, "Backlog").await;
    assert_eq!(backlog.card_count, 0);

    let to_done = update_column(
        &store,
        &UpdateColumnInput {
            project_id: p.id.clone(),
            id: backlog.id.clone(),
            expected_revision: backlog.revision,
            name: backlog.name.clone(),
            category: kanbania_core::features::columns::ColumnCategory::Done,
            wip_limit: None,
        },
    )
    .await
    .unwrap_err()
    .to_payload(None);
    assert_eq!(to_done.code, ErrorCode::Validation);
    assert_eq!(to_done.field.as_deref(), Some("category"));

    // Destino inexistente: NotFound aunque no haya tarjetas activas.
    let missing = delete_column(
        &store,
        &DeleteColumnInput {
            project_id: p.id.clone(),
            id: backlog.id.clone(),
            expected_revision: backlog.revision,
            destination_column_id: Some("no-existe".into()),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(missing.code(), ErrorCode::NotFound);

    // Destino «hecho»: la archivada va a la primera columna utilizable, no a «hecho».
    let done = column(&store, &p, "Hecho").await;
    delete_column(
        &store,
        &DeleteColumnInput {
            project_id: p.id.clone(),
            id: backlog.id.clone(),
            expected_revision: backlog.revision,
            destination_column_id: Some(done.id.clone()),
        },
    )
    .await
    .unwrap();
    let kept = get_story(&store, &p.id, &s.id).await.unwrap();
    assert_ne!(kept.column_id, done.id);
    assert_eq!(kept.column_id, column(&store, &p, "Listo").await.id);
}
