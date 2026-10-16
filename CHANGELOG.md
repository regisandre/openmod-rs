# Changelog

> Status: maintained with every release.
> Last checked: 2026-10-16

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
