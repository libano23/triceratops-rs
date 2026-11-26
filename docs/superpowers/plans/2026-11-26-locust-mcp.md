# Locust through MCP, Script and Expression — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An assistant connected through `triceratops mcp` can, at the user's choice, either
model directly (as today) or build a live, parametric Locust definition — including Rhai
Script and Expression nodes and new Helix / Populate / Voronoi components — with Booleans
that always finish.

**Architecture:** Booleans get a polygon budget and a time limit in `tri_geom::mesh_csg`
and become fallible. The Locust engine collects per-node diagnostics in `Ctx`. A headless
`locust::authoring` module turns assistant JSON (names instead of uids/indices, optional
positions) into a validated `Graph`; `mcp::handler` exposes six `locust_*` methods that
drive `App::locust` (the editor), and `mcp::bridge` publishes them as tools. Script and
Expression are *dynamic-port* nodes solved by `locust::script` on one shared Rhai engine.

**Tech Stack:** Rust 2021 workspace, `rhai` 1.x (`sync`, `no_module`), egui 0.37, rmcp 3.

**Spec:** `docs/superpowers/specs/2026-11-26-locust-mcp-design.md`

## Global Constraints

- Pure Rust dependencies only; the AppImage (ubuntu-22.04), Windows zip and macOS universal dmg builds must keep working.
- Locust core stays UI-free: everything except `locust/ui*.rs` is testable headless.
- Direct modelling through `run_commands` is unchanged in behaviour.
- Existing `.locust` files open and solve exactly as before (dynamic-port nodes are new kinds; nothing existing is renamed).
- New components use Grasshopper's names and port names where one exists.
- Scripts have no file, process or network access (`no_module`, no custom I/O functions).
- Expression and Script maths use radians; existing Sine/Cosine components stay in degrees.
- Commit as `Attilio Mandola <libanesevolante@pm.me>`; every commit message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Work on branch `locust-mcp`; no merge or tag without the user's go-ahead.
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass at the end of every task.

## Review Focus

1. **Assistant sends a graph while the user is wiring or has a Locust dialog open** → refused with "the user is working in Locust", canvas untouched (test in Task 6).
2. **A graph that references a node nickname used twice** → error naming the duplicate, not a silent pick of the first (test in Task 5).
3. **Script returning nothing / a wrong type for an output (e.g. a map)** → node error "output `Cells`: expected number, point, geometry or array", other outputs still delivered (test in Task 8).
4. **Populate / Surface Voronoi with Count 0 or a huge count (1e7)** → empty output with a warning for 0; for huge counts a clear "count limited to 100000" warning instead of a hang (test in Tasks 10–11).
5. **Expression with a typo in a variable name** (`sin(xx)`) → error with the unknown name and column, not "no result" (test in Task 7).

---

### Task 1: Bounded mesh-CSG Booleans (root cause + budget)

**Files:**
- Modify: `crates/geom/src/mesh_csg.rs`, `crates/geom/src/brep_boolean.rs`, `crates/geom/src/lib.rs`
- Modify (callers, follow the compile errors): `crates/app/src/commands/{mesh2,mesh_cmds,mesh3,surfaces3,misc4}.rs`, `crates/app/src/locust/components/solid.rs`
- Test: `crates/geom/src/mesh_csg.rs` (`#[cfg(test)]`), `crates/app/src/mcp/handler.rs` (`thread_repro`, already in the working tree)

**Interfaces:**
- Produces: `pub enum BooleanError { TooComplex { polygons: usize }, TimedOut { seconds: u64 }, Empty }` with `Display`:
  - TooComplex → `"Boolean too complex ({polygons} polygons) — simplify the inputs or use a coarser mesh"`
  - TimedOut → `"Boolean gave up after {seconds} s — simplify the inputs"`
  - Empty → `"Boolean produced nothing"`
