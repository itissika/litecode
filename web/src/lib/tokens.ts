/** K/M abbreviation keeps narrow popovers readable: 12.3K, 123K, 1.2M.
 *  Sub-1000 stays exact (cheap, and loses nothing at small magnitudes). */
export function formatToken(n: number): string {
  if (n < 1000) return n.toLocaleString();
  if (n < 10_000) return `${(n / 1000).toFixed(1)}K`;
  if (n < 1_000_000) return `${Math.round(n / 1000)}K`;
  if (n < 10_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  return `${Math.round(n / 1_000_000)}M`;
}
