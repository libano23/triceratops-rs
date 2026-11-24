//! Command registry: names, aliases, descriptions and the factory.

use super::annotate::*;
use super::blocks::*;
use super::cplane_cmds::*;
use super::create::*;
use super::curve2::*;
use super::curve_edit::*;
use super::edit::*;
use super::mesh_cmds::*;
use super::solid_edit::*;
use super::surface2::*;
use super::surface_edit::*;
use super::transform::*;
use super::misc::*;
use super::misc2::*;
use super::misc3::*;
use super::misc4::*;
use super::misc5::*;
use super::misc6::*;
use super::misc7::*;
use super::misc8::*;
use super::parametric_cmds::*;
use super::render_cmds::*;
use super::history_cmds::*;
use super::curves3::*;
use super::surfaces3::*;
use super::ui_cmds::*;
use super::mesh3::*;
use super::layouts;
use super::Step;
use super::mesh2::*;
use super::simple::*;
use super::mesh_cmds::{unweld, weld};
use tri_doc::Geometry;
use super::{AppRequest, Command, ObjFilter};

#[derive(Clone, Debug)]
pub struct CommandInfo {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
}

/// Every Triceratops command name: the implemented table plus the full Triceratops 8 catalog.
pub fn all_commands() -> Vec<CommandInfo> {
    let mut out: Vec<CommandInfo> = COMMANDS.iter().map(|c| CommandInfo { name: c.name, aliases: c.aliases, description: c.description }).collect();
    for (name, desc) in super::catalog::CATALOG {
        if !out.iter().any(|c| c.name.eq_ignore_ascii_case(name)) {
            out.push(CommandInfo { name, aliases: &[], description: desc });
        }
    }
    out.sort_by_key(|c| c.name.to_ascii_lowercase());
    out
}

/// Whether a command has an implementation in triceratops.
pub fn is_implemented(name: &str) -> bool {
    create_command(name).is_some()
}

macro_rules! cmds {
    ($( $name:literal, [$($alias:literal),*], $desc:literal );* $(;)?) => {
        pub const COMMANDS: &[CommandInfo] = &[ $( CommandInfo { name: $name, aliases: &[$($alias),*], description: $desc } ),* ];
    };
}

cmds! {
    "Theme", ["theme"], "Switch the user-interface theme (Triceratops Light, Triceratops Dark, Blender or a user theme file), save or export themes, choose the graphics backend";
    "Parametric", ["param"], "Parametric sketch mode (CADCraft-style): toggle automatic constraint solving, set the sketch plane";
    "GeomConstraint", ["gc"], "Add a geometric constraint between objects (Coincident, Collinear, Concentric, Fix, Parallel, Perpendicular, Horizontal, Vertical, Tangent, Smooth, Symmetric, Equal)";
    "GcCoincident", [], "Constrain two points to coincide";
    "GcCollinear", [], "Constrain two lines to be collinear";
    "GcConcentric", [], "Constrain two circles or arcs to share a center";
    "GcFix", [], "Fix an object in place";
    "GcParallel", [], "Constrain two lines to be parallel";
    "GcPerpendicular", [], "Constrain two lines to be perpendicular";
    "GcHorizontal", [], "Constrain a line to be horizontal in the sketch plane";
    "GcVertical", [], "Constrain a line to be vertical in the sketch plane";
    "GcTangent", [], "Constrain two curves to be tangent";
    "GcSmooth", [], "Constrain two curves to join smoothly";
    "GcSymmetric", [], "Constrain two objects to be symmetric about a line";
    "GcEqual", [], "Constrain two objects to have equal length or radius";
    "DimConstraint", ["dc"], "Add a dimensional constraint with a value or expression (Aligned, HorizontalDistance, VerticalDistance, Angular, Radius, Diameter)";
    "DcAligned", [], "Constrain the distance between two points";
    "DcHorizontal", [], "Constrain the horizontal distance between two points";
    "DcVertical", [], "Constrain the vertical distance between two points";
    "DcAngular", [], "Constrain the angle between two lines";
    "DcRadius", [], "Constrain the radius of a circle or arc";
    "DcDiameter", [], "Constrain the diameter of a circle or arc";
    "AutoConstrain", [], "Infer and add constraints between selected objects";
    "DelConstraint", [], "Remove constraints from objects";
    "Parameters", [], "List, set or delete user parameters and dimensional constraint expressions";
    "ConstraintBar", [], "List the constraints and show or hide the constraint glyphs";
    "ConstraintSettings", [], "Constraint inference tolerances";
    "Point", ["pt"], "Draw a single point object";
    "Points", [], "Draw multiple point objects";
    "Line", ["l"], "Draw a single line segment";
    "Lines", [], "Draw multiple adjoining line segments";
    "Polyline", ["pl"], "Draw a multi-segment polyline with options for line and arc segments, tracking line helpers, and close";
    "Rectangle", ["rec", "rect"], "Draw a rectangular closed polyline with options for starting at the center, three points, vertical, and rounded corners";
    "Circle", ["c"], "Draw a circle from center and radius, diameter, points on the circumference, and circumference length";
    "Arc", ["a"], "Draw an arc with options for center, start, angle, and direction";
    "Arc3Pt", [], "Draw an arc from start, end and a point on the arc";
    "Ellipse", ["el"], "Draw a closed elliptical curve from center and axes";
    "Polygon", ["pg"], "Draw a polygon with a specified number of sides";
    "Curve", ["cv"], "Draw a curve from control point locations";
    "InterpCrv", ["ic", "interpcurve"], "Draw a curve through selected locations";
    "SrfPt", ["srfpt"], "Draw a surface from three or four corner points";
    "Plane", ["planesrf"], "Draw a rectangular planar surface";
    "Box", ["b"], "Draw a solid box";
    "Sphere", ["sph"], "Draw a solid sphere";
    "Cylinder", ["cyl"], "Draw a solid cylinder";
    "Cone", [], "Draw a solid cone";
    "Torus", [], "Draw a torus";
    "ExtrudeCrv", ["ext", "extrude"], "Drive a curve in a straight line to create a surface or solid";
    "Revolve", ["rev"], "Revolve a curve around an axis to create a surface";
    "Loft", [], "Fit a surface through selected profile curves";
    "Cap", [], "Fill planar openings with surfaces to make closed polysurfaces";
    "ExtrudeSrf", [], "Drive a surface in a straight line to create a solid";
    "OffsetSrf", [], "Copy a surface so that all locations on the copy are the same distance from the original";
    "PlanarSrf", [], "Create planar surfaces from closed planar curves";
    "Delete", ["del", "erase"], "Delete selected objects";
    "Undo", ["u"], "Undo the last action";
    "Redo", ["r"], "Reverse the last Undo";
    "Move", ["m"], "Move objects from one location to another";
    "Copy", ["co"], "Duplicate objects";
    "Rotate", ["ro"], "Rotate objects around an axis perpendicular to the construction plane";
    "Scale", ["sc"], "Change the size of objects uniformly in the x, y, and z directions";
    "Scale1D", [], "Change the size of objects in one direction";
    "Scale2D", [], "Change the size of objects uniformly in two directions";
    "Mirror", ["mi"], "Create a mirror-image copy of objects";
    "Join", ["j"], "Connect curves, surface edges, or surfaces to form a single object";
    "Chamfer", [], "Create a line segment between two curves and trim or extend the curves to meet it";
    "Divide", [], "Divide curves by number of segments or segment length";
    "Intersect", [], "Create points at the intersections of curves";
    "Explode", ["x"], "Break objects down into components";
    "Hide", ["h"], "Conceal objects from view";
    "Show", ["sh"], "Redisplay all hidden objects";
    "Lock", [], "Lock objects so they cannot be selected for editing";
    "Unlock", [], "Unlock all locked objects";
    "SelAll", ["sa", "selectall"], "Select all objects";
    "SelNone", ["sn", "selectnone"], "Deselect all objects";
    "Invert", ["inv"], "Invert the selection";
    "SelCrv", [], "Select all curves";
    "SelSrf", [], "Select all surfaces";
    "SelPolysrf", [], "Select all polysurfaces";
    "SelPt", [], "Select all point objects";
    "SelMesh", [], "Select all meshes";
    "SelLast", [], "Select the last changed objects";
    "SelPrev", [], "Select the previous selection set";
    "Layer", ["lay"], "Manage layer properties";
    "ChangeLayer", ["cl"], "Change the layer of selected objects";
    "Properties", ["w", "what", "props"], "Manage object properties for the selected objects";
    "What", [], "Report the details of an object";
    "Distance", ["dist"], "Report the distance between two points";
    "Length", ["len"], "Report the length of curves";
    "Zoom", ["z"], "Zoom the view with options for extents, selected, window, dynamic, and all";
    "ZoomExtents", ["ze"], "Zoom the view to show all objects";
    "ZoomSelected", ["zs"], "Zoom the view to show all selected objects";
    "ZoomExtentsAll", ["zea"], "Zoom all viewports to show all objects";
    "4View", ["4", "fourview"], "Set up a four-viewport workspace";
    "3View", [], "Set up a three-viewport workspace";
    "MaxViewport", ["max"], "Maximize the active viewport";
    "SetView", [], "Set the view to a standard view";
    "Perspective", ["persp"], "Set the view to perspective";
    "Top", [], "Set the view to top";
    "Front", [], "Set the view to front";
    "Right", [], "Set the view to right";
    "Bottom", [], "Set the view to bottom";
    "Back", [], "Set the view to back";
    "Left", [], "Set the view to left";
    "SetDisplayMode", [], "Set the display mode of the active viewport";
    "Shaded", ["sf"], "Set the viewport to opaque shaded mode";
    "Wireframe", ["wf"], "Set the viewport display to unshaded wireframe";
    "Rendered", [], "Shade the viewport with a simulation of the rendered view";
    "Ghosted", [], "Set viewport display so surfaces are translucent";
    "XRay", ["xray"], "Shade with isoparametric curves not obscured by objects in front";
    "Pen", [], "Use white with black lines to simulate a pen drawing";
    "Technical", [], "Simulate technical drawing of 3-D objects";
    "Arctic", [], "Use white for all objects and the background";
    "Grid", [], "Set grid properties";
    "Snap", ["gridsnap"], "Toggle grid snap";
    "Ortho", [], "Toggle ortho";
    "Planar", [], "Toggle planar mode";
    "Osnap", [], "Toggle object snaps";
    "DisableOsnap", [], "Suspend persistent object snaps";
    "ProjectOsnap", [], "Project object snaps to the construction plane";
    "ShowOsnap", [], "Show the Osnap panel";
    "UndoView", [], "Undo the last view change";
    "RedoView", [], "Reverse the last UndoView";
    "New", [], "Create a new model";
    "Open", [], "Open an existing model file";
    "Save", [], "Save the current model";
    "SaveAs", [], "Save the current model under a new name";
    "Import", [], "Merge objects from another file";
    "Export", [], "Save selected objects to a new file";
    "Exit", ["quit"], "Exit Triceratops";
    "Help", ["?"], "Open the Help topic for the current command";
    "CommandHistory", [], "Open the command history window";
    "CommandList", ["commands"], "List all commands";
    "Options", [], "Manage global options";
    "DocumentProperties", ["docprops"], "Manage the settings for the current model";
    "Cancel", [], "Cancel the current command and deselect objects";
    "Dir", [], "Display and edit an object's normal direction";
    "Flip", [], "Reverse the normal direction of a curve or surface";
    "Group", ["g"], "Organize objects into a single component";
    "Ungroup", [], "Remove the group status from objects";
    "Trim", ["tr"], "Cut and delete selected portions of an object";
    "Split", ["sp"], "Divide objects using other objects as cutters";
    "Fillet", ["fi"], "Add an arc between two curves and trim or extend the curves to the arc";
    "FilletSrf", [], "Create a constant-radius round surface between two surfaces";
    "BooleanUnion", [], "Combine the volumes of one or more objects";
    "BooleanDifference", [], "Subtract the volume of one set of objects from another";
    "BooleanIntersection", [], "Intersect the volumes of two sets of objects";
    "Project", [], "Project curves or points toward a construction plane onto a surface";
    "Mesh", [], "Create a mesh from a NURBS surface or polysurface";
    "EditPtOn", [], "Display edit points on curves";
    "PointsOn", ["pon"], "Display curve and surface control points";
    "PointsOff", ["poff"], "Turn off control, edit, and solid points display";
    "TextObject", [], "Draw text-shaped curves";
    "Array", [], "Copy and space objects in columns, rows, and levels";
    "Align", [], "Align objects by their bounding boxes";
    "FlowAlongSrf", [], "Re-align objects from a source surface to a target surface";
    "PushPull", [], "Extrude a polysurface face interactively";
    "Check", [], "Report errors in an object's data structure";
    "Block", [], "Define a block object";
    "History", [], "Store the connection between a command's input geometry and the result";
    "Offset", [], "Copy a curve so that all locations on the copy are the same distance from the original";
    "Extend", [], "Lengthen a curve";
    "Rebuild", [], "Reconstruct selected curves or surfaces to a specified degree and control point count";
    "CPlane", [], "Set the construction plane in the active viewport";
    "Pan", [], "Shift the location of the view camera and target parallel to the view plane";
    "RotateView", [], "Rotate the view camera around the target";
    "SelectionFilter", [], "Set the selection filter";
    "Cut", [], "Delete objects and place them in the Clipboard";
    "CopyToClipboard", [], "Copy objects to the Clipboard";
    "Paste", [], "Insert objects from the Clipboard";
    "Area", [], "Report the area of closed curves and surfaces";
    "Volume", [], "Report the volume of closed surfaces";
    "EvaluatePt", [], "Report the coordinates of a location";
    "Angle", [], "Report the angle between two lines";
    "ViewportProperties", [], "Manage viewport properties";
    "ViewportTabs", [], "Display a tab control along the viewport edge";
    "Gumball", [], "Toggle the gumball widget";
    "SmartTrack", [], "Toggle SmartTrack";
    "Notes", [], "Open the Notes panel";
    "Print", [], "Print the current viewport";
    "Render", [], "Render the objects using the current renderer";
}