- `pub fn mesh_boolean(a: &Mesh, b: &Mesh, op: BooleanOp) -> Result<Mesh, BooleanError>` (was infallible)
- `pub fn mesh_boolean_tagged(...) -> Result<Vec<TaggedPolygon>, BooleanError>`
- `pub struct Budget { pub max_polygons: usize, pub time_limit: Duration }`, `impl Budget { pub fn for_inputs(na: usize, nb: usize) -> Self }` = `max_polygons: max(200_000, 40 * (na + nb))`, `time_limit: 30 s`; env `TRICERATOPS_BOOL_BUDGET` (polygons) overrides for experiments.
- `brep_boolean_verbose` maps a `BooleanError` into its `Err(String)` with the Display text.
- Deviation from the spec, on purpose: no Esc cancel. The Boolean runs while the GUI thread waits on it, so Esc cannot be read; the time limit is what ends a stuck Boolean for both the user and the MCP path.

- [ ] **Step 1: Find the root cause.** Run the repro under a memory cap so it cannot eat the machine:
  `(ulimit -v 8000000; TRICERATOPS_BOOL_DEBUG=1 timeout 300 cargo test -p triceratops --release thread_repro -- --ignored --nocapture)`.
  Use superpowers:systematic-debugging. Record (in the commit message) why the exact Boolean failed and why BSP splitting exploded (expected suspect: near-coplanar slivers of the swept thread re-split at every level). If it is a defect (e.g. a sliver used as a splitting plane, an epsilon mismatch), fix it there with a geom unit test reproducing the minimal shape.
- [ ] **Step 2: Write failing tests** in `mesh_csg.rs`:
  - `budget_trips_with_clear_error`: two `Mesh` spheres of 20k triangles each, offset by half a radius, with `TRICERATOPS_BOOL_BUDGET` unset but calling the internal `boolean_nodes_budgeted(.., Budget { max_polygons: 1000, time_limit: 30s })` → `Err(BooleanError::TooComplex { .. })`.
  - `ordinary_boolean_unaffected`: box minus cylinder via `mesh_boolean` → `Ok`, closed mesh, same triangle count as before this change (capture the number first).
- [ ] **Step 3: Run, confirm FAIL** (`cargo test -p tri-geom budget_`): does not compile / no budget.
- [ ] **Step 4: Implement.** Thread a `&mut Tracker { count, limit, deadline }` through `Node::new / build / clip_to / clip_polygons / invert` (the BSP recursion); increment on every polygon created by a split and on every node built; check deadline every 4096 increments; bail with `Err` up the stack. `with_big_stack` stays; a panic inside the thread becomes `Err(BooleanError::Empty)` instead of `expect`.
- [ ] **Step 5: Update callers.** Commands print the error text to the log and leave the document unchanged; Locust Solid components push the error to `ctx` diagnostics (after Task 3 lands they use `ctx.error`; until then return `None`).
- [ ] **Step 6: Run** `cargo test --workspace` and `(ulimit -v 8000000; timeout 120 cargo test -p triceratops --release thread_repro -- --ignored --nocapture)`. Expected: the repro finishes within 120 s with either a closed union or the clear error; peak memory < 8 GB.
- [ ] **Step 7: Make the repro a normal test**: rename `thread_repro::screw_thread_boolean` to `acceptance_tests::screw_thread_boolean_is_bounded`, drop `#[ignore]` if it runs < 20 s in debug, assert "added 1 object" or log contains `"Boolean"` error text.
- [ ] **Step 8: Commit** `Booleans: polygon budget and time limit, fallible mesh_boolean`.

### Task 2: Reap the window started by the bridge

**Files:** Modify `crates/app/src/mcp/bridge.rs:262-279`; test in the same file.

**Interfaces:** `fn launch(exe: &Path) -> std::io::Result<()>` unchanged signature; it spawns a detached thread that calls `child.wait()`.

- [ ] **Step 1: Failing test** (`#[cfg(unix)]` `launch_reaps_child`): `launch` a tiny program (`/bin/true` via a test-only `launch_cmd(cmd: Command)` helper split out of `launch`), sleep 300 ms, read `/proc/<pid>/stat` → state is not `Z` (file missing is fine).
- [ ] **Step 2: Run, FAIL** (state `Z`).
- [ ] **Step 3: Implement**: keep the `Child`, `std::thread::spawn(move || { let _ = child.wait(); })`.
- [ ] **Step 4: Run, PASS.** **Step 5: Commit** `MCP bridge: reap the window process`.

### Task 3: Per-node diagnostics in the Locust engine and on the canvas

