import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ImportPreview } from "../../contracts/generated/ImportPreview";
import * as commands from "../../shared/ipc/commands";
import { chooseFolder } from "../../shared/ui/chooseFolder";
import { ExportSection } from "./ExportSection";
import { ImportSection } from "./ImportSection";

vi.mock("../../shared/ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  exportProject: vi.fn(),
  previewImport: vi.fn(),
  applyImport: vi.fn(),
}));
vi.mock("../../shared/ui/chooseFolder", () => ({ chooseFolder: vi.fn() }));

const mocked = vi.mocked(commands);
const choose = vi.mocked(chooseFolder);

function preview(overrides: Partial<ImportPreview> = {}): ImportPreview {
  return {
    fingerprint: "huella",
    projectId: "p1",
    projectName: "Planificación",
    projectExists: true,
    items: [
      {
        id: "s1",
        kind: "story",
        label: "#1 Login",
        action: "create",
        localLabel: null,
      },
      {
        id: "s2",
        kind: "story",
        label: "#2 API",
        action: "unchanged",
        localLabel: null,
      },
    ],
    warnings: ["#1 «Login» se importará como #4."],
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("ImportSection", () => {
  it("previsualiza, muestra avisos y aplica con la huella de la vista previa", async () => {
    const user = userEvent.setup();
    const onImported = vi.fn();
    choose.mockResolvedValue("/ruta/paquete");
    mocked.previewImport.mockResolvedValue(preview());
    mocked.applyImport.mockResolvedValue({
      projectId: "p1",
      created: 1,
      updated: 0,
      unchanged: 1,
      keptLocal: 0,
    });
    render(<ImportSection onImported={onImported} />);
    await user.click(screen.getByRole("button", { name: "Elegir paquete…" }));

    expect(
      await screen.findByText("Actualizar «Planificación»"),
    ).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(
      "Vista previa lista. Se crea: 1 · Sin cambios: 1",
    );
    expect(
      screen.getByText("#1 «Login» se importará como #4."),
    ).toBeInTheDocument();
    expect(mocked.applyImport).not.toHaveBeenCalled();

    await user.click(
      screen.getByRole("button", { name: "Aplicar importación" }),
    );
    expect(mocked.applyImport).toHaveBeenCalledWith({
      path: "/ruta/paquete",
      fingerprint: "huella",
      resolutions: [],
    });
    await waitFor(() => {
      expect(onImported).toHaveBeenCalledWith(
        "p1",
        "Importación aplicada: 1 creadas, 0 actualizadas, 1 sin cambios, 0 conservadas.",
      );
    });
  });

  it("exige resolver cada conflicto antes de aplicar y envía la elección", async () => {
    const user = userEvent.setup();
    choose.mockResolvedValue("/ruta/paquete");
    mocked.previewImport.mockResolvedValue(
      preview({
        items: [
          {
            id: "s1",
            kind: "story",
            label: "#1 Login paquete",
            action: "conflict",
            localLabel: "#1 Login local",
          },
        ],
        warnings: [],
      }),
    );
    mocked.applyImport.mockResolvedValue({
      projectId: "p1",
      created: 0,
      updated: 1,
      unchanged: 0,
      keptLocal: 0,
    });
    render(<ImportSection onImported={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Elegir paquete…" }));

    const apply = await screen.findByRole("button", {
      name: "Aplicar importación",
    });
    expect(apply).toBeDisabled();
    expect(
      screen.getByText(
        "Elige una opción en cada conflicto para poder aplicar.",
      ),
    ).toBeInTheDocument();
    const group = screen.getByRole("group", {
      name: /Conflicto en historia #1 Login paquete/,
    });
    expect(
      within(group).getByLabelText(
        /Conservar la versión local \(#1 Login local\)/,
      ),
    ).toBeInTheDocument();
    await user.click(within(group).getByLabelText(/Usar la del paquete/));
    expect(apply).toBeEnabled();
    await user.click(apply);
    expect(mocked.applyImport).toHaveBeenCalledWith(
      expect.objectContaining({
        resolutions: [{ id: "s1", choice: "package" }],
      }),
    );
  });

  it("muestra el error de validación del paquete sin ofrecer aplicar", async () => {
    const user = userEvent.setup();
    choose.mockResolvedValue("/ruta/mala");
    mocked.previewImport.mockRejectedValue({
      code: "validation",
      message: "Ruta no permitida en el paquete: «../x.md».",
      recoverable: true,
    });
    render(<ImportSection onImported={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Elegir paquete…" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "No se puede importar: Ruta no permitida en el paquete",
    );
    expect(
      screen.queryByRole("button", { name: "Aplicar importación" }),
    ).not.toBeInTheDocument();
  });

  it("si los datos cambiaron, permite volver a previsualizar", async () => {
    const user = userEvent.setup();
    choose.mockResolvedValue("/ruta/paquete");
    mocked.previewImport.mockResolvedValue(preview());
    mocked.applyImport.mockRejectedValue({
      code: "validation",
      message: "El paquete o los datos cambiaron desde la vista previa.",
      recoverable: true,
    });
    render(<ImportSection onImported={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Elegir paquete…" }));
    await user.click(
      await screen.findByRole("button", { name: "Aplicar importación" }),
    );
    await user.click(
      await screen.findByRole("button", { name: "Volver a previsualizar" }),
    );
    expect(mocked.previewImport).toHaveBeenCalledTimes(2);
  });

  it("cancelar el selector no hace nada", async () => {
    const user = userEvent.setup();
    choose.mockResolvedValue(null);
    render(<ImportSection onImported={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Elegir paquete…" }));
    expect(mocked.previewImport).not.toHaveBeenCalled();
  });
});

describe("ExportSection", () => {
  it("exporta a la carpeta elegida y confirma con texto", async () => {
    const user = userEvent.setup();
    choose.mockResolvedValue("/Users/yo/Exportes");
    mocked.exportProject.mockResolvedValue({
      folderName: "kanbania-planificacion-20261110-120000",
      stories: 3,
      epics: 1,
    });
    render(<ExportSection projectId="p1" />);
    await user.click(
      screen.getByRole("button", { name: "Exportar a una carpeta…" }),
    );
    expect(mocked.exportProject).toHaveBeenCalledWith({
      projectId: "p1",
      destination: "/Users/yo/Exportes",
    });
    expect(await screen.findByRole("status")).toHaveTextContent(
      "✓ Exportado en «kanbania-planificacion-20261110-120000» (3 historias, 1 épicas)",
    );
  });

  it("muestra el error si no se puede escribir", async () => {
    const user = userEvent.setup();
    choose.mockResolvedValue("/solo-lectura");
    mocked.exportProject.mockRejectedValue({
      code: "validation",
      message: "Elige una carpeta existente en la que se pueda escribir.",
      recoverable: true,
    });
    render(<ExportSection projectId="p1" />);
    await user.click(
      screen.getByRole("button", { name: "Exportar a una carpeta…" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Elige una carpeta existente",
    );
  });
});
