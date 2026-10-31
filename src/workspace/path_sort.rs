/// Depth 0 first, then 1…N. Same parent stays together; siblings by name.
/// Used by glob listing and grep hit grouping so both imply the same tree shape.
pub fn glob_hit_key(path: &str) -> (usize, &str, &str) {
    let (parent, name) = match path.rsplit_once('/') {
        Some((parent, name)) => (parent, name),
        None => ("", path),
    };
    let depth = if parent.is_empty() {
        0
    } else {
        1 + parent.bytes().filter(|&b| b == b'/').count()
    };
    (depth, parent, name)
}

pub fn sort_glob_hits(hits: &mut [String]) {
    hits.sort_by(|a, b| glob_hit_key(a).cmp(&glob_hit_key(b)));
}

#[cfg(test)]
mod tests {}