**Files:**
- Modify: `crates/app/src/locust/components/mod.rs` (Ctx), `crates/app/src/locust/engine.rs`, `crates/app/src/locust/ui.rs` (badge + tooltip), `crates/app/src/locust/components/solid.rs`
- Test: `crates/app/src/locust/tests.rs`

**Interfaces:**
- Produces in `components/mod.rs`:
  - `pub enum Level { Info, Warning, Error }`, `pub struct Diagnostic { pub level: Level, pub message: String }`
  - `Ctx` gains `pub diagnostics: HashMap<String, Vec<Diagnostic>>` and `pub current: Option<String>` (uid being solved).
  - `impl Ctx { pub fn error(&mut self, msg: impl Into<String>); pub fn warn(&mut self, msg: impl Into<String>); pub fn print(&mut self, msg: impl Into<String>) }` — attach to `current` (`print` → `Level::Info`).
- Engine sets `ctx.current` before each node; a panic records `Error` with the panic payload text (`&str`/`String` downcast, else `"component panicked"`); an element component whose inputs are non-empty but produced no item in any output records `Warning "no result"`. Unknown kind records `Error "unknown component <kind>"`. Diagnostics of nodes inside clusters are reported on the cluster node, prefixed with the inner node label.

- [ ] **Step 1: Failing tests** in `tests.rs`: `panic_becomes_error_diagnostic` (a test-only registry with a panicking component, via `solve_with`), `empty_result_warns` (`Divide Curve` fed a Number), `unknown_kind_errors`, `boolean_budget_error_reaches_node` (Solid Union fed two meshes with `TRICERATOPS_BOOL_BUDGET=100`; message contains `"too complex"`).
- [ ] **Step 2: FAIL. Step 3: Implement.** Canvas: a small red (error) / orange (warning) dot at the node's top-right; hover tooltip lists messages; status line counts errors.
- [ ] **Step 4: PASS, clippy. Step 5: Commit** `Locust: per-node diagnostics`.

### Task 4: Dynamic-port nodes (shared by Script and Expression)

**Files:** Modify `crates/app/src/locust/graph.rs`, `crates/app/src/locust/engine.rs`, `crates/app/src/locust/ui.rs:60-90` (port lookup); Test `locust/tests.rs`.

**Interfaces:**
- Produces in `graph.rs`:
  - `pub struct PortDef { pub name: String, pub kind: String, pub access: Access }`, `pub enum Access { Item, List, Tree }` (JSON `"item" | "list" | "tree"`, default item).
  - `pub fn is_dynamic(kind: &str) -> bool` → `kind == "Script" || kind == "Expression"`.
  - `impl Graph { pub fn dynamic_ports(&self, node: &Node) -> (Vec<PortDef>, Vec<PortDef>) }` reading `node.values["_inputs"]`, `["_outputs"]`; defaults when absent: Expression `x,y,z` (Any, item, all optional) → `Result`; Script `x,y` → `a`.
  - `impl Graph { pub fn node_ports(&self, node: &Node) -> Ports }` — one place for (inputs, outputs) as (name, kind) pairs: cluster → `cluster_ports`, dynamic → `dynamic_ports`, otherwise from the registry. UI and authoring use this.
- Engine: `if is_dynamic(&node.kind)` → `crate::locust::script::solve_node(graph, node, ctx, &outputs)` (Task 7 provides it; in this task a stub returning empty trees).
- Inline values for dynamic inputs are keyed by port name.

- [ ] **Step 1: Failing tests**: `dynamic_ports_default_and_custom`, `node_ports_covers_all_three`, `dynamic_node_round_trips_json` (to_json/from_json keeps `_inputs`).
- [ ] **Step 2: FAIL. Step 3: Implement** (UI: dynamic nodes draw ports from `node_ports`; right-click menu "Add input", "Add output", "Rename port…", "Remove port", editing `_inputs`/`_outputs` with undo). **Step 4: PASS. Step 5: Commit** `Locust: nodes with their own ports`.

### Task 5: Authoring — assistant JSON to a validated graph

**Files:** Create `crates/app/src/locust/authoring.rs`; modify `locust/mod.rs`; test inside the new file.

