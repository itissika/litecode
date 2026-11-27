import { isMessageItem } from "../api/adapter";
import type { HumanRow, InputMessageItem, MessageSource } from "../api/types";

const BOT_ENVELOPE_LINE =
  /^\[from bot id=[^\]\s]+ name=[^\]]*\](?:\r?\n)?/gm;

function userBody(row: HumanRow | undefined): InputMessageItem | null {
  if (!row || row.kind !== "item/user") return null;
  const body = row.body;
  return isMessageItem(body) && body.role !== "assistant" ? body : null;
}

/** Structured bot sender identity from row metadata only (never parsed from human text). */
export function botSourcesForRow(row: HumanRow | undefined): MessageSource[] {
  if (!row) return [];
  return row.sources?.filter((source) => source.kind === "bot") ?? [];
}

export function botSourcesForRows(rows: readonly HumanRow[]): MessageSource[] {
  for (const row of rows) {
    const sources = botSourcesForRow(row);
    if (sources.length > 0) return sources;
  }
  return [];
}

/** A queue entry or row is bot-attributed once any bot sender is attached. */
export function isBotSourced(
  sources: readonly MessageSource[] | null | undefined,
): boolean {
  return (sources?.length ?? 0) > 0;
}

/** Remove the transport envelope from what the human reads. */
export function stripBotEnvelope(text: string): string {
  return text.replace(BOT_ENVELOPE_LINE, "");
}

/** Strip the transport envelope only for rows with structured bot provenance. */
export function rowWithoutBotEnvelope(row: HumanRow): HumanRow {
  if (row.kind !== "item/user" || botSourcesForRow(row).length === 0) return row;
  const body = userBody(row);
  if (!body) return row;
  let stripped = false;
  const content = body.content.map((part) => {
    if (stripped || part.type !== "input_text") return part;
    const text = stripBotEnvelope(part.text);
    if (text === part.text) return part;
    stripped = true;
    return { ...part, text };
  });
  return stripped ? { ...row, body: { ...body, content } } : row;
}

export function botSourceName(source: MessageSource): string {
  return source.name.trim() || source.id;
}
