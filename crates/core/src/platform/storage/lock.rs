use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

use crate::error::CoreError;

const LOCK_FILE: &str = "kanbania.lock";

/// Bloqueo exclusivo del directorio de datos durante la vida del proceso propietario.
/// El sistema operativo lo libera al cerrar el archivo o si el proceso termina abruptamente.
#[derive(Debug)]
pub(crate) struct DataDirLock {
    _file: File,
}

impl DataDirLock {
    pub(crate) fn acquire(data_dir: &Path) -> Result<Self, CoreError> {
        std::fs::create_dir_all(data_dir).map_err(CoreError::StorageIo)?;

        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(data_dir.join(LOCK_FILE))
            .map_err(CoreError::StorageIo)?;

        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(CoreError::StorageLocked),
            Err(TryLockError::Error(error)) => Err(CoreError::StorageIo(error)),
        }
    }
}
