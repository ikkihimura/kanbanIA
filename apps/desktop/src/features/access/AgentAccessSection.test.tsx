import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Agent } from "../../contracts/generated/Agent";
import type { AgentCredential } from "../../contracts/generated/AgentCredential";
import * as commands from "../../shared/ipc/commands";
import { AgentAccessSection } from "./AgentAccessSection";

vi.mock("../../shared/ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  listAgents: vi.fn(),
  listAgentCredentials: vi.fn(),
  createAgentCredential: vi.fn(),
  revokeAgentCredential: vi.fn(),
  getLocalServiceStatus: vi.fn(),
}));

const mocked = vi.mocked(commands);
const TOKEN = `kbia_${"ab".repeat(32)}`;

function agent(overrides: Partial<Agent> = {}): Agent {
  return {
    id: "a1",
    number: 1,
    name: "Robota",
    role: "",
    provider: "",
    model: "",
    capabilities: [],
    avatar: {
      head: 0,
      skin: 0,
      eyes: 0,
      mouth: 0,
      hair: 0,
      hairColor: 0,
      accessory: 0,
      background: 0,
    },
    assignedStories: 0,
    revision: 1,
    ...overrides,
  };
}

function credential(overrides: Partial<AgentCredential> = {}): AgentCredential {
  return {
    id: "c1",
    projectId: "p1",
    agentId: "a1",
    agentName: "Robota",
    agentNumber: 1,
    tokenHint: "abab",
    createdAt: "2026-11-12T10:00:00.000Z",
    lastUsedAt: null,
    revokedAt: null,
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocked.listAgents.mockResolvedValue([
    agent(),
    agent({ id: "a2", number: 2, name: "Lupa" }),
  ]);
  mocked.listAgentCredentials.mockResolvedValue([
    credential(),
    credential({
      id: "c0",
      tokenHint: "9f9f",
      lastUsedAt: "2026-11-11T09:00:00.000Z",
      revokedAt: "2026-11-11T10:00:00.000Z",
    }),
  ]);
  mocked.getLocalServiceStatus.mockResolvedValue({
    running: true,
    port: 51234,
  });
});

