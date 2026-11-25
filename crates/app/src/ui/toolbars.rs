//! Toolbar container with tabs (Standard, CPlanes, Set View, ...), the Main sidebar,
//! flyout toolbars and the middle-mouse Popup toolbar.

use egui::{Color32, Pos2};

use super::icons::{icon_button, IconButton, ICON};
use crate::app::{App, DisplayMode, Layout};

#[derive(Clone, Copy)]
pub struct Btn {
    pub icon: &'static str,
    pub tip: &'static str,
    pub left: &'static str,
    pub right: &'static str,
    pub flyout: &'static str,
}

const fn b(icon: &'static str, tip: &'static str, left: &'static str, right: &'static str, flyout: &'static str) -> Btn {
    Btn { icon, tip, left, right, flyout }
}

pub const TAB_NAMES: [&str; 15] = [
    "Standard",
    "CPlanes",
    "Set View",
    "Display",
    "Select",
    "Viewport Layout",
    "Visibility",
    "Transform",
    "Curve Tools",
    "Surface Tools",
    "SubD Tools",
    "Solid Tools",
    "Mesh Tools",
    "Render Tools",
    "Drafting",
];

/// Toolbar tabs that older layouts may still list; they are dropped when a layout loads.
pub const RETIRED_TABS: &[&str] = &["New"];

/// Standard toolbar, in Triceratops' order (left command / right command).
pub const STANDARD: &[Btn] = &[
    b("new", "New", "New", "", ""),
    b("open", "Open", "Open", "", ""),
    b("import", "Import / Open", "Import", "Open", ""),
    b("save", "Save", "Save", "", "File"),
    b("export", "Export / Save As", "Export", "SaveAs", ""),
    b("print", "Print", "Print", "", ""),
    b("documentproperties", "Document Properties / Options", "DocumentProperties", "Options", ""),
    b("cut", "Cut", "Cut", "", ""),
    b("copyclip", "Copy to Clipboard", "CopyToClipboard", "", ""),
    b("paste", "Paste", "Paste", "", ""),
    b("undo", "Undo / Redo", "Undo", "Redo", "Undo"),
    b("pan", "Pan", "Pan", "", ""),
    b("rotateview", "Rotate View", "RotateView", "", ""),
    b("zoom_dynamic", "Zoom Dynamic", "Zoom", "", ""),
    b("zoom_window", "Zoom Window / Zoom Target", "Zoom", "Zoom", ""),
    b("zoom_extents", "Zoom Extents / Zoom Extents All", "ZoomExtents", "ZoomExtentsAll", ""),
    b("zoom_selected", "Zoom Selected", "ZoomSelected", "ZoomSelected", ""),
    b("undo", "Undo View / Redo View", "UndoView", "RedoView", "View"),
    b("4view", "4 Viewports", "4View", "4View", "Viewport Layout"),
    b("setview", "Set View: World Right", "Right", "", "Set View"),
    b("cplane", "Set CPlane", "CPlane", "", "CPlanes"),
    b("showosnap", "Show Osnap", "ShowOsnap", "", ""),
    b("selall", "Select All", "SelAll", "", "Select"),
    b("hide", "Hide / Show", "Hide", "Show", "Visibility"),
    b("lock", "Lock / Unlock", "Lock", "Unlock", ""),
    b("layer", "Layers", "Layer", "", ""),
    b("properties", "Properties", "Properties", "", ""),
    b("shade", "Shade", "Shaded", "", ""),
    b("viewport_shaded", "Shaded / Wireframe viewport", "Shaded", "Wireframe", "Display"),
    b("render", "Render", "Render", "", ""),
    b("spotlight", "Spotlight", "Spotlight", "", ""),
    b("options", "Options", "Options", "", ""),
    b("dim", "Dimension", "Dim", "", ""),
    b("locust", "Locust node editor", "Locust", "", ""),
    b("help", "Help", "Help", "", ""),
];

pub const CPLANES: &[Btn] = &[
    b("cplane", "CPlane", "CPlane", "", ""),
    b("cplane", "CPlane: World Top", "CPlane", "", ""),
    b("cplane", "CPlane: World Front", "CPlane", "", ""),
    b("cplane", "CPlane: World Right", "CPlane", "", ""),
];

