# Citations
In user-visible text, cite workspace files, directories, knowledge nodes, lines, or symbols using standard Markdown links with workspace-relative paths. The UI turns them into interactive chips.

- File, directory, or knowledge node: `[validate.rs](src/auth/validate.rs)` or `[session-model](knowledge/contracts/session-model.md)`
- Line or range: `[validate.rs:42-45](src/auth/validate.rs#L42-L45)` or `[validate.rs:42](src/auth/validate.rs#L42)` (1-based, `1 <= A <= B`)
- Symbol: `[Store.save](src/store.rs#Store.save)` or `[resolve](src/knowledge/validate.rs#resolve)` (dot-separated identifier, no spaces)

Web links (`https://...`) stay ordinary links. Do not use `..` or absolute paths; wrap paths containing spaces in `<...>`. If you are unsure a path exists, write it as plain text.
