# openmod

> Status: alpha. Works end to end; image thresholds calibrated on 450 corpus images and 1,911 real party photos and event posters, text thresholds on 486 texts in 7 languages; text in images checked against RapidOCR and on the same 1,911 images.
> Last checked: 2026-10-18

Self-hosted image and text moderation in one Rust binary. CPU only, no GPU,
no Python at runtime, and nothing you check ever leaves your server.
Configurable workflows in the spirit of SightEngine (a hosted moderation
API), SightEngine-compatible endpoints, and a small admin interface to
choose what runs.

- **Images**: nudity, underwear, swimwear, suggestive, minors, gore,
  violence, self-harm, weapons, knives, drugs, cannabis, alcohol, smoking,
  hate symbols, offensive gestures, and any other category you can describe
  in a sentence (zero-shot, no training), with negative prompts for what
  your users post that only looks like it.
- **What is written on images**: the text of posters and screenshots (OCR)
  and QR codes, checked like a text: a link to an adult, gambling or
  phishing site, an extremist slogan, a phone number.
- **Texts**: unsafe content, violence, sexual content, hate, illegal
  activity, self-harm and more, in many languages (tested in de, en, es, fr,
  it, nl, uk), plus exact rules for emails, phone numbers, IBANs, card
  numbers, links, "message me on WhatsApp", spam, scams, swear words and
  extremist slogans, and links to 2.2 million listed adult, gambling,
  malware, phishing and scam sites.
- **Explicit answers**: every rejection names its category with a stable
  id, a title, a sentence you can show your users, the score and the
  threshold it crossed.
- **Pay only for what you load**: turn each tool on or off; a tool that is
  off uses no memory. From 19 MB (text rules only) to 1.74 GB (everything).

