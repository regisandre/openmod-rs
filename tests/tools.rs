//! The tools a server loads: what they cost, and how their files arrive.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use openmod::config::{Config, DEFAULT_CONFIG};
use openmod::download::fetch;
use openmod::edit::{Edit, apply_to_str};
use openmod::tools::{self, BASE_RAM_MB};
use sha2::{Digest, Sha256};
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
            "text rules only",
            vec![
                "marqo_nsfw",
                "owen_safety",
                "siglip",
                "gliguard",
                "qwen3guard",
            ],
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
fn a_model_file_counts_as_present_only_with_its_expected_size() {
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

    // (case, path, present)
    let cases: Vec<(&str, &Path, bool)> = vec![
        ("a known file with its size", &right, true),
        ("a known file with another size", &wrong, false),
        ("a custom file that exists", &custom, true),
        ("a file that does not exist", &absent, false),
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
