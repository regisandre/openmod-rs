//! Deterministic text rules: personal data, links and spam patterns.
//!
//! These run in microseconds and catch what models are bad at (exact
//! identifiers such as IBANs, card numbers and phone numbers). Every match is
//! certain, so every score is 1.0.

use crate::decision::Score;
use regex::Regex;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::LazyLock;

static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").unwrap());

static EMAIL_OBFUSCATED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)[\w.+\-]+\s*[\[(\{]\s*(?:at|@)\s*[\])\}]\s*[\w\-]+(?:\s*[\[(\{]\s*(?:dot|\.)\s*[\])\}]\s*[\w\-]+)+").unwrap()
});

static URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)\b(?:https?://|www\.)[^\s<>"']+"#).unwrap());

static BARE_DOMAIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[a-z0-9][a-z0-9\-]*(?:\.[a-z0-9\-]+)*\.(?:com|net|org|io|ly|xyz|shop|top|info|biz|ru|me|gg|co|app|link|click|site|online)\b(?:/\S*)?").unwrap()
});

static MESSAGING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:t\.me|telegram\.me|wa\.me|chat\.whatsapp\.com|signal\.me|discord\.gg|snapchat\.com/add)/\S*").unwrap()
});

/// Messaging apps named in plain words ("message me on WhatsApp").
static MESSAGING_APPS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(whats\s?app|telegram|viber|wechat|snapchat)\b").unwrap());

/// "Verify your card / account" in the seven languages. Only counts as a
/// scam next to a link, so "pay by card at the door" is fine.
static VERIFY_CREDENTIALS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)\b(verify|confirm|update|validate)\b.{0,25}\b(card|account|payment|details|password)",
        r"|\b(bestätige|bestätigen|verifiziere|aktualisiere)\b.{0,25}\b(karte|konto|zahlung|daten|passwort)",
        r"|\b(vérifiez|vérifie|confirmez|confirme|mettez à jour)\b.{0,25}\b(carte|compte|paiement|coordonnées|mot de passe)",
        r"|\b(verifica|verifique|confirma|confirme|actualiza)\b.{0,25}\b(tarjeta|cuenta|pago|datos|contraseña)",
        r"|\b(verifica|conferma|aggiorna)\b.{0,25}\b(carta|conto|account|pagamento|dati|password)",
        r"|\b(controleer|bevestig|verifieer)\b.{0,25}\b(kaart|rekening|account|betaling|gegevens|wachtwoord)",
        r"|(підтвердьте|підтверди|перевірте|оновіть).{0,25}(картк|рахун|акаунт|оплат|дані|пароль)",
    ))
    .unwrap()
});

/// "Payment first": the advance-payment ticket resale scam.
static ADVANCE_PAYMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)\b(payment|pay) first\b|\bupfront payment\b",
        r"|\berst zahlen\b|\bvorkasse\b",
        r"|\b(payer|paiement|payez) d['\x{2019}]abord\b",
        r"|\b(pago|pagar|paga) primero\b",
        r"|\bpagamento anticipato\b|\b(paga|pagare) prima\b",
        r"|\beerst betalen\b|\bvooruitbetal",
        r"|спочатку оплат|передоплат|оплата наперед",
    ))
    .unwrap()
});

static IBAN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[A-Za-z]{2}\d{2}(?:\s?[A-Za-z0-9]{4}){2,7}(?:\s?[A-Za-z0-9]{1,4})?\b").unwrap()
});

static CARD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(?:\d[ \-]?){12,18}\d\b").unwrap());

static PHONE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\+?\d(?:[\s.\-()]?\d){8,14}").unwrap());

fn hit(category: &str) -> Score {
    Score {
        category: category.to_string(),
        score: 1.0,
        source: "rules".to_string(),
    }
}

fn overlaps(span: &Range<usize>, taken: &[Range<usize>]) -> bool {
    taken
        .iter()
        .any(|t| span.start < t.end && t.start < span.end)
}

fn digits(s: &str) -> Vec<u32> {
    s.chars().filter_map(|c| c.to_digit(10)).collect()
}

fn luhn_valid(d: &[u32]) -> bool {
    let sum: u32 = d
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &x)| {
            if i % 2 == 1 {
                if x * 2 > 9 { x * 2 - 9 } else { x * 2 }
            } else {
                x
            }
        })
        .sum();
    sum % 10 == 0
}

