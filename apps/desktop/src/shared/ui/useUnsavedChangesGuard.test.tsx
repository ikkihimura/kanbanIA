import { render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as commands from "../ipc/commands";
import { useUnsavedChangesGuard } from "./useUnsavedChangesGuard";

vi.mock("../ipc/commands", async (importOriginal) => ({
  ...(await importOriginal<typeof commands>()),
  setUnsavedChanges: vi.fn(),
}));

const mocked = vi.mocked(commands);

function Editor({ dirty }: { dirty: boolean }) {
  useUnsavedChangesGuard(dirty);
  return null;
}

function Two({ a, b }: { a: boolean; b: boolean }) {
  return (
    <>
      <Editor dirty={a} />
      <Editor dirty={b} />
    </>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mocked.setUnsavedChanges.mockResolvedValue(undefined);
});

describe("useUnsavedChangesGuard", () => {
  it("con varios editores, el host ve la unión: limpiar uno no borra el otro", () => {
    const view = render(<Two a b={false} />);
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(true);

    view.rerender(<Two a b />);
    view.rerender(<Two a b={false} />);
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(true);

    view.rerender(<Two a={false} b={false} />);
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(false);
  });

  it("al desmontar un editor con borrador deja de contar", () => {
    const view = render(<Two a={false} b />);
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(true);
    view.rerender(<Editor dirty={false} />);
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(false);
    view.unmount();
    expect(mocked.setUnsavedChanges).toHaveBeenLastCalledWith(false);
  });
});
