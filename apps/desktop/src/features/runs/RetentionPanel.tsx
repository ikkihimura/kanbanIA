import { useEffect, useId, useRef, useState } from "react";
import type { CleanRunResult } from "../../contracts/generated/CleanRunResult";
import type { DiskUsage } from "../../contracts/generated/DiskUsage";
import type { OrphanWorktree } from "../../contracts/generated/OrphanWorktree";
import type { RetentionBranch } from "../../contracts/generated/RetentionBranch";
import type { RetentionItem } from "../../contracts/generated/RetentionItem";
import type { RetentionPlan } from "../../contracts/generated/RetentionPlan";
import type { WorktreeCheck } from "../../contracts/generated/WorktreeCheck";
import type { WorktreeRemoval } from "../../contracts/generated/WorktreeRemoval";
import {
  cleanRun,
  getRetentionPlan,
  removeOrphanWorktree,
  toUiError,
} from "../../shared/ipc/commands";
import { useMutation } from "../../shared/ipc/useMutation";
import { formatDateTime } from "../../shared/ui/format";
import { RUN_STATE_LABEL } from "./runFormat";

const UNITS = ["KB", "MB", "GB", "TB"] as const;
const decimal = new Intl.NumberFormat("es", {
  minimumFractionDigits: 1,
  maximumFractionDigits: 1,
});
const integer = new Intl.NumberFormat("es");

/** Tamaño legible en base 1024: «512 B», «1,5 KB», «3,0 GB». */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 1024)
    return `${integer.format(Math.max(0, Math.round(bytes || 0)))} B`;
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${decimal.format(value)} ${UNITS[unit] ?? "TB"}`;
}

/** Tamaño de una medición: «al menos …» si se cortó por límite de archivos o de tiempo. */
export function formatUsage(usage: DiskUsage): string {
  const size = formatBytes(usage.bytes);
  return usage.complete ? size : `al menos ${size}`;
}

function filesText(usage: DiskUsage): string {
  const n = integer.format(usage.files);
  const amount = usage.complete ? n : `al menos ${n}`;
  return usage.files === 1 && usage.complete
    ? "1 archivo"
    : `${amount} archivos`;
}

/** El cálculo con la forma que espera `useMutation` (sin argumentos). */
function loadPlan(): Promise<RetentionPlan> {
  return getRetentionPlan();
}

/** Lo marcado para un intento. */
interface ItemSelection {
  output: boolean;
  worktree: boolean;
  discard: boolean;
  branch: boolean;
}

/** Lo marcado para un worktree sin intento. */
interface OrphanSelection {
  remove: boolean;
  discard: boolean;
}

/** Por defecto: salida y worktree que se puede eliminar sin perder nada; nunca la rama. */
function defaultItem(item: RetentionItem): ItemSelection {
  const kind = item.worktree?.check.kind;
  return {
    output: item.output !== null,
    worktree: kind === "clean" || kind === "missing",
    discard: false,
    branch: false,
  };
}

function defaults(plan: RetentionPlan) {
  const items: Record<string, ItemSelection> = {};
  for (const item of plan.items) items[item.runId] = defaultItem(item);
  const orphans: Record<string, OrphanSelection> = {};
  for (const orphan of plan.orphans)
    orphans[orphan.path] = { remove: false, discard: false };
  return { items, orphans };
}

/** Eliminar este worktree funcionaría: está marcado y, si pierde algo, también «Descartar». */
function worktreeEffective(
  check: WorktreeCheck | undefined,
  remove: boolean,
  discard: boolean,
): boolean {
  if (!remove || check === undefined || check.kind === "unverifiable")
    return false;
  return check.kind !== "needsForce" || discard;
}

/** Lo que se enviará de verdad para un intento (lo marcado que puede funcionar). */
function effectiveItem(item: RetentionItem, sel: ItemSelection): ItemSelection {
  const worktree = worktreeEffective(
    item.worktree?.check,
    sel.worktree,
    sel.discard,
  );
  const branch =
    item.branch !== null &&
    sel.branch &&
    (!item.branch.worktreeLive || worktree);
  return {
    output: item.output !== null && sel.output,
    worktree,
    discard: worktree && item.worktree?.check.kind === "needsForce",
    branch,
  };
}

function effectiveOrphan(orphan: OrphanWorktree, sel: OrphanSelection) {
  const remove = worktreeEffective(orphan.check, sel.remove, sel.discard);
  return { remove, discard: remove && orphan.check.kind === "needsForce" };
}

function plural(n: number, one: string, many: string): string {
  return n === 1 ? `1 ${one}` : `${integer.format(n)} ${many}`;
}

/** Aviso de la rama: trabajo sin integrar (o que no se pudo saber). */
function branchWarning(branch: RetentionBranch): string {
  const n = branch.unintegratedCommits;
  if (n === null)
    return "No se pudo saber si tiene trabajo sin integrar: borrarla podría perder commits.";
  if (n === 0) return "Todos sus commits están también en otra rama.";
  return n === 1
    ? "Trabajo sin integrar: 1 commit que no está en otra rama."
    : `Trabajo sin integrar: ${integer.format(n)} commits que no están en otra rama.`;
}

function itemTitle(item: RetentionItem): string {
  return `#${String(item.storyNumber)} ${item.storyTitle} · intento #${String(item.runNumber)}`;
}