fn catalog_info(name: &str) -> Option<CommandInfo> {
    super::catalog::CATALOG
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(n, d)| CommandInfo { name: n, aliases: &[], description: d })
}

/// Resolve a typed name (case-insensitive, optional leading `_`, `!` or `-`).
pub fn find_command(typed: &str) -> Option<CommandInfo> {
    let t = typed.trim().trim_start_matches(['_', '!', '-', '\'']).to_ascii_lowercase();
    if t.is_empty() {
        return None;
    }
    if let Some(c) = COMMANDS.iter().find(|c| c.name.eq_ignore_ascii_case(&t) || c.aliases.iter().any(|a| a.eq_ignore_ascii_case(&t))) {
        return Some(CommandInfo { name: c.name, aliases: c.aliases, description: c.description });
    }
    catalog_info(&t)
}

pub fn command_info(name: &str) -> Option<CommandInfo> {
    COMMANDS
        .iter()
        .find(|c| c.name == name)
        .map(|c| CommandInfo { name: c.name, aliases: c.aliases, description: c.description })
        .or_else(|| catalog_info(name))
}

/// Autocomplete: exact prefix matches first, then names containing the text (fuzzy).
pub fn lookup(typed: &str, limit: usize) -> (Vec<CommandInfo>, Vec<CommandInfo>) {
    let t = typed.trim().trim_start_matches(['_', '!', '-']).to_ascii_lowercase();
    if t.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let all = all_commands();
    let mut exact: Vec<CommandInfo> = all.iter().filter(|c| c.name.to_ascii_lowercase().starts_with(&t)).cloned().collect();
    // implemented commands first, then alphabetical
    exact.sort_by_key(|c| (!is_implemented(c.name), c.name.to_ascii_lowercase()));
    let mut fuzzy: Vec<CommandInfo> = all
        .iter()
        .filter(|c| !c.name.to_ascii_lowercase().starts_with(&t) && c.name.to_ascii_lowercase().contains(&t))
        .cloned()
        .collect();
    fuzzy.sort_by_key(|c| (!is_implemented(c.name), c.name.to_ascii_lowercase()));
    exact.truncate(limit);
    fuzzy.truncate(limit.saturating_sub(exact.len()).max(3));
    (exact, fuzzy)
}

