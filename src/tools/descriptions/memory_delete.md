Delete entries by id from the current session's working memory.

Use it when a fact is no longer true, a preference changed, or an entry was superseded. Unknown ids are reported per item; the rest of the batch still applies.

Prefer deleting or merging stale entries over letting memory grow past its 32-entry / 4000-character budget. Ids are never reused.
