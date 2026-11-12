import { useEffect, useState } from "react";
import type { StoryClaim } from "../../contracts/generated/StoryClaim";

// Límite de `setTimeout` (≈ 24,8 días); por encima se dispararía al momento.
const MAX_DELAY = 2_147_483_647;

/**
 * Reclamos aún no vencidos según el reloj local. La lista se leyó en un momento dado y nadie
 * avisa cuando un reclamo vence: un temporizador al vencimiento más próximo vuelve a filtrar.
 */
export function useUnexpiredClaims(claims: StoryClaim[]): StoryClaim[] {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const next = claims
      .map((c) => Date.parse(c.expiresAt))
      .filter((t) => t > now)
      .reduce((a, b) => Math.min(a, b), Number.POSITIVE_INFINITY);
    if (next === Number.POSITIVE_INFINITY) return;
    // Si ya venció según el reloj real (p. ej. la lista llegó tarde), se recalcula enseguida.
    const delay = Math.min(Math.max(next - Date.now(), 0) + 1, MAX_DELAY);
    const timer = setTimeout(() => {
      setNow(Date.now());
    }, delay);
    return () => {
      clearTimeout(timer);
    };
  }, [claims, now]);

  return claims.filter((c) => Date.parse(c.expiresAt) > now);
}
