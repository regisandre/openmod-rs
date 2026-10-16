//! A deterministic extremism rule: the slogans, codes and names of extremist
//! movements and terrorist organisations, in the seven supported languages.
//!
//! It flags what promotes a movement by its own words, not opinions about
//! it: a lecture title or a news headline that names one is flagged too, so
//! keep this rule for content where such names have no place. Short codes
//! that also mean something ordinary are left out (`88`, `1488`, `ISIS`, a
//! common name); `KKK` counts in capitals only, since `kkk` is laughter in
//! some languages. The patterns read the deobfuscated text, so `s1eg h3il`
//! is caught; the initials read the raw text, where no flood is squeezed.

use crate::decision::Score;
use regex::Regex;
use std::sync::LazyLock;

const PATTERNS: &[&str] = &[
    // Neo-Nazi and white supremacist slogans and codes
    r"\bsieg\s*heil\b",
    r"\bheil\s+hitler\b",
    r"\bзиг\s*хайль\b",
    r"\bwhite\s+power\b",
    r"\bwhite\s+pride\s+world\s*wide\b",
    r"\bwpww\b",
    r"\brahowa\b",
    r"\bblood\s*(?:and|&|\+|n)\s*honou?r\b",
    r"\bcombat\s*18\b",
    r"\b14\s*/\s*88\b",
    r"\bwe\s+must\s+secure\s+the\s+existence\s+of\s+our\s+people\b",
    r"\bgas\s+the\s+jews\b",
    r"\bjuden\s+raus\b",
    r"\bmeine\s+ehre\s+hei(?:ß|ss)t\s+treue\b",
    r"\bblut\s+und\s+ehre\b",
    r"\bdeutschland\s+erwache\b",
    // Their organisations
    r"\bku\s*klux\s*klan\b",
    r"\bnsdap\b",
    r"\bwaffen[\s\-]?ss\b",
    r"\bhitler\s*jugend\b",
    r"\bhitler\s+youth\b",
    r"\batomwaffen\b",
    r"\bstormfront\b",
    r"\bdaily\s+stormer\b",
    r"\bhammerskins?\b",
    r"\bnordic\s+resistance\s+movement\b",
    // Terrorist organisations
    r"\bal[\s\-]?qa[e'ï]?[iy]?da\b",
    r"\bаль[\s\-]?ка[їи]д[аи]\b",
    r"\bdaesh\b",
    r"\bdaech\b",
    r"\bislamic\s+state\b",
    r"\b[ée]tat\s+islamique\b",
    r"\bislamischer?\s+staat\b",
    r"\bestado\s+isl[áa]mico\b",
    r"\bstato\s+islamico\b",
    r"\bislamitische\s+staat\b",
    r"\bісламськ\w*\s+держав\w*",
    r"\bисламск\w*\s+государств\w*",
    r"\bігіл\b",
    r"\bіділ\b",
    r"\bигил\b",
    r"\bboko\s+haram\b",
    r"\bal[\s\-]?shabaa?b\b",
    r"\blashkar[\s\-]e[\s\-]ta[iy]y?ba\b",
    r"\bjaish[\s\-]e[\s\-]moh?amm?ed\b",
    r"\bjabhat\s+al[\s\-]nusra\b",
];

static EXTREMISM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!("(?i)(?:{})", PATTERNS.join("|"))).expect("extremism patterns compile")
});

/// The Klan's initials, in capitals only.
static KLAN_INITIALS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bKKK\b").unwrap());

pub fn contains_extremism(raw: &str, deobfuscated: &str) -> bool {
    EXTREMISM.is_match(&deobfuscated.to_lowercase()) || KLAN_INITIALS.is_match(raw)
}

/// A certain `extremism` score when a slogan or a name is present.
pub fn scan(raw: &str, deobfuscated: &str) -> Option<Score> {
    contains_extremism(raw, deobfuscated).then(|| Score {
        category: "extremism".to_string(),
        score: 1.0,
        source: "rules".to_string(),
    })
}