const REMOVAL_RESULT: Record<WorktreeRemoval, string> = {
  clean: "worktree eliminado",
  discarded: "worktree descartado y eliminado",
  missing: "el worktree ya no existía (queda marcado como eliminado)",
};

function describeClean(result: CleanRunResult): string {
  const parts: string[] = [];
  if (result.outputRemoved) parts.push("salida eliminada");
  if (result.worktreeRemoval !== null)
    parts.push(REMOVAL_RESULT[result.worktreeRemoval]);
  if (result.branchDeleted) parts.push("rama borrada");
  return parts.length === 0 ? "no había nada que borrar" : parts.join(", ");
}

interface CleanOutcome {
  key: string;
  label: string;
  ok: boolean;
  text: string;
}

/** Resumen de lo que se va a limpiar (para la confirmación). */
interface Summary {
  outputs: number;
  worktrees: number;
  worktreesDiscarding: number;
  branches: number;
  branchesUnintegrated: number;
  branchesUnknown: number;
  orphans: number;
  orphansDiscarding: number;
  bytes: number;
  /** Alguna medición se cortó o no estaba disponible: `bytes` es un mínimo. */
  bytesPartial: boolean;
}

function summarize(
  plan: RetentionPlan,
  items: Record<string, ItemSelection>,
  orphans: Record<string, OrphanSelection>,
): Summary {
  const s: Summary = {
    outputs: 0,
    worktrees: 0,
    worktreesDiscarding: 0,
    branches: 0,
    branchesUnintegrated: 0,
    branchesUnknown: 0,
    orphans: 0,
    orphansDiscarding: 0,
    bytes: 0,
    bytesPartial: false,
  };
  const add = (usage: DiskUsage | null | undefined) => {
    if (usage == null) {
      s.bytesPartial = true;
      return;
    }
    s.bytes += usage.bytes;
    if (!usage.complete) s.bytesPartial = true;
  };
  for (const item of plan.items) {
    const sel = items[item.runId];
    if (!sel) continue;
    const eff = effectiveItem(item, sel);
    if (eff.output) {
      s.outputs += 1;
      add(item.output);
    }
    if (eff.worktree) {
      s.worktrees += 1;
      if (eff.discard) s.worktreesDiscarding += 1;
      // Una carpeta que ya no existe no ocupa nada.
      if (item.worktree?.check.kind !== "missing") add(item.worktree?.usage);
    }
    if (eff.branch && item.branch) {
      s.branches += 1;
      const n = item.branch.unintegratedCommits;
      if (n === null) s.branchesUnknown += 1;
      else if (n > 0) s.branchesUnintegrated += 1;
    }
  }
  for (const orphan of plan.orphans) {
    const sel = orphans[orphan.path];
    if (!sel) continue;
    const eff = effectiveOrphan(orphan, sel);
    if (eff.remove) {
      s.orphans += 1;
      if (eff.discard) s.orphansDiscarding += 1;
      if (orphan.check.kind !== "missing") add(orphan.usage);
    }
  }
  return s;
}

