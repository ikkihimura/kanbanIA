//! Retención (HU-025) en el núcleo: recibos de idempotencia de 30 días (con reloj controlable)
//! y días de retención de la configuración. La limpieza de intentos con worktrees reales se
//! prueba en `crates/runner/tests/cleanup.rs`.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use kanbania_core::error::CoreError;
use kanbania_core::features::projects::{CreateProjectInput, create_project, list_projects};
use kanbania_core::features::runs::{
    UpdateRunSettingsInput, get_run_settings, retention_candidates, update_run_settings,
};
use kanbania_core::platform::actor::Actor;
use kanbania_core::platform::clock::ManualClock;
use kanbania_core::platform::idempotency::RECEIPT_RETENTION_DAYS;
use kanbania_core::platform::storage::Store;

const DAY: Duration = Duration::from_secs(86_400);
/// 2026-10-31T00:00:00Z.
const START_MS: i64 = 1_793_404_800_000;

async fn store() -> (tempfile::TempDir, Store, Arc<ManualClock>) {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(ManualClock::new(START_MS));
    let store = Store::open(dir.path())
        .await
        .unwrap()
        .with_clock(clock.clone());
    (dir, store, clock)
}

async fn create(store: &Store, request_id: &str) -> String {
    create_project(
        store,
        "ui",
        &CreateProjectInput {
            request_id: request_id.into(),
            name: "Proyecto".into(),
        },
    )
    .await
    .unwrap()
    .id
}

/// CA-3: dentro de la ventana, repetir la petición devuelve el mismo resultado sin aplicarla
/// otra vez; al abrir el almacén (aquí, al llamar a la retención) se borran solo los recibos
/// de más de 30 días, con la hora del reloj del núcleo.
#[tokio::test]
async fn receipts_expire_after_their_window_and_not_before() {
    assert_eq!(RECEIPT_RETENTION_DAYS, 30);
    let (_dir, store, clock) = store().await;
    let old = create(&store, "antigua").await;
    clock.advance(29 * DAY);
    let recent = create(&store, "reciente").await;
    // Día 29: nada vence y el reintento de la primera sigue siendo idempotente.
    assert_eq!(store.prune_mutation_receipts().await.unwrap(), 0);
    assert_eq!(create(&store, "antigua").await, old);

    clock.advance(2 * DAY);
    // Día 31: vence la primera (día 0); la del día 29 sigue.
    assert_eq!(store.prune_mutation_receipts().await.unwrap(), 1);
    assert_eq!(store.prune_mutation_receipts().await.unwrap(), 0);
    assert_eq!(create(&store, "reciente").await, recent);
    assert_eq!(list_projects(&store).await.unwrap().len(), 2);
    // Fuera de la ventana, el mismo `request_id` es una petición nueva.
    assert_ne!(create(&store, "antigua").await, old);
    assert_eq!(list_projects(&store).await.unwrap().len(), 3);
}

/// CA-1: la antigüedad para proponer limpieza se configura (1..3650, por defecto 30) y un
/// cliente que no la envía no la cambia. La fecha de corte sale del reloj del núcleo.
#[tokio::test]
async fn retention_days_are_configurable() {
    let (_dir, store, _clock) = store().await;
    let current = get_run_settings(&store).await.unwrap();
    assert_eq!(current.retention_days, 30);
    let input = |days: Option<i64>, revision: i64| UpdateRunSettingsInput {
        expected_revision: revision,
        max_concurrency: current.max_concurrency,
        timeout_minutes: current.timeout_minutes,
        claude_acp_path: None,
        codex_acp_path: None,
        node_path: None,
        approval_notifications: false,
        retention_days: days,
    };
    for bad in [0, 3651, -1] {
        let error = update_run_settings(&store, &Actor::Human, &input(Some(bad), current.revision))
            .await
            .unwrap_err();
        assert!(
            matches!(
                error,
                CoreError::Validation {
                    field: "retentionDays",
                    ..
                }
            ),
            "{bad}: {error:?}"
        );
    }
    let saved = update_run_settings(&store, &Actor::Human, &input(Some(7), current.revision))
        .await
        .unwrap();
    assert_eq!(saved.retention_days, 7);
    let kept = update_run_settings(&store, &Actor::Human, &input(None, saved.revision))
        .await
        .unwrap();
    assert_eq!(kept.retention_days, 7);

    let plan = retention_candidates(&store).await.unwrap();
    assert_eq!(plan.retention_days, 7);
    assert_eq!(plan.cutoff_ms, START_MS - 7 * 86_400_000);
    assert_eq!(plan.cutoff, "2026-10-24T00:00:00.000Z");
    assert!(plan.items.is_empty() && !plan.truncated);
}
