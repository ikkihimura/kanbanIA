const dateTime = new Intl.DateTimeFormat("es", {
  dateStyle: "medium",
  timeStyle: "short",
});

/** Muestra un instante UTC (ISO 8601) en la zona local. */
export function formatDateTime(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? iso : dateTime.format(date);
}

const time = new Intl.DateTimeFormat("es", {
  hour: "2-digit",
  minute: "2-digit",
});

/** Solo la hora local (HH:MM) de un instante UTC (ISO 8601). */
export function formatTime(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? iso : time.format(date);
}