pub const SET_VIEW: &[Btn] = &[
    b("setview", "Top", "Top", "", ""),
    b("setview", "Bottom", "Bottom", "", ""),
    b("setview", "Left", "Left", "", ""),
    b("setview", "Right", "Right", "", ""),
    b("setview", "Front", "Front", "", ""),
    b("setview", "Back", "Back", "", ""),
    b("setview", "Perspective", "Perspective", "", ""),
    b("zoom_extents", "Zoom Extents", "ZoomExtents", "ZoomExtentsAll", ""),
    b("undo", "Undo View", "UndoView", "RedoView", ""),
];

pub const DISPLAY: &[Btn] = &[
    b("viewport_wireframe", "Wireframe", "Wireframe", "", ""),
    b("viewport_shaded", "Shaded", "Shaded", "", ""),
    b("render", "Rendered", "Rendered", "", ""),
    b("shade", "Ghosted", "Ghosted", "", ""),
    b("viewport_wireframe", "X-Ray", "XRay", "", ""),
    b("dim", "Technical", "Technical", "", ""),
    b("textobject", "Pen", "Pen", "", ""),
    b("sphere", "Arctic", "Arctic", "", ""),
];

pub const SELECT: &[Btn] = &[
    b("selall", "Select All / Select None", "SelAll", "SelNone", ""),
    b("selall", "Invert", "Invert", "", ""),
    b("points", "Select Points", "SelPt", "", ""),
    b("curve", "Select Curves", "SelCrv", "", ""),
    b("srfpt", "Select Surfaces", "SelSrf", "", ""),
    b("box", "Select Polysurfaces", "SelPolysrf", "", ""),
    b("mesh", "Select Meshes", "SelMesh", "", ""),
    b("history", "Select Last", "SelLast", "", ""),
];

pub const VIEWPORT_LAYOUT: &[Btn] = &[
    b("4view", "4 Viewports", "4View", "", ""),
    b("4view", "3 Viewports", "3View", "", ""),
    b("4view", "Maximize Viewport", "MaxViewport", "", ""),
];

pub const VISIBILITY: &[Btn] = &[
    b("hide", "Hide / Show", "Hide", "Show", ""),
    b("show", "Show", "Show", "", ""),
    b("lock", "Lock / Unlock", "Lock", "Unlock", ""),
    b("unlock", "Unlock", "Unlock", "", ""),
];

pub const TRANSFORM: &[Btn] = &[
    b("move", "Move", "Move", "", ""),
    b("copy", "Copy", "Copy", "", ""),
    b("rotate", "Rotate", "Rotate", "", ""),
    b("scale", "Scale 3-D / Scale 2-D", "Scale", "Scale2D", "Scale"),
    b("scale", "Scale 1-D", "Scale1D", "", ""),
    b("mirror", "Mirror", "Mirror", "", ""),
    b("array", "Rectangular Array / Polar Array", "Array", "ArrayPolar", "Array"),
    b("arraypolar", "Polar Array", "ArrayPolar", "", "Array"),
    b("arraycrv", "Array Along Curve", "ArrayCrv", "", "Array"),
    b("align", "Align", "Align", "", ""),
];

/// Array flyout: every array command, as in the reference toolbar.
pub const FLY_ARRAY: &[Btn] = &[
    b("array", "Rectangular Array", "Array", "", ""),
    b("arraypolar", "Polar Array", "ArrayPolar", "", ""),
    b("arraylinear", "Linear Array", "ArrayLinear", "", ""),
    b("arraycrv", "Array Along Curve", "ArrayCrv", "", ""),
    b("arraysrf", "Array Along Surface", "ArraySrf", "", ""),
    b("arraycrvonsrf", "Array Along Curve on Surface", "ArrayCrvOnSrf", "", ""),
    b("arrayhole", "Array Hole / Array Hole Polar", "ArrayHole", "ArrayHolePolar", ""),
];

pub const CURVE_TOOLS: &[Btn] = &[
    b("fillet", "Fillet Curves / Chamfer", "Fillet", "Chamfer", ""),
    b("offset", "Offset Curve", "Offset", "", ""),
    b("points", "Divide Curve", "Divide", "", ""),
    b("point", "Intersect", "Intersect", "", ""),
    b("extend", "Extend Curve", "Extend", "", ""),
    b("join", "Join", "Join", "", ""),
    b("explode", "Explode", "Explode", "", ""),
    b("rebuild", "Rebuild", "Rebuild", "", ""),
    b("trim", "Trim / Untrim", "Trim", "", ""),
    b("split", "Split", "Split", "", ""),
    b("dir", "Direction / Flip", "Dir", "Flip", ""),
];

