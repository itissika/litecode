/**
 * Emit values into the `run_code` output stream; callable anywhere, any number of times.
 * Accepts any JS value (`Error`, `Set`, `Map`, `Date`, `BigInt`, circular refs). `inspect(label, value)` labels a block.
 * Media handles (`__litecode_media`) are attached to the turn only when emitted via `inspect(...)`.
 */
declare function inspect(...values: unknown[]): void;
