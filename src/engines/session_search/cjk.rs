//! Chinese word segmentation for the sparse index.
//!
//! `unicode61` — the tokenizer the word path uses — walks a Chinese sentence to
//! its punctuation and hands back one token for the whole run, which is why the
//! product used to answer Chinese with trigram fragments under a 60 % coverage
//! gate: with no words, there was nothing else to count. This module is the
//! missing analyzer. It segments with jieba — the dictionary segmenter the wider
//! Chinese-search ecosystem uses (Lucene's smartcn/IK, Meilisearch's charabia,
//! and the term-oriented tools in between) — in `cut_for_search` mode, which
//! emits a word *and* its sub-words, so an out-of-vocabulary compound still
//! matches through its parts.
//!
//! One analyzer, both sides of the index, which is the invariant that makes the
//! match meaningful:
//!
//! * write side — [`segment`] rewrites each CJK run as space-separated words so
//!   `unicode61` can tokenize it at all (stored in `rows.text_words` and indexed
//!   by the `seg` table);
//! * query side — [`query_tokens`] yields the words a `MATCH` may be built from.
//!
//! Latin text is copied through untouched on both sides, so identifiers and
//! English queries keep the tokenization they already had.
//!
//! The dependency is `jieba-rs`: pure Rust, dictionary embedded in the binary,
//! no data files to install and no C library to link.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use jieba_rs::Jieba;

use super::query_plan;

/// Chinese function words: particles, pronouns, interrogatives and light verbs
/// whose presence says nothing about *which* row is wanted.
///
/// Conservative on purpose, and applied to the **query only** — the index keeps
/// every word the segmenter produced, so a phrase that contains glue still
/// matches it verbatim; only the loose word path ignores glue, exactly as
/// `query_plan::STOPWORDS` does for English.
const STOPWORDS: &[&str] = &[
    "的",
    "了",
    "着",
    "过",
    "吗",
    "呢",
    "吧",
    "啊",
    "呀",
    "哦",
    "嗯",
    "是",
    "在",
    "有",
    "和",
    "与",
    "及",
    "或",
    "而",
    "之",
    "其",
    "我",
    "你",
    "他",
    "她",
    "它",
    "们",
    "我们",
    "你们",
    "他们",
    "咱们",
    "这",
    "那",
    "哪",
    "这个",
    "那个",
    "这些",
    "那些",
    "什么",
    "怎么",
    "怎样",
    "如何",
    "为什么",
    "是否",
    "可以",
    "能不能",
    "能",
    "不能",
    "会",
    "要",
    "想",
    "请",
    "帮",
    "帮我",
    "给我",
    "一下",
    "一些",
    "一个",
    "现在",
    "就是",
    "还是",
    "然后",
    "因为",
    "所以",
    "但是",
    "如果",
    "时候",
    "已经",
    "一直",
    "总是",
    "只是",
    "不要",
    "没有",
    "不是",
    "进行",
    "通过",
    "关于",
    "对于",
    "以及",
    "并且",
    "而且",
    "或者",
    "不过",
    "还有",
    "让",
    "从",
    "把",
    "被",
    "给",
    "向",
    "往",
    "来",
    "去",
    "看",
    "看下",
    "看看",
    "一点",
];

/// Is this word glue on the loose Chinese path?
pub fn is_stopword(word: &str) -> bool {
    STOPWORDS.contains(&word)
}

/// Process-wide segmenter.
///
/// jieba builds its dictionary trie on first use; the embedder's tokenizer
/// ([`super::tokenizer::shared`]) is loaded the same way and for the same
/// reason — the cost belongs to the first query that needs it, not to startup.
fn jieba() -> &'static Jieba {
    static JIEBA: OnceLock<Jieba> = OnceLock::new();
    JIEBA.get_or_init(Jieba::new)
}

/// Does this character carry Chinese words and therefore need segmentation?
///
/// Han, kana, Hangul and the compatibility ideographs — deliberately *not* the
/// punctuation and fullwidth ranges `sparse::has_cjk` also covers: punctuation
/// breaks a run, it is not part of one, and after `normalize` the fullwidth
/// forms are ASCII anyway.
fn is_word_char(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF   // Hiragana + Katakana
        | 0x3400..=0x4DBF // CJK ext A
        | 0x4E00..=0x9FFF // CJK unified
        | 0xAC00..=0xD7AF // Hangul syllables
        | 0xF900..=0xFAFF // CJK compatibility ideographs
    )
}

/// Rewrite the CJK runs of an already-normalized text as space-separated words.
///
/// Everything outside those runs is copied through unchanged, so for any query
/// without CJK the segmented text tokenizes under `unicode61` exactly like the
/// text it came from (`rows.text_norm`).
pub fn segment(text: &str) -> String {
    if !text.chars().any(is_word_char) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let mut run = String::new();
    let mut run_is_words = false;
    for ch in text.chars() {
        let is_words = is_word_char(ch);
        if !run.is_empty() && is_words != run_is_words {
            push_run(&mut out, &run, run_is_words);
            run.clear();
        }
        run_is_words = is_words;
        run.push(ch);
    }
    push_run(&mut out, &run, run_is_words);
    out
}

