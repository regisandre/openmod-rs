//! What the text models see: obfuscation tricks are undone first.

use openmod::normalize::{collapse_repeats, deobfuscate, deobfuscate_within};
use openmod::profanity::contains_profanity;

#[test]
fn common_obfuscations_are_undone_and_normal_text_is_kept() {
    // (case, input, expected)
    let cases = [
        (
            "spaced letters",
            "je vais te t u e r 🔪 ce soir",
            "je vais te tuer 🔪 ce soir",
        ),
        (
            "spaced letters at the end",
            "ik maak je d o o d",
            "ik maak je dood",
        ),
        (
            "leetspeak inside a word",
            "this dr0p was cr4zy",
            "this drop was crazy",
        ),
        (
            "symbols used as letters",
            "you are a b!tch, a$$hole",
            "you are a bitch, asshole",
        ),
        (
            "mentions untouched",
            "shout-out to @dj_mouski",
            "shout-out to @dj_mouski",
        ),
        (
            "ordinals and units untouched",
            "1st floor, 3rd set, mp3 and covid19 rules",
            "1st floor, 3rd set, mp3 and covid19 rules",
        ),
        ("exclamations untouched", "sick!! wow!", "sick!! wow!"),
        ("fancy unicode letters", "𝓴𝓲𝓵𝓵 yourself", "kill yourself"),
        (
            "fullwidth letters",
            "ｆｒｅｅ ｔｉｃｋｅｔｓ",
            "free tickets",
        ),
        ("zero-width characters", "id\u{200B}iot", "idiot"),
        ("long letter floods", "nooooooo waaaay", "noo waay"),
        (
            "plain text untouched",
            "Killer lineup tonight, this set was sick!",
            "Killer lineup tonight, this set was sick!",
        ),
        (
            "numbers and times untouched",
            "Doors 22:00, EUR 12.50, 1500 people, B2B set",
            "Doors 22:00, EUR 12.50, 1500 people, B2B set",
        ),
        (
            "two single letters stay apart",
            "il y a du monde",
            "il y a du monde",
        ),
        (
            "cyrillic untouched",
            "Найкраща вечірка цього літа",
            "Найкраща вечірка цього літа",
        ),
        (
            "emails untouched",
            "mail jane@example.org",
            "mail jane@example.org",
        ),
    ];
    for (case, input, expected) in cases {
        assert_eq!(deobfuscate(input), expected, "{case}");
    }
}

#[test]
fn a_text_that_grows_past_twice_the_limit_when_normalized_is_refused() {
    let ligatures = "\u{FDFA}".repeat(20_000);
    let plain = "a".repeat(20_000);
    let few = format!("{}{}", "a".repeat(19_000), "\u{FDFA}".repeat(10));
    let under = "ab".repeat(19_999);
    let over = "ab".repeat(20_001);
    // (case, input, the limit in characters, kept)
    let cases: Vec<(&str, &str, usize, bool)> = vec![
        ("an 18-fold ligature flood", &ligatures, 20_000, false),
        ("plain letters at the limit", &plain, 20_000, true),
        ("a few ligatures in a normal text", &few, 20_000, true),
        ("just under twice the limit", &under, 20_000, true),
        ("just over twice the limit", &over, 20_000, false),
    ];
    let mut failures = Vec::new();
    for (case, input, max, kept) in cases {
        let got = deobfuscate_within(input, max);
        if got.is_some() != kept {
            failures.push(format!("{case}: kept = {}, want {kept}", got.is_some()));
        }
        if let Some(text) = got
            && text != deobfuscate(input)
        {
            failures.push(format!("{case}: the kept text differs from deobfuscate"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn a_stretched_swear_word_is_still_found() {
    // (case, text, found)
    let cases = [
        ("stretched vowel", "fuuuuck you", true),
        ("stretched in the middle", "what a shiiiit show", true),
        ("stretched twice", "biiiitch pleeease", true),
        ("a plain swear word", "fuck you", true),
        ("double letters kept", "espece de connard", true),
        (
            "a harmless stretched word",
            "nooooo way, that is greaaat",
            false,
        ),
        ("a harmless text", "see you at the cocktail class", false),
    ];
    let mut failures = Vec::new();
    for (case, text, want) in cases {
        let folded = deobfuscate(text);
        let found = contains_profanity(&folded) || contains_profanity(&collapse_repeats(&folded));
        if found != want {
            failures.push(format!("{case}: found = {found}, want {want}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
