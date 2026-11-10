// Catálogo de piezas del avatar: sprites originales de 16×16 dibujados por código, con una
// paleta limitada. El orden de cada lista es el índice que guarda el núcleo (AvatarSpec);
// las longitudes deben coincidir con contracts/generated/avatarLimits.json (hay prueba).
import type { AvatarSpec } from "../../contracts/generated/AvatarSpec";

export interface Swatch {
  name: string;
  color: string;
}

export const SKINS: Swatch[] = [
  { name: "Piel muy clara", color: "#f6d5b8" },
  { name: "Piel clara", color: "#e8b48a" },
  { name: "Piel media", color: "#c98e62" },
  { name: "Piel tostada", color: "#a0663f" },
  { name: "Piel oscura", color: "#6e4428" },
  { name: "Piel muy oscura", color: "#4a2c1a" },
  { name: "Acero", color: "#a9b4c9" },
  { name: "Cobre", color: "#d08a4e" },
];

export const HAIR_COLORS: Swatch[] = [
  { name: "Negro", color: "#1d1b2c" },
  { name: "Castaño", color: "#5a3a22" },
  { name: "Rubio", color: "#e6c25a" },
  { name: "Pelirrojo", color: "#c8582e" },
  { name: "Gris", color: "#9a9ab0" },
  { name: "Blanco", color: "#f2f2f2" },
  { name: "Azul", color: "#5aa0ff" },
  { name: "Rosa", color: "#ff7eb6" },
  { name: "Verde", color: "#5ad18a" },
  { name: "Morado", color: "#9a6bff" },
];

export const BACKGROUNDS: Swatch[] = [
  { name: "Noche", color: "#2a2a52" },
  { name: "Azul", color: "#3b5b8f" },
  { name: "Verde", color: "#2f6f5e" },
  { name: "Morado", color: "#6b3f7a" },
  { name: "Teja", color: "#8f4a3b" },
  { name: "Mostaza", color: "#8a7a2f" },
  { name: "Petróleo", color: "#1f4f6f" },
  { name: "Pizarra", color: "#5a5a6e" },
  { name: "Vino", color: "#7a2f4f" },
  { name: "Musgo", color: "#3f6f2f" },
];

export const HEADS = ["Redonda", "Cuadrada", "Robot", "Robot con cúpula"];
export const EYES = [
  "Puntos",
  "Grandes",
  "Alegres",
  "Entornados",
  "Visor",
  "Pantalla",
];
export const MOUTHS = ["Sonrisa", "Neutra", "Abierta", "Rejilla", "Sorpresa"];
export const HAIRS = [
  "Sin pelo",
  "Corto",
  "Largo",
  "Rizado",
  "Cresta",
  "Gorra",
  "Antena",
  "Casco",
];
export const ACCESSORIES = [
  "Ninguno",
  "Gafas",
  "Auriculares",
  "Bufanda",
  "Pendiente",
  "Insignia",
];

/** Número de opciones por pieza, para comparar con los límites del núcleo. */
export const CATALOG_SIZES = {
  head: HEADS.length,
  skin: SKINS.length,
  eyes: EYES.length,
  mouth: MOUTHS.length,
  hair: HAIRS.length,
  hairColor: HAIR_COLORS.length,
  accessory: ACCESSORIES.length,
  background: BACKGROUNDS.length,
} satisfies Record<keyof AvatarSpec, number>;

const INK = "#14142b";
const WHITE = "#f4f4ff";
const ACCENT = "#7ee0a1";
const VISOR = "#5ad6ff";
const GOLD = "#ffd166";
const SCARF = "#e05a5a";
const TONGUE = "#e8728a";
const GRILL = "#5a5a6e";

type Grid = (string | null)[][];

const SIZE = 16;

function rect(
  grid: Grid,
  x0: number,
  y0: number,
  x1: number,
  y1: number,
  color: string,
) {
  for (let y = Math.max(0, y0); y <= Math.min(SIZE - 1, y1); y++) {
    const row = grid[y];
    if (!row) continue;
    for (let x = Math.max(0, x0); x <= Math.min(SIZE - 1, x1); x++)
      row[x] = color;
  }
}

