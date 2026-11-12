---
name: printcad-scripting-agents
description: "Commands, scripts and AI agents: the command contract (core_document::command, CommandSpec, each bench's commands.rs and its add/add_on registries), the Lua scripting crate and drive_scripts, recording UI actions as calls, headless printcad --script, command prose files and docs/recipes, and the agents crate (ACP client, MCP server, the per-process socket, approvals, agent context, the assistant chat, mentions and attachments). Use when adding or changing a command or its prose, a script API, recording, a recipe, or code in crates/scripting, crates/agents, app/mcp.rs, app/agent_context.rs or ui/assistant.rs. Not for UI-only actions (use printcad-app-shell) or the bench seam (use printcad-workbench-api)."
---

# printCAD: commands, scripts and agents

The invariants in `CLAUDE.md` still bind here; this skill never weakens one.

## Commands and scripts

**Commands and scripts.** `core_document::command` is the command
contract: `CommandSpec` (id, typed `ParamSpec`s, `extra_args`), JSON
`CommandArgs` in, `CommandResult` out, `spec.check` before every call. A
bench registers commands in `configure` (ids unique across the registry)
in the order the generated reference lists them and runs them in
`Workbench::run_command` with the same context its tools get. Every
built-in bench keeps them in one `CommandTable<B>`
(`core_document::command`: every spec beside the function that runs it,
so an id is written once; registration order, `owns` and dispatch derive
from it), built from one `table(t)` per family module and kept in a
`OnceLock` in its `commands.rs`. `B` is the bench state a handler takes:
`add`/`add_by_id` for handlers needing only the context, `add_on`/
`add_on_by_id` for ones taking `&mut B` (Design, Surface, Inspection,
the Sketcher's exports, the Mesh's step results), `add_free`/`add_with`
for a handler a family's wrapper makes around a step several commands
share (the Sketcher's `editing`, `reading` and `editing_text`: load the
sketch, run the step, solve and store it; the Mesh's `on_scan`: the
scan and its starting mesh), and `include` for a family kept as a table
of its own (Inspection's `align_commands`, which tools ask `owns` of).
`run_on(bench, ..)` runs any; `run(..)`, for code holding the context
but not the bench (a panel), runs those needing no bench state. The
Sketcher and Assembly sort theirs by the `ORDER` list in `commands.rs`
(`sort_by_order`), which a test holds complete. `command::object` turns a JSON
value into arguments. Benches' tests run commands through
`core_document::testing` (the `test-support` feature, turned on only by
dev-dependencies): `ctx(doc)`, a context with no camera, and
`call_checked(bench, doc, id, args)`, a call checked against the bench's
registered spec as a script's is. The app's own are `doc.*` plus every keymap
command (`app/scripts.rs`). The `scripting` crate is Lua 5.4 (mlua,
vendored) and knows no command: a `scripting::Host` lists and runs them,
the prelude (`prelude.lua`) builds the `pc` namespace, `print`, `show`
and `help`. The console (`ui/console_view.rs`, output in the `console`
store) sends `UiCommand::RunConsole`, which queues it on the script thread
(`scripting::ScriptThread`, owning the engine; `PrintCadApp.scripting.thread`).
Each command the script calls comes back as `Event::Call` and runs on the
UI thread in `drive_scripts` (8 ms a frame, in the tab the run started in,
`in_script_tab`); `doc.rebuild` answers once the kernel is idle
(`scripting.rebuild`) rather than blocking a frame. The journal is held
(`OpJournal::hold`) from `Started` to `Finished`, so a run is one undo
step. The thread wakes the loop through `AppEvent::Script` and a busy A run that stops with an error or is stopped is taken back
(`OpJournal::roll_back`), leaving no undo step; an import inside it is a
barrier, so only what followed it is taken back.
thread counts as async work. Stop sets the engine's stop flag.
A run's chunk value comes back as `RunOutput::returned` (JSON: a table
with keys 1 to n a list, any other an object); the console shows it and
an agent's `lua` answers `{returned, printed}` (`script_answer`).
`doc.faces`/`doc.edges` (`app/edges.rs::edges_of`: kind, a point halfway
along and its direction, length, the faces' indices and names as strings)
read geometry where the body sits, the picks the edge-taking commands use.
Tools end in commands, which is what recording rests on: a bench calls
`ctx.record(id, args, result)` where a UI action ends in the code its
command runs (`HookOutcome.recorded`, carried through `PanelWriteback`
for panel hooks). The sketcher's click is `step::click`, shared with
`sketch.draw` (a shape is one call, flushed when the tool returns to rest);
drags are `step::drag`; Design records in `record_task` when a task
closes, diffing against what the command makes alone (`default_feature`);
Assembly in `record_joint`. The host maps its own UI commands in
`recorded_of` and keeps calls in `PrintCadApp.scripting.recording`
(`scripting::Recorder`, which names results and writes numbers exactly,
a double that is exactly a single as the single's digits, so a replay
meets stored values bit for bit); nothing records
while a script runs. The app's own
commands: `doc.*` (the pure ones in `scripts::document_command`, shared
with `headless.rs`), `app.*`, and every keymap command, the file ones
taking a `path` to skip their dialog. Script files: `script_library.rs`
reads `settings::scripts_dir()` every 2 s into `script.<name>` keymap
bindings (`Target::Script`), the Scripts menu, toolbar button and palette.
`printcad --script` (`headless.rs`) runs before the event loop, rebuilds
on the calling thread, logs to stderr. `docs/SCRIPTING.md`'s command
reference is generated; a test fails when it drifts
(`PRINTCAD_WRITE_DOCS=1` rewrites it). A `CommandSpec` carries `notes`,
`examples` (Lua ending in `assert`s) and `see_also` (`.note`, `.example`,
`.see_also`; a package's `bench_api::Command` the same), which the
reference prints. Every built-in family writes them as markdown, one
file beside each Rust file that makes specs (`app/scripts/catalog.md`,
`app/section_commands.md`, each bench's `commands/<family>.md`, Mesh's
`commands.md`, `steps.md` and `solids.md`, Inspection's
`align_commands.md`, Design's `generators.md`, the sketcher's
`generator/generator.md`, `project_tools`' `bench.md`: a
`## id` section per command, or one several commands share, its
paragraphs notes, a `See also:` line, `### title` and a ```lua block per
example, a `{{name}}` line a shared piece of setup), read once through
`core_document::CommandProse` and added to the specs by id after what
the code adds (`explain`, `CommandTable::explain`); ids, parameters,
`read_only`, agent access and prose code works out or adds to many
commands by rule (Design's feature and fillet notes, the joint and
surface step notes) stay in the code, and each family's test holds
every section of all its files to a registered command
(`testing::assert_prose_names_commands`);
`every_command_example_and_recipe_runs`
(`app/scripts.rs`) runs every example and every ```lua block of
`docs/recipes/*.md` from an empty document through the headless path
(`headless::run_in_empty_document`).
Commands never open a task; Design's make features through
`create_feature`, the toolbar's own path, then merge named fields into the
feature's JSON. `kernel_ogeom/tests/kernel/scripted_part.rs` runs a script through
the real benches to a solid.

## AI agents

**AI agents.** The `agents` crate knows no command either: `rpc`
(newline-delimited JSON-RPC), `acp` (`AgentChat`, a worker thread per
agent process speaking the Agent Client Protocol: `ChatCommand` in,
`ChatEvent` out), `mcp` (the server core over a `ToolHost`) and `bridge`
(`printcad --mcp`, relaying stdio to the app's socket with a header naming
the chat), and `discovery` (BM25 `search`, loose `describe` and the
one-line-per-entry `index` over plain `Entry`s, with the CAD `SYNONYMS`
table, pinned by the ranking test in `app/discovery.rs`, which builds the
entries from every command through `scripting::command_entry` and from
the guides' sections and `docs/recipes/`; the index goes into the
instructions, and the console's `help(word)` searches the same way).
`app/mcp.rs` listens on `$XDG_RUNTIME_DIR/printcad/mcp-<pid>.sock`,
one thread per client handing each tool call to the UI thread
(`drive_agent_tools`); `call` and `lua` run as script-thread jobs
(`Job::Command`/`Job::Script`, `RunKind::Agent` answering the tool), so
tab pinning, one undo step per call and Stop come from the script path.
A change waits in `PrintCadApp.agents.approvals` while its chat asks
(`asks_before_changes`); `CommandSpec::read_only` (declared by whoever
registers the command) is what never waits, and `CommandSpec::agent`
(`AgentAccess`) what an agent may never run (`app.quit`, `tab.close`,
`doc.set_agent_rules`) or must always have allowed, one `call` at a time
(files, import/export, slicer, undo/redo, tabs); `agent_check` enforces it
for `call` before it is held and for every command a `lua` script calls.
`app/agent_context.rs` is what an agent is told: the MCP instructions,
built per connection on the UI thread (the relay asks for them, and for
the `printcad://rules` and `printcad://context` resources, with requests
no client can name), the `context` tool, the prompts, and the user's rules
(`AiSettings.rules` for every document, `Document::agent_rules` saved with
one, op `SetAgentRules`), which also go with a chat's first prompt and with
the next one after they change. `app/chats.rs` keeps
`PrintCadApp.agents.chats`, each started with the relay as its MCP server
(every tool `always_load`, sent as `_meta."anthropic/alwaysLoad"`, so a
client that defers tools behind a search has them in its first turn;
`read_only` becomes `readOnlyHint`);
`ui/assistant.rs` draws them and answers with `UiCommand`s (the agent's
text as markdown, a copy button on a hovered message, the header's jumps
between the user's messages kept in `ChatScroll` from the last frame's
layout); a message
sent while the agent is on a turn waits in `Chat::queued` (edited or
dropped from the panel) and goes when the chat is ready again, one per
turn, the rules added as it goes; Stop sets `Chat::held` until the user
sends again or resumes; a call of
printCAD's own tools draws as `agent_context::tool_label` (the call's
`description` argument, which the instructions ask for, else the command
or the script it runs), from the arguments ACP sends as `rawInput`. A chat belongs
to the tab it started in (`Chat::tab`; its tool calls run there through
`submit_script_in`), and once it has a session id and the tab a file,
`persist_chats` keeps `{agent, session, title, folder, used}` with the file in the
app's `chats.json` (`app/chat_store.rs`); opening the file
(`restore_chats`) brings them back `Resting`, and the first time one is
shown `wake_chat` starts its agent with `session/load`
(`AgentChat::start`'s `resume`), which replays the conversation.
`chats.json` holds `documents` (open chats by file) and `closed` (by
file, each with `used`, milliseconds since the epoch); `close_chat`
moves a chat to `closed` (`chat_store::close`), and the history button
beside the chat tabs (`ui/assistant.rs::history_list`, rows from
`PrintCadApp::chat_history` over `Agents.history`, read again by
`refresh_chat_history`) reopens one on the tab on screen
(`reopen_chat`, `Chat::resting`, kept with that tab's file from then
on) or forgets it (`forget_chat`). Chats of a tab with no file stay in
`Agents.closed_unsaved` for the run (closed ones, and open ones whose
tab closes, `let_go_of_tab_chats`) and `persist_chats` moves them into
the store at the first save. The agent's
session options (`acp::SessionOption`: `configOptions`, or the older
`modes`/`models`) draw as the bar under the input; a change shows at once
and is put back if the agent refuses, an answer an agent-side update has
overtaken is dropped, and the choice is kept per agent in
`AgentSettings.choices`, put to each new chat in the agent's own order
(`put_choices`). A prompt's attachments (`acp::Attachment`, held on the
`Chat` until sent) become content blocks in the chat thread
(`prompt_blocks`, by the agent's `promptCapabilities`): pictures as
images, small UTF-8 files embedded, the rest as `resource_link`s. They
come from the "+" menu (`FileDialogKind::Attach`, `attach_view` over
`view_png`), a paste of file paths, or a drop on the panel (winit delivers
drops on X11 only). Typing `@` in the box completes mentions
(`app/mentions.rs`, through `ui_kit::completion::Completion`, the
formula fields' dropdown made general): every open file and the chat
tab's selection, `PrintCadApp::mention_options`. A pick puts its token in
the text, drawn as a chip by the box's layouter and taken away whole by
Backspace or Delete, and records `UiCommand::AddMention`, whose block
(`mention_block`: a file's bodies and features, a selection's faces with
`FaceRef::surface` and `Document::face_maker`, edges, bodies and tree
features) is taken then into `Chat::mentions`; a message keeps the ones
its text holds (`mentions::in_text`) through `Queued` and
`ChatEntry::User`, and each goes as an `Attachment::Text` block,
`<printcad:mention>{json}</printcad:mention>`, which `take_blocks` reads
back out of a replayed message. A new kind of mention is a variant of
`MentionSource`. `context`, `view`, `call`, `batch` and `lua` take `file`
(a path, file name or tab title, `target_tab`) to read another open
file; there only `read_only` commands run (`RunKind::Agent::elsewhere`,
checked per call, `changes_elsewhere`). The `view` tool and `doc.picture` (headless too)
draw through `doc_render::proof`: a `Request` (view by name, angles or direction
in the axis preset's frame, bodies, highlight, markers, section, edges,
xray, size, annotate) over `thumbnail::draw` and its `Look`, framed to
what is drawn, labels as SVG through usvg with only the bundled Plex
face (the file `ui_kit::theme::SANS_REGULAR_TTF` bundles); every view but `current`
draws the same bytes whatever the camera. Agents are configured in `UserSettings.ai`. `docs/AI.md` is the user guide. `docs/ASSEMBLY.md` is the Assembly user guide.
