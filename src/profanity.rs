//! A deterministic swear-word rule for the seven supported languages.
//!
//! Models judge harm; they do not flag a casual swear word, which is what a
//! classic profanity filter catches. Each pattern names whole words or a
//! closed list of endings, so words that merely contain a swear word
//! ("cocktail", "Scunthorpe", "cazzuola", "сукня") are not flagged. Run it on
//! deobfuscated text so "f u c k" and "fuuuck" are caught too.

use crate::decision::Score;
use regex::Regex;
use std::sync::LazyLock;

const PATTERNS: &[&str] = &[
    // English
    r"\b(mother)?fuck\w*",
    r"\bf[*#@$]ck\w*",
    r"\b(bull)?shit(s|ty|head|hole)?\b",
    r"\bsh[*#@$]t\b",
    r"\bcunts?\b",
    r"\bbitch(es|y)?\b",
    r"\b(ass|arse)holes?\b",
    r"\bdickheads?\b",
    r"\bwank(er|ers|ing)?\b",
    r"\bwhores?\b",
    r"\bsluts?\b",
    r"\bbastards?\b",
    r"\bfaggots?\b",
    r"\bretard(s|ed)?\b",
    r"\bnigg(a|as|er|ers)\b",
    // French
    r"\bputain\b",
    r"\bputes?\b",
    r"\bsalop(e|es|ard|ards|erie)\b",
    r"\bencul\w*",
    r"\bmerd(e|es|eux|euse)\b",
    r"\bniqu(e|er|ez)\b",
    r"\b(fdp|ntm|pd)\b",
    r"\bconn(ard|arde|ards|asse|asses)\b",
    r"\bb[aâ]tard(e|s|es)?\b",
    r"\bp[eé]tasses?\b",
    r"\bpouffiasses?\b",
    r"\bta gueule\b",
    r"\btapettes?\b",
    r"\bbranleu(r|rs|se)\b",
    r"\bcouill(e|es|on|ons)\b",
    // German
    r"\bschei(ß|ss)e\w*",
    r"\bfotzen?\b",
    r"\bhurensohn\w*",
    r"\barschl[oö]ch(er)?\b",
    r"\bwichser\w*",
    r"\bschlampen?\b",
    r"\b(ge)?fick(en|t|te|st|er|e|ung)?\b",
    r"\bmissgeburt\w*",
    r"\bspast(en|i)?\b",
    r"\bhuren?\b",
    // Spanish
    r"\bputas?\b",
    r"\bputos?\b",
    r"\bhij[oa] de puta\b",
    r"\bhijueputa\b",
    r"\bmierda\w*",
    r"\bcabr[oó]n(es|a|as)?\b",
    r"\bgilipollas\b",
    r"\bcoño\b",
    r"\bjoder\b",
    r"\bjodid[oa]s?\b",
    r"\bpendej(o|a|os|as|ada)\b",
    r"\bmaric(a|as|ón|on|ones)\b",
    r"\bzorras?\b",
    r"\bverga\b",
    r"\bching(a|ar|ada|ado)\b",
    // Italian
    r"\bcazz(o|i|ata|ate|one|oni)\b",
    r"\bincazz\w*",
    r"\bstronz(o|a|i|e|ata|ate)\b",
    r"\b(vaf)?fanculo\b",
    r"\bputtan(a|e|ata|ate)\b",
    r"\bminchi(a|e|ata)\b",
    r"\bcoglion(e|i|ata|ate)\b",
    r"\bfroci(o|i)\b",
    r"\btroi(a|e)\b",
    r"\bmerd(a|e|oso|osa)\b",
    // Dutch
    r"\bkut\b",
    r"\bkut(feest|zooi|wijf|ding|kop|smoel|hoer|leven|weer|muziek|dag|ventje|tje)\w*",
    r"\bklootzak\w*",
    r"\bhoer(en|tje)?\b",
    r"\btering(lijer|lijers)?\b",
    r"\bkanker(lijer|hoer|mongool|zooi|wijf)\w*",
    r"\bmongolen?\b",
    r"\bmongool\b",
    r"\bsletten?\b",
    r"\bneuk(en|t|te|ten)?\b",
    r"\blul\b",
    r"\bklote\b",
    // Ukrainian and Russian
    r"\bху[йяєїеёию]\w*",
    r"\bп[иі]зд\w*",
    r"\bбля(ть|дь|ді|дина|дство|я)?\b",
    r"\b(з['\x{2019}\x{02BC}]|за|на|по|роз|від|ви|у|до|при|пере|вы|от|под|недо)?(їб|йоб|єб|еб|ёб)(ати|ать|ал|ала|али|ан|ана|аний|аный|аш|ашить|ут|уть|у|уч|ен|ло|ла|ли|нул|нути|нуть|ись|ися)\w*",
    r"\bсук(а|и|у|ою|ам|ами)\b",
    r"\bсучк\w*",
    r"\bмуд(ак|аки|ило|ила|озвон)\w*",
    r"\bп[іи]д[оа]р\w*",
    r"\bкурв\w*",
    r"\bшлюх\w*",
    r"\bгандон\w*",
    r"\b(довбо|долбо)[йїеё]?[бо]\w*",
    r"\bдеб[іи]л\w*",
];

static PROFANITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!("(?i)(?:{})", PATTERNS.join("|"))).expect("profanity patterns compile")
});

pub fn contains_profanity(text: &str) -> bool {
    PROFANITY.is_match(&text.to_lowercase())
}

/// A certain `profanity` score when a swear word is present.
pub fn scan(text: &str) -> Option<Score> {
    contains_profanity(text).then(|| Score {
        category: "profanity".to_string(),
        score: 1.0,
        source: "rules".to_string(),
    })
}
