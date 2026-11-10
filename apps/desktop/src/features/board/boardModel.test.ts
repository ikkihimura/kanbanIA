import { describe, expect, it } from "vitest";
import type { StorySummary } from "../../contracts/generated/StorySummary";
import {
  applyMove,
  cardsByColumn,
  dropIndex,
  keyboardTarget,
  matchesFilter,
} from "./boardModel";
import { card, column } from "./boardFixtures";

const order = (stories: StorySummary[], columnId: string) =>
  stories
    .filter((s) => s.columnId === columnId && s.position !== null)
    .sort((a, b) => (a.position ?? 0) - (b.position ?? 0))
    .map((s) => `${s.id}@${String(s.position)}`);

describe("applyMove", () => {
  const stories = [
    card("s1", "a", 0),
    card("s2", "a", 1),
    card("s3", "a", 2),
    card("s4", "b", 0),
    card("s9", "a", null),
  ];

  it("reordena dentro de la columna con posiciones densas", () => {
    const next = applyMove(stories, "s1", "a", 2);
    expect(order(next, "a")).toEqual(["s2@0", "s3@1", "s1@2"]);
  });

  it("mueve entre columnas y compacta el origen", () => {
    const next = applyMove(stories, "s2", "b", 0);
    expect(order(next, "a")).toEqual(["s1@0", "s3@1"]);
    expect(order(next, "b")).toEqual(["s2@0", "s4@1"]);
  });

  it("acota el índice al final y no toca las archivadas", () => {
    const next = applyMove(stories, "s1", "b", 99);
    expect(order(next, "b")).toEqual(["s4@0", "s1@1"]);
    expect(next.find((s) => s.id === "s9")?.position).toBeNull();
    expect(applyMove(stories, "s9", "b", 0)).toBe(stories);
  });
});

describe("dropIndex", () => {
  const cells = [card("s1", "a", 0), card("s2", "a", 1), card("s3", "a", 2)];

  it("suelta antes de la tarjeta destino, sin contar la arrastrada", () => {
    expect(dropIndex(cells, "s1", { beforeStoryId: "s3" })).toBe(1);
    expect(dropIndex(cells, "s3", { beforeStoryId: "s1" })).toBe(0);
  });

  it("en el hueco: tras la última del carril o al final", () => {
    expect(
      dropIndex(cells, "x", { beforeStoryId: null, afterStoryId: "s1" }),
    ).toBe(1);
    expect(dropIndex(cells, "x", { beforeStoryId: null })).toBe(3);
  });
});

describe("keyboardTarget", () => {
  const columns = [column("a", 0), column("b", 1)];
  const stories = [
    card("s1", "a", 0),
    card("s2", "a", 1),
    card("s3", "a", 2),
    card("s4", "b", 0),
  ];
  const byColumn = cardsByColumn(columns, stories);
  const all = byColumn.get("a") ?? [];

  it("arriba/abajo intercambia con la vecina visible", () => {
    const [s1, s2, s3] = all;
    if (!s1 || !s2 || !s3) throw new Error("datos");
    expect(keyboardTarget(s2, columns, byColumn, all, "up")).toEqual({
      columnId: "a",
      index: 0,
    });
    // Con s2 oculta por un filtro, bajar s1 salta hasta s3.
    expect(keyboardTarget(s1, columns, byColumn, [s1, s3], "down")).toEqual({
      columnId: "a",
      index: 2,
    });
    expect(keyboardTarget(s1, columns, byColumn, all, "up")).toBeNull();
  });

  it("izquierda/derecha va al final de la columna contigua", () => {
    const s1 = all[0];
    if (!s1) throw new Error("datos");
    expect(keyboardTarget(s1, columns, byColumn, all, "right")).toEqual({
      columnId: "b",
      index: 1,
    });
    expect(keyboardTarget(s1, columns, byColumn, all, "left")).toBeNull();
  });
});

describe("matchesFilter", () => {
  const story = card("s7", "a", 0, {
    title: "Pantalla de login",
    epicId: "e1",
  });

  it("filtra por texto, por #número y por épica", () => {
    expect(matchesFilter(story, { text: "LOGIN", epicId: "" })).toBe(true);
    expect(matchesFilter(story, { text: "#7", epicId: "" })).toBe(true);
    expect(matchesFilter(story, { text: "#8", epicId: "" })).toBe(false);
    expect(matchesFilter(story, { text: "", epicId: "e1" })).toBe(true);
    expect(matchesFilter(story, { text: "", epicId: "none" })).toBe(false);
  });
});