**Interfaces:**
- Produces: `pub fn prepare(input: &serde_json::Value) -> Result<Graph, String>`.
- Accepted input = the file format (`nodes`, `connections`, `groups`, `comments`, `clusters`) plus:
  - node `uid` optional; `nickname` is a handle; node `type` must be a registry name, `Script`, `Expression` or an existing `Cluster:<id>`.
  - connection either the file form or `{"from": "<node>[.<output>]", "to": "<node>.<input>"}` where `<node>` is a uid or nickname and ports are names or indices; omitted output = output 0.
  - `values` keyed by input name or index (already understood by the engine).
  - `x`/`y` optional: missing ones get column = longest-path depth × 260, row = order within column × 140.
- Errors (exact prefixes, tests check them): `unknown component "<t>" (did you mean: a, b, c?)` (3 closest by edit distance), `unknown node "<n>"`, `nickname "<n>" is used by 2 nodes`, `node "<n>" has no input "<p>" (inputs: …)`, `node "<n>" has no output "<p>" (outputs: …)`, `the graph has a cycle through: a, b`, `input "<n>.<p>" is wired twice`.

- [ ] **Step 1: Failing tests**: `prepare_resolves_names_and_layout` (Number "Radius" → Circle.Radius, Circle → Extrude.Base; asserts connection indices and that x of Extrude > x of Circle), and one test per error above.
- [ ] **Step 2: FAIL. Step 3: Implement** using `Graph::node_ports`. **Step 4: PASS. Step 5: Commit** `Locust: authoring from assistant JSON`.

### Task 6: MCP tools for Locust

**Files:**
- Create: `crates/app/src/locust/report.rs`
- Modify: `crates/app/src/locust/ui.rs` (Editor API), `crates/app/src/mcp/handler.rs`, `crates/app/src/mcp/bridge.rs` (params, tools, INSTRUCTIONS), `crates/app/src/app.rs` only if a field is needed
- Test: `handler.rs` `acceptance_tests`, `report.rs`

**Interfaces:**
- `Editor` gains: `pub fn replace_root(&mut self, g: Graph)` (records undo tag `"Assistant graph"`, clears cluster stack and selection, opens the window, marks dirty); `pub fn busy(&self) -> bool` (dialog open, wire or band being dragged, or a canvas text field focused); `pub fn set_inputs(&mut self, values: &serde_json::Map<String, J>) -> Result<Vec<String>, String>` — key `"<node>"` sets the node's first input, `"<node>.<input>"` a named one; node by nickname or uid; one undo step; returns the changed keys; `pub fn root_outputs(&self) -> &Outputs`; `pub fn diagnostics(&self) -> &HashMap<String, Vec<Diagnostic>>`; `pub fn bake_counts(&self) -> (usize, usize)` (preview, bake); `solve` also stores root outputs, diagnostics and `last_solve: Duration`.
- `report.rs`: `pub fn solve_report(graph: &Graph, outputs: &Outputs, diags: &HashMap<String, Vec<Diagnostic>>, preview: usize, bake: usize, elapsed: Duration) -> serde_json::Value` →
  `{"nodes": N, "errors": E, "warnings": W, "preview": P, "bake": B, "solve_ms": T, "problems": [{uid, label, type, level, message}], "panels": [{label, text}], "results": [{label, port, items, kinds: {...}, bbox: [[..],[..]] | null, volume | area | length (first item, when cheap)}]}` where `results` covers every output wired into Preview, Custom Preview or Bake; `panels` uses `DataTree::format(6, 5)`.
- Handler methods (names = MCP tool names): `locust_components {query?, category?}` → `[{name, category, description, inputs:[{name, kind, default, slider, optional}], outputs:[..]}]` plus, always, entries for `Script` and `Expression` with usage text; `locust_get_graph` → root `to_json()`; `locust_set_graph {graph}` → `authoring::prepare` then `replace_root` then solve → report; `locust_set_values {values}` → report; `locust_solve` → report; `locust_bake` → `{"added": [ids]}` (one document undo step; Bake in the editor is reused).
- `locust_set_graph`, `locust_set_values`, `locust_bake` are refused with `"the user is working in Locust — try again shortly"` when `app.locust.busy()`; all locust methods are also refused under the existing `busy_with_user` rule.
- INSTRUCTIONS gain a "Two ways to model" paragraph: *"Model the way the user asks. Direct commands (run_commands) make fixed geometry. A Locust definition (locust_set_graph) makes a live, parametric model the user can adjust with sliders. If the user wants something adjustable, use Locust; if they have not said and it matters, ask them briefly."* plus a 10-line example graph (Number Radius → Circle → Extrude → Preview/Bake) and the connection shorthand.

