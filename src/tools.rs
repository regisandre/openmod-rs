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
    /// A list its publisher keeps up to date: not pinned, refreshed by the
    /// server every `lists_refresh_hours`. Its `size` is an estimate.
    List { url: &'static str },
}

/// Where the derived files are published. `OPENMOD_MODELS_URL` overrides it,
/// for a mirror.
pub const DERIVED_URL: &str =
    "https://github.com/regisandre/openmod-rs/releases/download/models-v1";

/// Memory of the server itself with no model loaded, text rules included.
pub const BASE_RAM_MB: u32 = 19;

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
            Source::List { url } => url.to_string(),
        }
    }

    /// Whether the file is pinned to a size and a SHA-256.
    pub fn pinned(&self) -> bool {
        !matches!(self.source, Source::List { .. })
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

const PPOCR_DET: &str = "PaddlePaddle/PP-OCRv5_mobile_det_onnx";
const PPOCR_DET_REV: &str = "e6f4fa85f00e168c862bc462aebca69eef9b3d3d";
const PPOCR_REC: &str = "PaddlePaddle/latin_PP-OCRv5_mobile_rec_onnx";
const PPOCR_REC_REV: &str = "89d3a50e2c27e2e7cceeab0e944c25c807d5db4f";

/// A list of HaGeZi's DNS blocklists, in its domain-per-line edition: a
/// listed domain covers its subdomains.
const fn hagezi(name: &'static str, url: &'static str, size: u64) -> File {
    File {
        name,
        source: Source::List { url },
        sha256: "",
        size,
    }
}

/// Every domain list reads it, so that a listed public suffix never covers
/// the sites registered under it.
const PUBLIC_SUFFIXES: File = File {
    name: "public_suffix_list.dat",
    source: Source::List {
        url: "https://publicsuffix.org/list/public_suffix_list.dat",
    },
    sha256: "",
    size: 340_000,
};

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
        summary: "Nudity and gore. Its nudity score is averaged with Marqo's, which cancels the mistakes each makes alone.",
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
        summary: "Weapons, drugs, underwear, minors, self-harm, hate symbols, violence, gore and more: any concept you describe in words.",
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
    Tool {
        id: "ocr",
        name: "PP-OCRv5",
        kind: Kind::Image,
        summary: "Reads the text of an image (posters, flyers, screenshots) and checks it like a text: links to adult, gambling or unsafe sites, extremist slogans, phone numbers and more.",
        ram_mb: OCR_RAM_MB,
        license: "Apache-2.0",
        files: &[
            File {
                name: "ppocrv5-mobile-det.onnx",
                source: hf(PPOCR_DET, PPOCR_DET_REV, "inference.onnx"),
                sha256: "a431985659dc921974177a95adcfbb90fd9e51989a5e04d70d0b75f597b6e61d",
                size: 4_826_518,
            },
            File {
                name: "ppocrv5-latin-rec.onnx",
                source: hf(PPOCR_REC, PPOCR_REC_REV, "inference.onnx"),
                sha256: "7888113072263cb471b93f66dd5e2ad70548dc526fa1ace760d0d973dd121498",
                size: 8_042_023,
            },
            File {
                name: "ppocrv5-latin-rec.yml",
                source: hf(PPOCR_REC, PPOCR_REC_REV, "inference.yml"),
                sha256: "0bbe984570f597af3638e50bdf2e8276f3ab26a61966096538b3b0d1849f5c84",
                size: 6_817,
            },
        ],
    },
    Tool {
        id: "qr",
        name: "QR codes",
        kind: Kind::Image,
        summary: "Reads the QR codes of an image and checks what they hold like a text: a code to an adult, gambling or unsafe site, a phone number, or any code at all.",
        ram_mb: QR_RAM_MB,
        license: "MIT OR Apache-2.0",
        files: &[],
    },
    Tool {
        id: "adult_sites",
        name: "Adult sites",
        kind: Kind::Text,
        summary: "Links to 90,000 adult sites, in texts, in the text of images and in QR codes. HaGeZi NSFW, refreshed daily.",
        ram_mb: ADULT_RAM_MB,
        license: "GPL-3.0 + MPL-2.0",
        files: &[
            hagezi(
                "hagezi-nsfw.txt",
                "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/nsfw-onlydomains.txt",
                1_430_000,
            ),
            PUBLIC_SUFFIXES,
        ],
    },
    Tool {
        id: "gambling_sites",
        name: "Gambling sites",
        kind: Kind::Text,
        summary: "Links to 600,000 gambling sites, in texts, in the text of images and in QR codes. HaGeZi Gambling, refreshed daily.",
        ram_mb: GAMBLING_RAM_MB,
        license: "GPL-3.0 + MPL-2.0",
        files: &[
            hagezi(
                "hagezi-gambling.txt",
                "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/gambling-onlydomains.txt",
                10_400_000,
            ),
            PUBLIC_SUFFIXES,
        ],
    },
    Tool {
        id: "unsafe_sites",
        name: "Unsafe sites",
        kind: Kind::Text,
        summary: "Links to 1.6 million malware, phishing and scam sites, in texts, in the text of images and in QR codes. HaGeZi Threat Intelligence Feeds, refreshed daily.",
        ram_mb: UNSAFE_RAM_MB,
        license: "GPL-3.0 + MPL-2.0",
        files: &[
            hagezi(
                "hagezi-tif.txt",
                "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/tif-onlydomains.txt",
                27_200_000,
            ),
            PUBLIC_SUFFIXES,
        ],
    },
];

// The readers are measured like the models. A list holds 8 bytes per domain
// (its hashes), which the measured peak drops match within a few MB.
const OCR_RAM_MB: u32 = 19;
const QR_RAM_MB: u32 = 1;
const ADULT_RAM_MB: u32 = 2;
const GAMBLING_RAM_MB: u32 = 6;
const UNSAFE_RAM_MB: u32 = 16;

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

/// A file is present when it exists: with the expected size when openmod
/// pins the file (its SHA-256 is checked when it is downloaded), not empty
/// when it is a list openmod downloads.
pub fn present(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => match file_for(path) {
            Some(f) if f.pinned() => f.size == meta.len(),
            Some(_) => meta.len() > 0,
            None => true,
        },
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
    let onnx = config
        .models
        .values()
        .any(|e| e.enabled && e.spec.uses_onnx());
    let runtime = if onnx { RUNTIME_RAM_MB } else { 0 };
    BASE_RAM_MB + runtime + enabled.iter().sum::<u32>()
}
