# Changelog

> Status: maintained with every release.
> Last checked: 2026-10-27

## 0.6.3 - 2026-10-27

- **Supply chain**: CI runs `cargo deny check` (RustSec advisories,
  permissive licenses only, no unknown registry or git source); the policy is
  in `deny.toml`.
- **Contributors**: the integration tests build as one test binary
  (`tests/it/`), and image and QR decoding are optimized in the dev profile,
  so `cargo test` runs several times faster.

## 0.6.2 - 2026-10-26

Hardening of the request path.

- **Bounded memory and work per request**: an image is cropped to its centred
  square before it is resized, a text is bounded after normalization as well
  as before it, and a check keeps its place in the queue until its work ends.
- **Word lists**: a word stretched on purpose ("fuuuck") is also read with its
  repeated letters collapsed.
- **Scores**: a score that is not a number no longer replaces a real one.
- **Dependencies**: `webpki-root-certs` 1.0.9 to 1.0.11.

## 0.6.1 - 2026-10-22

- **Compromised eMudhra CA removed from the trust roots**: `webpki-roots`
  1.0.9 to 1.0.10 and `rustls-webpki` 0.103.15 to 0.103.16. Eight eMudhra
  intermediate CA keys were disclosed as compromised on 2026-10-20, and the
  `emSign ECC Root CA - G3` root that signed them is gone from the bundled
  roots. openmod downloads its models over HTTPS with these roots, so a
  forged certificate for the model host would have been accepted before.

## 0.6.0 - 2026-10-19

A program can now use openmod as a library with workflows that live
somewhere other than `openmod.toml`: see "Use openmod as a library" in the
README.

- **Compile a workflow**: `Engine::compile_workflow(&spec, &concepts)`
  validates a `WorkflowSpec` (a `[workflows.<name>]` block, or the same
  object as JSON) with the checks of `openmod validate` and in the same words,
  and returns a `CompiledWorkflow`, or every `Problem` found. Both go through
  one function, so the same mistake gives the same text. A compiled workflow
  needs at least one rule (a configuration file may still hold a workflow
  without rules, which answers `workflow_unavailable`), and a custom category
  cannot take the name of one that exists.
- **Check against a compiled workflow**: `Engine::check_image_with` and
  `check_text_with` take the name the report carries, since a compiled
  workflow has none. `check_image` and `check_text` run the same code on the
  workflows of the configuration. `unchecked` and `workflow_unavailable` keep
  their meaning, worked out against what the engine has ready.
- **Image categories added at run time**: a zero-shot model block takes
  `keep_text_encoder = true` (default `false`), which keeps the text half of
  SigLIP, about 111 MB, in memory instead of freeing it after startup. A
  workflow compiled with custom categories (prompts, negatives and a `shift`)
  then carries their embeddings, and checking it costs one vision pass. A
  model that frees its text half, or that is not ready, gives a problem that
  says so. A category scores like the same one in the configuration when it
  is encoded in the same batch, that is when it is alone in its block; among
  others it can differ by a few hundredths, because the int8 text model scales
  its activations per batch. A workflow compiled by a model whose embeddings
  have another size is refused when checked (`CheckError::Incompatible`).
- **A stored form for compiled workflows**: a `CompiledWorkflow` is `Clone`,
  `Send` and `Sync`, and writes as JSON, `{"format": 1, "spec": ...,
  "concepts": [...]}`, with each embedding as base64 of little-endian f32.
  Reading refuses another `format`, an embedding that is cut short, has not
  the declared size or holds a NaN or an infinite value.
- **Shipped workflows and categories**: `openmod::catalog` exposes the
  workflows of the default configuration (`shipped_workflows`) and the
  category catalogue (`categories(&config)`, `shipped_categories`): for each
  category its id, kind, title, English message and the tools that score it.
- **The version**: `openmod::VERSION` is the version of the crate, for a
  program that embeds openmod and reports which one it runs.
- **Changed**: a rule whose `review` threshold is above its `reject` threshold
  is refused, in `openmod.toml` and when compiling: every score that reaches
  `review` already rejects, so such a rule never sent anything to review.
  Lower its `review`, or remove it. The shipped workflows are not concerned.
- `WorkflowSpec` now lives in `openmod::workflow` (`openmod::config` still
  exports it), `CheckError` has a variant `Incompatible`, and `ZeroShot::run`
  is split into `embed` and `score` so that one image pass serves every
  category.
- **Fixed**: every refusal of the API comes in the error envelope. A body
  over `max_upload_bytes` is `body_too_large` (413); a text that is not JSON
  or has no `text` is `bad_request` (400, it was a plain-text 422); a text
  sent without `Content-Type: application/json` is `unsupported_media`; a
  SightEngine form that is not multipart is `bad_request`. They all came in
  plain text before. `/v1/text` and `/v1/image` check the key first.
- **Fixed**: an image wider or taller than 16,384 pixels was refused as
  "larger than 0 pixels"; the message now states both limits.
- **Fixed**: a token shaped like an IBAN whose check digits were digits of
  another script (Devanagari, Arabic-Indic, fullwidth) crashed the check with
  a 500, in a text and in the text or QR code of an image. Such a token is not
  an IBAN: the rule now takes ASCII digits only.
