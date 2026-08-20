# LiteCode

Agent coding product. Kernel truth is OpenAI Responses `Item`. Session identity and order live on the log, not a second message type.

## Language

**seq**:
The identity and order of one row in the session log. What is in the database is authoritative. Modules key, sort, and address rows by `seq`.
_Avoid_: buffer index, live overlay id, `Item.id` as the row’s identity, a second in-memory cursor

**Surface**:
Which log rows the model currently sees, and in what order. Compact may hide rows with replace; hidden rows keep their `seq`.
_Avoid_: projecting a skip-empty message list as identity or as a persist signal

**Item**:
The payload of a log row: an OpenAI Responses item. Not a second Message type.
_Avoid_: Cordis Message, a parallel ContentBlock kernel

**封口**:
The same `seq` changing payload status to finished. Used when the user **取消** this turn: seal, then persist those items so the next turn can load that fact.
_Avoid_: opening a new row to mean “this turn stopped”, inferring cancel from a shorter surface

**回退**:
Product choice: physically delete log rows from a user-message anchor onward. Remaining `seq` values are the session. The next append uses the table’s current `MAX(seq)+1`.
_Avoid_: fork-a-child-session as LiteCode revert, calling delete “cancel”

**持久化**:
Writing `Item`s into the session log according to product intent (normally append). Cancel seals then writes; revert deletes and must not append the dead turn’s tail. Persist is not itself an interaction.
_Avoid_: persist ruler, prefix invalidation, `Discarded` meaning the user cancelled

**从日志得到 Surface**:
Walk log rows in `seq` order, apply append and replace, and you have the current Surface. A calculation, not a second store. Older notes say “fold” for this; prefer this phrase.
_Avoid_: fold as identity, fold as a fourth layer besides seq / Surface / Item