function summaryLines(s: Summary): string[] {
  const lines: string[] = [];
  if (s.outputs > 0)
    lines.push(
      `${plural(s.outputs, "salida de intento", "salidas de intentos")}.`,
    );
  if (s.worktrees > 0)
    lines.push(
      `${plural(s.worktrees, "worktree", "worktrees")}${
        s.worktreesDiscarding > 0
          ? ` (${integer.format(s.worktreesDiscarding)} descartando lo que tiene${s.worktreesDiscarding === 1 ? "" : "n"})`
          : ""
      }.`,
    );
  if (s.branches > 0) {
    const notes: string[] = [];
    if (s.branchesUnintegrated > 0)
      notes.push(
        `${integer.format(s.branchesUnintegrated)} con trabajo sin integrar`,
      );
    if (s.branchesUnknown > 0)
      notes.push(
        `${integer.format(s.branchesUnknown)} sin poder saber si tiene${s.branchesUnknown === 1 ? "" : "n"} trabajo sin integrar`,
      );
    lines.push(
      `${plural(s.branches, "rama", "ramas")}${notes.length > 0 ? ` (${notes.join("; ")})` : ""}.`,
    );
  }
  if (s.orphans > 0)
    lines.push(
      `${plural(s.orphans, "worktree sin intento", "worktrees sin intento")}${
        s.orphansDiscarding > 0
          ? ` (${integer.format(s.orphansDiscarding)} descartando lo que tiene${s.orphansDiscarding === 1 ? "" : "n"})`
          : ""
      }.`,
    );
  return lines;
}

function nothingSelected(s: Summary): boolean {
  return s.outputs + s.worktrees + s.branches + s.orphans === 0;
}

/**
 * «Limpieza de ejecuciones» (HU-025): calcula a demanda cuánto ocupan la salida de los
 * intentos y los worktrees, propone los intentos terminados hace más de la antigüedad
 * configurada y los worktrees sin intento, y limpia solo lo que la persona marca y confirma.
 * Nada se borra solo; las ramas se conservan salvo casilla explícita.
 */
