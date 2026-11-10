//! Pruebas de HU-004 a través de la API pública del núcleo, con SQLite temporal real.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use std::path::Path;

use kanbania_core::error::{CoreError, ErrorCode};
use kanbania_core::features::columns::{
    Column, ColumnCategory, CreateColumnInput, DeleteColumnInput, ReorderColumnInput,
    UpdateColumnInput, create_column, delete_column, list_columns, reorder_column, update_column,
};
use kanbania_core::features::projects::{CreateProjectInput, Project, create_project};
use kanbania_core::features::stories::{
    CreateStoryInput, MoveStoryInput, StorySummary, create_story, list_stories, move_story,
};
use kanbania_core::platform::storage::Store;

const ACTOR: &str = "ui";

fn rid() -> String {
    uuid::Uuid::now_v7().to_string()
}

async fn project(store: &Store, name: &str) -> Project {
    create_project(
        store,
        ACTOR,
        &CreateProjectInput {
            request_id: rid(),
            name: name.into(),
        },
    )
    .await
    .unwrap()
}

async fn columns(store: &Store, project: &Project) -> Vec<Column> {
    list_columns(store, &project.id).await.unwrap()
}

async fn column_named(store: &Store, project: &Project, name: &str) -> Column {
    columns(store, project)
        .await
        .into_iter()
        .find(|c| c.name == name)
        .unwrap()
}

async fn card(store: &Store, project: &Project, column: &Column, title: &str) -> StorySummary {
    create_story(
        store,
        ACTOR,
        &CreateStoryInput {
            request_id: rid(),
            project_id: project.id.clone(),
            title: title.into(),
            column_id: Some(column.id.clone()),
            priority: None,
        },
    )
    .await
    .unwrap()
}

async fn set_wip(store: &Store, column: &Column, limit: Option<i64>) -> Column {
    update_column(
        store,
        &UpdateColumnInput {
            project_id: column.project_id.clone(),
            id: column.id.clone(),
            expected_revision: column.revision,
            name: column.name.clone(),
            category: column.category,
            wip_limit: limit,
        },
    )
    .await
    .unwrap()
}

fn move_input(story: &StorySummary, to: &Column, index: i64) -> MoveStoryInput {
    MoveStoryInput {
        project_id: story.project_id.clone(),
        id: story.id.clone(),
        expected_revision: story.revision,
        to_column_id: to.id.clone(),
        to_index: index,
    }
}

async fn titles_in(store: &Store, project: &Project, column: &Column) -> Vec<String> {
    list_stories(store, &project.id, false)
        .await
        .unwrap()
        .into_iter()
        .filter(|s| s.column_id == column.id)
        .map(|s| s.title)
        .collect()
}

/// Invariante: posiciones densas 0..n en las columnas del proyecto y en cada columna.
async fn assert_dense(store: &Store, project: &Project) {
    let cols = columns(store, project).await;
    let positions: Vec<i64> = cols.iter().map(|c| c.position).collect();
    assert_eq!(
        positions,
        (0..cols.len() as i64).collect::<Vec<_>>(),
        "columnas"
    );
    let stories = list_stories(store, &project.id, false).await.unwrap();
    for column in &cols {
        let mut in_column: Vec<i64> = stories
            .iter()
            .filter(|s| s.column_id == column.id)
            .filter_map(|s| s.position)
            .collect();
        in_column.sort_unstable();
        assert_eq!(
            in_column,
            (0..in_column.len() as i64).collect::<Vec<_>>(),
            "tarjetas de «{}»",
            column.name
        );
        assert_eq!(column.card_count, in_column.len() as i64);
    }
}

fn names(columns: &[Column]) -> Vec<&str> {
    columns.iter().map(|c| c.name.as_str()).collect()
}

// ---------- CA-1: crear, renombrar, ordenar; categoría independiente del nombre ----------

