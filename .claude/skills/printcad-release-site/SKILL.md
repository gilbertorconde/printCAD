---
name: printcad-release-site
description: "What a printCAD release and the website are made of: the printcad executable, the version in crates/app_shell/Cargo.toml, RELEASE_NOTES.md, the corpus under crates/app_shell/tests/corpus, the vX.Y.Z tag and release.yml, package-release.sh, install.sh, the application icon and app-icon.sh, the platforms CI job, and the GitHub Pages site (site/, build-site.sh, pages.yml, adding a guide). Use when touching packaging, icons, the release workflow, the corpus, site/ or when adding a guide under docs/. Not for preparing or checking a release step by step (use release-publisher)."
---

# printCAD: release anatomy and website

## Release

- The executable is `printcad` (package `app_shell`). A release is the
  app's version in `crates/app_shell/Cargo.toml`, its section in
  `RELEASE_NOTES.md`, its designs in the corpus
  (`PRINTCAD_WRITE_CORPUS=1 cargo test -p app_shell every_corpus` saves
  each recipe's part into `crates/app_shell/tests/corpus/<version>/`
  with its measures, which every later build must rebuild to; the
  numbers are never changed to follow a newer build), and a tag `vX.Y.Z`: `.github/workflows/release.yml`
  builds Linux (Ubuntu 22.04, `.tar.gz`), Windows (`.zip`) and macOS (one
  universal `.app` in a `.dmg`, ad-hoc signed) through
  `scripts/package-release.sh` and publishes them as a GitHub release.
  The Linux archive carries `install.sh` (`scripts/linux-install.sh`: a
  per-user install with its menu entry and icon). The application icon is
  drawn in `crates/app_shell/assets/icon/` (`printcad.svg`, and
  `printcad-small.svg` for 48 px and below); `scripts/app-icon.sh` renders
  the PNG, `.ico` and `.icns` the builds carry: the window icon
  (`window_icon`), the Windows program's resource (`build.rs`,
  `winresource`), the macOS bundle's, and the desktop entry's, which the
  window finds through its app id `printcad`.
  The CI's `platforms` job runs clippy, and the tests when a change can
  differ by platform, on Windows and macOS (the `printcad-testing` skill).

## Website

- The website (GitHub Pages) is `site/` (a hand-written landing page,
  `index.html`, `style.css` and `main.js` over a small WebGL2 viewer,
  `gl.js`, drawing the parts in `site/assets/models.bin`, which
  `site/tools/pack-models.py` packs from STL exports of the scripts in
  `site/tools/scenes/`; the guides' stylesheet `guides.css` and index
  `site/guides.md`) plus every guide in `docs/` turned into a page by
  `scripts/build-site.sh` (pandoc, `site/tools/`: the page template and a
  filter sending guide links to pages and source links to GitHub);
  `.github/workflows/pages.yml` builds and publishes it on pushes touching
  them. A new guide goes into the script's `guides` list and
  `site/guides.md`.
