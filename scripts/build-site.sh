#!/usr/bin/env bash
# Build the website into a folder (default `_site`): the landing page (its
# script, WebGL viewer and packed models) and the guides' stylesheet from
# site/, the fonts and icon from the app, and every guide in docs/ (and
# docs/recipes/) turned into a page with pandoc.
#
#   scripts/build-site.sh [out-dir]
#
# Needs pandoc (the workflow pins 3.11). GitHub Pages serves the result (.github/workflows/pages.yml).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$root/_site}"
tools="$root/site/tools"

rm -rf "$out"
mkdir -p "$out/assets" "$out/fonts" "$out/docs/recipes"
cp "$root"/site/{index.html,style.css,main.js,gl.js,guides.css} "$out/"
cp -r "$root/site/assets/." "$out/assets/"
cp "$root/crates/app_shell/assets/icon/printcad.svg" \
  "$root/crates/app_shell/assets/icon/printcad-small.svg" "$out/assets/"
cp "$root"/crates/ui_kit/fonts/*.ttf "$root/crates/ui_kit/fonts/LICENSE.txt" "$out/fonts/"
mkdir -p "$out/docs/images"
cp "$root"/docs/images/*.png "$out/docs/images/"

# The guides, in the order the side menu lists them: "heading" lines start
# a group, the rest are paths under docs/ without ".md".
guides=(
  "heading Modelling" EDITING SKETCHER SURFACES ASSEMBLY HOLES VARIABLES TEXTURES MEASURE CAMERA
  "heading In a browser" BROWSER
  "heading Printing" PRINTING
  "heading Automating" SCRIPTING AI
  "heading Recipes"
  recipes/plate-pocket-hole recipes/bracket-fillet-holes recipes/revolved-bushing
  recipes/flange-bolt-circle recipes/variables-configurations recipes/hinged-arm
  recipes/surfaces-sewn-solid recipes/surfaces-trimmed-shade
  "heading Extending" PLUGINS WORKBENCH_GUIDE
  "heading Inside printCAD" ARCHITECTURE DOCUMENT_MODEL ROADMAP
)

# The side menu for a page at `docs/<current>.html` (`index` for the list).
nav() {
  local current="$1" up="$2" open=0 entry title
  printf '      <h4><a href="%sindex.html">All guides</a></h4>\n' "$up"
  for entry in "${guides[@]}"; do
    if [[ "$entry" == heading* ]]; then
      ((open)) && printf '      </ul>\n'
      printf '      <h4>%s</h4>\n      <ul>\n' "${entry#heading }"
      open=1
      continue
    fi
    title="$(head -1 "$root/docs/$entry.md" | sed 's/^# *//')"
    if [[ "$entry" == "$current" ]]; then
      printf '        <li><a href="%s%s.html" aria-current="page">%s</a></li>\n' "$up" "$entry" "$title"
    else
      printf '        <li><a href="%s%s.html">%s</a></li>\n' "$up" "$entry" "$title"
    fi
  done
  ((open)) && printf '      </ul>\n'
}

# The guides alone, in order, for the previous and next links.
pages=()
for entry in "${guides[@]}"; do
  [[ "$entry" == heading* ]] || pages+=("$entry")
done

# The title of guide `$1`.
title_of() {
  head -1 "$root/docs/$1.md" | sed 's/^# *//'
}

# One page: `docs/<name>.md` (or another markdown file standing as it) to
# `docs/<name>.html`; `prev` and `next` are the guides beside it.
page() {
  local name="$1" input="$2" source="$3" prev="$4" next="$5"
  local depth="${name//[^\/]/}"
  local up="" rootrel="../"
  for ((i = 0; i < ${#depth}; i++)); do
    up+="../"
    rootrel+="../"
  done
  local title
  title="$(head -1 "$input" | sed 's/^# *//')"
  local beside=()
  if [[ -n "$prev" ]]; then
    beside+=(--variable "prev_href=$up$prev.html" --variable "prev_title=$(title_of "$prev")")
  fi
  if [[ -n "$next" ]]; then
    beside+=(--variable "next_href=$up$next.html" --variable "next_title=$(title_of "$next")")
  fi
  pandoc "$input" \
    --from gfm+attributes --to html5 \
    --template "$tools/guide.html" \
    --lua-filter "$tools/links.lua" \
    --metadata source="$source" \
    --metadata pagetitle="$title" \
    --variable root="$rootrel" \
    --variable nav="$(nav "$name" "$up")" \
    --toc --toc-depth=3 \
    "${beside[@]}" \
    --output "$out/docs/$name.html"
}

for i in "${!pages[@]}"; do
  prev=""
  next=""
  ((i > 0)) && prev="${pages[i - 1]}"
  ((i + 1 < ${#pages[@]})) && next="${pages[i + 1]}"
  page "${pages[i]}" "$root/docs/${pages[i]}.md" "docs/${pages[i]}.md" "$prev" "$next"
done
page index "$root/site/guides.md" "docs/index.md" "" "${pages[0]}"
# The list's source link names the file it is made from.
sed -i 's#blob/master/docs/index.md#blob/master/site/guides.md#' "$out/docs/index.html"

echo "built $out"
