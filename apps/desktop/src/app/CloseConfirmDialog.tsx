import { useEffect, useRef, useState } from "react";
import { discardAndClose, onConfirmClose } from "../shared/ipc/commands";

/**
 * Diálogo que aparece cuando se intenta cerrar la ventana o salir (⌘Q) con cambios sin
 * guardar. El host ya canceló el cierre; aquí el usuario decide.
 */
export function CloseConfirmDialog() {
  const [open, setOpen] = useState(false);
  const [closing, setClosing] = useState(false);
  const keepEditingRef = useRef<HTMLButtonElement>(null);
  const previousFocus = useRef<Element | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;
    void onConfirmClose(() => {
      previousFocus.current = document.activeElement;
      setOpen(true);
    }).then((stop) => {
      if (active) unlisten = stop;
      else stop();
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (open) keepEditingRef.current?.focus();
  }, [open]);

  if (!open) return null;

  function keepEditing() {
    setOpen(false);
    // Devolver el foco a donde estaba el usuario (normalmente, el campo que editaba).
    if (previousFocus.current instanceof HTMLElement)
      previousFocus.current.focus();
  }

  return (
    <div className="modal-backdrop">
      <div
        className="modal panel"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="close-confirm-title"
        aria-describedby="close-confirm-text"
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            keepEditing();
            return;
          }
          // Mantener el foco dentro del diálogo mientras está abierto.
          if (event.key === "Tab") {
            const buttons = Array.from(
              event.currentTarget.querySelectorAll<HTMLButtonElement>(
                "button:not(:disabled)",
              ),
            );
            const first = buttons[0];
            const last = buttons[buttons.length - 1];
            if (event.shiftKey && document.activeElement === first) {
              event.preventDefault();
              last?.focus();
            } else if (!event.shiftKey && document.activeElement === last) {
              event.preventDefault();
              first?.focus();
            }
          }
        }}
      >
        <h2 id="close-confirm-title" className="panel__title">
          Cambios sin guardar
        </h2>
        <p id="close-confirm-text">
          Si sales ahora se perderán los cambios que no has guardado.
        </p>
        <div className="repo__actions">
          <button
            type="button"
            className="button"
            ref={keepEditingRef}
            onClick={keepEditing}
          >
            Seguir editando
          </button>
          <button
            type="button"
            className="button button--danger"
            aria-busy={closing}
            disabled={closing}
            onClick={() => {
              setClosing(true);
              void discardAndClose();
            }}
          >
            Salir sin guardar
          </button>
        </div>
      </div>
    </div>
  );
}