pub const SURFACE_TOOLS: &[Btn] = &[
    b("srfpt", "Surface from 3 or 4 Corner Points", "SrfPt", "", ""),
    b("plane", "Rectangular Plane: Corner to Corner", "Plane", "", ""),
    b("plane", "Surface from Planar Curves", "PlanarSrf", "", ""),
    b("extrudecrv", "Extrude Straight", "ExtrudeCrv", "", ""),
    b("revolve", "Revolve", "Revolve", "", ""),
    b("loft", "Loft", "Loft", "", ""),
    b("filletsrf", "Fillet Surface", "FilletSrf", "", ""),
    b("extrudecrv", "Extrude Surface", "ExtrudeSrf", "", ""),
    b("offset", "Offset Surface", "OffsetSrf", "", ""),
    b("trim", "Trim / Split", "Trim", "Split", ""),
];

pub const SOLID_TOOLS: &[Btn] = &[
    b("box", "Box: Corner to Corner, Height", "Box", "BoundingBox", "Solid"),
    b("sphere", "Sphere: Center, Radius", "Sphere", "Ellipsoid", "Solid"),
    b("cylinder", "Cylinder", "Cylinder", "Tube", "Solid"),
    b("cone", "Cone", "Cone", "TCone", "Solid"),
    b("torus", "Torus", "Torus", "Paraboloid", "Solid"),
    b("pyramid", "Pyramid", "Pyramid", "TruncatedPyramid", "Solid"),
    b("pipe", "Pipe", "Pipe", "MultiPipe", "Solid"),
    b("extrudecrv", "Extrude Closed Planar Curve", "ExtrudeCrv", "ExtrudeCrvTapered", "Solid"),
    b("extrudesrf", "Extrude Surface", "ExtrudeSrf", "ExtrudeSrfTapered", "Solid"),
    b("slab", "Slab", "Slab", "", ""),
    b("plane", "Cap Planar Holes", "Cap", "CreateSolid", ""),
    b("booleanunion", "Boolean Union", "BooleanUnion", "", "Solid"),
    b("booleandifference", "Boolean Difference", "BooleanDifference", "", "Solid"),
    b("booleanintersection", "Boolean Intersection", "BooleanIntersection", "", "Solid"),
    b("booleansplit", "Boolean Split", "BooleanSplit", "Boolean2Objects", "Solid"),
    b("filletedge", "Fillet Edge", "FilletEdge", "BlendEdge", "Solid"),
    b("chamferedge", "Chamfer Edge", "ChamferEdge", "", ""),
    b("shell", "Shell", "Shell", "", ""),
    b("makehole", "Make Hole", "MakeHole", "PlaceHole", "Solid"),
    b("roundhole", "Round Hole", "RoundHole", "RevolvedHole", "Solid"),
    b("movehole", "Move Hole", "MoveHole", "CopyHole", "Solid"),
    b("arrayhole", "Array Hole", "ArrayHole", "ArrayHolePolar", "Solid"),
    b("wirecut", "Wire Cut", "WireCut", "", ""),
    b("boss", "Boss", "Boss", "Rib", ""),
    b("pushpull", "Push/Pull", "PushPull", "", ""),
    b("moveface", "Move Face", "MoveFace", "MoveUntrimmedFace", ""),
    b("moveedge", "Move Edge", "MoveEdge", "MoveUntrimmedEdge", ""),
    b("extractsrf", "Extract Surface", "ExtractSrf", "DeleteFaces", ""),
    b("splitface", "Split Face", "SplitFace", "MergeAllCoplanarFaces", ""),
    b("solidpton", "Solid Control Points On", "SolidPtOn", "PointsOff", ""),
    b("volume", "Volume", "Volume", "Hydrostatics", ""),
];

