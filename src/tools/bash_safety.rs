//! Command safety classification for bash tool.
//!
//! Extracted from `bash.rs` to keep safety logic in a focused module.
//! All functions are `pub(crate)` — consumed by `bash.rs` and its tests.

use crate::permission::PermissionAction;

// ---------------------------------------------------------------------------
// Read-only command detection
// ---------------------------------------------------------------------------

/// Commands that are always safe to run concurrently because they perform
/// no side effects on the filesystem or process tree.
pub(crate) const READONLY_COMMANDS: &[&str] = &[
    "ls", "cat", "head", "tail", "find", "grep", "wc", "file", "stat", "pwd", "echo", "which",
    "type", "env", "printenv", "date", "whoami", "id", "uname", "df", "du", "free", "ps",
];

/// Read-only subcommands of multi-command tools (e.g. `git status`).
pub(crate) const READONLY_PREFIXES: &[&str] = &[
    // git read-only subcommands
    "git status",
    "git log",
    "git diff",
    "git show",
    "git branch", // list only; `git branch -d` is still readonly in terms of filesystem
    "git remote",
    // cargo read-only subcommands
    "cargo check",
    "cargo test",
    "cargo clippy",
    // version queries
    "rustc --version",
    "node --version",
    "python --version",
    "npm list",
    "pip list",
];

/// Pipe targets that make any preceding command safe (they limit output).
pub(crate) const READONLY_PIPE_SINKS: &[&str] = &["head", "less", "more"];

/// Commands that perform irreversible operations.
pub(crate) const DESTRUCTIVE_COMMANDS: &[&str] =
    &["rm", "rmdir", "mv", "chmod", "chown", "kill", "pkill", "dd"];

// ---------------------------------------------------------------------------
// Pipe / token helpers
// ---------------------------------------------------------------------------