export function RetentionPanel() {
  const id = useId();
  const calc = useMutation(loadPlan);
  const [plan, setPlan] = useState<RetentionPlan | null>(null);
  const [items, setItems] = useState<Record<string, ItemSelection>>({});
  const [orphans, setOrphans] = useState<Record<string, OrphanSelection>>({});
  const [confirming, setConfirming] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [results, setResults] = useState<CleanOutcome[] | null>(null);

  const triggerRef = useRef<HTMLButtonElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const resultsRef = useRef<HTMLHeadingElement>(null);
  // Al cerrar la confirmación, el disparador vuelve a montarse: el foco va a él tras renderizar.
  const restoreFocus = useRef(false);
  useEffect(() => {
    if (confirming) cancelRef.current?.focus();
    else if (restoreFocus.current) {
      restoreFocus.current = false;
      triggerRef.current?.focus();
    }
  }, [confirming]);
  // Tras limpiar, la confirmación (con el foco) desaparece: el foco va al resultado.
  const focusResults = useRef(false);
  useEffect(() => {
    if (results !== null && focusResults.current) {
      focusResults.current = false;
      resultsRef.current?.focus();
    }
  }, [results]);

  async function calculate(): Promise<void> {
    const result = await calc.mutate(undefined);
    if (result.ok) {
      const next = defaults(result.value);
      setPlan(result.value);
      setItems(next.items);
      setOrphans(next.orphans);
    }
  }

  function closeConfirm() {
    restoreFocus.current = true;
    setConfirming(false);
  }

  async function clean(current: RetentionPlan) {
    setCleaning(true);
    const outcomes: CleanOutcome[] = [];
    // Secuencial: un error no detiene los demás.
    for (const item of current.items) {
      const sel = items[item.runId];
      if (!sel) continue;
      const eff = effectiveItem(item, sel);
      if (!eff.output && !eff.worktree && !eff.branch) continue;
      const label = itemTitle(item);
      try {
        const value = await cleanRun({
          projectId: item.projectId,
          runId: item.runId,
          output: eff.output,
          worktree: eff.worktree,
          discardWorktreeChanges: eff.discard,
          branch: eff.branch,
        });
        outcomes.push({
          key: `run-${item.runId}`,
          label,
          ok: true,
          text: describeClean(value),
        });
      } catch (caught) {
        outcomes.push({
          key: `run-${item.runId}`,
          label,
          ok: false,
          text: toUiError(caught).message,
        });
      }
    }
    for (const orphan of current.orphans) {
      const sel = orphans[orphan.path];
      if (!sel) continue;
      const eff = effectiveOrphan(orphan, sel);
      if (!eff.remove) continue;
      const label = `Worktree sin intento ${orphan.path}`;
      try {
        const value = await removeOrphanWorktree({
          path: orphan.path,
          discardChanges: eff.discard,
        });
        outcomes.push({
          key: `orphan-${orphan.path}`,
          label,
          ok: true,
          text: REMOVAL_RESULT[value],
        });
      } catch (caught) {
        outcomes.push({
          key: `orphan-${orphan.path}`,
          label,
          ok: false,
          text: toUiError(caught).message,
        });
      }
    }
    focusResults.current = true;
    setResults(outcomes);
    setConfirming(false);
    setCleaning(false);
    // Lo limpiado ya no se propone: se vuelve a calcular (los resultados siguen visibles).
    await calculate();
  }

  const busy = calc.pending || cleaning;
  const summary = plan ? summarize(plan, items, orphans) : null;
  const empty = summary === null || nothingSelected(summary);
  const okCount = results?.filter((r) => r.ok).length ?? 0;
  const errorCount = (results?.length ?? 0) - okCount;

  return (
    <section className="panel" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className="panel__title">
        Limpieza de ejecuciones
      </h2>
      <p className="field__hint">
        La salida de los intentos y sus worktrees ocupan disco. Aquí puedes ver
        cuánto y elegir qué borrar de los intentos terminados hace más días que
        la antigüedad configurada en «Ejecuciones». Nada se borra solo: nunca se
        proponen intentos en curso ni con una entrega pendiente de revisión, y
        las ramas se conservan salvo que marques borrarlas.
      </p>
      <div className="run-start__actions">
        <button
          type="button"
          className="button button--secondary"
          aria-busy={calc.pending}
          disabled={busy || confirming}
          aria-describedby={`${id}-calc-hint`}
          onClick={() => {
            void calculate();
          }}
        >
          {plan === null
            ? "Calcular qué se puede limpiar"
            : "Volver a calcular"}
        </button>
      </div>
      <p id={`${id}-calc-hint`} className="field__hint">
        El cálculo consulta Git y mide el disco: puede tardar.
      </p>
      <div role="status">
        {calc.pending && (
          <p className="panel__muted">
            Calculando qué se puede limpiar (consultando Git y midiendo el
            disco)…
          </p>
        )}
      </div>
      {calc.error && (
        <p className="panel__error" role="alert">
          No se pudo calcular la limpieza: {calc.error.message}
        </p>
      )}

      {results !== null && (
        <section
          className="retention__results"
          aria-labelledby={`${id}-results`}
        >
          <h3
            id={`${id}-results`}
            className="run-start__label"
            ref={resultsRef}
            tabIndex={-1}
          >
            Resultado de la limpieza
          </h3>
          <p role="status">
            {results.length === 0
              ? "No se limpió nada."
              : `${plural(okCount, "elemento limpiado", "elementos limpiados")}, ${errorCount === 0 ? "ninguno con error" : `${integer.format(errorCount)} con error`}.`}
          </p>
          <ul className="retention__result-list">
            {results.map((r) => (
              <li
                key={r.key}
                className={
                  r.ok ? "retention__result" : "retention__result--error"
                }
              >
                {r.ok ? "✓ " : "Error: "}
                <span className="facts__path">{r.label}</span>: {r.text}
              </li>
            ))}
          </ul>
        </section>
      )}

      {plan !== null && (
        <PlanView
          plan={plan}
          items={items}
          orphans={orphans}
          disabled={busy || confirming}
          onItem={(runId, sel) => {
            setItems((prev) => ({ ...prev, [runId]: sel }));
          }}
          onOrphan={(path, sel) => {
            setOrphans((prev) => ({ ...prev, [path]: sel }));
          }}
        />
      )}

      {plan !== null &&
        summary !== null &&
        (plan.items.length > 0 || plan.orphans.length > 0) &&
        (!confirming ? (
          <div className="run-start__actions">
            <button
              type="button"
              className="button button--danger"
              ref={triggerRef}
              disabled={busy || empty}
              aria-describedby={empty ? `${id}-empty-hint` : undefined}
              onClick={() => {
                setConfirming(true);
              }}
            >
              Limpiar lo seleccionado…
            </button>
            {empty && (
              <p id={`${id}-empty-hint`} className="field__hint">
                Marca al menos un elemento para poder limpiar.
              </p>
            )}
          </div>
        ) : (
          <div
            className="repo__confirm retention__confirm"
            role="alertdialog"
            aria-labelledby={`${id}-confirm-title`}
            aria-describedby={`${id}-confirm-body`}
            onKeyDown={(event) => {
              if (event.key === "Escape" && !cleaning) {
                event.stopPropagation();
                closeConfirm();
              }
            }}
          >
            <p id={`${id}-confirm-title`}>
              <strong>¿Limpiar lo seleccionado? No se puede deshacer.</strong>
            </p>
            <div id={`${id}-confirm-body`}>
              <p>Se borrará:</p>
              <ul>
                {summaryLines(summary).map((line) => (
                  <li key={line}>{line}</li>
                ))}
              </ul>
              <p>
                Se liberarán aproximadamente{" "}
                {summary.bytesPartial ? "al menos " : ""}
                {formatBytes(summary.bytes)}.
              </p>
            </div>
            <div className="repo__actions">
              <button
                type="button"
                className="button button--danger"
                aria-busy={cleaning}
                disabled={cleaning}
                onClick={() => {
                  void clean(plan);
                }}
              >
                Limpiar
              </button>
              <button
                type="button"
                className="button button--secondary"
                ref={cancelRef}
                disabled={cleaning}
                onClick={closeConfirm}
              >
                Cancelar
              </button>
            </div>
            {cleaning && (
              <p className="panel__muted" role="status">
                Limpiando…
              </p>
            )}
          </div>
        ))}
    </section>
  );
}

