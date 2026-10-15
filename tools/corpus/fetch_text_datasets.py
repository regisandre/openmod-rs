"""Download small samples of public toxicity datasets into testdata/text/.

Usage: uv run --with datasets --with pandas --with pyarrow python -I fetch_text_datasets.py <out_dir>
"""
import json, random, sys, os
from datasets import load_dataset

out = sys.argv[1]
random.seed(7)
MAP = {"S1": "violence", "S2": "illegal", "S3": "sexual", "S5": "harassment", "S7": "pii", "S9": "violence",
       "S10": "hate", "S11": "self_harm", "S12": "sexual"}

def textdetox():
    rows = []
    for lang in ["en", "de", "es", "it", "fr", "uk"]:
        ds = load_dataset("textdetox/multilingual_toxicity_dataset", split=lang)
        tox = [r for r in ds if int(r["toxic"]) == 1 and 15 <= len(r["text"]) <= 600]
        non = [r for r in ds if int(r["toxic"]) == 0 and 15 <= len(r["text"]) <= 600]
        random.shuffle(tox); random.shuffle(non)
        for i, r in enumerate(tox[:100] + non[:100]):
            t = int(r["toxic"])
            rows.append({"id": f"textdetox-{lang}-{i:03d}", "lang": lang, "text": r["text"],
                         "expected": "reject" if t else "safe",
                         "categories": ["harassment"] if t else [], "context": "comment",
                         "original": {"toxic": t, "dataset": "textdetox/multilingual_toxicity_dataset"}})
        print("textdetox", lang, len(tox), len(non), flush=True)
    with open(os.path.join(out, "textdetox_sample.jsonl"), "w", encoding="utf-8") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")

def polyguard():
    quota = {"nl": 300, "de": 100, "fr": 100, "es": 100, "it": 100}
    got = {k: [] for k in quota}
    ds = load_dataset("ToxicityPrompts/PolyGuardMix", split="train", streaming=True)
    seen = 0
    langs_seen = {}
    for r in ds:
        seen += 1
        lg = (r["metadata"] or {}).get("language")
        langs_seen[lg] = langs_seen.get(lg, 0) + 1
        key = {"Dutch": "nl", "German": "de", "French": "fr", "Spanish": "es", "Italian": "it"}.get(lg, lg)
        if key in quota and len(got[key]) < quota[key] and 15 <= len(r["prompt"] or "") <= 1500:
            got[key].append(r)
        if all(len(got[k]) >= quota[k] for k in quota) or seen > 600000:
            break
        if seen % 50000 == 0:
            print("scanned", seen, {k: len(v) for k, v in got.items()}, flush=True)
    print("languages seen", langs_seen)
    with open(os.path.join(out, "polyguard_sample.jsonl"), "w", encoding="utf-8") as f:
        for lang, rs in got.items():
            for i, r in enumerate(rs):
                harmful = (r["prompt_harm_label"] or "").lower() == "yes"
                cats = r["prompt_safety_categories"]
                codes = [] if cats in (None, "None") else cats.split()
                if "S4" in codes:  # child sexual exploitation: never keep these rows
                    continue
                f.write(json.dumps({"id": f"polyguard-{lang}-{i:03d}", "lang": lang, "text": r["prompt"],
                    "expected": "reject" if harmful else "safe", "categories": sorted({MAP[c] for c in codes if c in MAP}), "context": "chat",
                    "original": {"prompt_harm_label": r["prompt_harm_label"], "prompt_safety_categories": cats,
                                 "source": (r["metadata"] or {}).get("source"), "dataset": "ToxicityPrompts/PolyGuardMix"}},
                    ensure_ascii=False) + "\n")
    print({k: len(v) for k, v in got.items()})

for fn in (textdetox, polyguard):
    try:
        fn()
    except Exception as e:
        print("FAILED", fn.__name__, repr(e))
