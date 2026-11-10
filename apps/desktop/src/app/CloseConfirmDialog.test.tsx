import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as commands from "../shared/ipc/commands";
import { CloseConfirmDialog } from "./CloseConfirmDialog";

vi.mock("../shared/ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  onConfirmClose: vi.fn(),
  discardAndClose: vi.fn(),
}));

const mocked = vi.mocked(commands);
let requestClose: () => void = () => undefined;
const unlisten = vi.fn();

beforeEach(() => {
  mocked.onConfirmClose.mockImplementation((handler) => {
    requestClose = handler;
    return Promise.resolve(unlisten);
  });
  mocked.discardAndClose.mockResolvedValue(undefined);
});

async function openDialog() {
  render(
    <>
      <input aria-label="Campo en edición" />
      <CloseConfirmDialog />
    </>,
  );
  await vi.waitFor(() => {
    expect(mocked.onConfirmClose).toHaveBeenCalled();
  });
  screen.getByLabelText("Campo en edición").focus();
  act(() => {
    requestClose();
  });
}

describe("CloseConfirmDialog", () => {
  it("no muestra nada hasta que el host pide confirmar", () => {
    render(<CloseConfirmDialog />);
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("pregunta, enfoca «Seguir editando» y al seguir vuelve al campo", async () => {
    const user = userEvent.setup();
    await openDialog();

    const dialog = screen.getByRole("alertdialog", {
      name: "Cambios sin guardar",
    });
    expect(dialog).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Seguir editando" }),
    ).toHaveFocus();

    await user.click(screen.getByRole("button", { name: "Seguir editando" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(screen.getByLabelText("Campo en edición")).toHaveFocus();
    expect(mocked.discardAndClose).not.toHaveBeenCalled();
  });

  it("Escape equivale a seguir editando y Tab no sale del diálogo", async () => {
    const user = userEvent.setup();
    await openDialog();

    await user.tab();
    expect(
      screen.getByRole("button", { name: "Salir sin guardar" }),
    ).toHaveFocus();
    await user.tab();
    expect(
      screen.getByRole("button", { name: "Seguir editando" }),
    ).toHaveFocus();

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("«Salir sin guardar» confirma el cierre al host", async () => {
    const user = userEvent.setup();
    await openDialog();

    await user.click(screen.getByRole("button", { name: "Salir sin guardar" }));
    expect(mocked.discardAndClose).toHaveBeenCalledTimes(1);
  });

  it("deja de escuchar al desmontar", async () => {
    const { unmount } = render(<CloseConfirmDialog />);
    await vi.waitFor(() => {
      expect(mocked.onConfirmClose).toHaveBeenCalled();
    });
    unmount();
    expect(unlisten).toHaveBeenCalled();
  });
});