#[tokio::test]
async fn new_project_gets_initial_flow() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store, "Flujo").await;

    let cols = columns(&store, &p).await;
    assert_eq!(
        names(&cols),
        ["Backlog", "Listo", "En curso", "Revisión", "Hecho"]
    );
    let categories: Vec<_> = cols.iter().map(|c| c.category).collect();
    assert_eq!(
        categories,
        [
            ColumnCategory::Backlog,
            ColumnCategory::Ready,
            ColumnCategory::Active,
            ColumnCategory::Review,
            ColumnCategory::Done
        ]
    );
    let positions: Vec<_> = cols.iter().map(|c| c.position).collect();
    assert_eq!(positions, [0, 1, 2, 3, 4]);
}

#[tokio::test]
async fn migration_seeds_initial_flow_for_existing_projects() {
    let data = tempfile::tempdir().unwrap();
    // Base en la versión 2 (antes de HU-004) con un proyecto existente.
    {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(data.path().join("kanbania.sqlite3"))
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
        let migrator =
            sqlx::migrate::Migrator::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"))
                .await
                .unwrap();
        migrator.run_to(2, &pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id, name) VALUES ('legacy', 'Antiguo')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    let store = Store::open(data.path()).await.unwrap();
    let cols = list_columns(&store, "legacy").await.unwrap();
    assert_eq!(
        names(&cols),
        ["Backlog", "Listo", "En curso", "Revisión", "Hecho"]
    );
}

#[tokio::test]
async fn create_rename_recategorize_and_reorder_persist_after_restart() {
    let data = tempfile::tempdir().unwrap();
    let p = {
        let store = Store::open(data.path()).await.unwrap();
        let p = project(&store, "Orden").await;
        let qa = create_column(
            &store,
            ACTOR,
            &CreateColumnInput {
                request_id: rid(),
                project_id: p.id.clone(),
                name: "QA ✅".into(),
                category: ColumnCategory::Review,
                wip_limit: Some(2),
            },
        )
        .await
        .unwrap();
        assert_eq!(qa.position, 5);

        // El nombre es libre; la categoría decide las reglas.
        let renamed = update_column(
            &store,
            &UpdateColumnInput {
                project_id: p.id.clone(),
                id: qa.id.clone(),
                expected_revision: qa.revision,
                name: "Pruebas".into(),
                category: ColumnCategory::Active,
                wip_limit: Some(2),
            },
        )
        .await
        .unwrap();
        assert_eq!(renamed.category, ColumnCategory::Active);

        reorder_column(
            &store,
            &ReorderColumnInput {
                project_id: p.id.clone(),
                id: renamed.id.clone(),
                expected_revision: renamed.revision,
                to_index: 2,
            },
        )
        .await
        .unwrap();
        store.close().await;
        p
    };

    let store = Store::open(data.path()).await.unwrap();
    let cols = columns(&store, &p).await;
    assert_eq!(
        names(&cols),
        [
            "Backlog",
            "Listo",
            "Pruebas",
            "En curso",
            "Revisión",
            "Hecho"
        ]
    );
    let positions: Vec<_> = cols.iter().map(|c| c.position).collect();
    assert_eq!(positions, [0, 1, 2, 3, 4, 5], "posiciones densas");
    assert_eq!(cols[2].wip_limit, Some(2));
    assert_dense(&store, &p).await;
}

#[tokio::test]
async fn reorder_moves_first_to_last_and_validates() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store, "Reordenar").await;
    let backlog = column_named(&store, &p, "Backlog").await;
    let input = |to_index, revision| ReorderColumnInput {
        project_id: p.id.clone(),
        id: backlog.id.clone(),
        expected_revision: revision,
        to_index,
    };

    let out_of_range = reorder_column(&store, &input(5, 1)).await.unwrap_err();
    assert_eq!(out_of_range.code(), ErrorCode::Validation);

    reorder_column(&store, &input(4, 1)).await.unwrap();
    assert_eq!(
        names(&columns(&store, &p).await),
        ["Listo", "En curso", "Revisión", "Hecho", "Backlog"]
    );
    assert_dense(&store, &p).await;

    let stale = reorder_column(&store, &input(0, 1)).await.unwrap_err();
    assert_eq!(stale.code(), ErrorCode::Conflict);
}

