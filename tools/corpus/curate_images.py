"""Remove manifest entries (and files) by path, then renumber each category contiguously.

Usage: python -I curate_images.py <images_dir> <path1> [<path2> ...]
"""
import json, os, sys, collections

root, drop = sys.argv[1], set(sys.argv[2:])
mp = os.path.join(root, "manifest.jsonl")
rows = [json.loads(l) for l in open(mp, encoding="utf-8")]
keep = []
for r in rows:
    if r["path"] in drop:
        os.remove(os.path.join(root, r["path"]))
    else:
        keep.append(r)
n = collections.Counter()
tmp = []
for r in keep:
    cat = r["category"]; n[cat] += 1
    new = f"{cat}/{cat}_{n[cat]:03d}.jpg"
    if new != r["path"]:
        os.replace(os.path.join(root, r["path"]), os.path.join(root, new + ".tmp"))
        tmp.append(new)
    r["path"] = new
for new in tmp:
    os.replace(os.path.join(root, new + ".tmp"), os.path.join(root, new))
open(mp, "w", encoding="utf-8", newline="\n").write("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in keep))
print(collections.Counter(r["category"] for r in keep))
