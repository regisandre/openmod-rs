//! What the text models see: obfuscation tricks are undone first.

use openmod::normalize::deobfuscate;

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