#[tokio::test]
async fn column_inputs_are_validated() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store, "Validar").await;
    let create = |name: &str, wip: Option<i64>| CreateColumnInput {
        request_id: rid(),
        project_id: p.id.clone(),
        name: name.into(),
        category: ColumnCategory::Active,
        wip_limit: wip,
    };

    for (input, field) in [
        (create("  ", None), "name"),
        (create("Ok", Some(0)), "wipLimit"),
        (create("Ok", Some(1000)), "wipLimit"),
    ] {
        let payload = create_column(&store, ACTOR, &input)
            .await
            .unwrap_err()
            .to_payload(None);
        assert_eq!(payload.code, ErrorCode::Validation);
        assert_eq!(payload.field.as_deref(), Some(field));
    }
}

// ---------- CA-2: eliminar columna ocupada con destino explícito y atómico ----------

#[tokio::test]
async fn delete_occupied_column_requires_destination_and_moves_cards_in_order() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store, "Eliminar").await;
    let ready = column_named(&store, &p, "Listo").await;
    let backlog = column_named(&store, &p, "Backlog").await;
    card(&store, &p, &backlog, "B0").await;
    for title in ["L0", "L1", "L2"] {
        card(&store, &p, &ready, title).await;
    }
    let ready = column_named(&store, &p, "Listo").await;

    let missing = delete_column(
        &store,
        &DeleteColumnInput {
            project_id: p.id.clone(),
            id: ready.id.clone(),
            expected_revision: ready.revision,
            destination_column_id: None,
        },
    )
    .await
    .unwrap_err()
    .to_payload(None);
    assert_eq!(missing.code, ErrorCode::Validation);
    assert_eq!(missing.field.as_deref(), Some("destinationColumnId"));
    assert!(
        missing.message.contains("3 tarjetas"),
        "{}",
        missing.message
    );

    delete_column(
        &store,
        &DeleteColumnInput {
            project_id: p.id.clone(),
            id: ready.id.clone(),
            expected_revision: ready.revision,
            destination_column_id: Some(backlog.id.clone()),
        },
    )
    .await
    .unwrap();

    assert_eq!(
        titles_in(&store, &p, &backlog).await,
        ["B0", "L0", "L1", "L2"]
    );
    let positions: Vec<_> = list_stories(&store, &p.id, false)
        .await
        .unwrap()
        .iter()
        .filter_map(|s| s.position)
        .collect();
    assert_eq!(positions, [0, 1, 2, 3]);
    let cols = columns(&store, &p).await;
    assert_eq!(names(&cols), ["Backlog", "En curso", "Revisión", "Hecho"]);
    assert_eq!(
        cols.iter().map(|c| c.position).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    assert_dense(&store, &p).await;
}

#[tokio::test]
async fn failed_delete_rolls_back_everything() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store, "Rollback").await;
    let ready = column_named(&store, &p, "Listo").await;
    let active = column_named(&store, &p, "En curso").await;
    let done = column_named(&store, &p, "Hecho").await;
    for title in ["A", "B"] {
        card(&store, &p, &ready, title).await;
    }
    let active = set_wip(&store, &active, Some(1)).await;
    let ready = column_named(&store, &p, "Listo").await;
    let before = (
        columns(&store, &p).await,
        list_stories(&store, &p.id, false).await.unwrap(),
    );

    let delete_into = |destination: &Column| DeleteColumnInput {
        project_id: p.id.clone(),
        id: ready.id.clone(),
        expected_revision: ready.revision,
        destination_column_id: Some(destination.id.clone()),
    };

    // Destino con WIP insuficiente: 2 tarjetas no caben en un límite de 1.
    let wip = delete_column(&store, &delete_into(&active))
        .await
        .unwrap_err();
    assert_eq!(wip.code(), ErrorCode::WipLimitExceeded);
    // Destino «hecho»: exige aceptación.
    let accept = delete_column(&store, &delete_into(&done))
        .await
        .unwrap_err();
    assert_eq!(accept.code(), ErrorCode::AcceptanceRequired);

    let after = (
        columns(&store, &p).await,
        list_stories(&store, &p.id, false).await.unwrap(),
    );
    assert_eq!(after, before, "nada cambió tras los fallos");
    assert_dense(&store, &p).await;
}

