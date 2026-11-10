import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Agent } from "../../contracts/generated/Agent";
import type { AvatarSpec } from "../../contracts/generated/AvatarSpec";
import * as commands from "../../shared/ipc/commands";
import { AvatarEditor } from "./AvatarEditor";
import { TeamView } from "./TeamView";

vi.mock("../../shared/ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  getUserProfile: vi.fn(),
  updateUserProfile: vi.fn(),
  listAgents: vi.fn(),
  createAgent: vi.fn(),
  updateAgent: vi.fn(),
}));

const mocked = vi.mocked(commands);

const AVATAR: AvatarSpec = {
  head: 0,
  skin: 1,
  eyes: 0,
  mouth: 0,
  hair: 1,
  hairColor: 2,
  accessory: 0,
  background: 5,
};

function agent(overrides: Partial<Agent> = {}): Agent {
  return {
    id: "a1",
    number: 1,
    name: "Robota",
    role: "Backend",
    provider: "Anthropic",
    model: "claude-opus-5-5",
    capabilities: ["implement"],
    avatar: { ...AVATAR, head: 2 },
    assignedStories: 2,
    revision: 1,
    ...overrides,
  };
}

beforeEach(() => {
  mocked.getUserProfile.mockResolvedValue({
    name: "Yo",
    avatar: AVATAR,
    revision: 1,
  });
  mocked.listAgents.mockResolvedValue([agent()]);
});

function Harness() {
  const [value, setValue] = useState(AVATAR);
  return <AvatarEditor value={value} onChange={setValue} name="Ikki" />;
}

describe("AvatarEditor", () => {
  it("se usa solo con teclado y la vista previa describe el resultado", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    // Colores: grupo de opciones navegable con flechas.
    const fondo = screen.getByRole("group", { name: /Fondo/ });
    const checked = within(fondo).getByRole("radio", { name: "Mostaza" });
    expect(checked).toBeChecked();
    checked.focus();
    await user.keyboard("{ArrowRight}");
    expect(
      within(fondo).getByRole("radio", { name: "Petróleo" }),
    ).toBeChecked();
    expect(fondo).toHaveTextContent("Fondo: Petróleo");

    // Formas: listas desplegables con etiqueta.
    await user.selectOptions(screen.getByLabelText("Cabeza"), "Robot");
    await user.selectOptions(screen.getByLabelText("Accesorio"), "Auriculares");
    expect(
      screen.getByRole("img", { name: /Avatar de Ikki: cabeza robot/ }),
    ).toHaveAccessibleName(
      "Avatar de Ikki: cabeza robot, ojos puntos, corto, auriculares",
    );
  });
});

