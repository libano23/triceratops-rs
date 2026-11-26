# Locust through MCP, with Script and Expression nodes — design

Date: 2026-11-26 · Status: draft for approval · Branch: `locust-mcp`

## Intent

Let an AI assistant connected through `triceratops mcp` build **Locust** definitions (the
node editor) instead of only typing modelling commands, so that what it makes is
**parametric**: a live graph in the user's editor whose sliders the user (or the
assistant) can move afterwards. Add a **Script** node (code, in Rhai) and an
**Expression** node so models can be defined mathematically, both by people and by
assistants.

The models the user wants to reach, which are the acceptance targets:

1. **Screw threads** (metric, adjustable diameter/pitch/length).
2. **Gears** of several kinds: involute spur, helical, rack; bevel as a stretch goal.
3. **Surfaces defined by equations**: `z = f(x, y)` and parametric `P(u, v)`.
4. **Voronoi patterns** on planar areas and on surfaces.
5. **Joints** (incastri): e.g. dovetail, mortise-and-tenon, snap fit, with a clearance
   parameter.

Future direction (not in this work): compatibility with Grasshopper. New components
follow Grasshopper's names and port conventions where one exists, so a later importer has
a straight mapping.

## Two ways to model, the user's choice

Both stay first-class: the assistant can model **directly** (command lines through
`run_commands`, as in 0.2.0) or build a **Locust workflow** (the new tools). Which one is
the user's call, not the assistant's. The MCP server instructions say so explicitly:
use the way the user asked for; if they ask for something adjustable/parametric, use
Locust; if they did not say and it matters, ask briefly rather than pick. The two can be
mixed (a Locust bake becomes ordinary objects that commands can work on; Locust can read
document geometry through its Geometry parameter). Nothing in this work changes or
weakens direct modelling.

## Goals

- An assistant can discover components, write a whole definition, solve it, read per-node
  errors, look at the preview, tweak parameters by name, and bake — through MCP.
- The definition lands in the user's open Locust editor, visible and undoable there.
- Script node: Rhai code with user-defined inputs and outputs, access to the geometry
  kernel through the same helpers the components use.
- Expression node: a one-line formula over named variables.
- New components needed by the five targets: Helix, Populate 2D, Populate Geometry,
  Voronoi, Surface Voronoi (or Voronoi + Map to Surface).
- Booleans are safe to drive from an assistant: they finish or fail with a clear error in
  bounded time and memory.
- Five example definitions (one per target) that solve in tests.

## Non-goals

- Python or any other script language; Grasshopper file import (later).
- Fine-grained node tools (`add_node`, `connect`, ...) — see Approaches.
- File access from scripts; Open/Save/Export for assistants (stays as it is).
- A new exact B-rep Boolean algorithm.

## Anti-goals (failure even if it "works")

- An assistant request or a script can freeze the window or eat the machine's memory
  (the 39 GB thread Boolean must not be repeatable).
- A script can read or write files, start processes or reach the network.
- Existing `.locust` files stop opening or solve differently.
- The user's canvas is replaced with no way back.

## Constraints

- Pure Rust dependencies only; builds for AppImage (ubuntu-22.04), Windows zip, macOS
  universal dmg must keep working.
- Locust core stays UI-free and headless-testable (`locust/` minus `ui.rs`).
- MCP requests run on the GUI thread through `mcp::handler::handle(&mut App, ...)`; the
  bridge has a timeout. Anything slow must be bounded.
- Commit identity and release flow as usual (feature branch, PR, CI on three OSes).

## Approaches considered

**A. Whole graph as JSON (chosen).** A handful of tools; the assistant sends the full
definition in Locust's existing JSON format, gets back a solve report, then patches
parameters by name. One call per structural change; robust; reuses `Graph::to_json` /
`from_json`. Cost: structural edits resend the graph (fine for 20–60 nodes).

**B. Granular tools** (`add_node`, `connect`, `set_value`, ...). Dozens of calls per
definition, slow, leaves broken intermediate states. Rejected.

**C. A + incremental patches** (add/remove nodes without resending). Deferred; add only if
A proves awkward.

