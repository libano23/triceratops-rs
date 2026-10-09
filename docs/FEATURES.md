# Triceratops: feature notes

Detailed, command-by-command notes on what is implemented and how. The overview is in
the [README](../README.md).

## Native .3dm files

`File > Open / Save / Import / Export` read and write Triceratops `.3dm` archives through
openNURBS 8 (vendored in `third_party/opennurbs`, MIT, built by `crates/on3dm`
with CMake on the first `cargo build`): points, NURBS curves and surfaces,
polysurfaces with trims and shared topology (closed solids stay solids), meshes,
layers, object names/colours, units, tolerance, notes, block definitions and
instances. Extrusions are read as polysurfaces. Annotation and light objects are
skipped with a message.

## 2D drafting base

DXF and DWG import/export go through the [cadcraft](https://github.com/storytold/cadcraft)
crates (MIT OR Apache-2.0): lines, points, circles, arcs, ellipses, polylines with
bulges, splines and block inserts are converted to NURBS curves and back
(`File > Open`, `Import`, `Export`). Dimensions, hatches and text from cadcraft
are the next 2D features to wire in.

## What works

Window: menu bar, command history + prompt with clickable `( Option=Value )`
options and autocomplete, toolbar tabs (Standard, CPlanes, Set View, Display,
Select, Viewport Layout, Visibility, Transform, Curve/Surface/Solid/Mesh tools,
Drafting), Main sidebar (reference order, flyouts, left/right-click commands),
4View / 3View / maximized viewports with title menus and viewport tabs,
Osnap control (End Near Point Mid Cen Int Perp Tan Quad Knot Vertex Project
Disable, Shift+click = one shot, Alt suspends), status bar panes
(CPlane/World, x y z, units/Delta, Layer, Grid Snap, Ortho, Planar, Osnap, ...),
right container panels (Properties object/view pages, Layers table, Display,
Help, Notes), Options and Document Properties dialogs, Loft Options dialog,
Command History (F2), middle-mouse Popup toolbar.

Input: `x,y,z`, `r`/`@` relative, `w` world, `wr`, polar `d<a`, distance and
angle constraints, elevator mode (Ctrl+click), Tab direction lock, Ortho (F8,
Shift toggles), Grid Snap (F9), Planar, command-line calculator.

Selection: click, Shift add, Ctrl remove, window (left→right) and crossing
(right→left, dashed) selection, selection menu for overlapping objects,
drag-move of selected objects, Esc deselect.

Navigation: right drag rotates (perspective) / pans (parallel), Shift+right
pans, Ctrl+right zooms, wheel zooms at cursor, arrows / PageUp / PageDown,
Home/End view undo/redo, Ctrl+Tab next viewport, Ctrl+M maximize.

Objects follow three levels: curves, surfaces and solids (closed
surfaces and closed polysurfaces). Closed planar curves extrude to solids,
cylinders/cones are capped, `Cap` closes planar openings and `PlanarSrf`
builds trimmed planar faces with holes. The Properties panel and `What`
report "open curve", "closed surface", "closed solid polysurface", ...

All 1069 reference command names are registered and every one of them does
something: modelling commands create or edit geometry, UI commands drive the
interface, and features that have no Linux/triceratops equivalent (Grasshopper,
licensing, digitizers, texture mapping, layouts, plug-ins, lights, materials)
print a one-line message that names the closest triceratops alternative.

Commands (Triceratops prompts and options): Point(s), Line, Lines, Polyline,
Rectangle, Circle, Arc, Arc3Pt, Ellipse, Polygon, Curve, InterpCrv, SrfPt,
Plane, PlanarSrf, Box, Sphere, Cylinder, Cone, Torus, ExtrudeCrv, ExtrudeSrf,
Revolve, Loft, Cap, OffsetSrf, BooleanUnion/Difference/Intersection (mesh
based), Trim, Split, Extend, Fillet, Chamfer, Offset, Rebuild, Divide,
Intersect, Join, Explode, PointsOn/PointsOff (drag control points), Delete,
Undo, Redo, Move, Copy, Rotate, Scale, Scale1D, Scale2D, Mirror, Hide, Show,
Lock, Unlock, Flip/Dir, Sel*, Layer, ChangeLayer, Properties, What, Distance,
Length, Zoom*, 4View, 3View, MaxViewport, SetView, display modes, Grid, Snap,
Ortho, Planar, Osnap, New/Open/Save/SaveAs/Import/Export, Help,
CommandHistory, CommandList, Options (Light/Dark scheme), DocumentProperties,
Sel* (OpenCrv, ClosedCrv, Line, Polyline, OpenSrf, ClosedSrf, Open/ClosedPolysrf,
Dup, Prev, Name, Small, ID, Layer, Window, Crossing, Group), LayerOn/Off/Lock/Unlock,
OneLayerOn, NewLayer, SetLayerToObject, CopyToLayer, Group/Ungroup/AddToGroup/
RemoveFromGroup/SetGroupName, Cut/CopyToClipboard/Paste, HideSwap, ShowSelected,
Isolate/Unisolate, Purge, SetObjectName, BoundingBox, Area, AreaCentroid, Volume,
VolumeCentroid, Angle, Radius, EvaluatePt, ZoomWindow, ZoomTarget, NamedView,
PlanView, Next/PrevViewport, List, Array, ArrayPolar, ArrayLinear, ArrayCrv,
Orient, Orient3Pt, Rotate3D, Shear, SetPt, ProjectToCPlane, Align, Helix, Spiral,
BlendCrv, Connect, ChangeDegree, CloseCrv, Smooth, SimplifyCrv, FitCrv,
InsertKnot, CurveThroughPt, Project, DupEdge, DupBorder, Section, Contour,
EdgeSrf/NetworkSrf/RuledSrf, Sweep1, Sweep2, Pipe, ExtrudeCrvAlongCrv,
ExtrudeCrvTapered, ExtrudeCrvToPoint, ExtendSrf, Untrim, MeshToNURB, Ellipsoid,
Pyramid, TCone, Tube, Slab, BooleanSplit, ExtractSrf, Mesh, MeshBoolean*, Weld,
Unweld, SplitDisjointMesh, ExtractPt, MeshPlane/Box/Sphere/Cylinder/Cone/Torus,
Text, TextObject, Dim, DimAligned, Leader, Hatch, DimRadius, DimDiameter, DimAngle,
DimCurveLength, Centermark, CPlane (3Point/Object/View/World/Previous/Elevation),
Rectangle Rounded, Rebuild (surfaces too), Gumball (move/scale/rotate handles on the
selection, status bar toggle), Block/Insert/ExplodeBlock/BlockManager/
SelBlockInstance, FilletEdge/ChamferEdge/BlendEdge (edges between planar faces),
BlendSrf, Patch (approximate), RotateView, Pan, ZoomIn/Out/Factor, Maximize,
Line sub-options (Angled, FourPoint, Bisector, Perpendicular, Tangent, Normal),
SubCrv/ExtractSubCrv/DeleteSubCrv, ReducePolyline, ConvertToBeziers, MakeUniform,
MakePeriodic/NonPeriodic, Reparameterize, Domain, CrvStart/End, InsertKink,
RemoveKnot, ClosestPt, Curvature, TweenCurves, LineThroughPt, PlaneThroughPt,
ExtractControlPolygon, MatchCrvDir, PointGrid, SrfPtGrid, SrfControlPtGrid,
ExtractIsocurve, ExtractWireframe, UntrimAll/Border/Holes, OffsetMesh, ExtrudeMesh,
PlanarMesh, UnifyMeshNormals, 3DFace, ScaleNU, Twist, Taper, Ribbon, Calc,
UndoMultiple/RedoMultiple, ClearUndo, Commands, PolygonCount, SystemInfo, Audit/Check/
SelBadObjects, Make2D (no hidden-line removal), CutPlane, Parabola, UngroupAll,
UnlockSelected, LockSwap, IsolateLock/UnisolateLock, SolidPtOn, SelVolumeSphere,
SelClosedMesh/OpenMesh/PlanarCrv/PlanarSrf/TrimmedSrf/UntrimmedSrf/Box/ShortCrv/
Color/Text/Hatch/Dim/Leader/DupAll/Boundary, PlanarUnion/PlanarDifference/
PlanarIntersection/CurveBoolean (planar regions via prism CSG + section), RoundHole,
MakeHole, Shell (mesh based), Bend, Stretch, Flow, Dot, NamedCPlane, SelChain,
InsertControlPoint/RemoveControlPoint, Weight, Conic, Fair.

Also: Circle Tangent/AroundCurve/FitPoints/Circumference/Area, Arc StartPoint+Center,
Isometric view, UnrollSrf/Smash (developable unfolding), MeshSplit/MeshTrim (closed,
solid, planar or closed-curve cutters), DimOrdinate, DimRotated, CurvatureGraph(Off),
Symmetry, Pull, Fin/OffsetNormal, InterpCrvOnSrf, CrvDeviation, Diameter, Area/
VolumeMoments, MakeUniformUV, ExtractMeshPart, Maelstrom, Distribute,
ChangeToCurrentLayer, MatchLayer, DupLayer, OneLayerOff, MergeAllCoplanarFaces,
FilletSrf (planar surfaces, trims both), SaveACopy, IncrementalSave, Revert,
ToggleRightSidebar, ViewportTabs, CommandHelp. Block definitions and instances are
read from and written to .3dm files (ON_InstanceDefinition / ON_InstanceRef).

Batch 7: ClippingPlane / EnableClippingPlane / DisableClippingPlane / SelClippingPlane
(display clipping in the wgpu shaders, one plane per view), NamedPosition, Snapshots,
SetUserText / GetUserText / SetDocumentUserText / GetDocumentUserText / DocumentText,
ReadCommandFile (runs a text file through the command line; also
`triceratops --script file.txt` at startup, one command-line entry per line), DeleteFaces, MoveFace,
MoveEdge (planar polysurfaces), ExtrudeSrfToPoint, ExtrudeSrfTapered, OffsetMultiple.

Later batches: mesh editing (FillMeshHoles, DupMeshEdge, MeshIntersect,
MeshSelfIntersect, Silhouette, MeshOutline, MeshFromPoints (Delaunay), MeshPolyline,
ExtractMeshFaces*, ExtractDuplicateMeshFaces, ExtractNonManifoldMeshEdges,
Collapse*/SwapMeshEdge/SplitMeshEdge, ReduceMesh, AlignVertices, Stitch,
SubDivide (Loop subdivision), SubD primitives/ToSubD/ToNURBS as smoothed meshes,
ShrinkWrap, Crease/RemoveCrease), selection (SelCircular, SelRectangular, Lasso,
SelFence, SelVolumeObject/Pipe, SelKey/Value, SelDim*, SelSubD, SelectionFilter,
control point ops SelU/SelV/NextU/..., InvertPt, HidePt/ShowPt), system and
scripting (RunPythonScript feeds the script's output to the command line,
ReadCommandFile, OpenURL, Run, Autosave, OptionsExport/Import, ExportWithOrigin,
SaveAsTemplate, Audit3dmFile, ExportLinkedBlocks, Minimize, Fullscreen), display
(ViewCaptureToFile/Clipboard, ScreenCapture*, Render and Print as PNG captures,
BatchRenderNamedViews, Turntable with Record, Sun, GroundPlane, Zebra / EMap /
Curvature / DraftAngle analysis display modes, draw order commands), and tools
(BoxEdit, CalcRPN, DupFaceBorder, JoinCopy, PointCloud, SplitFace, ZoomEnds/
Naked/NonManifold, MatchProperties, Hyperlink, FindText, DimArea/DimVolume).

## Parametric mode (inspired by CADCraft)

`Parametric` turns on a CADCraft-style sketch mode (status bar pane "Parametric"):
geometric constraints (`GeomConstraint` or `GcCoincident`, `GcHorizontal`,
`GcVertical`, `GcParallel`, `GcPerpendicular`, `GcTangent`, `GcConcentric`,
`GcCollinear`, `GcEqual`, `GcSymmetric`, `GcFix`, `GcSmooth`), dimensional constraints
with expressions (`DimConstraint`, `DcAligned`, `DcHorizontal`, `DcVertical`,
`DcAngular`, `DcRadius`, `DcDiameter`), `AutoConstrain`, `Parameters` (user variables,
`d1*2+5` style expressions), `DelConstraint`, `ConstraintBar`, `ConstraintSettings`.
Constraints are solved by the `cadcraft-constraints` damped Gauss-Newton solver on
planar lines, polylines, arcs, circles and points in the sketch plane, and are
re-solved after every edit (commands, dragging, gumball). Glyphs with the constraint
names and dimension values are drawn at their anchors.

## History, cages, holes, materials, lights, linetypes

- `History` / Record History pane: commands that build objects from other objects
  (ExtrudeCrv, Loft, Offset, ...) are recorded and replayed when a parent changes
  (`HistoryUpdate`, `HistoryPurge`, `SelObjectsWithHistory`, `SelChildren`, `SelParents`).
- `Cage` / `CageEdit`: Bézier free-form deformation; moving cage points re-deforms the
  captives (`ReleaseFromCage`, `ExtractOriginalCaptives`, `SelCaptives`, `SelControls`).
- Holes keep their definition curves: `MakeHole`, `RoundHole`, `PlaceHole`, `CopyHole`,
  `MoveHole`, `RotateHole`, `MirrorHole`, `ArrayHole`, `ArrayHolePolar` re-cut the target.
- Materials (colour, gloss, transparency, image texture, mapping) with the Materials panel,
  `RenderAssignMaterialTo*`, `Apply*Mapping`, `Picture`, `BackgroundBitmap`; lights
  (`PointLight`, `Spotlight`, `DirectionalLight`, `LinearLight`, `RectangularLight`, Lights
  panel) and `Sun` light the Rendered display mode; `GradientView` and the Rendering panel
  set backgrounds and ground plane.
- Linetypes (`Linetypes`, `SetLinetype`, `SetLayerLinetype`, `SetLinetypeScale`,
  `SetCustomLinetype`, `SelLinetype`, `ExtractLineTypeSegments`) are drawn as dashes.
- Scripts can reference objects at object prompts with `#id`, `#id@x,y,z` or `last`.

Booleans between surfaces and polysurfaces (`BooleanUnion`, `BooleanDifference`,
`BooleanIntersection`, `BooleanSplit`, `Boolean2Objects`) and the solid tools built on them
(`MakeHole`, `RoundHole`, `PlaceHole`, `RevolvedHole`, `WireCut`, `Boss`, `Rib`, `PushPull`)
produce **NURBS polysurfaces**: the CSG runs on the face tessellations with face tags and
surface parameters, and every surviving piece becomes the original surface trimmed by the
(u, v) outline of that piece, so a box minus a cylinder keeps its planar faces and the
cylindrical wall as exact surfaces. Mesh inputs still give meshes (`MeshBoolean*`), and the
Boolean fails with a message rather than returning meshes. Intersection edges are exact to the
absolute tolerance (0.001 by default): every edge vertex is projected onto both surfaces by
alternating Newton closest-point iterations, the two faces sharing an edge get the same
vertices, and edges are densified until the chords deviate less than the tolerance. `Shell`
offsets the planes of planar-faced solids and opens the picked faces with the NURBS Boolean;
`WireCut` closes open cutting curves far away so the cutter is a polysurface. Trimmed curved
faces draw their isocurves clipped to the trim loops.

All bars and panels are resizable; the Main sidebar reflows its icons to the
width you give it. Options (colour scheme, colours, osnaps, sidebar width) are
saved in `~/.config/triceratops/options.json`.

## Curves, surfaces, views, layouts, mesh tools (final batches)

- Curve batch: `ArcBlend`, `Blend`, `Catenary`, `Hyperbola`, `Parabola3Pt`, `Crv2View`,
  `CSec`, `ContinueCurve`, `Match`, `ModifyRadius`, `OffsetCrvOnSrf`, `OrientOnCrv/Srf`,
  `RevCloud`, `ShortPath`, `FilletCorners`, `MarkFoci`, `CreateUVCrv`/`ApplyCrv`,
  `FlattenSrf`, `Convert`, `CrvSeam`, `SoftMove`/`SoftEdit*`, `EndBulge`, `MoveUVN`,
  `HandleCurve`, `Sketch` (drag in a viewport to draw).
- Surface/solid batch: `ConvertToSingleSpans`, `DivideAlongCreases`, `CreateSolid`,
  `ChamferSrf`, `ConnectSrf`, `MergeSrf`, `MatchSrf`, `Bounce`, `InfinitePlane`,
  `TruncatedPyramid`, `Paraboloid`, `Drape`/`DrapePt`, `Heightfield`, `FlowAlongSrf`,
  `Splop`, `ScaleByPlane`, `ScalePositions`, `RemapCPlane`, `ArraySrf`, `ArrayCrvOnSrf`,
  `TweenSurfaces`, `RailRevolve`, `FoldFace`, `PushPull`, `WireCut`, `Boss`, `Rib`,
  `RibbonOffset`, `ExtrudeSrfAlongCrv`, `Hydrostatics`, `CutVolume`, `DimCreaseAngle`,
  `EdgeContinuity`, `PointCloudContour/Section`, `SphereTangentToThreeSurfaces`,
  `RevolvedHole`, `Boolean2Objects`, `Clash`, `ExtractPipedCurve`, `DraftAnglePoint`.
- Views and viewports: `NewViewport`, `CloseViewport`, `SplitViewport*`, `SwapView`,
  `ReadViewportsFromFile` (JSON), grid layout for any number of viewports, `TiltView`,
  `DollyZoom`, `RotateCamera`, `Camera`, `Target`, `WalkAbout` (WASD/QE/arrows),
  `SetZoomExtentsBorder`, `Zoom1To1Calibrate`, `AddGuide`/`RemoveGuide`, `ShowEdges`,
  `ShowEnds`, `ShowDir`, `ThicknessAnalysis`, `ShowZBuffer`, `SetObjectDisplayMode`,
  animations (`Turntable`, `SetPathAnimation`, `SetFlythroughAnimation`,
  `SetOneDaySunAnimation`, `SetSeasonalSunAnimation`, `PlayAnimation`, `RecordAnimation`,
  `ViewFrameNumber`, ...), gumball options (`GumballRelocate`, `GumballAlignment`, ...),
  `LayerStateManager`, `LayerBook`, `NamedSelections`, `UndoSelected`, `Worksession`
  (reference files), clipping sections and drawings (`ClippingSections`,
  `ClippingDrawings`, `SaveClippingSectionViews`, ...), `EarthAnchorPoint`,
  `ModelBasepoint`, block editing (`BlockEdit`, `AddObjectsToBlock`, `ReplaceBlock`,
  `CreateUniqueBlock`, `BlockResetScale`, `RescueBlockOrphans`), `Macros`, `LoadScript`,
  `RunScript`, `Pause`/`MultiPause` in command files.
- Page layouts: `Layout` creates a page viewport (paper drawn in page units) with nested
  `Detail` viewports that render the model with their own cameras; objects drawn on a page
  live in page space (`ChangeSpace`), `HideInDetail`/`ShowInDetail`,
  `HideLayersInDetail`, `CopyDetailToViewport`, `CopyViewportToDetail`, `CopyLayout`,
  `Layouts`, `LayoutProperties`; `Print` captures the active page.
- Mesh/SubD batch: `MeshRepair` (weld, degenerate removal, unified normals, hole filling),
  `Bridge`, `Inset`, `Bevel`, `Reflect`, `Radiate`, `MultiPipe`, `ApplyMesh`,
  `ApplyMeshUVN`, `MakeSubDFriendly`, `Slide`, `SubDExpandEdges`, `SubDSpinEdge`,
  `ExtrudeSubD`, `OffsetSubD`, `SubDDisplayToggle`, `QuadRemesh` (uniform triangles),
  brep edge tools (`UnjoinEdge`, `JoinEdge`, `SplitEdge`, `RebuildEdges`, `ReplaceEdge`,
  `RefitTrim`, `RemoveAllNakedMicroEdges`, `ExtractBadSrf`, `NonmanifoldMerge`,
  `CreateRegions`, `MergeFaces`).

## File formats

`File > Open / Import / Export` list every format through one registry
(`crates/doc/src/formats/mod.rs`). Besides the native `.tri`, `.3dm` (openNURBS) and
DXF/DWG (cadcraft):

| Kind | Import and export | Export only |
|------|-------------------|-------------|
| Meshes | STL (binary + ASCII), PLY (ASCII + binary), OFF, 3DS, glTF/GLB, COLLADA DAE, VRML WRL, X3D, DirectX X, AMF, 3MF (zip), GTS, RAW, FBX ASCII, SLC slices | POV-Ray, RenderMan RIB |
| NURBS | IGES (110/100/106/116/126/128/142/144/402, trimmed faces keep their loops), STEP AP214 (B-spline curves/surfaces, planes, cylinders, cones, spheres, tori, revolutions, extrusions, faces/loops/edges/shells/solids), VDA-FS (CURVE/SURF polynomial patches) | |
| 2D | SVG (paths, polylines, rects, circles, ellipses; curves are projected on the Top view) | PDF, EPS, AI |
| Points | PTS, XYZ, ASC, CSV, TXT | |

Export opens in the working file's folder with the working file's name (`export` while the
document is unsaved), remembers the last folder per working file and the last file type, and
always writes the extension of the file type chosen in the dialog (on Linux the XDG portal is
called directly through D-Bus because it is the only way to learn the selected filter; other
platforms fall back to `rfd`). Exporting over the open working file is refused.

Surfaces and polysurfaces are tessellated for the mesh formats; meshes go into IGES/STEP as
faceted polylines/planar faces. Proprietary formats Triceratops cannot read (SKP, SAT,
DGN, E57, LWO, binary FBX, ...) give a message naming the format to use instead.

## Window layout and settings

Every panel (Properties, Layers, Display, Materials, Lights, Rendering, Help, Notes) and
every toolbar group lives in a dockable container: drag a tab out to float it as a window,
drop it on another container to move it there, use the `▾` menu of a container to dock it
top/bottom/left/right, float it or hide it, and right-click a tab to float or close it.
Panels > Toolbars shows hidden toolbar groups again. Flyouts (the toolbars that open from a
button with a small triangle, such as Array, Scale, Circle or Arc) can be torn off: drag the
flyout's title or click its `⇱` button and it becomes a floating toolbar that can then be
docked like any other; Panels > Toolbars > Flyouts opens any of them floating directly. The layout, the modeling aids (osnaps,
Ortho, Grid Snap, Planar, Gumball, SmartTrack, history recording), the viewport arrangement
and display modes, the theme, the backend and the drag/gumball options are saved in
`~/.config/triceratops/options.json` whenever they change and restored at start.
`SaveWindowLayout` / `WindowLayout` (also Panels menu and Options > Appearance) save or load a
layout JSON file, `Reset` restores the stock layout, and Options > "Reset all options" puts
every setting back to the defaults.

## Themes (Triceratops and Blender looks)

On dark themes the toolbar icons are drawn inverted: near-black strokes become light, white and
pale fills become dark, accent colours keep their hue (`icons::draw_icon_themed`).

The whole interface is themed: `Theme` (also View menu > Theme and Options > Appearance)
switches between the built-in **Triceratops Light**, **Triceratops Dark** and **Blender** themes or any
user theme saved as JSON in `~/.config/triceratops/themes/`. A theme drives the egui widgets
(colours, rounding, region gaps, font size), the viewport colours (grid, axes, selection,
gumball) and the GPU-drawn viewport background gradient. The Blender theme reproduces the
Blender 4 dark editor look: dark gaps between rounded regions, grey widgets with the blue
accent, orange selection and Blender's red/green/blue axes. `Theme Save` stores the current
colours as a user theme, `Theme Export`/`Import` move theme files around, `Theme Backend`
(or `TRICERATOPS_BACKEND=gl|vulkan`) chooses the graphics backend: the renderer runs on OpenGL
ES (EGL) or Vulkan through wgpu, and on OpenGL view captures go through an offscreen target.

## Surface engine: projection, trimming, exact Booleans

`crates/geom/src/surface_ops.rs` holds the surface engine that Trim, Split, WireCut and the
Booleans share:

- `Projector` projects points and curves onto any NURBS surface along a direction: line/surface
  intersections are seeded by a tessellation and solved by a three-unknown Newton iteration, the
  curve is sampled only where it can reach the surface and refined until the chords follow the
  exact projection within the tolerance; branches are extended to the domain border, joined at
  tangential exits and split at the seams of closed surfaces.
- `split_region` partitions a trimmed (u, v) region by any number of paths (planar-graph face
  extraction with holes and islands), so a face can be cut by several crossing curves at once.

`crates/geom/src/brep_boolean_exact.rs` is the primary Boolean: face/face intersection curves
are traced on the tessellations, refined onto both surfaces by a least-norm Newton solve,
densified to the tolerance, their endpoints solved exactly on the trim edges they end on (triple
points) and split at seams; each face is partitioned by its curves and the pieces are classified
against the other solid (inside, outside, coincident with same or opposite normal). Faces keep
their original surfaces; only trim loops are added. The mesh-CSG rebuild of `brep_boolean.rs`
is used only when the exact result does not close. Every solid tool (BooleanUnion/Difference/
Intersection/Split, Boolean2Objects, MakeHole, RoundHole, RevolvedHole, PushPull, Boss, Rib,
Shell, WireCut on solids) goes through this path.

The Transform, Curve Tools and Main toolbars carry an Array button with a flyout (Rectangular,
Polar, Linear, Along Curve, Along Surface, Along Curve on Surface, Array Hole); `ArrayPolar`
has the reference options `Axis`, `StepAngle`, `Rotate=Yes|No`, `ZOffset` and the
"Press Enter to accept" stage, and accepts two reference points for the fill angle.

`WireCut` follows the reference prompts: cutting curve, objects, cut depth points (Enter for an
infinite cut) with `Direction=CPlaneNormal|View|Custom`, `KeepAll` and `DeleteInput`; closed
solids are cut with the exact Boolean, open surfaces and polysurfaces are trimmed face by face
with the projected curve and grouped by side.

## Not yet implemented

Boolean trim curves are polylines in (u, v) refined to the absolute tolerance (vertices lie on
both surfaces, triple points are solved on the edges; the curves are not analytic),
Shell handles planar-faced solids only,
FilletEdge on curved faces, true SubD surfaces (SubDs are smoothed triangle meshes,
quads are triangle pairs), SmartTrack, brep edge topology (one edge per face side, so
MergeEdge is a no-op). STEP import converts analytic surfaces to NURBS and drops trim loops
that cross a periodic seam (the face is then untrimmed). Grasshopper, licensing, digitizers
and plug-ins are not available; those commands say so and name the closest alternative.
