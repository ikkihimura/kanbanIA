import { useEffect, useId, useRef, useState } from "react";
import type { RunWorktree } from "../../contracts/generated/RunWorktree";
import type { WorktreeChanges } from "../../contracts/generated/WorktreeChanges";
import {
  getWorktreeChanges,
  removeRunWorktree,
} from "../../shared/ipc/commands";
import { useMutation } from "../../shared/ipc/useMutation";
import { formatDateTime } from "../../shared/ui/format";
import { REMOVAL_LABEL } from "./runFormat";

const count = new Intl.NumberFormat("es");

function checkChanges(args: {
  projectId: string;
  runId: string;
}): Promise<WorktreeChanges> {
  return getWorktreeChanges(args.projectId, args.runId);
}

function discardLabel(changes: WorktreeChanges): string {
  const n = count.format(changes.changes);
  if (!changes.complete) return `Descartar al menos ${n} cambios y eliminar`;
  return changes.changes === 1
    ? "Descartar 1 cambio y eliminar"
    : `Descartar ${n} cambios y eliminar`;
}

function ignoredCount(changes: WorktreeChanges): string {
  const n = count.format(changes.ignored);
  const amount = changes.complete ? n : `al menos ${n}`;
  return changes.ignored === 1 && changes.complete
    ? "1 archivo ignorado por Git"
    : `${amount} archivos ignorados por Git`;
}

function ignoredText(changes: WorktreeChanges): string {
  return `El worktree tiene ${ignoredCount(changes)} (p. ej. .env o compilados): eliminarlo los borra para siempre.`;
}

/**
 * Eliminarlo sin forzar se rechazaría: hay cambios sin confirmar, archivos ignorados o commits
 * en riesgo (decisión del propietario: nada se borra sin «Descartar … y eliminar»).
 */
function needsForce(changes: WorktreeChanges): boolean {
  return (
    !changes.missing &&
    (changes.changes > 0 || changes.ignored > 0 || changes.commitsAtRisk)
  );
}

/** Frases de la confirmación, de la más grave a la menos. */
function confirmSentences(changes: WorktreeChanges, branch: string): string[] {
  if (changes.missing)
    return [
      `La carpeta del worktree ya no existe: solo se marcará como eliminado. La rama «${branch}» se conserva.`,
    ];
  const sentences: string[] = [];
  if (changes.commitsAtRisk)
    sentences.push(
      "Hay commits que no están en ninguna rama (o el worktree ya no está en la rama del intento): si lo eliminas se perderán.",
    );
  if (changes.changes > 0)
    sentences.push(
      `El worktree tiene ${changesText(changes)} (archivos modificados, borrados o nuevos): eliminarlo los descarta para siempre.`,
    );
  if (changes.ignored > 0) sentences.push(ignoredText(changes));
  if (sentences.length === 0)
    sentences.push(
      "¿Eliminar el worktree? No tiene cambios sin confirmar. Se borra su carpeta.",
    );
  sentences.push(
    changes.commitsAtRisk
      ? `La rama «${branch}» se conserva, pero solo con los commits que ya estén en ella.`
      : changes.changes > 0
        ? `La rama «${branch}» se conserva, pero solo con lo que ya se confirmó en ella.`
        : `La rama «${branch}» se conserva.`,
  );
  return sentences;
}

function removeLabel(changes: WorktreeChanges): string {
  if (!needsForce(changes)) return "Eliminar worktree";
  if (changes.commitsAtRisk || (changes.changes > 0 && changes.ignored > 0))
    return "Descartar y eliminar";
  if (changes.changes > 0) return discardLabel(changes);
  return `Descartar ${ignoredCount(changes)} y eliminar`;
}

interface Props {
  projectId: string;
  runId: string;
  worktree: RunWorktree | null;
  /** El intento terminó: solo entonces se puede eliminar su worktree. */
  ended: boolean;
  /** Tras eliminar (o si el núcleo dice que ya cambió): volver a leer el detalle. */
  onChanged: () => void;
}

function changesText(changes: WorktreeChanges): string {
  const n = count.format(changes.changes);
  const amount = changes.complete ? n : `al menos ${n}`;
  return changes.changes === 1 && changes.complete
    ? "1 cambio sin confirmar"
    : `${amount} cambios sin confirmar`;
}

/**
 * Worktree de un intento (HU-018): ruta, rama y commit base, y eliminarlo solo en intentos
 * terminados. Con cambios, archivos ignorados o commits en riesgo exige una acción explícita
 * distinta («Descartar … y eliminar», `force`).
 * La rama se conserva; los commits fuera de ella, no.
 */