/// Naively split a command on `|` while respecting single quotes.
/// This is good enough for the read-only classification — we don't need
/// a full shell parser.
pub(crate) fn split_pipe_segments(command: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    for (i, ch) in command.char_indices() {
        match ch {
            '\'' if !in_double_quote => in_single_quote = !in_single_quote,
            '"' if !in_single_quote => in_double_quote = !in_double_quote,
            '|' if !in_single_quote && !in_double_quote => {
                segments.push(&command[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    segments.push(&command[start..]);

    segments
}

/// Extract the first whitespace-delimited token from a string.
pub(crate) fn first_token(s: &str) -> Option<&str> {
    s.split_whitespace().next()
}

/// Check if a command segment contains shell write redirections.
pub(crate) fn has_write_redirection(segment: &str) -> bool {
    // Look for > >> 2> &> redirections (but not 2>&1 which is just stderr->stdout)
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    for (i, token) in tokens.iter().enumerate() {
        // Standalone > or >>
        if *token == ">" || *token == ">>" {
            return true;
        }
        // Token starting with > or >> (e.g., >file, >>file)
        if token.starts_with('>') && !token.starts_with(">&") {
            return true;
        }
        // 2> (stderr redirect) but not 2>&1
        if *token == "2>" || token.starts_with("2>") && !token.starts_with("2>&") {
            return true;
        }
        // &> (redirect both)
        if *token == "&>" || token.starts_with("&>") {
            return true;
        }
        // Check for > after a previous token (e.g., echo hello > file)
        if i > 0 && *token == ">" {
            return true;
        }
    }
    false
}

/// Check if a single pipe segment (command + args) is read-only.
pub(crate) fn is_readonly_segment(segment: &str) -> bool {
    // If the segment contains shell redirections (> >> 2> &>), it's not read-only
    if has_write_redirection(segment) {
        return false;
    }

    // First, check against read-only prefixes (multi-word like "git status").
    for prefix in READONLY_PREFIXES {
        if segment == *prefix || segment.starts_with(&format!("{} ", prefix)) {
            return true;
        }
    }

    // Extract the base command (first token).
    let base_cmd = match first_token(segment) {
        Some(cmd) => cmd,
        None => return false,
    };

    READONLY_COMMANDS.contains(&base_cmd)
}

/// Strip trivial lexical obfuscation before safety matching (1.4).
///
/// Handles: backslash-escaped tokens (`\rm`), surrounding quote wrappers, and
/// command substitution / `eval` indirection (`$(rm -rf /)`, `` `rm -rf /` ``,
/// `eval "rm -rf /"`). This is a low-cost floor that prevents the most common
/// plain-text bypasses; it is not a shell parser.
fn preprocess_command(command: &str) -> String {
    let mut s = command.trim().to_string();
    // Expand `eval <inner>` by taking the following token(s).
    if let Some(rest) = strip_eval(&s) {
        s = rest;
    }
    // Unwrap `$( ... )` / `$(...)` command substitution, outermost-first.
    while let Some(start) = s.find("$(") {
        if let Some(inner) = balanced_parenthesized(&s, start + 2) {
            s = inner;
        } else {
            break;
        }
    }
    // Unwrap backtick command substitution (first pair).
    if let Some(open) = s.find('`')
        && let Some(close) = s[open + 1..].find('`')
    {
        s = s[open + 1..open + 1 + close].to_string();
    }
    // Remove backslash escapes.
    let mut unescaped = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                unescaped.push(next);
            }
        } else {
            unescaped.push(c);
        }
    }
    // Strip surrounding quote wrappers.
    let trimmed = unescaped.trim().to_string();
    if trimmed.len() >= 2 {
        let bytes = trimmed.as_bytes();
        let open = bytes[0];
        let close = bytes[trimmed.len() - 1];
        if (open == b'\'' && close == b'\'') || (open == b'"' && close == b'"') {
            return trimmed[1..trimmed.len() - 1].trim().to_string();
        }
    }
    trimmed
}

/// If `command` starts with `eval`, return the remainder after the keyword.
fn strip_eval(command: &str) -> Option<String> {
    let t = command.trim();
    if t.starts_with("eval") {
        let rest = t["eval".len()..].trim();
        if !rest.is_empty() {
            return Some(rest.to_string());
        }
    }
    None
}

/// Return the content of a balanced `( ... )` group starting at `open_idx`
/// (the index just after `$(`), or `None` if unbalanced.
fn balanced_parenthesized(s: &str, open_idx: usize) -> Option<String> {
    let mut depth = 0usize;
    for (i, ch) in s[open_idx..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return Some(s[open_idx..open_idx + i].to_string());
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Returns true when every pipe segment is read-only (for permission + concurrency).
pub fn is_readonly_command(command: &str) -> bool {
    let trimmed = preprocess_command(command);
    let segments = split_pipe_segments(&trimmed);
    if segments.is_empty() {
        return false;
    }

    for (i, segment) in segments.iter().enumerate() {
        let is_last = i == segments.len() - 1;
        if is_readonly_segment(segment.trim()) {
            continue;
        }
        if is_last && is_readonly_pipe_sink(segment.trim()) {
            continue;
        }
        return false;
    }
    true
}

/// Check if a pipe segment is a read-only sink (head/less/more).
pub(crate) fn is_readonly_pipe_sink(segment: &str) -> bool {
    let base_cmd = match first_token(segment) {
        Some(cmd) => cmd,
        None => return false,
    };
    READONLY_PIPE_SINKS.contains(&base_cmd)
}

// ---------------------------------------------------------------------------
// Dangerous command detection (safety floor)
// ---------------------------------------------------------------------------

/// Returns `Deny` for clearly destructive patterns, `Ask` for suspicious
/// ones, and `Allow` otherwise.
pub(crate) fn check_dangerous_command(command: &str) -> PermissionAction {
    let trimmed = preprocess_command(command);

    // --- Deny: absolutely destructive patterns ---

    // rm -rf / or rm -rf /*
    if is_rm_rf_root(&trimmed) {
        return PermissionAction::Deny;
    }

    // Fork bomb: :(){ :|:& };:
    if is_fork_bomb(&trimmed) {
        return PermissionAction::Deny;
    }

    // mkfs on system devices
    if is_mkfs_on_device(&trimmed) {
        return PermissionAction::Deny;
    }

    // dd writing to raw devices like /dev/sda
    if is_dd_to_device(&trimmed) {
        return PermissionAction::Deny;
    }

    // Redirect to /dev/sda or similar raw devices
    if redirects_to_raw_device(&trimmed) {
        return PermissionAction::Deny;
    }

    // --- Ask: suspicious but not clearly catastrophic ---

    // rm -rf on a specific (non-root) path
    if is_rm_rf(&trimmed) {
        return PermissionAction::Ask;
    }

    PermissionAction::Allow
}

pub(crate) fn is_rm_rf_root(command: &str) -> bool {
    // Match `rm -rf /` and `rm -rf /*` (with various flag orderings)
    let lower = command.to_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().collect();

    if tokens.len() < 3 {
        return false;
    }

    // Find the rm command
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == "rm" {
            break;
        }
        i += 1;
    }
    if i >= tokens.len() || tokens[i] != "rm" {
        return false;
    }

    // Check that -r and -f flags are present (in any combination)
    let mut has_r = false;
    let mut has_f = false;
    let mut j = i + 1;
    while j < tokens.len() {
        let tok = tokens[j];
        if tok.starts_with('-') && !tok.starts_with("--") {
            if tok.contains('r') {
                has_r = true;
            }
            if tok.contains('f') {
                has_f = true;
            }
            j += 1;
        } else {
            break;
        }
    }

    if !(has_r && has_f) {
        return false;
    }

    // Remaining tokens are the targets
    for target in &tokens[j..] {
        let t = target.trim_end_matches('/');
        if t.is_empty() || t == "*" || t == "/*" {
            return true;
        }
    }

    false
}

pub(crate) fn is_fork_bomb(command: &str) -> bool {
    // Classic fork bomb: :(){ :|:& };:
    // Strip whitespace and check for the characteristic pattern
    let compressed: String = command.chars().filter(|c| !c.is_whitespace()).collect();
    compressed.contains(":(){:|:&};:") || compressed.contains(":(){:|:&};")
}

pub(crate) fn is_mkfs_on_device(command: &str) -> bool {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    if tokens.is_empty() {
        return false;
    }

    // Check if the base command is mkfs (or mkfs.* variants)
    let base = tokens[0];
    if !base.starts_with("mkfs") {
        return false;
    }

    // If any argument looks like a raw device, deny
    for token in &tokens[1..] {
        if looks_like_raw_device(token) {
            return true;
        }
    }

    false
}

pub(crate) fn is_dd_to_device(command: &str) -> bool {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    if tokens.is_empty() || tokens[0] != "dd" {
        return false;
    }

    // Check for of= or oflag= pointing to a raw device
    for token in &tokens[1..] {
        if let Some(output) = token.strip_prefix("of=")
            && looks_like_raw_device(output)
        {
            return true;
        }
    }

    false
}

pub(crate) fn redirects_to_raw_device(command: &str) -> bool {
    // Check for shell redirections like > /dev/sda or >> /dev/sda
    let lower = command.to_lowercase();
    let re_patterns = [">", ">>"];

    for pat in &re_patterns {
        if let Some(pos) = lower.find(pat) {
            let after = &lower[pos + pat.len()..];
            let target = after.split_whitespace().next().unwrap_or("").trim();
            if looks_like_raw_device(target) {
                return true;
            }
        }
    }

    false
}

/// Heuristic: does the path look like a raw block device?
pub(crate) fn looks_like_raw_device(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.starts_with("/dev/sd")
        || lower.starts_with("/dev/hd")
        || lower.starts_with("/dev/nvme")
        || lower.starts_with("/dev/vd")
        || lower.starts_with("/dev/mapper/")
        || lower == "/dev/mem"
        || lower == "/dev/kmem"
        || lower == "/dev/port"
}

pub(crate) fn is_rm_rf(command: &str) -> bool {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == "rm" {
            break;
        }
        i += 1;
    }
    if i >= tokens.len() || tokens[i] != "rm" {
        return false;
    }

    let mut has_r = false;
    let mut has_f = false;
    for token in &tokens[i + 1..] {
        if token.starts_with('-') && !token.starts_with("--") {
            if token.contains('r') {
                has_r = true;
            }
            if token.contains('f') {
                has_f = true;
            }
        } else {
            break;
        }
    }

    has_r && has_f
}

// ---------------------------------------------------------------------------
// Destructive command detection (is_destructive)
// ---------------------------------------------------------------------------

pub(crate) fn is_destructive_command(command: &str) -> bool {
    let normalized = preprocess_command(command);
    let base_cmd = match first_token(&normalized) {
        Some(cmd) => cmd,
        None => return false,
    };

    DESTRUCTIVE_COMMANDS.contains(&base_cmd)
}

// ---------------------------------------------------------------------------
// Path extraction for write-lock keys (B2-3.3)
// ---------------------------------------------------------------------------

/// Commands whose non-flag arguments are file paths (read or write side).
const PATH_ARG_COMMANDS: &[&str] = &[
    "cat", "rm", "rmdir", "touch", "cp", "mv", "ln", "chmod", "chown", "head", "tail", "grep",
    "wc", "ls", "mkdir",
];

/// Best-effort extraction of file paths a bash command touches. Used for
/// same-turn per-path resource keys. If nothing can be extracted, the caller
/// takes no key (cross-session bash races are accepted).
pub(crate) fn extract_bash_paths(command: &str) -> Vec<String> {
    let normalized = preprocess_command(command);
    let mut paths = Vec::new();
    for segment in split_pipe_segments(&normalized) {
        collect_redirect_targets(segment, &mut paths);
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let base = tokens[0];
        if PATH_ARG_COMMANDS.contains(&base) {
            for t in &tokens[1..] {
                if t.starts_with('-') {
                    continue;
                }
                paths.push((*t).to_string());
            }
        }
    }
    paths
}

/// Collect `> file` / `>> file` / `2> file` redirection targets (write side).
fn collect_redirect_targets(segment: &str, out: &mut Vec<String>) {
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    let mut prev: Option<&str> = None;
    for token in tokens {
        if let Some(prev_tok) = prev
            && matches!(prev_tok, ">" | ">>" | "2>" | "&>")
        {
            out.push(token.to_string());
        }
        prev = Some(token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // 1.4: plain-text bypasses must be preprocessed away before matching.
    // -----------------------------------------------------------------------

    #[test]
    fn bypass_backslash_escaped_rm_is_denied() {
        // `\rm` with the backslash eaten becomes `rm -rf /` → Deny.
        assert_eq!(
            check_dangerous_command("\\rm -rf /"),
            PermissionAction::Deny
        );
        assert_eq!(
            check_dangerous_command("rm -rf \\/"),
            PermissionAction::Deny
        );
    }

    #[test]
    fn bypass_quote_wrapped_rm_is_denied() {
        // Single- and double-quote wrappers are stripped before matching.
        assert_eq!(
            check_dangerous_command("'rm -rf /'"),
            PermissionAction::Deny
        );
        assert_eq!(
            check_dangerous_command("\"rm -rf /\""),
            PermissionAction::Deny
        );
    }

    #[test]
    fn bypass_command_substitution_rm_is_denied() {
        assert_eq!(
            check_dangerous_command("$(rm -rf /)"),
            PermissionAction::Deny
        );
        assert_eq!(
            check_dangerous_command("echo $(rm -rf /)"),
            PermissionAction::Deny
        );
    }

    #[test]
    fn bypass_backtick_rm_is_denied() {
        assert_eq!(
            check_dangerous_command("`rm -rf /`"),
            PermissionAction::Deny
        );
    }

    #[test]
    fn bypass_eval_rm_is_denied() {
        assert_eq!(
            check_dangerous_command("eval rm -rf /"),
            PermissionAction::Deny
        );
        assert_eq!(
            check_dangerous_command("eval \"rm -rf /\""),
            PermissionAction::Deny
        );
    }

    #[test]
    fn bypass_nested_obfuscation_is_denied() {
        // Nested: quote wrapper containing a backtick-substituted escaped command.
        assert_eq!(
            check_dangerous_command("\"`\\rm -rf /`\""),
            PermissionAction::Deny
        );
    }

    #[test]
    fn bypass_readonly_classification_is_not_foiled() {
        // An escaped destructive command must NOT be treated as read-only.
        assert!(!is_readonly_command("\\rm file.txt"));
        assert!(!is_readonly_command("$(rm file.txt)"));
        assert!(!is_destructive_command("ls")); // sanity: readonly stays readonly
        assert!(is_destructive_command("\\rm file.txt"));
    }

    // -----------------------------------------------------------------------
    // B2-3.3: per-path key extraction
    // -----------------------------------------------------------------------

    #[test]
    fn extract_bash_paths_returns_argument_and_redirect_paths() {
        let paths = extract_bash_paths("cat a.txt > out.txt");
        assert!(paths.contains(&"a.txt".to_string()));
        assert!(paths.contains(&"out.txt".to_string()));
    }

    #[test]
    fn extract_bash_paths_skips_flags() {
        let paths = extract_bash_paths("rm -rf ./build");
        assert!(!paths.iter().any(|p| p.starts_with('-')));
        assert!(paths.contains(&"./build".to_string()));
    }
}
