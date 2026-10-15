//! How the admin interface edits `openmod.toml`: only what changes, with the
//! comments around it kept, and never into an invalid file.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use openmod::edit::{Edit, apply_to_str};
use openmod::workflow::Threshold;
use std::collections::BTreeMap;

const BASE: &str = r#"# Top comment.

[models.nsfw]
kind = "image_classifier"
path = "models/m.onnx"
size = 224
classes = ["NSFW", "SFW"]
categories = { NSFW = "nudity" }

[workflows.default.image]
# Keep this comment.
nudity = { reject = 0.8, review = 0.5 } # and this one

[workflows.profile.image]
nudity = { reject = 0.9 }
"#;

fn rules(list: &[(&str, Option<f32>, Option<f32>)]) -> BTreeMap<String, Threshold> {
    list.iter()
        .map(|(c, reject, review)| {
            (
                c.to_string(),
                Threshold {
                    reject: *reject,
                    review: *review,
                },
            )
        })
        .collect()
}

fn set_workflow(name: &str, image: &[(&str, Option<f32>, Option<f32>)]) -> Edit {
    Edit::SetWorkflow {
        name: name.to_string(),
        image: rules(image),
        text: BTreeMap::new(),
    }
}

#[test]
fn edits_change_only_their_target_and_keep_the_file_valid() {
    // (case, edit, fragments the result must contain, fragments it must not, or the expected error)
    let cases: Vec<(&str, Edit, Result<(Vec<&str>, Vec<&str>), &str>)> = vec![
        (
            "disable a tool",
            Edit::SetTool {
                id: "nsfw".into(),
                enabled: false,
            },
            Ok((
                vec!["enabled = false", "# Top comment.", "# Keep this comment."],
                vec![],
            )),
        ),
        (
            "change a threshold in place",
            set_workflow("default", &[("nudity", Some(0.7), Some(0.5))]),
            Ok((
                vec![
                    "# Keep this comment.\nnudity = { reject = 0.7, review = 0.5 } # and this one",
                ],
                vec!["0.8"],
            )),
        ),
        (
            "thresholds are written rounded",
            set_workflow("default", &[("nudity", Some(0.3), None)]),
            Ok((vec!["nudity = { reject = 0.3 }"], vec!["0.30000"])),
        ),
        (
            "remove every rule but keep the workflow",
            set_workflow("default", &[]),
            Ok((vec!["[workflows.default]"], vec!["reject = 0.8"])),
        ),
        (
            "create a workflow",
            set_workflow("avatars", &[("nudity", Some(0.6), None)]),
            Ok((
                vec!["[workflows.avatars.image]\nnudity = { reject = 0.6 }"],
                vec![],
            )),
        ),
        (
            "delete a workflow",
            Edit::DeleteWorkflow {
                name: "profile".into(),
            },
            Ok((vec!["[workflows.default.image]"], vec!["profile"])),
        ),
        (
            "the default workflow cannot be deleted",
            Edit::DeleteWorkflow {
                name: "default".into(),
            },
            Err("`default`"),
        ),
        (
            "an unknown workflow cannot be deleted",
            Edit::DeleteWorkflow {
                name: "nope".into(),
            },
            Err("no workflow named `nope`"),
        ),
        (
            "a workflow name is a plain identifier",
            set_workflow("Bad name!", &[("nudity", Some(0.6), None)]),
            Err("workflow name"),
        ),
        (
            "an unknown category is refused",
            set_workflow("default", &[("nudty", Some(0.6), None)]),
            Err("`nudty`"),
        ),
        (
            "a threshold above 1 is refused",
            set_workflow("default", &[("nudity", Some(1.5), None)]),
            Err("outside [0, 1]"),
        ),
        (
            "enable a built-in tool missing from the file",
            Edit::SetTool {
                id: "owen_safety".into(),
                enabled: true,
            },
            Ok((
                vec![
                    "[models.owen_safety]",
                    "owen-safety-xs.onnx",
                    "enabled = true",
                    "# Top comment.",
                ],
                vec![],
            )),
        ),
        (
            "an unknown tool is refused",
            Edit::SetTool {
                id: "nope".into(),
                enabled: true,
            },
            Err("unknown tool `nope`"),
        ),
    ];
    for (case, edit, expected) in cases {
        let result = apply_to_str(BASE, &edit).map_err(|e| format!("{e:#}"));
        match expected {
            Ok((present, absent)) => {
                let out = result.unwrap_or_else(|e| panic!("{case}: {e}"));
                for fragment in present {
                    assert!(
                        out.contains(fragment),
                        "{case}: missing {fragment:?} in\n{out}"
                    );
                }
                for fragment in absent {
                    assert!(
                        !out.contains(fragment),
                        "{case}: unexpected {fragment:?} in\n{out}"
                    );
                }
            }
            Err(fragment) => {
                let err = result.expect_err(case);
                assert!(err.contains(fragment), "{case}: {err}");
            }
        }
    }
}