export function WorktreeSection({
  projectId,
  runId,
  worktree,
  ended,
  onChanged,
}: Props) {
  const id = useId();
  const checker = useMutation(checkChanges);
  const remover = useMutation(removeRunWorktree);
  const [confirm, setConfirm] = useState<WorktreeChanges | null>(null);
  const [message, setMessage] = useState("");

  const titleRef = useRef<HTMLHeadingElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const keepRef = useRef<HTMLButtonElement>(null);
  // Al cerrar la confirmación, el disparador vuelve a montarse: el foco va a él tras renderizar.
  const restoreFocus = useRef(false);
  useEffect(() => {
    if (confirm) keepRef.current?.focus();
    else if (restoreFocus.current) {
      restoreFocus.current = false;
      triggerRef.current?.focus();
    }
  }, [confirm]);

  async function openConfirm() {
    setMessage("");
    remover.reset();
    const result = await checker.mutate({ projectId, runId });
    if (result.ok) setConfirm(result.value);
  }

  function closeConfirm() {
    restoreFocus.current = true;
    setConfirm(null);
    remover.reset();
  }

  async function handleRemove(force: boolean) {
    const result = await remover.mutate({ projectId, runId, force });
    if (result.ok) {
      setConfirm(null);
      const branch = result.value.branch;
      setMessage(
        result.value.removal === "missing"
          ? `La carpeta ya no existía: el worktree queda marcado como eliminado. La rama «${branch}» se conserva.`
          : `Worktree eliminado. La rama «${branch}» se conserva.`,
      );
      // El botón que abrió la confirmación desaparece: el foco va al título de la sección.
      titleRef.current?.focus();
      onChanged();
    } else if (result.error.field === "force") {
      // Aparecieron cambios, ignorados o commits en riesgo desde la comprobación: se vuelve a comprobar.
      const again = await checker.mutate({ projectId, runId });
      if (again.ok) setConfirm(again.value);
    } else if (result.error.field === "worktree") {
      onChanged();
    }
  }

  const removed = worktree?.removedAt != null;
  const force = confirm !== null && needsForce(confirm);
  const actionError = remover.error;

  return (
    <section className="run-worktree" aria-labelledby={`${id}-title`}>
      <h4
        id={`${id}-title`}
        className="relations__title"
        ref={titleRef}
        tabIndex={-1}
      >
        Worktree
      </h4>
      <p role="status" className="relations__saved">
        {message && `✓ ${message}`}
      </p>

      {worktree === null ? (
        <p className="panel__muted">
          {ended
            ? "Este intento no llegó a crear un worktree."
            : "Todavía no hay worktree: se crea al salir de la cola."}
        </p>
      ) : (
        <>
          <dl className="facts facts--compact">
            <dt>Ruta</dt>
            <dd className="repo__path">{worktree.path}</dd>
            <dt>Rama</dt>
            <dd>
              <span className="input--mono">{worktree.branch}</span>
              {worktree.branchDeletedAt !== null && (
                <>
                  {" "}
                  · ya no existe (anotado al limpiar el{" "}
                  <time dateTime={worktree.branchDeletedAt}>
                    {formatDateTime(worktree.branchDeletedAt)}
                  </time>
                  )
                </>
              )}
            </dd>
            <dt>Commit base</dt>
            <dd className="input--mono">{worktree.baseCommit}</dd>
            <dt>Creado</dt>
            <dd>
              <time dateTime={worktree.createdAt}>
                {formatDateTime(worktree.createdAt)}
              </time>
            </dd>
            {worktree.removedAt !== null && (
              <>
                <dt>Eliminado</dt>
                <dd>
                  <time dateTime={worktree.removedAt}>
                    {formatDateTime(worktree.removedAt)}
                  </time>
                  {worktree.removal && ` · ${REMOVAL_LABEL[worktree.removal]}`}
                </dd>
              </>
            )}
          </dl>
          {worktree.branchDeletedAt === null ? (
            <p className="field__hint">
              La rama se conserva siempre al eliminar el worktree: los commits
              que estén en ella no se pierden. Antes de eliminarlo se comprueba
              si hay cambios, archivos ignorados o commits fuera de la rama.
            </p>
          ) : (
            <p className="field__hint">
              La rama del intento ya no existe: se borró al limpiar, con
              confirmación explícita, o ya no estaba. Sus commits que no
              estuvieran en otra rama solo quedan en el reflog de Git hasta que
              este los recoja.
            </p>
          )}

          {!removed && !ended && (
            <p className="field__hint">
              Podrás eliminar el worktree cuando el intento termine.
            </p>
          )}
          {!removed && ended && confirm === null && (
            <div>
              <button
                type="button"
                className="button button--secondary"
                ref={triggerRef}
                aria-busy={checker.pending}
                disabled={checker.pending}
                aria-describedby={
                  checker.error ? `${id}-check-error` : undefined
                }
                onClick={() => {
                  void openConfirm();
                }}
              >
                Eliminar worktree…
              </button>
              {checker.pending && (
                <p className="panel__muted" role="status">
                  Comprobando cambios…
                </p>
              )}
              {checker.error && (
                <p
                  id={`${id}-check-error`}
                  className="field__error"
                  role="alert"
                >
                  No se pudieron comprobar los cambios: {checker.error.message}
                </p>
              )}
            </div>
          )}

          {!removed && confirm !== null && (
            <div
              className="repo__confirm"
              role="alertdialog"
              aria-labelledby={`${id}-confirm`}
              onKeyDown={(event) => {
                if (event.key === "Escape" && !remover.pending) {
                  event.stopPropagation();
                  closeConfirm();
                }
              }}
            >
              <div id={`${id}-confirm`}>
                {confirmSentences(confirm, worktree.branch).map((text) => (
                  <p key={text}>{text}</p>
                ))}
              </div>
              {actionError && (
                <p
                  id={`${id}-remove-error`}
                  className="field__error"
                  role="alert"
                >
                  {actionError.message}
                </p>
              )}
              <div className="repo__actions">
                <button
                  type="button"
                  className="button button--danger"
                  aria-busy={remover.pending}
                  disabled={remover.pending || checker.pending}
                  aria-describedby={
                    actionError ? `${id}-remove-error` : undefined
                  }
                  onClick={() => {
                    void handleRemove(force);
                  }}
                >
                  {removeLabel(confirm)}
                </button>
                <button
                  type="button"
                  className="button button--secondary"
                  ref={keepRef}
                  disabled={remover.pending}
                  onClick={closeConfirm}
                >
                  Mantener
                </button>
              </div>
            </div>
          )}
        </>
      )}
    </section>
  );
}
