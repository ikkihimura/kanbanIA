import { useMemo } from "react";
import type { AvatarSpec } from "../../contracts/generated/AvatarSpec";
import { describeAvatar, renderAvatar } from "./avatarCatalog";

interface Props {
  spec: AvatarSpec;
  /** Nombre de la persona o agente; forma parte de la etiqueta accesible. */
  name: string;
  /** Lado en píxeles (múltiplo de 16 para píxeles nítidos). */
  size?: number;
  /** Decorativo cuando el nombre ya aparece al lado como texto. */
  decorative?: boolean;
}

/** Avatar pixelado en SVG: un rectángulo por tramo horizontal de color, sin imágenes externas. */
export function PixelAvatar({
  spec,
  name,
  size = 48,
  decorative = false,
}: Props) {
  const runs = useMemo(() => {
    const out: { x: number; y: number; w: number; color: string }[] = [];
    renderAvatar(spec).forEach((row, y) => {
      let x = 0;
      while (x < row.length) {
        const color = row[x];
        let end = x;
        while (end + 1 < row.length && row[end + 1] === color) end++;
        if (color) out.push({ x, y, w: end - x + 1, color });
        x = end + 1;
      }
    });
    return out;
  }, [spec]);

  return (
    <svg
      className="pixel-avatar"
      width={size}
      height={size}
      viewBox="0 0 16 16"
      shapeRendering="crispEdges"
      {...(decorative
        ? { "aria-hidden": true }
        : {
            role: "img",
            "aria-label": `Avatar de ${name}: ${describeAvatar(spec)}`,
          })}
    >
      {runs.map((run) => (
        <rect
          key={`${String(run.x)}-${String(run.y)}`}
          x={run.x}
          y={run.y}
          width={run.w}
          height={1}
          fill={run.color}
        />
      ))}
    </svg>
  );
}
