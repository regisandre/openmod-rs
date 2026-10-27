//! The profanity rule: swear words in the seven languages, without the
//! classic false positives (words that merely contain a swear word).

use openmod::profanity::contains_profanity;

#[test]
fn swear_words_are_found_and_innocent_words_are_not() {
    // (case, text, expected)
    let cases = [
        // English
        ("en swear", "this set was fucking amazing", true),
        ("en insult word", "you stupid asshole", true),
        ("en masked vowel", "what the f*ck is this", true),
        (
            "en innocent containers",
            "cocktail class at Scunthorpe, assistant manager, Sussex",
            false,
        ),
        ("en names", "Dick Smith and Moby Dick", false),
        // French
        ("fr swear", "putain de soirée, merde alors", true),
        ("fr insult", "espèce de connard", true),
        ("fr innocent", "la conférence du cognac à Condom", false),
        // German
        ("de swear", "so eine Scheiße, du Arschloch", true),
        (
            "de innocent",
            "Schießen ist verboten, Fickel ist ein Nachname? nein: Fichte",
            false,
        ),
        // Spanish
        ("es swear", "qué mierda, hijo de puta", true),
        ("es innocent", "computadora, disputa y reputación", false),
        // Italian
        ("it swear", "che cazzo dici, stronzo", true),
        ("it vulgar plural", "basta con queste stronzate", true),
        (
            "it innocent containers",
            "cazzuola, scazzottata vietata, la minestra è calda, coglie l'attimo",
            false,
        ),
        // Dutch
        ("nl swear", "wat een kutfeest, klootzak", true),
        (
            "nl innocent",
            "de kuttenberg? nee: kustlijn en kanker onderzoek",
            false,
        ),
        // Ukrainian and Russian
        ("uk swear", "Та забирайте хоч кудись, бля!", true),
        ("uk mat root", "це повний піздєц", true),
        ("uk verb", "хотіло наєбати мене на 30 грн", true),
        ("uk insult", "ти мудак і сука", true),
        (
            "uk innocent",
            "купити хлібу собі на небо, себе не шкодуй",
            false,
        ),
        ("uk innocent words", "сукня, сукупність і бляшанка", false),
        // obfuscation is undone before the check
        ("spaced", "f u c k you", true),
    ];
    for (case, text, expected) in cases {
        let normalized = openmod::normalize::deobfuscate(text);
        assert_eq!(
            contains_profanity(&normalized),
            expected,
            "{case}: {text:?}"
        );
    }
}
