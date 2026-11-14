import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ActivityEntry } from "../../contracts/generated/ActivityEntry";
import type { ErrorPayload } from "../../contracts/generated/ErrorPayload";
import * as commands from "../../shared/ipc/commands";
import { formatDateTime, formatTime } from "../../shared/ui/format";
import { AgentActivityPanel } from "./AgentActivityPanel";

vi.mock("../../shared/ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  listStoryActivity: vi.fn(),
}));

const mocked = vi.mocked(commands);

const robo = { agentId: "a1", agentNumber: 2, agentName: "Robo" };
const nova = { agentId: "a2", agentNumber: 5, agentName: "Nova" };

function entry(
  kind: ActivityEntry["kind"],
  at: string,
  extra: Partial<ActivityEntry> = {},
): ActivityEntry {
  return { kind, agent: robo, detail: null, at, ...extra };
}

// Textos que afirmarían que el proceso del agente sigue vivo: nunca deben aparecer.
const EXECUTION_CLAIM =
  /trabajando|en ejecuci[oó]n|ejecut[aá]ndose|activo ahora|en curso|en l[ií]nea/i;

beforeEach(() => {
  vi.clearAllMocks();
});

describe("AgentActivityPanel", () => {
  it("muestra cada acción reportada con agente, texto y hora", async () => {
    // Hoy: se muestra solo la hora (HH:MM).
    const today = new Date();
    today.setHours(9, 41, 0, 0);
    const at = today.toISOString();
    mocked.listStoryActivity.mockResolvedValue({
      lastReportedAt: at,
      entries: [
        entry("delivered", at, { detail: "E2" }),
        entry("subtask_created", at, { detail: "#12" }),
        entry("blocker_cleared", at),
        entry("blocker_reported", at, { agent: nova }),
        entry("renewed", at),
        entry("claimed", at),
      ],
    });
    render(<AgentActivityPanel projectId="p1" storyId="s1" />);

    const list = await screen.findByRole("list", {
      name: /Acciones reportadas/,
    });
    const lines = within(list)
      .getAllByRole("listitem")
      .map((li) => li.textContent);
    const hhmm = formatTime(at);
    expect(lines).toEqual([
      `A-2 RoboEntregó E2${hhmm}`,
      `A-2 RoboCreó la subtarea #12${hhmm}`,
      `A-2 RoboQuitó su bloqueo${hhmm}`,
      `A-5 NovaReportó un bloqueo${hhmm}`,
      `A-2 RoboRenovó el reclamo${hhmm}`,
      `A-2 RoboReclamó${hhmm}`,
    ]);
    expect(mocked.listStoryActivity).toHaveBeenCalledWith({
      projectId: "p1",
      storyId: "s1",
    });
    expect(screen.getByText(/Reportado por el agente/)).toBeInTheDocument();
    expect(screen.getByText(/Última interacción reportada:/)).toHaveTextContent(
      `Última interacción reportada: ${hhmm}`,
    );
    expect(
      screen.getByText(/no indica que el agente siga ocupándose/),
    ).toBeInTheDocument();
    // La hora lleva el instante exacto, legible por máquina y con fecha completa al pasar.
    const times = list.querySelectorAll("time");
    expect(times[0]).toHaveAttribute("datetime", at);
    expect(times[0]).toHaveAttribute("title", formatDateTime(at));
  });

  it("de otro día muestra fecha y hora", async () => {
    const at = "2026-01-05T10:00:00Z";
    mocked.listStoryActivity.mockResolvedValue({
      lastReportedAt: at,
      entries: [entry("claimed", at)],
    });
    render(<AgentActivityPanel projectId="p1" storyId="s1" />);

    expect(
      await screen.findByText(/Última interacción reportada:/),
    ).toHaveTextContent(formatDateTime(at));
  });

  it("sin detalle, las acciones con referencia siguen siendo legibles", async () => {
    const at = "2026-01-05T10:00:00Z";
    mocked.listStoryActivity.mockResolvedValue({
      lastReportedAt: at,
      entries: [entry("subtask_created", at), entry("delivered", at)],
    });
    render(<AgentActivityPanel projectId="p1" storyId="s1" />);

    expect(await screen.findByText("Creó una subtarea")).toBeInTheDocument();
    expect(screen.getByText("Registró una entrega")).toBeInTheDocument();
  });

  it("muestra el estado de carga y el vacío", async () => {
    mocked.listStoryActivity.mockResolvedValue({
      lastReportedAt: null,
      entries: [],
    });
    render(<AgentActivityPanel projectId="p1" storyId="s1" />);

    expect(screen.getByRole("status")).toHaveTextContent("Cargando actividad…");
    expect(
      await screen.findByText(
        "Ningún agente ha reportado actividad en esta historia.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Última interacción/)).not.toBeInTheDocument();
    expect(screen.queryByRole("list")).not.toBeInTheDocument();
  });

  it("muestra el error y permite reintentar", async () => {
    const user = userEvent.setup();
    mocked.listStoryActivity
      .mockRejectedValueOnce({
        code: "not_found",
        message: "No se encontró: Historia.",
        recoverable: true,
      } satisfies ErrorPayload)
      .mockResolvedValue({ lastReportedAt: null, entries: [] });
    render(<AgentActivityPanel projectId="p1" storyId="s1" />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "No se pudo cargar la actividad del agente: No se encontró: Historia.",
    );
    await user.click(screen.getByRole("button", { name: "Reintentar" }));
    expect(
      await screen.findByText(/Ningún agente ha reportado actividad/),
    ).toBeInTheDocument();
  });

  it("nunca afirma que el agente siga ejecutando", async () => {
    const at = new Date().toISOString();
    const all: ActivityEntry["kind"][] = [
      "claimed",
      "renewed",
      "blocker_reported",
      "blocker_cleared",
      "subtask_created",
      "delivered",
    ];
    mocked.listStoryActivity.mockResolvedValue({
      lastReportedAt: at,
      entries: all.map((kind) => entry(kind, at, { detail: "#3" })),
    });
    const { container } = render(
      <AgentActivityPanel projectId="p1" storyId="s1" />,
    );

    await screen.findByRole("list", { name: /Acciones reportadas/ });
    expect(container.textContent).not.toMatch(EXECUTION_CLAIM);
    for (const element of container.querySelectorAll("[aria-label],[title]")) {
      expect(element.getAttribute("aria-label") ?? "").not.toMatch(
        EXECUTION_CLAIM,
      );
      expect(element.getAttribute("title") ?? "").not.toMatch(EXECUTION_CLAIM);
    }
  });

  it("vuelve a consultar al cambiar la señal, sin pasar por «Cargando»", async () => {
    const at = "2026-01-05T10:00:00Z";
    mocked.listStoryActivity
      .mockResolvedValueOnce({
        lastReportedAt: at,
        entries: [entry("claimed", at)],
      })
      .mockResolvedValue({
        lastReportedAt: at,
        entries: [entry("renewed", at), entry("claimed", at)],
      });
    const { rerender } = render(
      <AgentActivityPanel projectId="p1" storyId="s1" refreshSignal={0} />,
    );
    expect(await screen.findByText("Reclamó")).toBeInTheDocument();

    rerender(
      <AgentActivityPanel projectId="p1" storyId="s1" refreshSignal={1} />,
    );
    expect(screen.queryByText("Cargando actividad…")).not.toBeInTheDocument();
    expect(await screen.findByText("Renovó el reclamo")).toBeInTheDocument();
    expect(mocked.listStoryActivity).toHaveBeenCalledTimes(2);
  });
});