pub const SUBD_TOOLS: &[Btn] = &[
    b("box", "SubD Box", "SubDBox", "SubDPlane", ""),
    b("sphere", "SubD Sphere", "SubDSphere", "SubDEllipsoid", ""),
    b("cylinder", "SubD Cylinder", "SubDCylinder", "SubDTruncatedCone", ""),
    b("cone", "SubD Cone", "SubDCone", "", ""),
    b("torus", "SubD Torus", "SubDTorus", "", ""),
    b("tosubd", "SubD from Mesh / NURBS", "ToSubD", "ToNURBS", ""),
    b("loft", "SubD Loft", "SubDLoft", "SubDSweep1", ""),
    b("revolve", "SubD Revolve", "Revolve", "SubDSweep2", ""),
    b("pipe", "Multipipe", "MultiPipe", "", ""),
    b("quadremesh", "Quad Remesh", "QuadRemesh", "", ""),
    b("subdivide", "Subdivide", "SubDivide", "", ""),
    b("reflect", "Reflect", "Reflect", "RemoveSymmetry", ""),
    b("bridge", "Bridge", "Bridge", "Stitch", ""),
    b("fill", "Fill Hole", "Fill", "", ""),
    b("insertedge", "Insert Edge", "InsertEdge", "InsertPoint", ""),
    b("slide", "Slide", "Slide", "", ""),
    b("crease", "Crease", "Crease", "RemoveCrease", ""),
    b("extrudesubd", "Extrude SubD", "ExtrudeSubD", "OffsetSubD", ""),
    b("bevel", "Bevel", "Bevel", "", ""),
    b("mergefaces", "Merge Faces", "MergeFaces", "", ""),
    b("subddisplay", "Toggle SubD Display", "SubDDisplayToggle", "", ""),
    b("seledgeloop", "Select Edge Loop", "SelEdgeLoop", "SelEdgeRing", ""),
];

pub const MESH_TOOLS: &[Btn] = &[
    b("mesh", "Mesh from NURBS Object", "Mesh", "", ""),
    b("box", "Mesh Box", "MeshBox", "MeshPlane", ""),
    b("sphere", "Mesh Sphere", "MeshSphere", "MeshEllipsoid", ""),
    b("cylinder", "Mesh Cylinder", "MeshCylinder", "MeshTruncatedCone", ""),
    b("cone", "Mesh Cone", "MeshCone", "", ""),
    b("torus", "Mesh Torus", "MeshTorus", "", ""),
    b("meshpatch", "Mesh Patch", "MeshPatch", "MeshFromPoints", ""),
    b("3dface", "Single Mesh Face", "3DFace", "MeshFromLines", ""),
    b("offsetmesh", "Offset Mesh", "OffsetMesh", "ExtrudeMesh", ""),
    b("booleanunion", "Mesh Boolean Union", "MeshBooleanUnion", "", ""),
    b("booleandifference", "Mesh Boolean Difference", "MeshBooleanDifference", "", ""),
    b("booleanintersection", "Mesh Boolean Intersection", "MeshBooleanIntersection", "", ""),
    b("booleansplit", "Mesh Boolean Split", "MeshBooleanSplit", "", ""),
    b("meshsplit", "Mesh Split", "MeshSplit", "MeshTrim", ""),
    b("meshrepair", "Mesh Repair", "MeshRepair", "Check", ""),
    b("weld", "Weld", "Weld", "Unweld", ""),
    b("fillholes", "Fill Mesh Holes", "FillMeshHoles", "FillMeshHole", ""),
    b("unifynormals", "Unify Mesh Normals", "UnifyMeshNormals", "Flip", ""),
    b("reducemesh", "Reduce Mesh", "ReduceMesh", "", ""),
    b("quadremesh", "Quad Remesh", "QuadRemesh", "", ""),
    b("subdivide", "Subdivide Mesh", "SubDivide", "Smooth", ""),
    b("extractmesh", "Extract Mesh Faces", "ExtractMeshFaces", "ExtractMeshPart", ""),
    b("dupmeshedge", "Duplicate Mesh Edge", "DupMeshEdge", "MeshOutline", ""),
    b("meshtonurb", "Mesh to NURBS", "MeshToNURB", "ToSubD", ""),
    b("shrinkwrap", "Shrink Wrap", "ShrinkWrap", "", ""),
];

