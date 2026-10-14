# AI agents

printCAD talks to AI agents in two ways: it starts agents that speak the
Agent Client Protocol (ACP) and chats with them in the Assistant panel,
and it serves its commands over the Model Context Protocol (MCP), to
those agents and to any other MCP client.

An agent works through the same commands scripts use (see
[Scripting](SCRIPTING.md)), so it can do what a script can and nothing
more, and every change it makes is an ordinary undo step.

## Setting up an agent

Preferences › AI agents lists the agents. Each has a name, the command
that starts it, its arguments and extra environment variables. The two
presets fill these in:

| Agent | Command | Arguments |
| --- | --- | --- |
| Claude | `claude-agent-acp` | |
| Gemini CLI | `gemini` | `--experimental-acp` |

Any program that speaks ACP over its standard input and output works the
same way. The agent runs in the folder of the open document, or your home
folder for an unsaved one, and signs in the way it does on its own.

## Chatting

Windows › Assistant opens the panel on the right.
New chat starts one with a configured agent; each chat is a tab with its
own agent and history, and several can run at once.

- Enter sends, Shift+Enter starts a new line, and Up in an empty box
  brings back your last message to edit. While the agent works, the red
  square under the box stops its turn; Close ends the chat and its agent.
- The agent's replies show as formatted text: headings, lists, tables,
  links and code, each code block with its own copy button. Hovering a
  message shows a button that copies it (a reply as the markdown it was
  written in), and any text can be selected and copied.
- The arrows at the top of the chat jump to your previous or next
  message (Alt+Up, Alt+Down), and the double arrow back to the latest.
- A message sent while the agent works waits above the box, marked
  Queued, and goes when the turn ends, one per turn. Edit takes it back
  into the box with its attachments, and the cross drops it. Stop holds
  what is queued: sending again, or Send them, lets it go.
- The bar under the box has the settings the agent offers, as it names
  them: for Claude, the permission mode (Manual, Accept edits, Plan, Auto,
  Bypass permissions), the model and the effort. A change applies to the
  chat at once, and the agent's next chats start with it.
- The "+" at the left of the bar attaches files, or a picture of the view,
  to the next message; so does pasting files copied in a file manager, or
  dropping them on the panel (on X11; Wayland does not deliver drops to
  the app). Pictures go as pictures and small text files with their
  text; anything else, such as a STEP or STL file, goes as its path for the
  agent to open. Click an attachment to take it off.
