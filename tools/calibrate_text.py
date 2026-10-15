"""Sweep one text score's threshold on an `openmod eval --texts` dump.

Prints, for each threshold, how many violating texts (expected reject or
review) reach it and how many safe texts do, overall and per language.

Usage: calibrate_text.py <dump.jsonl> [score_key]
"""
import json
import sys
from collections import defaultdict

dump = sys.argv[1]
key = sys.argv[2] if len(sys.argv) > 2 else "text_unsafe"
rows = [json.loads(l) for l in open(dump, encoding="utf-8") if l.strip()]
rows = [r for r in rows if r.get("kind") == "text"]


def score(r):
    s = r["scores"]
    if key == "any_model":
        return max([v for k, v in s.items() if not k.startswith("pii") and k not in ("link", "spam", "messaging_handoff")] or [0])
    return s.get(key, 0.0)


bad = [r for r in rows if r["expected"] in ("reject", "review")]
safe = [r for r in rows if r["expected"] == "safe"]
print(f"{len(rows)} texts: {len(bad)} violating, {len(safe)} safe; score = {key}")
print(f"{'thr':>5} {'caught':>8} {'safe_hit':>9}")
for t in (0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9):
    caught = sum(score(r) >= t for r in bad)
    hit = sum(score(r) >= t for r in safe)
    print(f"{t:>5.2f} {caught / max(len(bad), 1):>7.0%} {hit / max(len(safe), 1):>8.0%}")

t = float(sys.argv[3]) if len(sys.argv) > 3 else 0.3
print(f"\nper language at {t}:")
by = defaultdict(lambda: [0, 0, 0, 0])
for r in rows:
    b = by[r["lang"]]
    if r["expected"] == "safe":
        b[2] += 1
        b[3] += score(r) >= t
    else:
        b[0] += 1
        b[1] += score(r) >= t
for lang, (n_bad, caught, n_safe, hit) in sorted(by.items()):
    print(f"  {lang}: caught {caught}/{n_bad}  safe flagged {hit}/{n_safe}")
