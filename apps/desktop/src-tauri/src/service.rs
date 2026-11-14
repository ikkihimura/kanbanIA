//! Servicio local (HU-012) mantenido por el host. Solo arranca si esta instancia posee el
//! almacén (el bloqueo del directorio de datos): una segunda instancia nunca publica otro
//! servicio ni pisa el descriptor. Cada petición pide el almacén al `StorageSlot`, así que
//! respeta restauraciones y cierres igual que la UI.

use std::sync::Arc;

use kanbania_core::error::{CoreError, ErrorCode, ErrorPayload};
use kanbania_core::features::access::LocalServiceStatus;
use kanbania_core::features::agent_activity::AgentChange;
use kanbania_core::platform::storage::Store;
use kanbania_local_service::{LocalService, StoreSource};
use tauri::async_runtime::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::storage::StorageSlot;

/// Evento que el host emite a la ventana principal cuando un agente cambia el tablero (carga:
/// `AgentChange`). La UI recarga; el núcleo sigue siendo la autoridad.
pub const AGENT_CHANGE_EVENT: &str = "agent-change";

#[derive(Default)]
pub struct ServiceSlot {
    running: Mutex<Option<LocalService>>,
    /// Tras salir no se vuelve a arrancar (un reintento o una restauración tardíos dejarían
    /// un descriptor huérfano).
    stopped: Mutex<bool>,
}

impl ServiceSlot {
    /// Arranca el servicio si aún no lo está y el almacén está listo. Idempotente.
    pub async fn ensure_started<R: Runtime>(&self, handle: &AppHandle<R>) {
        let mut running = self.running.lock().await;
        if running.is_some() || *self.stopped.lock().await {
            return;
        }
        let Some(slot) = handle.try_state::<StorageSlot>() else {
            return;
        };
        if !slot.is_ready().await {
            return;
        }
        let source: Arc<dyn StoreSource> = Arc::new(HostStore(handle.clone()));
        match LocalService::start(slot.data_dir(), source).await {
            Ok(service) => {
                // El puerto no es secreto; el token de sesión nunca se registra.
                eprintln!("[local-service] escuchando en 127.0.0.1:{}", service.port());
                *running = Some(service);
            }
            Err(error) => eprintln!("[local-service] no se pudo arrancar: {error}"),
        }
    }

    /// Activo solo si escucha y además el almacén está disponible (si no, toda petición
    /// recibiría «no disponible»).
    pub async fn status(&self, storage: &StorageSlot) -> LocalServiceStatus {
        let port = self.running.lock().await.as_ref().map(LocalService::port);
        let port = if storage.is_ready().await { port } else { None };
        LocalServiceStatus {
            running: port.is_some(),
            port,
        }
    }

    /// Para el servicio y borra su descriptor (al salir).
    pub async fn stop(&self) {
        let mut running = self.running.lock().await;
        *self.stopped.lock().await = true;
        if let Some(service) = running.take() {
            service.stop().await;
        }
    }
}

struct HostStore<R: Runtime>(AppHandle<R>);

impl<R: Runtime> StoreSource for HostStore<R> {
    fn store(
        &self,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Store, ErrorPayload>> + Send + '_>> {
        Box::pin(async move {
            match self.0.try_state::<StorageSlot>() {
                Some(slot) => slot.store().await.map_err(for_agents),
                None => Err(for_agents(CoreError::StorageLocked.to_payload(None))),
            }
        })
    }

    fn agent_changed(&self, change: AgentChange) {
        // Solo un aviso para recargar: si la ventana no está, no pasa nada.
        if let Err(error) = self.0.emit_to("main", AGENT_CHANGE_EVENT, change) {
            eprintln!("[local-service] no se pudo avisar a la ventana: {error}");
        }
    }
}

/// Dentro del host, «bloqueado» significa que el almacén se está restaurando o cerrando, no
/// que haya otra instancia: el mensaje de la UI («ciérrala») confundiría al agente.
fn for_agents(payload: ErrorPayload) -> ErrorPayload {
    if payload.code != ErrorCode::StorageLocked {
        return payload;
    }
    ErrorPayload {
        code: ErrorCode::StorageUnavailable,
        message: "El almacenamiento de Kanban IA no está disponible ahora mismo (restauración o \
                  cierre). Vuelve a intentarlo en unos segundos."
            .into(),
        recoverable: true,
        ..payload
    }
}

/// Arranca el servicio local si procede (tras abrir, reintentar o restaurar el almacén).
pub async fn start_local_service<R: Runtime>(handle: &AppHandle<R>) {
    if let Some(service) = handle.try_state::<ServiceSlot>() {
        service.ensure_started(handle).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_busy_store_is_reported_to_agents_as_temporarily_unavailable() {
        let busy = for_agents(CoreError::StorageLocked.to_payload(None));
        assert_eq!(busy.code, ErrorCode::StorageUnavailable);
        assert!(busy.recoverable);
        assert!(!busy.message.contains("Ciérralo"));
        let newer = CoreError::NewerSchema(99).to_payload(None);
        assert_eq!(for_agents(newer.clone()), newer);
    }
}