describe("AgentAccessSection", () => {
  it("muestra el servicio local y el estado de cada credencial con texto", async () => {
    render(<AgentAccessSection projectId="p1" />);
    expect(
      await screen.findByText(/activo en este equipo \(127\.0\.0\.1:51234\)/),
    ).toBeInTheDocument();
    const items = await screen.findAllByRole("listitem");
    expect(items).toHaveLength(2);
    const [active, revoked] = items;
    if (!active || !revoked) throw new Error("faltan credenciales");
    expect(active).toHaveTextContent("A-1 Robota …abab Activa");
    expect(active).toHaveTextContent("Último uso: nunca");
    expect(revoked).toHaveTextContent("Revocada");
    // Una revocada no se puede revocar otra vez.
    expect(within(revoked).queryByRole("button")).not.toBeInTheDocument();
    expect(mocked.listAgentCredentials).toHaveBeenCalledWith("p1");
  });

  it("sin servicio lo dice", async () => {
    mocked.getLocalServiceStatus.mockResolvedValue({ running: false });
    render(<AgentAccessSection projectId="p1" />);
    expect(
      await screen.findByText(
        "Servicio local: no disponible. Los agentes no pueden conectarse ahora; cierra Kanban IA y vuelve a abrirlo.",
      ),
    ).toBeInTheDocument();
  });

  it("crea una credencial y muestra el token una sola vez, con aviso y copia", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    mocked.createAgentCredential.mockResolvedValue({
      credential: credential({ id: "c2", agentId: "a2", agentName: "Lupa" }),
      token: TOKEN,
    });
    render(<AgentAccessSection projectId="p1" />);
    const create = screen.getByRole("button", { name: "Crear credencial" });
    expect(create).toBeDisabled();
    await user.selectOptions(
      await screen.findByLabelText("Agente"),
      await screen.findByRole("option", { name: "A-2 Lupa" }),
    );
    await user.click(create);
    expect(mocked.createAgentCredential).toHaveBeenCalledWith({
      projectId: "p1",
      agentId: "a2",
    });

    const field = await screen.findByRole("textbox", {
      name: "Credencial de Lupa",
    });
    expect(field).toHaveValue(TOKEN);
    expect(field).toHaveFocus();
    expect(screen.getByText(/no se volverá a mostrar/)).toBeInTheDocument();
    await waitFor(() => {
      expect(mocked.listAgentCredentials).toHaveBeenCalledTimes(2);
    });

    await user.click(screen.getByRole("button", { name: "Copiar" }));
    expect(writeText).toHaveBeenCalledWith(TOKEN);
    expect(
      await screen.findByText("✓ Copiada al portapapeles."),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Ya la guardé" }));
    expect(screen.queryByDisplayValue(TOKEN)).not.toBeInTheDocument();
    // El foco no se pierde al desaparecer el bloque.
    expect(
      screen.getByRole("heading", { name: "Acceso de agentes" }),
    ).toHaveFocus();
    expect(document.body).not.toHaveTextContent(TOKEN);
  });

  it("si no se puede copiar, deja el token seleccionado y lo explica", async () => {
    const user = userEvent.setup();
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: vi.fn().mockRejectedValue(new Error("denegado")) },
      configurable: true,
    });
    mocked.createAgentCredential.mockResolvedValue({
      credential: credential(),
      token: TOKEN,
    });
    render(<AgentAccessSection projectId="p1" />);
    await user.selectOptions(
      await screen.findByLabelText("Agente"),
      await screen.findByRole("option", { name: "A-1 Robota" }),
    );
    await user.click(screen.getByRole("button", { name: "Crear credencial" }));
    await user.click(await screen.findByRole("button", { name: "Copiar" }));
    expect(await screen.findByText(/No se pudo copiar/)).toBeInTheDocument();
  });

  it("sin agentes no se puede crear y se explica", async () => {
    mocked.listAgents.mockResolvedValue([]);
    render(<AgentAccessSection projectId="p1" />);
    const select = await screen.findByLabelText("Agente");
    await waitFor(() => {
      expect(select).toBeDisabled();
    });
    expect(select).toHaveAccessibleDescription(
      "Primero crea un agente en «Equipo».",
    );
    expect(
      screen.getByRole("button", { name: "Crear credencial" }),
    ).toBeDisabled();
  });

  it("revocar pide confirmación accesible y avisa al terminar", async () => {
    const user = userEvent.setup();
    mocked.revokeAgentCredential.mockResolvedValue(
      credential({ revokedAt: "2026-11-12T11:00:00.000Z" }),
    );
    render(<AgentAccessSection projectId="p1" />);
    const trigger = await screen.findByRole("button", { name: "Revocar…" });
    await user.click(trigger);
    const dialog = screen.getByRole("alertdialog", {
      name: /Revocar la credencial de Robota \(…abab\)/,
    });
    expect(
      within(dialog).getByRole("button", { name: "Cancelar" }),
    ).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
    expect(mocked.revokeAgentCredential).not.toHaveBeenCalled();

    await user.click(trigger);
    await user.click(screen.getByRole("button", { name: "Revocar" }));
    expect(mocked.revokeAgentCredential).toHaveBeenCalledWith({
      projectId: "p1",
      id: "c1",
    });
    expect(
      await screen.findByText(/Credencial de Robota revocada/),
    ).toBeInTheDocument();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Acceso de agentes" }),
    ).toHaveFocus();
  });

  it("muestra el error al crear", async () => {
    const user = userEvent.setup();
    mocked.createAgentCredential.mockRejectedValue({
      code: "validation",
      message: "Este proyecto ya tiene 20 credenciales activas.",
      recoverable: true,
    });
    render(<AgentAccessSection projectId="p1" />);
    await user.selectOptions(
      await screen.findByLabelText("Agente"),
      await screen.findByRole("option", { name: "A-1 Robota" }),
    );
    await user.click(screen.getByRole("button", { name: "Crear credencial" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "ya tiene 20 credenciales activas",
    );
  });

  it("si revocar falla, lo dice y la confirmación sigue abierta", async () => {
    const user = userEvent.setup();
    mocked.revokeAgentCredential.mockRejectedValue({
      code: "storage_unavailable",
      message: "No se pudo acceder al almacenamiento local.",
      recoverable: true,
    });
    render(<AgentAccessSection projectId="p1" />);
    await user.click(await screen.findByRole("button", { name: "Revocar…" }));
    await user.click(screen.getByRole("button", { name: "Revocar" }));
    const dialog = screen.getByRole("alertdialog");
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "No se pudo acceder al almacenamiento local.",
    );
  });

  it("si no cargan las credenciales o el estado del servicio, lo dice", async () => {
    mocked.listAgentCredentials.mockRejectedValue({
      code: "storage_locked",
      message: "Kanban IA ya está abierto con estos datos.",
      recoverable: true,
    });
    mocked.getLocalServiceStatus.mockRejectedValue(new Error("ipc"));
    render(<AgentAccessSection projectId="p1" />);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Kanban IA ya está abierto con estos datos.",
    );
    expect(
      await screen.findByText(
        "Servicio local: no se pudo consultar su estado. Cierra Kanban IA y vuelve a abrirlo.",
      ),
    ).toBeInTheDocument();
  });
});
