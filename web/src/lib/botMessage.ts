import { isMessageItem, itemPlainText } from "../api/adapter";
import type { HumanRow, InputMessageItem, MessageSource } from "../api/types";

/** Legacy rows wrote the sender into the text envelope. */
const LEGACY_BOT_PREFIX =
  /^\[from bot id=([^\]\s]+) name=([^\]]*)\](?:\r?\n)?/;
const LEGACY_BOT_LINE =
  /^\[from bot id=[^\]\s]+ name=[^\]]*\](?:\r?\n)?/gm;

function userBody(row: HumanRow | undefined): InputMessageItem | null {
  if (!row || row.kind !== "item/user") return null;
  const body = row.body;
  return isMessageItem(body) && body.role !== "assistant" ? body : null;
}

/** The sender of one user row, structured first, then the old text envelope. */
export function botSourcesForRow(row: HumanRow | undefined): MessageSource[] {
  if (!row) return [];
  const structured = row.sources?.filter((source) => source.kind === "bot") ?? [];
  if (structured.length > 0) return structured;
  const body = userBody(row);
  if (!body) return [];
  const match = LEGACY_BOT_PREFIX.exec(itemPlainText(body));
  if (!match) return [];
  return [{ kind: "bot", id: match[1], name: match[2] }];
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
  return text.replace(LEGACY_BOT_LINE, "");
}

/** A copy of a user row whose text no longer carries the bot envelope. */
export function rowWithoutBotEnvelope(row: HumanRow): HumanRow {
  if (row.kind !== "item/user") return row;
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