- [ ] **Step 1: Failing tests** in `acceptance_tests`: `locust_set_graph_solves_and_reports` (example graph → report `errors == 0`, `results[0].kinds.brep == 1`, volume ≈ π·6²·12 ±1%); `locust_set_values_by_nickname` (`{"Radius": 3}` → volume ≈ π·9·12); `locust_bad_graph_leaves_canvas` (unknown type → Err text with "did you mean", `locust_get_graph` unchanged); `locust_refused_while_user_busy` (Review Focus 1: set a dialog open via a test hook, expect the refusal text); `locust_bake_adds_objects` (ids returned, `app.doc` count +1, Undo removes it); `locust_components_search` (`query: "voronoi"` returns Voronoi after Task 11 — write now with `query: "extrude"`).
- [ ] **Step 2: FAIL. Step 3: Implement** handler + Editor API + report. **Step 4: Implement bridge tools** (one `#[tool]` per method with descriptions stating the JSON format briefly; params structs with `serde_json::Value` / `Map` fields and JsonSchema).
- [ ] **Step 5: PASS; run the MCP stdio smoke test** used in CI (`.github/workflows/release.yml` shows the command) locally and confirm `tools/list` contains the six `locust_*` tools.
- [ ] **Step 6: Commit** `MCP: Locust tools`.

### Task 7: Rhai engine and the Expression node

**Files:**
- Modify: `crates/app/Cargo.toml` (`rhai = { version = "1", features = ["sync", "no_module"] }`)
- Create: `crates/app/src/locust/script/mod.rs` (engine, limits, `solve_node`), `crates/app/src/locust/script/convert.rs` (Value ⇄ Dynamic)
- Modify: `locust/mod.rs`, `locust/engine.rs` (call `script::solve_node`), `locust/components/params.rs` (register catalogue entries `Script`, `Expression` in category `Script` with description; the engine never calls their func)
- Test: `crates/app/src/locust/script/tests.rs`

**Interfaces:**
- `pub fn engine() -> &'static rhai::Engine` (OnceLock): `set_max_operations(50_000_000)`, `set_max_call_levels(64)`, `set_max_string_size(1 << 20)`, `set_max_array_size(1_000_000)`, `set_max_map_size(100_000)`, `on_progress` aborting when a thread-local deadline (`5 s` per node solve) has passed with message `"script took longer than 5 s"`; constants `PI`, `E`; `deg(x)`, `rad(x)`.
- `convert.rs`: `pub fn to_dynamic(v: &Value) -> Dynamic`, `pub fn from_dynamic(d: Dynamic) -> Result<Value, String>`; Number↔FLOAT, Int↔INT, Bool, Text↔String, List↔Array, Point↔custom `Point` (getters/setters `x y z`, `+ - * /` with FLOAT, `dot`, `cross`, `length`, `unit`), Geometry↔custom `Geo` (`kind` getter), Plane/Pair/Colour↔custom types with getters. Unsupported (Map, FnPtr…) → `Err("expected number, point, geometry or array, got <type>")`.
- `pub fn solve_node(graph: &Graph, node: &Node, ctx: &mut Ctx, outputs: &Outputs) -> Vec<DataTree>`: builds input trees by port (wired, else inline value by name, else Null), applies access (List: each branch → one `Value::List` item at that path; Tree: whole tree → one `Value::List` of branch lists), matches with `match_trees`, evaluates once per combination with inputs as scope variables, collects outputs (Array → `Out::Many`, `()` → `Out::Skip`, else `Out::One`). Compiled `AST` cached in a `Mutex<HashMap<u64, Arc<AST>>>` keyed by source hash. Errors → `ctx.error` with Rhai's message and position (`"line 3, column 9: …"`), once per node (not per combination).
- Expression: source = `node.values["Expression"]` (default `"x"`), compiled with `compile_expression`, output port 0 gets the value.

