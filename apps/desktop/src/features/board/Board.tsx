import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import type { Agent } from "../../contracts/generated/Agent";
import type { Column } from "../../contracts/generated/Column";
import type { Epic } from "../../contracts/generated/Epic";
import type { StoryClaim } from "../../contracts/generated/StoryClaim";
import type { StorySummary } from "../../contracts/generated/StorySummary";
import {
  listActiveClaims,
  listAgents,
  listColumns,
  listEpics,
  listStories,
  moveStory,
  toUiError,
} from "../../shared/ipc/commands";
import { useIpcResource } from "../../shared/ipc/useIpcResource";
import { formatTime } from "../../shared/ui/format";
import { useUnexpiredClaims } from "../claims/useUnexpiredClaims";
import { PRIORITY_LABEL } from "../stories/priorities";
import { PixelAvatar } from "../team/PixelAvatar";
import { AcceptDialog } from "./AcceptDialog";
import {
  applyMove,
  cardsByColumn,
  dropIndex,
  keyboardTarget,
  laneOf,
  lanesFor,
  matchesFilter,
  type BoardFilter,
  type KeyMove,
  type LaneMode,
} from "./boardModel";

const NO_CARDS: StorySummary[] = [];
const NO_CLAIMS: StoryClaim[] = [];

const KEY_MOVES: Record<string, KeyMove> = {
  ArrowLeft: "left",
  ArrowRight: "right",
  ArrowUp: "up",
  ArrowDown: "down",
};

const MOVE_LABEL: Record<KeyMove, string> = {
  left: "a la columna anterior",
  right: "a la columna siguiente",
  up: "una posición arriba",
  down: "una posición abajo",
};

interface Props {
  projectId: string;
  onOpenStory: (storyId: string) => void;
}

interface Message {
  text: string;
  error: boolean;
}

interface DropHint {
  columnId: string;
  laneKey: string;
  /** `null` = al final del carril en esa columna. */
  beforeId: string | null;
}

interface Accepting {
  storyId: string;
  columnId: string;
  index: number;
}

/**
 * Tablero: columnas con sus tarjetas, filtros y carriles visuales. Mover es optimista: la
 * vista cambia al instante y vuelve atrás si el núcleo rechaza el movimiento (WIP, conflicto,
 * aceptación). Soltar en «hecho» abre la aceptación humana.
 */
