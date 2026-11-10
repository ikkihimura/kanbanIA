import { open } from "@tauri-apps/plugin-dialog";
import { useId } from "react";
import type { UiError } from "../../shared/ipc/commands";
import { FieldError } from "../../shared/ui/FieldError";

interface Props {
  label: string;
  value: string;
  error: UiError | null;
  onChange: (value: string) => void;
  autoFocus?: boolean;
}

/** Ruta de carpeta: se puede escribir o elegir con el selector nativo. El núcleo la valida. */
export function FolderField({
  label,
  value,
  error,
  onChange,
  autoFocus = false,
}: Props) {
  const inputId = useId();
  const errorId = `${inputId}-error`;

  async function pickFolder() {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "Elige la carpeta raíz del repositorio Git",
    });
    if (typeof selected === "string") onChange(selected);
  }

  return (
    <div className="field field--grow">
      <label htmlFor={inputId}>{label}</label>
      <div className="field__inline">
        <input
          id={inputId}
          className="input--mono"
          value={value}
          spellCheck={false}
          autoComplete="off"
          autoFocus={autoFocus}
          placeholder="/ruta/al/repositorio"
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? errorId : undefined}
          onChange={(event) => {
            onChange(event.target.value);
          }}
        />
        <button
          type="button"
          className="button button--secondary"
          onClick={() => {
            void pickFolder();
          }}
        >
          Elegir carpeta…
        </button>
      </div>
      <FieldError id={errorId} error={error} />
    </div>
  );
}
