import { useEffect, useId, useRef, useState } from "react";
import type { BackupInfo } from "../../contracts/generated/BackupInfo";
import type { BackupKind } from "../../contracts/generated/BackupKind";
import type { RestoreSource } from "../../contracts/generated/RestoreSource";
import {
  createBackup,
  listBackups,
  restoreBackup,
  saveBackupToFolder,
} from "../../shared/ipc/commands";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { useMutation } from "../../shared/ipc/useMutation";
import { chooseFolder } from "../../shared/ui/chooseFolder";
import { formatDateTime } from "../../shared/ui/format";

const KIND_LABEL: Record<BackupKind, string> = {
  manual: "Manual",
  pre_restore: "Automática: antes de restaurar",
  pre_migration: "Automática: antes de actualizar la base",
};

/** Lo que una copia no incluye (CA-3): resumen de los `excludes` del manifiesto del núcleo. */
const NOT_INCLUDED = [
  "El contenido de los repositorios vinculados (solo sus rutas).",
  "La salida de los procesos y los worktrees de los intentos (carpetas del directorio de datos). Los intentos sí se incluyen, con su estado.",
  "La exportación Markdown no es una copia de seguridad: solo lleva la planificación.",
];

function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

interface Props {
  /** `false` si el almacenamiento no está disponible: no se pueden crear copias. */
  canCreate: boolean;
  /** Empieza o termina una restauración: la app bloquea la interacción mientras tanto. */
  onRestoring: (busy: boolean) => void;
  /** Restauración terminada (con éxito: aviso para mostrar; con error: `null`). */
  onFinished: (notice: string | null) => void;
}

