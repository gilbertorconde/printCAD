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
| Claude Code | `claude-code-acp` | |
| Gemini CLI | `gemini` | `--experimental-acp` |

Any program that speaks ACP over its standard input and output works the
same way. The agent runs in the folder of the open document, or your home
folder for an unsaved one, and signs in the way it does on its own.

## Chatting

Windows › Assistant opens the panel on the right.
New chat starts one with a configured agent; each chat is a tab with its
own agent and history, and several can run at once.

- Enter sends, Shift+Enter starts a new line. Stop ends the agent's
  turn; Close ends the chat and its agent.
- The agent's thinking, the tools it calls and their results, and its
  plan show in the chat as it works.
- When the agent asks for permission to do something outside printCAD
  (edit a file, run a command), the chat shows its choices.

## Approving changes

"Ask before changes" (on by default, in Preferences and per chat) holds
every command that changes the document until you allow it. The held
change shows at the top of the panel as the call it makes, with Allow,
Deny, and Allow all in this chat, which turns asking off for that chat.
Commands that only read (listing bodies, measuring, looking at the view)
never wait.

Each call an agent makes is one undo step, labelled with the agent.

## The MCP server

While the application runs it listens on a local socket,
`$XDG_RUNTIME_DIR/printcad/mcp-<pid>.sock`. Its tools:

| Tool | What it does |
| --- | --- |
| `commands` | Lists the commands, with their arguments, optionally by prefix |
| `call` | Runs one command with named arguments and answers its result |
| `lua` | Runs a Lua script and answers what it printed |
| `view` | A picture of the scene from the current view |
| `log` | The application's recent messages |

`printcad --mcp` relays standard input and output to that socket, so any
MCP client can use the running application as a server:

```json
{ "mcpServers": { "printcad": { "command": "printcad", "args": ["--mcp"] } } }
```

It connects to the newest running application, or to the socket
`--socket <path>` or `PRINTCAD_MCP_SOCKET` names. The chats pass the
same relay to their agents.
