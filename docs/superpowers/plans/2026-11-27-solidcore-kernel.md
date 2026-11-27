# Exact solids on solidcore — Implementation Plan (milestones M1 + M2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Solids made by Triceratops primitives and planar extrusions carry an exact solidcore B-rep, and Booleans between two such solids run on the kernel and give correct volumes.

**Architecture:** A new crate `tri-kernel` wraps solidcore (primitives, planar-curve extrusion, Boolean, transform, volume, record → `Face` conversion). `tri_geom::Brep` gains an optional `exact` body (an `Arc` of solidcore's data-only record plus a fingerprint of the faces); legacy code reads faces as today and an exact body is trusted only while the fingerprint matches. `boolean_geometry_checked` routes exact × exact to the kernel and everything else to the current pipeline. Everything solidcore-shaped is behind the cargo feature `kernel-solidcore` (off by default until M3).

**Tech Stack:** Rust 1.99.1, glam `DVec3`/`DAffine3`, serde, solidcore (`solidcore-ops`, `solidcore-record`, `solidcore-tess`) as a git dependency pinned to the tangency-labs/solidcore mirror at `85298b6c435a64def982c944ccee213d94feeb97`.

**Spec:** `docs/superpowers/specs/2026-11-27-solidcore-kernel-design.md`. M3 (extrude/revolve/loft/sweep/PushPull/Shell/holes on exact bodies, default-on, Windows) gets its own plan after M2 ships.

## Global Constraints

- Linux first; Windows is an M3 exit criterion. Feature `kernel-solidcore` is **off by default** (spec: "off in release until M3").
- No silent mesh fallback for exact operands: on kernel error show the kernel's message, leave both inputs in the document (spec, owner-confirmed).
- Legacy operands (no valid exact body) keep today's Boolean unchanged.
- Old documents (no `exact` field) must load; `exact` is `#[serde(default)]`.
- Acceptance tolerances: 1e-3 % of the exact volume for analytic cases; 0.05 % for Monte Carlo reference cases; Boolean identities to 1e-6 mm³.
- Commit as the repo's configured identity (`libanesevolante@pm.me`); every commit ends with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Work on branch `solidcore-kernel`; do not merge or tag without the owner's go-ahead.
- Never fork solidcore; pin the git rev, bump deliberately.

## Review Focus

1. A kernel refusal (e.g. two near-coincident free-form faces) must leave both inputs in the document and show the kernel message — not delete inputs (Boolean has `DeleteInput=Yes`) and not fall back to mesh. → Task 7.
2. Stale exact body: a legacy command edits `faces` in place → `Brep::exact_valid()` must be false and the Boolean must take the legacy path, never the kernel. → Task 5.
3. Documents saved before this change (no `exact`) load and Boolean as before. → Task 5.
4. Esc/cancel during a kernel Boolean: no partial result, inputs intact. → Task 7.
5. Extreme scale: a 0.05 mm box and a 20 000 mm box still give volumes within tolerance and a closed solid. → Task 3.

---

## File structure

- Create `crates/kernel/Cargo.toml`, `crates/kernel/src/lib.rs` (public API, `Solid`, `KernelError`), `convert.rs` (glam/`NurbsCurve` ↔ solidcore types), `faces.rs` (record → `tri_geom::Face`), `tests/acceptance.rs` (analytic + identity cases).
- Modify `Cargo.toml` (workspace member), `crates/geom/Cargo.toml` + `crates/geom/src/brep.rs` + new `crates/geom/src/exact.rs` (`ExactBody`, fingerprint), `crates/app/Cargo.toml` (feature), `crates/app/src/commands/{create.rs,misc4.rs,transform.rs}`, `crates/app/src/commands/flow_tests.rs`, `docs/FEATURES.md`, `CHANGELOG.md`.

---

### Task 1: `tri-kernel` crate with a pinned solidcore dependency

**Files:** Create `crates/kernel/Cargo.toml`, `crates/kernel/src/lib.rs`; Modify `Cargo.toml` (workspace `members`).

**Interfaces:**
- Produces: crate `tri-kernel`, feature `solidcore` (optional deps `solidcore-ops`, `solidcore-record`, `solidcore-tess`, all `git = "https://github.com/tangency-labs/solidcore", rev = "85298b6c435a64def982c944ccee213d94feeb97"`); with the feature off the crate is empty.

- [ ] **Step 1: Write the failing test** `kernel_links_and_makes_a_box` in `crates/kernel/src/lib.rs` (`#[cfg(all(test, feature = "solidcore"))]`): `solidcore_ops::body::make_box([0.0;3], [2.0, 3.0, 4.0])` then `body.volume(1e-6)` equals 24.0 within 1e-9.
- [ ] **Step 2: Run** `cargo test -p tri-kernel --features solidcore` → FAIL (crate/dependency missing).
- [ ] **Step 3: Add the crate and dependencies** as above; edition from workspace. If cargo cannot resolve the git workspace member, add `package = "solidcore-ops"` explicitly. The first build is ~5 min; run it once to warm the cache.
- [ ] **Step 4: Run** the test again → PASS. Also `cargo build -p tri-kernel` (feature off) succeeds quickly.
- [ ] **Step 5: Commit** `Add tri-kernel crate with pinned solidcore dependency`.

### Task 2: `Solid`: primitives, volume, transform

**Files:** Modify `crates/kernel/src/lib.rs`; Create `crates/kernel/src/convert.rs`.

**Interfaces:**
- Produces (`#[cfg(feature = "solidcore")]`):
  - `pub struct Solid` (wraps `solidcore_ops::Body`; `Clone` via `Body::copy`).
  - `pub enum KernelError { Refused(String), Cancelled }` with `Display` = the kernel's message.
  - `Solid::cuboid(min: DVec3, max: DVec3) -> Result<Solid, KernelError>`; `Solid::cylinder(base: DVec3, axis: DVec3, radius: f64, height: f64)`; `sphere(center, radius)`; `cone(base: DVec3, axis: DVec3, base_radius, top_radius, height)`; `torus(center: DVec3, axis: DVec3, major, minor)`.
  - `Solid::volume(&self) -> Result<f64, KernelError>`; `Solid::transformed(&self, xf: &DAffine3) -> Result<Solid, KernelError>`.
  - `convert::matrix(xf: &DAffine3) -> [[f64; 4]; 4]` (the layout `Body::transform` expects — verify column-major vs row-major against solidcore's `affine` module and pin it with a test).

- [ ] **Step 1: Write failing tests** in `crates/kernel/tests/acceptance.rs`: `cylinder_volume` (r=3, h=7 → π·9·7), `sphere_volume`, `torus_volume` (2π²·M·m²), `cone_volume`, `axis_arbitrary` (cylinder along `[1,0.25,0.15]` has the same volume), `transform_moves_bbox` (translate then rotate 90° about Z keeps volume, moves a box's centre — read the bbox from `to_record` vertices).
- [ ] **Step 2: Run** `cargo test -p tri-kernel --features solidcore` → FAIL.
- [ ] **Step 3: Implement** with `solidcore_ops::body::{make_box,make_cylinder,make_sphere,make_cone,make_torus}` (they place on Z; orient/translate with `Body::transform` for other axes). Map every `solidcore_ops::Error` to `KernelError::Refused(e.to_string())`, `Error::Cancelled` to `Cancelled`.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `tri-kernel: primitives, volume, transform`.

### Task 3: Boolean and the analytic acceptance cases

**Files:** Modify `crates/kernel/src/lib.rs`, `crates/kernel/tests/acceptance.rs`.

**Interfaces:**
- Consumes: Task 2 `Solid`.
- Produces: `pub enum BooleanOp { Union, Difference, Intersection }`; `Solid::boolean(&self, other: &Solid, op: BooleanOp) -> Result<Solid, KernelError>` (inputs are not consumed: copy first; an empty result is `Refused("the result is empty")`).

- [ ] **Step 1: Write failing tests** (one `#[test]` each, tolerances from Global Constraints): `steinmetz_two_cylinders` (r=10, intersection = 16/3·r³ = 5333.333…), `tricylinder` (8(2−√2)r³), `crossing_cylinders_union` (2πr²·40 − 16/3 r³), `flush_hole` (40×40×20 box minus r=8 cylinder of height 20: 32000−π·64·20 … use the exact expression `40.0*40.0*20.0 - PI*64.0*20.0`), `tangent_cylinder_wall`, `sphere_tangent_to_box_faces` (box 40³ minus sphere r=20), `two_spheres_union` (lens formula π(4R+d)(2R−d)²/12), `sphere_inside_sphere_difference`, `scale_extremes` (a 0.05 mm box minus a 0.02 mm-radius cylinder, and a 20000 mm box minus a 5000 mm sphere; both volumes within tolerance, result faces > 0). Use the same geometry as `/tmp` evaluation scripts `run_cases.py` (parameters are in the spec's evidence section).
- [ ] **Step 2: Run** → FAIL (no `boolean`).
- [ ] **Step 3: Implement** via `solidcore_ops::boolean::boolean(a.copy(), b.copy(), op)`; `Ok(None)` → empty-result error.
- [ ] **Step 4: Run** → PASS (all). If a case fails on the pinned rev but passed in the evaluation (solidcraft's vendored copy), record the delta in the task commit message and stop: the pin needs another rev.
- [ ] **Step 5: Commit** `tri-kernel: Boolean with analytic acceptance cases`.

### Task 4: Planar-curve extrusion and the free-form identity cases

**Files:** Modify `crates/kernel/src/{lib.rs,convert.rs}`, `tests/acceptance.rs`.

**Interfaces:**
- Produces: `Solid::extrude_closed_curve(curve: &tri_geom::NurbsCurve, plane: &tri_geom::Plane, half: f64) -> Result<Solid, KernelError>` (symmetric about the plane, matching `prism_brep`'s contract in `commands/misc4.rs`); `convert::region2(curve: &NurbsCurve, plane: &Plane) -> Result<solidcore_ops::…::Region2, KernelError>` (loop of lines/arcs/cubic splines through `solidcore_ops::prism::fitted_loop` when the curve is not exactly representable; refuse non-planar or self-crossing).

- [ ] **Step 1: Write failing tests**: `extrude_rectangle_volume`, `extrude_circle_volume` (π r² · 2·half), `extrude_spline_blob_identities` — a periodic cubic spline blob (8 fit points `blob(r,k,phase)` from the evaluation) extruded, then for tools {oblique cylinder, sphere straddling the edge, a second shifted blob extrusion}: assert `(L−T)+(L∩T) = L` and `(L∪T) = L+T−(L∩T)` to 1e-6 mm³ (the free-form identity cases of the spec).
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** with `solidcore_ops::prism::extrude_planar_regions(plane, regions, -half, half)`; build the solidcore `Plane` from `tri_geom::Plane` origin/axes.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `tri-kernel: planar curve extrusion and free-form identities`.

### Task 5: `ExactBody` on `Brep`, fingerprint, serde

**Files:** Create `crates/geom/src/exact.rs`; Modify `crates/geom/src/{lib.rs,brep.rs}`, `crates/geom/Cargo.toml` (unconditional dep `solidcore-record`, same pinned git rev), `crates/doc/src/io.rs` tests.

**Interfaces:**
- Produces: `pub struct ExactBody { pub record: Arc<solidcore_record::brep::Brep>, pub faces_fingerprint: u64 }` (`Clone, Debug, PartialEq, Serialize, Deserialize`; `Arc` serde via the `rc` feature or a manual impl); `Brep { faces, closed, #[serde(default)] pub exact: Option<ExactBody> }`; `Brep::fingerprint(&self) -> u64` (hash of faces: control points, weights, knots, loops, as raw f64 bits); `Brep::exact_valid(&self) -> Option<&ExactBody>` (Some only when stored fingerprint equals the current one); `Brep::attach_exact(&mut self, record: Arc<…>)` (stamps the current fingerprint). `Brep::new`/`from_surfaces` set `exact: None`; `Brep::transform` clears `exact` (Task 9 re-attaches).

- [ ] **Step 1: Write failing tests** in `brep.rs` tests: `exact_valid_until_faces_change` (attach; `exact_valid` is Some; mutate one control point; now None), `serde_roundtrip_keeps_exact`, `old_json_without_exact_loads` (a JSON string of a `Brep` without the field deserialises with `exact == None`), and in `crates/doc/src/io.rs` `old_document_loads_and_saves_exact` (save/load a doc holding an exact Brep; a hand-written v-old document still loads).
- [ ] **Step 2: Run** `cargo test -p tri-geom -p tri-doc` → FAIL.
- [ ] **Step 3: Implement.** Fix every struct-literal construction of `Brep` that now needs the field (compiler lists them: add `exact: None`).
- [ ] **Step 4: Run** the whole workspace tests: `cargo test --workspace` → PASS.
- [ ] **Step 5: Commit** `Brep: optional exact body with face fingerprint`.

### Task 6: Record → `Face` conversion

**Files:** Create `crates/kernel/src/faces.rs`; Modify `crates/kernel/src/lib.rs`.

**Interfaces:**
- Consumes: Task 2 `Solid`; solidcore `Body::to_record`.
- Produces: `Solid::to_brep(&self, tolerance: f64) -> Result<tri_geom::Brep, KernelError>` returning a closed `Brep` with `exact` attached (via `attach_exact`) and one `Face` per record face: surface as `NurbsSurface` (record `Spline`/`RationalSpline` → direct; `Plane`, `Cylinder`, `Cone`, `Sphere`, `Torus` → exact rational NURBS covering the face's domain; `Blend`/`PlacedRevolution`/others → use the face's `nurbs` form), trim loops sampled from the 3D edges (each edge sampled **once** and shared by both faces so adjacent trims have identical points), projected onto the face's surface to (u, v) (reuse the closest-point routine used in `brep_boolean_exact.rs`; handle periodic seams like the existing sphere/cylinder faces do). `Solid::from_exact(brep: &tri_geom::Brep) -> Option<Solid>` (via `exact_valid` + `Body::from_record`).

- [ ] **Step 1: Write failing tests** in `tests/acceptance.rs`: for box, cylinder, sphere, torus, cone, `sphere − two cylinders` (the known-bad case) and the spline-blob prism: `brep.closed`, `brep.volume()` (the display-mesh volume already used by `volume_of`) within 0.2 % of `Solid::volume`, every face has ≥ 1 loop for non-periodic faces, and `from_exact(&brep)` is Some and has the same kernel volume (round trip).
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** `faces.rs`. Convert quadrics with the analytic NURBS constructors that already exist in `tri-geom` (`NurbsSurface::sphere`, extrude/revolve helpers) when parameter conventions line up; otherwise build the rational patch directly. Orient faces outward by the shell's `forward` flag.
- [ ] **Step 4: Run** → PASS. Also open each converted Brep in the app through the MCP tools once (Task 10) to see no cracks.
- [ ] **Step 5: Commit** `tri-kernel: record to NURBS faces with shared trim samples`.

### Task 7: Boolean hook with kernel errors and cancellation

**Files:** Modify `crates/app/Cargo.toml` (`[features] kernel-solidcore = ["dep:tri-kernel", "tri-kernel/solidcore"]`, `tri-kernel = { path = "../kernel", optional = true }`), `crates/app/src/commands/misc4.rs` (`boolean_geometry_checked`), `crates/app/src/commands/flow_tests.rs`.

**Interfaces:**
- Consumes: `Brep::exact_valid`, `Solid::from_exact`, `Solid::boolean`, `Solid::to_brep`.
- Produces: in `boolean_geometry_checked`, under `#[cfg(feature = "kernel-solidcore")]`: when both operands are `Geometry::Brep` with valid exact bodies → kernel path returning `Ok(Geometry::Brep(result))`; kernel `Refused(msg)` → `Err(msg)` **with no fallback**; `Cancelled` → `Err("cancelled".into())`.

- [ ] **Step 1: Write failing flow tests** (feature-gated, `Harness` style) `exact_sphere_minus_two_cylinders` (volume within 0.05 % of the Monte Carlo value 23658.8 — use the analytic-per-case expectation from the spec's table), `exact_torus_minus_bore` (9163.74 ± 0.05 %), `legacy_operand_uses_legacy_path` (a hand-built legacy Brep Boolean returns the same result as before), `kernel_refusal_keeps_inputs` (two near-coincident spline-prism bodies, 0.001 mm shift: the command reports the kernel message, ids of both inputs still present in the document), `cancelled_boolean_keeps_inputs` (cancel the job before completion).
- [ ] **Step 2: Run** `cargo test -p triceratops --features kernel-solidcore flow` → FAIL.
- [ ] **Step 3: Implement** the hook. Where the Boolean commands delete inputs on success (`DeleteInput=Yes`), the error path must not reach that code; verify by the refusal test. Run kernel calls inside the existing job machinery (`geom::job`), wiring Esc to `solidcore_ops::cancel`.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `Booleans between exact solids run on solidcore`.

### Task 8: Primitives and `prism_brep` attach exact bodies

**Files:** Modify `crates/app/src/commands/create.rs` (Box, Cylinder, Sphere, Cone, Torus creation), `crates/app/src/commands/misc4.rs` (`prism_brep`), `flow_tests.rs`.

**Interfaces:**
- Consumes: Task 2/4 constructors, Task 6 `to_brep`.
- Produces: under the feature, each of those commands returns a `Brep` from `Solid::…::to_brep(tol)` (so `exact` is attached); without the feature or on kernel refusal, the current construction is used unchanged.

- [ ] **Step 1: Write failing flow tests**: `box_command_makes_exact_solid` (after `Box` with the standard prompts the document's Brep has `exact_valid().is_some()`), the same for Cylinder/Sphere/Torus/Cone, and `wirecut_uses_exact_prism` (WireCut of a closed planar curve through an exact sphere gives volume within tolerance of the identity `V(sphere) − V(prism∩sphere)`).
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement**; keep legacy constructors for the no-feature build.
- [ ] **Step 4: Run** → PASS (feature on) and `cargo test --workspace` (feature off) → PASS.
- [ ] **Step 5: Commit** `Primitives and planar prisms carry exact bodies`.

### Task 9: Transforms keep the exact body

**Files:** Modify `crates/app/src/commands/transform.rs`, `crates/geom/src/brep.rs` (`transform`), `flow_tests.rs`.

**Interfaces:**
- Produces: for Move, Rotate, Mirror, uniform Scale and Copy/Array copies of an exact Brep: faces transformed as today **and** the record transformed with `Solid::transformed`, re-attached so `exact_valid()` stays Some. Non-rigid/non-uniform transforms leave `exact == None`.

- [ ] **Step 1: Write failing flow tests** `move_keeps_exact`, `rotate_keeps_exact`, `mirror_flips_orientation_and_keeps_volume` (mirror reverses orientation: kernel volume stays positive), `nonuniform_scale_drops_exact`, and `boolean_after_move_is_exact` (move a cylinder then subtract: kernel path taken — assert via the result's `exact_valid`).
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.** Where a transform is applied by the document layer, add the exact update in one place (`Brep` helper `transformed_exact(&self, xf) -> Option<…>` used by the commands) rather than per command.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `Rigid transforms keep exact bodies valid`.

### Task 10: Persistence, real-app check, docs

**Files:** Modify `crates/doc/src/io.rs` tests, `docs/FEATURES.md`, `CHANGELOG.md` (Unreleased), `docs/superpowers/specs/2026-11-27-solidcore-kernel-design.md` (status line).

- [ ] **Step 1: Write failing test** `exact_survives_undo_and_save`: create exact solid, Boolean, undo, redo, save, load: the Brep still has a valid exact body and the Boolean still routes to the kernel.
- [ ] **Step 2: Run** (feature on) → FAIL or PASS; fix whatever it exposes.
- [ ] **Step 3: Real-app check:** build with `--features kernel-solidcore`, open the app, and through the Triceratops MCP tools run the two known-bad cases (sphere − two cylinders; torus − bore along its tube) and record volume, face count, and a `capture_view` image in the PR description. Also try WireCut with a closed spline on an exact sphere.
- [ ] **Step 4: Docs:** `FEATURES.md` section "Exact solids (experimental, feature `kernel-solidcore`)": what is exact, fallbacks, the refusal policy; `CHANGELOG.md` Unreleased entry.
- [ ] **Step 5: Commit** and open a PR to `master`. Do not merge until the owner approves; report build-time and binary-size deltas measured with and without the feature (spec risk #1).

---

## Self-review

- **Spec coverage:** data model (T5), `tri-kernel` API incl. primitives/extrude/boolean/transform/volume/faces (T1–T4, T6), plug-in points `boolean_geometry_checked` / primitives / `prism_brep` (T7–T8), transforms (T9), undo/save (T5, T10), failure policy and cancellation (T7), acceptance cases (T3–T4, T7), milestones M1 (T1–T4, T6) and M2 (T5, T7–T10), risks (build-time and size measured in T10; near-coincident refusal tested in T7). Left to M3: revolve/loft/sweep/PushPull/Shell/holes, default-on, Windows, fillets/offsets/STEP. Spec's "volume and area from the kernel" is covered for volume (T2); area through the kernel `measure` family is **deferred to M3** with the display of properties.
- **Types:** `Solid`, `KernelError`, `BooleanOp`, `ExactBody`, `exact_valid`, `attach_exact`, `to_brep`, `from_exact` are named identically wherever used.