/// One run of [`segment`]: words get cut, everything else is copied verbatim.
/// A run boundary always becomes one space, so two scripts that touched in the
/// source (`网络abc`) stay two tokens instead of fusing into one.
fn push_run(out: &mut String, run: &str, segmented: bool) {
    if run.is_empty() {
        return;
    }
    if !out.is_empty() && !out.ends_with(' ') {
        out.push(' ');
    }
    if !segmented {
        out.push_str(run);
        return;
    }
    let mut first = true;
    for token in jieba().cut_for_search(run, true) {
        if token.word.trim().is_empty() {
            continue;
        }
        if !first {
            out.push(' ');
        }
        out.push_str(token.word);
        first = false;
    }
    if first {
        // Nothing survived (a run of bare punctuation); drop the separator the
        // caller just wrote rather than leaving a hole.
        out.pop();
    }
}

/// The words a CJK query is searched by.
///
/// Same segmenter as the write side, in query order, deduped, with glue removed
/// — the words the `seg` `MATCH` ORs together, and the terms its coverage counts.
/// A query of pure glue keeps its words: it is still a query.
pub fn query_tokens(normalized: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut run = String::new();
    let mut run_is_words = false;
    for ch in normalized.chars() {
        let is_words = is_word_char(ch);
        if !run.is_empty() && is_words != run_is_words {
            collect_run(&run, run_is_words, &mut tokens, &mut seen);
            run.clear();
        }
        run_is_words = is_words;
        run.push(ch);
    }
    collect_run(&run, run_is_words, &mut tokens, &mut seen);

    let informative: Vec<String> = tokens
        .iter()
        .filter(|t| !is_stopword(t) && !query_plan::is_stopword(t))
        .cloned()
        .collect();
    if informative.is_empty() {
        tokens
    } else {
        informative
    }
}

fn collect_run(run: &str, segmented: bool, tokens: &mut Vec<String>, seen: &mut BTreeSet<String>) {
    if run.is_empty() {
        return;
    }
    let mut push = |word: &str| {
        let word = word.trim();
        if word.is_empty() {
            return;
        }
        if seen.insert(word.to_string()) {
            tokens.push(word.to_string());
        }
    };
    if segmented {
        for token in jieba().cut_for_search(run, true) {
            push(token.word);
        }
    } else {
        // The same split `query_plan::terms_in_order` uses, so an identifier
        // stays one word here exactly as it does on the word path.
        for word in run.split(|c: char| !c.is_alphanumeric() && c != '_') {
            push(word);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chinese_run_becomes_words() {
        let words = segment("会话检索测试");
        let tokens: Vec<&str> = words.split(' ').collect();
        assert!(
            tokens.len() >= 2,
            "a sentence is more than one word: {words:?}"
        );
        assert!(tokens.contains(&"会话"), "{words:?}");
        assert!(tokens.contains(&"检索"), "{words:?}");
        assert!(tokens.iter().all(|t| !t.is_empty()), "{words:?}");
    }

    #[test]
    fn latin_text_is_copied_through_untouched() {
        for text in ["alpha beta gamma", "a_b_c v1.2", ""] {
            assert_eq!(segment(text), text, "{text:?} must not be rewritten");
        }
    }

    #[test]
    fn scripts_do_not_fuse_across_a_run_boundary() {
        let words = segment("abc网络def");
        let tokens: Vec<&str> = words.split(' ').collect();
        assert!(tokens.contains(&"abc"), "{words:?}");
        assert!(tokens.contains(&"def"), "{words:?}");
        assert!(
            tokens.contains(&"网络"),
            "the run contributes its words: {words:?}"
        );
        assert_eq!(tokens.len(), 3, "{words:?}");
    }

    #[test]
    fn query_tokens_are_ordered_deduped_and_glue_free() {
        let tokens = query_tokens(&super::super::sparse::normalize("重试次数能不能放大一点"));
        assert!(tokens.contains(&"重试".to_string()), "{tokens:?}");
        assert!(tokens.contains(&"次数".to_string()), "{tokens:?}");
        assert!(tokens.contains(&"放大".to_string()), "{tokens:?}");
        assert!(
            !tokens.contains(&"能不能".to_string()),
            "glue is dropped: {tokens:?}"
        );
        assert!(
            !tokens.contains(&"能".to_string()),
            "glue is dropped: {tokens:?}"
        );
        let mut sorted = tokens.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), tokens.len(), "no duplicates: {tokens:?}");
    }

    #[test]
    fn a_query_of_pure_glue_is_still_a_query() {
        let tokens = query_tokens(&super::super::sparse::normalize("的了"));
        assert_eq!(tokens.len(), 2, "{tokens:?}");
    }

    #[test]
    fn mixed_scripts_are_split_by_script() {
        let tokens = query_tokens(&super::super::sparse::normalize("codex 感知"));
        assert_eq!(tokens, ["codex", "感知"]);
        let tokens = query_tokens(&super::super::sparse::normalize("retry 重试"));
        assert_eq!(tokens, ["retry", "重试"]);
    }
}
