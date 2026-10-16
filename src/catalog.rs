//! Human-readable labels for category ids.
//!
//! Every reason returned by the API carries a stable `id` (for code and
//! translations), a short `text` title and a full `message` sentence that a
//! client can show to its end users as is.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub text: String,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    labels: BTreeMap<String, Label>,
}

const BUILTIN: &[(&str, &str, &str)] = &[
    // images
    (
        "nudity",
        "Nudity",
        "This content contains nudity or sexual content.",
    ),
    (
        "suggestive",
        "Suggestive content",
        "This content is sexually suggestive.",
    ),
    (
        "underwear",
        "Underwear",
        "This content shows people in underwear or lingerie.",
    ),
    (
        "swimwear",
        "Swimwear",
        "This content shows people in swimwear.",
    ),
    (
        "minor",
        "Minor",
        "This content shows children or teenagers.",
    ),
    ("knife", "Knife", "This content shows a knife."),
    ("cannabis", "Cannabis", "This content shows cannabis."),
    (
        "gore",
        "Gore",
        "This content shows gore, blood or graphic injuries.",
    ),
    (
        "violence",
        "Violence",
        "This content shows or promotes violence.",
    ),
    ("weapon", "Weapon", "This content shows a weapon."),
    ("alcohol", "Alcohol", "This content shows alcoholic drinks."),
    ("drugs", "Drugs", "This content shows or promotes drugs."),
    (
        "smoking",
        "Smoking",
        "This content shows smoking or tobacco products.",
    ),
    (
        "hate_symbol",
        "Hate symbol",
        "This content shows a hate symbol.",
    ),
    (
        "offensive_gesture",
        "Offensive gesture",
        "This content shows an offensive gesture.",
    ),
    (
        "self_harm",
        "Self-harm",
        "This content refers to suicide or self-harm.",
    ),
    // text
    (
        "hate",
        "Hate speech",
        "This content attacks people for who they are.",
    ),
    (
        "harassment",
        "Harassment",
        "This content insults or harasses someone.",
    ),
    (
        "sexual",
        "Sexual content",
        "This content contains sexual content.",
    ),
    (
        "illegal",
        "Illegal activity",
        "This content promotes illegal activity.",
    ),
    (
        "unethical",
        "Unethical content",
        "This content is abusive, hateful or unethical.",
    ),
    (
        "political",
        "Politically sensitive",
        "This content touches a politically sensitive topic.",
    ),
    (
        "copyright",
        "Copyright",
        "This content may violate copyright.",
    ),
    (
        "jailbreak",
        "Manipulation attempt",
        "This content tries to manipulate an automated system.",
    ),
    (
        "text_unsafe",
        "Unsafe text",
        "This text breaks the content policy.",
    ),
    ("pii", "Personal data", "This content shares personal data."),
    (
        "privacy",
        "Privacy violation",
        "This content invades someone's privacy.",
    ),
    (
        "misinformation",
        "Misinformation",
        "This content spreads false or misleading information.",
    ),
    (
        "child_safety",
        "Child safety",
        "This content may endanger children.",
    ),
    (
        "regulated_advice",
        "Regulated advice",
        "This content gives risky medical, legal or financial advice.",
    ),
    (
        "pii_email",
        "Email address",
        "This content contains an email address.",
    ),
    (
        "pii_phone",
        "Phone number",
        "This content contains a phone number.",
    ),
    (
        "pii_iban",
        "Bank account",
        "This content contains a bank account number.",
    ),
    (
        "pii_card",
        "Payment card",
        "This content contains a payment card number.",
    ),
    ("link", "Link", "This content contains a link."),
    (
        "messaging_handoff",
        "Off-platform contact",
        "This content asks to continue on another messaging app.",
    ),
    ("spam", "Spam", "This content looks like spam."),
    ("profanity", "Profanity", "This content contains profanity."),
    ("scam", "Scam", "This content looks like a scam."),
    (
        "extremism",
        "Extremism",
        "This content refers to an extremist movement, group or slogan.",
    ),
    (
        "link_adult",
        "Adult site link",
        "This content links to an adult website.",
    ),
    (
        "link_gambling",
        "Gambling site link",
        "This content links to a gambling website.",
    ),
    (
        "link_unsafe",
        "Unsafe site link",
        "This content links to a website known for phishing, scams or malware.",
    ),
    // read out of images
    ("qr_code", "QR code", "This image contains a QR code."),
];

/// How a category read out of an image is named: its prefix, what its title
/// adds, and what its message is about.
const READ: &[(&str, &str, &str)] = &[
    ("qr_", "in a QR code", "A QR code in this image"),
    ("text_", "in the image text", "Text in this image"),
];

impl Catalog {
    pub fn builtin() -> Self {
        let labels = BUILTIN
            .iter()
            .map(|(id, text, message)| {
                (
                    id.to_string(),
                    Label {
                        text: text.to_string(),
                        message: message.to_string(),
                    },
                )
            })
            .collect();
        Catalog { labels }
    }

    /// Adds or replaces the label of a category (used by config overrides).
    pub fn set(&mut self, id: &str, label: Label) {
        self.labels.insert(id.to_string(), label);
    }

    /// The label of `id`: its own, the one of the text category it was read
    /// from (`text_link_adult` from `link_adult`), or one made from the id.
    pub fn label(&self, id: &str) -> Label {
        if let Some(l) = self.labels.get(id) {
            return l.clone();
        }
        if let Some(l) = self.read_label(id) {
            return l;
        }
        let words = id.replace(['_', '-'], " ");
        let mut chars = words.chars();
        let text = match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        };
        Label {
            message: format!("This content was flagged for: {}.", words),
            text,
        }
    }

    fn read_label(&self, id: &str) -> Option<Label> {
        READ.iter().find_map(|(prefix, in_where, subject)| {
            let base = self.labels.get(id.strip_prefix(prefix)?)?;
            let rest = base
                .message
                .strip_prefix("This content ")
                .or_else(|| base.message.strip_prefix("This text "))?;
            Some(Label {
                text: format!("{} {in_where}", base.text),
                message: format!("{subject} {rest}"),
            })
        })
    }
}
