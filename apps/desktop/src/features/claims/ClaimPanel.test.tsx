import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ErrorPayload } from "../../contracts/generated/ErrorPayload";
import type { StoryClaim } from "../../contracts/generated/StoryClaim";
import * as commands from "../../shared/ipc/commands";
import { ClaimPanel } from "./ClaimPanel";

// Solo se simulan las llamadas IPC; errores y helpers son los reales.
vi.mock("../../shared/ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  listActiveClaims: vi.fn(),
  releaseClaim: vi.fn(),
}));

const mocked = vi.mocked(commands);

const EXPIRES = "2099-11-12T10:30:00Z";

function claim(overrides: Partial<StoryClaim> = {}): StoryClaim {
  return {
    claimId: "c1",
    storyId: "s1",
    storyNumber: 7,
    owner: { agentId: "a1", agentNumber: 3, agentName: "Jean Claude" },
    claimedAt: "2026-11-12T10:00:00Z",
    expiresAt: EXPIRES,
    renewedAt: null,
    ...overrides,
  };
}

function hourOf(iso: string) {
  return new Date(iso).toLocaleTimeString("es", {
    hour: "2-digit",
    minute: "2-digit",
  });
}

function renderPanel(onReleased = vi.fn()) {
  render(<ClaimPanel projectId="p1" storyId="s1" onReleased={onReleased} />);
  return { onReleased };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocked.listActiveClaims.mockResolvedValue([]);
  mocked.releaseClaim.mockResolvedValue(undefined);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("ClaimPanel", () => {
  it("sin reclamo vigente lo indica y no ofrece liberar", async () => {
    // Un reclamo de otra historia no cuenta.
    mocked.listActiveClaims.mockResolvedValue([
      claim({ storyId: "s2", claimId: "c2" }),
    ]);
    renderPanel();

    expect(await screen.findByText("Sin reclamo.")).toBeInTheDocument();
    expect(mocked.listActiveClaims).toHaveBeenCalledWith("p1");
    expect(
      screen.queryByRole("button", { name: /Liberar/ }),
    ).not.toBeInTheDocument();
  });

  it("muestra el agente, el vencimiento y la renovación", async () => {
    mocked.listActiveClaims.mockResolvedValue([
      claim({ renewedAt: "2026-11-12T10:05:00Z" }),
    ]);
    renderPanel();

    expect(await screen.findByText("A-3 Jean Claude")).toBeInTheDocument();
    expect(
      screen.getByText(new RegExp(`Vence a las ${hourOf(EXPIRES)}\\.`)),
    ).toBeInTheDocument();
    expect(screen.getByText(/^Renovado el /)).toBeInTheDocument();
    expect(screen.queryByText("Sin reclamo.")).not.toBeInTheDocument();
  });

  it("un reclamo sin renovar lo indica", async () => {
    mocked.listActiveClaims.mockResolvedValue([claim()]);
    renderPanel();

    expect(await screen.findByText(/Sin renovar\./)).toBeInTheDocument();
  });

  it("liberar pide confirmación, llama al comando y confirma con foco estable", async () => {
    const user = userEvent.setup();
    mocked.listActiveClaims.mockResolvedValueOnce([claim()]);
    const { onReleased } = renderPanel();

    await user.click(
      await screen.findByRole("button", { name: "Liberar reclamo…" }),
    );
    const dialog = screen.getByRole("alertdialog");
    expect(dialog).toHaveTextContent("dejará de poder renovarlo");
    expect(dialog).toHaveTextContent("también se quitará la asignación");
    expect(dialog).toHaveTextContent("libre para cualquier agente");
    expect(dialog).toHaveTextContent("bloquéala o muévela");
    expect(dialog).not.toHaveTextContent("Relaciones");
    expect(
      within(dialog).getByRole("button", { name: "Cancelar" }),
    ).toHaveFocus();
    expect(mocked.releaseClaim).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: "Liberar" }));

    expect(mocked.releaseClaim).toHaveBeenCalledWith({
      projectId: "p1",
      claimId: "c1",
    });
    expect(
      await screen.findByText(
        /Reclamo de Jean Claude liberado y asignación quitada/,
      ),
    ).toBeInTheDocument();
    expect(onReleased).toHaveBeenCalledTimes(1);
    expect(await screen.findByText("Sin reclamo.")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Reclamo" })).toHaveFocus();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(mocked.listActiveClaims).toHaveBeenCalledTimes(2);
  });

  it("cancelar (o Escape) no libera y devuelve el foco al botón", async () => {
    const user = userEvent.setup();
    mocked.listActiveClaims.mockResolvedValue([claim()]);
    renderPanel();

    await user.click(
      await screen.findByRole("button", { name: "Liberar reclamo…" }),
    );
    await user.click(screen.getByRole("button", { name: "Cancelar" }));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Liberar reclamo…" }),
    ).toHaveFocus();

    await user.click(screen.getByRole("button", { name: "Liberar reclamo…" }));
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(mocked.releaseClaim).not.toHaveBeenCalled();
  });

  it("si liberar falla, muestra el error y mantiene la confirmación", async () => {
    const user = userEvent.setup();
    mocked.listActiveClaims.mockResolvedValue([claim()]);
    mocked.releaseClaim.mockRejectedValue({
      code: "not_found",
      message: "No se encontró: Reclamo.",
      recoverable: true,
    } satisfies ErrorPayload);
    const { onReleased } = renderPanel();

    await user.click(
      await screen.findByRole("button", { name: "Liberar reclamo…" }),
    );
    await user.click(screen.getByRole("button", { name: "Liberar" }));

    const dialog = screen.getByRole("alertdialog");
    expect(
      await within(dialog).findByText("No se encontró: Reclamo."),
    ).toHaveAttribute("role", "alert");
    expect(
      within(dialog).getByRole("button", { name: "Liberar" }),
    ).toBeEnabled();
    expect(
      within(dialog).getByRole("button", { name: "Liberar" }),
    ).toHaveAttribute("aria-busy", "false");
    expect(screen.queryByText(/liberado/)).not.toBeInTheDocument();
    expect(onReleased).not.toHaveBeenCalled();
  });

  it("un reclamo deja de mostrarse al vencer, sin recargar", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date("2026-11-12T10:20:00Z"));
    mocked.listActiveClaims.mockResolvedValue([
      claim({ expiresAt: "2026-11-12T10:30:00Z" }),
    ]);
    renderPanel();

    expect(await screen.findByText("A-3 Jean Claude")).toBeInTheDocument();
    // Los efectos (que programan el temporizador) se ejecutan antes de avanzar el reloj.
    await act(async () => {
      await Promise.resolve();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(9 * 60 * 1000);
    });
    expect(screen.getByText("A-3 Jean Claude")).toBeInTheDocument();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(60 * 1000 + 10);
    });
    expect(screen.queryByText("A-3 Jean Claude")).not.toBeInTheDocument();
    expect(screen.getByText("Sin reclamo.")).toBeInTheDocument();
    expect(mocked.listActiveClaims).toHaveBeenCalledTimes(1);
  });

  it("un reclamo ya vencido al cargar no se muestra", async () => {
    mocked.listActiveClaims.mockResolvedValue([
      claim({ expiresAt: "2000-01-01T00:00:00Z" }),
    ]);
    renderPanel();

    expect(await screen.findByText("Sin reclamo.")).toBeInTheDocument();
  });

  it("si no se puede consultar, lo dice y permite reintentar", async () => {
    const user = userEvent.setup();
    mocked.listActiveClaims.mockRejectedValueOnce({
      code: "internal",
      message: "Error interno.",
      recoverable: true,
    } satisfies ErrorPayload);
    renderPanel();

    expect(
      await screen.findByText(/No se pudo consultar el reclamo/),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Reintentar" }));
    await waitFor(() => {
      expect(screen.getByText("Sin reclamo.")).toBeInTheDocument();
    });
  });
});
