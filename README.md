# openmod

> Status: alpha. Works end to end; thresholds calibrated on a small corpus (270 images, 486 texts in 7 languages).
> Last checked: 2026-10-16

Self-hosted image and text moderation in one Rust binary. CPU only, no GPU,
no Python at runtime, and nothing you check ever leaves your server.
Configurable workflows in the spirit of SightEngine, SightEngine-compatible
endpoints, and a small admin interface to choose what runs.

- **Images**: nudity, suggestive, gore, violence, weapons, drugs, alcohol,
  smoking, hate symbols, offensive gestures, and any other category you can
  describe in a sentence (zero-shot, no training).
- **Texts**: unsafe content, violence, sexual content, hate, illegal
  activity, self-harm and more, in many languages (tested in de, en, es, fr,
  it, nl, uk), plus exact rules for emails, phone numbers, IBANs, card
  numbers, links, "message me on WhatsApp", spam, scams and swear words.
- **Explicit answers**: every rejection names its category with a stable
  id, a title, a sentence you can show your users, the score and the
  threshold it crossed.
- **Pay only for what you load**: turn each tool on or off; a tool that is
  off uses no memory. From 8 MB (text rules only) to 1.55 GB (everything).

**Contents**: [Quick start](#quick-start) |
[Install](#install) |
[Admin interface](#the-admin-interface) |
[Choose what to load](#choose-what-to-load) |
[API](#api) |
[Workflows](#workflows) |
[Configuration](#configuration) |
[Command line](#command-line) |
[How it works](#how-it-works) |
[Results](#results) |
[Calibration](#calibration) |
[Security](#security) |
[Limitations](#limitations) |
[Development](#development)

## Quick start

```sh
git clone https://github.com/regisandre/openmod-rs && cd openmod-rs
cargo build --release
./target/release/openmod serve
```

On first start, openmod writes a default `openmod.toml`, then prints where
its admin interface listens and a one-time admin token:

```
  openmod admin interface: http://127.0.0.1:8089
  admin token (shown once): oma_...
```

Open that address and sign in with the token. The setup takes four steps:
choose the tools, download their models (each file is checked against a
pinned SHA-256), create an API key, done. Then:

```sh
curl -H "Authorization: Bearer $OPENMOD_KEY" --data-binary @photo.jpg \
  "http://127.0.0.1:8088/v1/image?workflow=default"
```

No browser at hand? `openmod fetch` downloads the models of the enabled
tools, and `OPENMOD_API_KEYS="key1,key2"` sets API keys from the
environment.

## Install

### Requirements

- **CPU**: x86-64 with AVX2 (x86-64-v3: most Intel CPUs since Haswell,
  2013, and AMD CPUs since Excavator, 2015; some Pentium and Celeron models
  lack it). The prebuilt ONNX Runtime needs it. On Linux,
  `grep -o -m1 avx2 /proc/cpuinfo` prints `avx2` when it is there.
- **Memory and disk**: they depend on the tools you turn on, see
  [Choose what to load](#choose-what-to-load). Everything on: 1.55 GB of
  memory and 1.26 GB of model files.
- **OS**: measured on Windows; every commit builds, passes the tests and
  starts in Docker on Linux in CI. Other platforms ONNX Runtime supports
  may work but are untested.

### From source

With Rust 1.85 or later:

```sh
cargo build --release
```

The binary is `target/release/openmod` (about 33 MB). ONNX Runtime is
downloaded and linked statically at build time, so the binary only needs
the C++ standard library at runtime.

### Docker

```sh
docker build -t openmod .
docker run -d --name openmod -v openmod-data:/data \
  -p 8088:8088 -p 127.0.0.1:8089:8089 openmod
docker logs openmod   # the admin token, at first start
```

Everything openmod writes lives in `/data`: the configuration, the models
and the state file. The admin port is published on the host's loopback
only; see [Reaching the admin interface](#reaching-the-admin-interface).

With Compose:

```yaml
services:
  openmod:
    build: .
    restart: unless-stopped
    ports: ["8088:8088", "127.0.0.1:8089:8089"]
    volumes: ["openmod-data:/data"]
volumes:
  openmod-data:
```

### systemd

```ini
[Unit]
Description=openmod moderation server
After=network-online.target

[Service]
User=openmod
WorkingDirectory=/var/lib/openmod
ExecStart=/usr/local/bin/openmod --config /var/lib/openmod/openmod.toml serve
Restart=on-failure
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
ReadWritePaths=/var/lib/openmod

[Install]
WantedBy=multi-user.target
```

`journalctl -u openmod` shows the admin token at first start.

### Reaching the admin interface

The admin interface has its own port, on loopback by default, so the API
can be public while the interface is not. From your machine, open an SSH
tunnel to the server:

```sh
ssh -L 8089:127.0.0.1:8089 you@your-server
```

then browse to `http://127.0.0.1:8089`.

Or put it behind an HTTPS reverse proxy with its own access control. The
proxy must pass the original host and scheme. With Nginx:

```nginx
location / {
    proxy_pass http://127.0.0.1:8089;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-Proto $scheme;
}
```

A proxy on the same machine reaches the interface on loopback, so `bind`
can stay on `127.0.0.1`.

## The admin interface

| Page | What it does |
|---|---|
| Tools | Turn each tool on or off and see what it costs in memory. Download missing model files, with progress. |
| Workflows | Create, edit and delete workflows. Each rule has a review and/or a reject threshold. Rules whose tool is off are marked as not checked. |
| API keys | Create one key per app, shown once, and revoke it. |
| Try it | Check a text or an image with any workflow and see the answer exactly as the API returns it. |

- **Changes apply at once**, without a restart. A reload loads only the
  models whose settings changed and keeps the others; checks in flight
  finish on the previous version.
- **The interface edits `openmod.toml` in place.** It changes only the lines
  concerned, keeps your comments, and refuses an edit that would make the
  file invalid. A hand edit is picked up at the next change made in the
  interface, or at restart.
- **Signing in** takes the admin token printed at first start.
  `openmod admin-token` prints a new one and signs every session out.
- `[admin] enabled = false` turns the interface off.

## Choose what to load

openmod is a set of tools you turn on and off. A tool that is off is not
loaded and uses no memory.

| Tool | Checks | Memory | Download | License |
|---|---|---|---|---|
| Marqo NSFW | `nudity` | 27 MB | 22 MB | Apache-2.0 |
| Owen safety | `nudity`, `gore` | 18 MB | 13 MB | MIT |
| SigLIP | `weapon`, `drugs`, `alcohol`, `smoking`, `suggestive`, `hate_symbol`, `offensive_gesture`, `violence`, `gore`, and your own | 227 MB | 207 MB | Apache-2.0 |
| GLiGuard | `text_unsafe` and 13 text categories | 667 MB | 484 MB | Apache-2.0 |
| Qwen3Guard | a second opinion on risky texts | 550 MB | 529 MB | Apache-2.0 |
| Text rules | `pii_*`, `link`, `messaging_handoff`, `spam`, `scam`, `profanity` | none | none | built in |

A tool's memory is how much the peak drops when that tool alone is turned
off. On top of the tools, the server takes 8 MB, and ONNX Runtime 53 MB
more as soon as one model is loaded.

Measured peaks for common choices (peak working set while checking,
x86-64, `threads_per_model = 2`):

| Profile | Tools on | Memory | Model files |
|---|---|---|---|
| Everything (default) | all five | 1.55 GB | 1.26 GB |
| Without Qwen3Guard | images and GLiGuard | 1.0 GB | 727 MB |
| Images only | Marqo, Owen, SigLIP | 333 MB | 243 MB |
| Nudity only | Marqo, Owen | 106 MB | 36 MB |
| Texts only | GLiGuard, Qwen3Guard | 1.34 GB | 1.01 GB |
| Text rules only | none | 8 MB | none |

Turn a tool off in the admin interface, or in `openmod.toml`:

```toml
[models.qwen3guard]
kind = "text_guard"
enabled = false
```

What happens then:
- A workflow rule whose tool is off is **not checked**. The response lists
  it in `unchecked`, the server log warns about it at startup, and the
  interface marks it. The rule and its thresholds stay in the file, ready
  for when the tool comes back.
- A workflow **none** of whose rules can be checked refuses to answer
  (`503 workflow_unavailable`) instead of accepting everything.
- With GLiGuard off and Qwen3Guard on, Qwen3Guard checks every text instead
  of giving a second opinion.
- The text rules are always on. Images only? Turn off GLiGuard and
  Qwen3Guard: 333 MB, and texts still get the rules.

## API

Authenticate with `Authorization: Bearer <key>` or `X-Api-Key: <key>`.

```sh
# Image: the raw bytes as the body
curl -H "Authorization: Bearer $KEY" --data-binary @photo.jpg \
  "http://127.0.0.1:8088/v1/image?workflow=default"

# Text: JSON
curl -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" \
  -d '{"text": "...", "workflow": "default"}' http://127.0.0.1:8088/v1/text
```

`GET /health` answers `{"status": "ok"}`, or `"degraded"` with the enabled
tools that are not ready.

### Migrating from SightEngine

The SightEngine workflow endpoints are there, as multipart forms:

| SightEngine | openmod |
|---|---|
| `POST /1.0/check-workflow.json` with `media`, `workflow` | same path, same fields |
| `POST /1.0/text/check-workflow.json` with `text`, `workflow` | same path, same fields |
| `api_user`, `api_secret` | `api_user` is ignored, `api_secret` holds an openmod API key |

The response keeps the shape of a SightEngine workflow answer
(`summary.action`, `summary.reject_prob`, `summary.reject_reason`) with
openmod's own category ids. Match on `reject_reason[].id`, never on the
English `text`.

### Response

```json
{
  "status": "success",
  "workflow": "default",
  "summary": {
    "action": "reject",
    "reject_prob": 0.97,
    "reject_reason": [
      {
        "id": "nudity",
        "text": "Nudity",
        "message": "This content contains nudity or sexual content.",
        "score": 0.97,
        "threshold": 0.8,
        "source": "marqo_nsfw"
      }
    ],
    "review_reason": []
  },
  "scores": { "nudity": 0.97, "gore": 0.01, "weapon": 0.02 },
  "time_ms": 212
}
```

- `action` is `accept`, `review` or `reject`.
- `id` is stable: use it to pick your own translated message.
- `text` and `message` are English defaults. Override them per category in
  `[labels.<id>]`.
- `unchecked` appears only when some rules of the workflow could not be
  checked because their tool is off or not ready.

### Errors

Errors share one envelope:

```json
{ "status": "failure", "error": { "type": "unsupported_media", "code": 415, "message": "..." } }
```

| `type` | HTTP | When |
|---|---|---|
| `unauthorized` | 401 | missing or wrong API key, or no key created yet |
| `bad_request` | 400 | empty body, missing field |
| `unknown_workflow` | 400 | no such workflow |
| `unsupported_media` | 415 | not JPEG, PNG, WebP or GIF |
| `corrupt_media` | 400 | the file does not decode |
| `image_too_large` | 413 | more pixels than `max_image_pixels` |
| `text_too_long` | 413 | more characters than `max_text_chars` |
| `workflow_unavailable` | 503 | the workflow has no rule for this kind of content, or none it can check on this server |
| `busy` | 503 | queue full for 30 s, retry later |

`sh scripts/smoke-test.sh safe.jpg unsafe.jpg` checks a running server
against all of the above.

## Workflows

A workflow lists the categories to check and, for each, a `reject` and/or
a `review` threshold in `[0, 1]`:
- a score at or above `reject` rejects the content;
- a score at or above `review` sends it to review;
- a category absent from the workflow is not checked at all.

Edit them in the admin interface, or in `openmod.toml`:

```toml
[workflows.profile_picture.image]
nudity = { reject = 0.8, review = 0.5 }
suggestive = { review = 0.5 }

[workflows.event_comment.text]
text_unsafe = { reject = 0.8, review = 0.5 }
hate = { reject = 0.8, review = 0.5 }
pii_phone = { review = 1.0 }
messaging_handoff = { reject = 1.0 }
```

The file ships with three: `default`, `strict` and `nudity_only`. A
request that names no workflow uses `default`, which cannot be deleted.
`openmod validate` refuses a workflow that names a category no model
produces, so a typo never silently disables a rule.

**Image categories**: `nudity`, `suggestive`, `gore`, `violence`, `weapon`,
`alcohol`, `drugs`, `smoking`, `hate_symbol`, `offensive_gesture`, plus your
own.

**Text categories**:
- from the models: `text_unsafe`, `violence`, `sexual`, `hate`, `illegal`,
  `self_harm`, `unethical`, `pii`, `privacy`, `child_safety`, `political`,
  `misinformation`, `copyright`, `regulated_advice`, `jailbreak`;
- from the rules (scores are 0 or 1): `pii_email`, `pii_phone`, `pii_iban`,
  `pii_card`, `link`, `messaging_handoff`, `spam`, `scam`, `profanity`.

`text_unsafe` is the overall verdict of the text models. The other model
categories are capped at it: they name the reason, but never flag a text
the models judge safe on their own.

### Adding an image category

Add a line to `[models.siglip.concepts]`:

```toml
fireworks = { shift = 6.0, prompts = ["a photo of fireworks", "a photo of pyrotechnics"] }
```

Then use `fireworks` in a workflow. `shift` moves the score so that 0.5
sits on the boundary measured on your data (see [Calibration](#calibration)).

### Changing the text labels

GLiGuard reads its task and label names, so you can edit them under
`[[models.gliguard.tasks]]` and map them to categories. Keep the two tasks
it was trained on (`prompt_safety`, `prompt_toxicity`) unless you measure
the change.

## Configuration

`openmod.toml`, with its defaults. Relative paths are relative to the file.

```toml
[server]
bind = "127.0.0.1:8088"
max_upload_bytes = 15_728_640     # request body limit
max_image_pixels = 50_000_000     # decoded width x height
max_text_chars = 20_000
max_concurrent = 2                # checks running inference at once
threads_per_model = 2             # CPU threads per ONNX model
# api_keys_file = "/etc/openmod/keys"   # one key per line
allow_unauthenticated = false     # loopback only
state_file = "openmod-state.toml" # admin token and API key digests

[admin]
enabled = true
bind = "127.0.0.1:8089"

[models.<name>]         # one block per tool, see openmod.toml
kind = "image_classifier" | "image_zero_shot" | "text_classifier" | "text_guard"
enabled = true

[workflows.<name>.image]
<category> = { reject = 0.8, review = 0.5 }

[labels.<category>]     # optional: your own title and message
text = "Nudity"
message = "This content contains nudity or sexual content."
```

| Environment variable | Effect |
|---|---|
| `OPENMOD_CONFIG` | the configuration file (default `openmod.toml`) |
| `OPENMOD_API_KEYS` | API keys, comma separated, at least 24 characters each |
| `OPENMOD_BIND`, `OPENMOD_ADMIN_BIND` | override `server.bind` and `admin.bind` |
| `OPENMOD_MODELS_URL` | a mirror for the derived model files |
| `RUST_LOG` | log filter, default `openmod=info,warn` |

The models, workflows and labels reload from the admin interface. The
`[server]` and `[admin]` settings take effect at restart.

## Command line

| Command | What it does |
|---|---|
| `openmod serve` | the API and the admin interface; writes a default configuration first if there is none |
| `openmod fetch [--all]` | downloads the model files of the enabled tools (`--all`: every configured tool) |
| `openmod admin-token` | prints a new admin token; the old one and its sessions stop working |
| `openmod image <file> [-w workflow]` | checks one image and prints the JSON answer |
| `openmod text <text or -> [-w workflow]` | checks a text (`-` reads stdin) |
| `openmod validate` | checks the configuration without loading any model, and lists the rules a disabled tool leaves unchecked |
| `openmod eval --images <manifest> --texts <jsonl> [-w workflow]` | scores a labelled corpus, see [Calibration](#calibration) |

Every command takes `-c <file>` (or `OPENMOD_CONFIG`).

## How it works

| Model | On disk | License | Scores |
|---|---|---|---|
| [Marqo/nsfw-image-detection-384](https://huggingface.co/Marqo/nsfw-image-detection-384) (ViT-tiny) | 22 MB | Apache-2.0 | `nudity` |
| [OwenElliott/image-safety-classifier-xs](https://huggingface.co/OwenElliott/image-safety-classifier-xs) (SwiftFormer) | 13 MB | MIT | `nudity`, `gore` |
| [SigLIP base](https://huggingface.co/google/siglip-base-patch16-224), int8 | 94 MB (+111 MB at startup) | Apache-2.0 | any category described by prompts |
| [GLiGuard](https://huggingface.co/fastino/GLiNER2-Guardrails-PII-Multi) (GLiNER2, mDeBERTa-v3) | 467 MB | Apache-2.0 | `text_unsafe` and 13 text categories |
| [Qwen3Guard-Gen-0.6B](https://huggingface.co/Qwen/Qwen3Guard-Gen-0.6B), 4-bit | 517 MB | Apache-2.0 | a second opinion on risky texts |
| Built-in rules | none | Apache-2.0 | PII, links, spam, scams, profanity |

- Every model runs on [ONNX Runtime](https://onnxruntime.ai) through
  [`ort`](https://ort.pyke.io), linked statically into the binary.
- A workflow only runs the models that score one of its categories: a
  nudity-only workflow never runs SigLIP or the text models.

**Images.**
- Two small classifiers score nudity and gore.
- SigLIP scores every other category zero-shot: each category is a list of
  English prompts in `openmod.toml`. The prompts are encoded once at
  startup, then the text half of SigLIP is freed, so an image costs one
  vision pass.

**Texts, in order.**
1. **Rules** read the raw text: emails, phones, IBANs (checksum), card
   numbers (Luhn), links, messaging handoffs, spam patterns, and scams (a
   link asking to verify a card or an account, or "payment first").
2. **Deobfuscation**: the models read the text with obfuscation undone:
   Unicode compatibility folding (fancy and fullwidth letters), invisible
   characters removed, letter floods squeezed, `t u e r` joined, leetspeak
   (`dr0p`, `b!tch`, `a$$hole`) turned back into letters.
3. **Profanity rule**: swear words in the seven languages, on the
   deobfuscated text.
4. **GLiGuard** classifies every text: safe or unsafe, plus 13 harm
   categories. It reads the label names, so labels can be edited without
   retraining.
5. **Qwen3Guard** gives a second opinion only when GLiGuard's `text_unsafe`
   is at least 0.2. Where both models score a category, the scores are
   averaged. On the bundled corpus this catches as many violations as
   always running both, while running Qwen3Guard on less than half of the
   texts.
6. **Verdict**: `text_unsafe` is the overall verdict, and every model
   category is capped at it.

**How the text models were exported.**
- `tools/export_qwen3guard.py` bakes Qwen3Guard's fixed safety prompt into
  the ONNX graph as a precomputed cache, and makes the graph return the
  logits at two positions only. One pass then gives the safe, unsafe or
  controversial verdict and the category, without text generation.
- `tools/export_gliguard.py` re-implements GLiNER2's input builder and
  checks it against the gliner2 library before exporting. Its embedding
  table is quantized to 4 bits; 4-bit matmuls cost too much accuracy on
  this encoder.
- The exported files are published with openmod's releases (`models-v1`)
  and pinned by SHA-256. `sh scripts/build-text-models.sh` rebuilds them
  from the original weights (needs [`uv`](https://docs.astral.sh/uv/) and
  about 10 minutes) if you would rather not trust the published copies.

## Results

On the corpus in `testdata/`:
- images: 270 Wikimedia Commons images in 12 groups;
- texts: 486 hand-written texts in 7 languages (296 to flag, 190 safe), and
  a 1,200-row sample of the textdetox multilingual toxicity dataset.

The thresholds were tuned on this same corpus, so read these numbers as an
upper bound. "Caught" counts a review or a reject.

**Images**, `strict` workflow, every image tool on:

| Group | Images | Caught |
|---|---|---|
| weapons | 30 | 83% |
| offensive gesture | 6 | 83% |
| alcohol | 20 | 80% |
| drugs | 20 | 80% |
| hate symbols | 15 | 80% |
| suggestive | 15 | 73% |
| gore and blood | 12 | 50% |
| smoking | 10 | 50% |
| artistic nudity (paintings, statues) | 20 | 40% |
| text in images | 12 | 17% (no OCR yet) |
| **safe**: events, crowds, stages | 80 | 0 flagged |
| **safe**: landscapes, portraits, products | 30 | 1 sent to review, 0 rejected |

**Texts**, own corpus, every text tool on:

| Category | Texts | Caught, `default` | Caught, `strict` |
|---|---|---|---|
| scam | 37 | 100% | 100% |
| illegal | 14 | 100% | 100% |
| spam | 14 | 100% | 100% |
| personal data | 22 | 95% | 100% |
| hate | 28 | 79% | 86% |
| harassment | 52 | 77% | 77% |
| sexual | 29 | 69% | 76% |
| violence | 43 | 67% | 67% |
| drugs | 43 | 51% | 53% |
| self-harm | 14 | 43% | 43% |
| **all to flag** | 296 | 76% | 78% |
| **safe texts flagged** | 190 | 5.8% (1.1% rejected) | 7.9% (4.2% rejected) |

By language, `default` workflow:

| | de | en | es | fr | it | nl | uk |
|---|---|---|---|---|---|---|---|
| caught | 69% | 84% | 79% | 76% | 74% | 81% | 69% |
| safe texts flagged | 2 of 26 | 1 of 27 | 0 of 28 | 2 of 28 | 2 of 27 | 2 of 27 | 2 of 27 |

**textdetox sample** (600 toxic, 600 non-toxic; en, de, es, it, fr, uk):

| Workflow | Toxic caught | Non-toxic flagged | Non-toxic rejected |
|---|---|---|---|
| `default` | 81% | 32% | 11% |
| `strict` | 90% | 35% | 27% |

textdetox follows another policy than openmod's defaults: its toxic class
is mostly casual swearing, which only `strict` flags (as `profanity`), and
its non-toxic class holds heated political comments, which the text models
often send to review. Ukrainian swearing is the clearest case: 40% caught
with `default`, against 78% with `strict`.

**Speed**, measured on a laptop CPU:

| Check | 2 threads per model | 8 threads per model, median (p95) |
|---|---|---|
| an image, every image tool | about 290 ms | 184 ms (237 ms) |
| a text, GLiGuard alone | about 110 ms | |
| a text, own corpus, both text tools | about 0.6 s when Qwen3Guard is consulted | 156 ms (716 ms) |

## Calibration

The bundled thresholds come from a small corpus. Measure on your own
content before relying on them:

```sh
openmod eval --images my/manifest.jsonl --texts my/corpus.jsonl --workflow strict --dump scores.jsonl
python tools/calibrate.py scores.jsonl 0.02        # images: recall at a false-positive budget, shift per category
python tools/calibrate_text.py scores.jsonl        # texts: threshold sweep, per language
python tools/category_rates.py scores.jsonl        # texts: hit rates per category
```

- `eval` prints, per category and per language, how many items were
  caught, missed or wrongly flagged.
- The `tools/` scripts read its dump, so thresholds can be tuned without
  running the models again.
- `testdata/README.md` describes the corpus format; the images and the
  third-party text samples are rebuilt by the scripts in `tools/corpus/`.

## Security

- **Inputs.** Images are decoded with hard limits on width, height, pixel
  count and memory, with EXIF orientation applied. Only JPEG, PNG, WebP and
  GIF (first frame) are accepted. URLs are never fetched, so there is no
  SSRF. Request bodies are capped by `max_upload_bytes`, and inference
  concurrency by `max_concurrent`.
- **API keys** are compared in constant time, as SHA-256 digests. An
  external key shorter than 24 characters is refused. With no key at all,
  every request is refused; the only exception is a loopback address with
  `allow_unauthenticated = true`.
- **Admin interface.**
  - Its own listener, on loopback unless you change it.
  - The admin token and the API keys it creates are 192-bit random values;
    the state file keeps only their SHA-256 digests, with owner-only
    permissions on Unix.
  - Sessions live in memory for 7 days at most, in an `HttpOnly`,
    `SameSite=Strict` cookie, `Secure` behind HTTPS. Rotating the token
    ends them all.
  - A write must carry a custom header and come from the interface's own
    origin, which a cross-site page cannot fake.
  - A strict Content Security Policy forbids inline scripts, and server
    data is always inserted as text, never as HTML.
  - Unlike some first-run wizards, the setup cannot be claimed by whoever
    reaches the page first: it needs the token from the server log.
- **Prompt injection.** Text sent to Qwen3Guard has its control tokens
  neutralized, so a message cannot close the conversation block and answer
  in the model's place. GLiGuard splits text into words first, so typing
  `[SEP_TEXT]` or `[L]` cannot inject a schema marker.
- **Model files** are downloaded over HTTPS from pinned revisions, kept
  only when their size and SHA-256 match, and written to a temporary file
  first, so a failed download never replaces a model.

Found a vulnerability? Please report it privately through GitHub's
"Report a vulnerability" on the Security tab rather than in an issue.

## Limitations

- Text inside images (posters, screenshots) is not read: there is no OCR
  yet.
- No minor or age detection. Do not rely on openmod for child-safety
  obligations.
- Zero-shot image categories are only as good as their prompts. Hate
  symbols, self-harm and violence are the hardest.
- The text models are safety guards. They judge harm, not tone: borderline
  texts (a joke, a dark mood, a drug reference) are often let through, and
  hostile political opinions are often flagged.
- Profanity is a word list. It catches casual swearing, not new slang.
- The memory figures were measured on Windows; expect similar numbers on
  Linux.

## Development

```sh
cargo test                              # the rules, decisions, config edits, access control, downloads; no model needed
OPENMOD_UI_DIR=ui openmod serve         # serve the interface from ui/ to edit it without rebuilding
sh scripts/smoke-test.sh safe.jpg unsafe.jpg   # a running server, end to end
```

- `src/`: the server. `ui/`: the admin interface, three files embedded in
  the binary, no framework and no build step.
- `tools/`: the export scripts for the text models and the calibration
  scripts. `tools/corpus/`: the scripts that build the evaluation corpus.
- `scripts/build-text-models.sh` rebuilds the derived text models;
  `scripts/publish-models.sh` publishes them as release assets.
- `testdata/`: the evaluation corpus. Only the hand-written texts are
  committed; see `testdata/README.md`.

## License

Apache-2.0 for the code (see `LICENSE`). Each model keeps its own license
(see `NOTICE`).