fn iban_valid(raw: &str) -> bool {
    let s: String = raw
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    if !(15..=34).contains(&s.len()) {
        return false;
    }
    let rearranged = format!("{}{}", &s[4..], &s[..4]);
    let mut rem: u32 = 0;
    for c in rearranged.chars() {
        let v = match c {
            '0'..='9' => c as u32 - '0' as u32,
            'A'..='Z' => c as u32 - 'A' as u32 + 10,
            _ => return false,
        };
        let width = if v >= 10 { 100 } else { 10 };
        rem = (rem * width + v) % 97;
    }
    rem == 1
}

/// A single character repeated eight times or more ("heyyyyyyyy").
fn has_character_flood(text: &str) -> bool {
    let mut prev = None;
    let mut run = 0;
    for c in text.chars() {
        if Some(c) == prev && !c.is_whitespace() {
            run += 1;
            if run >= 8 {
                return true;
            }
        } else {
            prev = Some(c);
            run = 1;
        }
    }
    false
}

/// The same word or phrase of up to three words, at least five letters long,
/// repeated over a large share of the text: "free free free free",
/// "BUY NOW!!! BUY NOW!!! BUY NOW!!!", "follow4follow ... follow4follow".
fn has_repeated_phrase(text: &str) -> bool {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    for n in 1..=3usize {
        if words.len() < n {
            break;
        }
        let mut counts: HashMap<&[String], usize> = HashMap::new();
        for gram in words.windows(n) {
            *counts.entry(gram).or_default() += 1;
        }
        let min_repeats = if n == 1 { 4 } else { 3 };
        for (gram, count) in counts {
            let letters: usize = gram.iter().map(|w| w.chars().count()).sum();
            if letters >= 5 && count >= min_repeats && count * n * 10 >= words.len() * 3 {
                return true;
            }
        }
    }
    false
}

/// Runs every rule on `text` and returns one score per matched category.
pub fn scan(text: &str) -> Vec<Score> {
    let mut out = Vec::new();
    let mut taken: Vec<Range<usize>> = Vec::new();

    let found = |category: &str, out: &mut Vec<Score>| {
        if !out.iter().any(|s: &Score| s.category == category) {
            out.push(hit(category));
        }
    };

    for m in EMAIL
        .find_iter(text)
        .chain(EMAIL_OBFUSCATED.find_iter(text))
    {
        taken.push(m.range());
        found("pii_email", &mut out);
    }
    for m in MESSAGING.find_iter(text) {
        taken.push(m.range());
        found("link", &mut out);
        found("messaging_handoff", &mut out);
    }
    if MESSAGING_APPS.is_match(text) {
        found("messaging_handoff", &mut out);
    }
    for m in URL.find_iter(text) {
        taken.push(m.range());
        found("link", &mut out);
    }
    for m in BARE_DOMAIN.find_iter(text) {
        if !overlaps(&m.range(), &taken) {
            taken.push(m.range());
            found("link", &mut out);
        }
    }
    for m in IBAN.find_iter(text) {
        // The pattern can swallow a following word ("... 6769 please"), so
        // try the longest valid prefix made of whole groups.
        let s = m.as_str();
        let mut ends = s.match_indices(' ').map(|(i, _)| i).chain([s.len()]);
        let valid_end = ends.rfind(|&end| iban_valid(&s[..end]));
        if let Some(end) = valid_end {
            taken.push(m.start()..m.start() + end);
            found("pii_iban", &mut out);
        }
    }
    for m in CARD.find_iter(text) {
        let d = digits(m.as_str());
        if (13..=19).contains(&d.len()) && luhn_valid(&d) && !overlaps(&m.range(), &taken) {
            taken.push(m.range());
            found("pii_card", &mut out);
        }
    }
    for m in PHONE.find_iter(text) {
        // A phone number stands alone: no letter or digit glued before it, and
        // no further digit after it (a longer number is something else).
        let before = text[..m.start()].chars().next_back();
        let mut after = text[m.end()..].chars();
        let next = after.next();
        let glued = before.is_some_and(|c| c.is_alphanumeric())
            || next.is_some_and(|c| c.is_ascii_digit())
            || (next.is_some_and(|c| " .-".contains(c))
                && after.next().is_some_and(|c| c.is_ascii_digit()));
        if glued || overlaps(&m.range(), &taken) {
            continue;
        }
        if (9..=15).contains(&digits(m.as_str()).len()) {
            found("pii_phone", &mut out);
        }
    }
    if has_character_flood(text) || has_repeated_phrase(text) {
        found("spam", &mut out);
    }
    let has_link = out.iter().any(|s| s.category == "link");
    if (has_link && VERIFY_CREDENTIALS.is_match(text)) || ADVANCE_PAYMENT.is_match(text) {
        found("scam", &mut out);
    }
    out
}
