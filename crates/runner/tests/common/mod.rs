//! Ayudas compartidas por las pruebas de procesos.

#![allow(dead_code, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

fn beats(dir: &Path) -> BTreeMap<String, String> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| {
            let content = std::fs::read_to_string(e.path()).unwrap_or_default();
            (e.file_name().to_string_lossy().into_owned(), content)
        })
        .collect()
}

/// Espera a que `slow --heartbeat <dir>` tenga los tres latidos (padre e hijos) y comprueba que
/// cambian: los tres procesos corren.
pub async fn wait_heartbeats_alive(dir: &Path) {
    let started = Instant::now();
    loop {
        let before = beats(dir);
        if before.len() == 3 {
            tokio::time::sleep(Duration::from_millis(700)).await;
            let after = beats(dir);
            if before
                .iter()
                .all(|(name, beat)| after.get(name) != Some(beat))
            {
                return;
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "los latidos no avanzan: {before:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Comprobación portable (también en Windows) de que ningún proceso del árbol sigue vivo: los
/// tres latidos dejan de cambiar.
pub async fn assert_heartbeats_stopped(dir: &Path) {
    // Margen para que un último `write` en curso termine.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = beats(dir);
    assert_eq!(before.len(), 3, "{before:?}");
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        beats(dir),
        before,
        "algún proceso sigue escribiendo su latido"
    );
}
