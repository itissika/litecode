# SessionData is one writer actor per workspace

`sessions.db` is a single SQLite file. Multiple read-write connections do not add throughput; they turn queueing into `SQLITE_BUSY`. LiteCode therefore gives each workspace one `SessionData` owner: a bounded writer actor on one connection, plus a small read-only pool.

A global write semaphore over many RW connections was rejected: callers would still open side doors, busy timeouts would leak into tools, and FTS/bootstrap would race the turn path. A single actor for *all* reads and writes was rejected: lexical search and transcript projection must not wait behind append. Reads use WAL snapshots on short-lived `query_only` connections.

The writer is the only place that runs session SQL. Tools, engines, CLI, and the code-search worker consume typed commands or a `SessionDataReader`. They never open `sessions.db` themselves.
