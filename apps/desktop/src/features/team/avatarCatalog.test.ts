import { describe, expect, it } from "vitest";
import limits from "../../contracts/generated/avatarLimits.json";
import {
  BACKGROUNDS,
  CATALOG_SIZES,
  HAIR_COLORS,
  SKINS,
  describeAvatar,
  renderAvatar,
} from "./avatarCatalog";

describe("catálogo de avatar", () => {
  it("tiene exactamente las opciones que valida el núcleo", () => {
    expect(CATALOG_SIZES).toEqual(limits);
  });

  it("dibuja una matriz 16×16 completa para todas las combinaciones de forma", () => {
    for (let head = 0; head < limits.head; head++) {
      for (let hair = 0; hair < limits.hair; hair++) {
        for (let accessory = 0; accessory < limits.accessory; accessory++) {
          const grid = renderAvatar({
            head,
            skin: 0,
            eyes: (head + hair) % limits.eyes,
            mouth: (hair + accessory) % limits.mouth,
            hair,
            hairColor: 0,
            accessory,
            background: 0,
          });
          expect(grid).toHaveLength(16);
          for (const row of grid) {
            expect(row).toHaveLength(16);
            expect(row.every((c) => typeof c === "string")).toBe(true);
          }
        }
      }
    }
  });

  it("usa una paleta limitada con nombres para lectores de pantalla", () => {
    for (const swatch of [...SKINS, ...HAIR_COLORS, ...BACKGROUNDS]) {
      expect(swatch.color).toMatch(/^#[0-9a-f]{6}$/);
      expect(swatch.name.length).toBeGreaterThan(0);
    }
    expect(
      describeAvatar({
        head: 2,
        skin: 6,
        eyes: 4,
        mouth: 3,
        hair: 6,
        hairColor: 7,
        accessory: 5,
        background: 3,
      }),
    ).toBe("cabeza robot, ojos visor, antena, insignia");
  });
});
