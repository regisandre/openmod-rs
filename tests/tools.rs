//! The tools a server loads: what they cost, and how their files arrive.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use openmod::config::{Config, DEFAULT_CONFIG};
use openmod::download::{fetch, fetch_list, needed};
use openmod::edit::{Edit, apply_to_str};
use openmod::tools::{self, BASE_RAM_MB};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("openmod-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn config_without(disabled: &[&str]) -> Config {
    let mut raw = DEFAULT_CONFIG.to_string();
    for id in disabled {
        raw = apply_to_str(
            &raw,
            &Edit::SetTool {
                id: id.to_string(),
                enabled: false,
            },
        )
        .unwrap();
    }
    Config::parse(&raw, Path::new(".")).unwrap()
}

#[test]
fn a_disabled_tool_costs_no_memory_and_the_rules_cost_nothing_extra() {
    let cost = |id: &str| tools::find(id).unwrap().ram_mb;
    let all = tools::estimate_mb(&config_without(&[]));
    // (case, disabled tools, expected estimate)
    let cases: Vec<(&str, Vec<&str>, u32)> = vec![
        ("everything", vec![], all),
        (
            "without Qwen3Guard",
            vec!["qwen3guard"],
            all - cost("qwen3guard"),
        ),
        (
            "images only",
            vec!["gliguard", "qwen3guard"],
            all - cost("gliguard") - cost("qwen3guard"),
        ),
        (
            "QR codes and domain lists only: no ONNX Runtime",
            vec![
                "marqo_nsfw",
                "owen_safety",
                "siglip",
                "gliguard",
                "qwen3guard",
                "ocr",
            ],
            BASE_RAM_MB
                + cost("qr")
                + cost("adult_sites")
                + cost("gambling_sites")
                + cost("unsafe_sites"),
        ),
        (
            "text rules only",
            tools::TOOLS.iter().map(|t| t.id).collect(),
            BASE_RAM_MB,
        ),
    ];
    for (case, disabled, expected) in cases {
        assert_eq!(
            tools::estimate_mb(&config_without(&disabled)),
            expected,
            "{case}"
        );
    }
}

#[test]
fn each_session_past_the_first_costs_every_loaded_model_again() {
    let estimate = |sessions: usize, concurrent: usize| {
        let mut config = config_without(&["gliguard", "qwen3guard"]);
        config.server.model_sessions = sessions;
        config.server.max_concurrent = concurrent;
        tools::estimate_mb(&config)
    };
    let one = estimate(1, 4);
    // QR codes and domain lists are loaded once whatever the sessions.
    let per_session: u32 = ["marqo_nsfw", "owen_safety", "siglip", "ocr"]
        .iter()
        .map(|id| tools::find(id).unwrap().session_mb)
        .sum();
    assert!(per_session > 0, "a model session costs memory");
    // (case, model_sessions, max_concurrent, expected estimate)
    let cases: Vec<(&str, usize, usize, u32)> = vec![
        ("one session", 1, 4, one),
        ("two", 2, 4, one + per_session),
        ("four", 4, 4, one + 3 * per_session),
        ("more than the checks at once", 8, 2, one + per_session),
    ];
    for (case, sessions, concurrent, expected) in cases {
        assert_eq!(estimate(sessions, concurrent), expected, "{case}");
    }

    // A model's own `sessions` counts for it alone: the text models kept at
    // one session cost nothing more when the others get two.
    let text_once = DEFAULT_CONFIG
        .replace(
            "[models.gliguard]\nkind = \"text_classifier\"\n",
            "[models.gliguard]\nkind = \"text_classifier\"\nsessions = 1\n",
        )
        .replace(
            "[models.qwen3guard]\nkind = \"text_guard\"\n",
            "[models.qwen3guard]\nkind = \"text_guard\"\nsessions = 1\n",
        );
    assert_eq!(text_once.matches("\nsessions = 1\n").count(), 2, "both set");
    let mut everything = Config::parse(&text_once, Path::new(".")).unwrap();
    let single = tools::estimate_mb(&everything);
    everything.server.model_sessions = 2;
    everything.server.max_concurrent = 2;
    assert_eq!(
        tools::estimate_mb(&everything),
        single + per_session,
        "two sessions, the text models once"
    );
}