- [ ] **Step 1: Failing tests**: `expression_maps_over_list` (`x*x` with Series 0..4 → 0,1,4,9,16), `expression_radians` (`sin(x)` at `PI/2` → 1), `expression_two_vars_broadcast` (`x+y`, x list of 3, y single), `expression_unknown_variable` (Review Focus 5: `sin(xx)` → error containing `xx` and `column`), `script_infinite_loop_is_cut` (Script source `loop {}` through `solve_node` → error containing `"longer than"` or `"operations"` within 6 s).
- [ ] **Step 2: FAIL. Step 3: Implement.** UI: Expression node shows its formula as an inline text field on the node.
- [ ] **Step 4: PASS; check binary size growth** (`ls -l target/release/triceratops` before/after; note in commit, expect < 3 MB). **Step 5: Commit** `Locust: Rhai engine and Expression node`.

### Task 8: Script node and its geometry API

**Files:**
- Create: `crates/app/src/locust/script/api.rs`
- Modify: `script/mod.rs` (register API, Script source = `node.values["code"]`), `locust/ui.rs` (code editor dialog)
- Test: `script/tests.rs`

**Interfaces:**
- Script outputs = scope variables named like the output ports at the end of the run; `print(x)` → `ctx.print`.
- `api.rs` registers:
  - `point(x, y, z)`, `vec(x, y, z)` (same `Point` type), `rng(seed) -> Rng` with `rng.next()` (0..1, splitmix64 — the same generator Task 10 uses: `crate::locust::components::random::SplitMix64`, created in this task).
  - `run(name: &str, args: Array) -> Dynamic`: calls any registry component except category `Output` and dynamic kinds; element components get the args cast per input kind as one combination; tree components get each arg as a tree (Array → list); returns output 0 (single item unwrapped, several items as Array, none as `()`); `run_all(name, args) -> Array` returns every output. Unknown component / wrong arg count → script error naming the inputs.
  - Shorthands implemented as `run` calls (one line each): `line(a,b)`, `polyline(pts, closed)`, `interpolate(pts)`, `circle(center, r)`, `helix(start, end, r, turns)`, `loft(curves)`, `extrude(geo, dir)`, `revolve(curve, a, b, angle)`, `sweep1(rail, profile)`, `cap(geo)`, `surface_from_grid(rows)` (rows = Array of Arrays of points), `box(corner, x, y, z)`, `cylinder(base, r, h)`, `sphere(c, r)`, `union(a, b)`, `difference(a, b)`, `intersection(a, b)`, `move(g, v)`, `rotate(g, angle_deg, center, axis)`, `scale(g, center, f)`, `mirror(g, origin, normal)`, `array_polar(g, center, count, angle_deg)`, `curve_point(c, t)` (t normalised 0..1), `surface_point(s, u, v)` (normalised), `length(c)`, `area(g)`, `volume(g)`.
- Code editor dialog: monospace multiline, Apply/Cancel, the error line underlined from the node's diagnostic position; open by double-click or "Edit code…".

- [ ] **Step 1: Failing tests**: `script_points_on_circle` (inputs `n`=8, `r`=10; `a = []; for i in 0..n { a.push(point(r*cos(i*2*PI/n), r*sin(i*2*PI/n), 0)) }` → 8 points, all |p| = 10); `script_run_extrude_volume` (`a = extrude(circle(point(0,0,0), 5), vec(0,0,10))` → brep volume ≈ 785.4 ±1%); `script_bad_output_type` (Review Focus 3: output `a = #{k: 1}` → error text from convert, second output `b = 1` still delivered); `script_list_access` (input `pts` with access list receives the whole branch: `a = pts.len()` → one item); `script_cannot_import` (`import "x" as y;` → compile error); `script_boolean_error_propagates` (difference of two huge spheres with `TRICERATOPS_BOOL_BUDGET=100` → error containing "too complex").
- [ ] **Step 2: FAIL. Step 3: Implement. Step 4: PASS. Step 5: Commit** `Locust: Script node with geometry API`.

### Task 9: Helix component

**Files:** Modify `crates/app/src/commands/curve2.rs` (extract), `crates/app/src/locust/components/curve.rs`; test `locust/tests.rs`.

**Interfaces:**
- `pub fn helix_curve(start: DVec3, end: DVec3, radius: f64, end_radius: f64, turns: f64, start_dir: Option<DVec3>) -> Option<NurbsCurve>` in `commands/curve2.rs`, used by `HelixCmd::build` (behaviour unchanged) and the component.
- Component `Helix` (Curve): `Start` pt(0,0,0), `End` pt(0,0,10), `Radius` 5, `Turns` 5 (slider 0.25..50), `Pitch` (opt, overrides Turns = height/pitch), `End Radius` (opt, default = Radius) → `Curve`.

