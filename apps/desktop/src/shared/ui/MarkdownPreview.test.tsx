import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { MarkdownPreview } from "./MarkdownPreview";

function renderMarkdown(source: string) {
  const { container } = render(<MarkdownPreview source={source} />);
  return container;
}

describe("MarkdownPreview", () => {
  it("renderiza Markdown habitual", () => {
    const container = renderMarkdown(
      "## Contexto\n\nTexto **importante**.\n\n- uno\n- dos\n\n| a | b |\n|---|---|\n| 1 | 2 |",
    );
    expect(
      screen.getByRole("heading", { level: 2, name: "Contexto" }),
    ).toBeInTheDocument();
    expect(container.querySelector("strong")).toHaveTextContent("importante");
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
    expect(screen.getByRole("table")).toBeInTheDocument();
  });

  it("no crea elementos ni atributos a partir de HTML crudo", () => {
    const container = renderMarkdown(
      [
        "<script>window.__pwned = true</script>",
        '<img src="x" onerror="window.__pwned = true">',
        '<iframe src="https://example.com"></iframe>',
        '<div onclick="alert(1)">clic</div>',
        "<style>body{display:none}</style>",
      ].join("\n\n"),
    );
    for (const tag of ["script", "img", "iframe", "style", ".markdown div"]) {
      expect(container.querySelector(tag)).toBeNull();
    }
    expect(container.querySelector("[onerror], [onclick]")).toBeNull();
    expect(
      (window as unknown as { __pwned?: boolean }).__pwned,
    ).toBeUndefined();
  });

  it("no genera enlaces navegables, tampoco javascript:", () => {
    const container = renderMarkdown(
      "[seguro](https://example.com) y [malo](javascript:alert(1)) y <https://auto.link>",
    );
    expect(container.querySelector("a")).toBeNull();
    expect(screen.getByText("seguro")).toBeInTheDocument();
    expect(container.innerHTML).not.toContain("javascript:");
  });

  it("no carga imágenes remotas", () => {
    const container = renderMarkdown(
      "![diagrama](https://tracker.example/pixel.png)",
    );
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByText("[imagen: diagrama]")).toBeInTheDocument();
  });

  it("muestra un estado vacío", () => {
    renderMarkdown("   ");
    expect(screen.getByText("Sin descripción.")).toBeInTheDocument();
  });
});
