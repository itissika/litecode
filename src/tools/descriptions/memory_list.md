List the current session's working memory: small facts, preferences, decisions, and constraints that must survive compaction.

Use it when you need to check what was already recorded before writing; after a compaction the snapshot is restored automatically. The result includes entry ids for `memory.upsert` / `memory.delete`.

Optional `query` is a case-insensitive substring over entry text. Optional `kind` filters by `fact`, `preference`, `decision`, or `constraint`. The result reports both the returned count and the total stored count.
