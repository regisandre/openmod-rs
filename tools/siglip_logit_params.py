"""Read SigLIP's logit_scale and logit_bias from the upstream safetensors
with two HTTP range requests, without downloading the whole checkpoint."""
import json
import struct
import sys
import urllib.request

repo = sys.argv[1] if len(sys.argv) > 1 else "google/siglip-base-patch16-224"
url = f"https://huggingface.co/{repo}/resolve/main/model.safetensors"


def fetch(start, end):
    req = urllib.request.Request(url, headers={"Range": f"bytes={start}-{end}"})
    with urllib.request.urlopen(req) as r:
        return r.read()


n = struct.unpack("<Q", fetch(0, 7))[0]
header = json.loads(fetch(8, 8 + n - 1))
for key in ("logit_scale", "logit_bias"):
    meta = header[key]
    a, b = meta["data_offsets"]
    raw = fetch(8 + n + a, 8 + n + b - 1)
    assert meta["dtype"] == "F32", meta
    print(key, struct.unpack("<f", raw)[0])
