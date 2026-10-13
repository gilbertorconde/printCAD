# printCAD in a browser

printCAD runs in a web browser as well as on a desktop: the website's Try
in browser opens it, with nothing to install. It is the same app, built for
the browser, so modelling, sketches, assemblies, surfaces, imports and
exports work as they do on a desktop. This page says what differs.

## What it needs

A recent desktop browser. It draws with WebGPU where the browser has it
(Chrome and Edge), WebGL2 otherwise (Firefox, Safari). The first visit
downloads about 35 MB, which the browser keeps for the next.

The geometry kernel works in background workers, so the page stays
responsive while bodies build, several at once. Where the browser lets the
page's workers share memory, each worker also uses several threads: the
page asks for that through a small service worker, and reloads once the
first time it is opened.

## Documents

The browser keeps your documents for this site, on this computer:

- **Save** keeps the document under a name, asked the first time. The start
  page lists every document the browser keeps, with its preview; a card's
  menu (right click) deletes one, which cannot be undone.
- **File › Download a copy** saves the document as a `.prtcad` file on
  disk, which the desktop app opens too. **Open** reads any `.prtcad` file
  from disk; saving it keeps a copy in the browser.
- **Autosave** keeps a copy of each edited document every few minutes
  (Preferences › General), and the start page offers it back after the page
  was closed. Leaving the page with unsaved edits asks first.

Clearing the site's data in the browser deletes the documents it keeps:
download a copy of anything you want to keep.

Imports read the file you pick; exports, pictures, recorded scripts and
Save as script arrive as downloads.

## Scripts and packages

The script console runs Lua as on a desktop ([Scripting](SCRIPTING.md));
Scripts › Run script… runs a `.lua` file you pick.

Workbench packages ([Packages](PLUGINS.md)) install from a `.pcbench`
file or from a workbench store, and the browser keeps them for the next
visit. A store needs to serve a copy of each package's release
(`mirror` in its index), which printCAD's own store does: a page cannot
fetch a GitHub release's download. In a browser a package has no folder of
its own, no network and no helpers, and no call has a time limit.

## 6-DoF mice

In Chrome and Edge a 6-DoF mouse connects through the browser: Preferences
› Input › 6-DoF mouse › Choose… lists the pucks plugged in, and one chosen
is found again on the next visit ([Camera](CAMERA.md)).

## What stays on the desktop

- Send to slicer, which runs another program.
- The AI assistant, which runs agent programs.
- Update checks: the page is always the latest release.
- Parts linked from another printCAD file, which are read again by their
  path.
- The scripts folder: scripts run from a file you pick.
- Installing a package from a GitHub address.
- Fonts installed on the machine for sketch text: the page offers the
  bundled fonts, and text in a font a desktop kept in the
  document is made from the kept copy as anywhere.
- The command line.
