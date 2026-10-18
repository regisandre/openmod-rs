"""Suggest per-category thresholds from an `openmod eval` dump.

For each score key, compares the images whose corpus category matches it
against the safe images, and prints the recall reachable at a given false
positive budget on safe images."""
import json
import math
import sys

dump = sys.argv[1]
budget = float(sys.argv[2]) if len(sys.argv) > 2 else 0.02
target = {  # score key -> corpus categories that should trigger it
    "nudity": ["nudity_artistic"], "suggestive": ["suggestive"], "gore": ["gore_blood"],
    "weapon": ["weapons"], "knife": ["weapons"], "alcohol": ["alcohol"], "drugs": ["drugs"],
    "cannabis": ["cannabis"], "smoking": ["smoking"], "hate_symbol": ["hate_symbols"],
    "offensive_gesture": ["offensive_gesture"], "violence": ["violence"], "self_harm": ["self_harm"],
    "underwear": ["underwear"], "swimwear": ["swimwear"], "minor": ["minors"],
}
rows = [json.loads(l) for l in open(dump, encoding="utf-8") if l.strip()]
rows = [r for r in rows if r.get("kind", "image") == "image"]  # a dump of --images and --texts holds both
safe = [r for r in rows if r["category"].startswith("safe_")]
print(f"{len(rows)} rows, {len(safe)} safe, FP budget {budget:.0%}")
print(f"{'key':<18}{'n':>4}{'thr':>11}{'recall':>8}{'safe_fp':>8}  safe max / target median  add_shift")
for key, cats in target.items():
    pos = sorted(r["scores"].get(key, 0) for r in rows if r["category"] in cats)
    neg = sorted((r["scores"].get(key, 0) for r in safe), reverse=True)
    if not pos or not any(key in r["scores"] for r in rows):
        continue
    k = int(len(neg) * budget)
    thr = neg[k] * 1.0001 if k < len(neg) else 0
    recall = sum(p >= thr for p in pos) / len(pos)
    fp = sum(n >= thr for n in neg)
    logit = lambda p: math.log(max(p, 1e-12) / max(1 - p, 1e-12))
    # Shift that puts 0.5 halfway (in logit space) between the highest safe
    # score and the median target score, or on the highest safe score when
    # the two overlap.
    lo, hi = logit(neg[0]), logit(pos[len(pos) // 2])
    add = -((lo + hi) / 2 if hi > lo else lo)
    print(f"{key:<18}{len(pos):>4}{thr:>11.5f}{recall:>8.0%}{fp:>8}  {neg[0]:.5f} / {pos[len(pos)//2]:.5f}  {add:+.2f}")