- [ ] **Step 1: Failing tests**: `helix_length_and_ends` (r 5, h 10, 5 turns: length ≈ √((2π·5·5)² + 10²) ±1%, start at (5,0,0) direction X, end z = 10); `helix_pitch_overrides_turns` (pitch 2, h 10 → 5 turns: same length); `helix_command_unchanged` (existing Helix command test still passes — run it).
- [ ] **Step 2: FAIL. Step 3: Implement. Step 4: PASS. Step 5: Commit** `Locust: Helix`.

### Task 10: Populate 2D and Populate Geometry

**Files:** Create `crates/app/src/locust/components/random.rs` (if not created in Task 8) and `crates/app/src/locust/components/populate.rs`; modify `components/mod.rs` (register); tests in `locust/tests.rs`.

**Interfaces:**
- `pub struct SplitMix64(u64)`, `impl SplitMix64 { pub fn new(seed: i64) -> Self; pub fn next_f64(&mut self) -> f64 }`.
- `Populate 2D` (Vector, tree component): `Region` (closed planar curve; default 100×100 rectangle on XY), `Count` int 100, `Seed` int 1 → `Population`. Rejection sampling in the curve's plane against its polyline approximation (`geo` helpers already used by Offset/Area); count capped at 100_000 with warning `"count limited to 100000"`; Count ≤ 0 → empty + warning `"count is 0"`.
- `Populate Geometry` (Vector): `Geometry` (surface/brep/mesh), `Count` 100, `Seed` 1 → `Population`; area-weighted triangle choice on `geo::to_mesh`, uniform barycentric, then snapped with the surface closest-point helper when the input is a brep/surface. Same caps.

- [ ] **Step 1: Failing tests**: `populate_2d_inside_and_deterministic` (circle r 10: 200 points, all |p| ≤ 10, same seed equal, seed 2 different); `populate_2d_count_limits` (Review Focus 4: 0 → empty + warning; 10_000_000 → 100_000 + warning; assert count and warning only); `populate_geometry_on_sphere` (r 10: 300 points, ||p| − 10| < 1e-3).
- [ ] **Step 2: FAIL. Step 3: Implement. Step 4: PASS. Step 5: Commit** `Locust: Populate 2D and Populate Geometry`.

### Task 11: Voronoi and Surface Voronoi

**Files:** Create `crates/app/src/locust/components/voronoi.rs` (`pub fn cells_2d(sites: &[DVec2], boundary: &[DVec2]) -> Vec<Vec<DVec2>>` + components); register in `components/mod.rs`; tests in the new file and `locust/tests.rs`.

**Interfaces:**
- `cells_2d`: for each site, start from the convex boundary polygon (non-convex boundaries: clip each cell against the boundary polygon afterwards with Sutherland–Hodgman per convex part, or reject non-convex with an error `"boundary must be convex"` — choose the latter in this release and document it), clip by the perpendicular-bisector half-plane of every other site (O(n²); fine to 5000 sites; above, warning and cap).
- `Voronoi` (Mesh, tree component, GH names): `Points`, `Boundary` (closed planar convex curve; default = bounding rectangle of the points grown 10%), `Plane` (default XY / fitted to boundary) → `Cells` (closed polylines, one per site, same order as points).
- `Surface Voronoi` (Surface): `Surface`, `Points` (opt), `Count` 50, `Seed` 1 → `Cells` (curves on the surface), `Sites` (points on the surface). Sites: given points → closest UV; else `Populate Geometry`. UV is scaled by the surface's average u and v lengths so cells are not stretched; cells computed with `cells_2d` in scaled UV against the domain rectangle; each edge densified to 8 segments, mapped through `surface.point_at`, interpolated (degree 3, periodic closed) — or polyline if interpolation fails.