#[tokio::test]
async fn at_least_one_usable_column_remains() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store, "Mínimo").await;
    for name in ["Listo", "En curso", "Revisión"] {
        let c = column_named(&store, &p, name).await;
        delete_column(
            &store,
            &DeleteColumnInput {
                project_id: p.id.clone(),
                id: c.id,
                expected_revision: c.revision,
                destination_column_id: None,
            },
        )
        .await
        .unwrap();
    }
    let backlog = column_named(&store, &p, "Backlog").await;

    let delete = delete_column(
        &store,
        &DeleteColumnInput {
            project_id: p.id.clone(),
            id: backlog.id.clone(),
            expected_revision: backlog.revision,
            destination_column_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(delete.code(), ErrorCode::Validation);

    let to_done = update_column(
        &store,
        &UpdateColumnInput {
            project_id: p.id.clone(),
            id: backlog.id.clone(),
            expected_revision: backlog.revision,
            name: "Backlog".into(),
            category: ColumnCategory::Done,
            wip_limit: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(to_done.code(), ErrorCode::Validation);
    assert_eq!(names(&columns(&store, &p).await), ["Backlog", "Hecho"]);
}

#[tokio::test]
async fn occupied_column_cannot_switch_to_or_from_done() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store, "Categorías").await;
    let review = column_named(&store, &p, "Revisión").await;
    card(&store, &p, &review, "en revisión").await;
    let review = column_named(&store, &p, "Revisión").await;

    let error = update_column(
        &store,
        &UpdateColumnInput {
            project_id: p.id.clone(),
            id: review.id.clone(),
            expected_revision: review.revision,
            name: "Revisión".into(),
            category: ColumnCategory::Done,
            wip_limit: None,
        },
    )
    .await
    .unwrap_err()
    .to_payload(None);
    assert_eq!(error.code, ErrorCode::Validation);
    assert_eq!(error.field.as_deref(), Some("category"));
}

// ---------- CA-3: WIP en Rust para toda entrada ----------

#[tokio::test]
async fn wip_limit_applies_to_create_and_move_with_explanatory_error() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store, "WIP").await;
    let backlog = column_named(&store, &p, "Backlog").await;
    let active = column_named(&store, &p, "En curso").await;
    let active = set_wip(&store, &active, Some(1)).await;
    card(&store, &p, &active, "ocupa").await;
    let waiting = card(&store, &p, &backlog, "espera").await;

    let create = create_story(
        &store,
        ACTOR,
        &CreateStoryInput {
            request_id: rid(),
            project_id: p.id.clone(),
            title: "no cabe".into(),
            column_id: Some(active.id.clone()),
            priority: None,
        },
    )
    .await
    .unwrap_err()
    .to_payload(None);
    assert_eq!(create.code, ErrorCode::WipLimitExceeded);
    assert!(
        create.message.contains("«En curso»") && create.message.contains("como máximo 1"),
        "{}",
        create.message
    );

    let moved = move_story(&store, &move_input(&waiting, &active, 0))
        .await
        .unwrap_err();
    assert_eq!(moved.code(), ErrorCode::WipLimitExceeded);

    // Reordenar dentro de una columna llena no es una entrada nueva.
    let inside = list_stories(&store, &p.id, false)
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.title == "ocupa")
        .unwrap();
    move_story(&store, &move_input(&inside, &active, 0))
        .await
        .unwrap();
}

