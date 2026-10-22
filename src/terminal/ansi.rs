//! Strip terminal escape sequences from PTY output for agent-readable text.

/// Remove ANSI/terminal control sequences from PTY output.
///
/// Handles CSI (`ESC [ …`), OSC (`ESC ] … BEL` / `ST`), and two-byte `ESC @–Z` escapes.
/// Interactive UI terminals keep raw bytes; agent tools should call this before returning text.
pub fn strip_ansi(s: &str) -> String {
    static CSI: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static OSC: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static TWO: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

    let csi = CSI.get_or_init(|| regex::Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]").expect("csi re"));
    let osc = OSC
        .get_or_init(|| regex::Regex::new(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?").expect("osc re"));
    let two = TWO.get_or_init(|| regex::Regex::new(r"\x1b[@-Z\\-_]").expect("two re"));

    let s = csi.replace_all(s, "");
    let s = osc.replace_all(&s, "");
    two.replace_all(&s, "").into_owned()
}

#[cfg(test)]
mod tests {
    

}
