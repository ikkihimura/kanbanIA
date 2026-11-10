//! HU-007: carga del tablero con 300 tarjetas. Registra tiempos (sin objetivo inventado) y
//! comprueba que el orden sigue denso tras movimientos y reinicio.

// Crate de pruebas: un fallo de preparación debe abortar la prueba.
#![allow(clippy::unwrap_used)]

use std::time::Instant;

use kanbania_core::features::columns::{ColumnCategory, list_columns};
use kanbania_core::features::projects::{CreateProjectInput, create_project};
use kanbania_core::features::stories::{
    CreateStoryInput, MoveStoryInput, create_story, list_stories, move_story,
};
use kanbania_core::platform::storage::Store;

#[tokio::test]
async fn three_hundred_cards_load_move_and_survive_restart() {
    let data = tempfile::tempdir().unwrap();
    let (project_id, ready_id) = {
        let store = Store::open(data.path()).await.unwrap();
        let p = create_project(
            &store,
            "ui",
            &CreateProjectInput {
                request_id: "p".into(),
                name: "Carga".into(),
            },
        )
        .await
        .unwrap();
        let started = Instant::now();
        for i in 0..300 {
            create_story(
                &store,
                "ui",
                &CreateStoryInput {
                    request_id: format!("s-{i}"),
                    project_id: p.id.clone(),
                    title: format!("Tarjeta {i}"),
                    column_id: None,
                    priority: None,
                },
            )
            .await
            .unwrap();
        }
        eprintln!("[medición] crear 300 tarjetas: {:?}", started.elapsed());

        let started = Instant::now();
        let stories = list_stories(&store, &p.id, false).await.unwrap();
        eprintln!(
            "[medición] listar 300 tarjetas (tablero): {:?}",
            started.elapsed()
        );
        assert_eq!(stories.len(), 300);

        // 50 movimientos atómicos de backlog a «listo», cada uno al principio.
        let ready = list_columns(&store, &p.id)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.category == ColumnCategory::Ready)
            .unwrap();
        let started = Instant::now();
        for s in stories.iter().take(50) {
            move_story(
                &store,
                &MoveStoryInput {
                    project_id: p.id.clone(),
                    id: s.id.clone(),
                    expected_revision: s.revision,
                    to_column_id: ready.id.clone(),
                    to_index: 0,
                },
            )
            .await
            .unwrap();
        }
        eprintln!("[medición] 50 movimientos: {:?}", started.elapsed());
        store.close().await;
        (p.id, ready.id)
    };

    let store = Store::open(data.path()).await.unwrap();
    let stories = list_stories(&store, &project_id, false).await.unwrap();
    let in_ready: Vec<_> = stories.iter().filter(|s| s.column_id == ready_id).collect();
    assert_eq!(in_ready.len(), 50);
    // El último movido quedó primero: el orden persiste tras reiniciar.
    assert_eq!(in_ready[0].title, "Tarjeta 49");
    let positions: Vec<_> = in_ready.iter().filter_map(|s| s.position).collect();
    assert_eq!(positions, (0..50).collect::<Vec<_>>());
    let backlog_positions: Vec<_> = stories
        .iter()
        .filter(|s| s.column_id != ready_id)
        .filter_map(|s| s.position)
        .collect();
    assert_eq!(backlog_positions, (0..250).collect::<Vec<_>>());
}
