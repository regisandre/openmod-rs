//! The built-in tools: what each one checks, what it costs in memory, and
//! where its files come from.
//!
//! A tool is one `[models.<id>]` block of the default configuration. The
//! text rules are not a tool: they are always on and cost nothing.

use crate::config::{Config, ModelSpec};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Image,
    Text,
}

#[derive(Debug)]
pub struct Tool {
    /// The `[models.<id>]` name in the configuration.
    pub id: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    /// One line for the admin interface.
    pub summary: &'static str,
    /// Memory it adds, in MB: how much the peak working set drops when this
    /// tool alone is turned off, every other tool on, `threads_per_model = 2`.
    pub ram_mb: u32,
    pub license: &'static str,
    pub files: &'static [File],
}

#[derive(Debug)]
pub struct File {
    pub name: &'static str,
    pub source: Source,
    pub sha256: &'static str,
    pub size: u64,
}

#[derive(Debug)]
pub enum Source {
    /// A file published upstream, pinned to a commit.
    HuggingFace {
        repo: &'static str,
        revision: &'static str,
        path: &'static str,
    },
    /// A file openmod derives from an upstream model (see `tools/export_*.py`),
    /// published with openmod's releases.
    Openmod,
}

/// Where the derived files are published. `OPENMOD_MODELS_URL` overrides it,
/// for a mirror.
pub const DERIVED_URL: &str =
    "https://github.com/regisandre/openmod-rs/releases/download/models-v1";

/// Memory of the server itself with no model loaded, text rules included.
pub const BASE_RAM_MB: u32 = 8;

/// What ONNX Runtime adds once, as soon as one model is loaded.
pub const RUNTIME_RAM_MB: u32 = 53;

impl File {
    pub fn url(&self) -> String {
        match &self.source {
            Source::HuggingFace {
                repo,
                revision,
                path,
            } => {
                format!("https://huggingface.co/{repo}/resolve/{revision}/{path}")
            }
            Source::Openmod => {
                let base =
                    std::env::var("OPENMOD_MODELS_URL").unwrap_or_else(|_| DERIVED_URL.to_string());
                format!("{}/{}", base.trim_end_matches('/'), self.name)
            }
        }
    }
}

const fn hf(repo: &'static str, revision: &'static str, path: &'static str) -> Source {
    Source::HuggingFace {
        repo,
        revision,
        path,
    }
}

const SIGLIP: &str = "Xenova/siglip-base-patch16-224";
const SIGLIP_REV: &str = "4649052661e53c7000355844105f8a1792088239";