#[tokio::test]
async fn lowering_limit_below_current_is_allowed_but_blocks_new_entries() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store, "Bajar límite").await;
    let active = column_named(&store, &p, "En curso").await;
    for title in ["a", "b", "c"] {
        card(&store, &p, &active, title).await;
    }
    let active = column_named(&store, &p, "En curso").await;
    let active = set_wip(&store, &active, Some(2)).await;
    assert_eq!(active.card_count, 3);

    let backlog = column_named(&store, &p, "Backlog").await;
    let extra = card(&store, &p, &backlog, "extra").await;
    let error = move_story(&store, &move_input(&extra, &active, 0))
        .await
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::WipLimitExceeded);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_moves_never_exceed_wip() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store, "Carrera").await;
    let backlog = column_named(&store, &p, "Backlog").await;
    let active = set_wip(&store, &column_named(&store, &p, "En curso").await, Some(1)).await;

    let mut moves = Vec::new();
    for i in 0..6 {
        let story = card(&store, &p, &backlog, &format!("t{i}")).await;
        let store = store.clone();
        let input = move_input(&story, &active, 0);
        moves.push(tokio::spawn(
            async move { move_story(&store, &input).await },
        ));
    }

    let mut ok = 0;
    for handle in moves {
        match handle.await.unwrap() {
            Ok(_) => ok += 1,
            Err(error) => assert_eq!(error.code(), ErrorCode::WipLimitExceeded),
        }
    }
    assert_eq!(ok, 1);
    assert_eq!(titles_in(&store, &p, &active).await.len(), 1);
    assert_dense(&store, &p).await;
}

#[tokio::test]
async fn moving_into_done_requires_acceptance() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store, "Hecho").await;
    let review = column_named(&store, &p, "Revisión").await;
    let done = column_named(&store, &p, "Hecho").await;
    let story = card(&store, &p, &review, "lista").await;

    let error = move_story(&store, &move_input(&story, &done, 0))
        .await
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::AcceptanceRequired);

    let create = create_story(
        &store,
        ACTOR,
        &CreateStoryInput {
            request_id: rid(),
            project_id: p.id.clone(),
            title: "directo".into(),
            column_id: Some(done.id.clone()),
            priority: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(create.code(), ErrorCode::AcceptanceRequired);
}

#[tokio::test]
async fn move_reorders_and_keeps_positions_dense() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let p = project(&store, "Posiciones").await;
    let backlog = column_named(&store, &p, "Backlog").await;
    let ready = column_named(&store, &p, "Listo").await;
    let a = card(&store, &p, &backlog, "A").await;
    card(&store, &p, &backlog, "B").await;
    card(&store, &p, &backlog, "C").await;
    card(&store, &p, &ready, "X").await;

    let a = move_story(&store, &move_input(&a, &backlog, 99))
        .await
        .unwrap();
    assert_eq!(titles_in(&store, &p, &backlog).await, ["B", "C", "A"]);
    assert_eq!(a.position, Some(2), "índice acotado al final");

    move_story(&store, &move_input(&a, &ready, 0))
        .await
        .unwrap();
    assert_eq!(titles_in(&store, &p, &backlog).await, ["B", "C"]);
    assert_eq!(titles_in(&store, &p, &ready).await, ["A", "X"]);
    assert_dense(&store, &p).await;
}

#[tokio::test]
async fn cards_cannot_target_columns_of_other_projects() {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).await.unwrap();
    let mine = project(&store, "Mío").await;
    let other = project(&store, "Ajeno").await;
    let my_backlog = column_named(&store, &mine, "Backlog").await;
    let foreign = column_named(&store, &other, "Backlog").await;
    let story = card(&store, &mine, &my_backlog, "privada").await;

    let error = move_story(&store, &move_input(&story, &foreign, 0))
        .await
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotFound);

    // Y la base lo impide aunque se salte el núcleo (clave foránea compuesta).
    let raw = store
        .write(|conn| {
            let foreign_id = foreign.id.clone();
            let story_id = story.id.clone();
            Box::pin(async move {
                sqlx::query("UPDATE stories SET column_id = ? WHERE id = ?")
                    .bind(foreign_id)
                    .bind(story_id)
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .await;
    assert!(matches!(raw, Err(CoreError::Database(_))));
}