- [ ] **Step 1: Failing tests**: `four_sites_four_quadrants` (sites (±25,±25) in 100×100 square → 4 cells of area 2500 each ±1e-6, each contains its site); `cells_tile_the_boundary` (30 Populate 2D points in 100×100, seed 1 → 30 cells, area sum 10000 ±0.1%); `surface_voronoi_on_cylinder` (cylinder patch r 10, 20 sites → 20 cells; 50 sampled points per cell lie within 1e-3 of the cylinder); `voronoi_rejects_concave_boundary` (L-shaped boundary → error diagnostic containing "convex").
- [ ] **Step 2: FAIL. Step 3: Implement. Step 4: PASS; `capture_view`-style visual check** later in Task 12. **Step 5: Commit** `Locust: Voronoi and Surface Voronoi`.

### Task 12: The five example definitions and a live run through MCP

**Files:**
- Create: `crates/app/src/locust/examples/{thread,spur_gear,equation_surface,voronoi,dovetail}.json` (authoring format, nicknamed sliders), `crates/app/src/locust/examples/mod.rs` (`pub const EXAMPLES: &[(&str, &str)]` name → `include_str!`), tests in `locust/examples/mod.rs`
- Modify: `locust/ui.rs` — an "Examples ▾" menu next to Open… loading one via `authoring::prepare` (undoable)

**Interfaces:** Consumes `authoring::prepare`, `engine::solve`, `Ctx::bake`, Tasks 7–11 components.

Each example must solve with zero errors, and its test asserts:
- [ ] **thread** (M10×1.5, length 20; sliders Diameter 10, Pitch 1.5, Length 20; core cylinder r = D/2 − 0.6134·P, ISO 60° triangular profile swept along `Helix`, `Solid Union`): one closed brep in bake; volume strictly between π(r_core)²·L and π(D/2)²·L; solve < 30 s in release.
- [ ] **spur_gear** (Script: involute profile from Module 2, Teeth 20, Pressure Angle 20°; Extrude 10; plus a Twist-based helical variant toggled by `Helix Angle` 0): closed brep; bbox XY diameter ≈ m(z+2) = 44 ±0.3; volume between π(m(z−2.5)/2)²·10 and π(22)²·10.
- [ ] **equation_surface** (Expression `sin(x)*cos(y)`, x,y ∈ [0, 2π] 25×25 via Series, Construct Point, Surface from Grid; second part: Script parametric torus knot tube `(2,3)` swept with Sweep1): surface z at u,v mapping to (π/2, 0) ≈ 1 ±0.02; tube brep exists.
- [ ] **voronoi** (Populate 2D 40 points seed 3 in 120×80 rectangle → Voronoi → Offset Curve −1.5 → Extrude 4 as a frame; and Surface Voronoi on a lofted wavy surface, 30 cells): 40 cells; 30 surface cells.
- [ ] **dovetail** (two Boxes 60×40×20; Script builds the dovetail tail prism (angle 1:6) from Width/Depth/Clearance sliders; part A = Box ∪ tail, part B = Box − (tail offset by Clearance)): both closed; `Solid Intersection` of A and B empty or volume < 1e-6 at Clearance 0.2; at Clearance 0 the parts touch (bbox overlap check).
- [ ] **Step: live run.** Release build, launch `triceratops.sh`, connect an assistant through `triceratops mcp` (this session's `mcp__triceratops__*` tools reconnect to the new binary after restart), build each of the five through `locust_set_graph` from scratch (not by loading the example), tweak one slider each with `locust_set_values`, `capture_view` Perspective for each. Fix what this finds (each fix with a test). Also do one direct-modelling task (a Box with a filleted edge via `run_commands`) to confirm nothing regressed.
- [ ] **Step: Commit** `Locust: five example definitions`.

### Task 13: Docs, version, PR

**Files:** `README.md` (Locust section: Script, Expression, new components; MCP section: two ways to model, the six tools), `docs/FEATURES.md`, `site/` copy if it lists features, root `Cargo.toml` version `0.3.0`; screenshot of a Voronoi or gear definition via `site/tools/shoot.sh` with `TRICERATOPS_BACKEND=gl` for the README/site.

- [ ] **Step 1:** Update docs; `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Step 2:** Commit `Docs and version 0.3.0`, push `locust-mcp`, open the PR (body ends with the Claude Code line), wait for CI + Build on Linux/Windows/macOS.
- [ ] **Step 3:** Report to the user; merge/tag only after their go-ahead (memory: merge & release on green).
