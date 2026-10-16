# Changelog

> Status: maintained with every release.
> Last checked: 2026-10-16

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
