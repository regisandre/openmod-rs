"""Compare ways of combining two text models, offline, with the engine's own
merge rule (mean per category where both score it, else the one score).

Takes two `openmod eval --texts` dumps of the same corpus, made with one
model each and a workflow listing every category. Rules (PII, off-platform
contact, spam) always count. For each combination, the threshold is set so
that at most a given share of safe texts is flagged, then the share of
violating texts caught is printed.

Usage: combine_text.py <first.jsonl> <second.jsonl> [budget=0.05] [by_language=0.1]
"""
import json
import sys
from collections import defaultdict

RULES = ("pii_email", "pii_phone", "pii_iban", "pii_card", "messaging_handoff", "spam", "link")
CERTAIN = ("pii_email", "pii_phone", "pii_iban", "pii_card", "messaging_handoff", "spam")


def load(path):
    return {r["id"]: r for r in map(json.loads, open(path, encoding="utf-8")) if r.get("kind") == "text"}


first, second = load(sys.argv[1]), load(sys.argv[2])
budget = float(sys.argv[3]) if len(sys.argv) > 3 else 0.05
lang_budget = float(sys.argv[4]) if len(sys.argv) > 4 else 0.10
ids = [i for i in first if i in second]
WATCH = sys.argv[5] if len(sys.argv) > 5 else "text_unsafe"


def model_scores(r):
    return {k: v for k, v in r["scores"].items() if k not in RULES}


def rule_hit(i):
    return any(first[i]["scores"].get(k, 0) >= 1.0 for k in CERTAIN)


def merged(i, use_second):
    a = model_scores(first[i])
    if not use_second:
        return a
    b = model_scores(second[i])
    out = dict(a)
    for k, v in b.items():
        out[k] = (a[k] + v) / 2 if k in a else v
    return out


def only_second(i):
    return model_scores(second[i])


def combo(kind, low=0.0, high=1.0, watch=None):
    watch = watch or WATCH
    def score(i):
        if kind == "first":
            s = merged(i, False)
        elif kind == "second":
            s = only_second(i)
        else:
            w = max(model_scores(first[i]).values(), default=0.0) if watch == "*" else model_scores(first[i]).get(watch, 0.0)
            s = merged(i, low <= w <= high)
        return max(s.values(), default=0.0)

    def runs(i):
        if kind == "first":
            return 0
        if kind == "second":
            return 1
        w = max(model_scores(first[i]).values(), default=0.0) if watch == "*" else model_scores(first[i]).get(watch, 0.0)
        return int(low <= w <= high)

    return score, runs


bad = [i for i in ids if first[i]["expected"] != "safe"]
safe = [i for i in ids if first[i]["expected"] == "safe"]


def threshold(f, b):
    neg = sorted((f(i) for i in safe if not rule_hit(i)), reverse=True)
    k = int(len(neg) * b)
    return neg[k] + 1e-6 if k < len(neg) else 0.0


combos = [
    ("first model only", *combo("first")),
    ("second model only", *combo("second")),
    ("both, always averaged", *combo("both")),
    ("second if first in [0.1, 0.9]", *combo("band", 0.1, 0.9)),
    ("second if first >= 0.01", *combo("band", 0.01, 1.0)),
    ("second if first >= 0.05", *combo("band", 0.05, 1.0)),
    ("second if first >= 0.2", *combo("band", 0.2, 1.0)),
]
print(f"{len(ids)} texts ({len(bad)} violating, {len(safe)} safe)")
print(f"{'combination':<34}{'second runs':>12}" + "".join(f"{f'caught @{b:.0%} safe':>18}" for b in (0.02, 0.05, 0.10)))
for name, f, runs in combos:
    row = f"{name:<34}{sum(map(runs, ids)) / len(ids):>12.0%}"
    for b in (0.02, 0.05, 0.10):
        t = threshold(f, b)
        caught = sum(rule_hit(i) or f(i) >= t for i in bad) / len(bad)
        row += f"{caught:>11.0%} (t={t:.2f})"
    print(row)

print(f"\nper language at {lang_budget:.0%} safe flagged:")
for name, f, _ in combos[:3]:
    t = threshold(f, lang_budget)
    by = defaultdict(lambda: [0, 0, 0, 0])
    for i in ids:
        b = by[first[i]["lang"]]
        hit = rule_hit(i) or f(i) >= t
        if first[i]["expected"] == "safe":
            b[2] += 1
            b[3] += hit
        else:
            b[0] += 1
            b[1] += hit
    print(f"  {name:<24}" + "  ".join(f"{l} {c}/{n} ({h}/{s})" for l, (n, c, s, h) in sorted(by.items())))
