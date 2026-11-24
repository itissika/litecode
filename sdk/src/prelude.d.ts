/**
 * Emit one or more values into the `run_code` output stream; callable anywhere, any number of times.
 * Pass structured values (tool results, plain objects, uniform object arrays, multiline strings) directly rather than pre-stringifying so the host renders compact tables, line-numbered files, and raw multiline text.
 * Accepts any JS value (including `Error`, `Set`, `Map`, `Date`, `BigInt`, circular refs). `inspect(label, value)` labels a rendered block.
 * Media handles (`__litecode_media`) are attached to the turn only when emitted via `inspect(...)`.
 */
declare function inspect(...values: unknown[]): void;