**Script language:** Rhai (chosen) — pure Rust, sandboxed by default, built-in operation
and depth limits, one engine also serves Expression. RustPython rejected (+15–20 MB, slow,
hard to interrupt, partial stdlib); pyo3/system Python rejected (breaks portable packages,
no sandbox).

## Order of work

1. Boolean safety (prerequisite: threads, gears and joints all go through Booleans).
2. Bridge zombie fix (small, same area).
3. Engine diagnostics + MCP Locust tools.
4. Expression and Script nodes (Rhai).
5. New components: Helix, Populate, Voronoi.
6. Example definitions + acceptance through MCP; README/site; release.

## 1. Boolean safety

Open issue: modelling a thread through MCP made one Boolean run 11+ minutes at 39 GB;
the GUI waits on it inside `with_big_stack` and SIGTERM is ignored. A reproduction test is
in the working tree (`mcp::handler::thread_repro`, `#[ignore]`): Cylinder, Helix, a
triangular profile, Sweep1, Cap, BooleanUnion.

- First find the root cause with that repro (why the exact B-rep Boolean falls back, and
  why the BSP mesh-CSG in `crates/geom/src/mesh_csg.rs` explodes: likely unbounded
  polygon splitting on near-coplanar/sliver faces of the swept thread). Fix the cause if
  it is a bug.
- Regardless, add a **budget** to mesh-CSG: a cap on BSP nodes / split polygons derived
  from the input sizes (builder picks the numbers so ordinary models are unaffected) and
  a **cancel flag** checked in the recursion. Exceeding it returns an error
  (`"Boolean too complex (N polygons) — simplify the inputs or raise the mesh tolerance"`),
  never a partial mesh. `mesh_boolean` gains a fallible variant; callers (commands, Locust
  Solid components, scripts) report the error instead of hanging.
- Cancel: the flag is set by Esc in the GUI and by the MCP timeout path, so a stuck
  request ends.
- Test: the repro finishes in bounded time (success or the clear error), and a test that
  forces the budget to trip.

## Long operations: Esc and live feedback (added 2026-11-26)

User requirement: every operation can be stopped with Esc, and long ones (Booleans above
all) show that they are running and not stuck — to the user and to the assistant.

- **Every command** can already be cancelled with Esc at any prompt; that stays.
- **Operations that can take long run as background jobs**: the window keeps drawing and
  responding while the work runs on a worker thread. The status bar shows a spinner and
  `<label> — 12 s · 1.3 M polygons · Esc to cancel`; Esc cancels, the result is
  discarded, the document is unchanged and the log says `<label> cancelled`.
  Covered in this work: every Boolean command (B-rep, mesh, planar/curve Booleans,
  BooleanSplit, Boolean2Objects), every other command that calls a Boolean, the Locust
  solve and Script nodes. A synchronous command step that takes over 1 s is logged under
  `TRICERATOPS_SLOW=1`, naming the command, so further commands can be moved to jobs.
- **Kernel side**: a cancel/progress token (`tri_geom::job`) is installed per worker
  thread (and carried into `with_big_stack`); the mesh-CSG recursion, the exact Boolean's
  loops and tessellation check it and report a stage text and a live counter. A cancelled
  kernel call returns `Err(Cancelled)` promptly (< 0.5 s).
- **Locust**: the solve runs as a job on a snapshot of the definition and of the document
  geometry it reads. Moving a slider during a solve cancels it and starts again; the last
  preview stays until the new one arrives; Esc cancels.
- **Assistant**: a request that starts a job is answered when the job ends, with no
  60-second cutoff while the window keeps reporting progress; the bridge forwards
  progress as MCP progress notifications when the client asked for them, `get_state`
  answers during a job with `busy: {label, elapsed_s, stage, count}`, and a new tool
  `cancel` stops the running job. If the user presses Esc the reply says
  `cancelled by the user`. Other changing requests during a job are refused with
  `busy: <label> running for 12 s (call cancel to stop it)`.
