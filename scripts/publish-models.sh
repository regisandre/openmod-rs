#!/usr/bin/env sh
# Publishes the derived model files as the assets of the `models-v1` release,
# where `openmod fetch` and the admin interface download them from (see
# DERIVED_URL in src/tools.rs). Run from the repository root, with the files
# in ./models (from scripts/build-text-models.sh) and the GitHub CLI signed in.
set -eu

DIR="${1:-models}"
FILES="qwen3guard-q4.onnx gliguard-emb4.onnx gliguard-tokenizer.json"

sha() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

notes="Model files openmod derives from upstream models. openmod checks each one against the SHA-256 pinned in src/tools.rs before using it.

| File | SHA-256 | Derived from | License |
|---|---|---|---|"
for f in $FILES; do
  case "$f" in
    qwen3guard*) from="Qwen/Qwen3Guard-Gen-0.6B, by tools/export_qwen3guard.py" ;;
    *) from="fastino/GLiNER2-Guardrails-PII-Multi, by tools/export_gliguard.py" ;;
  esac
  notes="$notes
| \`$f\` | \`$(sha "$DIR/$f")\` | $from | Apache-2.0 |"
done

set --
for f in $FILES; do set -- "$@" "$DIR/$f"; done
gh release create models-v1 --title "Model files v1" --notes "$notes" "$@"
