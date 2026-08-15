const INVALID_CHARS = /[\\/<>:"|?*\u0000-\u001f]/;

export function validateFileName(name: string): string | null {
  const trimmed = name.trim();
  if (!trimmed) return "Name cannot be empty";
  if (trimmed === "." || trimmed === "..") return "Invalid name";
  if (INVALID_CHARS.test(trimmed)) return "Name contains invalid characters";
  return null;
}

export function splitFileName(name: string): { stem: string; ext: string } {
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return { stem: name, ext: "" };
  return { stem: name.slice(0, dot), ext: name.slice(dot) };
}

/** VS Code-style unique sibling: `foo.ts` → `foo copy.ts` → `foo copy 2.ts`. */
export function uniqueChildName(existing: string[], desired: string): string {
  const set = new Set(existing.map((n) => n.toLowerCase()));
  if (!set.has(desired.toLowerCase())) return desired;
  const { stem, ext } = splitFileName(desired);
  const copy1 = `${stem} copy${ext}`;
  if (!set.has(copy1.toLowerCase())) return copy1;
  for (let i = 2; i < 1000; i++) {
    const candidate = `${stem} copy ${i}${ext}`;
    if (!set.has(candidate.toLowerCase())) return candidate;
  }
  return `${stem} copy ${Date.now()}${ext}`;
}

export function childNamesAt(
  children: Record<string, { name: string }[] | undefined>,
  parent: string,
): string[] {
  return (children[parent] ?? []).map((e) => e.name);
}