- **Budget**: the polygon budget stays, as a guard against memory blow-ups that would hurt
  the machine before anyone presses Esc. The 30-second time limit is dropped: Esc and
  `cancel` replace it.

## 2. Bridge zombies

`triceratops mcp` spawns the window and never waits on it. Reap the child (a thread that
`wait()`s), so no `<defunct>` remains. Test on Linux by process state where practical.

## 3. Engine diagnostics and MCP tools

### Diagnostics

The engine today turns a failing component into "no result" silently. Add per-node
diagnostics collected during a solve: `error` (panic message, script error with line and
column, Boolean budget error, invalid input) and `warning` (e.g. some combinations
produced nothing). The editor shows them on the node (red/orange badge, tooltip); the MCP
report returns them. Components can push a message through `Ctx`.

### Tools (bridge + handler)

| Tool | Does |
|------|------|
| `locust_components` | Search the catalogue by words/category: name, category, short description, inputs (name, kind, default, slider range, optional), outputs. Includes Script/Expression usage notes. |
| `locust_get_graph` | The current definition as JSON (root graph). |
| `locust_set_graph` | Replace the definition with the given JSON, open the editor, solve, return the solve report. Recorded in the editor's undo so the user can step back; also saved to autosave as usual. |
| `locust_set_values` | Set inputs by node nickname or uid (`{"Pitch": 1.5, "Gear.Teeth": 24}`), solve, return the report. For sliders, Number etc. |
| `locust_solve` | Re-solve (e.g. after document changes) and return the report. |
| `locust_bake` | Bake pending geometry; return added object ids (one document undo step). |

The preview is already drawn in the viewports, so `capture_view` shows it.

**Solve report:** node count; per node with something to say: uid, nickname, type,
diagnostics; a short summary of every Panel and of each output feeding Preview/Bake
(counts, geometry kinds, bounding box, volume/area where cheap); preview and bake
counts; solve time.

**Authoring convenience in `locust_set_graph`** (the JSON stays the file format):
- Positions optional: missing `x`/`y` get an automatic left-to-right layout by
  topological depth.
- Connections may name ports by name instead of index and nodes by nickname instead of
  uid (`{"from": "Radius", "to": "Circle.Radius"}`), resolved before loading; unknown
  names are an error listing what exists.
- Validation before replacing anything: unknown component types, bad ports, cycles →
  error, canvas unchanged.

The tool descriptions and `locust_components` must carry enough of the format (with a
tiny example) that an assistant can write a graph without guessing. Bump the MCP protocol
`VERSION` if the bridge/window handshake needs it.

## 4. Expression and Script (Rhai)

One shared Rhai `Engine` (built once, `OnceLock`) with the Locust API registered, the
standard Rhai sandbox (no file/system modules), and limits: max operations (order of
10⁷), call depth, string/array sizes, plus a wall-clock check via `on_progress` (order of
seconds) so no script can stall the solve. Compiled ASTs are cached per node by source
hash.

### Expression

- Inputs `x`, `y`, `z` (Any, optional) by default; output `Result`. Formula text in the
  node (`Expression` value, edited inline on the canvas).