interface PlanViewProps {
  plan: RetentionPlan;
  items: Record<string, ItemSelection>;
  orphans: Record<string, OrphanSelection>;
  disabled: boolean;
  onItem: (runId: string, sel: ItemSelection) => void;
  onOrphan: (path: string, sel: OrphanSelection) => void;
}

function PlanView({
  plan,
  items,
  orphans,
  disabled,
  onItem,
  onOrphan,
}: PlanViewProps) {
  const id = useId();
  const nothing = plan.items.length === 0 && plan.orphans.length === 0;
  return (
    <div className="retention">
      <dl className="facts facts--compact">
        <dt>Salida de los intentos</dt>
        <dd>
          {formatUsage(plan.runsUsage)} ({filesText(plan.runsUsage)})
        </dd>
        <dt>Worktrees</dt>
        <dd>
          {formatUsage(plan.worktreesUsage)} ({filesText(plan.worktreesUsage)})
        </dd>
      </dl>
      {(!plan.runsUsage.complete || !plan.worktreesUsage.complete) && (
        <p className="field__hint">
          «Al menos»: la medición se detuvo por límite de archivos o de tiempo.
        </p>
      )}

      {nothing && (
        <p className="panel__muted">
          No hay nada que limpiar con la antigüedad configurada.
        </p>
      )}

      {plan.items.length > 0 && (
        <section aria-labelledby={`${id}-items`}>
          <h3 id={`${id}-items`} className="run-start__label">
            Intentos terminados antes del {formatDateTime(plan.cutoff)} (más de{" "}
            {plural(plan.retentionDays, "día", "días")})
          </h3>
          <ul className="retention__list">
            {plan.items.map((item) => (
              <ItemRow
                key={item.runId}
                item={item}
                sel={items[item.runId] ?? defaultItem(item)}
                disabled={disabled}
                onChange={(sel) => {
                  onItem(item.runId, sel);
                }}
              />
            ))}
          </ul>
          {plan.itemsTruncated && (
            <p className="retention__warn">
              Hay más intentos para limpiar de los que se muestran: limpia estos
              y vuelve a calcular.
            </p>
          )}
        </section>
      )}

      {plan.orphans.length > 0 && (
        <section aria-labelledby={`${id}-orphans`}>
          <h3 id={`${id}-orphans`} className="run-start__label">
            Worktrees sin intento
          </h3>
          <p className="field__hint">
            Carpetas de worktrees que la base no conoce (p. ej. tras restaurar
            una copia anterior). La rama se conserva.
          </p>
          <ul className="retention__list">
            {plan.orphans.map((orphan) => (
              <OrphanRow
                key={orphan.path}
                orphan={orphan}
                sel={orphans[orphan.path] ?? { remove: false, discard: false }}
                disabled={disabled}
                onChange={(sel) => {
                  onOrphan(orphan.path, sel);
                }}
              />
            ))}
          </ul>
          {plan.orphansTruncated && (
            <p className="retention__warn">
              Hay más carpetas sin intento de las que se revisan de una vez:
              elimina estas y vuelve a calcular.
            </p>
          )}
        </section>
      )}
    </div>
  );
}

