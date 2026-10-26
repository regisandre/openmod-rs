//! Undoes the obfuscation tricks people use to slip past moderation, so the
//! text models read what a human reads. The rules keep the raw text.

use unicode_normalization::UnicodeNormalization;

/// Characters that render as nothing.
const INVISIBLE: &[char] = &[
    '\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}', '\u{FEFF}', '\u{00AD}',
];

fn leet(c: char) -> Option<char> {
    Some(match c {
        '0' => 'o',
        '1' => 'i',
        '3' => 'e',
        '4' => 'a',
        '5' => 's',
        '7' => 't',
        '@' => 'a',
        '$' => 's',
        '!' => 'i',
        _ => return None,
    })
}

/// A run of the same letter longer than two becomes two ("noooo" -> "noo").
fn squeeze_floods(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev = None;
    let mut run = 0;
    for c in text.chars() {
        if Some(c) == prev {
            run += 1;
        } else {
            prev = Some(c);
            run = 1;
        }
        if run <= 2 || !c.is_alphabetic() {
            out.push(c);
        }
    }
    out
}

/// Every run of the same letter becomes one ("fuuuck" -> "fuck"), so a word
/// stretched on purpose still meets the word lists.
pub fn collapse_repeats(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev = None;
    for c in text.chars() {
        if Some(c) != prev || !c.is_alphabetic() {
            out.push(c);
        }
        prev = Some(c);
    }
    out
}

/// A letter standing alone, maybe followed by punctuation ("r", "r.").
fn lone_letter(token: &str) -> Option<(char, &str)> {
    let mut chars = token.chars();
    let first = chars.next()?;
    let rest = chars.as_str();
    (first.is_alphabetic() && rest.chars().all(|c| c.is_ascii_punctuation()))
        .then_some((first, rest))
}

/// Three or more single letters separated by single spaces become one word
/// ("t u e r" -> "tuer"). Two stay apart, so "il y a" is untouched.
fn join_spaced_letters(text: &str) -> String {
    let tokens: Vec<&str> = text.split(' ').collect();
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let mut j = i;
        let mut word = String::new();
        let mut tail = "";
        while j < tokens.len() {
            let Some((letter, rest)) = lone_letter(tokens[j]) else {
                break;
            };
            word.push(letter);
            j += 1;
            if !rest.is_empty() {
                tail = rest;
                break;
            }
        }
        if j - i >= 3 {
            out.push(format!("{word}{tail}"));
            i = j;
        } else {
            out.push(tokens[i].to_string());
            i += 1;
        }
    }
    out.join(" ")
}

fn looks_like_address(token: &str) -> bool {
    token.contains("://")
        || token.to_lowercase().starts_with("www.")
        || token.find('@').is_some_and(|at| token[at..].contains('.'))
}

/// Digits and symbols used as letters inside a word ("dr0p", "b!tch",
/// "a$$hole", "put4") become letters. A symbol counts only between two
/// word characters, a digit also at the very end of a word of three letters
/// or more; ordinals, units, mentions, emails and URLs are left alone.
fn undo_leetspeak(text: &str) -> String {
    text.split(' ')
        .map(|token| {
            if looks_like_address(token) {
                return token.to_string();
            }
            let chars: Vec<char> = token.chars().collect();
            let is_part = |c: char| c.is_alphabetic() || leet(c).is_some();
            let mut out = String::with_capacity(token.len());
            let mut i = 0;
            while i < chars.len() {
                if !is_part(chars[i]) {
                    out.push(chars[i]);
                    i += 1;
                    continue;
                }
                let start = i;
                while i < chars.len() && is_part(chars[i]) {
                    i += 1;
                }
                let run = &chars[start..i];
                let letters = run.iter().filter(|c| c.is_alphabetic()).count();
                let followed_by_alnum = chars.get(i).is_some_and(|c| c.is_alphanumeric());
                let inner = run.len() > 2
                    && run[1..run.len() - 1].iter().any(|&c| leet(c).is_some())
                    && run[0].is_alphabetic()
                    && run[run.len() - 1].is_alphabetic();
                let last = run[run.len() - 1];
                let digit_end = letters >= 3
                    && last.is_ascii_digit()
                    && leet(last).is_some()
                    && !followed_by_alnum;
                if letters >= 2 && (inner || digit_end) {
                    out.extend(run.iter().map(|&c| {
                        if c.is_alphabetic() {
                            c
                        } else {
                            leet(c).unwrap_or(c)
                        }
                    }));
                } else {
                    out.extend(run);
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// [`deobfuscate`], but `None` when the result holds more than twice
/// `max_chars` characters. Normalization can swell a text (one U+FDFA becomes
/// eighteen characters), and the models read the swollen text, so the limit on
/// the raw text alone does not bound their work.
pub fn deobfuscate_within(text: &str, max_chars: usize) -> Option<String> {
    let out = deobfuscate(text);
    (out.chars().count() <= max_chars.saturating_mul(2)).then_some(out)
}

pub fn deobfuscate(text: &str) -> String {
    let folded: String = text.nfkc().filter(|c| !INVISIBLE.contains(c)).collect();
    undo_leetspeak(&join_spaced_letters(&squeeze_floods(&folded)))
}
