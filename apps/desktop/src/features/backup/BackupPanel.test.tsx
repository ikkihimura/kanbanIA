import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { BackupInfo } from "../../contracts/generated/BackupInfo";
import * as commands from "../../shared/ipc/commands";
import { chooseFolder } from "../../shared/ui/chooseFolder";
import { BackupPanel } from "./BackupPanel";

vi.mock("../../shared/ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  listBackups: vi.fn(),
  createBackup: vi.fn(),
  saveBackupToFolder: vi.fn(),
  restoreBackup: vi.fn(),
}));
vi.mock("../../shared/ui/chooseFolder", () => ({ chooseFolder: vi.fn() }));

const mocked = vi.mocked(commands);
const choose = vi.mocked(chooseFolder);

function backup(overrides: Partial<BackupInfo> = {}): BackupInfo {
  return {
    id: "20261110-120000-000-manual",
    kind: "manual",
    createdAt: "2026-11-10T12:00:00.000Z",
    appVersion: "0.1.0",
    schemaVersion: 7,
    bytes: 2048,
    excludes: [],
    ...overrides,
  };
}

function firstButton(name: string): HTMLElement {
  const [first] = screen.getAllByRole("button", { name });
  if (!first) throw new Error(`sin botón ${name}`);
  return first;
}

beforeEach(() => {
  vi.clearAllMocks();
  mocked.listBackups.mockResolvedValue([
    backup(),
    backup({
      id: "20261109-080000-000-pre-migration",
      kind: "pre_migration",
      createdAt: "2026-11-09T08:00:00.000Z",
      schemaVersion: 6,
      bytes: 3 * 1024 * 1024,
    }),
  ]);
});

