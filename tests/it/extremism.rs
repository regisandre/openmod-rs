//! The extremism rule: slogans, codes and organisation names, not opinions.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use openmod::extremism::contains_extremism;
use openmod::normalize::deobfuscate;

#[test]
fn extremist_slogans_and_groups_are_flagged_in_every_language_and_look_alikes_are_not() {
    // (case, text, flagged)
    let cases: Vec<(&str, &str, bool)> = vec![
        (
            "a Nazi salute, in capitals as on a poster",
            "SIEG HEIL",
            true,
        ),
        (
            "split over two lines, as OCR reads it",
            "heil\nhitler",
            true,
        ),
        ("a neo-Nazi network", "Blood & Honour concert", true),
        ("a white supremacist slogan", "white power forever", true),
        ("the 14/88 code", "14/88", true),
        ("the Klan by name", "ku klux klan rally", true),
        ("the Klan by its initials", "Join the KKK", true),
        ("a terrorist group", "Al-Qaeda", true),
        ("a terrorist group, another spelling", "Al Qaïda", true),
        ("a terrorist group, French", "Daech", true),
        ("the Islamic State, French", "État islamique", true),
        ("the Islamic State, German", "Islamischer Staat", true),
        ("the Islamic State, Spanish", "Estado Islámico", true),
        ("the Islamic State, Italian", "Stato Islamico", true),
        ("the Islamic State, Dutch", "Islamitische Staat", true),
        ("the Islamic State, Ukrainian", "Ісламська держава", true),
        ("the Islamic State, Russian initials", "ИГИЛ", true),
        ("leetspeak", "s1eg h3il", true),
        // look-alikes
        ("a DJ named Isis", "DJ Isis b2b Nova", false),
        ("a tempo", "88 bpm all night", false),
        ("a founding year", "since 1488", false),
        ("laughter in lowercase", "kkk haha", false),
        ("an anti-Nazi song", "Nazi punks f*** off", false),
        ("a religious phrase", "Allahu akbar", false),
        ("a powder, not power", "the white powder", false),
        ("victory, in German", "Sieg im Finale", false),
        ("a town", "Heilbronn", false),
        ("a festival name", "Golden Dawn festival", false),
    ];
    for (case, text, flagged) in cases {
        assert_eq!(
            contains_extremism(text, &deobfuscate(text)),
            flagged,
            "{case}"
        );
    }
}
