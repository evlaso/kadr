#!/usr/bin/env bash
# Builds the site that GitHub Pages serves into target/site: the landing
# page (docs/site), the manual in English (manual/) and Russian
# (manual/ru/) from docs/manual with mdBook (docs/manual/book/book.toml;
# the Russian book reuses it through MDBOOK_* overrides), and the
# screenshots of docs/images. `--serve` then opens it on localhost:8000.
set -euo pipefail
cd "$(dirname "$0")/.."
out=target/site
rm -rf "$out"
mkdir -p "$out"
(cd docs/manual/book && mdbook build)
(cd docs/manual/book && MDBOOK_BOOK__SRC=../ru MDBOOK_BOOK__LANGUAGE=ru MDBOOK_BOOK__TITLE="Справочник kadr" \
  MDBOOK_BUILD__BUILD_DIR=../../../target/site/manual/ru MDBOOK_OUTPUT__HTML__SITE_URL=/kadr/manual/ru/ mdbook build)
cp -r docs/site/. "$out/"
mkdir -p "$out/images"
cp docs/images/*.png "$out/images/"
echo "built $out"
if [[ "${1:-}" == "--serve" ]]; then
  python3 -m http.server -d "$out" 8000
fi