describe("BackupPanel", () => {
  it("lista las copias y dice qué no incluye", async () => {
    render(
      <BackupPanel canCreate onRestoring={vi.fn()} onFinished={vi.fn()} />,
    );
    expect(
      await screen.findByText(/Manual · esquema v7 · 2 KB/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        /Automática: antes de actualizar la base · esquema v6 · 3\.0 MB/,
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "La exportación Markdown no es una copia de seguridad: solo lleva la planificación.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        /La salida de los procesos y los worktrees de los intentos/,
      ),
    ).toBeInTheDocument();
  });

  it("crea una copia y recarga la lista", async () => {
    const user = userEvent.setup();
    mocked.createBackup.mockResolvedValue(
      backup({ createdAt: "2026-11-10T13:00:00.000Z" }),
    );
    render(
      <BackupPanel canCreate onRestoring={vi.fn()} onFinished={vi.fn()} />,
    );
    await screen.findByText(/Manual · esquema v7/);
    await user.click(screen.getByRole("button", { name: "Crear copia ahora" }));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "✓ Copia creada",
    );
    expect(mocked.listBackups).toHaveBeenCalledTimes(2);
  });

  it("sin almacenamiento disponible no se crean copias (y se explica), pero se puede restaurar", async () => {
    render(
      <BackupPanel
        canCreate={false}
        onRestoring={vi.fn()}
        onFinished={vi.fn()}
      />,
    );
    await screen.findByText(/Manual · esquema v7/);
    const create = screen.getByRole("button", { name: "Crear copia ahora" });
    expect(create).toBeDisabled();
    expect(create).toHaveAccessibleDescription(
      /el almacenamiento debe estar disponible/,
    );
    expect(firstButton("Restaurar…")).toBeEnabled();
  });
  it("restaurar pide confirmación accesible; cancelar o Escape no hacen nada", async () => {
    const user = userEvent.setup();
    const onFinished = vi.fn();
    render(
      <BackupPanel canCreate onRestoring={vi.fn()} onFinished={onFinished} />,
    );
    await screen.findByText(/Manual · esquema v7/);
    const trigger = firstButton("Restaurar…");
    await user.click(trigger);
    const confirm = screen.getByRole("alertdialog", {
      name: /Se reemplazarán todos los datos actuales/,
    });
    // Avisa de que restaurar detiene los intentos en marcha.
    expect(confirm).toHaveTextContent(
      "Los procesos de los intentos en marcha se detienen, y en los datos restaurados todo intento activo (también los que estaban en cola) queda como «Interrumpido».",
    );
    // El foco entra en la confirmación (en la opción segura) y vuelve al cerrarla.
    const cancel = within(confirm).getByRole("button", { name: "Cancelar" });
    expect(cancel).toHaveFocus();
    await user.click(cancel);
    expect(trigger).toHaveFocus();

    await user.click(trigger);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
    expect(mocked.restoreBackup).not.toHaveBeenCalled();
    expect(onFinished).not.toHaveBeenCalled();
  });
  it("restaura, avisa de la copia previa para deshacer y recarga la app", async () => {
    const user = userEvent.setup();
    const onFinished = vi.fn();
    const onRestoring = vi.fn();
    mocked.restoreBackup.mockResolvedValue({
      safetyBackup: backup({
        id: "x-pre-restore",
        kind: "pre_restore",
        createdAt: "2026-11-10T14:00:00.000Z",
      }),
      restoredSchemaVersion: 7,
    });
    render(
      <BackupPanel
        canCreate
        onRestoring={onRestoring}
        onFinished={onFinished}
      />,
    );
    await screen.findByText(/Manual · esquema v7/);
    await user.click(firstButton("Restaurar…"));
    await user.click(screen.getByRole("button", { name: "Restaurar" }));
    expect(onRestoring).toHaveBeenCalledWith(true);
    expect(mocked.restoreBackup).toHaveBeenCalledWith({
      kind: "backup",
      id: "20261110-120000-000-manual",
    });
    await waitFor(() => {
      expect(onFinished).toHaveBeenCalledWith(
        expect.stringContaining("Los anteriores quedaron en la copia del"),
      );
    });
  });
  it("si la copia está dañada lo dice y la app solo refresca el estado", async () => {
    const user = userEvent.setup();
    const onFinished = vi.fn();
    choose.mockResolvedValue("/Volumes/Externo/kanbania-backup-x");
    mocked.restoreBackup.mockRejectedValue({
      code: "validation",
      message:
        "La copia está dañada: su contenido no coincide con el manifiesto.",
      recoverable: true,
    });
    render(
      <BackupPanel canCreate onRestoring={vi.fn()} onFinished={onFinished} />,
    );
    await screen.findByText(/Manual · esquema v7/);
    await user.click(
      screen.getByRole("button", { name: "Restaurar desde una carpeta…" }),
    );
    expect(
      screen.getByText(/la carpeta \/Volumes\/Externo\/kanbania-backup-x/),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Restaurar" }));
    expect(mocked.restoreBackup).toHaveBeenCalledWith({
      kind: "folder",
      path: "/Volumes/Externo/kanbania-backup-x",
    });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "No se restauró: La copia está dañada",
    );
    await waitFor(() => {
      expect(onFinished).toHaveBeenCalledWith(null);
    });
  });

  it("guarda una copia en otra carpeta", async () => {
    const user = userEvent.setup();
    choose.mockResolvedValue("/Volumes/Externo");
    mocked.saveBackupToFolder.mockResolvedValue(
      "kanbania-backup-20261110-120000-000-manual",
    );
    render(
      <BackupPanel canCreate onRestoring={vi.fn()} onFinished={vi.fn()} />,
    );
    await screen.findByText(/Manual · esquema v7/);
    await user.click(firstButton("Guardar en carpeta…"));
    expect(mocked.saveBackupToFolder).toHaveBeenCalledWith({
      id: "20261110-120000-000-manual",
      destination: "/Volumes/Externo",
    });
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Copia guardada en «kanbania-backup-20261110-120000-000-manual»",
    );
  });
});