function sizeOf(usage: DiskUsage | null): string {
  return usage === null ? "tamaño no disponible" : formatUsage(usage);
}

function ItemRow({
  item,
  sel,
  disabled,
  onChange,
}: {
  item: RetentionItem;
  sel: ItemSelection;
  disabled: boolean;
  onChange: (sel: ItemSelection) => void;
}) {
  const id = useId();
  const wt = item.worktree;
  const check = wt?.check;
  const wtEffective = worktreeEffective(check, sel.worktree, sel.discard);
  const branchAllowed =
    item.branch !== null && (!item.branch.worktreeLive || wtEffective);
  return (
    <li className="retention__item">
      <fieldset className="retention__fieldset" disabled={disabled}>
        <legend className="retention__legend">
          {itemTitle(item)} · {item.projectName} · {RUN_STATE_LABEL[item.state]}{" "}
          · terminó{" "}
          <time dateTime={item.endedAt}>{formatDateTime(item.endedAt)}</time>
        </legend>

        {item.output !== null ? (
          <label className="checkbox retention__checkbox">
            <input
              type="checkbox"
              checked={sel.output}
              onChange={(event) => {
                onChange({ ...sel, output: event.target.checked });
              }}
            />
            Salida ({formatUsage(item.output)})
          </label>
        ) : (
          <p className="field__hint">Sin salida guardada.</p>
        )}

        {wt !== null && check !== undefined && (
          <div className="retention__worktree">
            {check.kind === "unverifiable" ? (
              <p className="retention__warn">
                Worktree <span className="facts__path">{wt.path}</span>: no se
                puede eliminar desde aquí. {check.reason}
              </p>
            ) : (
              <>
                <label className="checkbox retention__checkbox">
                  <input
                    type="checkbox"
                    checked={sel.worktree}
                    aria-describedby={`${id}-wt-path`}
                    onChange={(event) => {
                      const on = event.target.checked;
                      onChange({
                        ...sel,
                        worktree: on,
                        discard: on ? sel.discard : false,
                        // Sin worktree eliminado, una rama viva no se puede borrar.
                        branch:
                          on || !item.branch?.worktreeLive ? sel.branch : false,
                      });
                    }}
                  />
                  Worktree (
                  {check.kind === "missing"
                    ? "la carpeta ya no existe"
                    : sizeOf(wt.usage)}
                  )
                </label>
                <p id={`${id}-wt-path`} className="field__hint">
                  <span className="facts__path">{wt.path}</span> · rama{" "}
                  <span className="input--mono">{wt.branch}</span>
                </p>
                {check.kind === "needsForce" && (
                  <>
                    <p id={`${id}-force`} className="retention__warn">
                      {check.reason}
                    </p>
                    <label className="checkbox retention__checkbox retention__nested">
                      <input
                        type="checkbox"
                        checked={sel.discard}
                        disabled={!sel.worktree}
                        aria-describedby={`${id}-force`}
                        onChange={(event) => {
                          const on = event.target.checked;
                          onChange({
                            ...sel,
                            discard: on,
                            branch:
                              on || !item.branch?.worktreeLive
                                ? sel.branch
                                : false,
                          });
                        }}
                      />
                      Descartar lo que tiene y eliminar
                    </label>
                    {sel.worktree && !sel.discard && (
                      <p className="field__hint">
                        Sin «Descartar lo que tiene y eliminar», este worktree
                        no se eliminará.
                      </p>
                    )}
                  </>
                )}
              </>
            )}
          </div>
        )}

        {item.branch !== null && (
          <div className="retention__branch">
            <label className="checkbox retention__checkbox">
              <input
                type="checkbox"
                checked={sel.branch && branchAllowed}
                disabled={!branchAllowed}
                aria-describedby={`${id}-branch`}
                onChange={(event) => {
                  onChange({ ...sel, branch: event.target.checked });
                }}
              />
              Borrar la rama{" "}
              <span className="input--mono">{item.branch.name}</span>
            </label>
            <div id={`${id}-branch`}>
              <p
                className={
                  item.branch.unintegratedCommits === 0
                    ? "field__hint"
                    : "retention__warn"
                }
              >
                {branchWarning(item.branch)}
              </p>
              {item.branch.worktreeLive && (
                <p className="field__hint">
                  Para borrar la rama hay que eliminar también su worktree (se
                  elimina antes).
                </p>
              )}
            </div>
          </div>
        )}
      </fieldset>
    </li>
  );
}