#[test]
fn a_model_file_counts_as_present_only_with_its_expected_size_a_list_when_not_empty() {
    let dir = scratch("presence");
    let known = tools::file("owen-safety-xs.onnx").unwrap();
    let right = dir.join("owen-safety-xs.onnx");
    std::fs::File::create(&right)
        .unwrap()
        .set_len(known.size)
        .unwrap();
    let wrong = dir.join("sub").join("owen-safety-xs.onnx");
    std::fs::create_dir_all(wrong.parent().unwrap()).unwrap();
    std::fs::write(&wrong, b"truncated").unwrap();
    let custom = dir.join("my-model.onnx");
    std::fs::write(&custom, b"anything").unwrap();
    let absent = dir.join("absent.onnx");
    let list = dir.join("hagezi-nsfw.txt");
    std::fs::write(&list, b"adult-site.com").unwrap();
    let empty_list = dir.join("sub").join("hagezi-nsfw.txt");
    std::fs::write(&empty_list, b"").unwrap();

    // (case, path, present)
    let cases: Vec<(&str, &Path, bool)> = vec![
        ("a known file with its size", &right, true),
        ("a known file with another size", &wrong, false),
        ("a custom file that exists", &custom, true),
        ("a file that does not exist", &absent, false),
        ("a list of any size", &list, true),
        ("an empty list", &empty_list, false),
    ];
    for (case, path, present) in cases {
        assert_eq!(tools::present(path), present, "{case}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Serves `body` at `/ok` and a 404 anywhere else, for `connections` requests.
fn serve(body: &'static [u8], connections: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().take(connections) {
            let mut stream = stream.unwrap();
            let mut request = [0u8; 2048];
            let n = stream.read(&mut request).unwrap();
            let ok = String::from_utf8_lossy(&request[..n]).starts_with("GET /ok ");
            let head = if ok {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
            } else {
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_string()
            };
            stream.write_all(head.as_bytes()).unwrap();
            if ok {
                stream.write_all(body).unwrap();
            }
        }
    });
    format!("http://{addr}")
}

#[test]
fn a_download_is_kept_only_when_its_size_and_digest_match() {
    const BODY: &[u8] = b"pretend this is an onnx model";
    let sha: String = Sha256::digest(BODY)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let size = BODY.len() as u64;
    let base = serve(BODY, 4);
    let dir = scratch("download");

    // (case, path, sha256, size, expected error fragment or None)
    let cases: Vec<(&str, &str, String, u64, Option<&str>)> = vec![
        ("a matching file", "/ok", sha.clone(), size, None),
        (
            "a digest mismatch",
            "/ok",
            "0".repeat(64),
            size,
            Some("SHA-256"),
        ),
        (
            "a file larger than announced",
            "/ok",
            sha.clone(),
            size - 5,
            Some("bytes"),
        ),
        ("a missing file", "/nope", sha.clone(), size, Some("404")),
    ];
    for (i, (case, path, sha256, size, expected)) in cases.into_iter().enumerate() {
        let dest = dir.join(format!("model-{i}.onnx"));
        let result = fetch(&format!("{base}{path}"), &dest, &sha256, size, |_| {})
            .map_err(|e| format!("{e:#}"));
        let leftovers = std::fs::read_dir(&dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|x| x == "part")
            })
            .count();
        assert_eq!(leftovers, 0, "{case}: no partial file is left behind");
        match expected {
            None => {
                result.unwrap_or_else(|e| panic!("{case}: {e}"));
                assert_eq!(std::fs::read(&dest).unwrap(), BODY, "{case}");
            }
            Some(fragment) => {
                let err = result.expect_err(case);
                assert!(err.contains(fragment), "{case}: {err}");
                assert!(!dest.exists(), "{case}: nothing is kept");
            }
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Serves lists the way their publishers do: `/list` with its content,
/// `/unchanged` answering 304 to a conditional request, `/comments` a list
/// with no domain in it, `/huge` more than any list, 404 anywhere else.
fn serve_lists(connections: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().take(connections) {
            let mut stream = stream.unwrap();
            let mut request = [0u8; 4096];
            let n = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]).to_lowercase();
            let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
            let conditional = request.contains("\r\nif-modified-since:");
            let (status, body): (&str, Vec<u8>) = match path.as_str() {
                "/list" => ("200 OK", b"# a list\nnew-site.com\n".to_vec()),
                "/unchanged" if conditional => ("304 Not Modified", Vec::new()),
                "/unchanged" => ("200 OK", b"changed-site.com\n".to_vec()),
                "/comments" => ("200 OK", b"# nothing listed today\n\n".to_vec()),
                "/huge" => ("200 OK", vec![b'a'; 4096]),
                _ => ("404 Not Found", Vec::new()),
            };
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
        }
    });
    format!("http://{addr}")
}

