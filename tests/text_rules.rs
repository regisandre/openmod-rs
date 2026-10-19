//! Deterministic text rules: personal data, links and spam patterns.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use openmod::rules::scan;

fn categories(text: &str) -> Vec<String> {
    let mut c: Vec<String> = scan(text).into_iter().map(|s| s.category).collect();
    c.sort();
    c.dedup();
    c
}

#[test]
fn rules_flag_personal_data_links_and_spam_without_false_positives() {
    // (case, text, expected categories)
    let cases: Vec<(&str, &str, Vec<&str>)> = vec![
        // personal data
        (
            "email",
            "write me at jane.doe+club@example.org",
            vec!["pii_email"],
        ),
        (
            "obfuscated email",
            "jane.doe [at] example [dot] org",
            vec!["pii_email"],
        ),
        (
            "international phone",
            "call +32 470 12 34 56 tonight",
            vec!["pii_phone"],
        ),
        (
            "national phone",
            "appelle-moi au 06 12 34 56 78",
            vec!["pii_phone"],
        ),
        (
            "valid IBAN",
            "pay to BE71 0961 2345 6769 please",
            vec!["pii_iban"],
        ),
        (
            "invalid IBAN checksum is not flagged",
            "ref BE00 0961 2345 6769",
            vec![],
        ),
        (
            "valid card number (Luhn)",
            "card 4111 1111 1111 1111",
            vec!["pii_card"],
        ),
        (
            "random 16 digits failing Luhn",
            "order 1234 5678 9012 3456",
            vec![],
        ),
        // links
        (
            "http link",
            "tickets on https://cheap-tix.example.com/now",
            vec!["link"],
        ),
        ("bare www link", "see www.example.com", vec!["link"]),
        (
            "messaging handoff",
            "DM me on t.me/selltix",
            vec!["link", "messaging_handoff"],
        ),
        (
            "whatsapp handoff",
            "wa.me/32470123456 for pills",
            vec!["link", "messaging_handoff"],
        ),
        // spam
        ("character flood", "heyyyyyyyyyyyyyyyyy", vec!["spam"]),
        (
            "word repetition",
            "free free free free free free free free tickets",
            vec!["spam"],
        ),
        (
            "repeated phrase",
            "BUY NOW!!! BUY NOW!!! BUY NOW!!! click here click here click here",
            vec!["spam"],
        ),
        (
            "repeated token",
            "follow4follow follow4follow cheap followers 1000 for 5 euro follow4follow follow4follow",
            vec!["spam"],
        ),
        (
            "short refrain is not spam",
            "la la la la, what a night",
            vec![],
        ),
        (
            "one repeated word in a long text is not spam",
            "Techno, techno, techno: three floors of techno and house until the sun comes up",
            vec![],
        ),
        // off-platform contact and scams
        (
            "messaging app named",
            "message me on WhatsApp for a quick deal",
            vec!["messaging_handoff"],
        ),
        (
            "messaging app, Dutch",
            "app me op whatsapp",
            vec!["messaging_handoff"],
        ),
        (
            "phishing link",
            "Your order failed. Verify your card here: http://tickets-secure.example.com/login",
            vec!["link", "scam"],
        ),
        (
            "phishing link, French",
            "Vérifiez votre carte ici sinon elle sera annulée : http://billets.example.com",
            vec!["link", "scam"],
        ),
        (
            "phishing link, Ukrainian",
            "Підтвердьте картку тут: http://kvytky.example.com",
            vec!["link", "scam"],
        ),
        (
            "advance payment",
            "selling two tickets, payment first please",
            vec!["scam"],
        ),
        (
            "advance payment, German",
            "Zwei Tickets, erst zahlen bitte",
            vec!["scam"],
        ),
        (
            "card at the door is not a scam",
            "Pay by card at the door, tickets on https://tickets.example.com",
            vec!["link"],
        ),
        // safe texts that naive filters trip on
        (
            "date and time",
            "Doors open 22:00 on 14/02/2027, ends 06:00",
            vec![],
        ),
        ("price", "Early bird EUR 12.50, regular EUR 18.00", vec![]),
        (
            "year range and capacity",
            "Since 2009 - 2026, 1500 people",
            vec![],
        ),
        (
            "plain event text",
            "Killer lineup tonight, this set was sick!",
            vec![],
        ),
        (
            "ukrainian event text",
            "Найкраща вечірка цього літа, вхід вільний",
            vec![],
        ),
        (
            "version number is not a link",
            "Version 2.0 of the festival",
            vec![],
        ),
    ];

    for (case, text, expected) in cases {
        let got = categories(text);
        let expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        assert_eq!(got, expected, "{case}: {text:?}");
    }
}

#[test]
fn a_token_with_digits_of_another_script_is_no_iban_and_reads_without_a_crash() {
    // The regex engine counts the digits of every script as digits, but an
    // IBAN's are ASCII; a byte-indexed slice used to land inside one of them.
    let cases = [
        ("Devanagari digits", char::from_u32(0x0966)),
        ("fullwidth digits", char::from_u32(0xFF10)),
        ("Arabic-Indic digits", char::from_u32(0x0660)),
    ];
    for (case, digit) in cases {
        let digit = digit.expect("a valid code point");
        let text = format!("pay BE{digit}{digit} ABCD EFGH IJKL now");
        let result = std::panic::catch_unwind(|| categories(&text));
        match result {
            Ok(found) => assert!(found.is_empty(), "{case}: flagged {found:?}"),
            Err(_) => panic!("{case}: the scan crashed"),
        }
    }
}

#[test]
fn rule_scores_are_certain() {
    for s in scan("mail jane@example.org or +32 470 12 34 56") {
        assert_eq!(s.score, 1.0, "{s:?}");
        assert_eq!(s.source, "rules", "{s:?}");
    }
}
