import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { StoryDetail } from "../../contracts/generated/StoryDetail";
import type { StoryReview } from "../../contracts/generated/StoryReview";
import * as commands from "../../shared/ipc/commands";
import { AttentionSection } from "./AttentionSection";
import { ReviewPanel } from "./ReviewPanel";

vi.mock("../../shared/ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  getStoryReview: vi.fn(),
  acceptStory: vi.fn(),
  reopenStory: vi.fn(),
  submitDelivery: vi.fn(),
  returnDelivery: vi.fn(),
  setStoryBlocker: vi.fn(),
  listAttention: vi.fn(),
  listColumns: vi.fn(),
}));

const mocked = vi.mocked(commands);

function story(overrides: Partial<StoryDetail> = {}): StoryDetail {
  return {
    id: "s1",
    projectId: "p1",
    number: 4,
    columnId: "c-review",
    position: 0,
    title: "Informe",
    description: "",
    priority: "medium",
    archivedAt: null,
    acceptedAt: null,
    criteria: [
      { id: "k1", number: 1, text: "Descarga CSV" },
      { id: "k2", number: 2, text: "Con cabeceras" },
    ],
    revision: 3,
    createdAt: "2026-11-10T10:00:00.000Z",
    updatedAt: "2026-11-10T10:00:00.000Z",
    ...overrides,
  };
}

function review(overrides: Partial<StoryReview> = {}): StoryReview {
  return {
    storyId: "s1",
    acceptedAt: null,
    blocker: null,
    deliveries: [
      {
        id: "d1",
        storyId: "s1",
        number: 1,
        source: "reported",
        agentId: "g1",
        agentName: "Robota",
        summary: "Exportación lista",
        references: [
          {
            kind: "url",
            value: "https://example.com/pr/1",
            status: "unverified",
          },
          { kind: "path", value: "/tmp/borrado.csv", status: "missing" },
        ],
        status: "pending",
        returnReason: null,
        createdAt: "2026-11-10T11:00:00.000Z",
        decidedAt: null,
      },
    ],
    history: [],
    ...overrides,
  };
}

function renderPanel(detail = story(), locked = false) {
  const onStoryChanged = vi.fn();
  render(
    <ReviewPanel
      story={detail}
      locked={locked}
      onStoryChanged={onStoryChanged}
    />,
  );
  return onStoryChanged;
}

beforeEach(() => {
  mocked.getStoryReview.mockResolvedValue(review());
  mocked.listColumns.mockResolvedValue([]);
});

