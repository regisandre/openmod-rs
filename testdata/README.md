# openmod-rs testdata

> Status: local evaluation corpus, not shipped
> Last checked: 2026-10-15

Evaluation data for the openmod-rs moderation engine (text and images), used to compare it with the
SightEngine behaviour it replaces. Helper scripts live in `../tools/corpus/`.

## What may be committed

| Path | Committable | Why |
| --- | --- | --- |
| `text/corpus.jsonl` | yes | Written by us, no third-party material (some entries are deliberately offensive test strings) |
| `README.md`, `.gitignore`, `images/manifest.schema.json` | yes | Our own files |
| `text/textdetox_sample.jsonl` | no, local only | Third-party dataset, OpenRAIL++ terms, redistribution restrictions on use |
| `text/polyguard_sample.jsonl` | no, local only | Third-party dataset (CC-BY-4.0, attribution needed) and contains harmful prompts |
| `images/**` (incl. `manifest.jsonl`) | no, local only | Third-party images under mixed licenses, some sensitive (weapons, drugs, hate symbols, gore) |

`.gitignore` enforces this. Re-create the ignored parts with the scripts in `../tools/corpus/`.

## text/corpus.jsonl (Part A)

Schema: `{"id","lang","text","expected","categories","context"}` with `expected` in `safe|review|reject`,
`context` in `bio|event_description|comment|chat`, categories from `hate, harassment, sexual, violence,
self_harm, spam, scam, pii, drugs, illegal` (safe items have `[]`). Source: `../tools/corpus/text/*.txt`,
compiled by `../tools/corpus/build_text_corpus.py` (which validates the vocabulary and bans long dashes).
All phone numbers, IBANs, emails and URLs are fictional or documentation examples. The `mix-*` ids are
mixed-language items. Every language also carries obfuscated items (l33t, spaced letters, emoji).

| lang | safe | review | reject | total |
| --- | --- | --- | --- | --- |
| de | 26 | 11 | 31 | 68 |
| en | 27 | 12 | 33 | 72 |
| es | 28 | 11 | 31 | 70 |
| fr | 28 | 11 | 30 | 69 |
| it | 27 | 11 | 31 | 69 |
| nl | 27 | 12 | 30 | 69 |
| uk | 27 | 11 | 31 | 69 |

Totals include the 12 `mix-*` items, counted under their dominant language. 486 rows in all.

## text/textdetox_sample.jsonl (Part B)

`textdetox/multilingual_toxicity_dataset` (OpenRAIL++): 100 toxic + 100 non-toxic per language for
en, de, es, it, fr, uk (1,200 rows, texts of 15-600 chars, random sample with seed 7). `expected` is
`reject` for toxic and `safe` otherwise; the single category `harassment` is a coarse proxy for toxic.
The original label is kept in `original`.

## text/polyguard_sample.jsonl (Part B)

`ToxicityPrompts/PolyGuardMix` (CC-BY-4.0), prompts only, first matching rows of a streamed scan:
nl 295, de 100, es 99, it 98, fr 97 (689 rows). `expected` is `reject` when `prompt_harm_label` is
`yes`. Categories are mapped from the Llama-Guard-style codes (S1, S9 -> violence, S2 -> illegal,
S3 and S12 -> sexual, S5 -> harassment, S7 -> pii, S10 -> hate, S11 -> self_harm). The raw codes stay
in `original`. Rows tagged S4 (child sexual exploitation) were dropped on purpose and never kept.
Responses were not kept. Prompts are LLM-jailbreak style, not user comments, so expect a domain gap.

## images/ (Part C)

270 images, long side at most 1024 px, JPEG q85, one folder per category, indexed by
`images/manifest.jsonl` (schema in `images/manifest.schema.json`). Almost all come from Wikimedia
Commons (API search, license filter: CC BY, CC BY-SA, CC0, public domain; no NC or ND), downloaded
by `../tools/corpus/fetch_commons_images.py`. `text_in_image` is mostly synthetic posters and chat
screenshots rendered by `../tools/corpus/make_synthetic_text_images.py` (CC0).

| category | count | expected | notes |
| --- | --- | --- | --- |
| safe_event | 80 | safe | DJs, crowds, stages, posters, food stalls |
| safe_other | 30 | safe | landscapes, adult portraits, pets, products |
| suggestive | 15 | review | adult swimwear, lingerie and runway photos |
| nudity_artistic | 20 | review | classical paintings and statues |
| weapons | 30 | review | firearms, knives, swords |
| alcohol | 20 | review | bottles, glasses, bars |
| drugs | 20 | reject | pills, syringes, cannabis, paraphernalia |
| smoking | 10 | review | cigarettes, cigars, hookah |
| gore_blood | 12 | review | slaughterhouses, historical and medical injuries, SFX make-up |
| hate_symbols | 15 | reject | historical Nazi and KKK material, Confederate flag |
| offensive_gesture | 6 | review | middle finger |
| text_in_image | 12 | reject (2 safe controls) | insults, phone numbers, IBANs, scam links |

`expected` is a provisional policy guess for an events platform; the category says what the image shows and is the
ground truth. Tune the mapping from category to label to the real policy before scoring.

Licenses (count): CC BY-SA 4.0 and 3.0 and 2.0, CC BY 2.0 to 4.0, CC0, public domain. Attribution
data (author, source URL, license) is in the manifest; attribution is required for CC BY and BY-SA
if any image is ever redistributed.

## Not done

Open Images v7 and the Roboflow weapon dataset were not used: Commons covered the targets. No
Unsplash or Pexels. Corpus images were vetted by title/description keyword filters (minors, explicit
terms) and by contact-sheet review of the sensitive folders, not exhaustively frame by frame.
