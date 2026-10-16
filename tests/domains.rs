//! Domain lists: which links point to a listed site.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use openmod::config::Config;
use openmod::decision::Action;
use openmod::domains::{DomainList, Suffixes, hosts};
use openmod::engine::Engine;

/// A slice of the Public Suffix List with each kind of rule.
const SUFFIXES: &str = "\
// comments and blank lines are skipped

com
jp
tochigi.jp
*.kawasaki.jp
!city.kawasaki.jp
dynu.net
";

fn list(text: &str) -> DomainList {
    DomainList::build(&[text], &Suffixes::parse(SUFFIXES))
}

#[test]
fn a_list_reads_the_formats_blocklists_use() {
    let list = list(
        "# Title: a list\n\
         plain-domain.com\n\
         0.0.0.0 hosts-file.com\n\
         127.0.0.1 hosts-file-local.com # an inline comment\n\
         0.0.0.0 first-of-two.com second-of-two.com\n\
         ||adblock-rule.com^\n\
         ||adblock-option.com^$third-party\n\
         UPPER-CASE.COM.\n\
         ! an adblock comment\n\
         \n\
         localhost\n\
         0.0.0.0 0.0.0.0\n\
         192.168.1.1\n",
    );
    // (case, host, listed)
    let cases: Vec<(&str, &str, bool)> = vec![
        ("a bare domain", "plain-domain.com", true),
        ("a hosts-file line", "hosts-file.com", true),
        (
            "a hosts-file line with a comment",
            "hosts-file-local.com",
            true,
        ),
        ("the first name of a hosts line", "first-of-two.com", true),
        ("the second name of a hosts line", "second-of-two.com", true),
        ("an adblock rule", "adblock-rule.com", true),
        ("an adblock rule with options", "adblock-option.com", true),
        ("a domain written in capitals", "upper-case.com", true),
        ("a single label is not a domain", "localhost", false),
        ("an address is not a domain", "192.168.1.1", false),
    ];
    for (case, host, listed) in cases {
        assert_eq!(list.contains(host), listed, "{case}");
    }
    assert_eq!(list.len(), 8, "every domain counted once");
}

#[test]
fn a_listed_domain_covers_its_subdomains_unless_they_belong_to_others() {
    let list = list(
        "adult-site.com\n\
         shop.parent.com\n\
         only-www.com\nwww.only-www.com\nm.only-www.com\n\
         blogs.com\nanna.blogs.com\nbob.blogs.com\ncarl.blogs.com\n\
         tochigi.jp\n\
         shop.kawasaki.jp\n\
         city.kawasaki.jp\n\
         dynu.net\n",
    );
    // (case, host, listed)
    let cases: Vec<(&str, &str, bool)> = vec![
        ("the listed domain", "adult-site.com", true),
        ("its www", "www.adult-site.com", true),
        ("a deep subdomain", "a.b.adult-site.com", true),
        ("a look-alike is another domain", "notadult-site.com", false),
        ("the parent of a listed subdomain", "parent.com", false),
        (
            "www and m do not make a platform",
            "shop.only-www.com",
            true,
        ),
        ("a listed blog on a platform", "anna.blogs.com", true),
        ("the platform itself, listed", "blogs.com", true),
        (
            "an unlisted blog on the same platform",
            "dora.blogs.com",
            false,
        ),
        ("a public suffix matches itself", "tochigi.jp", true),
        (
            "a site under a listed public suffix",
            "utsunomiya.tochigi.jp",
            false,
        ),
        (
            "a public suffix by a wildcard rule matches itself",
            "shop.kawasaki.jp",
            true,
        ),
        ("a site under it", "store.shop.kawasaki.jp", false),
        (
            "an exception to a wildcard rule covers its subdomains",
            "www.city.kawasaki.jp",
            true,
        ),
        (
            "a dynamic DNS host under a listed provider",
            "my-nas.dynu.net",
            false,
        ),
        ("case and a trailing dot", "WWW.Adult-Site.COM.", true),
    ];
    for (case, host, listed) in cases {
        assert_eq!(list.contains(host), listed, "{case}");
    }
}