pub const RENDER_TOOLS: &[Btn] = &[
    b("render", "Render", "Render", "RenderPreview", ""),
    b("viewport_shaded", "Rendered Display", "SetDisplayMode", "", ""),
    b("materials", "Materials", "Materials", "RenderAssignMaterialToObjects", ""),
    b("spotlight", "Spotlight", "Spotlight", "PointLight", ""),
    b("directionallight", "Directional Light", "DirectionalLight", "RectangularLight", ""),
    b("sun", "Sun", "Sun", "GroundPlane", ""),
    b("mapping", "Planar Mapping", "ApplyPlanarMapping", "ApplyBoxMapping", ""),
    b("picture", "Picture", "Picture", "BackgroundBitmap", ""),
    b("gradient", "Gradient View", "GradientView", "", ""),
    b("capture", "View Capture to File", "ViewCaptureToFile", "ViewCaptureToClipboard", ""),
    b("turntable", "Turntable", "Turntable", "", ""),
];

pub const DRAFTING: &[Btn] = &[
    b("dim", "Linear Dimension", "Dim", "DimAligned", ""),
    b("dimradius", "Radius Dimension", "DimRadius", "DimDiameter", ""),
    b("dimangle", "Angle Dimension", "DimAngle", "DimOrdinate", ""),
    b("leader", "Leader", "Leader", "", ""),
    b("textobject", "Text", "Text", "TextObject", ""),
    b("dot", "Annotation Dot", "Dot", "", ""),
    b("hatch", "Hatch", "Hatch", "HatchScale", ""),
    b("make2d", "Make 2-D Drawing", "Make2D", "", ""),
    b("revcloud", "Revision Cloud", "RevCloud", "Arrowhead", ""),
    b("layout", "Layout", "Layout", "Detail", ""),
    b("print", "Print", "Print", "", ""),
];

pub const EMPTY: &[Btn] = &[];

pub fn tab_buttons(i: usize) -> &'static [Btn] {
    match i {
        0 => STANDARD,
        1 => CPLANES,
        2 => SET_VIEW,
        3 => DISPLAY,
        4 => SELECT,
        5 => VIEWPORT_LAYOUT,
        6 => VISIBILITY,
        7 => TRANSFORM,
        8 => CURVE_TOOLS,
        9 => SURFACE_TOOLS,
        10 => SUBD_TOOLS,
        11 => SOLID_TOOLS,
        12 => MESH_TOOLS,
        13 => RENDER_TOOLS,
        14 => DRAFTING,
        _ => EMPTY,
    }
}

/// Main toolbar (left sidebar), two columns read row by row: Triceratops 8 default order.
pub const MAIN: &[Btn] = &[
    b("cancel", "Cancel / Cancel All", "Cancel", "Cancel", ""),
    b("point", "Point / Points", "Point", "Points", "Point"),
    b("polyline", "Polyline / Line Segments", "Polyline", "Lines", "Lines"),
    b("curve", "Control Point Curve / Curve Through Points", "Curve", "InterpCrv", "Curve"),
    b("circle", "Circle: Center, Radius", "Circle", "", "Circle"),
    b("ellipse", "Ellipse: From Center", "Ellipse", "", "Ellipse"),
    b("arc", "Arc: Center, Start, Angle", "Arc", "", "Arc"),
    b("rectangle", "Rectangle: Corner to Corner", "Rectangle", "", "Rectangle"),
    b("polygon", "Polygon: Center, Radius", "Polygon", "", "Polygon"),
    b("fillet", "Fillet Curves", "Fillet", "", "Curve Tools"),
    b("srfpt", "Surface from 3 or 4 Corner Points", "SrfPt", "", "Surface"),
    b("filletsrf", "Fillet Surface", "FilletSrf", "", "Surface Tools"),
    b("box", "Box: Corner to Corner, Height", "Box", "", "Solid"),
    b("booleanunion", "Boolean Union", "BooleanUnion", "", "Solid Tools"),
    b("project", "Project to Surface", "Project", "", "Curve From Object"),
    b("mesh", "Mesh from NURBS Object / Mesh to NURBS", "Mesh", "MeshToNURB", "Mesh Tools"),
    b("join", "Join / Join Copy", "Join", "Join", "Join"),
    b("explode", "Explode / Extract Surface", "Explode", "ExtractSrf", ""),
    b("trim", "Trim / Untrim", "Trim", "Untrim", ""),
    b("split", "Split / Split with Isocurve", "Split", "Split", ""),
    b("group", "Group", "Group", "", "Grouping"),
    b("ungroup", "Ungroup / Ungroup All", "Ungroup", "Ungroup", ""),
    b("editpton", "Edit Points On / Points Off", "EditPtOn", "PointsOff", ""),
    b("pointson", "Control Points On / Points Off", "PointsOn", "PointsOff", "Point Editing"),
    b("textobject", "Text Object", "TextObject", "", ""),
    b("move", "Move", "Move", "", "Transform"),
    b("copy", "Copy / Copy In Place", "Copy", "Copy", ""),
    b("rotate", "Rotate 2-D / Rotate 3-D", "Rotate", "Rotate", ""),
    b("scale", "Scale 3-D / Scale 2-D", "Scale", "Scale2D", "Scale"),
    b("dir", "Direction / Flip", "Dir", "Flip", "Analyze"),
    b("array", "Rectangular Array / Polar Array", "Array", "ArrayPolar", "Array"),
    b("align", "Align", "Align", "", "Align and Distribute"),
    b("flowalongsrf", "Flow Along Surface", "FlowAlongSrf", "", "Deformation Tools"),
    b("pushpull", "PushPull", "PushPull", "", "PushPull"),
    b("check", "Check Objects", "Check", "", "Diagnostics"),
    b("block", "Define Block", "Block", "", ""),
    b("history", "Record History", "History", "", "History"),
];

