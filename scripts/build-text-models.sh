#!/usr/bin/env sh
# Builds the three files openmod derives from upstream text models, from the
# original weights, instead of downloading the published copies:
#   - qwen3guard-q4.onnx: Qwen3Guard-Gen with its prompt head precomputed;
#   - gliguard-emb4.onnx + gliguard-tokenizer.json: GLiGuard's classifier.
# `openmod fetch` downloads them prebuilt, with everything else. Build them
# yourself to check what the published files contain, or to change the
# export. Needs `uv`, several GB of temporary downloads and about 10 minutes.
# -f: no filename globbing, so the `torch==2.*` and `gliner2[local]` specs
# below reach uv untouched.
set -euf

DIR="${1:-models}"
TOOLS="$(cd "$(dirname "$0")/../tools" && pwd)"
mkdir -p "$DIR"

sha() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

PY="uv run --quiet --python 3.12 --with torch==2.* --with transformers>=4.51 --with onnx --with onnx-ir \
  --with onnxruntime --with onnxscript --with numpy \
  --index-url https://download.pytorch.org/whl/cpu --extra-index-url https://pypi.org/simple"

build() { # script, then the files it leaves in its output directory
  script="$1"
  shift
  echo "build   $* ($script)"
  tmp="$(mktemp -d)"
  extra=""
  [ "$script" = export_gliguard.py ] && extra="--with gliner2[local]"
  # shellcheck disable=SC2086
  $PY $extra python -I "$TOOLS/$script" "$tmp"
  for f in "$@"; do
    mv "$tmp/$f" "$DIR/$f"
    echo "        $f SHA-256 $(sha "$DIR/$f")"
  done
  rm -rf "$tmp"
}

build export_qwen3guard.py qwen3guard-q4.onnx
build export_gliguard.py gliguard-emb4.onnx gliguard-tokenizer.json
echo "built in $DIR; compare the digests with src/tools.rs"
