import { createEvent, fireEvent, render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useBlockExternalDrops } from "./useBlockExternalDrops";

function Host() {
  useBlockExternalDrops();
  return (
    <div>
      <p data-testid="zone">Zona</p>
      <textarea data-testid="text" />
    </div>
  );
}

describe("useBlockExternalDrops", () => {
  it("anula soltar fuera de campos de texto y lo permite en ellos", () => {
    const { getByTestId } = render(<Host />);
    const outside = createEvent.drop(getByTestId("zone"));
    fireEvent(getByTestId("zone"), outside);
    expect(outside.defaultPrevented).toBe(true);
    const over = createEvent.dragOver(getByTestId("zone"));
    fireEvent(getByTestId("zone"), over);
    expect(over.defaultPrevented).toBe(true);

    const inside = createEvent.drop(getByTestId("text"));
    fireEvent(getByTestId("text"), inside);
    expect(inside.defaultPrevented).toBe(false);
  });
});
