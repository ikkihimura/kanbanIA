//! Reloj del núcleo. Los reclamos (HU-014) caducan: sus reglas toman la hora de aquí y no de
//! `strftime('now')` de SQLite, para que las pruebas controlen el tiempo con [`ManualClock`].

use std::fmt;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Milisegundos desde la época Unix (UTC).
pub trait Clock: Send + Sync + fmt::Debug {
    fn now_ms(&self) -> i64;
}

/// Reloj del sistema (el de la app).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }
}

/// Reloj que solo avanza cuando se le pide (pruebas).
#[derive(Debug)]
pub struct ManualClock {
    now_ms: AtomicI64,
}

impl ManualClock {
    pub fn new(now_ms: i64) -> Self {
        Self {
            now_ms: AtomicI64::new(now_ms),
        }
    }

    /// Empieza en la hora actual del sistema.
    pub fn starting_now() -> Self {
        Self::new(SystemClock.now_ms())
    }

    pub fn advance(&self, by: Duration) {
        let ms = i64::try_from(by.as_millis()).unwrap_or(i64::MAX);
        self.now_ms.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

/// Expresión SQL que formatea una columna de milisegundos Unix como el resto de fechas del
/// esquema (`2026-11-12T10:30:00.000Z`). Úsala dentro de `concat!`.
macro_rules! iso_from_ms {
    ($column:literal) => {
        concat!(
            "strftime('%Y-%m-%dT%H:%M:%fZ', ",
            $column,
            " / 1000.0, 'unixepoch')"
        )
    };
}
pub(crate) use iso_from_ms;
