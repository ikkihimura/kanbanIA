import { useEffect } from "react";

function isEditable(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    (target instanceof HTMLElement && target.isContentEditable)
  );
}

/**
 * Con `dragDropEnabled: false` (necesario para el arrastre HTML5 en Windows), el WebView
 * gestiona lo que se suelta desde fuera: un archivo o enlace soltado en la ventana la haría
 * navegar y se perderían los borradores sin pasar por la confirmación. Aquí se anula esa
 * acción por defecto salvo en campos de texto; los arrastres del tablero ya la anulan ellos.
 */
export function useBlockExternalDrops() {
  useEffect(() => {
    function onDragOver(event: DragEvent) {
      if (event.defaultPrevented || isEditable(event.target)) return;
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "none";
    }
    function onDrop(event: DragEvent) {
      if (isEditable(event.target)) return;
      event.preventDefault();
    }
    document.addEventListener("dragover", onDragOver);
    document.addEventListener("drop", onDrop);
    return () => {
      document.removeEventListener("dragover", onDragOver);
      document.removeEventListener("drop", onDrop);
    };
  }, []);
}