export function Board({ projectId, onOpenStory }: Props) {
  const id = useId();
  const loadColumns = useCallback(() => listColumns(projectId), [projectId]);
  const loadStories = useCallback(
    () => listStories(projectId, false),
    [projectId],
  );
  const loadEpics = useCallback(() => listEpics(projectId), [projectId]);
  const loadClaims = useCallback(
    () => listActiveClaims(projectId),
    [projectId],
  );
  const columns = useIpcResource(loadColumns);
  const stories = useIpcResource(loadStories);
  const epics = useIpcResource(loadEpics);
  const agents = useIpcResource(listAgents);
  // Los reclamos son informativos: si no cargan, el tablero funciona igual.
  const claims = useIpcResource(loadClaims);

  const [filter, setFilter] = useState<BoardFilter>({ text: "", epicId: "" });
  const [laneMode, setLaneMode] = useState<LaneMode>("none");
  const [message, setMessage] = useState<Message | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const [dropHint, setDropHint] = useState<DropHint | null>(null);
  const [accepting, setAccepting] = useState<Accepting | null>(null);
  // Movimiento optimista ligado a los datos sobre los que se calculó: cuando llega la lista
  // recargada (otra identidad), deja de aplicarse sola.
  const [optimistic, setOptimistic] = useState<{
    base: StorySummary[];
    cards: StorySummary[];
  } | null>(null);
  const cardButtons = useRef(new Map<string, HTMLButtonElement>());
  // Tarjeta que debe tener el foco tras mover. Se mantiene hasta que llega una lista distinta
  // de `until` (la tarjeta puede remontarse en otra columna); `null` = basta con enfocarla una vez.
  // Es una ref: el efecto la consume al cambiar las tarjetas o cerrarse el diálogo.
  const focus = useRef<{ id: string; until: StorySummary[] | null } | null>(
    null,
  );
  const setFocus = (next: { id: string; until: StorySummary[] | null }) => {
    focus.current = next;
  };

  const base = stories.state.status === "ready" ? stories.state.data : null;
  const busy = optimistic !== null && optimistic.base === base;
  const cards = busy ? optimistic.cards : (base ?? NO_CARDS);
  const columnList = useMemo(
    () => (columns.state.status === "ready" ? columns.state.data : []),
    [columns.state],
  );
  const epicList = epics.state.status === "ready" ? epics.state.data : [];
  const agentList = agents.state.status === "ready" ? agents.state.data : [];
  // Un reclamo vencido deja de mostrarse al vencer, sin esperar a recargar.
  const activeClaims = useUnexpiredClaims(
    claims.state.status === "ready" ? claims.state.data : NO_CLAIMS,
  );
  const claimByStory = new Map(activeClaims.map((c) => [c.storyId, c]));
  const byColumn = useMemo(
    () => cardsByColumn(columnList, cards),
    [columnList, cards],
  );

  // Tras mover (teclado o aceptación), el foco sigue a la tarjeta aunque cambie de columna.
  // Nunca mientras el diálogo de aceptación está abierto: el foco debe quedarse dentro.
  useEffect(() => {
    const target = focus.current;
    if (target === null || accepting !== null) return;
    const button = cardButtons.current.get(target.id);
    if (button) {
      button.focus();
      if (target.until !== base) focus.current = null;
    }
  }, [cards, base, accepting]);

  // Recarga las historias y, con ellas, los reclamos vigentes.
  const reloadStories = useCallback(() => {
    stories.run(loadStories, { keepData: true });
    claims.run(loadClaims, { keepData: true });
  }, [stories, claims, loadStories, loadClaims]);

  const reload = useCallback(() => {
    reloadStories();
    columns.run(loadColumns, { keepData: true });
  }, [reloadStories, columns, loadColumns]);

  if (columns.state.status === "error" || stories.state.status === "error") {
    const text =
      columns.state.status === "error"
        ? columns.state.message
        : stories.state.status === "error"
          ? stories.state.message
          : "";
    return (
      <section className="panel" aria-label="Tablero">
        <p className="panel__error" role="alert">
          {text}
        </p>
        <button type="button" className="button" onClick={reload}>
          Reintentar
        </button>
      </section>
    );
  }
  if (base === null || columns.state.status !== "ready") {
    return (
      <p className="panel__muted" role="status">
        Cargando tablero…
      </p>
    );
  }
  const loaded = base;

  const columnName = (columnId: string) =>
    columnList.find((c) => c.id === columnId)?.name ?? "";

  async function move(
    storyId: string,
    columnId: string,
    index: number,
    keepFocus: boolean,
  ) {
    const story = cards.find((s) => s.id === storyId);
    if (busy) {
      if (story)
        setMessage({
          text: `Espera a que termine el movimiento anterior para mover #${String(story.number)}.`,
          error: false,
        });
      return;
    }
    const column = columnList.find((c) => c.id === columnId);
    if (!story || !column || story.position === null) return;
    if (column.category === "done" && story.acceptedAt === null) {
      setMessage(null);
      setAccepting({ storyId, columnId, index });
      return;
    }
    const sameColumn = story.columnId === columnId;
    const others = byColumn.get(columnId)?.length ?? 0;
    if (sameColumn && Math.min(index, others - 1) === story.position) return;

    setOptimistic({
      base: loaded,
      cards: applyMove(cards, storyId, columnId, index),
    });
    if (keepFocus) setFocus({ id: storyId, until: loaded });
    try {
      const saved = await moveStory({
        projectId,
        id: storyId,
        expectedRevision: story.revision,
        toColumnId: columnId,
        toIndex: index,
      });
      setMessage({
        text: `#${String(story.number)} movida a «${column.name}», posición ${String((saved.position ?? 0) + 1)}.`,
        error: false,
      });
      // La vista optimista se mantiene hasta que llega la lista guardada.
      reloadStories();
    } catch (raw) {
      const error = toUiError(raw);
      // Vuelve al estado anterior: la tarjeta sigue donde estaba.
      setOptimistic(null);
      if (error.code === "acceptance_required") {
        // La columna pasó a «hecho» en otra edición: se pide la aceptación.
        setAccepting({ storyId, columnId, index });
        reload();
        return;
      }
      const reloaded = error.code === "conflict";
      if (keepFocus) setFocus({ id: storyId, until: reloaded ? loaded : null });
      setMessage({
        text: reloaded
          ? `${error.message} Se recargó el tablero.`
          : `No se movió #${String(story.number)}: ${error.message}`,
        error: true,
      });
      if (reloaded) reload();
    }
  }

  function accepted(story: StorySummary) {
    setAccepting(null);
    // Mostrar ya la tarjeta aceptada en su sitio; sin mover hasta que llegue la recarga.
    setOptimistic({
      base: loaded,
      cards: applyMove(
        cards.map((s) =>
          s.id === story.id
            ? { ...story, columnId: s.columnId, position: s.position }
            : s,
        ),
        story.id,
        story.columnId,
        story.position ?? 0,
      ),
    });
    setFocus({ id: story.id, until: loaded });
    setMessage({
      text: `#${String(story.number)} aceptada y movida a «${columnName(story.columnId)}», posición ${String((story.position ?? 0) + 1)}.`,
      error: false,
    });
    reload();
  }

  function keyMove(story: StorySummary, cell: StorySummary[], key: KeyMove) {
    const target = keyboardTarget(story, columnList, byColumn, cell, key);
    if (!target) {
      setMessage({
        text: `#${String(story.number)} no se puede mover ${MOVE_LABEL[key]}.`,
        error: false,
      });
      return;
    }
    void move(story.id, target.columnId, target.index, true);
  }

  function drop(
    columnId: string,
    beforeId: string | null,
    afterId: string | null,
  ) {
    const dragged = dragId;
    setDragId(null);
    setDropHint(null);
    // Soltar sobre sí misma (o tras sí misma) no es un movimiento.
    if (dragged === null || beforeId === dragged || afterId === dragged) return;
    const index = dropIndex(byColumn.get(columnId) ?? [], dragged, {
      beforeStoryId: beforeId,
      afterStoryId: afterId,
    });
    void move(dragged, columnId, index, false);
  }

  function hint(next: DropHint) {
    if (
      dropHint?.columnId !== next.columnId ||
      dropHint.laneKey !== next.laneKey ||
      dropHint.beforeId !== next.beforeId
    ) {
      setDropHint(next);
    }
  }

  const lanes = lanesFor(laneMode, epicList, agentList);
  const laneKeys = new Set(lanes.map((l) => l.key));
  // Si una épica o agente no está en la lista (p. ej. no cargó), la tarjeta cae en «Sin…».
  const laneKeyOf = (s: StorySummary) => {
    const key = laneOf(s, laneMode);
    return laneKeys.has(key) ? key : "none";
  };
  const visible = cards.filter(
    (s) => s.position !== null && matchesFilter(s, filter),
  );
  const activeCount = cards.filter((s) => s.position !== null).length;
  const rows = lanes
    .map((lane) => ({
      lane,
      cells: columnList.map((column) =>
        (byColumn.get(column.id) ?? []).filter(
          (s) => matchesFilter(s, filter) && laneKeyOf(s) === lane.key,
        ),
      ),
    }))
    .filter(
      (row) => laneMode === "none" || row.cells.some((cell) => cell.length > 0),
    );

  return (
    <section className="board" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className="visually-hidden">
        Tablero
      </h2>
      <form
        className="board__toolbar form-row"
        aria-label="Filtros del tablero"
        onSubmit={(event) => {
          event.preventDefault();
        }}
      >
        <div className="field field--grow">
          <label htmlFor={`${id}-text`}>Buscar</label>
          <input
            id={`${id}-text`}
            type="search"
            value={filter.text}
            placeholder="Título o #número"
            autoComplete="off"
            onChange={(event) => {
              setFilter({ ...filter, text: event.target.value });
            }}
          />
        </div>
        <div className="field field--narrow">
          <label htmlFor={`${id}-epic`}>Épica</label>
          <select
            id={`${id}-epic`}
            value={filter.epicId}
            onChange={(event) => {
              setFilter({ ...filter, epicId: event.target.value });
            }}
          >
            <option value="">Todas</option>
            <option value="none">Sin épica</option>
            {epicList.map((e) => (
              <option key={e.id} value={e.id}>
                E-{e.number} {e.title}
              </option>
            ))}
          </select>
        </div>
        <div className="field field--narrow">
          <label htmlFor={`${id}-lanes`}>Carriles</label>
          <select
            id={`${id}-lanes`}
            value={laneMode}
            onChange={(event) => {
              setLaneMode(event.target.value as LaneMode);
            }}
          >
            <option value="none">Sin carriles</option>
            <option value="epic">Por épica</option>
            <option value="agent">Por agente</option>
          </select>
        </div>
      </form>
      <p className="field__hint" id={`${id}-help`}>
        Arrastra las tarjetas o usa Alt + flechas sobre una tarjeta para
        moverla. Los carriles solo agrupan: mover cambia la columna, no la épica
        ni el agente.
        {visible.length !== activeCount &&
          ` Mostrando ${String(visible.length)} de ${String(activeCount)} tarjetas.`}
      </p>
      <p role="status" className="relations__saved">
        {message && !message.error && `✓ ${message.text}`}
      </p>
      <p role="alert" className="panel__error">
        {message?.error && message.text}
      </p>
      {(epics.state.status === "error" || agents.state.status === "error") && (
        <p className="panel__error">
          No se pudieron cargar las épicas o los agentes: las tarjetas se
          muestran sin ellos.
        </p>
      )}
      {claims.state.status === "error" && (
        <p className="field__hint">
          No se pudieron cargar los reclamos de agentes: las tarjetas se
          muestran sin ellos.
        </p>
      )}

      {columnList.length === 0 ? (
        <p className="panel__muted">
          Este proyecto no tiene columnas. Créalas en «Configuración».
        </p>
      ) : (
        <div className="board__scroll">
          <div className="board__row board__head">
            {columnList.map((column) => (
              <ColumnHeader
                key={column.id}
                column={column}
                count={byColumn.get(column.id)?.length ?? 0}
              />
            ))}
          </div>
          {visible.length === 0 && (
            <p className="panel__muted">
              Ninguna tarjeta coincide con el filtro.
            </p>
          )}
          {rows.map(({ lane, cells }) => (
            <div
              key={lane.key}
              className="board__lane"
              role={laneMode === "none" ? undefined : "group"}
              aria-label={
                laneMode === "none" ? undefined : `Carril ${lane.label}`
              }
            >
              {laneMode !== "none" && (
                <h3 className="board__lane-title">{lane.label}</h3>
              )}
              <div className="board__row">
                {columnList.map((column, columnIndex) => {
                  const cell = cells[columnIndex] ?? [];
                  const endHint =
                    dropHint?.columnId === column.id &&
                    dropHint.laneKey === lane.key &&
                    dropHint.beforeId === null;
                  return (
                    <div
                      key={column.id}
                      className={`board__cell${endHint ? " board__cell--drop" : ""}`}
                      role="group"
                      aria-label={
                        laneMode === "none"
                          ? column.name
                          : `${column.name}, carril ${lane.label}`
                      }
                      onDragOver={(event) => {
                        if (dragId === null) return;
                        event.preventDefault();
                        hint({
                          columnId: column.id,
                          laneKey: lane.key,
                          beforeId: null,
                        });
                      }}
                      onDrop={(event) => {
                        event.preventDefault();
                        drop(
                          column.id,
                          null,
                          cell[cell.length - 1]?.id ?? null,
                        );
                      }}
                    >
                      <ol className="board__cards">
                        {cell.map((story) => (
                          <BoardCard
                            key={story.id}
                            story={story}
                            epic={epicList.find((e) => e.id === story.epicId)}
                            agent={agentList.find(
                              (a) => a.id === story.agentId,
                            )}
                            claim={claimByStory.get(story.id)}
                            helpId={`${id}-help`}
                            disabled={busy}
                            dragging={dragId === story.id}
                            dropBefore={
                              dropHint?.beforeId === story.id &&
                              dropHint.laneKey === lane.key
                            }
                            buttonRef={(element) => {
                              if (element)
                                cardButtons.current.set(story.id, element);
                              else cardButtons.current.delete(story.id);
                            }}
                            onOpen={() => {
                              onOpenStory(story.id);
                            }}
                            onKeyMove={(key) => {
                              keyMove(story, cell, key);
                            }}
                            onDragStart={() => {
                              setMessage(null);
                              setDragId(story.id);
                            }}
                            onDragEnd={() => {
                              setDragId(null);
                              setDropHint(null);
                            }}
                            onDragOver={() => {
                              hint({
                                columnId: column.id,
                                laneKey: lane.key,
                                beforeId: story.id,
                              });
                            }}
                            onDrop={() => {
                              drop(column.id, story.id, null);
                            }}
                            canDrop={dragId !== null}
                          />
                        ))}
                      </ol>
                    </div>
                  );
                })}
              </div>
            </div>
          ))}
        </div>
      )}

      {accepting && (
        <AcceptDialog
          projectId={projectId}
          storyId={accepting.storyId}
          columnName={columnName(accepting.columnId)}
          target={{ columnId: accepting.columnId, toIndex: accepting.index }}
          onAccepted={(story) => {
            accepted(story);
          }}
          onCancel={() => {
            setFocus({ id: accepting.storyId, until: null });
            setAccepting(null);
            setMessage({
              text: "Aceptación cancelada: la tarjeta sigue donde estaba.",
              error: false,
            });
          }}
        />
      )}
    </section>
  );
}

