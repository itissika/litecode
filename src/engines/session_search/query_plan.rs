//! One analysis of one query branch, shared by every retrieval layer.
//!
//! Every layer used to re-derive what the query "meant" on its own: the `LIKE`
//! path took the literal, the word path split it on non-alphanumerics, the
//! proximity path took the alphabetically-first six tokens, and the n-gram path
//! cut trigrams. Four derivations of one string, none of them aware of the
//! others — which is how a query like `the auth refactor token` ended up able to
//! match a row on `the` alone (the words are OR-ed, and `the` is nearly half the
//! corpus).
//!
//! This module is the single derivation. It answers: what are the query's words,
//! in what order, which of them carry intent, how many of them a hit must show
//! before it counts as *this query* rather than a bag of common tokens, which
//! trigrams the n-gram path may use, and which few words proximity should look
//! for. Layers consume it; none of them re-parse the query.
//!
//! # The gate
//!
//! `minimum_should_match` in the Lucene/Elasticsearch sense: of the query's
//! informative clauses, how many must match before a document is a hit at all.
//! FTS5 has no such parameter — its boolean operators are set operations, and a
//! bare query is an AND — so the gate is enforced here, by counting coverage,
//! and by generating an `AND` query when the gate demands everything.
//!
//! The numbers are LiteCode's own product decision, not an engine default:
//!
//! | informative words | must match |
//! |-------------------|------------|
//! | 1                 | 1          |
//! | 2                 | 2          |
//! | 3                 | 2          |
//! | 4+                | ceil(0.6n) |
//!
//! Two words meaning "both" is the strictest rule that still allows a two-word
//! phrase to be searched as words; beyond that the floor grows with the query so
//! a five-word question cannot be answered by two of its words.

use std::collections::BTreeSet;

use super::sparse::{has_cjk, normalize};

/// Words that carry no retrieval intent on their own.
///
/// Deliberately conservative: only function words whose presence says nothing
/// about *which* row is wanted. A term is never dropped when it looks like an
/// identifier, and never dropped from the exact-substring path at all — the
/// literal the user typed always matches verbatim, stopwords included.
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "if", "then", "than", "that", "this", "these", "those",
    "of", "in", "on", "at", "to", "for", "with", "by", "from", "as", "is", "are", "was", "were",
    "be", "been", "being", "it", "its", "do", "does", "did", "how", "why", "what", "when", "where",
    "which", "who", "so", "about", "into", "there", "their", "them", "they", "we", "our", "you",
    "your", "i", "my", "me",
];

/// Longest proximity window in terms, matching the lane's `NEAR` distance: more
/// phrases than this cannot all sit inside one window.
pub const NEAR_MAX_TERMS: usize = 6;

/// Below this many chars there is no trigram to match on.
pub const TRIGRAM_LEN: usize = 3;

/// Is this word one of the query's intent carriers, or glue?
pub fn is_stopword(term: &str) -> bool {
    STOPWORDS.contains(&term)
}

/// Does this word look like something the user typed *to be found exactly* — an
/// identifier, a path fragment, a version, a number? Those are never treated as
/// glue, however short.
pub fn is_protected(term: &str) -> bool {
    term.contains('_')
        || term.chars().any(|c| c.is_ascii_digit())
        || term.chars().any(|c| !c.is_ascii())
}

/// Split a normalized query into words, in the order they were written, deduped.
///
/// Query order matters: proximity must look for the terms as they were asked,
/// not in an order the collection happened to impose.
pub fn terms_in_order(text: &str) -> Vec<String> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut out = Vec::new();
    for token in text.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if !token.is_empty() && seen.insert(token) {
            out.push(token.to_string());
        }
    }
    out
}

/// Same words, sorted and deduped: the deterministic order a generated FTS5
/// query is built in, so the same query always compiles to the same string.
pub fn terms_sorted(text: &str) -> Vec<String> {
    terms_in_order(text).into_iter().collect::<BTreeSet<_>>().into_iter().collect()
}

/// Minimum number of informative words a hit must match, for `n` of them.
pub fn min_word_matches(n: usize) -> usize {
    match n {
        0 => 0,
        1 | 2 => n,
        3 => 2,
        _ => (n * 3).div_ceil(5).min(n),
    }
}

