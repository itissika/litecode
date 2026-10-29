/**
 * Compact modification stamp for dense file lists. Today shows the clock time,
 * this year shows month and day, older files add the year. `now` and `locale`
 * are arguments so the panel and tests pin both.
 */
export function formatFileTime(
  mtime: number,
  now: number,
  locale: string,
): string {
  const date = toDate(mtime);
  if (!date) return "—";
  const current = new Date(now);
  if (
    date.getFullYear() === current.getFullYear() &&
    date.getMonth() === current.getMonth() &&
    date.getDate() === current.getDate()
  ) {
    return date.toLocaleTimeString(locale, {
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    });
  }
  // Numeric month/day keeps the column narrow in every locale; a short month
  // name would overflow it in Chinese.
  const options: Intl.DateTimeFormatOptions =
    date.getFullYear() === current.getFullYear()
      ? { month: "short", day: "numeric" }
      : { year: "numeric", month: "2-digit", day: "2-digit" };
  return date.toLocaleDateString(locale, options);
}

/** Full stamp for a row tooltip, where width no longer matters. */
export function formatFileTimeFull(mtime: number, locale: string): string {
  const date = toDate(mtime);
  if (!date) return "—";
  return date.toLocaleString(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function toDate(mtime: number): Date | null {
  if (!Number.isFinite(mtime) || mtime <= 0) return null;
  const date = new Date(mtime);
  return Number.isNaN(date.getTime()) ? null : date;
}