function ColumnHeader({ column, count }: { column: Column; count: number }) {
  const full = column.wipLimit !== null && count >= column.wipLimit;
  return (
    <div className="board__column-head">
      <h3 className="board__column-name">{column.name}</h3>
      <span className="board__column-count">
        {column.wipLimit === null
          ? `${String(count)} tarjetas`
          : `${String(count)} / ${String(column.wipLimit)} WIP`}
      </span>
      {full && <span className="badge badge--warn">Límite WIP alcanzado</span>}
      {column.category === "done" && (
        <span className="board__column-note">Requiere aceptación</span>
      )}
    </div>
  );
}

interface CardProps {
  story: StorySummary;
  epic: Epic | undefined;
  agent: Agent | undefined;
  claim: StoryClaim | undefined;
  helpId: string;
  disabled: boolean;
  dragging: boolean;
  dropBefore: boolean;
  canDrop: boolean;
  buttonRef: (element: HTMLButtonElement | null) => void;
  onOpen: () => void;
  onKeyMove: (key: KeyMove) => void;
  onDragStart: () => void;
  onDragEnd: () => void;
  onDragOver: () => void;
  onDrop: () => void;
}

function BoardCard({
  story,
  epic,
  agent,
  claim,
  helpId,
  disabled,
  dragging,
  dropBefore,
  canDrop,
  buttonRef,
  onOpen,
  onKeyMove,
  onDragStart,
  onDragEnd,
  onDragOver,
  onDrop,
}: CardProps) {
  const claimId = useId();
  const blocked =
    story.blockerReason !== null || story.pendingPrerequisites > 0;
  const classes = [
    "card",
    dragging ? "card--dragging" : "",
    dropBefore ? "card--drop-before" : "",
    blocked ? "card--blocked" : "",
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <li
      className={classes}
      draggable={!disabled}
      onDragStart={(event) => {
        event.dataTransfer.effectAllowed = "move";
        event.dataTransfer.setData("text/plain", `#${String(story.number)}`);
        onDragStart();
      }}
      onDragEnd={onDragEnd}
      onDragOver={(event) => {
        if (!canDrop) return;
        event.preventDefault();
        event.stopPropagation();
        onDragOver();
      }}
      onDrop={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onDrop();
      }}
      onKeyDown={(event) => {
        const key = KEY_MOVES[event.key];
        if (!event.altKey || key === undefined) return;
        event.preventDefault();
        if (!disabled) onKeyMove(key);
      }}
    >
      <button
        type="button"
        className="card__open"
        ref={buttonRef}
        // El reclamo se anuncia al enfocar la tarjeta, antes de la ayuda de teclado.
        aria-describedby={claim ? `${claimId} ${helpId}` : helpId}
        aria-keyshortcuts="Alt+ArrowLeft Alt+ArrowRight Alt+ArrowUp Alt+ArrowDown"
        onClick={onOpen}
      >
        <span className="story-number">#{story.number}</span> {story.title}
      </button>
      <p className="card__meta">
        Prioridad {PRIORITY_LABEL[story.priority].toLowerCase()}
        {epic && ` · E-${String(epic.number)}`}
      </p>
      {story.acceptedAt && <span className="badge badge--ok">✓ Aceptada</span>}
      {story.blockerReason !== null && (
        <span className="badge badge--warn card__blocker">
          Bloqueada: {story.blockerReason}
        </span>
      )}
      {story.pendingPrerequisites > 0 && (
        <span className="badge badge--warn">
          Espera {story.pendingPrerequisites}{" "}
          {story.pendingPrerequisites === 1 ? "prerequisito" : "prerequisitos"}
        </span>
      )}
      {claim && (
        <p className="card__meta card__claim" id={claimId}>
          Reclamada por {claim.owner.agentName} · vence{" "}
          {formatTime(claim.expiresAt)}
        </p>
      )}
      {agent && (
        <span className="assignee card__agent">
          <PixelAvatar
            spec={agent.avatar}
            name={agent.name}
            size={24}
            decorative
          />
          {agent.name}
        </span>
      )}
      <div
        className="card__moves"
        aria-label={`Mover #${String(story.number)}`}
        role="group"
      >
        {(["left", "up", "down", "right"] as const).map((key) => (
          <button
            key={key}
            type="button"
            className="card__move"
            // Fuera del orden de tabulación: con teclado se usa Alt + flechas.
            tabIndex={-1}
            disabled={disabled}
            aria-label={`Mover #${String(story.number)} ${MOVE_LABEL[key]}`}
            onClick={() => {
              onKeyMove(key);
            }}
          >
            {{ left: "←", up: "↑", down: "↓", right: "→" }[key]}
          </button>
        ))}
      </div>
    </li>
  );
}