#[test]
fn an_allowed_domain_is_never_flagged_nor_are_its_subdomains() {
    let list = list(
        "concert-casino.com\n\
         shop.venues.com\n\
         gambling-site.com\n\
         notconcert-casino.com\n",
    )
    .allow(&["Concert-Casino.COM.", "venues.com"]);
    // (case, host, listed)
    let cases: Vec<(&str, &str, bool)> = vec![
        ("the allowed domain", "concert-casino.com", false),
        ("its subdomain", "tickets.concert-casino.com", false),
        (
            "a listed subdomain of an allowed domain",
            "shop.venues.com",
            false,
        ),
        ("another listed domain", "gambling-site.com", true),
        (
            "a listed domain that only ends like an allowed one",
            "notconcert-casino.com",
            true,
        ),
    ];
    for (case, host, listed) in cases {
        assert_eq!(list.contains(host), listed, "{case}");
    }
}

#[test]
fn links_are_found_in_free_text() {
    // (case, text, hosts found)
    let cases: Vec<(&str, &str, Vec<&str>)> = vec![
        (
            "a full URL",
            "visit https://www.casino.com/promo?x=1 now",
            vec!["www.casino.com"],
        ),
        (
            "bare domains, any TLD, any case",
            "casino.bet or Casino.Example.FR",
            vec!["casino.bet", "casino.example.fr"],
        ),
        (
            "a path after the domain",
            "tickets: night.events/e/123",
            vec!["night.events"],
        ),
        (
            "an obfuscated dot",
            "go to casino[.]com or casino2(.)com",
            vec!["casino.com", "casino2.com"],
        ),
        ("punycode", "xn--80ak6aa92e.com", vec!["xn--80ak6aa92e.com"]),
        (
            "one line per host, as OCR returns them",
            "BEST ODDS\nBET-NOW.COM\nwin big",
            vec!["bet-now.com"],
        ),
        (
            "the domain of an email address is not a link",
            "write to jane@club.example.org",
            vec![],
        ),
        (
            "times, versions and abbreviations",
            "doors 22.00, v1.2, e.g. this, i.e. that",
            vec![],
        ),
        ("an IP address", "http://192.168.0.1/admin", vec![]),
        (
            "the same host twice",
            "casino.com and casino.com",
            vec!["casino.com"],
        ),
    ];
    for (case, text, expected) in cases {
        assert_eq!(hosts(text), expected, "{case}");
    }
}

#[test]
fn a_text_linking_to_a_listed_site_scores_the_list_category() {
    let dir = std::env::temp_dir().join(format!("openmod-lists-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lists")).unwrap();
    std::fs::write(
        dir.join("lists/gambling.txt"),
        "casino-royal.bet\ncasinodeparis.fr\n",
    )
    .unwrap();
    std::fs::write(dir.join("lists/psl.dat"), "bet\n").unwrap();
    std::fs::write(
        dir.join("openmod.toml"),
        r#"
[models.gambling_sites]
kind = "domain_list"
paths = ["lists/gambling.txt"]
public_suffixes = "lists/psl.dat"
category = "link_gambling"
allow = ["casinodeparis.fr"]

[workflows.default.text]
link_gambling = { reject = 1.0 }
"#,
    )
    .unwrap();
    let server = Engine::load(Config::load(&dir.join("openmod.toml")).unwrap(), None);
    // (case, text, action)
    let cases: Vec<(&str, &str, Action)> = vec![
        (
            "a link to the listed site",
            "best odds on https://www.casino-royal.bet/promo",
            Action::Reject,
        ),
        (
            "a bare mention of it",
            "go to CASINO-ROYAL.BET",
            Action::Reject,
        ),
        ("another site", "tickets on night.events", Action::Accept),
        (
            "a listed site the tool allows",
            "live at the Casino de Paris, casinodeparis.fr",
            Action::Accept,
        ),
        (
            "the name without the domain",
            "casino royal night",
            Action::Accept,
        ),
    ];
    for (case, text, action) in cases {
        let report = server.check_text(text, "default").unwrap();
        assert_eq!(report.summary.action, action, "{case}");
        if action == Action::Reject {
            let reason = &report.summary.reject_reason[0];
            assert_eq!(reason.id, "link_gambling", "{case}");
            assert_eq!(
                reason.source, "gambling_sites",
                "{case}: the list that matched"
            );
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
