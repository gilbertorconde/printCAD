---
name: printcad-bench-hooks
description: "The PRINTCAD_* environment hooks for scripted, click-free runs (PRINTCAD_OPEN_FILE, PRINTCAD_BENCH_SKETCH, _EDIT, _TOOL, _SELECT, _CLICK, _SECTION, _UI, _PICTURE, _WINDOW, _TASK, _PREFS, _CHAT, PRINTCAD_WIDGET_MAP, PRINTCAD_AUTOSAVE_SECS and the rest), read once in app/bench_hooks.rs. Use when you need to open a document, select, click, run a tool or play UI steps without a person, take a picture of the view or the window for a smoke check or a pull request, time a scene, or add or change a hook. Not for the frame loop itself (use printcad-render-loop) or test layout (use printcad-testing)."
---

# printCAD: bench hooks

These hooks are part of the on-demand render loop (`printcad-render-loop`). `PRINTCAD_EXIT_AFTER_MS` and `PRINTCAD_GPU_VALIDATION` are described with the GPU invariants in `CLAUDE.md`; `PRINTCAD_EDGE_MIN_PX` / `PRINTCAD_NO_EDGES` in `printcad-render-loop`.

`PRINTCAD_OPEN_FILE` (several paths `;`-separated get a tab
each: a `.prtcad` opens there as `PRINTCAD_OPEN_DOC` would, another file
imports; with `PRINTCAD_OPEN_SAME_TAB` they all import into one scene, a
`.prtcad`'s bodies linked) / `PRINTCAD_OPEN_DOC` /
`PRINTCAD_BENCH_ORBIT` are bench hooks (`PRINTCAD_EDGE_MIN_PX` /
`PRINTCAD_NO_EDGES` are the renderer's, mesh.rs); `PRINTCAD_BENCH_SKETCH=1` opens a
constrained sketch for editing and `=pad` pads it and opens the Pad task;
`PRINTCAD_BENCH_EDIT=<feature name>` opens that feature for editing (a
sketch's session) once the kernel has been idle a moment, as a double
click on its row would;
`PRINTCAD_BENCH_POINTER=<fx>,<fy>` then holds the pointer at that
fraction of the viewport, with `PRINTCAD_BENCH_TOOL`'s tool in hand, so
a capture shows a tool's cues;
`PRINTCAD_BENCH_SELECT=<n or name>` selects a body once it has geometry,
the way a click on its tree row would, so a capture shows the selection
overlay (with no body of that name, the feature named exactly so, which
moves its body's tip as a click does); `PRINTCAD_BENCH_ISOLATE=<name>` isolates the view to the first
body whose name holds the text once it has geometry (the view mode
Isolate is); `PRINTCAD_BENCH_CHAT=<message>` sends the message, its `@` mentions
taken as picks would, in a chat with no agent behind it once the kernel
is idle (after `PRINTCAD_BENCH_SELECT`), and opens the assistant on it
with `@` typed, so a capture shows chips and the mention list;
`PRINTCAD_BENCH_TASK=appearance|placement|history|texture|print_layout|array|note|mark`
opens that application task on the first body (`note` its first note, or
a new one); `PRINTCAD_BENCH_NOTE=<number>` arms the note tool with that note's card
open; `PRINTCAD_BENCH_PREFS=<page label>`
opens Preferences on that page once; `PRINTCAD_BENCH_CLICK=<fx>,<fy>` snaps to a corner view and makes
one selection click at that fraction of the viewport, logging what the
pick, the edge test and the face hover saw and what got selected, and with
`PRINTCAD_BENCH_TOOL=<tool id>` then runs that tool on the selection as a
toolbar click would and logs every feature's rebuild error (frame.rs);
`PRINTCAD_BENCH_SECTION=<aim>[@<offset>][!][^<way>][~<body>]` turns the clipping plane on
once there is geometry (after the click hook's click), aimed `x`, `y`,
`z`, `view`, `face` (the selected flat face, else waiting for a click) or
`points:x,y,z;x,y,z;x,y,z` or `saved:<name>`, moved along its normal, `!` flipped, `^0`/`^1` a
wedge's second plane, the body named after `~` left uncut;
`PRINTCAD_BENCH_UI=<steps>` plays pointer steps into the window once
the scene has settled (`ui/bench_ui.rs`: `click:x,y`, `right:x,y`,
`move:x,y`, `wait:n`, `;`-separated, logical points; `view:fx,fy` a
click at that fraction of the viewport, handed to the active bench as
its clicks are, `type:text` typed into the field with the keys, `key:name`
a key, `Shift+D` with its modifiers, `press:x,y`/`release:x,y` the
primary button held and let go, the `move`s between them a drag), and a
picture
waits for them, so a capture shows a panel in a state only clicks reach;
`PRINTCAD_BENCH_PICTURE=<path>` writes one picture of the view there
once the scene settles (the renderer's scene readback, `request_capture`),
and `PRINTCAD_BENCH_WINDOW=<path>` one of the whole window, panels and
menus with it (`CaptureKind::Window`);
`PRINTCAD_AUTOSAVE_SECS` shortens the autosave wait (`app/recovery.rs`:
copies of edited tabs in the data folder's `recovery/`, taken away when
saved, closed or on exit, offered on the start page after a crash);
`PRINTCAD_BENCH_WORKBENCH=<bench id>` switches to that bench once the
document has a body (so `PRINTCAD_BENCH_TOOL` reaches its tools);
`PRINTCAD_WIDGET_MAP=<path>` writes where the toolbar's buttons and
groups are (`ui/widget_map.rs`: `button:<tool or command id>`,
`arrow:<id>` its variant chevron, `variant:<tool>:<variant>` a row of
the open list, `group:<group id>`, physical pixels from the window's corner) whenever
they move, so a scripted check clicks by id;
`PRINTCAD_BENCH_REPAIR=1` asks for the repair of every broken body once,
`PRINTCAD_BENCH_CONVERT=1` the conversion of every mesh body. Every
`PRINTCAD_BENCH_*`, `PRINTCAD_OPEN_*` and `PRINTCAD_EXIT_AFTER_MS` hook is
read once at start into `app/bench_hooks.rs`'s `BenchHooks` and played by
`drive_bench_hooks`; any of them skips the start page and leaves the
user's kept session alone.