/// The distinct trigrams of a string, in order.
pub fn grams_of(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < TRIGRAM_LEN {
        return Vec::new();
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    for window in chars.windows(TRIGRAM_LEN) {
        let gram: String = window.iter().collect();
        if gram.trim().is_empty() {
            continue;
        }
        if seen.insert(gram.clone()) {
            out.push(gram);
        }
    }
    out
}

/// One `|` branch of a query, analysed.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryBranch {
    /// The branch as written (trimmed).
    pub raw: String,
    /// The branch after the same fold the index was written with.
    pub normalized: String,
    /// Informative words, in query order, deduped.
    pub content_terms: Vec<String>,
    /// Every word, in query order, deduped — what the literal path covers.
    pub terms: Vec<String>,
    /// Minimum informative words a `Lexical`/`Proximity` hit must match.
    pub min_word_matches: usize,
    /// Distinct trigrams of the normalized branch (empty below the floor).
    pub grams: Vec<String>,
    /// `unicode61` cannot segment CJK, so the script routes the branch.
    pub cjk: bool,
}

impl QueryBranch {
    pub fn parse(raw: &str) -> Self {
        let normalized = normalize(raw.trim());
        let terms = terms_in_order(&normalized);
        // Glue is removed only from the *loose* word path, and never when that
        // would leave nothing: a query of pure stopwords is still a query.
        let kept: Vec<String> = terms
            .iter()
            .filter(|t| !is_stopword(t) || is_protected(t))
            .cloned()
            .collect();
        let content_terms = if kept.is_empty() { terms.clone() } else { kept };
        Self {
            raw: raw.trim().to_string(),
            min_word_matches: min_word_matches(content_terms.len()),
            grams: grams_of(&normalized),
            cjk: has_cjk(&normalized),
            normalized,
            terms,
            content_terms,
        }
    }

    /// A branch with nothing to search for.
    pub fn is_empty(&self) -> bool {
        self.normalized.trim().is_empty()
    }

    /// How many words a loose word hit must match.
    pub fn word_min(&self) -> usize {
        self.min_word_matches
    }

    /// How many distinct trigrams a hit must show to clear `floor` coverage.
    /// `floor` comes from the layer's declared semantics.
    pub fn gram_min(&self, floor: f64) -> usize {
        let total = self.grams.len();
        if total == 0 {
            return 0;
        }
        ((total as f64 * floor).ceil() as usize).clamp(1, total)
    }

    /// The words proximity looks for: the informative ones, in query order, at
    /// most as many as fit one `NEAR` window. Falls back to every word when the
    /// gate removed all of them.
    pub fn near_terms(&self) -> Vec<String> {
        let source = if self.content_terms.is_empty() {
            &self.terms
        } else {
            &self.content_terms
        };
        source.iter().take(NEAR_MAX_TERMS).cloned().collect()
    }

    /// A branch worth a proximity query at all: two terms, no CJK (the trigram
    /// path already matches substrings there).
    pub fn proximity_applies(&self) -> bool {
        !self.cjk && self.near_terms().len() >= 2
    }

    /// Whether a loose word layer can run at all.
    pub fn words_apply(&self) -> bool {
        !self.cjk && !self.content_terms.is_empty()
    }

    /// Whether a trigram layer can run at all.
    pub fn grams_apply(&self) -> bool {
        !self.grams.is_empty()
    }

    /// The shortest contiguous fragment of this branch that the fuzzy layer may
    /// accept as a typo. `fraction` comes from the layer's declared semantics.
    ///
    /// Always at least one full window plus a character: a single gram is a
    /// coincidence, not a near miss.
    pub fn min_fuzzy_run(&self, fraction: f64) -> usize {
        let chars = self.normalized.chars().count();
        (((chars as f64 * fraction).ceil() as usize).max(TRIGRAM_LEN + 1)).min(chars.max(TRIGRAM_LEN + 1))
    }
}

/// Split a query into its non-empty, deduped branches (`|` = any may match).
pub fn split_branches(query: &str) -> Vec<&str> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut out = Vec::new();
    for part in query.split('|') {
        let part = part.trim();
        if !part.is_empty() && seen.insert(part) {
            out.push(part);
        }
    }
    out
}