const FLY_POINT: &[Btn] = &[b("point", "Single Point", "Point", "", ""), b("points", "Multiple Points", "Points", "", "")];
const FLY_LINES: &[Btn] = &[
    b("line", "Single Line", "Line", "", ""),
    b("lines", "Line Segments", "Lines", "", ""),
    b("polyline", "Polyline", "Polyline", "", ""),
    b("rectangle", "Rectangle", "Rectangle", "", ""),
    b("polygon", "Polygon", "Polygon", "", ""),
];
const FLY_CURVE: &[Btn] = &[b("curve", "Control Point Curve", "Curve", "", ""), b("interpcrv", "Interpolate Points", "InterpCrv", "", "")];
const FLY_CIRCLE: &[Btn] = &[b("circle", "Circle: Center, Radius", "Circle", "", ""), b("circle3pt", "Circle: 3 Points", "Circle", "", "")];
const FLY_ELLIPSE: &[Btn] = &[b("ellipse", "Ellipse: From Center", "Ellipse", "", "")];
const FLY_ARC: &[Btn] = &[b("arc", "Arc: Center, Start, Angle", "Arc", "", ""), b("arc", "Arc: Start, End, Point on Arc", "Arc3Pt", "", "")];
const FLY_RECT: &[Btn] = &[b("rectangle", "Rectangle: Corner to Corner", "Rectangle", "", ""), b("plane", "Rectangular Plane", "Plane", "", "")];
const FLY_POLYGON: &[Btn] = &[b("polygon", "Polygon: Center, Radius", "Polygon", "", "")];
const FLY_SCALE: &[Btn] = &[b("scale", "Scale 3-D", "Scale", "", ""), b("scale", "Scale 2-D", "Scale2D", "", ""), b("scale", "Scale 1-D", "Scale1D", "", "")];
const FLY_ANALYZE: &[Btn] = &[b("distance", "Distance", "Distance", "", ""), b("dir", "Direction", "Dir", "", ""), b("properties", "What", "What", "", "")];
const FLY_JOIN: &[Btn] = &[b("join", "Join", "Join", "", ""), b("explode", "Explode", "Explode", "", "")];
const FLY_UNDO: &[Btn] = &[b("undo", "Undo", "Undo", "", ""), b("redo", "Redo", "Redo", "", "")];
const FLY_VIEW: &[Btn] = &[b("undo", "Undo View", "UndoView", "", ""), b("redo", "Redo View", "RedoView", "", "")];
const FLY_FILE: &[Btn] = &[b("save", "Save", "Save", "", ""), b("save", "Save As", "SaveAs", "", ""), b("export", "Export", "Export", "", "")];