/** Copias de seguridad locales y restauración (HU-011). */
export function BackupPanel({ canCreate, onRestoring, onFinished }: Props) {
  const id = useId();
  const { state, run } = useIpcResource(listBackups);
  const creator = useMutation(createBackup);
  const saver = useMutation(saveBackupToFolder);
  const restorer = useMutation(restoreBackup);
  const [confirm, setConfirm] = useState<{
    source: RestoreSource;
    label: string;
  } | null>(null);
  const [message, setMessage] = useState("");
  const [savingId, setSavingId] = useState<string | null>(null);
  // Confirmación accesible: el foco entra en ella al abrirse y vuelve al botón que la abrió.
  const trigger = useRef<HTMLElement | null>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (confirm) cancelRef.current?.focus();
  }, [confirm]);

  function openConfirm(next: { source: RestoreSource; label: string }) {
    trigger.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    restorer.reset();
    setMessage("");
    setConfirm(next);
  }

  function closeConfirm() {
    setConfirm(null);
    restorer.reset();
    trigger.current?.focus();
  }

  function reload() {
    run(listBackups, { keepData: true });
  }

  async function handleCreate() {
    setMessage("");
    const result = await creator.mutate(undefined);
    if (result.ok) {
      setMessage(`Copia creada (${formatDateTime(result.value.createdAt)}).`);
      reload();
    }
  }

  async function handleSave(backup: BackupInfo) {
    setMessage("");
    const destination = await chooseFolder("Elige dónde guardar la copia");
    if (destination === null) return;
    setSavingId(backup.id);
    const result = await saver.mutate({ id: backup.id, destination });
    setSavingId(null);
    if (result.ok)
      setMessage(
        `Copia guardada en «${result.value}» dentro de ${destination}.`,
      );
  }

  async function askFolder() {
    setMessage("");
    const path = await chooseFolder("Elige la carpeta de la copia a restaurar");
    if (path === null) return;
    openConfirm({
      source: { kind: "folder", path },
      label: `la carpeta ${path}`,
    });
  }

  async function handleRestore() {
    if (!confirm) return;
    onRestoring(true);
    const result = await restorer.mutate(confirm.source);
    reload();
    if (result.ok) {
      setConfirm(null);
      const safety = result.value.safetyBackup;
      onFinished(
        safety
          ? `Datos restaurados. Los anteriores quedaron en la copia del ${formatDateTime(safety.createdAt)}: restáurala si quieres deshacer.`
          : "Datos restaurados.",
      );
    } else {
      // También tras un error: el almacén pudo reabrirse o volver al estado anterior.
      onFinished(null);
    }
  }

  const error = creator.error ?? saver.error;

  return (
    <section className="panel" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className="panel__title">
        Copias de seguridad
      </h2>
      <p className="field__hint">
        Una copia guarda toda la base: proyectos, historias, épicas, equipo,
        revisiones e historial. Se conservan las 10 más recientes en el
        directorio de datos; guárdalas en otra carpeta (p. ej. un disco externo)
        para protegerte si se pierde este disco.
      </p>
      <details>
        <summary>Qué no incluye</summary>
        <ul>
          {NOT_INCLUDED.map((text) => (
            <li key={text}>{text}</li>
          ))}
        </ul>
      </details>

      <div className="repo__actions">
        <button
          type="button"
          className="button"
          aria-busy={creator.pending}
          aria-describedby={canCreate ? undefined : `${id}-no-create`}
          disabled={!canCreate || creator.pending || restorer.pending}
          onClick={() => {
            void handleCreate();
          }}
        >
          Crear copia ahora
        </button>
        <button
          type="button"
          className="button button--secondary"
          disabled={restorer.pending || creator.pending}
          onClick={() => {
            void askFolder();
          }}
        >
          Restaurar desde una carpeta…
        </button>
      </div>
      {!canCreate && (
        <p id={`${id}-no-create`} className="field__hint">
          Para crear copias, el almacenamiento debe estar disponible. Puedes
          restaurar una copia para recuperarlo.
        </p>
      )}

      <p role="status" className="relations__saved">
        {message && `✓ ${message}`}
      </p>
      {error && (
        <p className="field__error" role="alert">
          {error.message}
        </p>
      )}

      {confirm && (
        <div
          className="repo__confirm"
          role="alertdialog"
          aria-labelledby={`${id}-confirm`}
          onKeyDown={(event) => {
            if (event.key === "Escape" && !restorer.pending) closeConfirm();
          }}
        >
          <p id={`${id}-confirm`}>
            ¿Restaurar {confirm.label}? Se reemplazarán <strong>todos</strong>{" "}
            los datos actuales. Antes se guarda automáticamente una copia del
            estado actual para poder deshacerlo. Los cambios sin guardar de
            cualquier editor se perderán. Los procesos de los intentos en marcha
            se detienen, y en los datos restaurados todo intento activo (también
            los que estaban en cola) queda como «Interrumpido».
          </p>
          {restorer.error && (
            <p className="field__error" role="alert">
              No se restauró: {restorer.error.message}
            </p>
          )}
          <div className="repo__actions">
            <button
              type="button"
              className="button button--danger"
              aria-busy={restorer.pending}
              disabled={restorer.pending}
              onClick={() => {
                void handleRestore();
              }}
            >
              Restaurar
            </button>
            <button
              type="button"
              className="button button--secondary"
              ref={cancelRef}
              disabled={restorer.pending}
              onClick={closeConfirm}
            >
              Cancelar
            </button>
          </div>
        </div>
      )}

      {state.status === "loading" && (
        <p className="panel__muted">Cargando copias…</p>
      )}
      {state.status === "error" && (
        <p className="panel__error">{state.message}</p>
      )}
      {state.status === "ready" && state.data.length === 0 && (
        <p className="panel__muted">Aún no hay copias.</p>
      )}
      {state.status === "ready" && state.data.length > 0 && (
        <ul className="item-list">
          {state.data.map((backup) => (
            <li key={backup.id} className="backup-item">
              <span className="item-list__name">
                {formatDateTime(backup.createdAt)}
              </span>
              <span className="item-list__meta">
                {KIND_LABEL[backup.kind]} ·{" "}
                {backup.schemaVersion === null
                  ? "esquema no disponible"
                  : `esquema v${String(backup.schemaVersion)}`}{" "}
                · {formatBytes(backup.bytes)}
              </span>
              <span className="repo__actions">
                <button
                  type="button"
                  className="button button--secondary"
                  disabled={restorer.pending || creator.pending}
                  onClick={() => {
                    openConfirm({
                      source: { kind: "backup", id: backup.id },
                      label: `la copia del ${formatDateTime(backup.createdAt)}`,
                    });
                  }}
                >
                  Restaurar…
                </button>
                <button
                  type="button"
                  className="button button--secondary"
                  aria-busy={savingId === backup.id}
                  disabled={saver.pending}
                  onClick={() => {
                    void handleSave(backup);
                  }}
                >
                  Guardar en carpeta…
                </button>
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