/// How many of `terms` occur in `text` as whole words. Used by the membership
/// probe, which must answer for a pinned row without running the ranker.
pub fn count_word_terms(text: &str, terms: &[String]) -> usize {
    terms.iter().filter(|t| contains_word(text, t)).count()
}

/// How many of `grams` occur in `text` as substrings.
pub fn count_grams(text: &str, grams: &[String]) -> usize {
    grams.iter().filter(|g| text.contains(g.as_str())).count()
}

/// Are two adjacent grams present as one contiguous run? This is what separates
/// "a fragment of the phrase" from "a typo of the phrase" in the fuzzy layer.
///
/// Returns the length **in chars** of the longest unbroken fragment of the query
/// that `text` reproduces. A typo leaves a long fragment intact; an unrelated row
/// can only share a word or two, and coverage alone cannot tell those apart — a
/// shared suffix scores as well as a single wrong character.
pub fn longest_contiguous_run(text: &str, grams: &[String]) -> usize {
    if grams.is_empty() {
        return 0;
    }
    // Adjacent `TRIGRAM_LEN` windows overlap by `TRIGRAM_LEN - 1`, so gluing the
    // first gram to the second one's tail spells the run that must appear for the
    // two to be truly adjacent.
    let glue = |a: &str, b: &str| -> String {
        let tail: String = b.chars().skip(TRIGRAM_LEN - 1).collect();
        format!("{a}{tail}")
    };
    let mut best = 0usize;
    let mut run = 0usize;
    for pair in grams.windows(2) {
        if text.contains(&glue(&pair[0], &pair[1])) {
            run += if run == 0 { 2 } else { 1 };
        } else {
            run = 0;
        }
        best = best.max(run);
    }
    if best == 0 {
        // No two adjacent grams line up; a lone gram is still a fragment.
        return if grams.iter().any(|g| text.contains(g.as_str())) {
            TRIGRAM_LEN
        } else {
            0
        };
    }
    best + TRIGRAM_LEN - 1
}