#[test]
fn a_list_is_replaced_only_by_a_newer_list_that_holds_something() {
    let base = serve_lists(5);
    let dir = scratch("lists");
    let old: &[u8] = b"old-site.com\n";
    let new: &[u8] = b"# a list\nnew-site.com\n";
    // (case, path, a copy already there, replaced or an error fragment, the file after)
    let cases: Vec<(&str, &str, bool, Result<bool, &str>, Option<&[u8]>)> = vec![
        ("a first download", "/list", false, Ok(true), Some(new)),
        (
            "unchanged upstream since our copy",
            "/unchanged",
            true,
            Ok(false),
            Some(old),
        ),
        (
            "a list with no domain in it",
            "/comments",
            true,
            Err("no entry"),
            Some(old),
        ),
        ("larger than any list", "/huge", false, Err("larger"), None),
        ("gone upstream", "/nope", true, Err("404"), Some(old)),
    ];
    for (i, (case, path, existing, expected, after)) in cases.into_iter().enumerate() {
        let dest = dir.join(format!("list-{i}.txt"));
        if existing {
            std::fs::write(&dest, old).unwrap();
        }
        let result =
            fetch_list(&format!("{base}{path}"), &dest, 1024, |_| {}).map_err(|e| format!("{e:#}"));
        match (expected, &result) {
            (Ok(replaced), Ok(got)) => assert_eq!(*got, replaced, "{case}"),
            (Err(fragment), Err(err)) => assert!(err.contains(fragment), "{case}: {err}"),
            _ => panic!("{case}: {result:?}"),
        }
        assert_eq!(
            std::fs::read(&dest).ok().as_deref(),
            after,
            "{case}: the file after"
        );
    }
    let leftovers = std::fs::read_dir(&dir)
        .unwrap()
        .filter(|e| {
            let name = e.as_ref().unwrap().file_name();
            !name.to_string_lossy().ends_with(".txt")
        })
        .count();
    assert_eq!(leftovers, 0, "no partial file is left behind");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn every_missing_file_is_listed_once_even_when_several_tools_read_it() {
    let dir = scratch("needed");
    let adult_off = DEFAULT_CONFIG.replace(
        "[models.adult_sites]\nkind = \"domain_list\"\n",
        "[models.adult_sites]\nkind = \"domain_list\"\nenabled = false\n",
    );
    assert_ne!(adult_off, DEFAULT_CONFIG, "the adult list is turned off");
    let config = Config::parse(&adult_off, &dir).unwrap();
    let every: BTreeSet<String> = config.models.keys().cloned().collect();
    let names = |tools: Option<&BTreeSet<String>>| -> Vec<String> {
        needed(&config, tools).into_iter().map(|n| n.name).collect()
    };
    let (enabled, all) = (names(None), names(Some(&every)));
    // (case, file, times listed for the enabled tools, for every tool)
    let cases: Vec<(&str, &str, usize, usize)> = vec![
        ("a file every list reads", "public_suffix_list.dat", 1, 1),
        ("a list of an enabled tool", "hagezi-gambling.txt", 1, 1),
        ("a list of a disabled tool", "hagezi-nsfw.txt", 0, 1),
        ("a model file", "ppocrv5-mobile-det.onnx", 1, 1),
    ];
    for (case, file, on, any) in cases {
        let count = |names: &[String]| names.iter().filter(|n| *n == file).count();
        assert_eq!(count(&enabled), on, "{case}: enabled tools");
        assert_eq!(count(&all), any, "{case}: every tool");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