pub fn flyout_items(name: &str) -> &'static [Btn] {
    match name {
        "Point" => FLY_POINT,
        "Lines" => FLY_LINES,
        "Curve" => FLY_CURVE,
        "Circle" => FLY_CIRCLE,
        "Ellipse" => FLY_ELLIPSE,
        "Arc" => FLY_ARC,
        "Rectangle" => FLY_RECT,
        "Polygon" => FLY_POLYGON,
        "Curve Tools" => CURVE_TOOLS,
        "Surface" | "Surface Tools" => SURFACE_TOOLS,
        "Solid" | "Solid Tools" => SOLID_TOOLS,
        "SubD" | "SubD Tools" => SUBD_TOOLS,
        "Mesh" | "Mesh Tools" => MESH_TOOLS,
        "Transform" => TRANSFORM,
        "Scale" => FLY_SCALE,
        "Array" => FLY_ARRAY,
        "Analyze" => FLY_ANALYZE,
        "Display" => DISPLAY,
        "Set View" => SET_VIEW,
        "Select" => SELECT,
        "Visibility" => VISIBILITY,
        "Viewport Layout" => VIEWPORT_LAYOUT,
        "Join" => FLY_JOIN,
        "Undo" => FLY_UNDO,
        "View" => FLY_VIEW,
        "File" => FLY_FILE,
        _ => EMPTY,
    }
}

#[derive(Clone)]
struct FlyoutState {
    name: String,
    pos: Pos2,
}

fn flyout_id() -> egui::Id {
    egui::Id::new("flyout_state")
}

fn open_flyout(ctx: &egui::Context, name: &str, pos: Pos2) {
    ctx.data_mut(|d| d.insert_temp(flyout_id(), FlyoutState { name: name.to_string(), pos }));
}

/// Draw a toolbar button, dispatching left/right clicks and flyouts.
pub fn toolbar_button(ui: &mut egui::Ui, app: &mut App, btn: &Btn) {
    let resp = icon_button(ui, &IconButton { name: btn.icon, tooltip: btn.tip, size: ICON, flyout: !btn.flyout.is_empty(), selected: false });
    if resp.hovered() {
        if let Some(info) = crate::commands::command_info(btn.left) {
            app.status_hint = format!("{}: {}", btn.tip, info.description);
        }
    }
    let on_triangle = resp.hover_pos().is_some_and(|p| p.x > resp.rect.right() - 8.0 && p.y > resp.rect.bottom() - 8.0);
    if !btn.flyout.is_empty() && (resp.drag_started() || (resp.clicked() && on_triangle)) {
        open_flyout(ui.ctx(), btn.flyout, resp.rect.left_bottom());
        return;
    }
    if resp.clicked() && !btn.left.is_empty() {
        app.start_command(btn.left);
    } else if resp.secondary_clicked() && !btn.right.is_empty() {
        app.start_command(btn.right);
    }
}

/// Buttons of a toolbar group by name ("Main" or one of `TAB_NAMES`).
pub fn group_buttons(name: &str) -> &'static [Btn] {
    if name == "Main" {
        return MAIN;
    }
    match TAB_NAMES.iter().position(|n| *n == name) {
        Some(i) => tab_buttons(i),
        // torn-off flyouts are toolbar groups too
        None => flyout_items(name),
    }
}

/// Names of every flyout group (for the Panels > Toolbars menu).
pub const FLYOUT_NAMES: &[&str] = &["Array", "Scale", "Analyze", "Join", "Undo", "View", "File", "Curve", "Circle", "Ellipse", "Arc", "Rectangle", "Polygon"];

/// Draw a toolbar group: a wrapped row (top/bottom/floating) or a grid of icons (left/right).
pub fn draw_group(app: &mut App, ui: &mut egui::Ui, name: &str, vertical: bool) {
    let buttons = group_buttons(name);
    if buttons.is_empty() {
        ui.label(egui::RichText::new("(this toolbar is empty)").color(Color32::GRAY));
        return;
    }
    if vertical {
        let cell = ICON + 6.0 + 1.0;
        let cols = ((ui.available_width() - 2.0) / cell).floor().max(1.0) as usize;
        egui::ScrollArea::vertical().id_salt(("toolbar_grid", name)).show(ui, |ui| {
            egui::Grid::new(("toolbar_grid_inner", name)).spacing([1.0, 1.0]).show(ui, |ui| {
                for (i, btn) in buttons.iter().enumerate() {
                    toolbar_button(ui, app, btn);
                    if i % cols == cols - 1 {
                        ui.end_row();
                    }
                }
            });
        });
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            for (i, btn) in buttons.iter().enumerate() {
                if name == "Standard" && matches!(i, 3 | 8 | 11 | 18 | 21 | 25 | 27 | 29 | 31 | 34) {
                    ui.separator();
                }
                toolbar_button(ui, app, btn);
            }
        });
    }
}

