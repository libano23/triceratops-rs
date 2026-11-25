# MCP server: modelling in Triceratops from an AI assistant

Date: 2026-11-25
Status: draft for approval

## Intent

Let an AI assistant (Claude, Codex, or any MCP client) model in Triceratops from a
prompt: "a bracket with two 6 mm holes" becomes a sequence of real Triceratops
commands. The user watches the result build up **in the Triceratops window they
already have open**, can step in at any time, and can undo what the assistant did.
Target: the next release (0.2.0).

## Goals

- An MCP server that works with every common MCP client, including Claude Code,
  Claude Desktop and Codex.
- The assistant drives the **running** window and model; the viewports update as
  each command runs. If no window is open, one is started.
- The assistant can use the existing command set (about 1,070 commands, their
  prompts and options) rather than a separate, smaller API.
- The assistant can see what it made: the object list with types and sizes, and
  an image of the viewport.
- Every change can be undone with the normal Undo.

## Non-goals (v1)

- Network access: the server is local to the user's machine only.
- Several documents or windows at once (one window is driven; see "Which window").
- Editing Locust graphs through MCP (a possible v2: the Locust engine is already
  UI-free).
- Streaming progress for long commands; each call returns when it is done.

## Anti-goals (failure even if it works)

- A command started by the assistant opens a modal dialog and waits for a human
  (Loft options, file pickers, message boxes): the assistant hangs and the user
  is confused.
- The assistant's work freezes the UI, or the user's typing and the assistant's
  commands interleave into a broken command.
- Files are read or written somewhere the assistant did not name explicitly.
- Any process on the network can drive the app.

## Approaches considered

| | How | Trade-off |
|---|---|---|
| A | MCP over HTTP served from inside the app | One process, but HTTP transport support differs between clients; port management and exposure. |
| **B (chosen)** | `triceratops mcp`: a stdio MCP bridge that connects to the running app over a local socket, starting the app if needed | Works with every client (stdio is universal) and drives the window already open; costs a small internal protocol. |
| C | The assistant launches `triceratops --mcp` and talks to it over stdio | Simplest, but the window lives and dies with the assistant session and cannot attach to an open model. |

The user chose B.

## Design

### Pieces

```
assistant ──stdio/MCP──▶ triceratops mcp ──local socket/JSON──▶ Triceratops window
 (Claude, Codex)          (bridge, no window)                    (the App, GUI thread)
```

1. **The bridge** is the same executable started as `triceratops mcp`. It opens no
   window, speaks MCP on stdin/stdout using the official Rust SDK (`rmcp`, stdio
   transport), and forwards each tool call as one JSON request over the local
   socket. One binary means nothing extra to package: the AppImage, the Windows
   zip and the macOS app all already contain it.
2. **The app side** is a listener started with the window. Requests arrive on a
   background thread, are handed to the GUI thread (channel + winit
   `EventLoopProxy` wake-up), executed against `App` between frames, and the reply
   is sent back. Commands therefore run exactly as typed ones do, and the
   viewports repaint after each request.
3. **The internal protocol** is newline-delimited JSON over the socket: `{"id",
   "method", "params"}` → `{"id", "result"}` or `{"id", "error"}`. It is private
   to the two halves of the same binary and versioned with a `hello` handshake
   carrying the app version, so a bridge and an app of different versions refuse
   each other with a clear message.

### Local socket and which window