pub const TOOLS: &[Tool] = &[
    Tool {
        id: "marqo_nsfw",
        name: "Marqo NSFW",
        kind: Kind::Image,
        summary: "Nudity, with a small vision transformer (5.6M parameters).",
        ram_mb: 27,
        license: "Apache-2.0",
        files: &[File {
            name: "marqo-nsfw-384.onnx",
            source: hf(
                "KanariKanaru/nsfw-image-detection-384-onnx",
                "8edc47eedf74b30fd379673bc202fe3b754b1538",
                "model.onnx",
            ),
            sha256: "e9350e576608afe4b57a089ffeb0ebafa1389cdcea4882dd61df28c45f1c24d2",
            size: 22_450_633,
        }],
    },
    Tool {
        id: "owen_safety",
        name: "Owen safety",
        kind: Kind::Image,
        summary: "Nudity and gore, a second opinion that catches what the first misses.",
        ram_mb: 18,
        license: "MIT",
        files: &[File {
            name: "owen-safety-xs.onnx",
            source: hf(
                "OwenElliott/image-safety-classifier-xs",
                "54f4560bd9c5ee92d45dc30418a8f8680e80de6d",
                "onnx/image-safety-classifier-xs.onnx",
            ),
            sha256: "8c28c49d9075f3ad15ebdc2961f02d5b3f99be944815b848b49c9f0e6f3fb689",
            size: 13_137_569,
        }],
    },
    Tool {
        id: "siglip",
        name: "SigLIP",
        kind: Kind::Image,
        summary: "Weapons, drugs, alcohol, smoking, hate symbols, gestures, violence: any concept you describe in words.",
        ram_mb: 227,
        license: "Apache-2.0",
        files: &[
            File {
                name: "siglip-vision-int8.onnx",
                source: hf(SIGLIP, SIGLIP_REV, "onnx/vision_model_int8.onnx"),
                sha256: "6d00762fcb4aef9bdee1b886fcccc7df466f9eae321e180f97d88e24fa13ac72",
                size: 94_098_316,
            },
            File {
                name: "siglip-text-int8.onnx",
                source: hf(SIGLIP, SIGLIP_REV, "onnx/text_model_int8.onnx"),
                sha256: "9cb5102160e2a2b90c0a999ed6c2b4090865c9d5aa08f09cd10993ad9b38bd5f",
                size: 110_982_746,
            },
            File {
                name: "siglip-tokenizer.json",
                source: hf(SIGLIP, SIGLIP_REV, "tokenizer.json"),
                sha256: "4a17c975210be5ab4c36b47d8dae4eefb866dbfb1e676e394aad85dc30a3ae08",
                size: 2_398_744,
            },
        ],
    },
    Tool {
        id: "gliguard",
        name: "GLiGuard",
        kind: Kind::Text,
        summary: "Unsafe text and 13 categories (hate, violence, sexual, self-harm...) in many languages.",
        ram_mb: 667,
        license: "Apache-2.0",
        files: &[
            File {
                name: "gliguard-emb4.onnx",
                source: Source::Openmod,
                sha256: "59ba5737c9ec4fb699a42c95a6475611fa90632ac654abcc9d799efbfd0d92f0",
                size: 467_498_006,
            },
            File {
                name: "gliguard-tokenizer.json",
                source: Source::Openmod,
                sha256: "f6df10ec83bea993035b2dd7c39345a3d4fcf23421c2adb6cb4ffc1e6d1bc4b5",
                size: 16_020_604,
            },
        ],
    },
    Tool {
        id: "qwen3guard",
        name: "Qwen3Guard",
        kind: Kind::Text,
        summary: "A second opinion on risky texts that removes many confident mistakes. Slower.",
        ram_mb: 550,
        license: "Apache-2.0",
        files: &[
            File {
                name: "qwen3guard-q4.onnx",
                source: Source::Openmod,
                sha256: "0601fe4347e6c95c4f4d6db4912a143dbe9171f6285da6e9f35104220edc036c",
                size: 517_453_222,
            },
            File {
                name: "qwen3guard-tokenizer.json",
                source: hf(
                    "Qwen/Qwen3Guard-Gen-0.6B",
                    "fada3b2f655b89601929198343c94cd2f64d93cc",
                    "tokenizer.json",
                ),
                sha256: "aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4",
                size: 11_422_654,
            },
        ],
    },
];

pub fn find(id: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.id == id)
}

/// The built-in file with this name, in any tool.
pub fn file(name: &str) -> Option<&'static File> {
    TOOLS.iter().flat_map(|t| t.files).find(|f| f.name == name)
}

/// The built-in file a configured path points at, matched on its file name.
pub fn file_for(path: &Path) -> Option<&'static File> {
    file(path.file_name()?.to_str()?)
}

/// A file is present when it exists, with the expected size when openmod
/// knows the file. Its SHA-256 is checked when it is downloaded.
pub fn present(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => file_for(path).is_none_or(|f| f.size == meta.len()),
        _ => false,
    }
}

/// The memory a model costs: measured for the built-in tools, its file sizes
/// for a custom one.
pub fn model_ram_mb(id: &str, spec: &ModelSpec) -> u32 {
    if let Some(tool) = find(id) {
        return tool.ram_mb;
    }
    let bytes: u64 = spec
        .files()
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
    (bytes / 1_000_000) as u32
}

/// The memory the enabled tools of `config` need, in MB. Measured peaks
/// add up for the profiles in the README within a few percent; a single
/// text model alone peaks higher while it loads.
pub fn estimate_mb(config: &Config) -> u32 {
    let enabled: Vec<u32> = config
        .models
        .iter()
        .filter(|(_, e)| e.enabled)
        .map(|(id, e)| model_ram_mb(id, &e.spec))
        .collect();
    let runtime = if enabled.is_empty() {
        0
    } else {
        RUNTIME_RAM_MB
    };
    BASE_RAM_MB + runtime + enabled.iter().sum::<u32>()
}
