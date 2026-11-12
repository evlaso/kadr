#!/usr/bin/env bash
# Builds the site that GitHub Pages serves into target/site: the manual in
# English (at the root) and Russian (/ru/), with Eleventy from
# docs/manual and website (its config, layouts and styles), and a search
# index over the result (Pagefind). Needs Node; `--serve` then shows it on
# localhost:8000.
set -euo pipefail
cd "$(dirname "$0")/.."
site=website
[ -d "$site/node_modules" ] || npm --prefix "$site" ci --no-audit --no-fund
rm -rf target/site
"$site/node_modules/.bin/eleventy" --config="$site/eleventy.config.js" --quiet
"$site/node_modules/.bin/pagefind" --site target/site >/dev/null
echo "built target/site"
if [[ "${1:-}" == "--serve" ]]; then
  python3 -m http.server -d target/site 8000
fi