function OrphanRow({
  orphan,
  sel,
  disabled,
  onChange,
}: {
  orphan: OrphanWorktree;
  sel: OrphanSelection;
  disabled: boolean;
  onChange: (sel: OrphanSelection) => void;
}) {
  const id = useId();
  const check = orphan.check;
  return (
    <li className="retention__item">
      <fieldset className="retention__fieldset" disabled={disabled}>
        <legend className="retention__legend">
          <span className="facts__path">{orphan.path}</span>
        </legend>
        <p className="field__hint">
          Rama:{" "}
          {orphan.branch === null ? (
            "ninguna (HEAD no apunta a una rama)"
          ) : (
            <span className="input--mono">{orphan.branch}</span>
          )}{" "}
          ·{" "}
          {check.kind === "missing"
            ? "la carpeta ya no existe"
            : sizeOf(orphan.usage)}{" "}
          · {orphanState(check)}
        </p>
        {check.kind === "unverifiable" ? (
          <p className="retention__warn">
            No se puede eliminar desde aquí. {check.reason}
          </p>
        ) : (
          <>
            <label className="checkbox retention__checkbox">
              <input
                type="checkbox"
                checked={sel.remove}
                onChange={(event) => {
                  const on = event.target.checked;
                  onChange({ remove: on, discard: on ? sel.discard : false });
                }}
              />
              Eliminar
            </label>
            {check.kind === "needsForce" && (
              <>
                <p id={`${id}-force`} className="retention__warn">
                  {check.reason}
                </p>
                <label className="checkbox retention__checkbox retention__nested">
                  <input
                    type="checkbox"
                    checked={sel.discard}
                    disabled={!sel.remove}
                    aria-describedby={`${id}-force`}
                    onChange={(event) => {
                      onChange({ ...sel, discard: event.target.checked });
                    }}
                  />
                  Descartar lo que tiene
                </label>
                {sel.remove && !sel.discard && (
                  <p className="field__hint">
                    Sin «Descartar lo que tiene», esta carpeta no se eliminará.
                  </p>
                )}
              </>
            )}
          </>
        )}
      </fieldset>
    </li>
  );
}

function orphanState(check: WorktreeCheck): string {
  switch (check.kind) {
    case "clean":
      return "sin cambios";
    case "needsForce":
      return "eliminarla perdería algo";
    case "missing":
      return "ya no existe";
    case "unverifiable":
      return "no se pudo comprobar";
  }
}