pub fn create_command(name: &str) -> Option<Box<dyn Command>> {
    Some(match name {
        "Point" => Box::new(PointCmd::new(false)),
        "Points" => Box::new(PointCmd::new(true)),
        "Line" => Box::new(LineCmd::default()),
        "Lines" => Box::new(PolylineCmd::lines()),
        "Polyline" => Box::new(PolylineCmd::default()),
        "Rectangle" => Box::new(RectangleCmd::default()),
        "Circle" => Box::new(CircleCmd::default()),
        "Arc" => Box::new(ArcCmd::center()),
        "Arc3Pt" => Box::new(ArcCmd::three_point()),
        "Ellipse" => Box::new(EllipseCmd::default()),
        "Polygon" => Box::new(PolygonCmd::default()),
        "Curve" => Box::new(CurveCmd::control_points()),
        "InterpCrv" => Box::new(CurveCmd::interpolated()),
        "SrfPt" => Box::new(SrfPtCmd::default()),
        "Plane" => Box::new(PlaneCmd::default()),
        "Box" => Box::new(BoxCmd::default()),
        "Sphere" => Box::new(SphereCmd::default()),
        "Cylinder" => Box::new(CylinderCmd::new(Solid::Cylinder)),
        "Cone" => Box::new(CylinderCmd::new(Solid::Cone)),
        "Torus" => Box::new(TorusCmd::default()),
        "ExtrudeCrv" => Box::new(ExtrudeCmd::default()),
        "Revolve" => Box::new(RevolveCmd::default()),
        "Loft" => Box::new(LoftCmd::default()),
        "Cap" => Box::new(CapCmd),
        "ExtrudeSrf" => Box::new(ExtrudeSrfCmd::default()),
        "EdgeSrf" => Box::new(EdgeSrfCmd),
        "NetworkSrf" => Box::new(EdgeSrfCmd),
        "RuledSrf" => Box::new(EdgeSrfCmd),
        "Sweep1" => Box::new(Sweep1Cmd::new(false)),
        "ExtrudeCrvAlongCrv" => Box::new(Sweep1Cmd::new(true)),
        "Sweep2" => Box::new(Sweep2Cmd::default()),
        "Pipe" => Box::new(PipeCmd::default()),
        "ExtrudeCrvTapered" => Box::new(ExtrudeTaperedCmd::default()),
        "ExtrudeCrvToPoint" => Box::new(ExtrudeToPointCmd::default()),
        "ExtendSrf" => Box::new(ExtendSrfCmd),
        "Untrim" => Box::new(UntrimCmd),
        "MeshToNURB" => Box::new(MeshToNurbCmd),
        "Ellipsoid" => Box::new(EllipsoidCmd::default()),
        "Pyramid" => Box::new(PyramidCmd::default()),
        "TCone" => Box::new(TConeCmd::default()),
        "Tube" => Box::new(TubeCmd::default()),
        "Slab" => Box::new(SlabCmd::default()),
        "BooleanSplit" => Box::new(BooleanSplitCmd::default()),
        "ExtractSrf" => Box::new(ExtractSrfCmd),
        "Mesh" => Box::new(MeshCmd),
        "Text" => Box::new(TextCmd::new(false)),
        "Block" => Box::new(BlockCmd::default()),
        "FilletEdge" => Box::new(EdgeRoundCmd::new(false)),
        "SubCrv" => Box::new(CurveUtilCmd::new(CurveUtil::SubCrv)),
        "PlanarUnion" => Box::new(PlanarBoolCmd::new(tri_geom::BooleanOp::Union, false)),
        "UnrollSrf" => Box::new(UnrollCmd("UnrollSrf")),
        "Pull" => Box::new(PullCmd::default()),
        "Theme" => Box::new(theme_cmd()),
        "Parametric" => Box::new(ParametricCmd),
        "GeomConstraint" => Box::new(GeomConstraintCmd::new(None)),
        "GcCoincident" => Box::new(GeomConstraintCmd::new(Some("Coincident"))),
        "GcCollinear" => Box::new(GeomConstraintCmd::new(Some("Collinear"))),
        "GcConcentric" => Box::new(GeomConstraintCmd::new(Some("Concentric"))),
        "GcFix" => Box::new(GeomConstraintCmd::new(Some("Fix"))),
        "GcParallel" => Box::new(GeomConstraintCmd::new(Some("Parallel"))),
        "GcPerpendicular" => Box::new(GeomConstraintCmd::new(Some("Perpendicular"))),
        "GcHorizontal" => Box::new(GeomConstraintCmd::new(Some("Horizontal"))),
        "GcVertical" => Box::new(GeomConstraintCmd::new(Some("Vertical"))),
        "GcTangent" => Box::new(GeomConstraintCmd::new(Some("Tangent"))),
        "GcSmooth" => Box::new(GeomConstraintCmd::new(Some("Smooth"))),
        "GcSymmetric" => Box::new(GeomConstraintCmd::new(Some("Symmetric"))),
        "GcEqual" => Box::new(GeomConstraintCmd::new(Some("Equal"))),
        "DimConstraint" => Box::new(DimConstraintCmd::new(None)),
        "DcAligned" => Box::new(DimConstraintCmd::new(Some("Aligned"))),
        "DcHorizontal" => Box::new(DimConstraintCmd::new(Some("HorizontalDistance"))),
        "DcVertical" => Box::new(DimConstraintCmd::new(Some("VerticalDistance"))),
        "DcAngular" => Box::new(DimConstraintCmd::new(Some("Angular"))),
        "DcRadius" => Box::new(DimConstraintCmd::new(Some("Radius"))),
        "DcDiameter" => Box::new(DimConstraintCmd::new(Some("Diameter"))),
        "AutoConstrain" => Box::new(auto_constrain()),
        "DelConstraint" => Box::new(DelConstraintCmd),
        "Parameters" => Box::new(ParametersCmd::default()),
        "ConstraintBar" => Box::new(ConstraintBarCmd),
        "ConstraintSettings" => Box::new(ConstraintSettingsCmd),

        "ViewCaptureToFile" => Box::new(capture_cmd("ViewCaptureToFile", false, false)),
        "ViewCaptureToClipboard" => Box::new(capture_cmd("ViewCaptureToClipboard", true, false)),
        "ScreenCaptureToFile" => Box::new(capture_cmd("ScreenCaptureToFile", false, true)),
        "ScreenCaptureToClipboard" => Box::new(capture_cmd("ScreenCaptureToClipboard", true, true)),
        "Print" => Box::new(capture_cmd("Print (PNG export of the active viewport)", false, false)),
        "Render" => Box::new(render_cmd()),
        "BatchRenderNamedViews" => Box::new(batch_render()),
        "Turntable" => Box::new(TurntableCmd::default()),
        "SetTurntableAnimation" => Box::new(TurntableCmd::default()),
        "Sun" => Box::new(SunCmd),
        "GroundPlane" => Box::new(GroundPlaneCmd),
        "Gumball" => Box::new(set_mode_cmd("gumball")),
        "BringForward" => Box::new(draw_order(1, false)),
        "SendBackward" => Box::new(draw_order(-1, false)),
        "BringToFront" => Box::new(draw_order(1, true)),
        "SendToBack" => Box::new(draw_order(-1, true)),
        "ClearDrawOrder" => Box::new(draw_order(0, true)),
        "BoxEdit" => Box::new(box_edit()),
        "CalcRPN" => Box::new(calc_rpn()),
        "CreaseSplitting" => Box::new(FlagCmd("CreaseSplitting", "Yes", "Crease splitting: surfaces with kinks are split into polysurface faces at creation.")),
        "DupFaceBorder" => Box::new(dup_face_border()),
        "FitCurveToSurface" => Box::new(PullCmd::default()),
        "FitSrf" => Box::new(RebuildSrfCmd::default()),
        "RebuildUV" => Box::new(RebuildSrfCmd::default()),
        "JoinCopy" => Box::new(join_copy()),
        "MergeFaces" => Box::new(merge_faces()),
        "MoveExtractedIsocurve" => Box::new(move_extracted_isocurve()),
        "PointCloud" => Box::new(point_cloud()),
        "ReducePointCloud" => Box::new(reduce_point_cloud()),
        "SplitFace" => Box::new(split_face()),
        "TruncatedCone" => Box::new(TConeCmd::default()),
        "ZoomEnds" => Box::new(zoom_special("ZoomEnds")),
        "ZoomNaked" => Box::new(zoom_special("ZoomNaked")),
        "ZoomNonManifold" => Box::new(zoom_special("ZoomNonManifold")),

        "DigBeep" => Box::new(MsgCmd("Digitizing arms are not supported by triceratops.")),
        "DigCalibrate" => Box::new(MsgCmd("Digitizing arms are not supported by triceratops.")),
        "DigCamera" => Box::new(MsgCmd("Digitizing arms are not supported by triceratops.")),
        "DigClick" => Box::new(MsgCmd("Digitizing arms are not supported by triceratops.")),
        "DigDisconnect" => Box::new(MsgCmd("Digitizing arms are not supported by triceratops.")),
        "DigLine" => Box::new(MsgCmd("Digitizing arms are not supported by triceratops.")),
        "DigPause" => Box::new(MsgCmd("Digitizing arms are not supported by triceratops.")),
        "DigScale" => Box::new(MsgCmd("Digitizing arms are not supported by triceratops.")),
        "DigSection" => Box::new(MsgCmd("Digitizing arms are not supported by triceratops.")),
        "DigSketch" => Box::new(MsgCmd("Digitizing arms are not supported by triceratops.")),
        "Digitize" => Box::new(MsgCmd("Digitizing arms are not supported by triceratops.")),
        "Grasshopper" => Box::new(MsgCmd("Grasshopper is not available in triceratops.")),
        "GrasshopperDeveloperSettings" => Box::new(MsgCmd("Grasshopper is not available in triceratops.")),
        "GrasshopperFolders" => Box::new(MsgCmd("Grasshopper is not available in triceratops.")),
        "GrasshopperGetSDKDocumentation" => Box::new(MsgCmd("Grasshopper is not available in triceratops.")),
        "GrasshopperIgnorePlugin" => Box::new(MsgCmd("Grasshopper is not available in triceratops.")),
        "GrasshopperLoadOneByOne" => Box::new(MsgCmd("Grasshopper is not available in triceratops.")),
        "GrasshopperPlayer" => Box::new(MsgCmd("Grasshopper is not available in triceratops.")),
        "GrasshopperPluginList" => Box::new(MsgCmd("Grasshopper is not available in triceratops.")),
        "CheckInLicense" => Box::new(MsgCmd("triceratops is free software; no license management is needed.")),
        "CheckOutLicense" => Box::new(MsgCmd("triceratops is free software; no license management is needed.")),
        "Login" => Box::new(MsgCmd("triceratops is free software; no license management is needed.")),
        "Logout" => Box::new(MsgCmd("triceratops is free software; no license management is needed.")),
        "MigratePlugins" => Box::new(FnCmd(|ctx| { ctx.log("triceratops has no plug-in system; Python scripts run with RunPythonScript."); Step::Done })),
        "PackageManager" => Box::new(open_url_cmd("", Some("https://example.invalid/triceratops/packages"))),
        "PlugInManager" => Box::new(MsgCmd("triceratops has no plug-in system; Python scripts run with RunPythonScript.")),
        "SetCurrentRenderPlugIn" => Box::new(MsgCmd("The built-in renderer is the only render engine; Render captures the Rendered display mode.")),
        "GetIssueState" => Box::new(get_issue_state()),
        "IGESStudy" => Box::new(MsgCmd("IGES import/export uses fixed options (NURBS curves, surfaces and trimmed faces); there is no IGES options dialog.")),
        "IgesImportOptions" => Box::new(MsgCmd("IGES import/export uses fixed options (NURBS curves, surfaces and trimmed faces); there is no IGES options dialog.")),
        "ReadEveryIGESEntity" => Box::new(MsgCmd("IGES import/export uses fixed options (NURBS curves, surfaces and trimmed faces); there is no IGES options dialog.")),
        "SetIgesLayerLevelMap" => Box::new(MsgCmd("IGES import/export uses fixed options (NURBS curves, surfaces and trimmed faces); there is no IGES options dialog.")),
        "STEPTree" => Box::new(MsgCmd("STEP export always writes millimetres with 0.001 tolerance; there is no STEP options dialog.")),
        "StepUnitsAndTolerance" => Box::new(MsgCmd("STEP export always writes millimetres with 0.001 tolerance; there is no STEP options dialog.")),
        "Worksession" => Box::new(worksession()),
        "PurgeRefObjects" => Box::new(purge_ref_objects()),
        "LimitReferenceModel" => Box::new(limit_reference_model()),
        "ApplyBoxMapping" => Box::new(apply_mapping(tri_doc::MappingKind::Box)),
        "ApplyCustomMapping" => Box::new(apply_mapping(tri_doc::MappingKind::Surface)),
        "ApplyCylindricalMapping" => Box::new(apply_mapping(tri_doc::MappingKind::Cylindrical)),
        "ApplyOcsMapping" => Box::new(apply_mapping(tri_doc::MappingKind::Planar)),
        "ApplyPlanarMapping" => Box::new(apply_mapping(tri_doc::MappingKind::Planar)),
        "ApplySphericalMapping" => Box::new(apply_mapping(tri_doc::MappingKind::Spherical)),
        "ApplySurfaceMapping" => Box::new(apply_mapping(tri_doc::MappingKind::Surface)),
        "MappingWidget" => Box::new(mapping_widget(true)),
        "MappingWidgetOff" => Box::new(mapping_widget(false)),
        "Unwrap" => Box::new(apply_mapping(tri_doc::MappingKind::Planar)),
        "UVEditor" => Box::new(apply_mapping(tri_doc::MappingKind::Planar)),
        "PackTextures" => Box::new(pack_textures(true)),
        "UnpackTextures" => Box::new(pack_textures(false)),
        "Bake" => Box::new(compute_vertex_colors("Bake")),
        "BakeMapping" => Box::new(compute_vertex_colors("BakeMapping")),
        "AssignBlankTexture" => Box::new(assign_texture()),
        "ExtractCustomMappingObject" => Box::new(extract_custom_mapping()),
        "RemoveMappingChannel" => Box::new(remove_mapping()),
        "MatchMapping" => Box::new(match_mapping()),
        "SetMeshSurfaceParameters" => Box::new(set_mesh_surface_parameters()),
        "ComputeVertexColors" => Box::new(compute_vertex_colors("ComputeVertexColors")),
        "DownloadLibraryTextures" => Box::new(download_library_textures()),
        "RefreshAllTextures" => Box::new(refresh_textures()),
        "ExtractUVMesh" => Box::new(extract_uv_mesh()),
        "SelMappingWidget" => Box::new(SelByCmd { label: "SelMappingWidget", pred: |_, o| o.attributes.mapping.is_some() }),
        "Picture" => Box::new(PictureCmd::new(false)),
        "BackgroundBitmap" => Box::new(PictureCmd::new(true)),
        "PerspectiveMatch" => Box::new(perspective_match()),
        "AcadSchemes" => Box::new(acad_schemes()),
        "Alerter" => Box::new(alerter()),
        "AttachGHSData" => Box::new(MsgCmd("GHS hydrostatics data is not supported; use Hydrostatics for the displacement.")),
        "AutomaticSubDFromMesh" => Box::new(automatic_subd_from_mesh()),
        "CopyLinkedBlockDefinition" => Box::new(copy_block_definition()),
        "ExportRuiFile" => Box::new(export_rui()),
        "Notifications" => Box::new(notifications()),
        "ResetMessageBoxes" => Box::new(reset_message_boxes()),
        "DecimalPoint" => Box::new(decimal_point()),
        "ImportAnnotationStyles" => Box::new(MsgCmd("triceratops has one annotation style; text height is set with the Height option of Text and Dim commands.")),
        "DupAnnotationStyle" => Box::new(MsgCmd("triceratops has one annotation style; use MatchAnnotation to copy text heights.")),
        "ConvertTextToBlockAttribute" => Box::new(MsgCmd("Block attributes are not stored; the text stays an ordinary text object in the block.")),
        "AddMissingBlockAttributeKeys" => Box::new(MsgCmd("Block attributes are not stored in triceratops.")),
        "CheckNewObjects" => Box::new(check_new_objects()),
        "Macros" => Box::new(macros_cmd()),
        "LoadScript" => Box::new(load_script(false)),
        "RunScript" => Box::new(run_loaded_script()),
        "ToolbarLock" => Box::new(FlagCmd("Toolbar.Locked", "Toggle", "Toolbar lock toggled: toolbars cannot be dragged while locked.")),
        "ShowToolbar" => Box::new(ReqCmd(AppRequest::ToggleUi("left_sidebar".into()))),
        "Menus" => Box::new(ReqCmd(AppRequest::ToggleUi("menus".into()))),
        "PopupMenu" => Box::new(ReqCmd(AppRequest::ToggleUi("popup".into()))),
        "PopupPopular" => Box::new(ReqCmd(AppRequest::ToggleUi("popup".into()))),
        "PopupToolbar" => Box::new(ReqCmd(AppRequest::ToggleUi("popup".into()))),
        "SaveWindowLayout" => Box::new(SeqCmd::new(vec![Ask::Text("Layout file name (Enter for a dialog)")], |a, ctx| {
            ctx.requests.push(AppRequest::LayoutFile { op: "save".into(), path: a[0].text() });
            Step::Done
        })),
        "WindowLayout" => Box::new(SeqCmd::new(vec![Ask::Text("Layout file name (Enter for a dialog)")], |a, ctx| {
            ctx.requests.push(AppRequest::LayoutFile { op: "load".into(), path: a[0].text() });
            Step::Done
        })),
        "Reset" => Box::new(ReqCmd(AppRequest::LayoutFile { op: "reset".into(), path: String::new() })),
        "CommandPrompt" => Box::new(OptCmd("command_area", "Toggle")),
        "DisplayCommandPrompt" => Box::new(OptCmd("command_area", "Toggle")),
        "MultiPause" => Box::new(PauseCmd("MultiPause")),
        "Pause" => Box::new(PauseCmd("Pause")),
        "Enter" => Box::new(PauseCmd("Enter")),
        "EnterEnd" => Box::new(PauseCmd("EnterEnd")),
        "SelLinetype" => Box::new(sel_linetype()),
        "Linetypes" => Box::new(LinetypesCmd),
        "LinetypeDisplay" => Box::new(FlagCmd("LinetypeDisplay", "Model", "Linetype patterns are scaled in model units (see SetLinetypeScale).")),
        "SetLinetype" => Box::new(set_linetype()),
        "SetLayerLinetype" => Box::new(set_layer_linetype()),
        "SetLinetypeScale" => Box::new(set_linetype_scale()),
        "SetCustomLinetype" => Box::new(set_custom_linetype()),
        "ExtractLineTypeSegments" => Box::new(extract_linetype_segments()),
        "History" => Box::new(HistoryCmd),
        "HistoryPurge" => Box::new(history_purge()),
        "HistoryUpdate" => Box::new(history_update()),
        "SelObjectsWithHistory" => Box::new(sel_history("all")),
        "SelChildren" => Box::new(sel_history("children")),
        "SelParents" => Box::new(sel_history("parents")),
        "SelExtrusion" => Box::new(sel_extrusion()),
        "SelPicture" => Box::new(sel_picture()),
        "SelDetail" => Box::new(layouts::sel_detail()),
        "SelNamedViewWidget" => Box::new(FnCmd(|ctx| { ctx.log("Named view widgets are not drawn; use NamedView to restore a view."); Step::Done })),
        "SelCaptives" => Box::new(sel_cage("captives")),
        "SelControls" => Box::new(sel_cage("controls")),
        "Cage" => Box::new(cage_cmd()),
        "CageEdit" => Box::new(CageEditCmd::default()),
        "ReleaseFromCage" => Box::new(release_from_cage()),
        "ExtractOriginalCaptives" => Box::new(extract_original_captives()),
        "SelDimOverride" => Box::new(sel_dim_override(false)),
        "SelDimTextOverride" => Box::new(sel_dim_override(true)),
        "SelMirroredBlocks" => Box::new(sel_mirrored_blocks()),
        "SelEdgeLoop" => Box::new(sel_edges_of_mesh("SelEdgeLoop")),
        "SelEdgeRing" => Box::new(sel_edges_of_mesh("SelEdgeRing")),
        "SelFaceLoop" => Box::new(sel_edges_of_mesh("SelFaceLoop")),
        "SelFacesToBoundary" => Box::new(sel_faces_to_boundary()),
        "SelSubDEdges" => Box::new(sel_edges_of_mesh("SelSubDEdges")),
        "SelMeshEdges" => Box::new(sel_edges_of_mesh("SelMeshEdges")),
        "SelNakedMeshEdgePt" => Box::new(sel_naked_mesh_edge_pt()),
        "SelectionFilterEdges" => Box::new(selection_filter_sub("Edges")),
        "SelectionFilterFaces" => Box::new(selection_filter_sub("Faces")),
        "SelectionFilterVertices" => Box::new(selection_filter_sub("Vertices")),
        "SelectionFilterToggle" => Box::new(selection_filter_toggle()),
        "Layout" => Box::new(layouts::layout()),
        "Layouts" => Box::new(layouts::layouts()),
        "LayoutProperties" => Box::new(layouts::layout_properties()),
        "Detail" => Box::new(layouts::detail()),
        "ChangeSpace" => Box::new(layouts::change_space()),
        "CopyLayout" => Box::new(layouts::copy_layout()),
        "ImportLayout" => Box::new(layouts::import_layout()),
        "HideInDetail" => Box::new(layouts::detail_visibility("hide_in_detail")),
        "HideLayersInDetail" => Box::new(layouts::detail_visibility("hide_layers")),
        "ShowInDetail" => Box::new(layouts::detail_visibility("show_in_detail")),
        "ShowLayersInDetail" => Box::new(layouts::detail_visibility("show_layers")),
        "ShowSelectedInDetail" => Box::new(layouts::detail_visibility("show_selected_in_detail")),
        "CopyDetailToViewport" => Box::new(layouts::detail_viewport_copy(true)),
        "CopyViewportToDetail" => Box::new(layouts::detail_viewport_copy(false)),
        "PrintDisplay" => Box::new(print_display()),
        "SetDimensionLayer" => Box::new(set_dimension_layer()),
        "HatchBase" => Box::new(hatch_base()),
        "HatchScale" => Box::new(hatch_scale()),
        "AddGuide" => Box::new(add_guide()),
        "RemoveGuide" => Box::new(remove_guide()),
        "AutoAlignCPlane" => Box::new(CPlaneOpCmd("auto_align")),
        "MPlane" => Box::new(mplane()),
        "OneView" => Box::new(CPlaneOpCmd("one_view")),
        "CopyCPlaneToAll" => Box::new(CPlaneOpCmd("copy_all")),
        "CopyCPlaneSettingsToAll" => Box::new(CPlaneOpCmd("copy_all")),
        "SynchronizeCPlanes" => Box::new(CPlaneOpCmd("sync")),
        "SetZoomExtentsBorder" => Box::new(zoom_border()),
        "Zoom1To1Calibrate" => Box::new(zoom_calibrate()),
        "SnapToLocked" => Box::new(FlagCmd("Osnap.Locked", "Toggle", "Object snaps now also use locked objects.")),
        "SnapToOccluded" => Box::new(FlagCmd("Osnap.Occluded", "Toggle", "Object snaps now also use hidden (occluded) points in shaded views.")),
        "SnapToMeshes" => Box::new(FlagCmd("Osnap.Meshes", "Toggle", "Object snaps now also use mesh vertices and edges.")),
        "SnapToMeshObject" => Box::new(FlagCmd("Osnap.Meshes", "Yes", "Object snaps use mesh vertices and edges.")),
        "SnapToSubDObject" => Box::new(FlagCmd("Osnap.SubD", "Yes", "Object snaps use SubD vertices and edges.")),
        "OrthoSnapToCPlaneZ" => Box::new(ortho_snap_cplane_z()),
        "DragMode" => Box::new(drag_mode()),
        "DragStrength" => Box::new(drag_strength()),
        "DragCopy" => Box::new(FlagCmd("Drag.Copy", "Toggle", "DragCopy: Alt-drag copies the selection.")),
        "RememberCopyOptions" => Box::new(FlagCmd("Copy.Remember", "Yes", "Copy command options are remembered between sessions.")),
        "CullControlPolygon" => Box::new(FlagCmd("Grips.Cull", "Toggle", "Back-facing control points are hidden while CullControlPolygon is on.")),
        "HBar" => Box::new(CurveUtilCmd::new(CurveUtil::ConvertToBeziers)),
        "HandleCurve" => Box::new(HandleCurveCmd::default()),
        "Sketch" => Box::new(SketchCmd),
        "SoftEditCrv" => Box::new(soft_edit("SoftEditCrv", ObjFilter::Curve)),
        "SoftEditSrf" => Box::new(soft_edit("SoftEditSrf", ObjFilter::Surface)),
        "SoftMove" => Box::new(soft_edit("SoftMove", ObjFilter::Any)),
        "SoftTransform" => Box::new(soft_edit("SoftTransform", ObjFilter::Any)),
        "FixedLengthCrvEdit" => Box::new(fixed_length_edit()),
        "EndBulge" => Box::new(end_bulge()),
        "MoveUVN" => Box::new(move_uvn()),
        "SetSurfaceTangent" => Box::new(match_srf()),
        "SrfSeam" => Box::new(split_spans(false)),
        "CrvSeam" => Box::new(crv_seam()),
        "MergeSrf" => Box::new(merge_srf()),
        "MergeEdge" => Box::new(merge_edge(false)),
        "MergeAllEdges" => Box::new(merge_edge(true)),
        "SplitEdge" => Box::new(split_edge()),
        "UnjoinEdge" => Box::new(unjoin_edge()),
        "JoinEdge" => Box::new(join_edge()),
        "RebuildEdges" => Box::new(rebuild_edges()),
        "RefitTrim" => Box::new(refit_trim()),
        "ReplaceEdge" => Box::new(replace_edge()),
        "RemoveAllNakedMicroEdges" => Box::new(remove_micro_edges()),
        "ExtractBadSrf" => Box::new(extract_bad_srf()),
        "MatchSrf" => Box::new(match_srf()),
        "ConnectSrf" => Box::new(connect_srf()),
        "NonmanifoldMerge" => Box::new(nonmanifold_merge()),
        "CreateRegions" => Box::new(create_regions()),
        "SphereTangentToThreeSurfaces" => Box::new(sphere_tangent_three()),
        "SplitRefitSurface" => Box::new(split_spans(false)),
        "FilletSrfCrv" => Box::new(BlendSrfCmd::default()),
        "FilletSrfToRail" => Box::new(BlendSrfCmd::default()),
        "VariableFilletSrf" => Box::new(FilletSrfCmd::default()),
        "VariableBlendSrf" => Box::new(BlendSrfCmd::default()),
        "VariableChamferSrf" => Box::new(chamfer_srf()),
        "VariableOffsetSrf" => Box::new(OffsetSrfCmd::default()),
        "CopyHole" => Box::new(copy_hole()),
        "MoveHole" => Box::new(move_hole()),
        "RotateHole" => Box::new(rotate_hole()),
        "MirrorHole" => Box::new(mirror_hole()),
        "ArrayHole" => Box::new(array_hole()),
        "ArrayHolePolar" => Box::new(array_hole_polar()),
        "RevolvedHole" => Box::new(revolved_hole()),
        "Boolean2Objects" => Box::new(boolean_two_objects()),
        "Clash" => Box::new(clash()),
        "QuadRemesh" => Box::new(quad_remesh()),
        "QuadrangulateMesh" => Box::new(ngons("QuadrangulateMesh")),
        "TriangulateNonPlanarQuads" => Box::new(triangulate("TriangulateNonPlanarQuads")),
        "TriangulateRenderMeshes" => Box::new(triangulate("TriangulateRenderMeshes")),
        "AddNgonsToMesh" => Box::new(ngons("AddNgonsToMesh")),
        "DeleteMeshNgons" => Box::new(ngons("DeleteMeshNgons")),
        "Merge2MeshFaces" => Box::new(merge_two_faces()),
        "MeshRepair" => Box::new(mesh_repair()),
        "Bevel" => Box::new(bevel()),
        "Bridge" => Box::new(bridge()),
        "Fill" => Box::new(fill_holes_cmd(true)),
        "Inset" => Box::new(inset()),
        "InsertEdge" => Box::new(mesh_pick("InsertEdge", "Select mesh near the edge to split", |m, p| tri_geom::mesh_ops::split_nearest_edge(m, p))),
        "Slide" => Box::new(slide()),
        "SubDCrease" => Box::new(crease_cmd(false)),
        "SubDExpandEdges" => Box::new(expand_edges()),
        "SubDSpinEdge" => Box::new(mesh_pick("SubDSpinEdge", "Select SubD near the edge to spin", |m, p| tri_geom::mesh_ops::swap_nearest_edge(m, p))),
        "SubDUnfriend" => Box::new(crease_cmd(true)),
        "PackSubDFaces" => Box::new(mesh_map("PackSubDFaces", "Select SubDs to repack", |m| super::mesh_cmds::weld(m, 1e-9 * (1.0 + m.bounding_box().diagonal())))),
        "RepairSubD" => Box::new(mesh_repair()),
        "Radiate" => Box::new(radiate(false)),
        "RadiateFind" => Box::new(radiate(true)),
        "RemoveSymmetry" => Box::new(remove_symmetry()),
        "MultiPipe" => Box::new(multipipe()),
        "SubDDisplayToggle" => Box::new(subd_display_toggle()),
        "SubDFaceEdgeVertexToggle" => Box::new(selection_filter_sub("Faces")),
        "Reflect" => Box::new(reflect()),
        "MakeSubDFriendly" => Box::new(make_subd_friendly()),
        "SelSubDFriendlyCrv" => Box::new(sel_subd_friendly_crv()),
        "ExtrudeSubD" => Box::new(extrude_subd()),
        "OffsetSubD" => Box::new(offset_subd()),
        "ApplyCrv" => Box::new(apply_crv()),
        "ApplyMesh" => Box::new(apply_mesh(false)),
        "ApplyMeshUVN" => Box::new(apply_mesh(true)),
        "DraftAnglePoint" => Box::new(draft_angle_point()),
        "DimRecenterText" => Box::new(dim_recenter_text()),
        "MatchAnnotation" => Box::new(match_annotation()),
        "RLeaderEdit" => Box::new(edit_text("RLeaderEdit")),
        "RTextEdit" => Box::new(edit_text("RTextEdit")),
        "TextProperties" => Box::new(edit_text("TextProperties")),
        "ConvertDots" => Box::new(convert_dots()),
        "SelAnnotationStyle" => Box::new(sel_by_text_key("SelAnnotationStyle", "TextHeight")),
        "SelFontUse" => Box::new(sel_by_text_key("SelFontUse", "Font")),
        "Hydrostatics" => Box::new(hydrostatics()),
        "CutVolume" => Box::new(cut_volume()),
        "DimCreaseAngle" => Box::new(dim_crease_angle()),
        "EdgeContinuity" => Box::new(edge_continuity()),
        "PointCloudContour" => Box::new(point_cloud_section(true)),
        "PointCloudSection" => Box::new(point_cloud_section(false)),
        "Drape" => Box::new(drape(false)),
        "DrapePt" => Box::new(drape(true)),
        "Heightfield" => Box::new(heightfield()),
        "Squish" => Box::new(UnrollCmd("Smash")),
        "SquishBack" => Box::new(MsgCmd("Squish keeps no link to the original: use Undo, or FlowAlongSrf from the flat pattern back onto the surface.")),
        "SquishInfo" => Box::new(FnCmd(|ctx| { ctx.log("Squish/Smash flatten by unrolling the surface grid along its rows (no stretch compensation)."); Step::Done })),
        "DevLoft" => Box::new(EdgeSrfCmd),
        "Splop" => Box::new(flow_along_srf("Splop")),
        "FlowAlongSrf" => Box::new(flow_along_srf("FlowAlongSrf")),
        "ScaleByPlane" => Box::new(scale_by_plane()),
        "RemapCPlane" => Box::new(remap_cplane()),
        "ArraySrf" => Box::new(array_srf()),
        "ArrayCrvOnSrf" => Box::new(array_crv_on_srf()),
        "TweenSurfaces" => Box::new(tween_surfaces()),
        "RailRevolve" => Box::new(rail_revolve()),
        "FoldFace" => Box::new(fold_face()),
        "PushPull" => Box::new(push_pull()),
        "WireCut" => Box::new(WireCutCmd::default()),
        "Boss" => Box::new(boss_rib(false)),
        "Rib" => Box::new(boss_rib(true)),
        "RibbonOffset" => Box::new(ribbon_offset()),
        "ExtrudeSrfAlongCrv" => Box::new(extrude_srf_along_crv()),
        "MoveUntrimmedEdge" => Box::new(MoveFaceCmd::new(true)),
        "MoveUntrimmedFace" => Box::new(MoveFaceCmd::new(false)),
        "Convert" => Box::new(convert()),
        "ArcBlend" => Box::new(arc_blend()),
        "Blend" => Box::new(BlendCrvCmd::default()),
        "Catenary" => Box::new(catenary()),
        "Hyperbola" => Box::new(hyperbola()),
        "Parabola3Pt" => Box::new(parabola_3pt()),
        "Crv2View" => Box::new(crv_2_view()),
        "CSec" => Box::new(csec()),
        "InsertLineIntoCrv" => Box::new(insert_line_into_crv()),
        "InsertEditPoint" => Box::new(ControlPointCmd("Insert")),
        "ContinueCurve" => Box::new(ContinueCurveCmd::new(false)),
        "ContinueInterpCrv" => Box::new(ContinueCurveCmd::new(true)),
        "CurveThroughPolyline" => Box::new(curve_through_polyline()),
        "CurveThroughSrfControlPt" => Box::new(curve_through_srf_control_pt()),
        "ExtendCrvOnSrf" => Box::new(extend_crv_on_srf()),
        "ExtendDynamic" => Box::new(ExtendCmd::default()),
        "GCon" => Box::new(gcon()),
        "Match" => Box::new(match_crv()),
        "ModifyRadius" => Box::new(modify_radius()),
        "MoveCrv" => Box::new(move_crv()),
        "OffsetCrvOnSrf" => Box::new(offset_crv_on_srf()),
        "OrientCrvToEdge" => Box::new(orient_crv_to_edge()),
        "OrientOnCrv" => Box::new(orient_on_crv()),
        "OrientOnSrf" => Box::new(orient_on_srf()),
        "RebuildCrvNonUniform" => Box::new(RebuildAnyCmd::default()),
        "RevCloud" => Box::new(RevCloudCmd::default()),
        "ShortPath" => Box::new(short_path()),
        "AlignProfiles" => Box::new(align_profiles()),
        "Arrowhead" => Box::new(arrowhead()),
        "FilletCorners" => Box::new(fillet_corners()),
        "MarkFoci" => Box::new(mark_foci()),
        "EvaluateUVPt" => Box::new(evaluate_uv_pt()),
        "PointsFromUV" => Box::new(points_from_uv()),
        "PointDeviation" => Box::new(point_deviation()),
        "CreateUVCrv" => Box::new(create_uv_crv()),
        "FlattenSrf" => Box::new(flatten_srf()),
        "ConvertToSingleSpans" => Box::new(split_spans(false)),
        "DivideAlongCreases" => Box::new(split_spans(true)),
        "CreateSolid" => Box::new(create_solid()),
        "ChamferSrf" => Box::new(chamfer_srf()),
        "Bounce" => Box::new(bounce()),
        "InfinitePlane" => Box::new(infinite_plane()),
        "PlaceHole" => Box::new(place_hole()),
        "TruncatedPyramid" => Box::new(truncated_pyramid()),
        "Paraboloid" => Box::new(paraboloid()),
        "ExtractPipedCurve" => Box::new(extract_piped_curve()),
        "ClearAllMeshes" => Box::new(ReqCmd(AppRequest::RefreshTextures)),
        "ClearAnalysisMeshes" => Box::new(ReqCmd(AppRequest::RefreshTextures)),
        "RefreshShade" => Box::new(ReqCmd(AppRequest::RefreshTextures)),
        "HideRenderMesh" => Box::new(render_mesh_visibility("hide")),
        "ShowRenderMesh" => Box::new(render_mesh_visibility("show")),
        "ToggleRenderMesh" => Box::new(render_mesh_visibility("toggle")),
        "SetObjectDisplayMode" => Box::new(set_object_display_mode()),
        "ClearAllObjectDisplayModes" => Box::new(clear_object_display_modes()),
        "ShowZBuffer" => Box::new(ReqCmd(AppRequest::SetDisplayMode("ZBuffer".into()))),
        "SynchronizeRenderColors" => Box::new(synchronize_render_colors()),
        "RemovePerFaceColors" => Box::new(set_per_face_color(true)),
        "SetPerFaceColorByFacePack" => Box::new(set_per_face_color(false)),
        "ClippingDrawings" => Box::new(clipping_drawings(false)),
        "EditClippingDrawings" => Box::new(clipping_drawings(true)),
        "UpdateClippingDrawings" => Box::new(clipping_drawings(true)),
        "NestedClippingDrawing" => Box::new(clipping_drawings(false)),
        "ExportClippingDrawings" => Box::new(export_clipping_sections()),
        "ExportClippingSectionInfo" => Box::new(export_clipping_section_info()),
        "ClippingSections" => Box::new(clipping_sections("sections")),
        "ClearClippingSections" => Box::new(clear_clipping_sections()),
        "ExtractClippingSections" => Box::new(clipping_sections("sections")),
        "ExtractClippingSlices" => Box::new(clipping_sections("slices")),
        "SaveClippingSectionCPlanes" => Box::new(clipping_section_views(true)),
        "SaveClippingSectionViews" => Box::new(clipping_section_views(false)),
        "ViewClippingSections" => Box::new(view_clipping_sections()),
        "ClippingPlaneProperties" => Box::new(clipping_plane_properties()),
        "EarthAnchorPoint" => Box::new(earth_anchor()),
        "ModelBasepoint" => Box::new(model_basepoint()),
        "LayerBook" => Box::new(layer_book()),
        "LayerStateManager" => Box::new(layer_state_manager()),
        "HighlightObjectLayers" => Box::new(highlight_object_layers()),
        "NamedSelections" => Box::new(named_selections()),
        "BlockEdit" => Box::new(block_edit()),
        "BlockResetScale" => Box::new(block_reset_scale()),
        "AddObjectsToBlock" => Box::new(add_objects_to_block()),
        "CreateUniqueBlock" => Box::new(create_unique_block()),
        "ReplaceBlock" => Box::new(replace_block()),
        "RescueBlockOrphans" => Box::new(rescue_block_orphans()),
        "Rescue3dmFile" => Box::new(rescue_3dm()),
        "GumballAlignment" => Box::new(gumball_op("alignment")),
        "GumballAutoReset" => Box::new(gumball_op("auto_reset")),
        "GumballDynamicRelocate" => Box::new(gumball_op("relocate")),
        "GumballRelocate" => Box::new(gumball_op("relocate")),
        "GumballReset" => Box::new(gumball_op("reset")),
        "GumballScaleMode" => Box::new(gumball_op("scale_mode")),
        "GumballSettings" => Box::new(gumball_settings()),
        "Camera" => Box::new(set_camera()),
        "DollyZoom" => Box::new(dolly_zoom()),
        "RotateCamera" => Box::new(rotate_camera()),
        "TiltView" => Box::new(tilt_view()),
        "WalkAbout" => Box::new(OptCmd("walk", "Toggle")),
        "Target" => Box::new(set_target()),
        "MoveTargetToObjects" => Box::new(ReqCmd(AppRequest::LookAtTarget)),
        "SetViewFocusDistance" => Box::new(focus_distance()),
        "OrientCameraToSrf" => Box::new(orient_camera_to_srf()),
        "NewFloatingViewport" => Box::new(ViewportOpCmd("float")),
        "ToggleFloatingViewport" => Box::new(ViewportOpCmd("float")),
        "NewViewport" => Box::new(ViewportOpCmd("new")),
        "CloseViewport" => Box::new(ViewportOpCmd("close")),
        "SplitViewportHorizontal" => Box::new(ViewportOpCmd("split_h")),
        "SplitViewportVertical" => Box::new(ViewportOpCmd("split_v")),
        "BringViewportToTop" => Box::new(ViewportOpCmd("to_top")),
        "NextViewportToTop" => Box::new(ViewportOpCmd("to_top")),
        "PushViewportToBack" => Box::new(ViewportOpCmd("to_back")),
        "SwapView" => Box::new(ViewportOpCmd("swap")),
        "ReadViewportsFromFile" => Box::new(read_viewports()),
        "GradientView" => Box::new(GradientViewCmd),
        "Lights" => Box::new(RequestCmd(AppRequest::ShowPanel("Lights".into()))),
        "PointLight" => Box::new(light_cmd("Point")),
        "Spotlight" => Box::new(light_cmd("Spot")),
        "DirectionalLight" => Box::new(light_cmd("Directional")),
        "LinearLight" => Box::new(light_cmd("Linear")),
        "RectangularLight" => Box::new(light_cmd("Rectangular")),
        "SelLight" => Box::new(SelByCmd { label: "SelLight", pred: |_, o| has_user_text(o, "Light") }),
        "EditLightByHighlight" => Box::new(light_by_highlight()),
        "EditLightByLooking" => Box::new(light_by_looking()),
        "SetSpotlightToView" => Box::new(light_by_looking()),
        "SetViewToSpotlight" => Box::new(view_to_spotlight()),
        "Materials" => Box::new(RequestCmd(AppRequest::ShowPanel("Materials".into()))),
        "Textures" => Box::new(RequestCmd(AppRequest::ShowPanel("Materials".into()))),
        "Environments" => Box::new(RequestCmd(AppRequest::ShowPanel("Rendering".into()))),
        "Libraries" => Box::new(libraries()),
        "ContentFilter" => Box::new(content_filter()),
        "RenderAssignMaterialToLayer" => Box::new(assign_material("layer")),
        "RenderAssignMaterialToLayersOfObjects" => Box::new(assign_material("layers_of_objects")),
        "RenderAssignMaterialToObjects" => Box::new(assign_material("objects")),
        "RenderMergeIdenticalMaterials" => Box::new(merge_identical_materials()),
        "SelMaterialName" => Box::new(sel_material_name()),
        "RenderReportImageFiles" => Box::new(FnCmd(|ctx| { let n = ctx.doc.settings.materials.iter().filter(|m| m.texture.is_some()).count(); ctx.log(format!("{n} materials reference image files.")); Step::Done })),
        "RenderReportMissingImageFiles" => Box::new(FnCmd(|ctx| { let missing: Vec<String> = ctx.doc.settings.materials.iter().filter_map(|m| m.texture.clone()).filter(|t| !std::path::Path::new(t).exists()).collect(); ctx.log(if missing.is_empty() { "No missing image files.".to_string() } else { format!("Missing: {}", missing.join(", ")) }); Step::Done })),
        "RenderOpenRenderImage" => Box::new(open_url_cmd("Image file to open", None)),
        "RenderOpenLastRendering" => Box::new(FnCmd(|ctx| { match ctx.memory.get("Render.Last").cloned() { Some(p) => { let _ = super::simple::open_external(&p); } None => ctx.log("No rendering has been made yet (Render)."), } Step::Done })),
        "CloseRenderWindow" => Box::new(FnCmd(|ctx| { ctx.log("Renderings open in the system image viewer; close it from there."); Step::Done })),
        "CopyRenderWindowToClipboard" => Box::new(capture_cmd("CopyRenderWindowToClipboard", true, false)),
        "SaveRenderWindowAs" => Box::new(capture_cmd("SaveRenderWindowAs", false, false)),
        "RenderBlowup" => Box::new(capture_cmd("RenderBlowup (captures the active viewport)", false, false)),
        "RenderInWindow" => Box::new(capture_cmd("RenderInWindow (captures the active viewport)", false, false)),
        "RenderPreview" => Box::new(ReqCmd(AppRequest::SetDisplayMode("Rendered".into()))),
        "RenderPreviewInWindow" => Box::new(ReqCmd(AppRequest::SetDisplayMode("Rendered".into()))),
        "RenderPreviewWindow" => Box::new(ReqCmd(AppRequest::SetDisplayMode("Rendered".into()))),
        "RenderWindow" => Box::new(render_cmd()),
        "Rendering" => Box::new(RequestCmd(AppRequest::ShowPanel("Rendering".into()))),
        "SetOneDaySunAnimation" => Box::new(anim_setup("sun_day")),
        "SetSeasonalSunAnimation" => Box::new(anim_setup("sun_season")),
        "SetFlythroughAnimation" => Box::new(anim_setup("fly")),
        "SetPathAnimation" => Box::new(anim_setup("path")),
        "ExportBitmaps" => Box::new(export_bitmaps()),
        "PlayAnimation" => Box::new(AnimCtl("play")),
        "ViewFirstFrame" => Box::new(AnimCtl("first")),
        "ViewFrameNumber" => Box::new(AnimCtl("frame")),
        "ViewLastFrame" => Box::new(AnimCtl("last")),
        "ViewNextFrame" => Box::new(AnimCtl("next")),
        "ViewPreviousFrame" => Box::new(AnimCtl("prev")),
        "RecordAnimation" => Box::new(AnimCtl("record")),
        "ThicknessAnalysis" => Box::new(thickness_analysis(false)),
        "ThicknessAnalysisOff" => Box::new(thickness_analysis(true)),
        "Zebra" => Box::new(RequestCmd(AppRequest::SetDisplayMode("Zebra".into()))),
        "ZebraOff" => Box::new(RequestCmd(AppRequest::SetDisplayMode("Shaded".into()))),
        "EMap" => Box::new(RequestCmd(AppRequest::SetDisplayMode("EMap".into()))),
        "EMapOff" => Box::new(RequestCmd(AppRequest::SetDisplayMode("Shaded".into()))),
        "CurvatureAnalysis" => Box::new(RequestCmd(AppRequest::SetDisplayMode("Curvature".into()))),
        "CurvatureAnalysisOff" => Box::new(RequestCmd(AppRequest::SetDisplayMode("Shaded".into()))),
        "DraftAngleAnalysis" => Box::new(RequestCmd(AppRequest::SetDisplayMode("DraftAngle".into()))),
        "DraftAngleAnalysisOff" => Box::new(RequestCmd(AppRequest::SetDisplayMode("Shaded".into()))),
        "ShowEdges" => Box::new(OptCmd("show_edges", "On")),
        "ShowEdgesOff" => Box::new(OptCmd("show_edges", "Off")),
        "ShowEnds" => Box::new(OptCmd("show_ends", "On")),
        "ShowEndsOff" => Box::new(OptCmd("show_ends", "Off")),
        "ShowDir" => Box::new(OptCmd("show_dir", "On")),
        "ShowDirOff" => Box::new(OptCmd("show_dir", "Off")),
        "UndoSelected" => Box::new(undo_selected()),
        "ScalePositions" => Box::new(scale_positions()),
        "SelDimAngular" => Box::new(SelByCmd { label: "SelDimAngular", pred: |_, o| group_prefix(o, "DimAngle") }),
        "SelDimLinear" => Box::new(SelByCmd { label: "SelDimLinear", pred: |_, o| group_prefix(o, "Dim") && !group_prefix(o, "DimAngle") && !group_prefix(o, "DimRadius") && !group_prefix(o, "DimDiameter") && !group_prefix(o, "DimOrdinate") }),
        "SelDimRadial" => Box::new(SelByCmd { label: "SelDimRadial", pred: |_, o| group_prefix(o, "DimRadius") || group_prefix(o, "DimDiameter") }),
        "SelDimOrdinate" => Box::new(SelByCmd { label: "SelDimOrdinate", pred: |_, o| group_prefix(o, "DimOrdinate") }),
        "SelDimCentermark" => Box::new(SelByCmd { label: "SelDimCentermark", pred: |_, o| group_prefix(o, "Centermark") }),
        "SelDot" => Box::new(SelByCmd { label: "SelDot", pred: |_, o| group_prefix(o, "Dot") }),
        "SelPtCloud" => Box::new(SelByCmd { label: "SelPtCloud", pred: |_, o| group_prefix(o, "PointCloud") }),
        "SelNonManifold" => Box::new(SelByCmd { label: "SelNonManifold", pred: |_, o| matches!(&o.geometry, Geometry::Mesh(m) if is_nonmanifold(m)) }),
        "SelSelfIntersectingCrv" => Box::new(SelByCmd { label: "SelSelfIntersectingCrv", pred: |_, o| matches!(&o.geometry, Geometry::Curve(c) if curve_self_intersects(c)) }),
        "SelRenderColor" => Box::new(SelByCmd { label: "SelRenderColor", pred: |d, o| d.selected_ids().iter().filter_map(|id| d.get(*id)).any(|s| d.display_color(s) == d.display_color(o)) }),
        "SelClippingPlaneInViewport" => Box::new(ClipStateCmd("SelClippingPlane")),
        "SelSubD" => Box::new(sel_subd("all")),
        "SelOpenSubD" => Box::new(sel_subd("open")),
        "SelClosedSubD" => Box::new(sel_subd("closed")),
        "SelKey" => Box::new(sel_key(false)),
        "SelKeyValue" => Box::new(sel_key(true)),
        "SelValue" => Box::new(sel_value()),
        "SelLayerNumber" => Box::new(sel_layer_number()),
        "Select" => Box::new(select_by_name()),
        "SelVolumeObject" => Box::new(sel_volume_object()),
        "SelVolumePipe" => Box::new(sel_volume_pipe()),
        "SelCircular" => Box::new(sel_circular()),
        "SelRectangular" => Box::new(sel_rectangular()),
        "SelFence" => Box::new(SelPolygonCmd::new("SelFence", true)),
        "Lasso" => Box::new(SelPolygonCmd::new("Lasso", false)),
        "SelBrush" => Box::new(SelPolygonCmd::new("SelBrush", true)),
        "SelBrushPoints" => Box::new(SelPolygonCmd::new("SelBrushPoints", true)),
        "SelMeshPart" => Box::new(sel_mesh_part()),
        "SelConnectedMeshFaces" => Box::new(sel_mesh_part()),
        "SelMeshFacesByArea" => Box::new(sel_mesh_faces_by("Area")),
        "SelMeshFacesByAspectRatio" => Box::new(sel_mesh_faces_by("AspectRatio")),
        "SelMeshFacesByDraftAngle" => Box::new(sel_mesh_faces_by("DraftAngle")),
        "SelMeshFacesByEdgeLength" => Box::new(sel_mesh_faces_by("EdgeLength")),
        "SelectionFilter" => Box::new(SelectionFilterCmd),
        "SelectionFilterNone" => Box::new(ReqCmd(AppRequest::SelectionFilter(None))),
        "SelectionFilterEnable" => Box::new(ReqCmd(AppRequest::SelectionFilter(None))),
        "IntersectSelf" => Box::new(intersect_self()),
        "IntersectTwoSets" => Box::new(intersect_two_sets()),
        "RunPythonScript" => Box::new(run_python()),
        "EditPythonScript" => Box::new(edit_file_cmd()),
        "EditScript" => Box::new(edit_file_cmd()),
        "ScriptEditor" => Box::new(edit_file_cmd()),
        "MacroEditor" => Box::new(edit_file_cmd()),
        "OpenURL" => Box::new(open_url_cmd("Web address", None)),
        "WebBrowser" => Box::new(open_url_cmd("Web address", Some("https://www.triceratops.com"))),
        "FileExplorer" => Box::new(open_url_cmd("Folder", Some("."))),
        "Run" => Box::new(run_program()),
        "Echo" => Box::new(FlagCmd("Echo", "Yes", "Command echo on.")),
        "NoEcho" => Box::new(FlagCmd("Echo", "No", "Command echo off.")),
        "SetRedrawOn" => Box::new(FlagCmd("Redraw", "Yes", "Redraw enabled.")),
        "SetRedrawOff" => Box::new(FlagCmd("Redraw", "No", "Redraw suspended until SetRedrawOn (display still updates in triceratops).")),
        "Repeat" => Box::new(ReqCmd(AppRequest::RepeatLast)),
        "Minimize" => Box::new(ReqCmd(AppRequest::WindowOp("minimize".into()))),
        "Fullscreen" => Box::new(ReqCmd(AppRequest::WindowOp("fullscreen".into()))),
        "Autosave" => Box::new(ReqCmd(AppRequest::Autosave)),
        "OptionsExport" => Box::new(ReqCmd(AppRequest::OptionsFile { export: true })),
        "OptionsImport" => Box::new(ReqCmd(AppRequest::OptionsFile { export: false })),
        "OptionsPage" => Box::new(RequestCmd(AppRequest::Options("View".into()))),
        "DocumentPropertiesPage" => Box::new(RequestCmd(AppRequest::DocumentProperties("Units".into()))),
        "ExportLinkedBlocks" => Box::new(ReqCmd(AppRequest::ExportBlocks)),
        "SaveAsTemplate" => Box::new(ReqCmd(AppRequest::SaveTemplate)),
        "ExportWithOrigin" => Box::new(export_with_origin()),
        "SetWorkingFolder" => Box::new(set_working_folder()),
        "OrthoAngle" => Box::new(ortho_angle()),
        "SnapSize" => Box::new(snap_size()),
        "PerspectiveAngle" => Box::new(perspective_angle()),
        "ZoomLens" => Box::new(zoom_lens()),
        "SetOrtho" => Box::new(set_mode_cmd("ortho")),
        "SetPlanar" => Box::new(set_mode_cmd("planar")),
        "SetSnap" => Box::new(set_mode_cmd("snap")),
        "SetActiveViewport" => Box::new(set_active_viewport()),
        "LockViewport" => Box::new(ReqCmd(AppRequest::SetOption { key: "lock_viewport".into(), value: String::new() })),
        "MatchProperties" => Box::new(match_properties()),
        "FindText" => Box::new(find_text()),
        "ScaleTextHeight" => Box::new(scale_text_height()),
        "DimArea" => Box::new(dim_area_volume(false)),
        "DimVolume" => Box::new(dim_area_volume(true)),
        "Plan" => Box::new(RequestCmd(AppRequest::PlanView)),
        "NextOrthoViewport" => Box::new(RequestCmd(AppRequest::NextViewport(true))),
        "NextPerspectiveViewport" => Box::new(RequestCmd(AppRequest::NextViewport(true))),
        "ToggleLeftSidebar" => Box::new(RequestCmd(AppRequest::ToggleUi("left_sidebar".into()))),
        "SelControlPoint" => Box::new(ReqCmd(AppRequest::GripOp("SelControlPoint".into()))),
        "SelControlPointRegion" => Box::new(ReqCmd(AppRequest::GripOp("SelControlPointRegion".into()))),
        "InvertPt" => Box::new(ReqCmd(AppRequest::GripOp("InvertPt".into()))),
        "SelU" => Box::new(ReqCmd(AppRequest::GripOp("SelU".into()))),
        "SelV" => Box::new(ReqCmd(AppRequest::GripOp("SelV".into()))),
        "SelUV" => Box::new(ReqCmd(AppRequest::GripOp("SelUV".into()))),
        "NextU" => Box::new(ReqCmd(AppRequest::GripOp("NextU".into()))),
        "PrevU" => Box::new(ReqCmd(AppRequest::GripOp("PrevU".into()))),
        "NextV" => Box::new(ReqCmd(AppRequest::GripOp("NextV".into()))),
        "PrevV" => Box::new(ReqCmd(AppRequest::GripOp("PrevV".into()))),
        "AddNextU" => Box::new(ReqCmd(AppRequest::GripOp("AddNextU".into()))),
        "AddPrevU" => Box::new(ReqCmd(AppRequest::GripOp("AddPrevU".into()))),
        "AddNextV" => Box::new(ReqCmd(AppRequest::GripOp("AddNextV".into()))),
        "AddPrevV" => Box::new(ReqCmd(AppRequest::GripOp("AddPrevV".into()))),
        "SelConnected" => Box::new(ReqCmd(AppRequest::GripOp("SelConnected".into()))),
        "HidePt" => Box::new(ReqCmd(AppRequest::GripOp("HidePt".into()))),
        "ShowPt" => Box::new(ReqCmd(AppRequest::GripOp("ShowPt".into()))),
        "EditPtOn" => Box::new(ReqCmd(AppRequest::GripOp("EditPtOn".into()))),
        "FillMeshHole" => Box::new(fill_holes_cmd(false)),
        "FillMeshHoles" => Box::new(fill_holes_cmd(true)),
        "DupMeshEdge" => Box::new(dup_mesh_edges()),
        "DupMeshHoleBoundary" => Box::new(dup_mesh_edges()),
        "MeshIntersect" => Box::new(mesh_intersect(false)),
        "MeshSelfIntersect" => Box::new(mesh_intersect(true)),
        "Silhouette" => Box::new(silhouette_cmd(false)),
        "MeshOutline" => Box::new(silhouette_cmd(true)),
        "MeshFromPoints" => Box::new(mesh_from_points()),
        "MeshPolyline" => Box::new(mesh_polyline()),
        "ExtractMeshFaces" => Box::new(extract_mesh_faces_pick()),
        "ExtractMeshFacesByArea" => Box::new(extract_mesh_faces_by("Area")),
        "ExtractMeshFacesByAspectRatio" => Box::new(extract_mesh_faces_by("AspectRatio")),
        "ExtractMeshFacesByEdgeLength" => Box::new(extract_mesh_faces_by("EdgeLength")),
        "ExtractMeshFacesByDraftAngle" => Box::new(extract_mesh_faces_by("DraftAngle")),
        "ExtractDuplicateMeshFaces" => Box::new(extract_duplicate_faces()),
        "ExtractNonManifoldMeshEdges" => Box::new(extract_nonmanifold()),
        "CullDegenerateMeshFaces" => Box::new(mesh_map("CullDegenerateMeshFaces", "Select meshes", |m| weld(m, 1e-9 * (1.0 + m.bounding_box().diagonal())))),
        "ReduceMesh" => Box::new(reduce_mesh()),
        "AlignVertices" => Box::new(align_vertices()),
        "Stitch" => Box::new(stitch()),
        "SubDivide" => Box::new(subdivide_cmd("SubDivide", true)),
        "ToSubD" => Box::new(subdivide_cmd("ToSubD", false)),
        "ToNURBS" => Box::new(mesh_to_nurb_or_subd("ToNURBS")),
        "SubDBox" => Box::new(SubDPrimCmd::new("MeshBox")),
        "SubDSphere" => Box::new(SubDPrimCmd::new("MeshSphere")),
        "SubDCylinder" => Box::new(SubDPrimCmd::new("MeshCylinder")),
        "SubDCone" => Box::new(SubDPrimCmd::new("MeshCone")),
        "SubDTorus" => Box::new(SubDPrimCmd::new("MeshTorus")),
        "SubDPlane" => Box::new(SubDPrimCmd::new("MeshPlane")),
        "SubDEllipsoid" => Box::new(mesh_ellipsoid()),
        "SubDTruncatedCone" => Box::new(mesh_tcone()),
        "MeshEllipsoid" => Box::new(mesh_ellipsoid()),
        "MeshTruncatedCone" => Box::new(mesh_tcone()),
        "SubDLoft" => Box::new(LoftCmd::default()),
        "SubDSweep1" => Box::new(Sweep1Cmd::new(false)),
        "SubDSweep2" => Box::new(Sweep2Cmd::default()),
        "ShrinkWrap" => Box::new(shrink_wrap()),
        "Crease" => Box::new(crease_cmd(false)),
        "RemoveCrease" => Box::new(crease_cmd(true)),
        "CollapseMeshFacesByArea" => Box::new(collapse_by("Area")),
        "CollapseMeshFacesByAspectRatio" => Box::new(collapse_by("AspectRatio")),
        "CollapseMeshFacesByEdgeLength" => Box::new(collapse_by("EdgeLength")),
        "CollapseMeshEdge" => Box::new(mesh_pick("CollapseMeshEdge", "Select mesh edge to collapse", |m, p| tri_geom::mesh_ops::collapse_nearest_edge(m, p))),
        "CollapseMeshFace" => Box::new(mesh_pick("CollapseMeshFace", "Select mesh face to collapse", |m, p| tri_geom::mesh_ops::collapse_nearest_edge(&tri_geom::mesh_ops::collapse_nearest_edge(m, p), p))),
        "CollapseMeshVertex" => Box::new(mesh_pick("CollapseMeshVertex", "Select mesh vertex to collapse", |m, p| tri_geom::mesh_ops::collapse_nearest_vertex(m, p))),
        "SwapMeshEdge" => Box::new(mesh_pick("SwapMeshEdge", "Select mesh edge to swap", |m, p| tri_geom::mesh_ops::swap_nearest_edge(m, p))),
        "SplitMeshEdge" => Box::new(mesh_pick("SplitMeshEdge", "Select mesh edge to split", |m, p| tri_geom::mesh_ops::split_nearest_edge(m, p))),
        "InsertPoint" => Box::new(mesh_pick("InsertPoint", "Select mesh edge to insert a point on", |m, p| tri_geom::mesh_ops::split_nearest_edge(m, p))),
        "PatchSingleFace" => Box::new(fill_holes_cmd(false)),
        "MatchMeshEdge" => Box::new(align_vertices()),
        "WeldEdge" => Box::new(mesh_map("WeldEdge", "Select meshes", |m| weld(m, 1e-6 * (1.0 + m.bounding_box().diagonal())))),
        "WeldVertices" => Box::new(mesh_map("WeldVertices", "Select meshes", |m| weld(m, 1e-6 * (1.0 + m.bounding_box().diagonal())))),
        "UnweldEdge" => Box::new(mesh_map("UnweldEdge", "Select meshes", |m| unweld(m))),
        "UnweldVertex" => Box::new(mesh_map("UnweldVertex", "Select meshes", |m| unweld(m))),
        "RebuildMesh" => Box::new(mesh_map("RebuildMesh", "Select meshes", |m| weld(m, 1e-9 * (1.0 + m.bounding_box().diagonal())))),
        "ExtractMeshEdges" => Box::new(extract_mesh_faces_by("DraftAngle")),
        "SplitMeshWithCurve" => Box::new(MeshSplitCmd::new(false)),
        "PolylineOnMesh" => Box::new(PullCmd::default()),
        "ExtractRenderMesh" => Box::new(MeshCmd),
        "ExtractAnalysisMesh" => Box::new(MeshCmd),
        "MeshPatch" => Box::new(mesh_from_points()),
        "MeshFromLines" => Box::new(mesh_from_points()),
        "Hyperlink" => Box::new(hyperlink()),
        "Tutorials" => Box::new(open_url_cmd("", Some("https://www.triceratops.com/learn/"))),
        "Audit3dmFile" => Box::new(ReqCmd(AppRequest::Audit3dmFile)),
        "RenderArctic" => Box::new(RequestCmd(AppRequest::SetDisplayMode("Arctic".into()))),
        "Shade" => Box::new(RequestCmd(AppRequest::SetDisplayMode("Shaded".into()))),
        "ShadeSelected" => Box::new(RequestCmd(AppRequest::SetDisplayMode("Shaded".into()))),
        "FlatShade" => Box::new(RequestCmd(AppRequest::SetDisplayMode("Shaded".into()))),
        "ClippingPlane" => Box::new(ClippingPlaneCmd::default()),
        "EnableClippingPlane" => Box::new(ClipStateCmd("EnableClippingPlane")),
        "DisableClippingPlane" => Box::new(ClipStateCmd("DisableClippingPlane")),
        "SelClippingPlane" => Box::new(ClipStateCmd("SelClippingPlane")),
        "NamedPosition" => Box::new(NamedPositionCmd(false)),
        "Snapshots" => Box::new(NamedPositionCmd(true)),
        "SetUserText" => Box::new(UserTextCmd("SetUserText")),
        "GetUserText" => Box::new(UserTextCmd("GetUserText")),
        "SetDocumentUserText" => Box::new(UserTextCmd("SetDocumentUserText")),
        "GetDocumentUserText" => Box::new(UserTextCmd("GetDocumentUserText")),
        "DocumentText" => Box::new(UserTextCmd("DocumentText")),
        "ReadCommandFile" => Box::new(ReadCommandFileCmd),
        "DeleteFaces" => Box::new(DeleteFacesCmd),
        "MoveFace" => Box::new(MoveFaceCmd::new(false)),
        "MoveEdge" => Box::new(MoveFaceCmd::new(true)),
        "ExtrudeSrfToPoint" => Box::new(ExtrudeSrfVariantCmd::new(false)),
        "ExtrudeSrfTapered" => Box::new(ExtrudeSrfVariantCmd::new(true)),
        "OffsetMultiple" => Box::new(OffsetMultipleCmd::default()),
        "Fin" => Box::new(FinCmd::default()),
        "OffsetNormal" => Box::new(FinCmd::default()),
        "InterpCrvOnSrf" => Box::new(InterpCrvOnSrfCmd::default()),
        "CrvDeviation" => Box::new(CrvDeviationCmd::default()),
        "Diameter" => Box::new(DiameterCmd),
        "AreaMoments" => Box::new(MomentsCmd(false)),
        "VolumeMoments" => Box::new(MomentsCmd(true)),
        "MakeUniformUV" => Box::new(MakeUniformUVCmd),
        "ExtractMeshPart" => Box::new(ExtractMeshPartCmd),
        "ExtractConnectedMeshFaces" => Box::new(ExtractMeshPartCmd),
        "Maelstrom" => Box::new(MaelstromWrap(MaelstromCmd::default())),
        "Distribute" => Box::new(DistributeCmd),
        "ChangeToCurrentLayer" => Box::new(LayerUtilCmd("ChangeToCurrentLayer")),
        "MatchLayer" => Box::new(LayerUtilCmd("MatchLayer")),
        "DupLayer" => Box::new(LayerUtilCmd("DupLayer")),
        "OneLayerOff" => Box::new(LayerUtilCmd("OneLayerOff")),
        "MergeAllCoplanarFaces" => Box::new(MergeCoplanarCmd),
        "MergeCoplanarFace" => Box::new(MergeCoplanarCmd),
        "MergeAllFaces" => Box::new(MergeCoplanarCmd),
        "FilletSrf" => Box::new(FilletSrfCmd::default()),
        "ConvertExtrusion" => Box::new(NoopCmd("Extrusions are already polysurfaces in triceratops.")),
        "UseExtrusions" => Box::new(NoopCmd("triceratops always creates polysurfaces instead of lightweight extrusions.")),
        "SaveACopy" => Box::new(RequestCmd(AppRequest::SaveCopy { incremental: false })),
        "IncrementalSave" => Box::new(RequestCmd(AppRequest::SaveCopy { incremental: true })),
        "SaveSmall" => Box::new(RequestCmd(AppRequest::FileSave)),
        "Revert" => Box::new(RequestCmd(AppRequest::Revert)),
        "ToggleRightSidebar" => Box::new(RequestCmd(AppRequest::ToggleUi("right_sidebar".into()))),
        "ViewportTabs" => Box::new(RequestCmd(AppRequest::ToggleUi("viewport_tabs".into()))),
        "CommandHelp" => Box::new(RequestCmd(AppRequest::ShowPanel("Help".into()))),
        "PropertiesPage" => Box::new(RequestCmd(AppRequest::ShowPanel("Properties".into()))),
        "DisplayProperties" => Box::new(RequestCmd(AppRequest::ShowPanel("Display".into()))),
        "UnrollSrfUV" => Box::new(UnrollCmd("UnrollSrf")),
        "Smash" => Box::new(UnrollCmd("Smash")),
        "MeshSplit" => Box::new(MeshSplitCmd::new(false)),
        "MeshTrim" => Box::new(MeshSplitCmd::new(true)),
        "DimOrdinate" => Box::new(DimOrdinateCmd::default()),
        "DimRotated" => Box::new(DimRotatedCmd::default()),
        "CurvatureGraph" => Box::new(CurvatureGraphCmd(true)),
        "ExtractCurvatureGraph" => Box::new(CurvatureGraphCmd(true)),
        "CurvatureGraphOff" => Box::new(CurvatureGraphCmd(false)),
        "Symmetry" => Box::new(SymmetryCmd::default()),
        "Isometric" => Box::new(RequestCmd(AppRequest::SetView("Isometric".into()))),
        "PlanarDifference" => Box::new(PlanarBoolCmd::new(tri_geom::BooleanOp::Difference, false)),
        "PlanarIntersection" => Box::new(PlanarBoolCmd::new(tri_geom::BooleanOp::Intersection, false)),
        "CurveBoolean" => Box::new(PlanarBoolCmd::new(tri_geom::BooleanOp::Union, true)),
        "RoundHole" => Box::new(RoundHoleCmd::default()),
        "MakeHole" => Box::new(MakeHoleCmd::default()),
        "Shell" => Box::new(ShellCmd::default()),
        "Bend" => Box::new(DeformCmd::new("Bend")),
        "Flow" => Box::new(DeformCmd::new("Flow")),
        "Stretch" => Box::new(StretchCmd(DeformCmd::new("Stretch"))),
        "Dot" => Box::new(DotCmd::default()),
        "NamedCPlane" => Box::new(NamedCPlaneCmd),
        "SelChain" => Box::new(SelChainCmd),
        "InsertControlPoint" => Box::new(ControlPointCmd("Insert")),
        "RemoveControlPoint" => Box::new(ControlPointCmd("Remove")),
        "Weight" => Box::new(ControlPointCmd("Weight")),
        "Conic" => Box::new(ConicCmd::default()),
        "Fair" => Box::new(SmoothCmd::default()),
        "ExtractSubCrv" => Box::new(CurveUtilCmd::new(CurveUtil::ExtractSubCrv)),
        "DeleteSubCrv" => Box::new(CurveUtilCmd::new(CurveUtil::DeleteSubCrv)),
        "ReducePolyline" => Box::new(CurveUtilCmd::new(CurveUtil::ReducePolyline)),
        "ConvertToBeziers" => Box::new(CurveUtilCmd::new(CurveUtil::ConvertToBeziers)),
        "MakeUniform" => Box::new(CurveUtilCmd::new(CurveUtil::MakeUniform)),
        "MakePeriodic" => Box::new(CurveUtilCmd::new(CurveUtil::MakePeriodic)),
        "MakeNonPeriodic" => Box::new(CurveUtilCmd::new(CurveUtil::MakeNonPeriodic)),
        "Reparameterize" => Box::new(CurveUtilCmd::new(CurveUtil::Reparameterize)),
        "Domain" => Box::new(CurveUtilCmd::new(CurveUtil::Domain)),
        "CrvStart" => Box::new(CurveUtilCmd::new(CurveUtil::CrvStart)),
        "CrvEnd" => Box::new(CurveUtilCmd::new(CurveUtil::CrvEnd)),
        "InsertKink" => Box::new(CurveUtilCmd::new(CurveUtil::InsertKink)),
        "RemoveKnot" => Box::new(CurveUtilCmd::new(CurveUtil::RemoveKnot)),
        "RemoveMultiKnot" => Box::new(CurveUtilCmd::new(CurveUtil::RemoveKnot)),
        "ClosestPt" => Box::new(CurveUtilCmd::new(CurveUtil::ClosestPt)),
        "Curvature" => Box::new(CurveUtilCmd::new(CurveUtil::Curvature)),
        "TweenCurves" => Box::new(CurveUtilCmd::new(CurveUtil::TweenCurves)),
        "LineThroughPt" => Box::new(CurveUtilCmd::new(CurveUtil::LineThroughPt)),
        "PlaneThroughPt" => Box::new(CurveUtilCmd::new(CurveUtil::PlaneThroughPt)),
        "ExtractControlPolygon" => Box::new(CurveUtilCmd::new(CurveUtil::ExtractControlPolygon)),
        "MatchCrvDir" => Box::new(CurveUtilCmd::new(CurveUtil::MatchCrvDir)),
        "MergeCrv" => Box::new(JoinCmd),
        "PointGrid" => Box::new(GridMakeCmd::new(GridKind::PointGrid)),
        "SrfPtGrid" => Box::new(GridMakeCmd::new(GridKind::SrfPtGrid)),
        "SrfControlPtGrid" => Box::new(GridMakeCmd::new(GridKind::SrfControlPtGrid)),
        "ExtractIsocurve" => Box::new(GridMakeCmd::new(GridKind::ExtractIsocurve)),
        "ExtractWireframe" => Box::new(GridMakeCmd::new(GridKind::ExtractWireframe)),
        "UntrimAll" => Box::new(SrfUtilCmd::new(SrfUtil::UntrimAll)),
        "UntrimBorder" => Box::new(SrfUtilCmd::new(SrfUtil::UntrimBorder)),
        "UntrimHoles" => Box::new(SrfUtilCmd::new(SrfUtil::UntrimHoles)),
        "ShrinkTrimmedSrf" => Box::new(SrfUtilCmd::new(SrfUtil::ShrinkTrimmedSrf)),
        "ShrinkTrimmedSrfToEdge" => Box::new(SrfUtilCmd::new(SrfUtil::ShrinkTrimmedSrf)),
        "TriangulateMesh" => Box::new(SrfUtilCmd::new(SrfUtil::TriangulateMesh)),
        "UnifyMeshNormals" => Box::new(SrfUtilCmd::new(SrfUtil::UnifyMeshNormals)),
        "RebuildMeshNormals" => Box::new(SrfUtilCmd::new(SrfUtil::UnifyMeshNormals)),
        "OffsetMesh" => Box::new(SrfUtilCmd::new(SrfUtil::OffsetMesh)),
        "ExtrudeMesh" => Box::new(SrfUtilCmd::new(SrfUtil::ExtrudeMesh)),
        "PlanarMesh" => Box::new(SrfUtilCmd::new(SrfUtil::PlanarMesh)),
        "3DFace" => Box::new(SrfUtilCmd::new(SrfUtil::MeshFromPoints3DFace)),
        "ScaleNU" => Box::new(ScaleNUCmd::default()),
        "Twist" => Box::new(TwistTaperCmd::new(true)),
        "Taper" => Box::new(TwistTaperCmd::new(false)),
        "Ribbon" => Box::new(RibbonCmd::default()),
        "DimRadius" => Box::new(Dim2Cmd::new(DimKind::Radius)),
        "DimDiameter" => Box::new(Dim2Cmd::new(DimKind::Diameter)),
        "DimAngle" => Box::new(Dim2Cmd::new(DimKind::Angle)),
        "Centermark" => Box::new(Dim2Cmd::new(DimKind::Centermark)),
        "DimCurveLength" => Box::new(Dim2Cmd::new(DimKind::CurveLength)),
        "Calc" => Box::new(CalcCmd),
        "UndoMultiple" => Box::new(UndoMultipleCmd(false)),
        "RedoMultiple" => Box::new(UndoMultipleCmd(true)),
        "ClearUndo" => Box::new(ReportCmd("ClearUndo")),
        "Commands" => Box::new(ReportCmd("Commands")),
        "PolygonCount" => Box::new(ReportCmd("PolygonCount")),
        "SystemInfo" => Box::new(ReportCmd("SystemInfo")),
        "Audit" => Box::new(ReportCmd("Audit")),
        "Check" => Box::new(ReportCmd("Check")),
        "SelBadObjects" => Box::new(ReportCmd("SelBadObjects")),
        "Make2D" => Box::new(Make2DCmd),
        "CutPlane" => Box::new(CutPlaneCmd::default()),
        "Parabola" => Box::new(ParabolaCmd::default()),
        "UngroupAll" => Box::new(SelStateCmd("UngroupAll")),
        "UnlockSelected" => Box::new(SelStateCmd("UnlockSelected")),
        "LockSwap" => Box::new(SelStateCmd("LockSwap")),
        "IsolateLock" => Box::new(SelStateCmd("IsolateLock")),
        "UnisolateLock" => Box::new(SelStateCmd("UnisolateLock")),
        "PtOffSelected" => Box::new(SelStateCmd("PtOffSelected")),
        "SolidPtOn" => Box::new(SelStateCmd("SolidPtOn")),
        "SelVolumeSphere" => Box::new(SelVolumeSphereCmd::default()),
        "SelClosedMesh" => Box::new(SelByCmd { label: "SelClosedMesh", pred: |_, o| matches!(&o.geometry, Geometry::Mesh(m) if m.is_closed()) }),
        "SelOpenMesh" => Box::new(SelByCmd { label: "SelOpenMesh", pred: |_, o| matches!(&o.geometry, Geometry::Mesh(m) if !m.is_closed()) }),
        "SelPlanarCrv" => Box::new(SelByCmd { label: "SelPlanarCrv", pred: |_, o| matches!(&o.geometry, Geometry::Curve(c) if is_planar_curve(c)) }),
        "SelPlanarSrf" => Box::new(SelByCmd { label: "SelPlanarSrf", pred: |_, o| match &o.geometry { Geometry::Surface(s) => s.degree_u == 1 && s.degree_v == 1 && s.count_u == 2 && s.count_v == 2, Geometry::Brep(b) => b.faces.len() == 1 && super::solid_edit::planar_loops(&b.faces[0]).is_some(), _ => false } }),
        "SelTrimmedSrf" => Box::new(SelByCmd { label: "SelTrimmedSrf", pred: |_, o| matches!(&o.geometry, Geometry::Brep(b) if b.faces.len() == 1 && b.faces[0].is_trimmed()) }),
        "SelUntrimmedSrf" => Box::new(SelByCmd { label: "SelUntrimmedSrf", pred: |_, o| matches!(&o.geometry, Geometry::Surface(_)) }),
        "SelBox" => Box::new(SelByCmd { label: "SelBox", pred: |_, o| matches!(&o.geometry, Geometry::Brep(b) if is_box(b)) }),
        "SelShortCrv" => Box::new(SelByCmd { label: "SelShortCrv", pred: |d, o| matches!(&o.geometry, Geometry::Curve(c) if c.length() < d.settings.absolute_tolerance * 10.0) }),
        "SelColor" => Box::new(SelByCmd { label: "SelColor", pred: |d, o| d.selected_ids().iter().filter_map(|id| d.get(*id)).any(|s| d.display_color(s) == d.display_color(o)) }),
        "SelText" => Box::new(SelByCmd { label: "SelText", pred: |_, o| o.attributes.groups.iter().any(|g| g.starts_with("Text")) }),
        "SelHatch" => Box::new(SelByCmd { label: "SelHatch", pred: |_, o| o.attributes.groups.iter().any(|g| g.starts_with("Hatch")) }),
        "SelDim" => Box::new(SelByCmd { label: "SelDim", pred: |_, o| o.attributes.groups.iter().any(|g| g.starts_with("Dim")) }),
        "SelLeader" => Box::new(SelByCmd { label: "SelLeader", pred: |_, o| o.attributes.groups.iter().any(|g| g.starts_with("Leader")) }),
        "SelDupAll" => Box::new(SelByCmd { label: "SelDupAll", pred: |d, o| d.objects().any(|p| p.id != o.id && p.geometry == o.geometry) }),
        "SelBlockInstanceNamed" => Box::new(SelBlockInstanceCmd),
        "SelBoundary" => Box::new(SelByCmd { label: "SelBoundary", pred: |_, o| matches!(&o.geometry, Geometry::Curve(c) if c.is_closed()) }),
        "BlendEdge" => Box::new(EdgeRoundCmd::new(false)),
        "ChamferEdge" => Box::new(EdgeRoundCmd::new(true)),
        "BlendSrf" => Box::new(BlendSrfCmd::default()),
        "Patch" => Box::new(PatchCmd::default()),
        "RotateView" => Box::new(ViewOpCmd::new("RotateView")),
        "Pan" => Box::new(ViewOpCmd::new("Pan")),
        "ZoomIn" => Box::new(RequestCmd(AppRequest::CameraOp { op: "Zoom".into(), dx: 1.25, dy: 0.0 })),
        "ZoomOut" => Box::new(RequestCmd(AppRequest::CameraOp { op: "Zoom".into(), dx: 0.8, dy: 0.0 })),
        "Maximize" => Box::new(RequestCmd(AppRequest::MaxViewport)),
        "Restore" => Box::new(RequestCmd(AppRequest::MaxViewport)),
        "SetMaximizedViewport" => Box::new(RequestCmd(AppRequest::MaxViewport)),
        "Insert" => Box::new(InsertCmd::default()),
        "ExplodeBlock" => Box::new(ExplodeBlockCmd),
        "BlockManager" => Box::new(BlockManagerCmd),
        "SelBlockInstance" => Box::new(SelBlockInstanceCmd),
        "TextObject" => Box::new(TextCmd::new(true)),
        "Dim" => Box::new(DimCmd::new(false)),
        "DimAligned" => Box::new(DimCmd::new(true)),
        "Leader" => Box::new(LeaderCmd::default()),
        "Hatch" => Box::new(HatchCmd::default()),
        "MeshBooleanUnion" => Box::new(MeshBooleanCmd::new(tri_geom::BooleanOp::Union, false)),
        "MeshBooleanDifference" => Box::new(MeshBooleanCmd::new(tri_geom::BooleanOp::Difference, false)),
        "MeshBooleanIntersection" => Box::new(MeshBooleanCmd::new(tri_geom::BooleanOp::Intersection, false)),
        "MeshBooleanSplit" => Box::new(MeshBooleanCmd::new(tri_geom::BooleanOp::Difference, true)),
        "Weld" => Box::new(MeshOpCmd(MeshOp::Weld)),
        "Unweld" => Box::new(MeshOpCmd(MeshOp::Unweld)),
        "SplitDisjointMesh" => Box::new(MeshOpCmd(MeshOp::SplitDisjoint)),
        "ExtractPt" => Box::new(MeshOpCmd(MeshOp::ExtractPt)),
        "MeshPlane" => Box::new(MeshPrimCmd::new("MeshPlane")),
        "MeshBox" => Box::new(MeshPrimCmd::new("MeshBox")),
        "MeshSphere" => Box::new(MeshPrimCmd::new("MeshSphere")),
        "MeshCylinder" => Box::new(MeshPrimCmd::new("MeshCylinder")),
        "MeshCone" => Box::new(MeshPrimCmd::new("MeshCone")),
        "MeshTorus" => Box::new(MeshPrimCmd::new("MeshTorus")),
        "OffsetSrf" => Box::new(OffsetSrfCmd::default()),
        "BooleanUnion" => Box::new(BooleanCmd::new(tri_geom::BooleanOp::Union)),
        "BooleanDifference" => Box::new(BooleanCmd::new(tri_geom::BooleanOp::Difference)),
        "BooleanIntersection" => Box::new(BooleanCmd::new(tri_geom::BooleanOp::Intersection)),
        "PointsOn" => Box::new(PointsOnCmd),
        "PointsOff" => Box::new(RequestCmd(AppRequest::PointsOff)),
        "PlanarSrf" => Box::new(PlanarSrfCmd),
        "Delete" => Box::new(DeleteCmd),
        "Undo" => Box::new(UndoCmd(false)),
        "Redo" => Box::new(UndoCmd(true)),
        "Move" => Box::new(MoveCmd::default()),
        "Copy" => Box::new(CopyCmd::default()),
        "Rotate" => Box::new(RotateCmd::default()),
        "Scale" => Box::new(ScaleCmd::new(3)),
        "Scale2D" => Box::new(ScaleCmd::new(2)),
        "Scale1D" => Box::new(ScaleCmd::new(1)),
        "Mirror" => Box::new(MirrorCmd::default()),
        "Array" => Box::new(ArrayCmd::default()),
        "ArrayPolar" => Box::new(ArrayPolarCmd::default()),
        "ArrayLinear" => Box::new(ArrayLinearCmd::default()),
        "ArrayCrv" => Box::new(ArrayCrvCmd::default()),
        "Orient" => Box::new(OrientCmd::new(false)),
        "Orient3Pt" => Box::new(OrientCmd::new(true)),
        "Rotate3D" => Box::new(Rotate3DCmd::default()),
        "Shear" => Box::new(ShearCmd::default()),
        "SetPt" => Box::new(SetPtCmd::new(false)),
        "ProjectToCPlane" => Box::new(SetPtCmd::new(true)),
        "Align" => Box::new(AlignCmd::default()),
        "Join" => Box::new(JoinCmd),
        "Trim" => Box::new(TrimCmd::default()),
        "Split" => Box::new(SplitCmd::default()),
        "Extend" => Box::new(ExtendCmd::default()),
        "Fillet" => Box::new(FilletCmd::default()),
        "Chamfer" => Box::new(ChamferCmd::default()),
        "Offset" => Box::new(OffsetCmd::default()),
        "Rebuild" => Box::new(RebuildAnyCmd::default()),
        "CPlane" => Box::new(CPlaneCmd::default()),
        "RectangleRounded" => Box::new(RoundedRectCmd::default()),
        "Divide" => Box::new(DivideCmd::default()),
        "Intersect" => Box::new(IntersectCmd),
        "Helix" => Box::new(HelixCmd::new(false)),
        "Spiral" => Box::new(HelixCmd::new(true)),
        "BlendCrv" => Box::new(BlendCrvCmd::default()),
        "Connect" => Box::new(ConnectCmd::default()),
        "ChangeDegree" => Box::new(ChangeDegreeCmd::default()),
        "CloseCrv" => Box::new(CloseCrvCmd),
        "Smooth" => Box::new(SmoothCmd::default()),
        "SimplifyCrv" => Box::new(SimplifyCrvCmd),
        "FitCrv" => Box::new(FitCrvCmd::default()),
        "InsertKnot" => Box::new(InsertKnotCmd),
        "CurveThroughPt" => Box::new(CurveThroughPtCmd),
        "Project" => Box::new(ProjectCmd::default()),
        "DupEdge" => Box::new(DupEdgeCmd),
        "DupBorder" => Box::new(DupBorderCmd),
        "Section" => Box::new(SectionCmd::default()),
        "Contour" => Box::new(ContourCmd::default()),
        "Explode" => Box::new(ExplodeCmd),
        "Hide" => Box::new(HideCmd),
        "Show" => Box::new(ShowCmd),
        "Lock" => Box::new(LockCmd),
        "Unlock" => Box::new(UnlockCmd),
        "Flip" => Box::new(FlipCmd),
        "Dir" => Box::new(FlipCmd),
        "SelAll" => Box::new(SelectCmd::All),
        "SelNone" => Box::new(SelectCmd::None),
        "Invert" => Box::new(SelectCmd::Invert),
        "SelCrv" => Box::new(SelectCmd::Filter(ObjFilter::Curve)),
        "SelSrf" => Box::new(SelectCmd::Filter(ObjFilter::Surface)),
        "SelPolysrf" => Box::new(SelectCmd::Filter(ObjFilter::Polysurface)),
        "SelPt" => Box::new(SelectCmd::Filter(ObjFilter::Point)),
        "SelMesh" => Box::new(SelectCmd::Filter(ObjFilter::Mesh)),
        "SelLast" => Box::new(SelectCmd::Last),
        "SelPrev" => Box::new(SelCmd(SelKind::Prev)),
        "SelOpenCrv" => Box::new(SelCmd(SelKind::OpenCrv)),
        "SelClosedCrv" => Box::new(SelCmd(SelKind::ClosedCrv)),
        "SelLine" => Box::new(SelCmd(SelKind::Line)),
        "SelPolyline" => Box::new(SelCmd(SelKind::Polyline)),
        "SelOpenSrf" => Box::new(SelCmd(SelKind::OpenSrf)),
        "SelClosedSrf" => Box::new(SelCmd(SelKind::ClosedSrf)),
        "SelOpenPolysrf" => Box::new(SelCmd(SelKind::OpenPolysrf)),
        "SelClosedPolysrf" => Box::new(SelCmd(SelKind::ClosedPolysrf)),
        "SelDup" => Box::new(SelCmd(SelKind::Dup)),
        "SelVisible" => Box::new(SelCmd(SelKind::Visible)),
        "SelHidden" => Box::new(SelCmd(SelKind::Hidden)),
        "SelLocked" => Box::new(SelCmd(SelKind::Locked)),
        "SelName" => Box::new(SelNameCmd),
        "SelSmall" => Box::new(SelSmallCmd),
        "SelID" => Box::new(SelIdCmd),
        "SelLayer" => Box::new(SelLayerCmd),
        "SelWindow" => Box::new(SelWindowCmd::new(false)),
        "SelCrossing" => Box::new(SelWindowCmd::new(true)),
        "SelGroup" => Box::new(GroupCmd::new(GroupOp::Select)),
        "LayerOn" => Box::new(LayerOpCmd(LayerOp::On)),
        "LayerOff" => Box::new(LayerOpCmd(LayerOp::Off)),
        "LayerLock" => Box::new(LayerOpCmd(LayerOp::Lock)),
        "LayerUnlock" => Box::new(LayerOpCmd(LayerOp::Unlock)),
        "OneLayerOn" => Box::new(LayerOpCmd(LayerOp::OneOn)),
        "NewLayer" => Box::new(LayerOpCmd(LayerOp::New)),
        "SetLayerToObject" => Box::new(LayerOpCmd(LayerOp::SetCurrentToObject)),
        "CopyToLayer" => Box::new(CopyToLayerCmd::default()),
        "Group" => Box::new(GroupCmd::new(GroupOp::Group)),
        "Ungroup" => Box::new(GroupCmd::new(GroupOp::Ungroup)),
        "AddToGroup" => Box::new(AddToGroupCmd::default()),
        "RemoveFromGroup" => Box::new(GroupCmd::new(GroupOp::RemoveFrom)),
        "SetGroupName" => Box::new(GroupCmd::new(GroupOp::SetName)),
        "HideSwap" => Box::new(VisCmd(VisOp::HideSwap)),
        "ShowSelected" => Box::new(VisCmd(VisOp::ShowSelected)),
        "Isolate" => Box::new(VisCmd(VisOp::Isolate)),
        "Unisolate" => Box::new(VisCmd(VisOp::Unisolate)),
        "Purge" => Box::new(PurgeCmd),
        "SetObjectName" => Box::new(SetObjectNameCmd::default()),
        "BoundingBox" => Box::new(BoundingBoxCmd),
        "Area" => Box::new(MeasureCmd(Measure::Area)),
        "AreaCentroid" => Box::new(MeasureCmd(Measure::AreaCentroid)),
        "Volume" => Box::new(MeasureCmd(Measure::Volume)),
        "VolumeCentroid" => Box::new(MeasureCmd(Measure::VolumeCentroid)),
        "Angle" => Box::new(AngleCmd::default()),
        "Radius" => Box::new(RadiusCmd),
        "EvaluatePt" => Box::new(EvaluatePtCmd),
        "ZoomWindow" => Box::new(ZoomWindowCmd::new(false)),
        "ZoomTarget" => Box::new(ZoomWindowCmd::new(true)),
        "NamedView" => Box::new(NamedViewCmd),
        "List" => Box::new(ListCmd),
        "Cut" => Box::new(RequestCmd(AppRequest::CopyToClipboard { cut: true })),
        "CopyToClipboard" => Box::new(RequestCmd(AppRequest::CopyToClipboard { cut: false })),
        "Paste" => Box::new(RequestCmd(AppRequest::Paste)),
        "PlanView" => Box::new(RequestCmd(AppRequest::PlanView)),
        "NextViewport" => Box::new(RequestCmd(AppRequest::NextViewport(true))),
        "PrevViewport" => Box::new(RequestCmd(AppRequest::NextViewport(false))),
        "About" => Box::new(RequestCmd(AppRequest::Message("triceratops 0.1 — a Rust re-implementation of Triceratops.".into()))),
        "Layer" => Box::new(LayerCmd::default()),
        "ChangeLayer" => Box::new(ChangeLayerCmd::default()),
        "Properties" => Box::new(RequestCmd(AppRequest::ShowPanel("Properties".into()))),
        "What" => Box::new(WhatCmd),
        "Distance" => Box::new(DistanceCmd::default()),
        "Length" => Box::new(LengthCmd),
        "Zoom" => Box::new(ZoomCmd::default()),
        "ZoomExtents" => Box::new(RequestCmd(AppRequest::ZoomExtents { all: false, selected: false })),
        "ZoomSelected" => Box::new(RequestCmd(AppRequest::ZoomExtents { all: false, selected: true })),
        "ZoomExtentsAll" => Box::new(RequestCmd(AppRequest::ZoomExtents { all: true, selected: false })),
        "4View" => Box::new(RequestCmd(AppRequest::Layout("4View".into()))),
        "3View" => Box::new(RequestCmd(AppRequest::Layout("3View".into()))),
        "MaxViewport" => Box::new(RequestCmd(AppRequest::MaxViewport)),
        "SetView" => Box::new(SetViewCmd::default()),
        "Perspective" | "Top" | "Front" | "Right" | "Bottom" | "Back" | "Left" => {
            Box::new(RequestCmd(AppRequest::SetView(name.to_string())))
        }
        "SetDisplayMode" => Box::new(SetDisplayModeCmd::default()),
        "Shaded" | "Wireframe" | "Rendered" | "Ghosted" | "XRay" | "Pen" | "Technical" | "Arctic" => {
            Box::new(RequestCmd(AppRequest::SetDisplayMode(name.to_string())))
        }
        "Grid" => Box::new(GridCmd::default()),
        "Snap" => Box::new(RequestCmd(AppRequest::ToggleSnap("grid".into()))),
        "Ortho" => Box::new(RequestCmd(AppRequest::ToggleSnap("ortho".into()))),
        "Planar" => Box::new(RequestCmd(AppRequest::ToggleSnap("planar".into()))),
        "Osnap" => Box::new(RequestCmd(AppRequest::ToggleSnap("osnap".into()))),
        "DisableOsnap" => Box::new(RequestCmd(AppRequest::ToggleSnap("disable".into()))),
        "ProjectOsnap" => Box::new(RequestCmd(AppRequest::ToggleSnap("project".into()))),
        "ShowOsnap" => Box::new(RequestCmd(AppRequest::ShowPanel("Osnap".into()))),
        "UndoView" => Box::new(RequestCmd(AppRequest::UndoView)),
        "RedoView" => Box::new(RequestCmd(AppRequest::RedoView)),
        "New" => Box::new(RequestCmd(AppRequest::FileNew)),
        "Open" => Box::new(RequestCmd(AppRequest::FileOpen)),
        "Save" => Box::new(RequestCmd(AppRequest::FileSave)),
        "SaveAs" => Box::new(RequestCmd(AppRequest::FileSaveAs)),
        "Import" => Box::new(RequestCmd(AppRequest::Import)),
        "Export" => Box::new(RequestCmd(AppRequest::Export)),
        "Exit" => Box::new(RequestCmd(AppRequest::Quit)),
        "Help" => Box::new(RequestCmd(AppRequest::Help)),
        "CommandHistory" => Box::new(RequestCmd(AppRequest::CommandHistory)),
        "CommandList" => Box::new(CommandListCmd),
        "Options" => Box::new(RequestCmd(AppRequest::Options("Appearance".into()))),
        "DocumentProperties" => Box::new(RequestCmd(AppRequest::DocumentProperties("Units".into()))),
        "Cancel" => Box::new(SelectCmd::None),
        "Notes" => Box::new(RequestCmd(AppRequest::ShowPanel("Notes".into()))),
        "ViewportProperties" => Box::new(RequestCmd(AppRequest::ShowPanel("Properties".into()))),
        _ => return None,
    })
}
