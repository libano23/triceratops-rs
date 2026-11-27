# Exact solids on the solidcore kernel — design

Status: draft for approval, 2026-11-27. Evaluation and spike evidence below; nothing in the product is built yet.

## Why

Triceratops solids are NURBS faces with trim loops stored as **polylines in (u, v)** and no edge
objects. Booleans run on face tessellations and the surviving pieces are turned back into trimmed
faces. Consequences the owner does not accept ("I want clean, precise solids, not mesh
approximations"):

- intersection edges are polylines, never analytic curves;
- results can be wrong while reported as `closed solid polysurface`. Measured 2026-11-27 against
  analytic or Monte Carlo volumes: sphere minus two cylinders **+11 %**, torus minus a bore along
  its tube **+5.9 %**; simple cases were within 0.2 % (tessellation-limited);
- no persistent topology to build exact fillets, offsets, shells on.

## Evidence: the solidcore kernel

solidcore (Apache-2.0, Rust, a port of NVIDIA's SMLib, vendored inside tangency-labs/solidcraft)
is a real B-rep kernel: faces on analytic and NURBS surfaces, edges with 3D curves and pcurves,
regions, tolerances, STEP I/O, fillets/offsets/shell/sweep/loft. Tested 2026-11-27 through
`solidcraft-cli` 0.51:

- 12 analytic/tangent/coincident cases (equal-radius crossing cylinders, tricylinder, flush hole,
  tangent cylinder, sphere tangent to a box's six faces, tori, oblique cylinder, shell): all within
  1e-5 % of the exact volume, or within the Monte Carlo noise (0.03 %) where no formula exists;
  0.08–1 s each.
- free-form: a 3-section spline loft combined with an oblique cylinder, a sphere and a second loft:
  the identities (L−T)+(L∩T)=L and L∪T=L+T−L∩T hold to 0.0000 mm³; 0.3–0.5 s each.
- limits: a loft against itself shifted by 0.001 mm (near-coincident free-form faces) is **refused
  with an error** for ∩, − and ∪ (no wrong result); a modelled M8 thread takes ~11 s.
- cost: `solidcore-ops` alone builds in 4 min 30 s wall (≈30 CPU-min) release on 16 cores; the
  whole solidcraft-cli, 13 min 43 s. API is 0.0.x; its own notes say deferred paths limit coverage
  and some coincident curved configurations still fail.

Not yet tested: Booleans between free-form NURBS bodies that come from Triceratops' own surfaces
(e.g. a Rhino-style loft with rails), STEP/3dm round trips of results, behaviour on Windows.

## Goals

1. Exact solids: results carry true edges (analytic where the geometry is), not polylines.
2. Booleans (union, difference, intersection, split), WireCut, holes, Boss, Rib, PushPull and Shell
   give correct volumes on the cases above and stay closed solids.
3. Existing documents, every existing command and file format keep working unchanged.
4. A change is only shipped when the comparison cases (below) pass.

## Non-goals

- Replacing NURBS surfaces and polysurfaces (open, non-solid work stays as is).
- Importing existing polyline-trimmed solids exactly: impossible without edge curves. They stay
  legacy and keep the current Boolean as fallback.
- A parametric history/timeline (that is SolidCraft's product, not ours).
- Mesh-based engines (Manifold/OpenSCAD style): they approximate by design.
- Windows/macOS parity in the first milestone (Linux first; see Risks).

## Anti-goals

- Silent wrong results. A refusal with a clear message is better than a bad solid.
- Making the app slower to start or the AppImage much larger without the owner choosing it.
- Forking solidcore. We depend on it; fixes go upstream.

## Approaches considered

1. **New `Geometry::Solid` variant.** Cleanest type, but 219 matches on `Geometry::Brep` across 40
   files must each be revisited (display, selection, transform, save, 3dm/STEP/IGES, Locust,
   every command). Large and risky for a first step.
2. **Exact "shadow" on the existing `Brep` (recommended).** `Brep` gains an optional exact body
   next to its faces. Everything that reads faces keeps working untouched; the exact body is used
   only by solid operations. A fingerprint of the faces guards against staleness.
3. **Use solidcore as an opaque Boolean engine over legacy solids.** Rejected: legacy trims are
   polylines, so conversion in is inexact and results would be no better than today's.

The owner chose "native solids on solidcore" (2026-11-27); approach 2 is how it is built, because it
reaches the same behaviour with a reviewable first slice and can later be promoted to approach 1 if
the shadow proves limiting. *(This how-choice is the builder's call, flagged for the owner to
confirm when approving this document.)*

## Design

### Data model

`tri_geom::Brep` becomes:

```text
Brep { faces, closed, exact: Option<ExactBody> }      // serde default: None
ExactBody { record: Arc<solidcore_record::Brep>, faces_fingerprint: u64 }
```

- The `faces` stay authoritative for everything that is not a solid operation (display, curve
  extraction, export, selection, legacy commands). They are generated **from** the exact body: each
  record face → its surface as `NurbsSurface` (analytic quadrics as exact rational NURBS, the way
  Triceratops already makes spheres/cylinders) with trim loops sampled from the edges at the
  tessellation tolerance.
- `faces_fingerprint` hashes the faces (control points, knots, loops). An exact body is **valid only
  while the fingerprint matches**. Any legacy command that edits faces in place simply invalidates
  it: the object degrades to today's behaviour, never to a wrong exact body.
- Rigid transforms (Move, Rotate, Mirror, Scale uniform, Copy, Array) are the common case, so they
  are taught to transform the record too and refresh the fingerprint. Non-rigid edits drop it.
- Undo already clones `State`, so the record sits behind `Arc` (cheap clones). Save: the record is
  serialised with its own serde (solidcore's `deny_unknown_fields` schema) inside the document JSON,
  versioned; older files simply have `exact: None`.

### New crate `tri-kernel`

One crate owns every solidcore dependency (mirrors solidcraft's `kernel-api` seam so no other
crate sees solidcore types):

```text
tri-kernel: Solid (wraps ExactBody)
  primitives: box, cylinder, sphere, cone, torus, ellipsoid
  from curves: extrude(closed planar curve), revolve, loft, sweep (NurbsCurve → record curves)
  boolean(a, b, op) -> Result<Solid, KernelError>
  transform(&Solid, DAffine3), volume(&Solid), to_faces(&Solid, tol) -> Vec<Face>
  from_brep(&Brep) -> Option<Solid>       // Some only if exact is present and fingerprint matches
```

`tri-geom` does not depend on solidcore; it only stores the opaque record (type from
`solidcore-record`, a small data-only crate). Solid operations live in `tri-kernel`, called by
`tri-app`.

### Where it plugs in

- `boolean_geometry_checked` (the single Boolean entry, `commands/misc4.rs`): if both operands have
  a valid exact body → `tri-kernel::boolean`; otherwise the current pipeline, unchanged.
- Primitive creation (`create.rs`), `prism_brep` (feeds WireCut, holes, Boss, Rib) and the
  Extrude/Revolve/Loft/Sweep solid paths build an exact body next to the faces when the inputs allow;
  otherwise no exact body.
- Failure policy: if solidcore errors or exceeds its budget, report the kernel's message; do **not**
  silently fall back to the mesh Boolean for exact operands in the first milestone (a wrong solid is
  worse than a refusal). Legacy operands keep the legacy path. The owner may later ask for a fallback.
- Cancellation: kernel calls run in the existing background-job machinery; solidcore has its own
  cancel/fuel hooks (`solidcore-ops::cancel`, `fuel`) wired to Esc.

### Display and mesh

Display meshes come from `to_faces` (faces keep their original surfaces; trims are sampled from
exact edges, so both sides of an edge share identical sample points — no cracks). Volume and
area come from the kernel (`measure`), replacing `volume_of`/`area_of` on the display mesh for
exact solids.

## Milestones

1. **Kernel seam** (Linux only): `tri-kernel` with primitives + Boolean + faces conversion +
   volume, behind a cargo feature `kernel-solidcore`, off in release until M3. Passing the
   comparison cases through a test harness. *Exit: all cases below within tolerance.*
2. **Plug-in**: `exact` field on `Brep`, primitives, `prism_brep`, `boolean_geometry_checked`,
   transforms, undo/save/load. *Exit: the two known-bad cases correct in the real app, verified
   through the MCP tools.*
3. **Operations**: extrude/revolve/loft/sweep/PushPull/Shell/holes/WireCut on exact bodies; feature
   on by default; release pipeline (AppImage + Windows) measured.
4. **Later, separately decided**: fillets and offsets from solidcore, STEP round trip of exact
   bodies, promote shadow to a first-class type.

## Acceptance cases

The 12 analytic/tangent/coincident cases and 4 free-form identity cases from the evaluation, run as
`cargo test` on `tri-kernel`; tolerance 1e-3 % for analytic cases, 0.05 % for Monte Carlo cases,
identities to 1e-6 mm³. Plus the two known-bad Triceratops cases (sphere − two cylinders, torus −
bore) in the real app. The scripts used in the evaluation (`run_cases.py`, `freeform.py`) are the
starting point; they will be turned into Rust tests.

## Risks

- **Build cost.** +4.5 min wall / 30 CPU-min per clean release build (more for the whole
  workspace on CI runners with fewer cores); binary size not yet measured. Mitigation: cargo
  feature, `sccache`, and measure before M3.
- **API stability.** solidcore is 0.0.x. Pin an exact commit, vendor like solidcraft does, upgrade
  deliberately.
- **Near-coincident free-form faces** are refused today (seen in testing). Surface as a clear
  message; report upstream.
- **Thread/helical Booleans** are slow (~11 s for M8). Run as cancellable jobs; compare to our
  helical-sweep Boolean before promising parity.
- **Windows/macOS** unbuilt. M1 is Linux only; cross-platform is an M3 exit criterion. macOS is
  already dropped from releases after 0.3.0.
- **Free-form NURBS from our own Loft/Sweep** may differ from solidcraft's constructions; the
  conversion `NurbsCurve → record curve` is new code and must be tested on them.

## Left to the builder

Exact record ↔ `Face` conversion details (seam handling, degenerate poles, orientation), how
analytic surfaces are represented when a face is later edited, the precise fingerprint function,
module layout inside `tri-kernel`, and the test-harness plumbing.

## Open points for the owner

1. Confirm approach 2 (shadow on `Brep`) as the way to build option 1.
2. Confirm "no silent mesh fallback for exact operands" (refuse with the kernel's message).
3. Confirm Linux-first, cross-platform only at M3.