- Element component: matched like the others, so a list of `x` gives a list of results.
- Maths in **radians** for `sin`/`cos`/... (Grasshopper's Expression convention), with
  `pi`, `e`, `deg(x)`, `rad(x)`. Note: Locust's existing Sine/Cosine components use
  degrees; this difference is documented in the component description.
- The user/assistant can add/rename variables (same mechanism as Script ports).

### Script

- Node stores `code`, and its port lists: `inputs: [{name, kind, access}]`,
  `outputs: [{name}]`, where access is `item` (default; element-wise like other
  components) or `list` / `tree` (whole branch / whole tree as arrays). Ports are
  per-node, resolved the way clusters' ports are (`Graph::cluster_ports` precedent); the
  editor gets "+ input / + output / rename" on the node and a code editor dialog
  (monospace, error line highlighted).
- Inputs appear as Rhai variables; outputs are whatever variables of those names hold
  at the end (scalar → one item, array → list). `print()` goes to the node's messages
  (shown in the report and in a tooltip/panel).
- API (thin wrappers over the existing component functions and `tri_geom`, nothing
  re-implemented): `point`, `vec`, vector maths; `line`, `polyline`, `interpolate`,
  `nurbs_curve`, `circle`, `arc`, `helix`; `curve_point(c, t)`, `divide`, `length`,
  `offset`; `surface_from_grid`, `loft`, `extrude`, `revolve`, `sweep1`, `cap`,
  `surface_point(s, u, v)`; `box`, `cylinder`, `sphere`, `union`, `difference`,
  `intersection` (budgeted); `move`, `rotate`, `scale`, `mirror`, `array_polar`;
  `area`, `volume`, `bbox`; `random(seed)`. Exact list is the builder's call, guided by
  what the five examples need.
- Geometry crosses the boundary as Locust `Value`s (Rhai custom types), so script
  outputs wire into normal components and vice versa.

## 5. New components

Names/ports follow Grasshopper where it has one.

- **Helix** (Curve): base plane/point, axis, radius, pitch or turns, optional end radius
  (conical). Reuses the `Helix` command's curve construction.
- **Populate 2D** (Vector): region (closed planar curve or rectangle), count, seed →
  points; deterministic for a seed; optional relaxation not required.
- **Populate Geometry** (Vector): surface/brep, count, seed → points on it (area
  weighted).
- **Voronoi** (Mesh/Vector category as in GH): points, boundary (closed planar curve or
  rectangle), plane → closed polyline cells clipped to the boundary. Builder's call on
  algorithm; half-plane clipping per cell (no dependency) is enough for a few thousand
  points; a Delaunay crate is acceptable if pure Rust.
- **Surface Voronoi** (Surface): surface, points (or count+seed) → cells computed in the
  surface's UV domain (area-corrected sampling) and mapped onto it as curves on the
  surface; the user can then offset/pull/split. Exact approach is the builder's call;
  result must look right on a non-planar surface (check by capture).

Joints need no new component beyond working Booleans: Offset Surface / Scale give the
clearance; the example shows how.

## 6. Examples and acceptance

Five definitions in `site/tools/scenes/` (or `crates/app/src/locust/examples/`), each
also built in a headless test and, once, through the real MCP bridge by an assistant:

1. M10×1.5 thread on a 20 mm rod: Helix + profile + Sweep1 + Union, sliders for
   diameter, pitch, length. Valid closed solid, bounded solve time.
2. Involute spur gear: Script computing the tooth profile from module, teeth, pressure
   angle; Extrude; a helical variant via Twist/Script.
3. Equation surface: Expression `sin(x)*cos(y)` over a grid → Surface from Grid; and a
   parametric one (torus-knot tube or similar) from Script.
4. Voronoi: on a rectangle (cells offset into a frame and extruded) and on a curved
   surface.
5. Dovetail joint: two parts from Boxes and a Script-made dovetail, Difference with a
   clearance slider; parts don't intersect at clearance > 0.

Acceptance also checks: invalid graph → error and unchanged canvas; script infinite loop
→ error within the limit; Boolean budget trip → clear error; Undo in the editor restores
the previous canvas; existing tests and `.locust` files unaffected.

Docs: README Locust section and `docs/FEATURES.md` mention Script/Expression and the MCP
tools; MCP server instructions mention Locust. Release as 0.3.0 after the user's
go-ahead.

## Left to the builder

Budget numbers; Rhai limit values; the exact script API list; Voronoi algorithm; layout
algorithm for unpositioned nodes; diagnostic badge styling; where example files live;
whether Surface Voronoi is one component or Voronoi + a Map to Surface component.

## Calls made in writing this (check these)

- Expression uses radians (Grasshopper convention), unlike Locust's degree-based trig
  components.
- `locust_set_graph` replaces the whole canvas without asking; it is one editor undo
  step. Assistant Locust requests are refused while the user is typing in the editor or
  has one of its dialogs open, the same rule as for commands.
- The Boolean budget returns an error, never a partial or approximate result.
- Bevel gears are a stretch goal; spur, helical and rack are required.
