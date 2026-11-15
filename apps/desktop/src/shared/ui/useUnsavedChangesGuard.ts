import { useEffect, useState } from "react";
import { setUnsavedChanges } from "../ipc/commands";

// El host guarda un solo indicador. Para que varios editores convivan (p. ej. el editor de
// historias y la configuración de Sistema), la UI lleva el conjunto de los que tienen
// borrador y envía al host la unión: hay cambios sin guardar si alguno los tiene.
const dirtyOwners = new Set<symbol>();

function report(): void {
  void setUnsavedChanges(dirtyOwners.size > 0);
}

/**
 * Informa al host de si hay cambios sin guardar, para que cerrar la ventana o salir (⌘Q)
 * pida confirmación. Admite varios editores a la vez: limpiar uno no borra el borrador de
 * otro. Al desmontar el componente, deja de contar el suyo.
 */
export function useUnsavedChangesGuard(dirty: boolean) {
  const [owner] = useState(() => Symbol("unsaved-changes"));

  useEffect(() => {
    if (dirty) dirtyOwners.add(owner);
    else dirtyOwners.delete(owner);
    report();
  }, [dirty, owner]);

  useEffect(
    () => () => {
      dirtyOwners.delete(owner);
      report();
    },
    [owner],
  );
}