- The agent's thinking, the tools it calls and their results, and its
  plan show in the chat as it works. A call of printCAD's own tools reads
  as what it does: the few words the agent gave it ("Pocket the bolt
  holes"), else the command it runs or a script's first comment; hovering
  shows the tool's own name.
- When the agent asks for permission to do something outside printCAD
  (edit a file, run a command), the chat shows its choices.

## Chats stay with their document

A chat works on the document of the tab it was started in, whichever tab
is on screen. Once the document has a file, its chats are kept with it:
open the file again and they come back, and the first time one is shown
its agent starts in the same session, the conversation replayed, so it
continues where it left off. (The agent must be able to reload sessions;
Claude can. One that cannot starts a new chat and says so.) Closing a
chat's tab forgets it for the file; closing the document's tab keeps
them. They are kept in the application's own folder, not in the file, so
a document you pass on carries no conversations.

## What the agent knows

Every agent is told, when it connects, that it works inside printCAD while
you watch, on the document of its chat's tab, and how printCAD's tools work.
It can ask at any time what is open: the document and its file, unsaved
changes, the workbench, what is selected and being edited, the bodies and
the features that fail to build (the `context` tool).

It is told to work script-first: to plan, then make a part in one Lua
script that computes positions, reads faces and edges (`doc.faces`,
`doc.edges`) and picks from them, rebuilds and returns what it made, so
a part takes one step you approve and one undo, not one per command.

Some commands an agent never runs, whatever you allowed: quitting printCAD
and closing a tab would end its own session. The rules the agent keeps to
are yours to set, so it cannot change them either. Commands that reach past
the document wait for your OK every time, even in a chat where changes run
without asking: new and open, save as, import, export, send to slicer, undo
and redo, opening, reopening and switching tabs, and reading a file into
the document (replacing a body's shape, laying a picture in a sketch,
making a sketch of a DXF drawing), and writing a picture of the model to
a file (`doc.picture`; the `view` tool shows the agent the same picture
without asking). An agent runs those one at a time,
never from a script. Sketch text is the one exception: a font file or an
installed font that text names is kept in the document without asking, so
an agent can letter a part from a script.

## Rules

Rules are what agents keep to, in plain words: units, wall thicknesses,
naming, what never to change.

- **For every document:** Preferences › AI agents › Rules for every
  document.
- **For one document:** the Rules button beside New chat, in the Assistant
  panel. These are saved with the document, and undo takes a change back.
  (`doc.set_agent_rules` sets them from a script; an agent cannot.)
- **Your agent's own files:** an agent starts in the document's folder,
  where many read instruction files of their own on start (`AGENTS.md`;
  Claude reads `CLAUDE.md`).

An agent gets both sets of rules when its chat starts, and a chat already
open gets them again with its next message after you change them.

## Approving changes

"Ask before changes" (on by default, in Preferences and per chat) holds
every command that changes the document until you allow it. The held
change shows at the top of the panel as what the agent says it is for
and the call it makes, with Allow,
Deny, and Allow all in this chat, which turns asking off for that chat.
Commands that only read (listing bodies, measuring, looking at the view)
never wait when called on their own; a Lua script is held whatever it
does.

Each call an agent makes is one undo step, labelled with the agent. A
call or script that fails takes back what it changed and leaves no step,
and its error says so.

## The MCP server

While the application runs it listens on a local socket,
`$XDG_RUNTIME_DIR/printcad/mcp-<pid>.sock`. Its tools:

| Tool | What it does |
| --- | --- |
| `context` | What is open, selected and being edited, and the rules |
| `commands` | Lists the commands, with their arguments, optionally by prefix |
| `search` | Finds commands and guide sections by the words of a task, several queries at once |
| `describe` | A command's or guide section's whole entry, by id, bare name or a near spelling |
| `call` | Runs one command with named arguments and answers its result |
| `lua` | Runs a Lua script and answers `{returned, printed}` as JSON: what the script returns and what it printed |
| `view` | A picture of the bodies, from the current view or a fixed one |
| `log` | The application's recent messages |

### Pictures for proof

`view` draws on the CPU, so the same arguments give the same picture
whatever the user's camera is doing. With no arguments it looks the way
the view looks. Its arguments, all optional:

| Argument | What it does |
| --- | --- |
| `view` | `"current"` (the default), `"iso"`, `"front"`, `"back"`, `"left"`, `"right"`, `"top"`, `"bottom"`, `{azimuth, elevation}` in degrees (azimuth 0 is the front, 90 the right side; elevation 90 looks straight down), or `{direction, up?}`, the way the view looks. Directions follow the axis preset in Preferences (Z up by default). |
| `bodies` | Only these bodies (ids or names), framed to them. Every visible body otherwise. |
| `highlight` | A list of `{body, faces?, edges?, color?}`: faces by their `doc.faces` index or name, edges by the kernel's edge index, the whole body when neither is given. The colour is `[r, g, b]` from 0 to 1 or `"#rrggbb"`; amber when left out. |
| `markers` | A list of `{point, label?, color?}`: a dot and a label at a world point. |
| `section` | `{origin, normal}`: what lies on the side the normal points to is cut away, and the cut shows flat and darker. |
| `edges` | The faces' outlines (on by default). |
| `xray` | Bodies see-through, highlighted faces solid. |
| `size` | `[width, height]`, 800 × 600 by default, at most 2048 a side. |
| `annotate` | An axis triad and the drawn bodies' box with its sizes in mm. |

Every picture is orthographic and framed to what it draws, markers
included. The command `doc.picture{path, …}` takes the same arguments
and writes the PNG instead, in the app or in `printcad --script`, where
`"current"` draws the isometric view. Since it writes a file, an agent's
`doc.picture` waits for your OK every time.

Every tool is marked to load from the start, so an agent that defers
tools until it searches for them (Claude Code does, once many are
installed) has printCAD's at hand in its first turn. `context`, `commands`,
`search`, `describe`, `log` and `view` are marked read-only, `call` and `lua` as able to change
things, and every tool as reaching nothing outside printCAD.

An agent is told every command by its id and summary, one line each, when
it connects. `search` ranks commands and guide sections by their words and
knows the words users say for them (round for fillet, shell for
thickness, bore for hole), and `describe` gives what one takes and
answers. The Lua console's `help` searches the same way when given a word
that is not a command prefix: `help("round the edges")`.

The server also offers documents a client can read or attach
(`printcad://rules`, `printcad://context`, and the scripting and AI guides
as `printcad://guide/scripting` and `printcad://guide/ai`) and prompts,
which clients such as Claude Code offer as slash commands:

| Prompt | What it asks |
| --- | --- |
| `review-for-printing` | Check the document for what would print badly, and propose fixes |
| `explain-model` | How the document is built, feature by feature |
| `make-parametric` | Propose variables for sizes that belong together |

`printcad --mcp` relays standard input and output to that socket, so any
MCP client can use the running application as a server:

```json
{ "mcpServers": { "printcad": { "command": "printcad", "args": ["--mcp"] } } }
```

It connects to the newest running application, or to the socket
`--socket <path>` or `PRINTCAD_MCP_SOCKET` names. The chats pass the
same relay to their agents.
