// Contraste WCAG 2.x de los tokens de color de global.css (texto normal: ≥ 4.5:1).
/// <reference types="node" />
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

// Vitest se ejecuta desde apps/desktop. (Vite no procesa CSS en las pruebas: se lee el archivo.)
const css: string = readFileSync(
  resolve(process.cwd(), "src/app/global.css"),
  "utf8",
);

const root = css.slice(
  css.indexOf(":root {"),
  css.indexOf("}", css.indexOf(":root {")),
);
const tokens = new Map<string, string>();
for (const match of root.matchAll(/--(color-[\w-]+):\s*(#[0-9a-f]{6})/g)) {
  const [, name, value] = match;
  if (name && value) tokens.set(name, value);
}

function luminance(hex: string): number {
  const channels = [1, 3, 5].map(
    (i) => parseInt(hex.slice(i, i + 2), 16) / 255,
  );
  const [r = 0, g = 0, b = 0] = channels.map((c) =>
    c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4,
  );
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function ratio(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return ((hi ?? 0) + 0.05) / ((lo ?? 0) + 0.05);
}

const token = (name: string) => {
  const value = tokens.get(name);
  if (!value) throw new Error(`Falta el token ${name}`);
  return value;
};

describe("contraste de colores", () => {
  const backgrounds = ["color-bg", "color-surface", "color-surface-raised"];
  const texts = [
    "color-text",
    "color-muted",
    "color-accent",
    "color-warn",
    "color-danger",
  ];

  for (const bg of backgrounds) {
    for (const fg of texts) {
      it(`${fg} sobre ${bg} ≥ 4.5:1`, () => {
        expect(ratio(token(fg), token(bg))).toBeGreaterThanOrEqual(4.5);
      });
    }
  }

  it("texto de botones (fondo acento) ≥ 4.5:1", () => {
    expect(
      ratio(token("color-bg"), token("color-accent")),
    ).toBeGreaterThanOrEqual(4.5);
    expect(
      ratio(token("color-bg"), token("color-danger")),
    ).toBeGreaterThanOrEqual(4.5);
  });

  it("foco visible ≥ 3:1 sobre los fondos", () => {
    for (const bg of backgrounds) {
      expect(ratio(token("color-focus"), token(bg))).toBeGreaterThanOrEqual(3);
    }
  });
});
