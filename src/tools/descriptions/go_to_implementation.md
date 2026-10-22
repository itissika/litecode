Jump to implementations at a file position (traits/interfaces → concrete impls when supported). Pass `file_path` plus 1-based `line` and UTF-16 `character`. Same `{ locations, message }` shape as `go_to_definition`. Prefer `go_to_definition` for the type itself.
Example:
const impls = await tools.go_to_implementation({ file_path: "src/trait_foo.rs", line: 3, character: 10 });
return { locations: impls.locations, message: impls.message };