describe("ReviewPanel", () => {
  it("muestra entregas reportadas y referencias inaccesibles como tales", async () => {
    renderPanel();

    expect(
      await screen.findByText("Reportada por Robota", { exact: false }),
    ).toBeInTheDocument();
    expect(screen.getByText("URL no verificada")).toBeInTheDocument();
    expect(screen.getByText("No encontrada")).toBeInTheDocument();
    expect(
      screen.getByText("1 entrega pendiente de revisión"),
    ).toBeInTheDocument();
  });

  it("aceptar exige marcar todos los criterios y envía los marcados", async () => {
    const user = userEvent.setup();
    mocked.acceptStory.mockResolvedValue({} as never);
    const onStoryChanged = renderPanel();

    const accept = await screen.findByRole("button", {
      name: "Aceptar historia",
    });
    expect(accept).toBeDisabled();
    await user.click(screen.getByLabelText("Cumple CA-1: Descarga CSV"));
    expect(accept).toBeDisabled();
    await user.click(screen.getByLabelText("Cumple CA-2: Con cabeceras"));
    expect(accept).toBeEnabled();
    await user.click(accept);

    await waitFor(() => {
      expect(onStoryChanged).toHaveBeenCalled();
    });
    expect(mocked.acceptStory).toHaveBeenCalledWith({
      projectId: "p1",
      storyId: "s1",
      expectedRevision: 3,
      satisfiedCriteria: ["k1", "k2"],
    });
    expect(
      await screen.findByText("✓ Historia #4 aceptada."),
    ).toBeInTheDocument();
  });

  it("sin criterios no se puede aceptar y lo explica", async () => {
    renderPanel(story({ criteria: [] }));

    expect(
      await screen.findByText(
        /Añade al menos un criterio de aceptación verificable/,
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Aceptar historia" }),
    ).toBeDisabled();
  });

  it("con cambios sin guardar bloquea aceptar y entregar", async () => {
    const user = userEvent.setup();
    renderPanel(story(), true);

    expect(
      await screen.findByText(/Guarda o descarta los cambios/),
    ).toBeInTheDocument();
    await user.click(screen.getByLabelText("Cumple CA-1: Descarga CSV"));
    await user.click(screen.getByLabelText("Cumple CA-2: Con cabeceras"));
    expect(
      screen.getByRole("button", { name: "Aceptar historia" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Registrar entrega" }),
    ).toBeDisabled();
  });

  it("reabrir exige motivo y muestra el historial conservado", async () => {
    const user = userEvent.setup();
    mocked.getStoryReview.mockResolvedValue(
      review({
        acceptedAt: "2026-11-10T12:00:00.000Z",
        history: [
          {
            kind: "accepted",
            criteria: [{ number: 1, text: "Descarga CSV" }],
            reason: null,
            createdAt: "2026-11-10T12:00:00.000Z",
          },
        ],
      }),
    );
    mocked.reopenStory.mockResolvedValue({} as never);
    renderPanel(story({ acceptedAt: "2026-11-10T12:00:00.000Z" }));

    const reopen = await screen.findByRole("button", { name: "Reabrir" });
    expect(reopen).toBeDisabled();
    expect(screen.getByText("✓ CA-1: Descarga CSV")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Aceptar historia" }),
    ).toBeNull();

    await user.type(
      screen.getByLabelText("Motivo de la reapertura"),
      "Regresión",
    );
    await user.click(reopen);
    expect(mocked.reopenStory).toHaveBeenCalledWith({
      projectId: "p1",
      storyId: "s1",
      expectedRevision: 3,
      reason: "Regresión",
    });
  });

  it("registra una entrega con referencias y la envía a revisión", async () => {
    const user = userEvent.setup();
    const [base] = review().deliveries;
    if (!base) throw new Error("falta la entrega de ejemplo");
    mocked.submitDelivery.mockResolvedValue({
      ...base,
      id: "d2",
      number: 2,
      source: "manual",
    });
    renderPanel();

    await user.click(
      await screen.findByRole("button", { name: "Registrar entrega" }),
    );
    await user.type(
      screen.getByLabelText("Resumen de lo entregado"),
      "Segunda versión",
    );
    await user.click(screen.getByRole("button", { name: "Añadir referencia" }));
    await user.type(
      screen.getByLabelText("Referencia 1"),
      "https://example.com/pr/2",
    );
    await user.click(screen.getByRole("button", { name: "Registrar" }));

    await waitFor(() => {
      expect(mocked.submitDelivery).toHaveBeenCalledTimes(1);
    });
    const input = mocked.submitDelivery.mock.calls[0]?.[0];
    expect(input?.summary).toBe("Segunda versión");
    expect(input?.references).toEqual([
      { kind: "url", value: "https://example.com/pr/2" },
    ]);
    expect(input?.sendToReview).toBe(true);
    expect(
      await screen.findByText("✓ Entrega E2 registrada; revisión solicitada."),
    ).toBeInTheDocument();
  });

  it("muestra el error de acción no permitida que devuelve el núcleo", async () => {
    const user = userEvent.setup();
    mocked.acceptStory.mockRejectedValue({
      code: "forbidden",
      message:
        "Solo una persona puede aceptar una historia; un agente no puede hacerlo.",
      recoverable: false,
    });
    renderPanel();

    await user.click(await screen.findByLabelText("Cumple CA-1: Descarga CSV"));
    await user.click(screen.getByLabelText("Cumple CA-2: Con cabeceras"));
    await user.click(screen.getByRole("button", { name: "Aceptar historia" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Solo una persona",
    );
  });
});

describe("ReviewPanel: criterios vigentes", () => {
  it("al cambiar la revisión se vuelven a comprobar y no se envían criterios borrados", async () => {
    const user = userEvent.setup();
    mocked.acceptStory.mockResolvedValue({} as never);
    const onStoryChanged = vi.fn();
    const { rerender } = render(
      <ReviewPanel
        story={story()}
        locked={false}
        onStoryChanged={onStoryChanged}
      />,
    );
    await user.click(await screen.findByLabelText("Cumple CA-1: Descarga CSV"));
    await user.click(screen.getByLabelText("Cumple CA-2: Con cabeceras"));

    // Se guardó una edición que borró CA-2: la revisión sube y las marcas se reinician.
    rerender(
      <ReviewPanel
        story={story({
          revision: 4,
          criteria: [{ id: "k1", number: 1, text: "Descarga CSV" }],
        })}
        locked={false}
        onStoryChanged={onStoryChanged}
      />,
    );
    const ca1 = await screen.findByLabelText("Cumple CA-1: Descarga CSV");
    expect(ca1).not.toBeChecked();
    expect(
      screen.getByRole("button", { name: "Aceptar historia" }),
    ).toHaveAccessibleDescription(
      "Marca todos los criterios para poder aceptar.",
    );
    await user.click(ca1);
    await user.click(screen.getByRole("button", { name: "Aceptar historia" }));

    expect(mocked.acceptStory).toHaveBeenCalledWith(
      expect.objectContaining({
        expectedRevision: 4,
        satisfiedCriteria: ["k1"],
      }),
    );
  });
});

describe("AttentionSection", () => {
  it("una fila por historia con todos sus motivos", async () => {
    mocked.listAttention.mockResolvedValue([
      {
        storyId: "s1",
        number: 4,
        title: "Informe",
        columnName: "Revisión",
        blockerReason: "Esperando diseño",
        pendingPrerequisites: 1,
        pendingDeliveries: 2,
        since: "2026-11-10T11:00:00.000Z",
      },
    ]);
    const onOpen = vi.fn();
    render(<AttentionSection projectId="p1" onOpenStory={onOpen} />);

    expect(
      await screen.findByRole("heading", { name: "Atención (1)" }),
    ).toBeInTheDocument();
    const row = screen.getByRole("button", { name: /#4 Informe/ });
    expect(row).toHaveTextContent(
      "Revisión · Bloqueada: Esperando diseño · 2 entregas pendientes de revisión · 1 prerequisito sin aceptar",
    );
    await userEvent.setup().click(row);
    expect(onOpen).toHaveBeenCalledWith("s1");
  });

  it("muestra el estado vacío", async () => {
    mocked.listAttention.mockResolvedValue([]);
    render(<AttentionSection projectId="p1" onOpenStory={vi.fn()} />);

    expect(await screen.findByText(/Nada pendiente/)).toBeInTheDocument();
  });
});