/// Whole-word containment that does not need a regex: the match must not have an
/// alphanumeric (or `_`) neighbour on either side.
pub fn contains_word(text: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    let boundary = |c: char| !c.is_alphanumeric() && c != '_';
    let mut from = 0usize;
    while let Some(offset) = text[from..].find(word) {
        let start = from + offset;
        let end = start + word.len();
        let before_ok = match text[..start].chars().next_back() {
            Some(c) => boundary(c),
            None => true,
        };
        let after_ok = match text[end..].chars().next() {
            Some(c) => boundary(c),
            None => true,
        };
        if before_ok && after_ok {
            return true;
        }
        from = start + word.chars().next().map(char::len_utf8).unwrap_or(1);
        if from >= text.len() {
            break;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_keep_query_order_and_dedupe() {
        assert_eq!(terms_in_order("beta alpha beta"), ["beta", "alpha"]);
        assert_eq!(terms_sorted("beta alpha beta"), ["alpha", "beta"]);
        assert_eq!(terms_in_order("src/main.rs"), ["src", "main", "rs"]);
    }

    #[test]
    fn the_gate_grows_with_the_query() {
        assert_eq!(min_word_matches(1), 1);
        assert_eq!(min_word_matches(2), 2, "two words is an AND");
        assert_eq!(min_word_matches(3), 2);
        assert_eq!(min_word_matches(4), 3);
        assert_eq!(min_word_matches(5), 3);
        assert_eq!(min_word_matches(6), 4);
        assert_eq!(min_word_matches(0), 0);
    }

    #[test]
    fn stopwords_leave_the_loose_path_but_not_the_literal() {
        let branch = QueryBranch::parse("the auth refactor token");
        assert_eq!(branch.terms, ["the", "auth", "refactor", "token"]);
        assert_eq!(branch.content_terms, ["auth", "refactor", "token"]);
        assert_eq!(branch.word_min(), 2, "3 informative words -> at least 2");
        // The literal path keeps everything: the user typed those words.
        assert!(branch.normalized.contains("the"));
    }

    #[test]
    fn a_query_of_pure_glue_is_still_a_query() {
        let branch = QueryBranch::parse("the of and");
        assert_eq!(branch.content_terms, ["the", "of", "and"]);
        assert_eq!(branch.word_min(), 2);
    }

    #[test]
    fn identifiers_are_never_dropped_as_glue() {
        let branch = QueryBranch::parse("the a_b_c token");
        assert!(branch.content_terms.contains(&"a_b_c".to_string()));
        let version = QueryBranch::parse("in v1_2");
        assert!(version.content_terms.contains(&"v1_2".to_string()));
    }

    #[test]
    fn cjk_routes_to_grams_and_latin_to_words() {
        let cjk = QueryBranch::parse("为什么稀疏检索有噪音");
        assert!(cjk.cjk);
        assert!(!cjk.words_apply());
        assert_eq!(cjk.grams.len(), 8, "ten chars, eight windows of three");
        assert_eq!(cjk.gram_min(0.6), 5);
        assert_eq!(cjk.gram_min(0.35), 3);

        let latin = QueryBranch::parse("session search");
        assert!(!latin.cjk);
        assert!(latin.words_apply());
        assert!(latin.grams_apply());
        assert!(latin.proximity_applies());
    }

    #[test]
    fn near_terms_follow_the_query_not_the_alphabet() {
        let branch = QueryBranch::parse("zebra apple mango");
        assert_eq!(branch.near_terms(), ["zebra", "apple", "mango"]);
        // Glue is skipped first, so proximity spends its window on intent.
        let noisy = QueryBranch::parse("the zebra of apple");
        assert_eq!(noisy.near_terms(), ["zebra", "apple"]);
    }

    #[test]
    fn a_single_word_has_no_proximity_query() {
        let branch = QueryBranch::parse("retry");
        assert!(!branch.proximity_applies());
        assert_eq!(branch.word_min(), 1);
    }

    #[test]
    fn word_counting_is_whole_word_only() {
        let terms = vec!["the".to_string()];
        assert_eq!(count_word_terms("the cat", &terms), 1);
        assert_eq!(count_word_terms("there cat", &terms), 0);
        assert_eq!(count_word_terms("cat the", &terms), 1);
        let terms = vec!["session".to_string()];
        assert_eq!(count_word_terms("sessions", &terms), 0);
    }

    #[test]
    fn gram_counting_and_contiguity() {
        let grams = grams_of("为什么稀疏检索");
        assert_eq!(grams.len(), 5, "seven chars, five windows of three");
        let text = normalize("这里为什么稀疏检索会有噪音");
        assert_eq!(count_grams(&text, &grams), 5, "the whole branch is present");
        assert_eq!(
            longest_contiguous_run(&text, &grams),
            7,
            "the branch is one unbroken fragment"
        );
        let partial = normalize("为什么会有噪音");
        assert!(count_grams(&partial, &grams) < grams.len());
        assert_eq!(longest_contiguous_run(&partial, &grams), 3, "one lone gram");
    }

    #[test]
    fn a_shared_suffix_is_not_a_fragment_of_the_query() {
        // The case that separates "typo" from "different string": a row sharing
        // only the trailing word of a longer query.
        let query = QueryBranch::parse("LIVE_TAIL_MARKER");
        let other = normalize("ARCHIVED_OLD_MARKER buried before compact");
        assert!(count_grams(&other, &query.grams) < query.grams.len());
        assert_eq!(
            longest_contiguous_run(&other, &query.grams),
            7,
            "only `_marker`"
        );
        assert!(
            7 < query.min_fuzzy_run(3.0 / 4.0),
            "a shared word must not clear the fragment floor"
        );

        // One wrong character in an otherwise identical string does.
        let typo = QueryBranch::parse("UNIQUE_SESSION_PHRAZE");
        let row = normalize("UNIQUE_SESSION_PHRASE is here");
        assert!(longest_contiguous_run(&row, &typo.grams) >= typo.min_fuzzy_run(3.0 / 4.0));
    }

    #[test]
    fn branches_split_like_the_product_contract() {
        assert_eq!(split_branches("a|b"), ["a", "b"]);
        assert_eq!(split_branches(" a | b | a |"), ["a", "b"]);
        assert!(split_branches("|").is_empty());
    }
}

