//! Who may call the API and the admin interface.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use axum::http::{HeaderMap, HeaderValue, Method};
use openmod::admin::{Sessions, same_origin_write, session_cookie};
use openmod::keys::{KeySet, LiveKeys, State};
use std::time::Duration;

fn key_set(state: &State) -> KeySet {
    let mut keys = KeySet::default();
    keys.extend_state(state);
    keys
}

#[test]
fn an_api_key_matches_only_while_it_exists() {
    let mut state = State::default();
    let website = state.add_key("website").unwrap();
    let app = state.add_key("app").unwrap();
    let before = key_set(&state);
    let id = state.api_keys[0].id.clone();
    assert!(state.revoke(&id));
    let after = key_set(&state);
    let external =
        KeySet::from_lines(["# a comment", "", "  external-key-0123456789abcdef  "].into_iter())
            .unwrap();

    // (case, key set, presented key, expected)
    let cases: Vec<(&str, &KeySet, &str, bool)> = vec![
        ("a generated key matches", &before, &website, true),
        ("a second generated key matches", &before, &app, true),
        (
            "a wrong key does not",
            &before,
            "om_000000000000000000000000000000000000000000000000",
            false,
        ),
        ("a revoked key no longer matches", &after, &website, false),
        ("the other key still matches", &after, &app, true),
        (
            "an external key matches, trimmed",
            &external,
            "external-key-0123456789abcdef",
            true,
        ),
        (
            "a comment line is not a key",
            &external,
            "# a comment",
            false,
        ),
    ];
    for (case, keys, presented, expected) in cases {
        assert_eq!(keys.matches(presented), expected, "{case}");
    }
    assert!(
        website.starts_with("om_") && website.len() >= 40,
        "generated keys are long: {website}"
    );
    let short = KeySet::from_lines(["too-short"].into_iter());
    assert!(
        short.is_err(),
        "an external key under 24 characters is refused"
    );
}

#[test]
fn the_state_file_keeps_digests_never_secrets() {
    let path = std::env::temp_dir().join(format!("openmod-state-{}.toml", std::process::id()));
    let mut state = State::default();
    let key = state.add_key("website").unwrap();
    let token = state.rotate_admin_token();
    state.save(&path).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();
    let loaded = State::load(&path).unwrap();
    std::fs::remove_file(&path).unwrap();

    assert!(!written.contains(&key), "the API key itself is not stored");
    assert!(
        !written.contains(&token),
        "the admin token itself is not stored"
    );
    assert!(
        key_set(&loaded).matches(&key),
        "the stored digest still matches the key"
    );
    assert!(
        loaded.is_admin(&token),
        "the stored digest still matches the admin token"
    );
    assert!(
        State::load(&path.with_extension("missing"))
            .unwrap()
            .api_keys
            .is_empty(),
        "no file, no keys"
    );
}

#[test]
fn a_key_added_or_revoked_by_another_process_applies_without_a_restart() {
    let path = std::env::temp_dir().join(format!("openmod-live-{}.toml", std::process::id()));
    let mut state = State::default();
    let first = state.add_key("first").unwrap();
    state.save(&path).unwrap();
    let mut live = LiveKeys::load(KeySet::default(), &path).unwrap();
    let first_before = live.current().matches(&first);

    // `openmod key add` and `openmod key revoke` edit the file while the
    // server runs.
    let mut other = State::load(&path).unwrap();
    let second = other.add_key("second key").unwrap();
    let id = other.api_keys[0].id.clone();
    assert!(other.revoke(&id));
    other.save(&path).unwrap();
    let keys = live.current().clone();
    std::fs::write(&path, "api_keys = [ half written").unwrap();
    let after_corruption = live.current().matches(&second);
    std::fs::remove_file(&path).unwrap();
    let after_removal = live.current().matches(&second);

    // (case, matches, expected)
    let cases = [
        ("a key that exists at startup", first_before, true),
        ("a key added since", keys.matches(&second), true),
        ("a key revoked since", keys.matches(&first), false),
        (
            "a state file that does not parse keeps the keys already loaded",
            after_corruption,
            true,
        ),
        (
            "a deleted state file leaves no key of its own",
            after_removal,
            false,
        ),
    ];
    for (case, got, expected) in cases {
        assert_eq!(got, expected, "{case}");
    }
}