function dot(grid: Grid, x: number, y: number, color: string) {
  rect(grid, x, y, x, y, color);
}

function headMask(head: number): Grid {
  const mask: Grid = Array.from({ length: SIZE }, () =>
    Array<string | null>(SIZE).fill(null),
  );
  switch (head) {
    case 0: // redonda
      rect(mask, 5, 3, 10, 3, "h");
      rect(mask, 4, 4, 11, 10, "h");
      rect(mask, 5, 11, 10, 11, "h");
      break;
    case 1: // cuadrada
      rect(mask, 4, 3, 11, 11, "h");
      break;
    case 2: // robot
      rect(mask, 3, 3, 12, 11, "h");
      break;
    default: // robot con cúpula
      rect(mask, 6, 3, 9, 3, "h");
      rect(mask, 5, 4, 10, 4, "h");
      rect(mask, 4, 5, 11, 11, "h");
  }
  return mask;
}

/** Píxeles del avatar: matriz 16×16 de colores (fila por fila). */
export function renderAvatar(spec: AvatarSpec): Grid {
  // Índices fuera de rango (no deberían llegar: el núcleo valida) caen en un color neutro.
  const bg = BACKGROUNDS[spec.background]?.color ?? "#2a2a52";
  const skin = SKINS[spec.skin]?.color ?? "#e8b48a";
  const hairColor = HAIR_COLORS[spec.hairColor]?.color ?? "#1d1b2c";
  const robot = spec.head >= 2;
  const grid: Grid = Array.from({ length: SIZE }, () =>
    Array<string | null>(SIZE).fill(bg),
  );

  // Hombros.
  rect(grid, 2, 13, 13, 15, robot ? "#7a8499" : "#4b4b8f");
  rect(grid, 6, 12, 9, 12, skin);

  // Cabeza con contorno.
  const mask = headMask(spec.head);
  for (let y = 0; y < SIZE; y++) {
    for (let x = 0; x < SIZE; x++) {
      if (mask[y]?.[x]) {
        dot(grid, x, y, skin);
      } else {
        const neighbour = [
          [x - 1, y],
          [x + 1, y],
          [x, y - 1],
          [x, y + 1],
        ].some(([nx, ny]) => mask[ny ?? -1]?.[nx ?? -1]);
        if (neighbour) dot(grid, x, y, INK);
      }
    }
  }
  if (robot) {
    // Remaches.
    dot(grid, spec.head === 2 ? 4 : 5, 10, GRILL);
    dot(grid, spec.head === 2 ? 11 : 10, 10, GRILL);
  }

  // Pelo o tocado.
  switch (spec.hair) {
    case 1: // corto
      rect(grid, 4, 2, 11, 3, hairColor);
      dot(grid, 4, 4, hairColor);
      dot(grid, 11, 4, hairColor);
      break;
    case 2: // largo
      rect(grid, 4, 2, 11, 3, hairColor);
      rect(grid, 3, 4, 4, 12, hairColor);
      rect(grid, 11, 4, 12, 12, hairColor);
      break;
    case 3: // rizado
      for (let x = 3; x <= 12; x += 2) rect(grid, x, 1, x, 3, hairColor);
      rect(grid, 4, 2, 11, 3, hairColor);
      dot(grid, 3, 5, hairColor);
      dot(grid, 12, 5, hairColor);
      break;
    case 4: // cresta
      rect(grid, 7, 0, 8, 3, hairColor);
      break;
    case 5: // gorra
      rect(grid, 4, 1, 11, 3, hairColor);
      rect(grid, 9, 4, 13, 4, hairColor);
      dot(grid, 7, 1, WHITE);
      break;
    case 6: // antena
      rect(grid, 7, 1, 8, 2, INK);
      rect(grid, 6, 0, 9, 0, hairColor);
      break;
    case 7: // casco
      rect(grid, 4, 1, 11, 4, hairColor);
      rect(grid, 3, 3, 12, 4, hairColor);
      dot(grid, 5, 2, WHITE);
      break;
    default:
      break;
  }

  // Ojos.
  switch (spec.eyes) {
    case 0:
      dot(grid, 6, 7, INK);
      dot(grid, 9, 7, INK);
      break;
    case 1:
      rect(grid, 5, 6, 6, 7, WHITE);
      rect(grid, 9, 6, 10, 7, WHITE);
      dot(grid, 6, 7, INK);
      dot(grid, 9, 7, INK);
      break;
    case 2:
      dot(grid, 5, 7, INK);
      dot(grid, 6, 6, INK);
      dot(grid, 7, 7, INK);
      dot(grid, 8, 7, INK);
      dot(grid, 9, 6, INK);
      dot(grid, 10, 7, INK);
      break;
    case 3:
      rect(grid, 5, 7, 6, 7, INK);
      rect(grid, 9, 7, 10, 7, INK);
      break;
    case 4:
      rect(grid, 4, 6, 11, 7, VISOR);
      rect(grid, 5, 6, 6, 6, WHITE);
      break;
    default:
      rect(grid, 5, 6, 6, 7, INK);
      rect(grid, 9, 6, 10, 7, INK);
      dot(grid, 5, 6, ACCENT);
      dot(grid, 9, 6, ACCENT);
  }

  // Boca.
  switch (spec.mouth) {
    case 0:
      dot(grid, 6, 9, INK);
      dot(grid, 9, 9, INK);
      rect(grid, 7, 10, 8, 10, INK);
      break;
    case 1:
      rect(grid, 6, 10, 9, 10, INK);
      break;
    case 2:
      rect(grid, 6, 9, 9, 9, INK);
      rect(grid, 6, 10, 9, 10, TONGUE);
      break;
    case 3:
      rect(grid, 5, 9, 10, 10, GRILL);
      for (let x = 6; x <= 9; x += 2) rect(grid, x, 9, x, 10, INK);
      break;
    default:
      rect(grid, 7, 9, 8, 10, INK);
  }

  // Accesorio.
  switch (spec.accessory) {
    case 1: // gafas
      for (const x0 of [4, 8]) {
        rect(grid, x0, 5, x0 + 3, 5, INK);
        rect(grid, x0, 8, x0 + 3, 8, INK);
        rect(grid, x0, 6, x0, 7, INK);
        rect(grid, x0 + 3, 6, x0 + 3, 7, INK);
      }
      break;
    case 2: // auriculares
      rect(grid, 3, 1, 12, 1, INK);
      rect(grid, 2, 2, 2, 5, INK);
      rect(grid, 13, 2, 13, 5, INK);
      rect(grid, 1, 6, 2, 9, ACCENT);
      rect(grid, 13, 6, 14, 9, ACCENT);
      break;
    case 3: // bufanda
      rect(grid, 3, 12, 12, 13, SCARF);
      rect(grid, 10, 14, 11, 15, SCARF);
      break;
    case 4: // pendiente
      dot(grid, 12, 10, GOLD);
      break;
    case 5: // insignia
      rect(grid, 10, 14, 11, 15, GOLD);
      break;
    default:
      break;
  }

  return grid;
}

/** Descripción textual para lectores de pantalla. */
export function describeAvatar(spec: AvatarSpec): string {
  const parts = [
    `cabeza ${HEADS[spec.head]?.toLowerCase() ?? ""}`,
    `ojos ${EYES[spec.eyes]?.toLowerCase() ?? ""}`,
  ];
  if (spec.hair > 0) parts.push(HAIRS[spec.hair]?.toLowerCase() ?? "");
  if (spec.accessory > 0)
    parts.push(ACCESSORIES[spec.accessory]?.toLowerCase() ?? "");
  return parts.join(", ");
}