- Linux and macOS: a Unix socket at `$XDG_RUNTIME_DIR/triceratops/<pid>.sock`
  (macOS / no runtime dir: the user's temporary directory), permissions 0600.
  Windows: a named pipe `\\.\pipe\triceratops-<user>-<pid>`. Crate:
  `interprocess` (both platforms, one API).
- Each running window registers its socket; the bridge connects to the **most
  recently started** live one. `TRICERATOPS_SOCKET` overrides the choice. Stale
  sockets (process gone) are removed when found.
- If no window is running, the bridge starts `triceratops` (its own executable)
  and waits up to 20 s for its socket.
- Nothing listens on a network interface.

### Tools exposed to the assistant

Kept few and general, because the command line already is the modelling API:

| Tool | What it does | Returns |
|---|---|---|
| `run_commands(lines: string[])` | Runs lines exactly like a script file: command names, points `x,y,z`, numbers, option names, `#id`/`last` object references, `""` for Enter. | The command-line log produced, ids of objects added / changed / deleted, and the prompt now waiting (if a command still wants input) with its options. |
| `get_state()` | Current prompt and options, selected ids, active viewport, units, layers. | JSON. |
| `list_objects(filter?)` | Objects with id, type, layer, name, bounding box, closed/solid flag. Filter by type, layer, ids. | JSON. |
| `object_info(id)` | Details for one object: type, bbox, and area / volume / length where defined. | JSON. |
| `find_commands(query)` | Searches the command catalogue (names, aliases, descriptions). | Matching commands with descriptions. |
| `capture_view(viewport?, width?, height?)` | Renders a viewport to an image. | PNG as MCP image content, so the assistant can look at its work. |
| `undo(steps?)` / `redo(steps?)` | Normal document undo / redo. | Log lines. |

Server instructions sent with the MCP `initialize` reply explain the scripting
syntax (the rules already in the README's "Scripts" section), that units are the
document's, how to read prompts, and to check results with `list_objects` /
`capture_view`.

### Behaviour while the assistant works

- **No modal UI from assistant calls.** Requests carry an "assistant" origin.
  For them:
  - the Loft options dialog is skipped and the defaults are used (options can be
    given as command options where the command supports them);
  - commands that would open a file picker (open, save, import, export, capture)
    require a path and fail with a clear error otherwise; relative paths resolve
    against the document's folder, or the user's home when there is none;
  - message boxes become log lines in the reply.
- **No interleaving.** While a request runs, typed input is held until it ends.
  If the user has a command half-entered when a request arrives, the request
  is rejected with "the user is running a command" rather than mixing the two.
- **Undo.** Each command keeps its own undo step, as when typed. A whole
  `run_commands` call is *not* folded into one step in v1.
- **Visible.** The status bar shows "Assistant connected" while a bridge is
  attached, and the command history shows the assistant's lines like typed ones.
- **Off switch.** Options → General → "Allow AI assistants (MCP)", on by
  default; when off, no socket is created.
- **Timeouts.** A request that does not finish in 60 s returns an error; the
  command keeps running in the app and can be cancelled with Esc as usual.

### Set-up for users (README section and website)

```sh
claude mcp add triceratops -- /path/to/Triceratops.AppImage mcp          # Claude Code
codex mcp add triceratops -- "C:\Triceratops\triceratops.exe" mcp       # Codex
```
plus the equivalent `claude_desktop_config.json` / Codex `config.toml` snippets,
with the per-OS path to the executable.

## Testing

- **Unit:** request handling against a headless `App` (no window): each tool,
  object references, prompt reporting, dialog suppression, path rules, undo.
- **Integration:** bridge ↔ app over a real socket on each OS in CI, through an
  `rmcp` client: `initialize`, `tools/list`, `run_commands` building a box with a
  hole, `list_objects`, `capture_view` returning a PNG.
- **Acceptance (asked by the user):** after implementation, Claude (this
  assistant) connects to a running Triceratops through `triceratops mcp` and
  models something real from a prompt, checking its work with `list_objects`
  and `capture_view`; screenshots of the live window are shared.

## Decided by the user

- Approach B; the assistant works in the open window, which updates live.
- Final check: the assistant tests it by modelling something itself.

## Calls made here (open to change)

- One executable with an `mcp` subcommand instead of a second binary.
- A small set of general tools centred on `run_commands`, not one tool per
  command.
- MCP on by default with an Options switch and a status-bar indicator.
- Most recently started window wins; `TRICERATOPS_SOCKET` overrides.
- No modal dialogs for assistant calls; explicit paths required for files.
- Each command is its own undo step (no grouping in v1).

## Left to the builder

- Exact JSON shapes of replies, and how `ObjFilter` / prompt kinds are rendered
  in them.
- How the GUI thread is woken (`EventLoopProxy` user event vs. polling in
  `about_to_wait`).
- Rendering `capture_view` off-screen at a size other than the window's, reusing
  the existing capture path where possible.
- Windows: confirm that the GUI-subsystem executable inherits stdin/stdout pipes
  when started by an MCP client; if not, add a console-subsystem `triceratops-mcp.exe`
  next to it.
