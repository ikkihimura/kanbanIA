import { useEffect } from "react";
import { setUnsavedChanges } from "../ipc/commands";

/**
 * Informa al host de si hay cambios sin guardar, para que cerrar la ventana o salir (⌘Q)
 * pida confirmación. Al desmontar el componente, deja de bloquear.
 */
export function useUnsavedChangesGuard(dirty: boolean) {
  useEffect(() => {
    void setUnsavedChanges(dirty);
  }, [dirty]);

  useEffect(
    () => () => {
      void setUnsavedChanges(false);
    },
    [],
  );
}