describe("TeamView", () => {
  it("muestra agentes separados del usuario y sin actividad simulada", async () => {
    render(<TeamView />);

    expect(await screen.findByDisplayValue("Yo")).toBeInTheDocument();
    const card = (await screen.findByText("Robota")).closest("li");
    expect(card).not.toBeNull();
    expect(card).toHaveTextContent("2 historias (sin actividad registrada)");
    expect(card).toHaveTextContent("claude-opus-5-5");
    expect(card).not.toHaveTextContent(/trabajando|en línea|activo/i);
  });

  it("cambiar el modelo conserva id y avatar del agente", async () => {
    const user = userEvent.setup();
    mocked.updateAgent.mockImplementation((input) =>
      Promise.resolve(agent({ model: input.model, revision: 2 })),
    );
    render(<TeamView />);

    await user.click(
      await screen.findByRole("button", { name: "Editar A-1 Robota" }),
    );
    const model = screen.getByLabelText("Modelo (informativo)");
    await user.clear(model);
    await user.type(model, "modelo-nuevo");
    await user.click(screen.getByRole("button", { name: "Guardar agente" }));

    await waitFor(() => {
      expect(mocked.updateAgent).toHaveBeenCalledTimes(1);
    });
    const input = mocked.updateAgent.mock.calls[0]?.[0];
    expect(input?.id).toBe("a1");
    expect(input?.model).toBe("modelo-nuevo");
    expect(input?.avatar).toEqual(agent().avatar);
    expect(input?.name).toBe("Robota");
    expect(
      await screen.findByText("✓ Agente A-1 Robota guardado."),
    ).toBeInTheDocument();
  });

  it("muestra el error de nombre duplicado junto al campo", async () => {
    const user = userEvent.setup();
    mocked.createAgent.mockRejectedValue({
      code: "already_exists",
      message: "Un agente con ese nombre ya existe",
      recoverable: true,
      field: "name",
    });
    render(<TeamView />);

    await user.click(
      await screen.findByRole("button", { name: "Nuevo agente" }),
    );
    await user.type(
      screen.getByLabelText("Nombre (identidad estable)"),
      "robota",
    );
    await user.click(screen.getByRole("button", { name: "Crear agente" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Un agente con ese nombre ya existe",
    );
    expect(screen.getByLabelText("Nombre (identidad estable)")).toHaveAttribute(
      "aria-invalid",
      "true",
    );
  });

  it("guardar el perfil conserva el foco y anuncia el resultado", async () => {
    const user = userEvent.setup();
    mocked.updateUserProfile.mockImplementation((input) =>
      Promise.resolve({ name: input.name, avatar: input.avatar, revision: 2 }),
    );
    render(<TeamView />);

    const name = await screen.findByLabelText("Nombre");
    await user.clear(name);
    await user.type(name, "Ikki{Enter}");

    expect(await screen.findByText("✓ Perfil guardado.")).toHaveAttribute(
      "role",
      "status",
    );
    expect(screen.getByLabelText("Nombre")).toHaveFocus();
    expect(screen.getByLabelText("Nombre")).toBe(name);
  });

  it("un conflicto en el perfil conserva lo editado y el reintento usa la revisión vigente", async () => {
    const user = userEvent.setup();
    mocked.updateUserProfile
      .mockRejectedValueOnce({
        code: "conflict",
        message: "Otra edición cambió estos datos.",
        recoverable: true,
        current: { name: "Otro", avatar: AVATAR, revision: 5 },
      })
      .mockImplementationOnce((input) =>
        Promise.resolve({
          name: input.name,
          avatar: input.avatar,
          revision: 6,
        }),
      );
    render(<TeamView />);

    const name = await screen.findByLabelText("Nombre");
    await user.clear(name);
    await user.type(name, "Mío");
    await user.click(screen.getByRole("button", { name: "Guardar perfil" }));

    expect(
      await screen.findByText(/Tus cambios siguen aquí/),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Nombre")).toHaveValue("Mío");

    await user.click(screen.getByRole("button", { name: "Guardar perfil" }));
    await waitFor(() => {
      expect(mocked.updateUserProfile).toHaveBeenCalledTimes(2);
    });
    expect(mocked.updateUserProfile.mock.calls[1]?.[0].expectedRevision).toBe(
      5,
    );
    expect(await screen.findByText("✓ Perfil guardado.")).toBeInTheDocument();
  });

  it("un conflicto al editar un agente no bloquea: el reintento usa la revisión vigente", async () => {
    const user = userEvent.setup();
    mocked.updateAgent
      .mockRejectedValueOnce({
        code: "conflict",
        message: "Otra edición cambió estos datos.",
        recoverable: true,
        current: agent({ revision: 4 }),
      })
      .mockImplementationOnce((input) =>
        Promise.resolve(agent({ model: input.model, revision: 5 })),
      );
    render(<TeamView />);

    await user.click(
      await screen.findByRole("button", { name: "Editar A-1 Robota" }),
    );
    const model = screen.getByLabelText("Modelo (informativo)");
    await user.clear(model);
    await user.type(model, "otro");
    await user.click(screen.getByRole("button", { name: "Guardar agente" }));
    expect(
      await screen.findByText(/Tus cambios siguen aquí/),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Modelo (informativo)")).toHaveValue("otro");

    await user.click(screen.getByRole("button", { name: "Guardar agente" }));
    await waitFor(() => {
      expect(mocked.updateAgent).toHaveBeenCalledTimes(2);
    });
    expect(mocked.updateAgent.mock.calls[0]?.[0].expectedRevision).toBe(1);
    expect(mocked.updateAgent.mock.calls[1]?.[0].expectedRevision).toBe(4);
  });
});
