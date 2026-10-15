"""Per-category hit rates on an `openmod eval --texts` dump: for each score
key, the share of safe texts and of violating texts at or above a few
thresholds. Use it to set per-category thresholds that keep false positives
low. Usage: category_rates.py <dump.jsonl> [<dump.jsonl> ...]
"""
import json
import sys

rows = [r for p in sys.argv[1:] for r in map(json.loads, open(p, encoding="utf-8")) if r.get("kind") == "text"]
safe = [r for r in rows if r["expected"] == "safe"]
bad = [r for r in rows if r["expected"] != "safe"]
keys = sorted({k for r in rows for k in r["scores"]})
thresholds = (0.5, 0.8, 0.9, 0.95, 0.99)
print(f"{len(rows)} texts: {len(safe)} safe, {len(bad)} violating. Cells: safe% / violating%")
print(f"{'category':<18}" + "".join(f"{f'>= {t}':>16}" for t in thresholds))
for k in keys:
    cells = []
    for t in thresholds:
        s = sum(r["scores"].get(k, 0) >= t for r in safe) / len(safe)
        b = sum(r["scores"].get(k, 0) >= t for r in bad) / len(bad)
        cells.append(f"{s:>7.1%} /{b:>6.1%}")
    print(f"{k:<18}" + "".join(f"{c:>16}" for c in cells))
