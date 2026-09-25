# Workbench packages

A workbench package adds a workbench to printCAD without changing or
rebuilding the app: tools, feature kinds with parametric solids, task
panels, viewport drawing, commands for scripts and agents, and a
Preferences page. A package is a WebAssembly component, so one file runs
on every system printCAD runs on, and it runs sandboxed: it reaches its own
folder and nothing else unless the user allows more.

The design is [RFC 0001](rfcs/0001-wasm-workbenches.md). The complete
example is `sdk/examples/gear`, a spur gear workbench. A repository to start
a package from, with CI and releases on tags already set up, is
[PrintCAD-example-wb](https://github.com/gilbertorconde/PrintCAD-example-wb).

## Installing one

Preferences › Workbench packages installs a `.pcbench` file (Install from
a file…) or a package published on GitHub: type the repository's address
(`https://github.com/owner/repo`, or `owner/repo`) to take its latest
release, or a release's address (`…/releases/tag/v1.2.0`) to take that
one. A package installed from GitHub remembers where it came from; Check
for updates looks for a newer release (and does so at every start unless
you turn that off), and Update to … installs it, keeping the package's
data and what you allowed it. An update that holds a different package,
or whose download does not match GitHub's checksum, is refused.

The page lists what is installed, whether each loaded, turns one
off, removes it, and says what each may reach beyond its own folder:

| Capability | What it allows |
| --- | --- |
| `save_dialog` | ask you where to save a file it made (allowed unless you turn it off) |
| `helper` | run native programs the package ships, outside the sandbox |
| `network` | open network connections |

An install, an update or a removal takes effect at once: the workbench
appears in the workbench list (or leaves it), and features it owns in open
documents rebuild with the version now running. Turning a package on or
off, and what it may reach, take effect when you apply Preferences.
Packages live in
`~/.local/share/printcad/workbenches/<id>/`, each with a `data/` folder that
is the only part of the disk it sees (as `/data`).

A document made with a package keeps its features when opened without it:
the tree marks them "Needs <package> <version>", their bodies keep the
shape they were saved with, and they cannot be deleted until the package is
back.

## Writing one

A package is a Rust crate built as a `cdylib` for `wasm32-wasip2` against
the SDK (`sdk/printcad-bench-sdk`). Other languages that build WebAssembly
components can implement `crates/bench_api/wit/workbench.wit` directly;
the values it carries are the `bench_api` types as JSON.

```toml
# Cargo.toml
[lib]
crate-type = ["cdylib"]

[dependencies]
printcad-bench-sdk = { path = "…/sdk/printcad-bench-sdk" }
```

```rust
use printcad_bench_sdk::{Bench, api::*, bench, host};

#[derive(Default)]
struct Hello;

impl Bench for Hello {
    fn describe(&self) -> Registration {
        Registration {
            label: "Hello".into(),
            tools: vec![Tool {
                id: "acme.hello.wave".into(),
                label: "Wave".into(),
                behavior: ToolBehavior::Action,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn input(&mut self, input: &Input) -> bool {
        if matches!(input.event, Event::ToolActivated) {
            host::info("Hello");
            return true;
        }
        false
    }
}

bench!(Hello);
```

Every method of `Bench` has a default; implement what the workbench offers.

### The manifest

`bench.toml` sits beside the component:

```toml
id = "acme.hello"            # lowercase, reverse-domain
name = "Hello"
version = "0.1.0"
api = "printcad:workbench@0.1"
description = "Says hello"
feature_kinds = ["acme.hello.note"]   # each starts with the id
memory_mb = 256              # default 1024, at most 4096

[capabilities]
save_dialog = true
```

Tool, action and command ids start with the package id too; others are
left out with a warning.

### Building and packing

```sh
cargo build --release --target wasm32-wasip2
mkdir -p pkg/icons
cp bench.toml pkg/
cp target/wasm32-wasip2/release/hello.wasm pkg/bench.wasm
cp icons/*.svg pkg/icons/
tar czf hello.pcbench -C pkg .
```

To publish, attach the `.pcbench` file to a GitHub release. The release's
tag is its version for updates (`v0.2.0` is newer than `v0.1.0`); the
first `.pcbench` asset of the latest release is what users get.

Icons are 24×24 SVGs drawn in white (`#fff`) with a 1.5 px stroke; the app
tints them. A tool or feature names one by its file name (`gear` for
`icons/gear.svg`), or any icon of the app's own set.

## What a bench does

**Features.** A bench owns the feature kinds its manifest lists. It makes
and changes them only through `host` calls, each an ordinary edit: undone
with Ctrl+Z, recorded, sent to peers. `host::add_feature`,
`set_feature_data`, `remove_feature`, `create_body`, and
`host::call(api::calls::…)` for the rest. A feature's data is any JSON the
bench likes.

**Parameters and formulas.** `parameters` lists a feature's numbers: a key,
a JSON pointer into its data, a dimension. Those numbers take formulas
anywhere the app shows them, and the data a bench reads (`host::feature`,
the nodes it is handed) already has the formulas' values in.

**Solids.** When a feature changes, the app asks `rebuild` for a plan per
body: kernel operations (`SolidOp`: extrusions, revolutions, lofts,
booleans, fillets and the rest), each naming the feature it is for. The
kernel runs them natively; a failing op marks its feature.

**Panels.** A bench declares its task panel and Preferences page as
widgets (`Widget`): numbers, choices, toggles, text fields, buttons, pick
rows, lists, tables, groups, progress, notes. A number bound to a feature's
parameter (`bind`) takes formulas like any field of the app's own. Changes
come back as `panel_event`; OK and Cancel as `task_close`.

**Drawing.** `frame` answers everything the bench shows: the task, the
panel, the viewport's hint, badge and footer, the status bar's items,
lines, marks and labels in world space, and meshes. The app keeps it and
asks again after an event, a document or selection change, or
`host::redraw`, and moves the drawing with the view itself.

**Input.** `input` gets clicks, key presses, tool activations and actions,
with what is under the cursor and what is selected (`Pointer`: the ray,
the point and body hit, the picked face and edges, the active feature).

**Commands.** Commands in `describe` appear to Lua scripts
(`pc.acme.hello.…`), AI agents and recordings, and run in `run_command`.

**Jobs.** Long work runs as a job: `host::start_job(entry, input)` runs
`Bench::job` in an instance of its own away from the window. It reports
with `host::progress`, stops when `host::cancelled`, and a `Progress`
widget naming the job shows how far it is. Its result comes back as
`Event::JobFinished`. A job allowed `helper` may run a program from the
package's `helpers/<os>-<arch>/` folder with `host::helper`, for work
that needs every core.

## Limits

A call that draws or handles input has 25 ms, any other call 1 s; a job
has no limit and stops when the user stops it. A call over its budget, a
panic or memory past `memory_mb` stops the bench's instance, which starts
afresh (its state lost); after three such failures in a session the bench
is turned off until the app starts again.