#[test]
fn an_admin_session_ends_with_its_token_or_its_lifetime() {
    let mut state = State::default();
    let first = state.rotate_admin_token();
    let first_digest = state.admin_digest();
    let sessions = Sessions::new(Duration::from_secs(3600));
    let id = sessions.create(first_digest.unwrap());
    let expired = Sessions::new(Duration::ZERO);
    let stale = expired.create(first_digest.unwrap());
    let second = state.rotate_admin_token();

    // (case, accepted)
    let cases: Vec<(&str, bool)> = vec![
        ("the current admin token", state.is_admin(&second)),
        ("a rotated-away admin token", !state.is_admin(&first)),
        (
            "a live session under the token that created it",
            sessions.check(&id, first_digest),
        ),
        (
            "the same session after the token is rotated",
            !sessions.check(&id, state.admin_digest()),
        ),
        ("an expired session", !expired.check(&stale, first_digest)),
        (
            "an unknown session id",
            !sessions.check("not-a-session", first_digest),
        ),
    ];
    for (case, ok) in cases {
        assert!(ok, "{case}");
    }
}

#[test]
fn admin_writes_must_come_from_the_interface_itself() {
    let headers = |pairs: &[(&'static str, &'static str)]| {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_static(v));
        }
        h
    };
    let ui = [
        ("host", "127.0.0.1:8089"),
        ("x-openmod-admin", "1"),
        ("origin", "http://127.0.0.1:8089"),
        ("sec-fetch-site", "same-origin"),
    ];
    // (case, method, headers, allowed)
    let cases: Vec<(&str, Method, HeaderMap, bool)> = vec![
        (
            "a read needs nothing",
            Method::GET,
            headers(&[("host", "127.0.0.1:8089")]),
            true,
        ),
        (
            "a write from the interface",
            Method::POST,
            headers(&ui),
            true,
        ),
        (
            "a write from a client without browser headers",
            Method::POST,
            headers(&[("host", "127.0.0.1:8089"), ("x-openmod-admin", "1")]),
            true,
        ),
        (
            "a write without the interface header",
            Method::POST,
            headers(&ui[..1]),
            false,
        ),
        (
            "a write from another site",
            Method::POST,
            headers(&[
                ("host", "127.0.0.1:8089"),
                ("x-openmod-admin", "1"),
                ("sec-fetch-site", "cross-site"),
            ]),
            false,
        ),
        (
            "a write from another origin",
            Method::DELETE,
            headers(&[
                ("host", "127.0.0.1:8089"),
                ("x-openmod-admin", "1"),
                ("origin", "http://evil.example"),
            ]),
            false,
        ),
        (
            "a write from a sandboxed frame",
            Method::PUT,
            headers(&[
                ("host", "127.0.0.1:8089"),
                ("x-openmod-admin", "1"),
                ("origin", "null"),
            ]),
            false,
        ),
    ];
    for (case, method, h, allowed) in cases {
        assert_eq!(same_origin_write(&method, &h), allowed, "{case}");
    }
}

#[test]
fn the_session_cookie_is_out_of_reach_of_scripts_and_other_sites() {
    // (case, behind HTTPS, attributes that must appear, attributes that must not)
    let cases: Vec<(&str, bool, Vec<&str>, Vec<&str>)> = vec![
        (
            "plain HTTP on loopback",
            false,
            vec!["HttpOnly", "SameSite=Strict", "Path=/"],
            vec!["Secure"],
        ),
        (
            "behind an HTTPS proxy",
            true,
            vec!["HttpOnly", "SameSite=Strict", "Secure"],
            vec![],
        ),
    ];
    for (case, https, present, absent) in cases {
        let cookie = session_cookie("abc", https);
        assert!(cookie.starts_with("openmod_admin=abc;"), "{case}: {cookie}");
        for a in present {
            assert!(cookie.contains(a), "{case}: missing {a} in {cookie}");
        }
        for a in absent {
            assert!(!cookie.contains(a), "{case}: unexpected {a} in {cookie}");
        }
    }
}
