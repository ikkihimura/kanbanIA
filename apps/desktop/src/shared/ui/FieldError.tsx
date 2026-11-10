import type { UiError } from "../ipc/commands";

interface Props {
  id: string;
  error: UiError | null;
}

/** Mensaje de error de un campo; enlazar con `aria-describedby={id}` en el control. */
export function FieldError({ id, error }: Props) {
  if (!error) return null;
  return (
    <p id={id} className="field__error" role="alert">
      {error.message}
    </p>
  );
}
