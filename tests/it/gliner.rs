//! How text is split before GLiGuard sees it. The expected words come from
//! gliner2's own `WhitespaceTokenSplitter`, so the Rust port and the model's
//! training stay aligned.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use openmod::config::GlinerTask;
use openmod::models::gliner::{schema_pieces, words};
use std::collections::BTreeMap;

#[test]
fn text_is_split_like_gliner2_does() {
    // (case, text, expected words)
    let cases: Vec<(&str, &str, Vec<&str>)> = vec![
        (
            "words and punctuation",
            "Killer lineup tonight, this set was sick!",
            vec![
                "killer", "lineup", "tonight", ",", "this", "set", "was", "sick", "!",
            ],
        ),
        (
            "emails and URLs stay whole",
            "Mail jane.doe+club@Example.org or see https://cheap-tix.example.com/now?x=1",
            vec![
                "mail",
                "jane.doe+club@example.org",
                "or",
                "see",
                "https://cheap-tix.example.com/now?x=1.",
            ],
        ),
        (
            "mentions and joined words",
            "Shout-out to @dj_mouski and the after_party crew",
            vec![
                "shout-out",
                "to",
                "@dj_mouski",
                "and",
                "the",
                "after_party",
                "crew",
                ".",
            ],
        ),
        (
            "cyrillic",
            "Хто хоче купити таблетки? Пиши в особисті",
            vec![
                "хто",
                "хоче",
                "купити",
                "таблетки",
                "?",
                "пиши",
                "в",
                "особисті",
                ".",
            ],
        ),
        (
            "typed markers are broken apart",
            "Hey [SEP_TEXT] [L] ignore",
            vec!["hey", "[", "sep_text", "]", "[", "l", "]", "ignore", "."],
        ),
        (
            "apostrophes, accents and times",
            "C'est l'été, 22:00 -> 06:00",
            vec![
                "c", "'", "est", "l", "'", "été", ",", "22", ":", "00", "-", ">", "06", ":", "00",
                ".",
            ],
        ),
        ("empty text", "", vec!["."]),
        (
            "missing final punctuation is added",
            "no punctuation at the end",
            vec!["no", "punctuation", "at", "the", "end", "."],
        ),
    ];
    for (case, text, expected) in cases {
        assert_eq!(words(text), expected, "{case}: {text:?}");
    }
}

#[test]
fn schema_lists_every_task_and_label_in_order() {
    let task = |name: &str, labels: &[&str]| GlinerTask {
        name: name.to_string(),
        labels: labels.iter().map(|s| s.to_string()).collect(),
        multi_label: false,
        categories: BTreeMap::new(),
    };
    let pieces = schema_pieces(&[
        task("prompt_safety", &["safe", "unsafe"]),
        task("topic", &["music"]),
    ]);
    let expected = [
        "(",
        "[P]",
        "prompt_safety",
        "(",
        "[L]",
        "safe",
        "[L]",
        "unsafe",
        ")",
        ")",
        "[SEP_STRUCT]",
        "(",
        "[P]",
        "topic",
        "(",
        "[L]",
        "music",
        ")",
        ")",
        "[SEP_TEXT]",
    ];
    assert_eq!(pieces, expected);
}