- README: what a first install needs before the first command, the steps
  of the Quick start and of the systemd install from a release archive, a
  Compose memory limit that fits the default tools, the HTTP answers that
  come without the error envelope, and the calibration manifest format.
- `rust-version` is 1.92, what `ort` 2.0.0-rc.14 needs (it said 1.85).
- `tools/calibrate.py` reads a dump of `--images` and `--texts` together
  instead of failing on its text rows.
- A `.dockerignore`: `docker build` no longer sends `target/`, the models
  and the corpus to the builder.

## 0.5.0 - 2026-10-16

- **Removed** `model_sessions` and the per-model `sessions` of 0.4.0.
  Loading each model several times gave 3.2 images a second instead of 3.0
  on the test machine, for 3.5 times the memory: the machine saturates
  before the models do. A configuration that sets either is refused; drop
  the line.
- Kept: a check that panics no longer stops its model. Its lock is taken
  over by the next check instead of failing every check after it.
- README: a [Sizing](README.md#sizing) section, the CPU and memory to plan
  for a given flow of images and texts.

## 0.4.0 - 2026-10-16

- **Model sessions**: `model_sessions` (default 1, at most `max_concurrent`)
  loads each ONNX model that many times, so that as many checks run it at
  once; a model's own `sessions` overrides it. Each extra session is
  another copy of the model in memory, and the admin interface's memory
  estimate counts it. The CPU limit stays `max_concurrent` checks of
  `threads_per_model` threads each.
- A check that panics no longer leaves its model's lock poisoned: the
  session goes back to its pool and the next check runs as usual.

## 0.3.1 - 2026-10-16

- Logs leave out ONNX Runtime's warnings (its errors still show): PP-OCR
  made it print two dozen lines about unused initializers at every start.

## 0.3.0 - 2026-10-16

What is written on an image now counts: the text of posters and
screenshots, and QR codes, are checked like a text.

- **OCR** (`image_ocr`, PP-OCRv5 mobile, Apache-2.0): reads the text of an
  image. That text goes through everything a text goes through, and each
  category comes back with a `text_` prefix: `text_link_adult`,
  `text_extremism`, `text_pii_phone` (`text_unsafe` keeps its name).
- **QR codes** (`qr_reader`): reads the QR codes of an image. What they hold
  goes through the text rules and the domain lists, with a `qr_` prefix
  (`qr_link_gambling`); `qr_code` scores any code at all.
- **Domain lists** (`domain_list`): a link to a listed site scores the list's
  category, `link_adult`, `link_gambling` or `link_unsafe`, in texts and in
  what the OCR and the QR reader find. Three of HaGeZi's DNS blocklists
  (GPL-3.0) ship: NSFW, Gambling, and the Threat Intelligence Feeds for
  malware, phishing and scam sites. They download with the models and are
  fetched again every `lists_refresh_hours` (24). A listed public suffix,
  or a listed domain that hosts other people's sites, only matches itself,
  and `allow` names the domains a list is wrong about.
- **`extremism`**: a rule for the slogans, codes and names of extremist
  movements and terrorist organisations, in the seven languages.
- A category read out of an image takes its title and message from the
  text category: "Text in this image links to an adult website."
- The `default` image workflow refuses an extremist slogan and a link or QR
  code to an adult or unsafe site, and sends gambling links to review. The
  `strict` one also sends the text of an image through the text models
  (`text_unsafe`, `text_hate`...), to review only, and flags phone numbers,
  emails, IBANs and any QR code.
- The admin interface shows, for a tool that is off, the rules it would let
  run again.
- The memory estimate no longer counts ONNX Runtime when only the QR reader
  and the lists are on.

## 0.2.1 - 2026-10-16

- Logs without colour codes when stderr is not a terminal: `docker logs`
  and journald no longer show raw escape sequences.
- `openmod fetch` prints one line per file outside a terminal instead of a
  hundred progress updates.

## 0.2.0 - 2026-10-16

Tested against 1,911 real event posters and party photos: the image
categories were rebuilt so that ordinary party photos stop being flagged.

- **Negative prompts** for zero-shot categories (`negatives = [...]`): an
  image closer to a look-alike than to the category scores low. Dancers with
  raised arms no longer read as a fight, hoods as the Ku Klux Klan.
- **`[combine]`**: a category several models score can take their mean
  instead of the highest score. The shipped configuration averages the two
  nudity models.
- **New image categories**: `underwear`, `swimwear`, `minor`, `self_harm`,
  `knife`, `cannabis`. `weapon` is now firearms only and `drugs` hard drugs
  only; knives and cannabis have their own categories.
- All SigLIP prompts and shifts re-tuned. With `strict`, ordinary party
  photos and posters rejected went from 5.6% to 1.5%, and sent to review
  from 16% to 5.8%.
- **`openmod key add | list | revoke`**: API keys without the admin
  interface. A running server picks up keys created or revoked this way at
  its next request.
- **Docker image** published with every release:
  `ghcr.io/regisandre/openmod-rs`.
- Every category of the default configuration has its own title and message.

Breaking: a workflow that relied on `weapon` for knives or on `drugs` for
cannabis must add `knife` or `cannabis`.

## 0.1.0 - 2026-10-16

First release: images and texts, workflows, SightEngine-compatible
endpoints, admin interface.