/// Flyout toolbars and the middle-mouse "Popup" toolbar.
pub fn flyouts(app: &mut App, ctx: &egui::Context) {
    let state: Option<FlyoutState> = ctx.data(|d| d.get_temp(flyout_id()));
    if let Some(st) = state {
        let items = flyout_items(&st.name);
        let mut close = items.is_empty();
        let mut tear: Option<Pos2> = None;
        let area = egui::Area::new(egui::Id::new("flyout_area")).fixed_pos(st.pos).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    // the title bar is a drag handle: dragging it tears the flyout off
                    let title = ui.add(egui::Label::new(egui::RichText::new(&st.name).small().color(Color32::GRAY)).sense(egui::Sense::drag()))
                        .on_hover_text("Drag to tear off as a floating toolbar");
                    if title.drag_stopped() {
                        tear = ui.ctx().input(|i| i.pointer.latest_pos());
                    }
                    if title.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("⇱").on_hover_text("Float this toolbar").clicked() {
                            tear = Some(st.pos + egui::vec2(0.0, 24.0));
                        }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(6.0 * (ICON + 8.0));
                    for btn in items {
                        let resp = icon_button(ui, &IconButton { name: btn.icon, tooltip: btn.tip, size: ICON, flyout: false, selected: false });
                        if resp.clicked() {
                            app.start_command(btn.left);
                            close = true;
                        } else if resp.secondary_clicked() && !btn.right.is_empty() {
                            app.start_command(btn.right);
                            close = true;
                        }
                    }
                });
            });
        });
        if let Some(p) = tear {
            app.dock.float_group(crate::ui::dock::ContainerKind::Toolbars, &st.name, p.x - 20.0, p.y - 10.0);
            app.options_dirty = true;
            close = true;
        }
        let clicked_outside = !ctx.input(|i| i.pointer.any_down() && area.response.rect.expand(4.0).contains(i.pointer.press_origin().unwrap_or_default())) && ctx.input(|i| i.pointer.any_pressed()) && !area.response.hovered() && !ctx.input(|i| i.pointer.interact_pos().is_some_and(|p| area.response.rect.expand(4.0).contains(p)));
        if close || clicked_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.data_mut(|d| d.remove::<FlyoutState>(flyout_id()));
        }
    }
    if let Some(pos) = app.popup_toolbar {
        let items: &[Btn] = &[
            b("move", "Move", "Move", "", ""),
            b("copy", "Copy", "Copy", "", ""),
            b("rotate", "Rotate", "Rotate", "", ""),
            b("scale", "Scale", "Scale", "", ""),
            b("mirror", "Mirror", "Mirror", "", ""),
            b("join", "Join", "Join", "", ""),
            b("explode", "Explode", "Explode", "", ""),
            b("trim", "Trim", "Trim", "", ""),
            b("split", "Split", "Split", "", ""),
            b("hide", "Hide", "Hide", "", ""),
            b("show", "Show", "Show", "", ""),
            b("zoom_extents", "Zoom Extents", "ZoomExtents", "", ""),
            b("undo", "Undo", "Undo", "", ""),
            b("delete", "Delete", "Delete", "", ""),
        ];
        let mut close = false;
        let area = egui::Area::new(egui::Id::new("popup_toolbar")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(egui::RichText::new("Popup").small().color(Color32::GRAY));
                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(7.0 * (ICON + 8.0));
                    for btn in items {
                        let resp = icon_button(ui, &IconButton { name: btn.icon, tooltip: btn.tip, size: ICON, flyout: false, selected: false });
                        if resp.clicked() {
                            app.start_command(btn.left);
                            close = true;
                        }
                    }
                });
            });
        });
        let outside = ctx.input(|i| i.pointer.any_pressed()) && !ctx.input(|i| i.pointer.interact_pos().is_some_and(|p| area.response.rect.expand(4.0).contains(p)));
        if close || outside {
            app.popup_toolbar = None;
        }
    }
    let _ = (Layout::Four, DisplayMode::Shaded);
}