**Contents**: [Quick start](#quick-start) |
[Install](#install) |
[Admin interface](#the-admin-interface) |
[Choose what to load](#choose-what-to-load) |
[API](#api) |
[Workflows](#workflows) |
[Configuration](#configuration) |
[Command line](#command-line) |
[Use openmod as a library](#use-openmod-as-a-library) |
[How it works](#how-it-works) |
[Results](#results) |
[Sizing](#sizing) |
[Calibration](#calibration) |
[Security](#security) |
[Limitations](#limitations) |
[Development](#development)

**Words used here**: a *tool* is one model, reader or domain list that you
turn on or off; a *workflow* is a named set of categories, each with its
thresholds, that a request picks; the *state file* keeps the admin token and
the digests of the API keys.

## Quick start

You need git, Rust 1.92 or later ([rustup](https://rustup.rs)), a C++
toolchain for the linker (`build-essential` on Debian and Ubuntu, the
Visual Studio Build Tools on Windows), an x86-64 CPU with AVX2 (see
[Requirements](#requirements)) and internet access: the build downloads
ONNX Runtime. The release build takes a few minutes.

1. Build and start the server:

   ```sh
   git clone https://github.com/regisandre/openmod-rs && cd openmod-rs
   cargo build --release
   ./target/release/openmod serve    # target\release\openmod.exe on Windows
   ```

   It reads `openmod.toml` in the current directory (the repository ships
   one; where there is none, `serve` writes the default), then prints where
   its admin interface listens and a one-time admin token:

   ```
     openmod admin interface: http://127.0.0.1:8089
     admin token (shown once): oma_...
   ```

   Until their files are downloaded, the tools are not ready: the log warns
   about each one, `/health` answers `degraded`, and an image checked with
   the `default` workflow gets `503 workflow_unavailable`.

2. Open that address and sign in with the token. The setup takes four
   steps: choose the tools, download their files (each model file is
   checked against a pinned SHA-256), create an API key, done. The shipped
   configuration turns every tool on: 1.32 GB to download and 1.74 GB of
   memory. For images only, turn GLiGuard and Qwen3Guard off in the first
   step: 295 MB and 528 MB.

3. Check an image with the key the setup showed:

   ```sh
   export OPENMOD_KEY=om_...    # the key from step 2
   curl -H "Authorization: Bearer $OPENMOD_KEY" --data-binary @photo.jpg \
     "http://127.0.0.1:8088/v1/image?workflow=default"
   ```

   The answer is described under [Response](#response).

No browser at hand? Before `serve`, run `openmod fetch` (the files of the
enabled tools) and `openmod key add my-app` (prints a new API key). A
running server takes a new key at once, and downloaded files at its next
start or the next change saved in the admin interface.

Or run the published image:

```sh
docker run -d --name openmod -v openmod-data:/data \
  -p 8088:8088 -p 127.0.0.1:8089:8089 ghcr.io/regisandre/openmod-rs:latest
docker logs openmod   # the admin token
```

## Install

### Requirements

- **CPU**: x86-64 with AVX2 (x86-64-v3: most Intel CPUs since Haswell,
  2013, and AMD CPUs since Excavator, 2015; some Pentium and Celeron models
  lack it). The prebuilt ONNX Runtime needs it. On Linux,
  `grep -o -m1 avx2 /proc/cpuinfo` prints `avx2` when it is there.
- **Memory and disk**: they depend on the tools you turn on, see
  [Choose what to load](#choose-what-to-load). Everything on: 1.74 GB of
  memory once loaded and 1.32 GB of model files and lists; images only:
  528 MB and 295 MB. Checks in flight add to it, see [Results](#results).
- **OS**: measured on Windows; every commit builds, passes the tests and
  starts in Docker on Linux in CI. On Linux, the prebuilt ONNX Runtime
  needs glibc 2.38 or later (Ubuntu 24.04, Debian 13, Fedora 39 and later)
  to build and to run. Other platforms ONNX Runtime supports may work but
  are untested.

### From source

With Rust 1.92 or later (what `ort`, the ONNX Runtime binding, needs) and
a C++ toolchain:

```sh
cargo build --release
```

The binary is `target/release/openmod` (`openmod.exe` on Windows, about
33 MB). ONNX Runtime is
downloaded and linked statically at build time, so the binary only needs
the C++ standard library at runtime.

### Docker

Every release publishes an image for x86-64 Linux,
`ghcr.io/regisandre/openmod-rs:<version>` (and `latest`):

```sh
docker run -d --name openmod -v openmod-data:/data \
  -p 8088:8088 -p 127.0.0.1:8089:8089 ghcr.io/regisandre/openmod-rs:latest
docker logs openmod                         # the admin token, at first start
docker exec openmod openmod fetch           # or download the models from the admin interface
docker exec openmod openmod key add my-app  # an API key, printed once
```

To build it yourself: `docker build -t openmod .`, then use `openmod` in
place of `ghcr.io/regisandre/openmod-rs:latest` above.

Everything openmod writes lives in `/data`: the configuration, the models
and the state file. The admin port is published on the host's loopback
only; see [Reaching the admin interface](#reaching-the-admin-interface).

With Compose:

```yaml
services:
  openmod:
    image: ghcr.io/regisandre/openmod-rs:latest
    restart: unless-stopped
    ports: ["8088:8088", "127.0.0.1:8089:8089"]
    volumes: ["openmod-data:/data"]
    mem_limit: 3g   # every tool on (the default); 1.5g with the text models off
volumes:
  openmod-data:
```

### systemd

1. Download a release archive (binary, `openmod.toml`, README, licenses)
   from [Releases](https://github.com/regisandre/openmod-rs/releases), check
   it, and install the binary, a system user and the configuration:

   ```sh
   v=v0.6.0   # the latest release
   base=https://github.com/regisandre/openmod-rs/releases/download/$v
   curl -fLO "$base/openmod-$v-x86_64-linux.tar.gz"
   curl -fLO "$base/openmod-$v-x86_64-linux.tar.gz.sha256"
   sha256sum -c "openmod-$v-x86_64-linux.tar.gz.sha256"
   tar xzf "openmod-$v-x86_64-linux.tar.gz"
   sudo install -m 755 "openmod-$v-x86_64-linux/openmod" /usr/local/bin/openmod
   sudo useradd --system --create-home --home-dir /var/lib/openmod --shell /usr/sbin/nologin openmod
   sudo install -o openmod -g openmod -m 600 "openmod-$v-x86_64-linux/openmod.toml" /var/lib/openmod/
   ```

2. Save this unit as `/etc/systemd/system/openmod.service`:

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

3. Start it, then read the admin token it prints at first start:

   ```sh
   sudo systemctl daemon-reload
   sudo systemctl enable --now openmod
   journalctl -u openmod
   ```

### Reaching the admin interface

The admin interface has its own port, on loopback by default, so the API
can be public while the interface is not. From your machine, open an SSH
tunnel to the server:

```sh
ssh -L 8089:127.0.0.1:8089 you@your-server
```

then browse to `http://127.0.0.1:8089`.

Or put it behind an HTTPS reverse proxy with its own access control. The
proxy must pass the original host, port included (the interface refuses a
change coming from a page whose origin names another host), and the scheme
(the session cookie is `Secure` behind HTTPS). With Nginx:

```nginx
location / {
    proxy_pass http://127.0.0.1:8089;
    proxy_set_header Host $http_host;
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
| SigLIP | `weapon`, `knife`, `drugs`, `cannabis`, `alcohol`, `smoking`, `underwear`, `swimwear`, `suggestive`, `minor`, `self_harm`, `hate_symbol`, `offensive_gesture`, `violence`, `gore`, and your own | 227 MB | 207 MB | Apache-2.0 |
| GLiGuard | `text_unsafe` and 13 text categories | 667 MB | 484 MB | Apache-2.0 |
| Qwen3Guard | a second opinion on risky texts | 550 MB | 529 MB | Apache-2.0 |
| PP-OCRv5 | the text of an image, checked like a text: `text_*` | 19 MB | 13 MB | Apache-2.0 |
| QR codes | what QR codes hold, checked like a text: `qr_*`, and `qr_code` | 1 MB | none | MIT OR Apache-2.0 |
| Adult sites | `link_adult`: 90,000 domains | 2 MB | 1.4 MB, refreshed daily | GPL-3.0 |
| Gambling sites | `link_gambling`: 600,000 domains | 6 MB | 10 MB, refreshed daily | GPL-3.0 |
| Unsafe sites | `link_unsafe`: 1.6 million malware, phishing and scam domains | 16 MB | 27 MB, refreshed daily | GPL-3.0 |
| Text rules | `pii_*`, `link`, `messaging_handoff`, `spam`, `scam`, `profanity`, `extremism` | none | none | built in |

A tool's memory is how much the peak drops when that tool alone is turned
off. On top of the tools, the server takes 19 MB, and ONNX Runtime 53 MB
more as soon as one model is loaded. The domain lists are
[HaGeZi's DNS blocklists](https://github.com/hagezi/dns-blocklists) and read
the [Public Suffix List](https://publicsuffix.org) (MPL-2.0); both are
downloaded from their publishers, never shipped.

Measured peaks for common choices (peak working set of `openmod image` or
`openmod text`, models loaded and one check run, x86-64,
`threads_per_model = 2`). A profile peaks above the sum of its tools: each
model's load spike counts in full when no larger one hides it.

| Profile | Tools on | Memory | Files |
|---|---|---|---|
| Everything (default) | all | 1.74 GB | 1.32 GB |
| Without Qwen3Guard | images, OCR, QR, lists, GLiGuard | 1.19 GB | 779 MB |
| Images only | Marqo, Owen, SigLIP, OCR, QR, lists | 528 MB | 295 MB |
| Images without OCR | Marqo, Owen, SigLIP, QR, lists | 507 MB | 282 MB |
| Nudity only | Marqo, Owen | 111 MB | 36 MB |
| Texts only | GLiGuard, Qwen3Guard, lists | 1.34 GB | 1.05 GB |
| Text rules and lists | lists | 45 MB | 39 MB |
| Text rules only | none | 11 MB | none |

Turn a tool off in the admin interface, or add `enabled = false` to its
block in `openmod.toml` (the block is already in the file; keep its other
lines):

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
- The OCR and the QR reader run only for a workflow that names one of their
  categories (`text_*`, `qr_*`): a nudity-only workflow never pays for them.
- The text rules are always on. Images only? Turn off GLiGuard and
  Qwen3Guard: 333 MB, and texts still get the rules.

## API

Authenticate with `Authorization: Bearer <key>` or `X-Api-Key: <key>`.

```sh
# Image: the raw bytes as the body
curl -H "Authorization: Bearer $OPENMOD_KEY" --data-binary @photo.jpg \
  "http://127.0.0.1:8088/v1/image?workflow=default"

# Text: JSON
curl -H "Authorization: Bearer $OPENMOD_KEY" -H "Content-Type: application/json" \
  -d '{"text": "...", "workflow": "default"}' http://127.0.0.1:8088/v1/text
```

`GET /health` needs no key and answers
`{"status": "ok", "version": "0.6.0", "problems": []}`, or `"degraded"`
with one line per enabled tool that is not ready
(`"problems": ["siglip is not ready"]`).

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
        "source": "marqo_nsfw+owen_safety"
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
- `source` names the model that gave the score, or the models whose scores
  were combined (`marqo_nsfw+owen_safety`, see
  [Combining models](#combining-models)).
- `text` and `message` are English defaults. Override them per category in
  `[labels.<id>]`.
- `unchecked` appears only when some rules of the workflow could not be
  checked because their tool is off or not ready.

### Errors

openmod's errors share one envelope:

```json
{ "status": "failure", "error": { "type": "unsupported_media", "code": 415, "message": "..." } }
```

| `type` | HTTP | When |
|---|---|---|
| `unauthorized` | 401 | missing or wrong API key, or no key created yet; checked first, except on the SightEngine endpoints, whose key travels in the form |
| `bad_request` | 400 | empty body, a text that is not JSON or has no `text`, a SightEngine form that is not multipart or lacks a field |
| `unknown_workflow` | 400 | no such workflow |
| `unsupported_media` | 415 | not JPEG, PNG, WebP or GIF, or a text sent without `Content-Type: application/json` |
| `corrupt_media` | 400 | the file does not decode |
| `body_too_large` | 413 | a request body over `max_upload_bytes` |
| `image_too_large` | 413 | more pixels than `max_image_pixels`, or a side over 16,384 pixels |
| `text_too_long` | 413 | more characters than `max_text_chars` |
| `workflow_unavailable` | 503 | the workflow has no rule for this kind of content, or none it can check on this server |
| `busy` | 503 | queue full for 30 s, retry later |
| `internal` | 500 | a bug: the server log says what happened |

`scripts/smoke-test.sh` checks a running server end to end: health, keys,
both APIs, the SightEngine endpoints, the error envelope (401, 400, 415) and
the admin interface's access control. It is in the repository only, not in
the image or the release archive, and takes an image of yours that should
pass and one that should not:

```sh
OPENMOD_KEY=om_... sh scripts/smoke-test.sh safe.jpg unsafe.jpg [api_url] [admin_url]
```

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

The file ships with three:
- `default`: rejects what is plainly unacceptable, sends doubtful cases to
  review;
- `strict`: everything openmod can check;
- `nudity_only`: for profile pictures.

A request that names no workflow uses `default`, which cannot be deleted.
`openmod validate` refuses a workflow that names a category no model
produces, so a typo never silently disables a rule. It also refuses a rule
whose `review` threshold is above its `reject` threshold: every score that
reaches `review` already rejects, so nothing would ever be sent to review.

**Image categories**:
- from the models: `nudity`, `underwear`, `swimwear`, `suggestive`,
  `minor`, `gore`, `violence`, `self_harm`, `weapon` (firearms), `knife`,
  `drugs` (hard drugs), `cannabis`, `alcohol`, `smoking`, `hate_symbol`,
  `offensive_gesture`, plus your own;
- read out of the image (scores are 0 or 1 for rules and lists): every text
  category with a `text_` prefix for its text (`text_extremism`,
  `text_link_adult`, `text_pii_phone`; `text_unsafe` keeps its name), every
  rule and list category with a `qr_` prefix for its QR codes
  (`qr_link_gambling`, `qr_pii_iban`), and `qr_code` for any code at all.

**Text categories**:
- from the models: `text_unsafe`, `violence`, `sexual`, `hate`, `illegal`,
  `self_harm`, `unethical`, `pii`, `privacy`, `child_safety`, `political`,
  `misinformation`, `copyright`, `regulated_advice`, `jailbreak`;
- from the rules (scores are 0 or 1): `pii_email`, `pii_phone`, `pii_iban`,
  `pii_card`, `link`, `messaging_handoff`, `spam`, `scam`, `profanity`,
  `extremism`;
- from the domain lists (0 or 1): `link_adult`, `link_gambling`,
  `link_unsafe`, plus the category of any list you add.

`text_unsafe` is the overall verdict of the text models. The other model
categories are capped at it: they name the reason, but never flag a text
the models judge safe on their own.

### Text and QR codes in images

A workflow that names a `text_*` category runs the OCR, one that names a
`qr_*` category the QR reader:

```toml
[workflows.posters.image]
nudity = { reject = 0.8, review = 0.5 }
text_extremism = { reject = 1.0 }
text_link_adult = { reject = 1.0 }
text_link_gambling = { review = 1.0 }
qr_link_unsafe = { reject = 1.0 }
qr_code = { review = 1.0 }
```

- The text of an image goes through everything a text goes through: the
  rules, the domain lists, and the text models when the workflow names one
  of their categories (`text_hate`). A QR code goes through the rules and
  the lists only.
- A reason read out of an image says so: "Text in this image links to an
  adult website.", "A QR code in this image links to a gambling website."

### Domain lists

A link to a listed site scores the list's category: `link_adult` in a text,
`text_link_adult` printed on an image, `qr_link_adult` behind a QR code.

- **What is a link**: a URL, `www.` and bare domains of any TLD, an
  obfuscated dot (`casino[.]com`). The domain of an email address is not a
  link.
- **Subdomains**: a listed domain covers its subdomains, except where they
  belong to someone else. A listed public suffix (`tochigi.jp`, a dynamic
  DNS provider) matches only itself, and so does a listed domain with
  three or more listed subdomains besides `www` and `m`: it hosts other
  people's sites (a blog platform), and only the listed ones match.
- **Allow**: a list is wrong now and then. The domains named in `allow`
  are never flagged by that list, nor are their subdomains:

```toml
[models.gambling_sites]
kind = "domain_list"
paths = ["lists/hagezi-gambling.txt"]
public_suffixes = "lists/public_suffix_list.dat"
category = "link_gambling"
allow = ["casinodeparis.fr"]   # a concert hall, not a casino
```

- **Which lists**: HaGeZi's NSFW, Gambling and Threat Intelligence Feeds.
  On the 2,820 domains that the venues, promoters and artists of the real
  content below link to, they flag one, a concert hall named after a
  casino.
  The Block List Project lists, used before, flagged eight, among them a
  dance music community and a hardcore festival as adult sites, and their
  phishing and malware lists caught 1 of 269 fresh OpenPhish hosts and 26
  of 579 URLhaus hosts, against 19 and 308 for the Threat Intelligence
  Feeds.
- **Refresh**: `openmod fetch` and the admin interface download the lists;
  the server asks for each one again every `lists_refresh_hours` (24), with
  `If-Modified-Since` for publishers that honour it, and reloads the lists
  that changed. A list that comes
  back empty is refused: the copy in place stays.
- **Your own list**: one domain per line, as a bare domain, a hosts-file
  line or an adblock rule. Its category is yours to name:

```toml
[models.banned_sites]
kind = "domain_list"
paths = ["lists/banned.txt"]
public_suffixes = "lists/public_suffix_list.dat"
category = "link_banned"
```

  Then use `link_banned`, `text_link_banned` or `qr_link_banned` in a
  workflow. A file openmod does not know is read as it is, never
  downloaded.

### Adding an image category

Add a line to `[models.siglip.concepts]`:

```toml
fireworks = { shift = 6.0, prompts = ["a photo of fireworks", "a photo of pyrotechnics"] }
```

Then use `fireworks` in a workflow. `shift` moves the score so that 0.5
sits on the boundary measured on your data (see [Calibration](#calibration)).

**Negatives** name what your users post that looks like the category
without being it. An image closer to a negative than to every prompt
scores low:

```toml
violence = { shift = 2.1, prompts = [
  "a photo of people fighting",
  "a photo of a violent assault",
], negatives = [
  "a photo of people dancing at a party",
] }
```

On real party photos, that one negative is what stops raised arms on a
dance floor from reading as a fight. The score is SigLIP's own
probability for the best prompt, times the probability that the image shows
the prompt rather than the best negative.

### Combining models

When several models score one category, the highest score counts by
default: any model can flag the content on its own. `[combine]` makes them
agree instead:

```toml
[combine]
nudity = "mean"
```

The shipped configuration averages Marqo and Owen for `nudity`. On 985 real
party photos and event posters, each one alone flagged shirtless dancers and
pink posters the other let through: 46 of them scored 0.5 or more with the
highest score, 14 with the mean, while the artistic nudity of the corpus
kept the same detection rate.

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
lists_refresh_hours = 24          # domain lists asked for again; 0 never

[admin]
enabled = true
bind = "127.0.0.1:8089"

[models.<name>]         # one block per tool, see openmod.toml
kind = "image_classifier" | "image_zero_shot" | "text_classifier" | "text_guard"
     | "image_ocr" | "qr_reader" | "domain_list"
enabled = true
keep_text_encoder = false  # image_zero_shot only, see "Use openmod as a library"

[workflows.<name>.image]
<category> = { reject = 0.8, review = 0.5 }

[labels.<category>]     # optional: your own title and message
text = "Nudity"
message = "This content contains nudity or sexual content."

[combine]               # optional, per category: "max" (default) or "mean"
nudity = "mean"
```

| Environment variable | Effect |
|---|---|
| `OPENMOD_CONFIG` | the configuration file (default `openmod.toml`) |
| `OPENMOD_API_KEYS` | API keys, comma separated, at least 24 characters each |
| `OPENMOD_BIND`, `OPENMOD_ADMIN_BIND` | override `server.bind` and `admin.bind` |
| `OPENMOD_MODELS_URL` | a mirror for the derived model files |
| `OPENMOD_UI_DIR` | serve the admin interface from this directory instead of the copy built in, to edit it |
| `RUST_LOG` | log filter, default `openmod=info,ort=error,warn` |

The models, workflows and labels reload from the admin interface. The
`[server]` and `[admin]` settings take effect at restart.

## Command line

| Command | What it does |
|---|---|
| `openmod serve` | the API and the admin interface; writes a default configuration first if there is none |
| `openmod fetch [--all]` | downloads the model files and domain lists of the enabled tools (`--all`: every configured tool); a running server loads them at its next start or the next change saved in the admin interface |
| `openmod admin-token` | prints a new admin token; the old one and its sessions stop working |
| `openmod key add <name>` | creates an API key and prints it once; a running server accepts it at its next request |
| `openmod key list` | lists the keys: id, first characters, creation date, name |
| `openmod key revoke <id>` | revokes a key; a running server refuses it at its next request |
| `openmod image <file> [-w workflow]` | checks one image and prints the JSON answer |
| `openmod text <text or -> [-w workflow]` | checks a text (`-` reads stdin) |
| `openmod validate` | checks the configuration without loading any model, and lists the rules a disabled tool leaves unchecked |
| `openmod eval [--images <manifest>] [--texts <jsonl>] [-w workflow] [--dump <file>]` | scores a labelled corpus (images, texts or both) and writes every score to `--dump` (default `eval-scores.jsonl`), see [Calibration](#calibration) |

Every command takes `-c <file>` (or `OPENMOD_CONFIG`).

## Use openmod as a library

openmod is also a Rust crate. A program that keeps its workflows somewhere
other than `openmod.toml`, in a database for instance, can load the engine
once, validate and compile a workflow it holds as JSON, check content
against it, and store the compiled form:

```rust
use openmod::config::{ConceptSpec, Config};
use openmod::engine::Engine;
use openmod::workflow::{CompiledWorkflow, WorkflowSpec};
use std::collections::BTreeMap;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    // Load the engine once: it reads the models and keeps them in memory.
    let engine = Engine::load(Config::load(Path::new("openmod.toml"))?, None);

    // A workflow your program keeps in its own store, as JSON...
    let spec: WorkflowSpec = serde_json::from_str(
        r#"{
            "image": { "nudity": { "reject": 0.8, "review": 0.5 }, "fireworks": { "review": 0.6 } },
            "text": { "pii_phone": { "review": 1.0 } }
        }"#,
    )?;
    // ...with an image category of its own, described in words.
    let concepts: BTreeMap<String, ConceptSpec> = serde_json::from_str(
        r#"{ "fireworks": { "shift": 6.0,
                            "prompts": ["a photo of fireworks"],
                            "negatives": ["a photo of a city at night"] } }"#,
    )?;

    // The same checks, in the same words, as `openmod validate`.
    let workflow = match engine.compile_workflow(&spec, &concepts) {
        Ok(workflow) => workflow,
        Err(problems) => {
            for problem in &problems {
                eprintln!("{problem}");
            }
            anyhow::bail!("the workflow is not valid");
        }
    };

    // A compiled workflow has no name: you give the one the report carries.
    let report = engine.check_text_with("call me on +32 470 12 34 56", "profile", &workflow)?;
    println!("{:?}", report.summary.action);
    let report = engine.check_image_with(&std::fs::read("photo.jpg")?, "profile", &workflow)?;
    println!("{}", serde_json::to_string_pretty(&report)?);

    // Store it as JSON; reading it back needs no model.
    let json = serde_json::to_string(&workflow)?;
    let _workflow: CompiledWorkflow = serde_json::from_str(&json)?;
    Ok(())
}
```

- **Compile.** `compile_workflow` makes the checks of `openmod validate`, in
  the same words: a category no tool produces, a threshold outside [0, 1], a
  rule with no threshold, a `review` above its `reject`. It adds two of its
  own: the workflow needs at least one rule (a configuration file may keep an
  empty one, which answers `workflow_unavailable`), and a custom category
  cannot take the name of a category that exists, built in or scored by a
  configured tool. It returns every problem found, each a `Problem` whose
  `message` is the text.
- **Check.** `check_image_with` and `check_text_with` take the name that the
  report carries. `unchecked` and `workflow_unavailable` mean what they mean
  for a workflow of `openmod.toml`: they are worked out against what this
  engine has ready. Like `check_image` and `check_text`, which now run the
  same code on the workflows of the configuration, they block: call them from
  a worker thread.
- **Custom image categories.** Prompts, negatives and a `shift`, as in
  [Adding an image category](#adding-an-image-category), are encoded when the
  workflow is compiled, by the first zero-shot model (in name order) that keeps
  its text half: set `keep_text_encoder = true` in its block of
  `openmod.toml`. That keeps the text half of SigLIP base, about 111 MB, in
  memory instead of freeing it after startup. Without it, compiling a workflow
  that has custom categories returns a problem that says so. The compiled
  workflow carries the embeddings, so checking it costs one vision pass, like
  any other category of the model.
  - A compiled workflow remembers the model that encoded its categories and the
    size of their embeddings (768 for SigLIP base). Where that model is not
    ready, the categories are `unchecked`; where it gives embeddings of another
    size, the check fails with `CheckError::Incompatible`: compile the workflow
    again.
  - Each category is encoded on its own, its prompts then its negatives in one
    batch. The int8 text model scales its activations once per batch, so a
    category scores exactly like the same one in `[models.siglip.concepts]`
    when it is the only one there, and can differ when it sits among others:
    on the 15 categories of the shipped configuration and 36 real images,
    15 of 540 scores moved by more than 0.05, none by more than 0.2.
    Calibrate the `shift` of a custom category on its compiled form.
- **Version.** `openmod::VERSION` is the version of the crate, for a program
  that reports which openmod it runs.
- **Store.** A compiled workflow is `Clone` (the data is shared), `Send` and
  `Sync`. It writes as `{"format": 1, "spec": ..., "concepts": [...]}`, each
  embedding as the little-endian bytes of its f32 values in base64. Reading
  refuses another `format`, an embedding that is not base64, is cut short or
  has not the declared size, and a NaN or an infinite value, which would leave
  a rule silently unflagged.
- **Templates and categories.** `openmod::catalog::shipped_workflows()`
  returns the workflows of the default configuration (`default`, `strict`,
  `nudity_only`) as specs to compile, and `catalog::shipped_categories()` the
  categories they can name: id, kind (image or text), title, English message
  and the tools that score it. `catalog::categories(&config)` lists those of
  your own configuration.

## How it works

| Model | On disk | License | Scores |
|---|---|---|---|
| [Marqo/nsfw-image-detection-384](https://huggingface.co/Marqo/nsfw-image-detection-384) (ViT-tiny) | 22 MB | Apache-2.0 | `nudity` |
| [OwenElliott/image-safety-classifier-xs](https://huggingface.co/OwenElliott/image-safety-classifier-xs) (SwiftFormer) | 13 MB | MIT | `nudity`, `gore` |
| [SigLIP base](https://huggingface.co/google/siglip-base-patch16-224), int8 | 94 MB (+111 MB at startup) | Apache-2.0 | any category described by prompts |
| [GLiGuard](https://huggingface.co/fastino/GLiNER2-Guardrails-PII-Multi) (GLiNER2, mDeBERTa-v3) | 467 MB | Apache-2.0 | `text_unsafe` and 13 text categories |
| [Qwen3Guard-Gen-0.6B](https://huggingface.co/Qwen/Qwen3Guard-Gen-0.6B), 4-bit | 517 MB | Apache-2.0 | a second opinion on risky texts |
| [PP-OCRv5 mobile det](https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_det_onnx) and [latin rec](https://huggingface.co/PaddlePaddle/latin_PP-OCRv5_mobile_rec_onnx) | 13 MB | Apache-2.0 | the text of an image |
| [rqrr](https://github.com/WanzenBug/rqrr), a port of quirc | none | MIT OR Apache-2.0, ISC | QR codes |
| [HaGeZi's DNS blocklists](https://github.com/hagezi/dns-blocklists): NSFW, Gambling, Threat Intelligence Feeds | 39 MB | GPL-3.0 | links to listed sites |
| Built-in rules | none | Apache-2.0 | PII, links, spam, scams, profanity, extremism |

- Every model runs on [ONNX Runtime](https://onnxruntime.ai) through
  [`ort`](https://ort.pyke.io), linked statically into the binary.
- A workflow only runs the models that score one of its categories: a
  nudity-only workflow never runs SigLIP or the text models.

**Images.**
- Two small classifiers score nudity and gore; their nudity scores are
  averaged (`[combine]`).
- SigLIP scores every other category zero-shot: each category is a list of
  English prompts in `openmod.toml`, plus negatives, the look-alikes that
  pull its score down. The prompts are encoded once at startup, then the
  text half of SigLIP is freed, so an image costs one vision pass.
  `keep_text_encoder = true` keeps that half (111 MB more in memory), to
  [add categories at run time](#use-openmod-as-a-library).

**Text and QR codes in images.**
- The OCR runs PaddleOCR's own pipeline on two ONNX models: the detector
  sees the image at most 960 pixels on its longer side; its probability map
  is cut at 0.3, each region becomes the smallest rectangle around it, kept
  when the map averages 0.6 inside and grown like DB's unclip; each box is
  cut out of the full-size image, straightened, read at 48 pixels high and
  decoded greedily (CTC). Lines read with less than 0.5 confidence are
  dropped, and words boxed apart on one line are joined back.
- The QR reader searches a grey copy of the image at most 2048 pixels on
  its longer side, then its negative for codes printed light on dark.
- What they read becomes a text (at most `max_text_chars`) and is checked as
  described below, its categories renamed with `text_` or `qr_`.

**Texts, in order.**
1. **Rules** read the raw text: emails, phones, IBANs (checksum), card
   numbers (Luhn), links, messaging handoffs, spam patterns, and scams (a
   link asking to verify a card or an account, or "payment first"). The
   links found are looked up in the domain lists the workflow needs, each
   domain as a 64-bit hash, so a million domains take 8 MB.
2. **Deobfuscation**: the models read the text with obfuscation undone:
   Unicode compatibility folding (fancy and fullwidth letters), invisible
   characters removed, letter floods squeezed, `t u e r` joined, leetspeak
   (`dr0p`, `b!tch`, `a$$hole`) turned back into letters.
3. **Profanity and extremism rules**: swear words, and the slogans, codes
   and names of extremist movements and terrorist organisations, in the
   seven languages, on the deobfuscated text. Codes that also mean
   something ordinary (`88`, `1488`, `ISIS`) are left out.
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
- images: 450 Wikimedia Commons images in 18 groups (the groups added in
  0.2.0 were reviewed by hand and their off-topic images left out);
- texts: 486 hand-written texts in 7 languages (296 to flag, 190 safe), and
  a 1,200-row sample of the textdetox multilingual toxicity dataset;

and on real content: 1,911 published event posters, party photos and
artist, venue and organiser pictures from the nightlife scene. 985 of them
were used to tune the image prompts and thresholds, the other 926 only to
check them.

The corpus thresholds were tuned on the corpus itself, so read those numbers
as an upper bound. "Caught" counts a review or a reject.

**Images**, every image tool on:

| Group | Images | Caught, `strict` |
|---|---|---|
| weapons (firearms, knives, swords) | 30 | 100% |
| underwear and lingerie | 15 | 100% |
| self-harm | 7 | 100% |
| alcohol | 20 | 95% |
| cannabis | 11 | 91% |
| hate symbols | 15 | 87% |
| offensive gesture | 6 | 83% |
| hard drugs | 20 | 75% |
| swimwear | 16 | 75% |
| violence | 6 | 67% |
| minors (everyday photos of children) | 39 | 64% |
| smoking | 10 | 60% |
| suggestive | 15 | 60% |
| gore and blood | 12 | 58% |
| artistic nudity (paintings, statues) | 20 | 35% |
| text in images (10 to flag, 2 controls) | 12 | 100%, controls pass |
| **safe**: events, crowds, stages | 80 | 0 flagged |
| **safe**: landscapes, portraits, products | 30 | 0 flagged |

**Real event posters and party photos**, nothing in them meant to be
refused:

| Images | `default` rejected / sent to review | `strict` rejected / sent to review |
|---|---|---|
| 985 used for tuning | 0.3% / 2.2% | 1.5% / 8.6% |
| 926 held out | 0.4% / 3.3% | 2.2% / 8.4% |

Before the 0.2.0 prompts and the nudity mean, `strict` rejected 5.6% of the
same 985 images and sent 16% to review: shirtless dancers read as nudity,
raised arms as fights, hoods as the Ku Klux Klan. Since 0.3.0 it also reads
what is written on them, and sends more to review: the phone numbers and
emails printed on posters, and the DJ names the text models find risky.
None of the 1,911 links to a listed site or carries an extremist slogan, so
the `default` rates did not move.

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
| an image, the image models (Marqo, Owen, SigLIP) | about 290 ms | 184 ms (237 ms) |
| a real poster or party photo, every image tool, OCR and QR included | 830 ms median, 1.2 s p90 | |
| a text, GLiGuard alone | about 110 ms | |
| a text, own corpus, both text tools | about 0.6 s when Qwen3Guard is consulted | 156 ms (716 ms) |

**Throughput** of one server on the same laptop (Intel i7-10875H, 8 cores,
16 threads), clients keeping its queue full, real posters and party photos
for the images, the own corpus for the texts:

| Checks | `max_concurrent`, threads per model | Per second | CPU used | CPU per check | Memory under load |
|---|---|---|---|---|---|
| images, every image tool | 1, 2 | 1.2 | 1.6 cores | 1.3 s | 0.9 GB |
| images, every image tool | 4, 1 | 2.3 | 3.1 cores | 1.35 s | 1.0 GB |
| images, every image tool | 8, 2 | 3.0 | 7.0 cores | 2.3 s | 1.2 GB |
| images, image models only | 1, 2 | 2.6 | 1.6 cores | 0.6 s | 0.5 GB |
| texts, GLiGuard | 2, 2 | 5.5 | 1.9 cores | 0.34 s | 1.0 GB |
| texts, GLiGuard and Qwen3Guard | 2, 2 | 2.1 | 2.7 cores | 1.3 s | 1.6 GB |
| texts, rules and lists | 16, 1 | 12,400 | 4.7 cores | 0.4 ms | 0.5 GB |

About three images a second is what this laptop gives with the OCR on,
however the work is split: more threads, each model loaded eight times
(tried in 0.4: 3.2 a second on 12 cores and 4.2 GB, so removed again), or
two servers side by side (3.2 together) all stop there, and the CPU per
check grows with every core added. A machine with more cores, and cores
that keep their speed under load, goes further.

Qwen3Guard only runs on texts GLiGuard finds risky, three in five in the
own corpus: on everyday content it costs much less. A server under load holds more than the peaks
of [Choose what to load](#choose-what-to-load): the images being decoded,
the ones waiting their turn, and the buffers ONNX Runtime keeps for them.

## Sizing

What a server needs, from the measurements in [Results](#results). A
check's CPU time was measured one at a time on a laptop core; a server core
runs a check about 1.3 times faster, and with every core busy each check
costs up to twice as much, as on the laptop. The tables plan between the
two: load-test the machine you pick before a big launch.

**Per check**, on one server core:

| Check | CPU time | Memory of one server |
|---|---|---|
| an image, every image tool (OCR and QR included) | 1 to 2 s | 0.5 GB loaded, 1 to 1.2 GB with checks in flight |
| an image, the image models only | 0.45 to 1 s | 0.5 to 0.6 GB |
| a text, GLiGuard | 0.25 to 0.5 s | 1 GB |
| a text, GLiGuard and Qwen3Guard | 0.35 to 1.3 s, more when many texts are risky | 1.6 GB |
| a text, rules and lists | 0.4 ms | 50 MB, 0.5 GB at 10,000 a second |

**For a steady flow**, CPU at 70%. One server handles about three images a
second with every image tool on, about four with the image models only or
with both text models, so the memory counts the servers a flow needs:

| Flow | Images, every tool | Images, models only | Texts, both text models | Texts, rules and lists |
|---|---|---|---|---|
| 100 a minute | 3 to 5 vCPU, 2 GB | 2 vCPU, 1 GB | 1 to 3 vCPU, 2 GB | 1 vCPU, 0.5 GB |
| 1,000 a minute | 24 to 48 vCPU, 8 GB | 11 to 24 vCPU, 3 GB | 8 to 31 vCPU, 7 GB | 1 vCPU, 0.5 GB |
| 100 a second | 145 to 290 vCPU, 40 GB | 65 to 145 vCPU, 15 GB | 50 to 185 vCPU, 40 GB | 1 vCPU, 0.5 GB |
| 1,000 a second | a GPU, not supported yet | 650 to 1,450 vCPU, 150 GB | 500 to 1,850 vCPU, 400 GB | 1 vCPU, 0.5 GB |

- Past one server, run several behind a load balancer. Each loads its own
  models, and `max_concurrent` times `threads_per_model` should stay near
  the cores each one gets.
- A check waits at most 30 s for its turn, then gets `503 busy`: size for
  the peaks, not for the average.
- Shared vCPU plans slow down under a steady load: take dedicated vCPU for
  a constant flow.
- Images are where the cost is, and the OCR is about half of it: a
  workflow that names no `text_*` or `qr_*` category never runs the OCR or
  the QR reader.

## Calibration

The bundled thresholds come from a small corpus and a sample of real
event photos. Measure on your own content before relying on them:

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
- An image manifest holds one JSON object per line, with a path relative
  to the manifest's directory, a category and the expected action
  (`safe`, `review` or `reject`):
  `{"path": "photos/0001.jpg", "category": "safe_mine", "expected": "safe"}`.
  `calibrate.py` counts the categories starting with `safe_` as the content
  that should pass.
- Your own content is the best calibration set. Put a few hundred of your
  users' ordinary images in a manifest with `"expected": "safe"`, run
  `eval` with a workflow that lists every category at `review = 0.5`, and
  look at the images that score highest in each category: what they show is
  what that category's `negatives` should describe. Then raise each `shift`
  until about one image in 200 of yours scores 0.5 or more, and check the
  result on images you did not look at.

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
- **Domain lists** change every day, so they are not pinned: they come over
  HTTPS from their publishers, capped at 256 MB, refused when they hold no
  entry, and swapped in whole. A bad list can only flag or miss links; it
  never runs code.
- **Text and QR codes in images**: the OCR reads at most 200 lines of an
  image scaled to 960 pixels, the QR reader at most 8 codes. The QR decoder
  can panic on a malformed image; the panic is caught and the image holds
  no code.

### Run it with ClamAV

openmod judges what an image shows; it does not look for malware. A picture
that passes moderation can still carry an exploit for an image library or
hide a payload in a polyglot file, and an upload form usually takes more
than images. Add [ClamAV](https://www.clamav.net) on the same server and
call `clamd` from your upload handler next to openmod: reject a detected
file before it is stored, whatever openmod said about it. If you also run
[openfraud](https://github.com/regisandre/openfraud-rs), put the SHA-256 of
a detected file in its `malware_hashes` list so whoever uploads it again is
quarantined.

For the server around it, [CrowdSec](https://www.crowdsec.net) and
[Fail2Ban](https://github.com/fail2ban/fail2ban) block addresses that scan
or brute-force the host, and [AIDE](https://aide.github.io/) reports any
file that changes afterwards, such as a swapped binary or model, or a
rewritten config.

Found a vulnerability? Please report it privately through GitHub's
"Report a vulnerability" on the Security tab rather than in an issue.

## Limitations

- The OCR reads Latin scripts, so not Ukrainian: point `recognizer` and
  `dictionary` at PaddleOCR's `cyrillic_PP-OCRv5_mobile_rec_onnx` to read
  Cyrillic instead. Small, blurred or heavily stylised lettering is missed.
- A domain list knows only the sites it lists: a link to a new site passes.
  It also lists sites by mistake now and then: `allow` the ones you meet.
  The extremism rule is a word list too: it catches slogans and names, not
  opinions, and flags a lecture or a news headline that names a group.
- Minors are detected zero-shot, from the whole picture: about half of the
  everyday photos of children, rarely a young-looking adult. There is no
  face or age model. Do not rely on openmod for child-safety obligations.
- Zero-shot image categories are only as good as their prompts and
  negatives. Gore, violence, drugs and artistic nudity are the hardest.
- Nudity was measured on paintings, statues, underwear and swimwear, not on
  explicit photos, which the corpus leaves out on purpose. On their own
  test sets, Marqo reports 98.6% accuracy (20,000 images) and Owen 97.8%.
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
