//! Application state: document, viewports, command runner, modeling aids,
//! picking/selection and display batch building.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use glam::{DAffine3, DVec2, DVec3};
use tri_doc::{io, Document, Geometry, ObjectId};
use tri_geom::{BoundingBox, NurbsSurface, Plane, Ray};

use crate::camera::{Camera, Projection, ViewHistory, ViewRect};
use crate::commands::{self, match_option, AppRequest, CmdCtx, CommandRunner, Input, Preview, PromptKind};
use crate::input::{parse_number, parse_point_input, PointInput};
use crate::render::scene::{surface_lines, SceneCache};
use crate::render::Batch;
use crate::snap::{OsnapSettings, SnapKind, SnapQuery};
use crate::theme::{lin, lin_a, AppearanceColors};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    Wireframe,
    Shaded,
    Rendered,
    Ghosted,
    XRay,
    Technical,
    Pen,
    Arctic,
    Zebra,
    EMap,
    Curvature,
    DraftAngle,
    ZBuffer,
    Thickness,
}

impl DisplayMode {
    pub const ALL: [DisplayMode; 14] = [
        DisplayMode::Wireframe,
        DisplayMode::Shaded,
        DisplayMode::Rendered,
        DisplayMode::Ghosted,
        DisplayMode::XRay,
        DisplayMode::Technical,
        DisplayMode::Pen,
        DisplayMode::Arctic,
        DisplayMode::Zebra,
        DisplayMode::EMap,
        DisplayMode::Curvature,
        DisplayMode::DraftAngle,
        DisplayMode::ZBuffer,
        DisplayMode::Thickness,
    ];
    /// Shader analysis mode id (0 = none).
    pub fn analysis(&self) -> u32 {
        match self {
            DisplayMode::Zebra => 1,
            DisplayMode::EMap => 2,
            DisplayMode::Curvature => 3,
            DisplayMode::DraftAngle => 4,
            DisplayMode::ZBuffer => 5,
            DisplayMode::Thickness => 6,
            _ => 0,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            DisplayMode::Wireframe => "Wireframe",
            DisplayMode::Shaded => "Shaded",
            DisplayMode::Rendered => "Rendered",
            DisplayMode::Ghosted => "Ghosted",
            DisplayMode::XRay => "X-Ray",
            DisplayMode::Technical => "Technical",
            DisplayMode::Pen => "Pen",
            DisplayMode::Arctic => "Arctic",
            DisplayMode::Zebra => "Zebra",
            DisplayMode::EMap => "EMap",
            DisplayMode::Curvature => "Curvature",
            DisplayMode::DraftAngle => "DraftAngle",
            DisplayMode::ZBuffer => "ZBuffer",
            DisplayMode::Thickness => "Thickness",
        }
    }
    pub fn from_name(n: &str) -> Option<Self> {
        let n = n.to_ascii_lowercase().replace('-', "");
        Self::ALL.iter().copied().find(|m| m.name().to_ascii_lowercase().replace('-', "") == n)
    }
    pub fn shaded(&self) -> bool {
        !matches!(self, DisplayMode::Wireframe)
    }
    pub fn isocurves(&self) -> bool {
        matches!(self, DisplayMode::Wireframe | DisplayMode::Shaded | DisplayMode::Ghosted | DisplayMode::XRay)
    }
    pub fn white_background(&self) -> bool {
        matches!(self, DisplayMode::Technical | DisplayMode::Pen | DisplayMode::Arctic)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Single,
    Four,
    Three,
    /// All viewports in a grid (NewViewport / SplitViewport).
    Grid,
}

#[derive(Clone)]
pub struct Viewport {
    pub name: String,
    pub camera: Camera,
    pub rect: ViewRect,
    pub display: DisplayMode,
    pub history: ViewHistory,
    /// Custom construction plane set with the CPlane command.
    pub cplane_override: Option<Plane>,
    pub cplane_previous: Vec<Option<Plane>>,
    /// Gradient background (top, bottom) set with GradientView.
    pub gradient: Option<([u8; 3], [u8; 3])>,
    /// Page layout data when this viewport is a page (Layout command).
    pub page: Option<PageLayout>,
    /// Index of the page viewport this detail belongs to.
    pub detail_of: Option<usize>,
    /// Detail rectangle on the page: x, y, width, height in page units.
    pub detail_rect: [f64; 4],
    /// Objects and layers hidden in this detail only.
    pub detail_hidden: Vec<ObjectId>,
    pub detail_hidden_layers: Vec<usize>,
}

/// Paper size of a page layout viewport (page units are the model units, like Triceratops' layout units).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PageLayout {
    pub width: f64,
    pub height: f64,
}

/// Serialisable viewport description (ReadViewportsFromFile, ImportLayout).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ViewportSave {
    pub name: String,
    pub camera: Camera,
    #[serde(default)]
    pub page: Option<PageLayout>,
    #[serde(default)]
    pub detail_of: Option<String>,
    #[serde(default)]
    pub detail_rect: [f64; 4],
}

fn fmt_num_app(v: f64) -> String {
    crate::commands::fmt_num(v)
}

fn fmt_rect(r: [f64; 4]) -> String {
    format!("{} {} {} {}", fmt_num_app(r[0]), fmt_num_app(r[1]), fmt_num_app(r[2]), fmt_num_app(r[3]))
}

impl Viewport {
    fn new(name: &str, camera: Camera, display: DisplayMode) -> Self {
        Self { name: name.into(), camera, rect: ViewRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }, display, history: ViewHistory::default(), cplane_override: None, cplane_previous: Vec::new(), gradient: None, page: None, detail_of: None, detail_rect: [0.0; 4], detail_hidden: Vec::new(), detail_hidden_layers: Vec::new() }
    }
    pub fn is_page(&self) -> bool {
        self.page.is_some()
    }
    pub fn is_detail(&self) -> bool {
        self.detail_of.is_some()
    }
    pub fn cplane(&self) -> Plane {
        self.cplane_override.unwrap_or_else(|| self.camera.default_cplane())
    }
    pub fn set_cplane(&mut self, p: Option<Plane>) {
        self.cplane_previous.push(self.cplane_override);
        self.cplane_override = p;
    }
    pub fn previous_cplane(&mut self) {
        if let Some(p) = self.cplane_previous.pop() {
            self.cplane_override = p;
        }
    }
}

/// Status bar toggles (Modeling aids).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ModelingAids {
    pub grid_snap: bool,
    pub ortho: bool,
    pub ortho_angle: f64,
    pub planar: bool,
    pub smarttrack: bool,
    pub gumball: bool,
    pub record_history: bool,
    pub osnap: OsnapSettings,
    pub world_coords: bool,
}

impl Default for ModelingAids {
    fn default() -> Self {
        Self {
            grid_snap: false,
            ortho: false,
            ortho_angle: 90.0,
            planar: false,
            smarttrack: false,
            gumball: true,
            record_history: false,
            osnap: OsnapSettings::default(),
            world_coords: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Cursor {
    pub viewport: usize,
    pub screen: DVec2,
    pub world: DVec3,
    pub snap: Option<SnapKind>,
}

#[derive(Clone, Copy, Debug)]
pub struct WindowDrag {
    pub viewport: usize,
    pub start: DVec2,
    pub current: DVec2,
}

#[derive(Clone, Debug)]
pub struct DragMove {
    pub viewport: usize,
    pub ids: Vec<ObjectId>,
    pub from: DVec3,
    pub to: DVec3,
}

#[derive(Clone, Debug)]
pub struct SelectionMenu {
    pub screen: DVec2,
    pub candidates: Vec<ObjectId>,
    pub shift: bool,
    pub ctrl: bool,
    pub hovered: Option<ObjectId>,
}

/// Cursor constraints typed at a point prompt or set with Tab / Ctrl.
#[derive(Clone, Debug, Default)]
pub struct Constraints {
    pub distance: Option<f64>,
    pub angle_deg: Option<f64>,
    /// Direction lock (Tab) from the base point.
    pub direction: Option<DVec3>,
    /// Elevator mode: vertical line through this point.
    pub elevator: Option<DVec3>,
}

pub struct LoftDialog {
    pub ids: Vec<ObjectId>,
    pub style: usize,
    pub closed: bool,
    pub rebuild: usize,
    pub rebuild_count: usize,
    pub refit_tol: f64,
}

pub const LOFT_STYLES: [&str; 5] = ["Normal", "Loose", "Tight", "Straight sections", "Uniform"];

#[derive(Default)]
pub struct Panels {
    pub tab: String,
    pub visible: bool,
    pub layer_selected: Option<usize>,
    pub layer_editing: Option<(usize, String)>,
    pub material_selected: Option<usize>,
    pub material_new_name: String,
}

/// Where and how the last exports were written (see `App::file_export`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportMemory {
    pub last_dir: String,
    pub last_format: String,
    /// (working file path, last export folder used for it)
    pub dirs: Vec<(String, String)>,
}

impl ExportMemory {
    /// Initial folder: for a saved working file the folder last used to export it, else the
    /// working file's own folder; for an unsaved document the last export folder.
    pub fn folder_for(&self, doc_path: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
        match doc_path {
            Some(p) => {
                let key = p.to_string_lossy();
                self.dirs.iter().find(|(d, _)| *d == key).map(|(_, f)| std::path::PathBuf::from(f)).filter(|f| f.is_dir()).or_else(|| p.parent().map(|d| d.to_path_buf()))
            }
            None => Some(std::path::PathBuf::from(&self.last_dir)).filter(|f| !self.last_dir.is_empty() && f.is_dir()),
        }
    }

    /// Initial name: the working file's name, or "export", with the extension of `ext`.
    pub fn name_for(doc_path: Option<&std::path::Path>, ext: &str) -> String {
        let stem = doc_path.and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "export".into());
        format!("{stem}.{ext}")
    }

    pub fn remember(&mut self, doc_path: Option<&std::path::Path>, exported: &std::path::Path, ext: &str) {
        let Some(dir) = exported.parent().map(|d| d.to_string_lossy().to_string()) else { return };
        self.last_dir = dir.clone();
        self.last_format = ext.to_string();
        if let Some(p) = doc_path {
            let key = p.to_string_lossy().to_string();
            self.dirs.retain(|(d, _)| *d != key);
            self.dirs.push((key, dir));
            if self.dirs.len() > 50 {
                self.dirs.remove(0);
            }
        }
    }
}

/// Persisted user options (~/.config/triceratops/options.json).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct UserOptions {
    pub dark_mode: bool,
    /// Name of the UI theme (built-in "Triceratops Light", "Triceratops Dark", "Blender" or a user theme file).
    #[serde(default)]
    pub theme: String,
    /// Graphics backend: "auto", "gl" (OpenGL ES through EGL) or "vulkan".
    #[serde(default)]
    pub backend: String,
    pub colors: AppearanceColors,
    pub osnap: OsnapSettings,
    pub sidebar_width: f32,
    pub panels_visible: bool,
    pub panel_tab: String,
    pub grid_snap: bool,
    pub ortho: bool,
    pub planar: bool,
    /// Every modeling aid (osnaps, ortho, planar, gumball, SmartTrack, history recording, ...).
    pub aids: Option<ModelingAids>,
    /// Window layout: docked and floating containers of panels and toolbars.
    pub dock: Option<crate::ui::dock::DockLayout>,
    pub show_command_area: bool,
    pub show_viewport_tabs: bool,
    /// AI assistants may drive this window through `triceratops mcp`.
    pub allow_assistants: bool,
    /// Viewport arrangement ("Four", "Three", "Single", "Grid").
    pub viewport_layout: String,
    /// Display mode per viewport name.
    pub display_modes: Vec<(String, String)>,
    pub zoom_border: f64,
    pub drag_mode: String,
    pub drag_strength: f64,
    pub gumball_align: String,
    pub toolbar_tab: usize,
    /// Export dialog memory: last folder for unsaved documents, last file type (extension),
    /// and the last export folder of each saved working file.
    pub last_export_dir: String,
    pub last_export_format: String,
    pub export_dirs: Vec<(String, String)>,
}

impl Default for UserOptions {
    fn default() -> Self {
        Self {
            dark_mode: false,
            theme: "Triceratops Light".into(),
            backend: "auto".into(),
            colors: AppearanceColors::default(),
            osnap: OsnapSettings::default(),
            sidebar_width: 70.0,
            panels_visible: true,
            panel_tab: "Properties".into(),
            grid_snap: false,
            ortho: false,
            planar: false,
            aids: None,
            dock: None,
            show_command_area: true,
            show_viewport_tabs: true,
            allow_assistants: true,
            viewport_layout: "Four".into(),
            display_modes: Vec::new(),
            zoom_border: 0.05,
            drag_mode: "CPlane".into(),
            drag_strength: 1.0,
            gumball_align: "CPlane".into(),
            toolbar_tab: 0,
            last_export_dir: String::new(),
            last_export_format: "3dm".into(),
            export_dirs: Vec::new(),
        }
    }
}

pub fn options_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("triceratops").join("options.json"))
}

/// A control point grip shown by PointsOn.
#[derive(Clone, Copy, Debug)]
pub struct Grip {
    pub object: ObjectId,
    pub index: usize,
    pub pos: DVec3,
    pub selected: bool,
}

#[derive(Clone, Debug)]
pub struct GripDrag {
    pub viewport: usize,
    pub to: DVec3,
    pub started: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GumballHandle {
    Move(usize),
    Scale(usize),
    Rotate(usize),
    Free,
}

#[derive(Clone, Debug)]
pub struct GumballDrag {
    pub viewport: usize,
    pub handle: GumballHandle,
    pub origin: DVec3,
    pub axes: [DVec3; 3],
    pub start: DVec3,
    pub originals: Vec<(ObjectId, Geometry)>,
    pub last_xf: DAffine3,
}

/// Screen-space description of the gumball for drawing and hit testing.
#[derive(Clone, Debug)]
pub struct GumballScreen {
    pub origin: DVec2,
    pub world_origin: DVec3,
    pub axes: [DVec3; 3],
    pub arrow_end: [DVec2; 3],
    pub scale_pos: [DVec2; 3],
    pub arcs: [Vec<DVec2>; 3],
}

/// A clipping plane object: a planar surface in the document whose plane clips the display.
#[derive(Clone, Debug)]
pub struct ClipPlane {
    pub object: ObjectId,
    pub enabled: bool,
    pub flipped: bool,
}

/// A view capture queued for the event loop.
#[derive(Clone, Debug, Default)]
pub struct CaptureRequest {
    pub path: String,
    pub clipboard: bool,
    pub whole_window: bool,
    /// Named view to restore before capturing.
    pub view: Option<String>,
    /// Viewport to capture instead of the active one.
    pub viewport: Option<usize>,
    /// Keep the PNG in memory under this id (for an assistant) instead of saving it.
    pub memory: Option<u64>,
    /// Scale the image down to fit this size, keeping its proportions.
    pub max_size: Option<(u32, u32)>,
}

/// One frame of an animation: optional camera and sun direction.
#[derive(Clone, Debug)]
pub struct AnimFrame {
    pub camera: Option<Camera>,
    pub sun: Option<DVec3>,
}

/// Camera / sun animation (Turntable, SetPathAnimation, SetFlythroughAnimation, sun animations).
#[derive(Clone, Debug, Default)]
pub struct Animation {
    pub frames: Vec<AnimFrame>,
    pub index: usize,
    pub playing: bool,
    pub record_dir: Option<String>,
}

pub struct App {
    /// Ask the renderer to reload texture files.
    pub refresh_textures: bool,
    /// True while history records are being replayed (no recording, no nested updates).
    pub replaying: bool,
    /// Points collected while dragging during the Sketch command.
    pub sketch_points: Option<Vec<DVec3>>,
    /// Show texture mapping boxes for selected objects.
    pub mapping_widgets: bool,
    /// Pending window operations for the event loop ("minimize", "fullscreen", "maximize").
    pub window_ops: Vec<String>,
    /// Pending view capture requests.
    pub capture_requests: Vec<CaptureRequest>,
    /// Captures kept in memory for assistants: (id, PNG bytes or error).
    pub finished_captures: Vec<(u64, Result<Vec<u8>, String>)>,
    pub next_capture_id: u64,
    /// Sun light direction (towards the light) when enabled.
    pub sun: Option<DVec3>,
    /// Ground plane height when enabled.
    pub ground_plane: Option<f64>,
    pub animation: Animation,
    /// Analysis overlays: naked edges, curve ends, directions, camera frustum.
    pub show_edges: bool,
    pub show_ends: bool,
    pub show_dir: bool,
    pub show_camera: bool,
    /// Infinite construction guides (two points each).
    pub guides: Vec<(DVec3, DVec3)>,
    pub zoom_border: f64,
    pub px_per_mm: f64,
    pub walk_mode: bool,
    pub show_command_area: bool,
    /// Dockable window layout and the on-screen rectangles of its containers.
    pub dock: crate::ui::dock::DockLayout,
    pub dock_rects: HashMap<u32, egui::Rect>,
    /// Set when an option changed in the UI; saved at the end of the frame.
    pub options_dirty: bool,
    pub last_saved_options: Option<UserOptions>,
    pub export_memory: ExportMemory,
    pub gumball_origin: Option<DVec3>,
    pub gumball_axes: Option<[DVec3; 3]>,
    pub gumball_auto_reset: bool,
    pub gumball_align: String,
    pub drag_mode: String,
    pub drag_strength: f64,
    pub layer_states: Vec<(String, Vec<tri_doc::Layer>)>,
    pub layer_book_index: usize,
    pub named_selections: Vec<(String, Vec<ObjectId>)>,
    pub mplane_object: Option<ObjectId>,
    pub render_mesh_hidden: bool,
    pub print_display: bool,
    /// Restrict picking to a kind of object ("curve", "surface", "polysurface", "mesh", "point").
    pub selection_filter: Option<String>,
    pub clipping_planes: Vec<ClipPlane>,
    pub show_viewport_tabs: bool,
    pub gumball_drag: Option<GumballDrag>,
    pub clipboard: Vec<(Geometry, tri_doc::Attributes)>,
    pub named_views: Vec<(String, Camera)>,
    pub points_on: Vec<ObjectId>,
    pub grips: Vec<Grip>,
    pub grip_drag: Option<GripDrag>,
    pub dark_mode: bool,
    /// Active UI theme; `theme_revision` bumps whenever it changes so the egui style is re-applied.
    pub theme: crate::theme::Theme,
    pub theme_revision: u64,
    pub backend: String,
    pub sidebar_width: f32,
    pub doc: Document,
    pub viewports: Vec<Viewport>,
    pub active: usize,
    pub layout: Layout,
    pub runner: CommandRunner,
    pub log: Vec<String>,
    pub cmd_text: String,
    pub autocomplete_index: Option<usize>,
    pub aids: ModelingAids,
    pub colors: AppearanceColors,
    pub scene: SceneCache,
    /// The Locust node editor and its viewport preview.
    pub locust: crate::locust::ui::Editor,
    locust_displays: Vec<(crate::render::scene::ObjectDisplay, [f32; 4], [f32; 4])>,
    locust_displays_rev: u64,
    pub cursor: Option<Cursor>,
    pub pick_order: Vec<ObjectId>,
    pub window_drag: Option<WindowDrag>,
    pub drag_move: Option<DragMove>,
    pub selection_menu: Option<SelectionMenu>,
    pub constraints: Constraints,
    pub one_shot_snap: Option<SnapKind>,
    /// SmartTrack: object-snap points the cursor has rested on during this command.
    pub track_points: Vec<DVec3>,
    /// SmartTrack lines the cursor currently follows (for drawing).
    pub track_lines: Vec<(DVec3, DVec3)>,
    pub alt_down: bool,
    pub shift_down: bool,
    pub requests: Vec<AppRequest>,
    pub quit: bool,
    pub show_help: bool,
    pub show_history: bool,
    pub options_page: Option<String>,
    pub docprops_page: Option<String>,
    pub loft_dialog: Option<LoftDialog>,
    pub popup_toolbar: Option<egui::Pos2>,
    pub panels: Panels,
    pub toolbar_tab: usize,
    pub status_hint: String,
    pub memory: HashMap<String, String>,
    pub last_selection: Vec<ObjectId>,
    pub last_added: Vec<ObjectId>,
    pub message_box: Option<String>,
    /// Commands are being run for an AI assistant: no modal dialogs, messages go to the log.
    pub assistant_mode: bool,
    /// The running command was started by an assistant.
    pub assistant_owns_command: bool,
    /// Script lines waiting for the running job to end, and whether an assistant sent them.
    pub pending_lines: Vec<String>,
    pending_assistant: bool,
    /// Assistants may connect (Options > General); saved with the options.
    pub allow_assistants: bool,
    /// A `triceratops mcp` bridge is attached right now (status bar).
    pub assistant_connected: bool,
    scene_batches: Vec<Batch>,
    scene_keys: Vec<Option<u64>>,
    pub overlay_batches: Vec<Batch>,
    pub viewport_tab_order: Vec<usize>,
}

impl App {
    pub fn new() -> Self {
        let viewports = vec![
            Viewport::new("Top", Camera::top(), DisplayMode::Wireframe),
            Viewport::new("Perspective", Camera::perspective(), DisplayMode::Shaded),
            Viewport::new("Front", Camera::front(), DisplayMode::Wireframe),
            Viewport::new("Right", Camera::right(), DisplayMode::Wireframe),
        ];
        let n = viewports.len();
        let mut app = Self {
            refresh_textures: false,
            replaying: false,
            sketch_points: None,
            mapping_widgets: false,
            window_ops: Vec::new(),
            capture_requests: Vec::new(),
            finished_captures: Vec::new(),
            next_capture_id: 1,
            sun: None,
            ground_plane: None,
            animation: Animation::default(),
            show_edges: false,
            show_ends: false,
            show_dir: false,
            show_camera: false,
            guides: Vec::new(),
            track_points: Vec::new(),
            track_lines: Vec::new(),
            zoom_border: 0.05,
            px_per_mm: 3.78,
            walk_mode: false,
            show_command_area: true,
            dock: crate::ui::dock::DockLayout::default_layout(),
            dock_rects: HashMap::new(),
            options_dirty: false,
            last_saved_options: None,
            export_memory: ExportMemory::default(),
            gumball_origin: None,
            gumball_axes: None,
            gumball_auto_reset: true,
            gumball_align: "CPlane".into(),
            drag_mode: "CPlane".into(),
            drag_strength: 1.0,
            layer_states: Vec::new(),
            layer_book_index: 0,
            named_selections: Vec::new(),
            mplane_object: None,
            render_mesh_hidden: false,
            print_display: false,
            selection_filter: None,
            clipping_planes: Vec::new(),
            show_viewport_tabs: true,
            gumball_drag: None,
            clipboard: Vec::new(),
            named_views: Vec::new(),
            points_on: Vec::new(),
            grips: Vec::new(),
            grip_drag: None,
            dark_mode: false,
            theme: crate::theme::Theme::default_light(),
            theme_revision: 0,
            backend: "auto".into(),
            sidebar_width: 70.0,
            doc: Document::new(),
            viewports,
            active: 1,
            layout: Layout::Four,
            runner: CommandRunner::default(),
            log: vec!["Command: ".into()],
            cmd_text: String::new(),
            autocomplete_index: None,
            aids: ModelingAids::default(),
            colors: AppearanceColors::default(),
            scene: SceneCache::default(),
            locust: Default::default(),
            locust_displays: Vec::new(),
            locust_displays_rev: 0,
            cursor: None,
            pick_order: Vec::new(),
            window_drag: None,
            drag_move: None,
            selection_menu: None,
            constraints: Constraints::default(),
            one_shot_snap: None,
            alt_down: false,
            shift_down: false,
            requests: Vec::new(),
            quit: false,
            show_help: false,
            show_history: false,
            options_page: None,
            docprops_page: None,
            loft_dialog: None,
            popup_toolbar: None,
            panels: Panels { tab: "Properties".into(), visible: true, ..Default::default() },
            toolbar_tab: 0,
            status_hint: String::new(),
            memory: HashMap::new(),
            last_selection: Vec::new(),
            last_added: Vec::new(),
            message_box: None,
            assistant_mode: false,
            assistant_owns_command: false,
            pending_lines: Vec::new(),
            pending_assistant: false,
            allow_assistants: true,
            assistant_connected: false,
            scene_batches: (0..n).map(|_| Batch::default()).collect(),
            scene_keys: vec![None; n],
            overlay_batches: (0..n).map(|_| Batch::default()).collect(),
            viewport_tab_order: vec![1, 0, 2, 3],
        };
        app.zoom_extents(false, true);
        if !cfg!(test) {
            app.load_options();
        }
        app
    }

    pub fn load_options(&mut self) {
        if cfg!(test) {
            return;
        }
        let Some(p) = options_path() else { return };
        let Ok(text) = std::fs::read_to_string(&p) else { return };
        if let Ok(o) = serde_json::from_str::<UserOptions>(&text) {
            self.dark_mode = o.dark_mode;
            self.backend = if o.backend.is_empty() { "auto".into() } else { o.backend.clone() };
            if !o.theme.is_empty() {
                if let Some(t) = crate::theme::Theme::by_name(&o.theme) {
                    self.theme = t;
                    self.dark_mode = self.theme.dark;
                    self.theme_revision += 1;
                }
            } else if o.dark_mode {
                self.theme = crate::theme::Theme::default_dark();
                self.theme_revision += 1;
            }
            self.colors = o.colors;
            if o.dark_mode && self.colors.prompt_background == AppearanceColors::default().prompt_background {
                self.colors.set_scheme(true);
            }
            self.aids.osnap = o.osnap;
            self.sidebar_width = o.sidebar_width.clamp(40.0, 600.0);
            self.aids.grid_snap = o.grid_snap;
            self.aids.ortho = o.ortho;
            self.aids.planar = o.planar;
            if let Some(aids) = o.aids {
                self.aids = aids;
            }
            if let Some(mut dock) = o.dock {
                dock.drop_retired_tabs();
                self.dock = dock;
            }
            self.show_command_area = o.show_command_area;
            self.show_viewport_tabs = o.show_viewport_tabs;
            self.allow_assistants = o.allow_assistants;
            self.layout = match o.viewport_layout.as_str() { "Three" => Layout::Three, "Single" => Layout::Single, "Grid" => Layout::Grid, _ => Layout::Four };
            for (name, mode) in &o.display_modes {
                if let (Some(v), Some(m)) = (self.viewports.iter_mut().find(|v| &v.name == name), DisplayMode::from_name(mode)) {
                    v.display = m;
                }
            }
            self.zoom_border = o.zoom_border.clamp(0.0, 1.0);
            self.drag_mode = o.drag_mode;
            self.drag_strength = o.drag_strength.clamp(0.01, 10.0);
            self.gumball_align = o.gumball_align;
            self.toolbar_tab = o.toolbar_tab;
            self.export_memory = ExportMemory { last_dir: o.last_export_dir, last_format: o.last_export_format, dirs: o.export_dirs };
            self.memory.insert("RecordHistory".into(), if self.aids.record_history { "Yes".into() } else { "No".into() });
        }
        self.last_saved_options = Some(self.options_snapshot());
    }

    /// Show a panel (unhide its container and activate its tab).
    pub fn show_panel(&mut self, name: &str) {
        self.dock.show_tab(crate::ui::dock::ContainerKind::Panels, name);
        self.options_dirty = true;
    }

    /// Hide every container docked on a side, or show them again when all are hidden.
    pub fn toggle_side(&mut self, side: crate::ui::dock::Placement) {
        use crate::ui::dock::Placement;
        let shown = self.dock.containers_at(&side);
        if shown.is_empty() {
            // bring back hidden containers that were last docked here, else nothing
            let ids: Vec<u32> = self.dock.containers.iter().filter(|c| c.placement == Placement::Hidden).map(|c| c.id).collect();
            for id in ids {
                if let Some(c) = self.dock.container_mut(id) {
                    c.placement = side.clone();
                }
            }
        } else {
            for id in shown {
                self.dock.hide(id);
            }
        }
        self.options_dirty = true;
    }

    /// Window layout files and resets: "save"/"load" (with a dialog when the path is empty),
    /// "reset" (stock layout) and "reset_all" (every option back to the defaults).
    pub fn layout_file(&mut self, op: &str, path: &str) {
        match op {
            "save" => {
                let p = if path.is_empty() {
                    if self.file_dialog_blocked("SaveWindowLayout") { return; }
                    rfd::FileDialog::new().add_filter("Window layout (*.json)", &["json"]).set_file_name("layout.json").save_file()
                } else {
                    Some(std::path::PathBuf::from(path))
                };
                let Some(p) = p else { return };
                let mut l = self.dock.clone();
                l.show_command_area = self.show_command_area;
                l.show_viewport_tabs = self.show_viewport_tabs;
                match l.save(&p) {
                    Ok(()) => self.log.push(format!("Window layout saved to {}.", p.display())),
                    Err(e) => self.log.push(format!("Unable to save the layout: {e}")),
                }
            }
            "load" => {
                if path.is_empty() && self.file_dialog_blocked("LoadWindowLayout") {
                    return;
                }
                let p = if path.is_empty() { rfd::FileDialog::new().add_filter("Window layout (*.json)", &["json"]).pick_file() } else { Some(std::path::PathBuf::from(path)) };
                let Some(p) = p else { return };
                match crate::ui::dock::DockLayout::load(&p) {
                    Ok(l) => {
                        self.show_command_area = l.show_command_area;
                        self.show_viewport_tabs = l.show_viewport_tabs;
                        self.dock = l;
                        self.options_dirty = true;
                        self.log.push(format!("Window layout loaded from {}.", p.display()));
                    }
                    Err(e) => self.log.push(format!("Unable to load the layout: {e}")),
                }
            }
            "reset" => {
                self.dock = crate::ui::dock::DockLayout::default_layout();
                self.show_command_area = true;
                self.show_viewport_tabs = true;
                self.options_dirty = true;
                self.log.push("Window layout reset to the defaults.".into());
            }
            "reset_all" => {
                let d = UserOptions::default();
                self.dock = crate::ui::dock::DockLayout::default_layout();
                self.show_command_area = true;
                self.show_viewport_tabs = true;
                self.aids = ModelingAids::default();
                self.colors = AppearanceColors::default();
                self.theme = crate::theme::Theme::default_light();
                self.dark_mode = false;
                self.theme_revision += 1;
                self.backend = d.backend;
                self.sidebar_width = d.sidebar_width;
                self.zoom_border = d.zoom_border;
                self.drag_mode = d.drag_mode;
                self.drag_strength = d.drag_strength;
                self.gumball_align = d.gumball_align;
                self.gumball_origin = None;
                self.gumball_axes = None;
                self.layout = Layout::Four;
                self.options_dirty = true;
                self.doc.touch_revision();
                self.log.push("All options reset to the defaults.".into());
            }
            _ => {}
        }
    }

    /// Save the options when anything changed since the last save (called every frame while
    /// no mouse button is down, so drags do not spam the disk).
    pub fn autosave_options(&mut self) {
        let snap = self.options_snapshot();
        if self.last_saved_options.as_ref() != Some(&snap) {
            self.last_saved_options = Some(snap);
            self.save_options();
        }
        self.options_dirty = false;
    }

    pub fn save_options(&self) {
        if cfg!(test) {
            return;
        }
        let Some(p) = options_path() else { return };
        let o = self.options_snapshot();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&o) {
            let _ = std::fs::write(&p, text);
        }
    }

    /// The current state of every persisted option.
    pub fn options_snapshot(&self) -> UserOptions {
        UserOptions {
            dark_mode: self.dark_mode,
            theme: self.theme.name.clone(),
            backend: self.backend.clone(),
            colors: self.colors.clone(),
            osnap: self.aids.osnap,
            sidebar_width: self.sidebar_width,
            panels_visible: self.panels.visible,
            panel_tab: self.panels.tab.clone(),
            grid_snap: self.aids.grid_snap,
            ortho: self.aids.ortho,
            planar: self.aids.planar,
            aids: Some(self.aids.clone()),
            dock: Some(self.dock.clone()),
            show_command_area: self.show_command_area,
            show_viewport_tabs: self.show_viewport_tabs,
            allow_assistants: self.allow_assistants,
            viewport_layout: match self.layout { Layout::Three => "Three", Layout::Single => "Single", Layout::Grid => "Grid", Layout::Four => "Four" }.into(),
            display_modes: self.viewports.iter().filter(|v| !v.is_detail() && !v.is_page()).map(|v| (v.name.clone(), v.display.name().to_string())).collect(),
            zoom_border: self.zoom_border,
            drag_mode: self.drag_mode.clone(),
            drag_strength: self.drag_strength,
            gumball_align: self.gumball_align.clone(),
            toolbar_tab: self.toolbar_tab,
            last_export_dir: self.export_memory.last_dir.clone(),
            last_export_format: self.export_memory.last_format.clone(),
            export_dirs: self.export_memory.dirs.clone(),
        }
    }

    pub fn set_dark_mode(&mut self, dark: bool) {
        self.set_theme(if dark { crate::theme::Theme::default_dark() } else { crate::theme::Theme::default_light() });
    }

    /// Switch the UI theme: widgets, viewport colours and background follow the theme.
    pub fn set_theme(&mut self, theme: crate::theme::Theme) {
        self.dark_mode = theme.dark;
        self.colors = theme.viewport.clone();
        self.theme = theme;
        self.theme_revision += 1;
        self.doc.touch_revision();
        self.save_options();
    }

    pub fn set_theme_by_name(&mut self, name: &str) -> bool {
        match crate::theme::Theme::by_name(name) {
            Some(t) => {
                self.log.push(format!("Theme \"{}\" applied.", t.name));
                self.set_theme(t);
                true
            }
            None => {
                let names: Vec<String> = crate::theme::Theme::all().into_iter().map(|t| t.name).collect();
                self.log.push(format!("No theme named \"{name}\". Available: {}", names.join(", ")));
                false
            }
        }
    }

    /// Theme requests from the Theme command: "set:<name>", "list", "export:<file>", "import:<file>",
    /// "save:<name>" (current colours as a user theme), "reload", "backend:<auto|gl|vulkan>".
    pub fn theme_op(&mut self, op: &str) {
        let (cmd, arg) = op.split_once(':').unwrap_or((op, ""));
        match cmd {
            "set" => {
                self.set_theme_by_name(arg);
            }
            "list" => {
                let names: Vec<String> = crate::theme::Theme::all().into_iter().map(|t| if t.name == self.theme.name { format!("[{}]", t.name) } else { t.name }).collect();
                self.log.push(format!("Themes: {} (user themes in {})", names.join(", "), crate::theme::Theme::user_dir().map(|d| d.display().to_string()).unwrap_or_default()));
            }
            "export" => {
                let mut t = self.theme.clone();
                t.viewport = self.colors.clone();
                match t.save(std::path::Path::new(arg)) {
                    Ok(()) => self.log.push(format!("Theme written to {arg}.")),
                    Err(e) => self.log.push(format!("Unable to write {arg}: {e}")),
                }
            }
            "import" => match crate::theme::Theme::load(std::path::Path::new(arg)) {
                Ok(t) => {
                    let _ = t.save_user();
                    self.log.push(format!("Theme \"{}\" imported and applied.", t.name));
                    self.set_theme(t);
                }
                Err(e) => self.log.push(format!("Unable to read {arg}: {e}")),
            },
            "save" => {
                let mut t = self.theme.clone();
                t.viewport = self.colors.clone();
                if !arg.trim().is_empty() {
                    t.name = arg.trim().to_string();
                }
                match t.save_user() {
                    Ok(p) => {
                        self.log.push(format!("Theme \"{}\" saved to {}.", t.name, p.display()));
                        self.set_theme(t);
                    }
                    Err(e) => self.log.push(format!("Unable to save the theme: {e}")),
                }
            }
            "reload" => {
                let name = self.theme.name.clone();
                self.set_theme_by_name(&name);
            }
            "backend" => {
                let b = arg.trim().to_ascii_lowercase();
                if ["auto", "gl", "vulkan"].contains(&b.as_str()) {
                    self.backend = b.clone();
                    self.save_options();
                    self.log.push(format!("Graphics backend set to {b}; restart triceratops to apply (TRICERATOPS_BACKEND overrides it)."));
                } else {
                    self.log.push("Backend must be auto, gl or vulkan.".into());
                }
            }
            _ => {}
        }
    }

    /// Viewports shown by the layout (pages included), without their details.
    pub fn base_viewports(&self) -> Vec<usize> {
        let active = self.viewports[self.active].detail_of.unwrap_or(self.active);
        let base: Vec<usize> = match self.layout {
            Layout::Single => vec![active],
            Layout::Four => (0..self.viewports.len()).collect(),
            Layout::Three => vec![0, 1, 2],
            Layout::Grid => (0..self.viewports.len()).collect(),
        };
        base.into_iter().filter(|i| *i < self.viewports.len() && !self.viewports[*i].is_detail()).collect()
    }

    pub fn visible_viewports(&self) -> Vec<usize> {
        let mut out = self.base_viewports();
        for page in out.clone() {
            if self.viewports[page].is_page() {
                out.extend(self.details_of(page));
            }
        }
        out
    }

    pub fn details_of(&self, page: usize) -> Vec<usize> {
        (0..self.viewports.len()).filter(|i| self.viewports[*i].detail_of == Some(page)).collect()
    }

    /// Whether an object is drawn in the given viewport (page space vs model space, per-detail hiding).
    pub fn object_in_viewport(&self, vp: usize, o: &tri_doc::Object) -> bool {
        let v = &self.viewports[vp];
        let space = o.attributes.user_text.iter().find(|(k, _)| k == "Space").map(|(_, s)| s.as_str());
        match (&v.page, v.detail_of) {
            (Some(_), _) => space == Some(v.name.as_str()),
            (None, Some(_)) => space.is_none() && !v.detail_hidden.contains(&o.id) && !v.detail_hidden_layers.contains(&o.attributes.layer),
            _ => space.is_none(),
        }
    }

    /// Keep the per-viewport batch vectors in step with the viewport list.
    fn sync_batch_vectors(&mut self) {
        let n = self.viewports.len();
        self.scene_batches.resize_with(n, Default::default);
        self.overlay_batches.resize_with(n, Default::default);
        self.scene_keys.resize(n, None);
    }

    fn page_camera(w: f64, h: f64) -> Camera {
        let mut cam = Camera::top();
        cam.target = DVec3::new(w * 0.5, h * 0.5, 0.0);
        cam.ortho_height = h * 1.15;
        cam.distance = (w + h).max(1.0);
        cam
    }

    /// Layout operations: "new" (name;w;h;details), "delete", "rename:<new>", "activate", "size:w;h",
    /// "detail:x;y;w;h", "copy", "hide_in_detail", "show_in_detail", "show_selected_in_detail",
    /// "hide_layers:<names>", "show_layers:<names>", "detail_to_viewport:<name>", "viewport_to_detail:<name>",
    /// "sel_detail", "list", "change_space:<Model|Page>".
    pub fn layout_op(&mut self, op: &str, name: &str) {
        let (cmd, arg) = op.split_once(':').unwrap_or((op, ""));
        match cmd {
            "new" => {
                let parts: Vec<&str> = arg.split(';').collect();
                let w: f64 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(297.0);
                let h: f64 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(210.0);
                let details: usize = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
                let mut n = 1;
                let mut page_name = if name.is_empty() { format!("Page {n}") } else { name.to_string() };
                while self.viewports.iter().any(|v| v.name.eq_ignore_ascii_case(&page_name)) {
                    n += 1;
                    page_name = format!("Page {n}");
                }
                let mut v = Viewport::new(&page_name, Self::page_camera(w, h), DisplayMode::Wireframe);
                v.page = Some(PageLayout { width: w, height: h });
                v.rect = self.viewports[self.active].rect;
                self.viewports.push(v);
                let page = self.viewports.len() - 1;
                self.viewport_tab_order.push(page);
                // initial details: one big detail, or a grid of up to four
                let margin = (w.min(h)) * 0.05;
                let rects: Vec<[f64; 4]> = match details {
                    0 => vec![],
                    1 => vec![[margin, margin, w - 2.0 * margin, h - 2.0 * margin]],
                    2 => vec![[margin, margin, (w - 3.0 * margin) * 0.5, h - 2.0 * margin], [margin * 2.0 + (w - 3.0 * margin) * 0.5, margin, (w - 3.0 * margin) * 0.5, h - 2.0 * margin]],
                    _ => {
                        let (cw, ch) = ((w - 3.0 * margin) * 0.5, (h - 3.0 * margin) * 0.5);
                        vec![[margin, margin * 2.0 + ch, cw, ch], [margin * 2.0 + cw, margin * 2.0 + ch, cw, ch], [margin, margin, cw, ch], [margin * 2.0 + cw, margin, cw, ch]]
                    }
                };
                let cams = [Camera::top(), Camera::front(), Camera::right(), Camera::perspective()];
                for (k, r) in rects.into_iter().enumerate() {
                    self.add_detail(page, r, cams[k % 4].clone());
                }
                self.sync_batch_vectors();
                self.active = page;
                self.layout = Layout::Single;
                self.log.push(format!("Layout \"{page_name}\" created ({} x {} {}).", fmt_num_app(w), fmt_num_app(h), self.doc.settings.units));
            }
            "detail" => {
                let Some(page) = self.active_page() else {
                    self.log.push("Detail works on a page layout (Layout command).".into());
                    return;
                };
                let parts: Vec<f64> = arg.split(';').filter_map(|s| s.parse().ok()).collect();
                if parts.len() < 4 {
                    return;
                }
                let cam = self.viewports.iter().find(|v| v.name == "Perspective").map(|v| v.camera.clone()).unwrap_or_else(Camera::perspective);
                self.add_detail(page, [parts[0], parts[1], parts[2], parts[3]], cam);
                self.sync_batch_vectors();
                self.active = page;
            }
            "delete" => {
                let Some(page) = self.find_page(name) else { return };
                let mut remove: Vec<usize> = self.details_of(page);
                remove.push(page);
                remove.sort_unstable_by(|a, b| b.cmp(a));
                for idx in remove {
                    self.remove_viewport(idx);
                }
                self.sync_batch_vectors();
                self.active = 0;
                self.layout = Layout::Four;
            }
            "rename" => {
                let Some(page) = self.find_page(name) else { return };
                if !arg.trim().is_empty() {
                    let old = self.viewports[page].name.clone();
                    self.viewports[page].name = arg.trim().to_string();
                    let ids: Vec<ObjectId> = self.doc.objects().filter(|o| o.attributes.user_text.iter().any(|(k, v)| k == "Space" && *v == old)).map(|o| o.id).collect();
                    for id in ids {
                        if let Some(o) = self.doc.get_mut(id) {
                            for (k, v) in o.attributes.user_text.iter_mut() {
                                if k == "Space" {
                                    *v = arg.trim().to_string();
                                }
                            }
                        }
                    }
                    self.doc.touch_revision();
                }
            }
            "activate" => {
                let Some(page) = self.find_page(name) else {
                    self.log.push(format!("No layout named \"{name}\"."));
                    return;
                };
                self.active = page;
                self.layout = Layout::Single;
            }
            "size" => {
                let Some(page) = self.active_page() else { return };
                let parts: Vec<f64> = arg.split(';').filter_map(|s| s.parse().ok()).collect();
                if parts.len() >= 2 {
                    self.viewports[page].page = Some(PageLayout { width: parts[0], height: parts[1] });
                    self.viewports[page].camera = Self::page_camera(parts[0], parts[1]);
                    self.doc.touch_revision();
                }
            }
            "copy" => {
                let Some(page) = self.active_page() else { return };
                let src = self.viewports[page].clone();
                let mut n = 2;
                let mut new_name = format!("{} ({})", src.name, n);
                while self.viewports.iter().any(|v| v.name == new_name) {
                    n += 1;
                    new_name = format!("{} ({})", src.name, n);
                }
                let mut copy = src.clone();
                copy.name = new_name.clone();
                self.viewports.push(copy);
                let np = self.viewports.len() - 1;
                self.viewport_tab_order.push(np);
                for d in self.details_of(page) {
                    let mut dv = self.viewports[d].clone();
                    dv.detail_of = Some(np);
                    self.viewports.push(dv);
                }
                // copy page-space objects
                let objs: Vec<(Geometry, tri_doc::Attributes)> = self.doc.objects().filter(|o| o.attributes.user_text.iter().any(|(k, v)| k == "Space" && *v == src.name)).map(|o| (o.geometry.clone(), o.attributes.clone())).collect();
                self.doc.begin_undo("CopyLayout");
                for (g, mut attrs) in objs {
                    for (k, v) in attrs.user_text.iter_mut() {
                        if k == "Space" {
                            *v = new_name.clone();
                        }
                    }
                    self.doc.add_with(g, attrs);
                }
                self.sync_batch_vectors();
                self.active = np;
                self.log.push(format!("Layout copied to \"{new_name}\"."));
            }
            "hide_in_detail" | "show_in_detail" | "show_selected_in_detail" => {
                let Some(detail) = self.active_detail() else {
                    self.log.push("Activate a detail first (click inside it on the page).".into());
                    return;
                };
                let ids = self.doc.selected_ids();
                let v = &mut self.viewports[detail];
                match cmd {
                    "hide_in_detail" => {
                        for id in ids {
                            if !v.detail_hidden.contains(&id) {
                                v.detail_hidden.push(id);
                            }
                        }
                    }
                    "show_selected_in_detail" => v.detail_hidden.retain(|id| !ids.contains(id)),
                    _ => v.detail_hidden.clear(),
                }
                self.doc.clear_selection();
                self.doc.touch_revision();
            }
            "hide_layers" | "show_layers" => {
                let Some(detail) = self.active_detail() else {
                    self.log.push("Activate a detail first.".into());
                    return;
                };
                let idxs: Vec<usize> = arg.split(';').filter_map(|n| self.doc.find_layer(n.trim())).collect();
                let v = &mut self.viewports[detail];
                if cmd == "hide_layers" {
                    for i in idxs {
                        if !v.detail_hidden_layers.contains(&i) {
                            v.detail_hidden_layers.push(i);
                        }
                    }
                } else if idxs.is_empty() {
                    v.detail_hidden_layers.clear();
                } else {
                    v.detail_hidden_layers.retain(|i| !idxs.contains(i));
                }
                self.doc.touch_revision();
            }
            "detail_to_viewport" | "viewport_to_detail" => {
                let Some(detail) = self.active_detail() else {
                    self.log.push("Activate a detail first.".into());
                    return;
                };
                let Some(other) = self.viewports.iter().position(|v| v.name.eq_ignore_ascii_case(arg.trim()) && !v.is_page() && !v.is_detail()) else {
                    self.log.push(format!("No model viewport named \"{}\".", arg.trim()));
                    return;
                };
                if cmd == "detail_to_viewport" {
                    self.viewports[other].camera = self.viewports[detail].camera.clone();
                    self.viewports[other].display = self.viewports[detail].display;
                } else {
                    self.viewports[detail].camera = self.viewports[other].camera.clone();
                    self.viewports[detail].display = self.viewports[other].display;
                }
            }
            "sel_detail" => {
                let Some(page) = self.active_page() else { return };
                let details = self.details_of(page);
                let names: Vec<String> = details.iter().map(|d| format!("{} [{}]", self.viewports[*d].name, fmt_rect(self.viewports[*d].detail_rect))).collect();
                self.log.push(if names.is_empty() { "No details on this page.".into() } else { format!("Details: {}", names.join(", ")) });
                if let Some(first) = details.first() {
                    self.active = *first;
                }
            }
            "list" => {
                let names: Vec<String> = self.viewports.iter().filter(|v| v.is_page()).map(|v| { let p = v.page.as_ref().unwrap(); format!("{} ({} x {})", v.name, fmt_num_app(p.width), fmt_num_app(p.height)) }).collect();
                self.log.push(if names.is_empty() { "No layouts (use Layout to create one).".into() } else { format!("Layouts: {}", names.join(", ")) });
            }
            "change_space" => {
                let ids = self.doc.selected_ids();
                let target_page = if arg == "Model" { None } else { self.active_page().map(|p| self.viewports[p].name.clone()) };
                if arg != "Model" && target_page.is_none() {
                    self.log.push("Activate a page layout to move objects into page space.".into());
                    return;
                }
                self.doc.begin_undo("ChangeSpace");
                for id in &ids {
                    if let Some(o) = self.doc.get_mut(*id) {
                        o.attributes.user_text.retain(|(k, _)| k != "Space");
                        if let Some(p) = &target_page {
                            o.attributes.user_text.push(("Space".into(), p.clone()));
                        }
                    }
                }
                self.doc.touch_revision();
                self.log.push(format!("{} objects moved to {} space.", ids.len(), if target_page.is_some() { "page" } else { "model" }));
            }
            _ => {}
        }
    }

    fn add_detail(&mut self, page: usize, rect: [f64; 4], camera: Camera) -> usize {
        let n = self.details_of(page).len() + 1;
        let mut v = Viewport::new(&format!("Detail {n}"), camera, DisplayMode::Wireframe);
        let bb = self.doc.bounding_box();
        let bb = if bb.is_empty() { BoundingBox::new(DVec3::splat(-12.0), DVec3::splat(12.0)) } else { bb.inflate(bb.diagonal() * 0.1 + 0.5) };
        v.camera.zoom_to(&bb);
        v.detail_of = Some(page);
        v.detail_rect = rect;
        v.rect = self.viewports[page].rect;
        self.viewports.push(v);
        self.viewports.len() - 1
    }

    fn remove_viewport(&mut self, idx: usize) {
        self.viewports.remove(idx);
        self.viewport_tab_order.retain(|v| *v != idx);
        for v in self.viewport_tab_order.iter_mut() {
            if *v > idx {
                *v -= 1;
            }
        }
        for v in self.viewports.iter_mut() {
            if let Some(p) = v.detail_of {
                if p > idx {
                    v.detail_of = Some(p - 1);
                }
            }
        }
        if self.active >= self.viewports.len() {
            self.active = 0;
        }
    }

    pub fn active_page(&self) -> Option<usize> {
        let v = &self.viewports[self.active];
        if v.is_page() { Some(self.active) } else { v.detail_of }
    }

    pub fn active_detail(&self) -> Option<usize> {
        if self.viewports[self.active].is_detail() { Some(self.active) } else { None }
    }

    fn find_page(&self, name: &str) -> Option<usize> {
        if name.is_empty() {
            return self.active_page();
        }
        self.viewports.iter().position(|v| v.is_page() && v.name.eq_ignore_ascii_case(name))
    }

    /// Screen rectangle of a detail inside its page viewport.
    pub fn detail_screen_rect(&self, detail: usize) -> Option<ViewRect> {
        let d = &self.viewports[detail];
        let page = &self.viewports[d.detail_of?];
        let [x, y, w, h] = d.detail_rect;
        let (a, _) = page.camera.project(&page.rect, DVec3::new(x, y, 0.0))?;
        let (b, _) = page.camera.project(&page.rect, DVec3::new(x + w, y + h, 0.0))?;
        Some(ViewRect { x: a.x.min(b.x), y: a.y.min(b.y), w: (a.x - b.x).abs().max(1.0), h: (a.y - b.y).abs().max(1.0) })
    }

    pub fn cplane(&self) -> Plane {
        self.viewports[self.active].cplane()
    }

    fn with_ctx<R>(&mut self, f: impl FnOnce(&mut CommandRunner, &mut CmdCtx) -> R) -> R {
        let cplane = self.viewports[self.active].cplane();
        let view_dir = self.viewports[self.active].camera.view_dir();
        let mut ctx = CmdCtx {
            doc: &mut self.doc,
            log: &mut self.log,
            cplane,
            view_dir,
            view_eye: self.viewports[self.active].camera.eye(),
            requests: &mut self.requests,
            memory: &mut self.memory,
        };
        let slow = std::env::var_os("TRICERATOPS_SLOW").is_some().then(std::time::Instant::now);
        let r = f(&mut self.runner, &mut ctx);
        if let Some(t) = slow {
            if t.elapsed() > std::time::Duration::from_secs(1) {
                eprintln!("slow step: {} {} ms", self.runner.last_command.as_deref().unwrap_or("?"), t.elapsed().as_millis());
            }
        }
        r
    }

    // ----- background jobs -------------------------------------------------

    /// The job the running command waits for, as the status bar shows it.
    pub fn job_status(&self) -> Option<crate::jobs::JobStatus> {
        let j = self.runner.job()?;
        let (stage, count) = j.token.snapshot();
        Some(crate::jobs::JobStatus { label: j.label.clone(), elapsed: j.started.elapsed(), stage, count, cancelling: j.cancelled.is_some() })
    }

    /// Stop the running job (Esc does this through `cancel`). False when none runs.
    pub fn cancel_job(&mut self, reason: &str) -> bool {
        self.pending_lines.clear();
        self.runner.cancel_job(reason)
    }

    /// Wait for the running job and the script lines queued behind it (tests and headless use).
    #[cfg(test)]
    pub fn settle(&mut self) {
        while self.runner.job().is_some() || !self.pending_lines.is_empty() || self.locust.solving().is_some() {
            let polled = self.poll_job() | self.locust.poll();
            if !polled {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
    }

    /// Called every frame: when the job has ended, apply its result and run the script lines
    /// that waited for it. True when a job ended.
    pub fn poll_job(&mut self) -> bool {
        if !self.runner.job().is_some_and(|j| j.is_finished()) {
            return false;
        }
        let before: Vec<ObjectId> = self.doc.objects().map(|o| o.id).collect();
        let was = self.assistant_mode;
        self.assistant_mode = self.assistant_owns_command;
        self.with_ctx(|r, ctx| r.poll_job(ctx));
        self.after_command_step(&before);
        self.process_requests();
        self.assistant_mode = was;
        if self.runner.job().is_none() && !self.pending_lines.is_empty() {
            let lines = std::mem::take(&mut self.pending_lines);
            self.assistant_mode = self.pending_assistant;
            self.run_script_lines(&lines);
            self.assistant_mode = was;
        }
        true
    }

    // ----- command line ---------------------------------------------------

    /// Start a command from a button or menu (echoed with a leading underscore like Triceratops macros).
    pub fn start_command(&mut self, name: &str) {
        let typed = format!("_{name}");
        self.start_command_typed(name, &typed);
    }

    fn start_command_typed(&mut self, name: &str, typed: &str) {
        if let Some(j) = self.runner.job() {
            // the running job keeps its owner (an assistant's stays the assistant's)
            let msg = format!("{} is running — wait for it or press Esc to cancel it.", j.label);
            self.log.push(msg);
            return;
        }
        // remember who started it, so an assistant never feeds input into the user's command
        self.assistant_owns_command = self.assistant_mode;
        if self.assistant_mode {
            // assistants name objects explicitly (#id, last): a leftover selection would be
            // taken as preselection and shift every input to the following prompt
            self.doc.clear_selection();
        }
        let before: Vec<ObjectId> = self.doc.objects().map(|o| o.id).collect();
        self.pick_order = self.doc.selected_ids();
        self.constraints = Constraints::default();
        let name = name.to_string();
        let typed = typed.to_string();
        let pre = self.pick_order.clone();
        self.with_ctx(|r, ctx| r.start(&name, &typed, ctx, &pre));
        self.after_command_step(&before);
    }

    /// Feed an input to the active command as if it came from the viewport (used by scripts and tests).
    pub fn feed_input(&mut self, input: Input) {
        let before: Vec<ObjectId> = self.doc.objects().map(|o| o.id).collect();
        self.with_ctx(|r, ctx| r.feed(input, ctx));
        self.after_command_step(&before);
    }

    /// Replay history records and cage deformations whose parents are among `changed`.
    pub fn update_dependents(&mut self, changed: &[ObjectId]) {
        if self.replaying || changed.is_empty() {
            return;
        }
        // cages: captives follow their cage points
        let cages = self.doc.state.history.cages.clone();
        for c in cages.iter().filter(|c| c.points.iter().any(|p| changed.contains(p))) {
            self.apply_cage(c);
        }
        let records: Vec<tri_doc::history::HistoryRecord> = self.doc.state.history.records.iter().filter(|r| r.enabled && r.parents().iter().any(|p| changed.contains(p)) && !r.outputs.iter().any(|o| changed.contains(o))).cloned().collect();
        for r in records {
            self.replay_record(&r);
        }
    }

    fn apply_cage(&mut self, c: &tri_doc::history::CageRecord) {
        let current: Vec<DVec3> = c.points.iter().filter_map(|id| self.doc.get(*id)).filter_map(|o| if let Geometry::Point(p) = o.geometry { Some(p) } else { None }).collect();
        if current.len() != c.points.len() {
            return;
        }
        for (id, original) in &c.captives {
            if self.doc.get(*id).is_some() {
                self.doc.replace_geometry(*id, c.deform_geometry(&current, original));
            }
        }
    }

    /// Replay one history record: delete its children and run the command again with the same inputs.
    pub fn replay_record(&mut self, r: &tri_doc::history::HistoryRecord) {
        if r.parents().iter().any(|p| self.doc.get(*p).is_none()) {
            return;
        }
        self.replaying = true;
        let attrs: Vec<tri_doc::Attributes> = r.outputs.iter().filter_map(|id| self.doc.get(*id).map(|o| o.attributes.clone())).collect();
        for id in &r.outputs {
            self.doc.remove(*id);
        }
        let before: Vec<ObjectId> = self.doc.objects().map(|o| o.id).collect();
        let saved_sel = self.doc.selected_ids();
        let name = r.command.clone();
        let inputs: Vec<Input> = r.inputs.iter().map(|h| match h {
            tri_doc::history::HistInput::Point(p) => Input::Point(*p),
            tri_doc::history::HistInput::Objects(v) => Input::Objects(v.clone()),
            tri_doc::history::HistInput::Pick(id, p) => Input::Pick(*id, *p),
            tri_doc::history::HistInput::Number(n) => Input::Number(*n),
            tri_doc::history::HistInput::Text(t) => Input::Text(t.clone()),
            tri_doc::history::HistInput::Option(o) => Input::Option(o.clone()),
            tri_doc::history::HistInput::Empty => Input::Empty,
        }).collect();
        let typed = format!("_{name}");
        self.with_ctx(|run, ctx| {
            run.start(&name, &typed, ctx, &[]);
            for inp in inputs {
                if !run.is_active() {
                    break;
                }
                run.feed(inp, ctx);
                // replay is not interactive: long work runs inline
                run.wait_job(ctx);
            }
            if run.is_active() {
                run.cancel(ctx);
            }
        });
        let added: Vec<ObjectId> = self.doc.objects().map(|o| o.id).filter(|id| !before.contains(id)).collect();
        for (k, id) in added.iter().enumerate() {
            if let (Some(a), Some(o)) = (attrs.get(k), self.doc.get_mut(*id)) {
                o.attributes = a.clone();
            }
        }
        if let Some(rec) = self.doc.state.history.records.iter_mut().find(|x| x.id == r.id) {
            rec.outputs = added;
        }
        self.doc.clear_selection();
        for id in saved_sel {
            self.doc.select(id, true);
        }
        self.replaying = false;
    }

    /// Re-solve the parametric constraints after an edit (parametric mode), keeping `keep` fixed.
    pub fn resolve_constraints(&mut self, keep: &[ObjectId]) {
        if !self.doc.state.parametric.enabled || self.doc.state.parametric.constraints.is_empty() {
            return;
        }
        match self.doc.solve_constraints(keep) {
            Ok(o) => {
                for m in o.messages {
                    self.log.push(m);
                }
            }
            Err(e) => self.log.push(format!("Constraint solve failed: {e}")),
        }
    }

    /// Track objects added by the last command (SelLast) and keep the pick order in sync.
    fn after_command_step(&mut self, before: &[ObjectId]) {
        if !self.runner.is_active() {
            let added: Vec<ObjectId> = self.doc.objects().map(|o| o.id).filter(|id| !before.contains(id)).collect();
            if !added.is_empty() {
                if self.viewports[self.active].is_page() {
                    let page_name = self.viewports[self.active].name.clone();
                    for id in &added {
                        if let Some(o) = self.doc.get_mut(*id) {
                            if !o.attributes.user_text.iter().any(|(k, _)| k == "Space") {
                                o.attributes.user_text.push(("Space".into(), page_name.clone()));
                            }
                        }
                    }
                }
                self.last_added = added.clone();
                self.memory.insert("SelLast".into(), added.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","));
                if self.doc.state.parametric.enabled && self.memory.get("Parametric.Infer").is_some_and(|v| v == "Yes") {
                    let n = self.doc.auto_constrain(&added).len();
                    if n > 0 {
                        self.log.push(format!("{n} constraints inferred."));
                    }
                }
            }
            let mut touched: Vec<ObjectId> = Vec::new();
            if let Some((name, inputs)) = self.runner.finished.take() {
                touched = inputs.iter().flat_map(|i| match i { Input::Objects(v) => v.clone(), Input::Pick(id, _) => vec![*id], _ => Vec::new() }).collect();
                if self.aids.record_history && !self.replaying && !added.is_empty() {
                    let hist: Vec<tri_doc::history::HistInput> = inputs.iter().filter_map(|i| match i {
                        Input::Point(p) => Some(tri_doc::history::HistInput::Point(*p)),
                        Input::Objects(v) => Some(tri_doc::history::HistInput::Objects(v.clone())),
                        Input::Pick(id, p) => Some(tri_doc::history::HistInput::Pick(*id, *p)),
                        Input::Number(n) => Some(tri_doc::history::HistInput::Number(*n)),
                        Input::Text(t) => Some(tri_doc::history::HistInput::Text(t.clone())),
                        Input::Option(o) => Some(tri_doc::history::HistInput::Option(o.clone())),
                        Input::Empty => Some(tri_doc::history::HistInput::Empty),
                        Input::Cancel => None,
                    }).collect();
                    let parents: Vec<ObjectId> = hist.iter().flat_map(|h| h.object_ids()).collect();
                    let ok = !parents.is_empty() && parents.iter().all(|p| self.doc.get(*p).is_some() && !added.contains(p));
                    if ok {
                        let id = self.doc.state.history.next_id();
                        self.doc.state.history.records.push(tri_doc::history::HistoryRecord { id, command: name.clone(), inputs: hist, outputs: added.clone(), enabled: true });
                        self.log.push(format!("History recorded for {name}."));
                    }
                }
            }
            let keep: Vec<ObjectId> = if added.is_empty() { self.doc.selected_ids() } else { added };
            self.resolve_constraints(&keep);
            let mut changed = keep.clone();
            changed.extend(touched);
            changed.sort_unstable();
            changed.dedup();
            self.update_dependents(&changed);
            self.constraints = Constraints::default();
            self.one_shot_snap = None;
        }
        self.sync_pick_order();
    }

    fn sync_pick_order(&mut self) {
        let sel = &self.doc.selection;
        self.pick_order.retain(|id| sel.contains(id));
        for id in sel.iter() {
            if !self.pick_order.contains(id) {
                self.pick_order.push(*id);
            }
        }
    }

    /// Enter / Space pressed on the command line (or right mouse button in a viewport).
    pub fn submit(&mut self) {
        let text = std::mem::take(&mut self.cmd_text).trim().to_string();
        self.autocomplete_index = None;
        if let Some(j) = self.runner.job() {
            let msg = format!("{} is running — wait for it or press Esc to cancel it.", j.label);
            self.log.push(msg);
            return;
        }
        if self.runner.is_active() {
            if text.is_empty() {
                self.submit_enter();
            } else {
                self.feed_text(&text);
            }
            return;
        }
        if text.is_empty() {
            if let Some(last) = self.runner.last_command.clone() {
                self.start_command_typed(&last, &last);
            }
            return;
        }
        match commands::find_command(&text) {
            Some(info) => self.start_command_typed(info.name, &text),
            None => {
                // autocomplete: unique prefix
                let (exact, _) = commands::lookup(&text, 50);
                if exact.len() == 1 {
                    let n = exact[0].name;
                    self.start_command_typed(n, &text);
                } else {
                    self.log.push(format!("Command: {text}"));
                    self.log.push(format!("Unknown command: {text}"));
                }
            }
        }
    }

    pub fn submit_enter(&mut self) {
        let before: Vec<ObjectId> = self.doc.objects().map(|o| o.id).collect();
        if let Some(p) = self.runner.prompt().cloned() { match p.kind {
            PromptKind::Objects { min, .. } => {
                self.sync_pick_order();
                let ids = self.pick_order.clone();
                if ids.len() < min {
                    if ids.is_empty() {
                        self.log.push("No objects selected.".into());
                    } else {
                        self.log.push(format!("Select at least {min} objects."));
                    }
                } else {
                    self.with_ctx(|r, ctx| r.feed(Input::Objects(ids), ctx));
                }
            }
            PromptKind::Number | PromptKind::PointOrNumber => {
                if let Some(d) = p.default.as_deref().and_then(parse_number) {
                    self.with_ctx(|r, ctx| r.feed(Input::Number(d), ctx));
                } else {
                    self.with_ctx(|r, ctx| r.feed(Input::Empty, ctx));
                }
            }
            PromptKind::Text => {
                if let Some(d) = p.default.clone() {
                    self.with_ctx(|r, ctx| r.feed(Input::Text(d), ctx));
                } else {
                    self.with_ctx(|r, ctx| r.feed(Input::Empty, ctx));
                }
            }
            _ => self.with_ctx(|r, ctx| r.feed(Input::Empty, ctx)),
        } }
        self.after_command_step(&before);
    }

    fn feed_text(&mut self, text: &str) {
        let before: Vec<ObjectId> = self.doc.objects().map(|o| o.id).collect();
        let cplane = self.cplane();
        let base = self.runner.base_point();
        let Some(prompt) = self.runner.prompt().cloned() else { return };
        // options first: "Name" or "Name=Value"
        let (oname, ovalue) = match text.split_once('=') {
            Some((n, v)) => (n.trim(), Some(v.trim())),
            None => (text, None),
        };
        if let Some(o) = match_option(&prompt.options, oname) {
            let n = o.name.clone();
            self.with_ctx(|r, ctx| r.feed_option(&n, ovalue, ctx));
            self.after_command_step(&before);
            return;
        }
        self.runner.set_answer(text);
        let input = match prompt.kind {
            PromptKind::Point | PromptKind::PointOrNumber => {
                match parse_point_input(text, &cplane, base) {
                    Some(PointInput::Point(p)) => Some(Input::Point(p)),
                    Some(PointInput::Distance(d)) => {
                        if prompt.kind == PromptKind::PointOrNumber {
                            Some(Input::Number(d))
                        } else if base.is_some() {
                            // distance constraint: if the cursor is in a viewport, finish the pick now
                            if let Some(c) = self.cursor {
                                let b = base.unwrap();
                                let dir = (c.world - b).normalize_or(cplane.x_axis);
                                Some(Input::Point(b + dir * d.abs()))
                            } else {
                                self.constraints.distance = Some(d.abs());
                                self.log.push(format!("Distance constraint: {}", commands::fmt_num(d.abs())));
                                None
                            }
                        } else if let Some(e) = self.constraints.elevator {
                            Some(Input::Point(e + cplane.z_axis * d))
                        } else {
                            Some(Input::Text(text.to_string()))
                        }
                    }
                    Some(PointInput::Angle(a)) => {
                        self.constraints.angle_deg = Some(a);
                        self.log.push(format!("Angle constraint: {}", commands::fmt_num(a)));
                        None
                    }
                    Some(PointInput::Polar { distance, angle_deg }) => {
                        let b = base.unwrap_or(cplane.origin);
                        let (s, c) = angle_deg.to_radians().sin_cos();
                        Some(Input::Point(b + cplane.x_axis * (distance * c) + cplane.y_axis * (distance * s)))
                    }
                    None => {
                        self.log.push(format!("Unknown command: {text}"));
                        None
                    }
                }
            }
            PromptKind::Number => match parse_number(text) {
                Some(n) => Some(Input::Number(n)),
                None => {
                    self.log.push(format!("Unknown command: {text}"));
                    None
                }
            },
            PromptKind::Objects { .. } | PromptKind::ObjectPick { .. } if text.starts_with('#') || text.eq_ignore_ascii_case("last") => {
                // scripted object references: "#12", "#12,#13", "#12@1,2,3" (pick point), "last"
                let mut ids: Vec<ObjectId> = Vec::new();
                let mut pick_pt: Option<DVec3> = None;
                if text.eq_ignore_ascii_case("last") {
                    ids = self.last_added.clone();
                } else {
                    for part in text.split(',').map(|s| s.trim()) {
                        let (idtxt, at) = match part.split_once('@') { Some((i, p)) => (i, Some(p)), None => (part, None) };
                        if let Ok(id) = idtxt.trim_start_matches('#').parse::<ObjectId>() {
                            if self.doc.get(id).is_some() {
                                ids.push(id);
                            }
                        }
                        if let Some(p) = at {
                            let nums: Vec<f64> = p.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                            if nums.len() == 3 {
                                pick_pt = Some(DVec3::new(nums[0], nums[1], nums[2]));
                            }
                        }
                        if at.is_some() {
                            break;
                        }
                    }
                    // "#12@1,2,3" splits on ',' above; re-parse the point from the whole text
                    if let Some((_, p)) = text.split_once('@') {
                        let nums: Vec<f64> = p.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                        if nums.len() == 3 {
                            pick_pt = Some(DVec3::new(nums[0], nums[1], nums[2]));
                        }
                    }
                }
                if ids.is_empty() {
                    self.log.push(format!("No object matches {text}"));
                    None
                } else if matches!(prompt.kind, PromptKind::ObjectPick { .. }) {
                    let id = ids[0];
                    let p = pick_pt.unwrap_or_else(|| self.doc.get(id).map(|o| self.doc.object_bbox(o).center()).unwrap_or(DVec3::ZERO));
                    Some(Input::Pick(id, p))
                } else {
                    Some(Input::Objects(ids))
                }
            }
            PromptKind::Objects { .. } | PromptKind::ObjectPick { .. } | PromptKind::Text | PromptKind::Options => {
                // a number typed at an object prompt (Extend, Offset) is delivered as a number
                match parse_number(text) {
                    Some(n) if matches!(prompt.kind, PromptKind::Objects { .. } | PromptKind::ObjectPick { .. }) => Some(Input::Number(n)),
                    _ => Some(Input::Text(text.to_string())),
                }
            }
        };
        if let Some(input) = input {
            self.with_ctx(|r, ctx| r.feed(input, ctx));
        }
        self.after_command_step(&before);
    }

    /// A command-line option was clicked.
    pub fn option_clicked(&mut self, name: &str) {
        let before: Vec<ObjectId> = self.doc.objects().map(|o| o.id).collect();
        let n = name.to_string();
        self.with_ctx(|r, ctx| r.feed_option(&n, None, ctx));
        self.after_command_step(&before);
    }

    pub fn cancel(&mut self) {
        self.cmd_text.clear();
        self.autocomplete_index = None;
        self.selection_menu = None;
        self.window_drag = None;
        self.drag_move = None;
        self.popup_toolbar = None;
        if self.runner.job().is_some() {
            let reason = if self.assistant_mode { "cancelled" } else { "cancelled by the user" };
            self.cancel_job(reason);
        } else if self.locust.cancel_solve() {
            // Esc stopped the Locust solve
        } else if self.runner.is_active() {
            self.with_ctx(|r, ctx| r.cancel(ctx));
            self.constraints = Constraints::default();
        } else if self.any_grip_selected() || !self.doc.selection.is_empty() {
            for g in self.grips.iter_mut() {
                g.selected = false;
            }
            self.deselect_all();
        } else if !self.grips.is_empty() {
            // a second Esc turns control points off, as in Triceratops
            self.points_on.clear();
            self.grips.clear();
        } else {
            self.deselect_all();
        }
    }

    fn deselect_all(&mut self) {
        if !self.doc.selection.is_empty() {
            self.last_selection = self.doc.selected_ids();
        }
        self.doc.clear_selection();
        self.pick_order.clear();
    }

    pub fn delete_key(&mut self) {
        if !self.runner.is_active() && !self.doc.selection.is_empty() {
            self.start_command("Delete");
        }
    }

    pub fn tab_lock(&mut self) {
        if let (Some(b), Some(c)) = (self.runner.base_point(), self.cursor) {
            if self.constraints.direction.is_none() {
                let d = (c.world - b).normalize_or(DVec3::X);
                self.constraints.direction = Some(d);
            } else {
                self.constraints.direction = None;
            }
        }
    }

    // ----- picking ----------------------------------------------------------

    fn wants_point(&self) -> bool {
        self.runner.prompt().is_some_and(|p| p.wants_point())
    }

    fn wants_objects(&self) -> bool {
        self.runner.prompt().is_some_and(|p| p.wants_objects())
    }

    pub fn can_select(&self) -> bool {
        !self.runner.is_active() || self.wants_objects()
    }

    /// Compute the constrained / snapped world point under the cursor.
    pub fn snapped_point(&self, vp: usize, pos: DVec2) -> (DVec3, Option<SnapKind>) {
        let v = &self.viewports[vp];
        let cam = &v.camera;
        let rect = &v.rect;
        let cplane = v.cplane();
        let base = self.runner.base_point();
        let ray = cam.ray_at(rect, pos.x, pos.y);

        // elevator mode: vertical line through the elevator base
        if let Some(e) = self.constraints.elevator {
            let (_, _, p) = ray.closest_to_segment(e - cplane.z_axis * 1e6, e + cplane.z_axis * 1e6);
            let mut p = p;
            if self.aids.grid_snap {
                let s = self.doc.settings.grid_snap_spacing.max(1e-9);
                let w = (p - e).dot(cplane.z_axis);
                p = e + cplane.z_axis * ((w / s).round() * s);
            }
            return (p, None);
        }

        // object snaps, including the points of what the command has drawn so far
        let mut near_hit = None;
        if !self.alt_down {
            let pending = self.pending_snaps();
            let q = SnapQuery {
                cache: &self.scene,
                doc: &self.doc,
                cam,
                rect,
                cursor: pos,
                base,
                cplane: &cplane,
                settings: &self.aids.osnap,
                one_shot: self.one_shot_snap,
                pending: &pending,
            };
            if let Some(h) = q.find() {
                let mut p = h.point;
                if let (Some(d), Some(b)) = (self.constraints.distance, base) {
                    p = b + (p - b).normalize_or(cplane.x_axis) * d;
                }
                // a point on a curve gives way to a SmartTrack line or intersection
                if h.kind != SnapKind::Near {
                    return (p, Some(h.kind));
                }
                near_hit = Some((p, h.kind));
            }
        }
        if self.constraints.distance.is_none() && self.constraints.direction.is_none() && self.constraints.angle_deg.is_none() {
            if let Some((p, _)) = self.smart_track(vp, pos) {
                return (p, None);
            }
        }
        if let Some((p, k)) = near_hit {
            return (p, Some(k));
        }

        // construction plane
        let mut p = match cplane.intersect_ray(&ray) {
            Some(t) if t > 0.0 || cam.projection == Projection::Parallel => ray.at(t),
            _ => {
                let fallback = Plane::from_normal(cam.target, cam.view_dir());
                fallback.intersect_ray(&ray).map_or(cam.target, |t| ray.at(t))
            }
        };
        if self.aids.planar {
            if let Some(b) = base {
                let w = (b - cplane.origin).dot(cplane.z_axis);
                let (u, vv, _) = cplane.closest_parameters(p);
                p = cplane.point_at_3(u, vv, w);
            }
        }
        if let Some(b) = base {
            if let Some(dir) = self.constraints.direction {
                let d = (p - b).dot(dir);
                p = b + dir * d;
            } else if let Some(a) = self.constraints.angle_deg {
                let (s, c) = a.to_radians().sin_cos();
                let dir = cplane.x_axis * c + cplane.y_axis * s;
                let d = (p - b).dot(dir);
                p = b + dir * d;
            } else if self.aids.ortho != self.shift_down {
                let d = p - b;
                let (du, dv, dw) = (d.dot(cplane.x_axis), d.dot(cplane.y_axis), d.dot(cplane.z_axis));
                let step = self.aids.ortho_angle.to_radians().max(1e-6);
                let ang = (dv.atan2(du) / step).round() * step;
                let len = (du * du + dv * dv).sqrt();
                let (s, c) = ang.sin_cos();
                p = b + cplane.x_axis * (len * c) + cplane.y_axis * (len * s) + cplane.z_axis * dw;
            }
            if let Some(dist) = self.constraints.distance {
                p = b + (p - b).normalize_or(cplane.x_axis) * dist;
                return (p, None);
            }
        }
        if self.aids.grid_snap && self.constraints.angle_deg.is_none() && self.constraints.direction.is_none() {
            let s = self.doc.settings.grid_snap_spacing.max(1e-9);
            let (u, w, z) = cplane.closest_parameters(p);
            p = cplane.point_at_3((u / s).round() * s, (w / s).round() * s, (z / s).round() * s);
        }
        (p, None)
    }

    pub fn update_cursor(&mut self, vp: usize, pos: DVec2) {
        self.update_scene();
        if !self.runner.is_active() || !self.aids.smarttrack {
            self.track_points.clear();
        }
        let (world, snap) = self.snapped_point(vp, pos);
        self.track_lines = if snap.is_none() { self.smart_track(vp, pos).map(|t| t.1).unwrap_or_default() } else { Vec::new() };
        // resting on an object snap makes that point a SmartTrack point
        if self.aids.smarttrack && self.wants_point() && snap.is_some_and(|k| k != SnapKind::Near) && !self.track_points.iter().any(|q| q.distance(world) < 1e-9) {
            self.track_points.push(world);
            if self.track_points.len() > 6 {
                self.track_points.remove(0);
            }
        }
        self.cursor = Some(Cursor { viewport: vp, screen: pos, world, snap });
    }

    /// Object-snap points of the geometry the running command has drawn so far: the
    /// vertices and midpoints of a polyline in progress, the ends of a line, and so on.
    fn pending_snaps(&self) -> Vec<(DVec3, SnapKind)> {
        let mut out = Vec::new();
        if !self.wants_point() {
            return out;
        }
        let seg = |a: DVec3, b: DVec3, out: &mut Vec<(DVec3, SnapKind)>| {
            out.push((a, SnapKind::End));
            out.push((b, SnapKind::End));
            out.push(((a + b) * 0.5, SnapKind::Mid));
        };
        for p in self.runner.preview(None, &self.doc) {
            match p {
                Preview::Line(a, b) => seg(a, b, &mut out),
                Preview::Polyline(pts) => {
                    for w in pts.windows(2) {
                        seg(w[0], w[1], &mut out);
                    }
                }
                Preview::Curve(c) => {
                    out.push((c.start_point(), SnapKind::End));
                    out.push((c.end_point(), SnapKind::End));
                }
                Preview::Point(p) => out.push((p, SnapKind::Point)),
                _ => {}
            }
        }
        if let Some(b) = self.runner.base_point() {
            out.push((b, SnapKind::End));
        }
        out
    }

    /// SmartTrack: horizontal and vertical (construction-plane x and y) lines through the
    /// tracking points — the snap points the cursor has rested on and the last picked
    /// point. Near one line the cursor slides along it; near two it snaps to where they
    /// cross. Returns the point and the lines to draw.
    fn smart_track(&self, vp: usize, pos: DVec2) -> Option<(DVec3, Vec<(DVec3, DVec3)>)> {
        if !self.aids.smarttrack || !self.wants_point() || self.alt_down {
            return None;
        }
        let v = &self.viewports[vp];
        let (cam, rect) = (&v.camera, &v.rect);
        let cp = v.cplane();
        let mut points = self.track_points.clone();
        if let Some(b) = self.runner.base_point() {
            if !points.iter().any(|q| q.distance(b) < 1e-9) {
                points.push(b);
            }
        }
        if points.is_empty() {
            return None;
        }
        let radius = 8.0;
        let scale = cam.world_per_pixel(rect.h) * 100.0;
        // (screen distance, point, direction)
        let mut near: Vec<(f64, DVec3, DVec3)> = Vec::new();
        for &tp in &points {
            for dir in [cp.x_axis, cp.y_axis] {
                let (Some((s0, _)), Some((s1, _))) = (cam.project(rect, tp), cam.project(rect, tp + dir * scale)) else { continue };
                let d = s1 - s0;
                if d.length() < 1e-6 {
                    continue; // seen end-on
                }
                let n = DVec2::new(-d.y, d.x).normalize();
                let dist = (pos - s0).dot(n).abs();
                if dist <= radius {
                    near.push((dist, tp, dir));
                }
            }
        }
        if near.is_empty() {
            return None;
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        let ray = cam.ray_at(rect, pos.x, pos.y);
        let long = scale * 1e4;
        // two crossing lines: their intersection, in the construction plane through the first
        let (_, a, da) = near[0];
        if let Some(&(_, b, _)) = near.iter().skip(1).find(|n| n.2.cross(da).length() > 1e-9) {
            let to2 = |p: DVec3| DVec2::new((p - cp.origin).dot(cp.x_axis), (p - cp.origin).dot(cp.y_axis));
            let (a2, b2) = (to2(a), to2(b));
            let (u, w) = if da.dot(cp.x_axis).abs() > 0.5 { (b2.x, a2.y) } else { (a2.x, b2.y) };
            let elev = (a - cp.origin).dot(cp.z_axis);
            let p = cp.point_at_3(u, w, elev);
            if cam.project(rect, p).is_some_and(|(s, _)| (s - pos).length() <= radius * 1.5) {
                return Some((p, vec![(a, p), (b, p)]));
            }
        }
        let (_, _, p) = ray.closest_to_segment(a - da * long, a + da * long);
        Some((p, vec![(a, p)]))
    }

    pub fn clear_cursor(&mut self) {
        self.cursor = None;
    }

    /// Left click in a viewport.
    pub fn viewport_click(&mut self, vp: usize, pos: DVec2, shift: bool, ctrl: bool) {
        self.active = vp;
        self.popup_toolbar = None;
        self.update_scene();
        if self.wants_point() {
            if ctrl && self.constraints.elevator.is_none() {
                // elevator mode: first click fixes the base, second the elevation
                let (p, _) = self.snapped_point(vp, pos);
                self.constraints.elevator = Some(p);
                return;
            }
            let (p, _) = self.snapped_point(vp, pos);
            self.constraints.elevator = None;
            self.constraints.distance = None;
            self.constraints.angle_deg = None;
            self.constraints.direction = None;
            self.one_shot_snap = None;
            let before: Vec<ObjectId> = self.doc.objects().map(|o| o.id).collect();
            self.with_ctx(|r, ctx| r.feed(Input::Point(p), ctx));
            self.after_command_step(&before);
            return;
        }
        if self.runner.prompt().is_some_and(|p| p.wants_pick()) {
            let v = &self.viewports[vp];
            let hits = self.filter_hits(self.scene.pick_all(&self.doc, &v.camera, &v.rect, pos.x, pos.y, 6.0));
            let hits: Vec<crate::render::scene::PickHit> = hits.into_iter().filter(|h| self.doc.get(h.id).is_some_and(|o| self.object_in_viewport(vp, o))).collect();
            let filter = match self.runner.prompt().map(|p| p.kind.clone()) {
                Some(PromptKind::ObjectPick { filter }) => filter,
                _ => commands::ObjFilter::Any,
            };
            let hit = hits.into_iter().find(|h| self.doc.get(h.id).is_some_and(|o| filter.accepts(&o.geometry)));
            if let Some(h) = hit {
                let before: Vec<ObjectId> = self.doc.objects().map(|o| o.id).collect();
                self.with_ctx(|r, ctx| r.feed(Input::Pick(h.id, h.point), ctx));
                self.after_command_step(&before);
            }
            return;
        }
        if !self.can_select() {
            return;
        }
        if self.gumball_hit(vp, pos).is_some() {
            return;
        }
        if !self.grips.is_empty() && !self.runner.is_active() {
            if let Some(gi) = self.grip_at(vp, pos, 6.0) {
                if ctrl {
                    self.grips[gi].selected = false;
                } else if shift {
                    self.grips[gi].selected = !self.grips[gi].selected;
                } else {
                    for g in self.grips.iter_mut() {
                        g.selected = false;
                    }
                    self.grips[gi].selected = true;
                }
                self.doc.clear_selection();
                return;
            }
            if !shift && !ctrl {
                for g in self.grips.iter_mut() {
                    g.selected = false;
                }
            }
        }
        let v = &self.viewports[vp];
        let hits = self.filter_hits(self.scene.pick_all(&self.doc, &v.camera, &v.rect, pos.x, pos.y, 5.0));
        if hits.len() >= 2 {
            // ambiguous: candidates at nearly the same depth, or several wire objects
            let ambiguous = (hits[0].depth - hits[1].depth).abs() < 2e-3
                || hits.iter().take(2).all(|h| self.scene.get(h.id).is_some_and(|o| o.mesh.is_none()));
            if ambiguous {
                self.selection_menu = Some(SelectionMenu {
                    screen: pos,
                    candidates: hits.iter().map(|h| h.id).collect(),
                    shift,
                    ctrl,
                    hovered: None,
                });
                return;
            }
        }
        match hits.first() {
            Some(h) => self.apply_pick(h.id, shift, ctrl),
            None => {
                if !self.wants_objects() && !shift && !ctrl {
                    self.deselect_all();
                }
            }
        }
    }

    pub fn apply_pick(&mut self, id: ObjectId, shift: bool, ctrl: bool) {
        // picking a grouped object selects its whole group, as in Triceratops
        let members = self.doc.group_members(id);
        if ctrl {
            for m in &members {
                self.doc.select(*m, false);
            }
        } else if shift || self.wants_objects() {
            if self.wants_objects() && !shift && !self.doc.is_selected(id) {
                for m in &members {
                    self.doc.select(*m, true);
                }
            } else {
                let on = !self.doc.is_selected(id);
                for m in &members {
                    self.doc.select(*m, on);
                }
            }
        } else {
            self.doc.clear_selection();
            for m in &members {
                self.doc.select(*m, true);
            }
        }
        if self.doc.is_selected(id) {
            if !self.pick_order.contains(&id) {
                self.pick_order.push(id);
            }
        } else {
            self.pick_order.retain(|i| *i != id);
        }
        if self.wants_objects() {
            let n = self.doc.selection.len();
            if let Some(o) = self.doc.get(id) {
                let kind = o.geometry.kind();
                if self.doc.is_selected(id) {
                    self.log.push(format!("1 {kind} added to selection. {n} total."));
                } else {
                    self.log.push(format!("1 {kind} removed from selection. {n} total."));
                }
            }
        }
    }

    /// Scene lights for the shader (3 vec4 per light) in Rendered mode.
    pub fn light_uniforms(&self, vp: usize) -> Vec<[f32; 4]> {
        if self.viewports[vp].display != DisplayMode::Rendered {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (_, kind, pos, dir, color, intensity, on) in self.doc.lights() {
            if !on || out.len() >= 24 {
                continue;
            }
            let kind_id = match kind.as_str() { "Directional" => 0.0, "Spot" => 2.0, _ => 1.0 };
            let angle = 30.0f32.to_radians();
            out.push([pos.x as f32, pos.y as f32, pos.z as f32, kind_id]);
            out.push([dir.x as f32, dir.y as f32, dir.z as f32, angle]);
            out.push([color[0], color[1], color[2], intensity]);
        }
        if let Some(s) = self.sun {
            if out.len() < 24 {
                out.push([0.0, 0.0, 0.0, 0.0]);
                out.push([-s.x as f32, -s.y as f32, -s.z as f32, 0.0]);
                out.push([1.0, 0.97, 0.9, 1.0]);
            }
        }
        out
    }

    pub fn gradient_for(&self, vp: usize) -> Option<[f32; 3]> {
        let v = &self.viewports[vp];
        if v.display.white_background() || v.is_page() {
            return None;
        }
        v.gradient.or(self.theme.viewport_gradient).map(|(_, bottom)| { let c = lin(bottom); [c[0], c[1], c[2]] })
    }

    /// Active clipping plane for a viewport as (normal, offset); fragments with n·p + d > 0 are hidden.
    pub fn clip_for(&self, _vp: usize) -> Option<[f32; 4]> {
        for cp in &self.clipping_planes {
            if !cp.enabled {
                continue;
            }
            let Some(o) = self.doc.get(cp.object) else { continue };
            let (p, n) = match &o.geometry {
                Geometry::Surface(s) => {
                    let (u0, u1) = s.domain_u();
                    let (v0, v1) = s.domain_v();
                    (s.point_at((u0 + u1) * 0.5, (v0 + v1) * 0.5), s.normal_at((u0 + u1) * 0.5, (v0 + v1) * 0.5))
                }
                Geometry::Brep(b) if !b.faces.is_empty() => b.faces[0].sample(),
                _ => continue,
            };
            let n = if cp.flipped { -n } else { n };
            let d = -n.dot(p);
            return Some([n.x as f32, n.y as f32, n.z as f32, d as f32]);
        }
        None
    }

    fn sync_clipping_memory(&mut self) {
        self.clipping_planes.retain(|c| self.doc.get(c.object).is_some());
        let ids: Vec<String> = self.clipping_planes.iter().map(|c| c.object.to_string()).collect();
        self.memory.insert("ClippingPlanes".into(), ids.join(","));
    }

    /// Gumball geometry for a viewport, when it should be shown.
    pub fn gumball_screen(&self, vp: usize) -> Option<GumballScreen> {
        if !self.aids.gumball || self.doc.selection.is_empty() || self.runner.is_active() {
            return None;
        }
        let v = &self.viewports[vp];
        let (world_origin, axes) = match &self.gumball_drag {
            Some(d) => (d.origin, d.axes),
            None => {
                let bb = self.doc.selected_ids().iter().filter_map(|id| self.doc.get(*id)).fold(BoundingBox::EMPTY, |b, o| b.union(&o.geometry.bounding_box()));
                if bb.is_empty() {
                    return None;
                }
                let cp = v.cplane();
                let axes = match self.gumball_align.as_str() {
                    "World" => [DVec3::X, DVec3::Y, DVec3::Z],
                    "View" => {
                        let (r, u) = v.camera.basis();
                        [r, u, -v.camera.view_dir()]
                    }
                    _ => [cp.x_axis, cp.y_axis, cp.z_axis],
                };
                (self.gumball_origin.unwrap_or(bb.center()), self.gumball_axes.unwrap_or(axes))
            }
        };
        let (origin, _) = v.camera.project(&v.rect, world_origin)?;
        let length = v.camera.world_per_pixel(v.rect.h) * 70.0;
        let mut arrow_end = [DVec2::ZERO; 3];
        let mut scale_pos = [DVec2::ZERO; 3];
        let mut arcs: [Vec<DVec2>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for i in 0..3 {
            arrow_end[i] = v.camera.project(&v.rect, world_origin + axes[i] * length)?.0;
            scale_pos[i] = v.camera.project(&v.rect, world_origin + axes[i] * (length * 0.55))?.0;
            let (a, b) = (axes[(i + 1) % 3], axes[(i + 2) % 3]);
            let r = length * 0.8;
            for k in 0..=24 {
                let t = std::f64::consts::FRAC_PI_2 * k as f64 / 24.0;
                if let Some((p, _)) = v.camera.project(&v.rect, world_origin + a * (r * t.cos()) + b * (r * t.sin())) {
                    arcs[i].push(p);
                }
            }
        }
        Some(GumballScreen { origin, world_origin, axes, arrow_end, scale_pos, arcs })
    }

    fn gumball_hit(&self, vp: usize, pos: DVec2) -> Option<(GumballHandle, GumballScreen)> {
        let g = self.gumball_screen(vp)?;
        let tol = 8.0;
        let seg_dist = |p: DVec2, a: DVec2, b: DVec2| {
            let ab = b - a;
            let l2 = ab.length_squared();
            let t = if l2 < 1e-12 { 0.0 } else { ((p - a).dot(ab) / l2).clamp(0.0, 1.0) };
            (a + ab * t - p).length()
        };
        for i in 0..3 {
            if (g.scale_pos[i] - pos).length() <= tol {
                return Some((GumballHandle::Scale(i), g));
            }
        }
        for i in 0..3 {
            if seg_dist(pos, g.origin, g.arrow_end[i]) <= tol || (g.arrow_end[i] - pos).length() <= tol * 1.5 {
                return Some((GumballHandle::Move(i), g));
            }
        }
        for i in 0..3 {
            if g.arcs[i].windows(2).any(|w| seg_dist(pos, w[0], w[1]) <= tol) {
                return Some((GumballHandle::Rotate(i), g));
            }
        }
        if (g.origin - pos).length() <= tol {
            return Some((GumballHandle::Free, g));
        }
        None
    }

    /// Point on the handle's constraint (axis line or rotation plane) under the cursor.
    fn gumball_cursor_point(&self, vp: usize, pos: DVec2, handle: GumballHandle, origin: DVec3, axes: [DVec3; 3]) -> DVec3 {
        let v = &self.viewports[vp];
        let ray = v.camera.ray_at(&v.rect, pos.x, pos.y);
        match handle {
            GumballHandle::Move(i) | GumballHandle::Scale(i) => {
                let (_, _, p) = ray.closest_to_segment(origin - axes[i] * 1e6, origin + axes[i] * 1e6);
                p
            }
            GumballHandle::Rotate(i) => {
                let plane = Plane::from_normal(origin, axes[i]);
                plane.intersect_ray(&ray).map_or(origin, |t| ray.at(t))
            }
            GumballHandle::Free => {
                let plane = Plane::from_normal(origin, v.camera.view_dir());
                plane.intersect_ray(&ray).map_or(origin, |t| ray.at(t))
            }
        }
    }

    fn gumball_transform(&self, d: &GumballDrag, to: DVec3) -> DAffine3 {
        match d.handle {
            GumballHandle::Move(_) | GumballHandle::Free => DAffine3::from_translation(to - d.start),
            GumballHandle::Scale(i) => {
                let axis = d.axes[i];
                let s0 = (d.start - d.origin).dot(axis);
                let s1 = (to - d.origin).dot(axis);
                let f = if s0.abs() < 1e-9 { 1.0 } else { (s1 / s0).max(1e-6) };
                let m = glam::DMat3::IDENTITY + glam::DMat3::from_cols(axis * (axis.x * (f - 1.0)), axis * (axis.y * (f - 1.0)), axis * (axis.z * (f - 1.0)));
                DAffine3::from_translation(d.origin) * DAffine3::from_mat3(m) * DAffine3::from_translation(-d.origin)
            }
            GumballHandle::Rotate(i) => {
                let axis = d.axes[i];
                let a = (d.start - d.origin) - axis * (d.start - d.origin).dot(axis);
                let b = (to - d.origin) - axis * (to - d.origin).dot(axis);
                let ang = a.normalize_or(DVec3::X).cross(b.normalize_or(DVec3::X)).dot(axis).asin();
                let ang = if a.dot(b) < 0.0 { std::f64::consts::PI - ang } else { ang };
                DAffine3::from_translation(d.origin) * DAffine3::from_quat(glam::DQuat::from_axis_angle(axis, ang)) * DAffine3::from_translation(-d.origin)
            }
        }
    }

    fn apply_gumball(&mut self, xf: DAffine3) {
        let Some(d) = &self.gumball_drag else { return };
        let originals = d.originals.clone();
        for (id, g) in originals {
            let mut g2 = g;
            g2.transform(&xf);
            self.doc.replace_geometry(id, g2);
        }
        if let Some(d) = &mut self.gumball_drag {
            d.last_xf = xf;
        }
    }

    /// Rebuild grip positions from the document (keeping the selection state when `keep` is false).
    pub fn rebuild_grips(&mut self, reset_selection: bool) {
        let selected: std::collections::HashSet<(ObjectId, usize)> = if reset_selection { Default::default() } else { self.grips.iter().filter(|g| g.selected).map(|g| (g.object, g.index)).collect() };
        self.points_on.retain(|id| self.doc.get(*id).is_some());
        let mut grips = Vec::new();
        for id in &self.points_on {
            let Some(o) = self.doc.get(*id) else { continue };
            let pts: Vec<DVec3> = match &o.geometry {
                Geometry::Curve(c) => c.control_points.clone(),
                Geometry::Surface(s) => s.control_points.clone(),
                _ => Vec::new(),
            };
            for (i, p) in pts.into_iter().enumerate() {
                grips.push(Grip { object: *id, index: i, pos: p, selected: selected.contains(&(*id, i)) });
            }
        }
        self.grips = grips;
    }

    fn grip_at(&self, vp: usize, pos: DVec2, tol_px: f64) -> Option<usize> {
        let v = &self.viewports[vp];
        let mut best: Option<(usize, f64)> = None;
        for (i, g) in self.grips.iter().enumerate() {
            if let Some((s, _)) = v.camera.project(&v.rect, g.pos) {
                let d = (s - pos).length();
                if d <= tol_px && best.is_none_or(|b| d < b.1) {
                    best = Some((i, d));
                }
            }
        }
        best.map(|b| b.0)
    }

    pub fn any_grip_selected(&self) -> bool {
        self.grips.iter().any(|g| g.selected)
    }

    /// Move the selected grips (and the control points they stand for) by `delta`.
    pub fn move_selected_grips(&mut self, delta: DVec3) {
        let moves: Vec<(ObjectId, usize)> = self.grips.iter().filter(|g| g.selected).map(|g| (g.object, g.index)).collect();
        for (id, idx) in moves {
            if let Some(o) = self.doc.get_mut(id) {
                match &mut o.geometry {
                    Geometry::Curve(c) => {
                        if let Some(p) = c.control_points.get_mut(idx) {
                            *p += delta;
                        }
                        // periodic curves: keep the wrapped control points in sync
                        let n = c.control_points.len();
                        let p = c.degree;
                        let periodic = c.knots.windows(2).all(|w| (w[1] - w[0] - (c.knots[1] - c.knots[0])).abs() < 1e-9) && n > p;
                        if periodic {
                            if idx < p {
                                c.control_points[n - p + idx] = c.control_points[idx];
                            } else if idx >= n - p {
                                c.control_points[idx - (n - p)] = c.control_points[idx];
                            }
                        }
                    }
                    Geometry::Surface(s) => {
                        if let Some(p) = s.control_points.get_mut(idx) {
                            *p += delta;
                        }
                    }
                    _ => {}
                }
            }
        }
        self.rebuild_grips(false);
    }

    pub fn selection_menu_pick(&mut self, id: Option<ObjectId>) {
        if let Some(menu) = self.selection_menu.take() {
            if let Some(id) = id {
                self.apply_pick(id, menu.shift, menu.ctrl);
            }
        }
    }

    pub fn viewport_right_click(&mut self, vp: usize) {
        self.active = vp;
        self.submit();
    }

    /// Start a left drag: move selected objects when pressing on one of them, else window select.
    pub fn begin_left_drag(&mut self, vp: usize, pos: DVec2) {
        self.active = vp;
        self.update_scene();
        if self.runner.is_active() && self.memory.get("Sketch.Active").is_some_and(|v| v == "Yes") {
            let (p, _) = self.snapped_point(vp, pos);
            self.sketch_points = Some(vec![p]);
            return;
        }
        if let Some((handle, g)) = self.gumball_hit(vp, pos) {
            let originals: Vec<(ObjectId, Geometry)> = self.doc.selected_ids().iter().filter_map(|id| self.doc.get(*id).map(|o| (*id, o.geometry.clone()))).collect();
            let start = self.gumball_cursor_point(vp, pos, handle, g.world_origin, g.axes);
            self.doc.begin_undo("Gumball");
            self.gumball_drag = Some(GumballDrag { viewport: vp, handle, origin: g.world_origin, axes: g.axes, start, originals, last_xf: DAffine3::IDENTITY });
            return;
        }
        if self.runner.is_active() && !self.wants_objects() {
            return;
        }
        if !self.runner.is_active() && !self.grips.is_empty() {
            if let Some(gi) = self.grip_at(vp, pos, 6.0) {
                if !self.grips[gi].selected {
                    for g in self.grips.iter_mut() {
                        g.selected = false;
                    }
                    self.grips[gi].selected = true;
                }
                let (from, _) = self.snapped_point(vp, pos);
                self.grip_drag = Some(GripDrag { viewport: vp, to: from, started: false });
                return;
            }
        }
        let v = &self.viewports[vp];
        if !self.runner.is_active() {
            if let Some(h) = self.scene.pick(&self.doc, &v.camera, &v.rect, pos.x, pos.y, 5.0).filter(|h| self.doc.get(h.id).is_some_and(|o| self.object_in_viewport(vp, o))) {
                if self.doc.is_selected(h.id) {
                    let (from, _) = self.snapped_point(vp, pos);
                    self.drag_move = Some(DragMove { viewport: vp, ids: self.doc.selected_ids(), from, to: from });
                    return;
                }
            }
        }
        self.window_drag = Some(WindowDrag { viewport: vp, start: pos, current: pos });
    }

    pub fn update_left_drag(&mut self, pos: DVec2) {
        if self.sketch_points.is_some() {
            let vp = self.active;
            let (p, _) = self.snapped_point(vp, pos);
            if let Some(pts) = &mut self.sketch_points {
                if pts.last().is_none_or(|l| (*l - p).length() > 1e-9) {
                    pts.push(p);
                }
            }
            return;
        }
        if let Some(d) = self.gumball_drag.clone() {
            let to = self.gumball_cursor_point(d.viewport, pos, d.handle, d.origin, d.axes);
            let xf = self.gumball_transform(&d, to);
            self.apply_gumball(xf);
            return;
        }
        if let Some(d) = &mut self.window_drag {
            d.current = pos;
        } else if let Some(d) = self.grip_drag.clone() {
            let (to, _) = self.snapped_point(d.viewport, pos);
            let delta = to - d.to;
            if !d.started {
                self.doc.begin_undo("Drag control points");
            }
            self.move_selected_grips(delta);
            if let Some(g) = &mut self.grip_drag {
                g.to = to;
                g.started = true;
            }
        } else if let Some(d) = self.drag_move.clone() {
            let (to, _) = self.snapped_point(d.viewport, pos);
            if let Some(dm) = &mut self.drag_move {
                dm.to = to;
            }
        }
    }

    pub fn end_left_drag(&mut self, shift: bool, ctrl: bool) {
        if let Some(pts) = self.sketch_points.take() {
            let text = pts.iter().map(|p| format!("{},{},{}", p.x, p.y, p.z)).collect::<Vec<_>>().join(";");
            self.feed_input(Input::Text(text));
            return;
        }
        if let Some(d) = self.gumball_drag.take() {
            if d.last_xf == DAffine3::IDENTITY {
                // nothing moved: drop the undo record by undoing it silently
                let _ = self.doc.undo();
            } else {
                self.log.push(format!("Command: _Gumball{}", match d.handle {
                    GumballHandle::Move(_) | GumballHandle::Free => "Move",
                    GumballHandle::Scale(_) => "Scale",
                    GumballHandle::Rotate(_) => "Rotate",
                }));
                let keep: Vec<ObjectId> = d.originals.iter().map(|(id, _)| *id).collect();
                self.resolve_constraints(&keep);
                self.update_dependents(&keep);
            }
            return;
        }
        if self.grip_drag.take().is_some() {
            let keep: Vec<ObjectId> = self.points_on.clone();
            self.resolve_constraints(&keep);
            self.update_dependents(&keep);
            self.rebuild_grips(false);
            return;
        }
        if let Some(d) = self.window_drag.take() {
            self.window_select_finish(d, shift, ctrl);
        } else if let Some(d) = self.drag_move.take() {
            let delta = d.to - d.from;
            let v = &self.viewports[d.viewport];
            let px = v.camera.world_per_pixel(v.rect.h);
            if delta.length() > 4.0 * px {
                self.doc.begin_undo("Move");
                self.doc.transform_objects(&d.ids, &DAffine3::from_translation(delta));
                self.log.push("Command: _Move".into());
                self.resolve_constraints(&d.ids);
                self.update_dependents(&d.ids);
            }
        }
    }

    fn window_select_finish(&mut self, drag: WindowDrag, shift: bool, ctrl: bool) {
        let v = &self.viewports[drag.viewport];
        let min = drag.start.min(drag.current);
        let max = drag.start.max(drag.current);
        if (max - min).length() < 6.0 {
            // a short drag is a click (Triceratops tolerates small mouse movement)
            self.viewport_click(drag.viewport, drag.start, shift, ctrl);
            return;
        }
        let crossing = drag.current.x < drag.start.x;
        if !self.grips.is_empty() && !self.runner.is_active() {
            let cam = v.camera.clone();
            let rect = v.rect;
            let mut any = false;
            for g in self.grips.iter_mut() {
                if let Some((s, _)) = cam.project(&rect, g.pos) {
                    if s.x >= min.x && s.x <= max.x && s.y >= min.y && s.y <= max.y {
                        g.selected = !ctrl;
                        any = true;
                    } else if !shift && !ctrl {
                        g.selected = false;
                    }
                }
            }
            if any {
                self.doc.clear_selection();
                return;
            }
        }
        let ids: Vec<ObjectId> = self.scene.pick_window(&self.doc, &v.camera, &v.rect, min, max, crossing).into_iter().filter(|id| self.doc.get(*id).is_some_and(|o| self.object_in_viewport(drag.viewport, o))).collect();
        if !shift && !ctrl && !self.wants_objects() {
            self.deselect_all();
        }
        for id in &ids {
            self.doc.select(*id, !ctrl);
            if ctrl {
                self.pick_order.retain(|i| i != id);
            } else if !self.pick_order.contains(id) {
                self.pick_order.push(*id);
            }
        }
        if self.wants_objects() {
            self.log.push(format!("{} objects added to selection. {} total.", ids.len(), self.doc.selection.len()));
        }
    }

    // ----- navigation -------------------------------------------------------

    pub fn begin_view_change(&mut self, vp: usize) {
        let cam = self.viewports[vp].camera.clone();
        self.viewports[vp].history.push(&cam);
    }

    pub fn orbit(&mut self, vp: usize, dx: f64, dy: f64) {
        self.viewports[vp].camera.orbit(dx, dy);
    }

    pub fn pan(&mut self, vp: usize, dx: f64, dy: f64) {
        let h = self.viewports[vp].rect.h;
        self.viewports[vp].camera.pan(dx, dy, h);
    }

    pub fn zoom_drag(&mut self, vp: usize, dy: f64) {
        let factor = (-dy * 0.01).exp();
        self.viewports[vp].camera.zoom(factor, None);
    }

    pub fn zoom(&mut self, vp: usize, pos: DVec2, scroll: f64) {
        self.begin_view_change(vp);
        let factor = (scroll * 0.0025).exp();
        let v = &self.viewports[vp];
        let ray = v.camera.ray_at(&v.rect, pos.x, pos.y);
        let anchor = self
            .scene
            .ray_hit_any(&ray)
            .map(|t| ray.at(t))
            .or_else(|| v.cplane().intersect_ray(&ray).filter(|t| *t > 0.0).map(|t| ray.at(t)))
            .or_else(|| Plane::from_normal(v.camera.target, v.camera.view_dir()).intersect_ray(&ray).map(|t| ray.at(t)));
        self.viewports[vp].camera.zoom(factor, anchor);
    }

    pub fn key_nav(&mut self, key: egui::Key, shift: bool, ctrl: bool) -> bool {
        let vp = self.active;
        let rotatable = self.viewports[vp].camera.rotatable;
        let step = 15.0;
        match key {
            egui::Key::ArrowLeft | egui::Key::ArrowRight | egui::Key::ArrowUp | egui::Key::ArrowDown => {
                self.begin_view_change(vp);
                let (dx, dy) = match key {
                    egui::Key::ArrowLeft => (-step, 0.0),
                    egui::Key::ArrowRight => (step, 0.0),
                    egui::Key::ArrowUp => (0.0, -step),
                    _ => (0.0, step),
                };
                if shift || !rotatable {
                    self.pan(vp, -dx * 2.0, -dy * 2.0);
                } else {
                    self.orbit(vp, dx * 5.0, dy * 5.0);
                }
                true
            }
            egui::Key::PageUp | egui::Key::PageDown => {
                self.begin_view_change(vp);
                let f = if key == egui::Key::PageUp { 1.25 } else { 0.8 };
                if ctrl && rotatable {
                    let cam = &mut self.viewports[vp].camera;
                    let d = cam.view_dir() * cam.distance * 0.1 * if key == egui::Key::PageUp { 1.0 } else { -1.0 };
                    cam.target += d;
                } else {
                    self.viewports[vp].camera.zoom(f, None);
                }
                true
            }
            egui::Key::Home => {
                self.undo_view();
                true
            }
            egui::Key::End => {
                self.redo_view();
                true
            }
            _ => false,
        }
    }

    pub fn undo_view(&mut self) {
        let v = &mut self.viewports[self.active];
        if let Some(c) = v.history.undo(&v.camera.clone()) {
            v.camera = c;
        }
    }

    pub fn redo_view(&mut self) {
        let v = &mut self.viewports[self.active];
        if let Some(c) = v.history.redo(&v.camera.clone()) {
            v.camera = c;
        }
    }

    pub fn zoom_extents(&mut self, selected: bool, all_views: bool) {
        let bb = if selected {
            self.doc.selected_ids().iter().filter_map(|id| self.doc.get(*id)).fold(BoundingBox::EMPTY, |b, o| b.union(&o.geometry.bounding_box()))
        } else {
            self.doc.bounding_box()
        };
        let bb = if bb.is_empty() { BoundingBox::new(DVec3::splat(-12.0), DVec3::splat(12.0)) } else { bb.inflate(bb.diagonal() * self.zoom_border + 0.5) };
        let targets: Vec<usize> = if all_views { (0..self.viewports.len()).collect() } else { vec![self.active] };
        for vp in targets {
            self.begin_view_change(vp);
            self.viewports[vp].camera.zoom_to(&bb);
        }
    }

    pub fn set_view(&mut self, name: &str) {
        if let Some(cam) = Camera::by_name(name) {
            self.begin_view_change(self.active);
            let v = &mut self.viewports[self.active];
            let target = v.camera.target;
            let dist = v.camera.distance;
            let oh = v.camera.ortho_height;
            v.camera = cam;
            v.camera.target = target;
            v.camera.distance = dist;
            v.camera.ortho_height = oh;
            let lower = name.to_ascii_lowercase();
            v.name = match lower.as_str() {
                "persp" | "perspective" => "Perspective".into(),
                n => {
                    let mut c = n.chars();
                    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
                }
            };
        }
    }

    pub fn set_display(&mut self, vp: usize, mode: DisplayMode) {
        self.viewports[vp].display = mode;
    }

    pub fn toggle_maximize(&mut self, vp: usize) {
        self.active = vp;
        self.layout = match self.layout {
            Layout::Single => if self.viewports.len() > 4 { Layout::Grid } else { Layout::Four },
            _ => Layout::Single,
        };
    }

    /// Viewport management: new, close, split, to_top, to_back, swap.
    pub fn viewport_op(&mut self, op: &str) {
        match op {
            "new" | "split_h" | "split_v" | "float" => {
                let n = self.viewports.len() + 1;
                let mut v = Viewport::new(&format!("Viewport {n}"), self.viewports[self.active].camera.clone(), self.viewports[self.active].display);
                v.rect = self.viewports[self.active].rect;
                self.viewports.push(v);
                self.viewport_tab_order.push(self.viewports.len() - 1);
                self.sync_batch_vectors();
                self.active = self.viewports.len() - 1;
                self.layout = Layout::Grid;
                self.log.push(format!("Viewport {n} added."));
            }
            "close" => {
                if self.viewports.len() <= 1 {
                    self.log.push("The last viewport cannot be closed.".into());
                    return;
                }
                let idx = self.active;
                let mut remove = self.details_of(idx);
                remove.push(idx);
                remove.sort_unstable_by(|a, b| b.cmp(a));
                for i in remove {
                    self.remove_viewport(i);
                }
                self.sync_batch_vectors();
                self.active = 0;
                self.layout = if self.viewports.iter().filter(|v| !v.is_detail()).count() == 4 { Layout::Four } else { Layout::Grid };
            }
            "to_top" | "to_back" => {
                let idx = self.active;
                if let Some(pos) = self.viewport_tab_order.iter().position(|v| *v == idx) {
                    self.viewport_tab_order.remove(pos);
                    if op == "to_top" {
                        self.viewport_tab_order.insert(0, idx);
                    } else {
                        self.viewport_tab_order.push(idx);
                    }
                }
            }
            "swap" => {
                let vis = self.visible_viewports();
                if let Some(i) = vis.iter().position(|v| *v == self.active) {
                    let other = vis[(i + 1) % vis.len()];
                    if other != self.active {
                        let (a, b) = (self.active, other);
                        let ca = self.viewports[a].camera.clone();
                        let cb = self.viewports[b].camera.clone();
                        self.viewports[a].camera = cb;
                        self.viewports[b].camera = ca;
                        let da = self.viewports[a].display;
                        self.viewports[a].display = self.viewports[b].display;
                        self.viewports[b].display = da;
                    }
                }
            }
            _ => {}
        }
    }

    /// Set the camera target to the centre of the selection (or all objects) without moving the eye.
    pub fn target_to_objects(&mut self) {
        let ids = self.doc.selected_ids();
        let bb = if ids.is_empty() { self.doc.bounding_box() } else { ids.iter().filter_map(|id| self.doc.get(*id)).fold(BoundingBox::EMPTY, |b, o| b.union(&self.doc.object_bbox(o))) };
        if bb.is_empty() {
            return;
        }
        let v = &mut self.viewports[self.active];
        v.history.push(&v.camera.clone());
        let eye = v.camera.eye();
        let c = bb.center();
        let d = eye - c;
        v.camera.target = c;
        v.camera.distance = d.length().max(0.01);
        if v.camera.rotatable {
            v.camera.yaw = d.y.atan2(d.x);
            v.camera.pitch = (d.z / d.length().max(1e-9)).clamp(-1.0, 1.0).asin();
        }
    }

    /// Build an animation from a camera path (and optional target path) or a sun sweep.
    pub fn setup_animation(&mut self, kind: &str, frames: usize, path: &[DVec3], targets: &[DVec3], record: Option<String>) {
        let n = frames.max(2);
        let base = self.viewports[self.active].camera.clone();
        let sample = |pts: &[DVec3], f: f64| -> DVec3 {
            if pts.len() < 2 {
                return pts.first().copied().unwrap_or(base.target);
            }
            let t = f * (pts.len() - 1) as f64;
            let i = (t.floor() as usize).min(pts.len() - 2);
            pts[i] + (pts[i + 1] - pts[i]) * (t - i as f64)
        };
        let mut out = Vec::with_capacity(n);
        for k in 0..n {
            let f = k as f64 / (n - 1) as f64;
            let frame = match kind {
                "turntable" => {
                    let mut c = base.clone();
                    c.yaw = base.yaw + std::f64::consts::TAU * k as f64 / n as f64;
                    AnimFrame { camera: Some(c), sun: None }
                }
                "sun_day" | "sun_season" => {
                    let az = if kind == "sun_day" { -90.0 + 180.0 * f } else { 135.0 };
                    let alt = if kind == "sun_day" { (std::f64::consts::PI * f).sin() * 60.0 } else { 20.0 + 50.0 * (std::f64::consts::PI * f).sin() };
                    let (sa, ca) = az.to_radians().sin_cos();
                    let (sl, cl) = alt.to_radians().sin_cos();
                    AnimFrame { camera: None, sun: Some(DVec3::new(sa * cl, ca * cl, sl).normalize()) }
                }
                _ => {
                    // "path": camera along `path` looking at the fixed target; "fly": camera and target paths
                    let eye = sample(path, f);
                    let tgt = if targets.is_empty() { base.target } else { sample(targets, f) };
                    let mut c = base.clone();
                    c.target = tgt;
                    let d = eye - tgt;
                    c.distance = d.length().max(0.01);
                    c.yaw = d.y.atan2(d.x);
                    c.pitch = (d.z / d.length().max(1e-9)).clamp(-1.0, 1.0).asin();
                    c.rotatable = true;
                    c.up_override = None;
                    AnimFrame { camera: Some(c), sun: None }
                }
            };
            out.push(frame);
        }
        self.animation = Animation { frames: out, index: 0, playing: false, record_dir: record };
        self.log.push(format!("Animation with {n} frames ready: PlayAnimation, RecordAnimation or ViewFrameNumber."));
    }

    /// Apply one animation frame to the active viewport.
    pub fn apply_animation_frame(&mut self, index: usize) {
        let Some(f) = self.animation.frames.get(index).cloned() else { return };
        self.animation.index = index;
        if let Some(c) = f.camera {
            self.viewports[self.active].camera = c;
        }
        if let Some(s) = f.sun {
            self.sun = Some(s);
        }
    }

    pub fn next_viewport(&mut self) {
        let vis = self.visible_viewports();
        if let Some(i) = vis.iter().position(|v| *v == self.active) {
            self.active = vis[(i + 1) % vis.len()];
        }
    }

    // ----- osnap ------------------------------------------------------------

    pub fn one_shot(&mut self, kind: SnapKind) {
        self.one_shot_snap = Some(kind);
    }

    // ----- requests from commands ---------------------------------------

    pub fn process_requests(&mut self) {
        let reqs = std::mem::take(&mut self.requests);
        for r in reqs {
            match r {
                AppRequest::ZoomExtents { all, selected } => self.zoom_extents(selected, all),
                AppRequest::ZoomWindow => self.log.push("Zoom window: drag a window in a viewport (not implemented yet).".into()),
                AppRequest::SetView(n) => self.set_view(&n),
                AppRequest::Layout(l) => {
                    self.layout = match l.as_str() {
                        "3View" => Layout::Three,
                        _ => Layout::Four,
                    }
                }
                AppRequest::MaxViewport => self.toggle_maximize(self.active),
                AppRequest::SetDisplayMode(m) => {
                    if let Some(mode) = DisplayMode::from_name(&m) {
                        self.viewports[self.active].display = mode;
                    }
                }
                AppRequest::ToggleSnap(s) => match s.as_str() {
                    "grid" => self.aids.grid_snap = !self.aids.grid_snap,
                    "ortho" => self.aids.ortho = !self.aids.ortho,
                    "planar" => self.aids.planar = !self.aids.planar,
                    "disable" => self.aids.osnap.disable = !self.aids.osnap.disable,
                    "project" => self.aids.osnap.project = !self.aids.osnap.project,
                    _ => self.aids.osnap.disable = !self.aids.osnap.disable,
                },
                AppRequest::FileNew => self.file_new(),
                AppRequest::FileOpen => self.file_open(),
                AppRequest::FileSave => self.file_save(false),
                AppRequest::FileSaveAs => self.file_save(true),
                AppRequest::Import => self.file_import(),
                AppRequest::Export => self.file_export(),
                AppRequest::Locust => self.locust.show_window(),
                AppRequest::Quit => {
                    self.locust.save_autosave();
                    self.save_options();
                    self.quit = true;
                }
                AppRequest::Help => self.show_help = true,
                AppRequest::ShowPanel(p) => {
                    self.show_panel(&p);
                }
                AppRequest::CommandHistory => self.show_history = true,
                AppRequest::Options(p) => self.options_page = Some(p),
                AppRequest::DocumentProperties(p) => self.docprops_page = Some(p),
                AppRequest::UndoView => self.undo_view(),
                AppRequest::RedoView => self.redo_view(),
                AppRequest::LoftDialog(ids) => {
                    self.loft_dialog = Some(LoftDialog { ids, style: 0, closed: false, rebuild: 0, rebuild_count: 10, refit_tol: 0.001 });
                    if self.assistant_mode {
                        // nobody to click OK: loft with the default options
                        self.finish_loft(true);
                    }
                }
                AppRequest::Message(m) if self.assistant_mode => self.log.push(m),
                AppRequest::Message(m) => self.message_box = Some(m),
                AppRequest::PointsOn(ids) => {
                    for id in ids {
                        if !self.points_on.contains(&id) {
                            self.points_on.push(id);
                        }
                    }
                    self.rebuild_grips(true);
                }
                AppRequest::PointsOff => {
                    self.points_on.clear();
                    self.grips.clear();
                }
                AppRequest::CopyToClipboard { cut } => {
                    let ids = self.doc.selected_ids();
                    self.clipboard = ids.iter().filter_map(|id| self.doc.get(*id)).map(|o| (o.geometry.clone(), o.attributes.clone())).collect();
                    self.log.push(format!("{} objects copied to the clipboard.", self.clipboard.len()));
                    if cut {
                        self.doc.begin_undo("Cut");
                        for id in ids {
                            self.doc.remove(id);
                        }
                    }
                }
                AppRequest::Paste => {
                    if self.clipboard.is_empty() {
                        self.log.push("The clipboard is empty.".into());
                    } else {
                        self.doc.begin_undo("Paste");
                        self.doc.clear_selection();
                        for (g, a) in self.clipboard.clone() {
                            let id = self.doc.add_with(g, a);
                            self.doc.select(id, true);
                        }
                    }
                }
                AppRequest::NamedView { name, restore } => {
                    if restore {
                        if let Some((_, cam)) = self.named_views.iter().find(|(n, _)| n.eq_ignore_ascii_case(&name)).cloned() {
                            self.begin_view_change(self.active);
                            self.viewports[self.active].camera = cam;
                            self.viewports[self.active].name = name;
                        } else {
                            self.log.push(format!("No named view \"{name}\"."));
                        }
                    } else {
                        let cam = self.viewports[self.active].camera.clone();
                        self.named_views.retain(|(n, _)| !n.eq_ignore_ascii_case(&name));
                        self.named_views.push((name.clone(), cam));
                        self.log.push(format!("Saved view \"{name}\"."));
                    }
                }
                AppRequest::ZoomWindowPoints(a, b) => {
                    let v = &mut self.viewports[self.active];
                    let bb = BoundingBox::from_points(&[a, b]);
                    v.history.push(&v.camera.clone());
                    v.camera.zoom_to(&bb);
                }
                AppRequest::PlanView => {
                    let cp = self.cplane();
                    let v = &mut self.viewports[self.active];
                    v.history.push(&v.camera.clone());
                    let dist = v.camera.distance;
                    v.camera.pitch = 90.0f64.to_radians();
                    v.camera.yaw = 0.0;
                    v.camera.projection = Projection::Parallel;
                    v.camera.up_override = Some(cp.y_axis);
                    v.camera.rotatable = false;
                    v.camera.distance = dist;
                }
                AppRequest::SetCPlane(p) => {
                    let v = &mut self.viewports[self.active];
                    match p {
                        Some(p) => v.set_cplane(Some(p)),
                        None => v.previous_cplane(),
                    }
                }
                AppRequest::CameraOp { op, dx, dy } => {
                    let v = &mut self.viewports[self.active];
                    v.history.push(&v.camera);
                    match op.as_str() {
                        "RotateView" => v.camera.orbit(dx.to_radians() * 100.0, dy.to_radians() * 100.0),
                        "Pan" => {
                            let (w, h) = (v.rect.w, v.rect.h);
                            v.camera.pan(dx * w, dy * h, h);
                        }
                        "Zoom" => v.camera.zoom(dx, None),
                        _ => {}
                    }
                }
                AppRequest::SaveCopy { incremental } => self.file_save_copy(incremental),
                AppRequest::SetOption { key, value } => self.set_option(&key, &value),
                AppRequest::ExportWithOrigin { ids, origin } => self.export_with_origin(&ids, origin),
                AppRequest::GripOp(op) => self.grip_op(&op),
                AppRequest::SelectionFilter(f) => {
                    self.selection_filter = f.clone();
                    self.log.push(match f {
                        Some(k) => format!("Selection filter: {k}"),
                        None => "Selection filter: all objects".into(),
                    });
                }
                AppRequest::WindowOp(op) => self.window_ops.push(op),
                AppRequest::Autosave => {
                    if let Some(dir) = options_path().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
                        let path = dir.join("autosave.3dm");
                        match tri_doc::io3dm::save(&self.doc, &path, false) {
                            Ok(_) => self.log.push(format!("Autosave written to {}", path.display())),
                            Err(e) => self.log.push(format!("Autosave failed: {e:#}")),
                        }
                    }
                }
                AppRequest::OptionsFile { export } => self.options_file(export),
                AppRequest::RepeatLast => {
                    if let Some(last) = self.runner.last_command.clone() {
                        self.start_command_typed(&last, &last);
                    }
                }
                AppRequest::ExportBlocks => self.export_blocks(),
                AppRequest::Audit3dmFile => {
                    if self.file_dialog_blocked("Audit3dmFile") { continue; }
                    if let Some(p) = rfd::FileDialog::new().add_filter("3D Models (*.3dm)", &["3dm"]).pick_file() {
                        match tri_doc::io3dm::load(&p) {
                            Ok((state, settings, skipped)) => self.log.push(format!("{}: {} objects, {} layers, {} block definitions, units {}, {} unsupported objects. File reads correctly.", p.display(), state.objects.len(), state.layers.len(), state.blocks.len(), settings.units, skipped)),
                            Err(e) => self.log.push(format!("{}: cannot be read ({e:#})", p.display())),
                        }
                    }
                }
                AppRequest::SaveTemplate => {
                    if let Some(dir) = options_path().and_then(|p| p.parent().map(|d| d.join("templates"))) {
                        let _ = std::fs::create_dir_all(&dir);
                        if self.file_dialog_blocked("SaveAsTemplate") { continue; }
                        if let Some(p) = rfd::FileDialog::new().set_directory(&dir).add_filter("3D Models (*.3dm)", &["3dm"]).set_file_name("Template.3dm").save_file() {
                            match tri_doc::io3dm::save(&self.doc, &p, false) {
                                Ok(_) => self.log.push(format!("Template saved as {}", p.display())),
                                Err(e) => self.log.push(format!("Save failed: {e:#}")),
                            }
                        }
                    }
                }
                AppRequest::ViewCapture { path, clipboard } if self.assistant_mode && !clipboard && path.trim_start_matches("screen:").is_empty() => {
                    // the capture runs on a later frame, after assistant mode ends: refuse now
                    self.log.push("Capture needs a file path when run by an assistant".into());
                }
                AppRequest::ViewCapture { path, clipboard } => {
                    let whole = path.starts_with("screen:");
                    let path = path.trim_start_matches("screen:").to_string();
                    self.capture_requests.push(CaptureRequest { path, clipboard, whole_window: whole, ..Default::default() });
                }
                AppRequest::BatchRenderNamedViews(dir) => {
                    if self.named_views.is_empty() {
                        self.log.push("No named views to render.".into());
                    }
                    for (name, _) in self.named_views.clone() {
                        let path = std::path::Path::new(&dir).join(format!("{}.png", name.replace('/', "_"))).to_string_lossy().to_string();
                        self.capture_requests.push(CaptureRequest { path, view: Some(name), ..Default::default() });
                    }
                }
                AppRequest::RefreshTextures => self.refresh_textures = true,
                AppRequest::HistoryUpdate(ids) => {
                    let records: Vec<tri_doc::history::HistoryRecord> = self.doc.state.history.records.iter().filter(|r| ids.is_empty() || r.outputs.iter().any(|o| ids.contains(o))).cloned().collect();
                    let n = records.len();
                    for r in records {
                        self.replay_record(&r);
                    }
                    let cages = self.doc.state.history.cages.clone();
                    for c in &cages {
                        self.apply_cage(c);
                    }
                    self.log.push(format!("{n} history records updated."));
                }
                AppRequest::Gradient(g) => {
                    self.viewports[self.active].gradient = g;
                }
                AppRequest::LookAt { from, to } => {
                    let v = &mut self.viewports[self.active];
                    v.history.push(&v.camera.clone());
                    v.camera.target = to;
                    let d = from - to;
                    v.camera.distance = d.length().max(0.01);
                    v.camera.yaw = d.y.atan2(d.x);
                    v.camera.pitch = (d.z / d.length().max(1e-9)).clamp(-1.0, 1.0).asin();
                }
                AppRequest::Sun(dir) => {
                    self.sun = dir.map(|(az, alt)| {
                        let (sa, ca) = az.to_radians().sin_cos();
                        let (sl, cl) = alt.to_radians().sin_cos();
                        DVec3::new(sa * cl, ca * cl, sl).normalize()
                    });
                    self.log.push(if self.sun.is_some() { "Sun on.".into() } else { "Sun off.".into() });
                }
                AppRequest::GroundPlane(h) => {
                    self.ground_plane = h;
                    self.doc.touch_revision();
                }
                AppRequest::Turntable { frames, record } => {
                    self.setup_animation("turntable", frames, &[], &[], record);
                    self.animation.playing = true;
                }
                AppRequest::ViewportOp(op) => self.viewport_op(&op),
                AppRequest::AnimSetup { kind, frames, path, targets, record } => self.setup_animation(&kind, frames, &path, &targets, record),
                AppRequest::AnimControl(op, arg) => {
                    let n = self.animation.frames.len();
                    if n == 0 {
                        self.log.push("No animation is set up (Turntable, SetPathAnimation, SetFlythroughAnimation, SetOneDaySunAnimation).".into());
                    } else {
                        match op.as_str() {
                            "play" => {
                                self.animation.record_dir = None;
                                self.animation.index = 0;
                                self.animation.playing = true;
                            }
                            "record" => {
                                let dir = std::env::temp_dir().join("triceratops-animation");
                                let _ = std::fs::create_dir_all(&dir);
                                self.animation.record_dir = Some(dir.to_string_lossy().to_string());
                                self.animation.index = 0;
                                self.animation.playing = true;
                                self.log.push(format!("Recording frames to {}", dir.display()));
                            }
                            "first" => self.apply_animation_frame(0),
                            "last" => self.apply_animation_frame(n - 1),
                            "next" => self.apply_animation_frame((self.animation.index + 1) % n),
                            "prev" => self.apply_animation_frame((self.animation.index + n - 1) % n),
                            _ => self.apply_animation_frame((arg.max(0) as usize).min(n - 1)),
                        }
                    }
                }
                AppRequest::Guide(a, b) => match (a, b) {
                    (Some(a), Some(b)) => self.guides.push((a, b)),
                    (Some(a), None) => {
                        // remove the guide nearest to the point
                        if let Some((i, _)) = self.guides.iter().enumerate().map(|(i, (p, q))| { let d = (*q - *p).normalize_or(DVec3::X); (i, ((a - *p) - d * (a - *p).dot(d)).length()) }).min_by(|x, y| x.1.partial_cmp(&y.1).unwrap()) {
                            self.guides.remove(i);
                        }
                    }
                    _ => self.guides.clear(),
                },
                AppRequest::LookAtTarget => self.target_to_objects(),
                AppRequest::LayoutOp { op, name } => self.layout_op(&op, &name),
                AppRequest::ThemeOp(op) => self.theme_op(&op),
                AppRequest::LayoutFile { op, path } => self.layout_file(&op, &path),
                AppRequest::GumballOp { op, points, value } => self.gumball_op(&op, &points, &value),
                AppRequest::LayerState { op, name } => self.layer_state(&op, &name),
                AppRequest::NamedSelection { op, name } => self.named_selection(&op, &name),
                AppRequest::CPlaneOp(op) => self.cplane_op(&op),
                AppRequest::ViewportsFile { path, save } => self.viewports_file(&path, save),
                AppRequest::Worksession { op, path } => self.worksession(&op, &path),
                AppRequest::RunLines(lines) => {
                    for line in lines {
                        self.cmd_text = line;
                        self.submit();
                        self.process_requests();
                    }
                }
                AppRequest::RunPython(path) => match std::process::Command::new("python3").arg(&path).output() {
                    Ok(out) => {
                        let err = String::from_utf8_lossy(&out.stderr).to_string();
                        if !err.trim().is_empty() {
                            self.log.push(format!("python3: {}", err.trim()));
                        }
                        let lines: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
                        for line in lines {
                            self.cmd_text = line;
                            self.submit();
                            self.process_requests();
                        }
                    }
                    Err(e) => self.log.push(format!("Unable to run python3: {e}")),
                },
                AppRequest::ExportSelected { ids, path } => self.export_ids(&ids, &path),
                AppRequest::NamedCPlane { name, plane: p } => {
                    self.memory.insert(format!("NamedCPlane.{name}"), format!("{} {} {} {} {} {} {} {} {}", p.origin.x, p.origin.y, p.origin.z, p.x_axis.x, p.x_axis.y, p.x_axis.z, p.y_axis.x, p.y_axis.y, p.y_axis.z));
                }
                AppRequest::NamedSectionView { name, plane: p } => {
                    let mut cam = Camera::top();
                    let bb = self.doc.bounding_box();
                    let d = if bb.is_empty() { 50.0 } else { bb.diagonal() };
                    cam.target = p.origin;
                    cam.distance = d;
                    cam.rotatable = true;
                    cam.up_override = Some(p.y_axis);
                    cam.yaw = p.z_axis.y.atan2(p.z_axis.x);
                    cam.pitch = p.z_axis.z.clamp(-1.0, 1.0).asin();
                    cam.ortho_height = d;
                    self.named_views.retain(|(n, _)| n != &name);
                    self.named_views.push((name, cam));
                }
                AppRequest::AddClippingPlane(id) => {
                    self.clipping_planes.push(ClipPlane { object: id, enabled: true, flipped: false });
                    self.sync_clipping_memory();
                }
                AppRequest::SetClippingPlanes { ids, enabled, flip } => {
                    for cp in self.clipping_planes.iter_mut() {
                        if ids.is_empty() || ids.contains(&cp.object) {
                            if flip {
                                cp.flipped = !cp.flipped;
                            } else {
                                cp.enabled = enabled;
                            }
                        }
                    }
                    self.sync_clipping_memory();
                }
                AppRequest::RunCommandFile(path) => self.run_command_file(&path),
                AppRequest::Revert => {
                    if let Some(p) = self.doc.path.clone() {
                        self.open_path(p);
                    } else {
                        self.log.push("The document has not been saved yet.".into());
                    }
                }
                AppRequest::ToggleUi(what) => match what.as_str() {
                    "right_sidebar" => self.toggle_side(crate::ui::dock::Placement::Right),
                    "left_sidebar" => self.toggle_side(crate::ui::dock::Placement::Left),
                    "viewport_tabs" => self.show_viewport_tabs = !self.show_viewport_tabs,
                    _ => {}
                },
                AppRequest::CPlaneToView => {
                    let v = &mut self.viewports[self.active];
                    let (right, up) = v.camera.basis();
                    let plane = Plane { origin: v.camera.target, x_axis: right, y_axis: up, z_axis: right.cross(up) };
                    v.set_cplane(Some(plane));
                }
                AppRequest::NextViewport(forward) => {
                    if forward {
                        self.next_viewport();
                    } else {
                        let vis = self.visible_viewports();
                        if let Some(i) = vis.iter().position(|v| *v == self.active) {
                            self.active = vis[(i + vis.len() - 1) % vis.len()];
                        }
                    }
                }
            }
        }
    }

    pub fn finish_loft(&mut self, ok: bool) {
        let Some(d) = self.loft_dialog.take() else { return };
        if !ok {
            self.log.push("Loft cancelled.".into());
            return;
        }
        let mut curves: Vec<tri_geom::NurbsCurve> = d.ids.iter().filter_map(|id| self.doc.get(*id)).filter_map(|o| o.geometry.as_curve().cloned()).collect();
        if curves.len() < 2 {
            return;
        }
        // align curve directions to the first curve
        for i in 1..curves.len() {
            let prev_start = curves[i - 1].start_point();
            if (curves[i].end_point() - prev_start).length() < (curves[i].start_point() - prev_start).length() {
                curves[i].reverse();
            }
        }
        if d.closed {
            let first = curves[0].clone();
            curves.push(first);
        }
        if d.rebuild == 1 {
            let n = d.rebuild_count.max(2);
            curves = curves
                .iter()
                .filter_map(|c| {
                    let (t0, t1) = c.domain();
                    let pts: Vec<DVec3> = (0..n).map(|i| c.point_at(t0 + (t1 - t0) * i as f64 / (n - 1) as f64)).collect();
                    tri_geom::NurbsCurve::from_control_points(3.min(n - 1), pts).ok()
                })
                .collect();
        }
        let degree_v = match d.style {
            3 => 1,
            _ => 3,
        };
        let result = NurbsSurface::loft(&curves, degree_v).map(|mut s| {
            if d.style == 4 {
                // uniform: replace v knots with a uniform clamped vector
                s.knots_v = tri_geom::basis::uniform_clamped_knots(s.count_v, s.degree_v);
            }
            s
        });
        match result {
            Ok(s) => {
                self.doc.begin_undo("Loft");
                let id = self.doc.add(Geometry::Surface(s));
                self.doc.clear_selection();
                self.doc.select(id, true);
                self.last_added = vec![id];
            }
            Err(e) => self.log.push(format!("Loft failed: {e}")),
        }
    }

    fn file_new(&mut self) {
        self.doc = Document::new();
        self.log.push("New document".into());
        self.zoom_extents(false, true);
    }

    fn file_open(&mut self) {
        if self.file_dialog_blocked("Open") { return; }
        let Some(path) = Self::import_dialog(true).pick_file() else {
            return;
        };
        self.open_path(path);
    }

    /// File dialog listing every readable format (native ones first when opening).
    fn import_dialog(open: bool) -> rfd::FileDialog {
        let mut all: Vec<&str> = if open { vec!["3dm", io::NATIVE_EXTENSION, "dxf", "dwg", "obj"] } else { vec!["dxf", "dwg", "obj", "3dm", io::NATIVE_EXTENSION] };
        all.extend(tri_doc::formats::import_extensions());
        let mut d = rfd::FileDialog::new().add_filter("All supported", &all);
        if open {
            d = d.add_filter("3D Models (*.3dm)", &["3dm"]).add_filter("Triceratops Models (*.tri)", &[io::NATIVE_EXTENSION]);
        }
        d = d.add_filter("AutoCAD Drawing Exchange (*.dxf)", &["dxf"]).add_filter("AutoCAD Drawing (*.dwg)", &["dwg"]).add_filter("Wavefront OBJ (*.obj)", &["obj"]);
        if !open {
            d = d.add_filter("3D Models (*.3dm)", &["3dm"]).add_filter("Triceratops Models (*.tri)", &[io::NATIVE_EXTENSION]);
        }
        for f in tri_doc::formats::FORMATS.iter().filter(|f| f.import) {
            d = d.add_filter(format!("{} (*.{})", f.name, f.extensions.join(", *.")), f.extensions);
        }
        d
    }

    fn import_into(&mut self, path: &std::path::Path) -> anyhow::Result<usize> {
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "dxf" | "dwg" => tri_doc::dxf::import(&mut self.doc, path),
            "obj" => io::import_obj(&mut self.doc, path),
            e if tri_doc::formats::format_for(e).is_some_and(|f| f.import) || tri_doc::formats::unsupported_reason(e).is_some() => tri_doc::formats::import(&mut self.doc, path),
            "3dm" => {
                let (state, _settings, skipped) = tri_doc::io3dm::load(path)?;
                let n = state.objects.len();
                let base = self.doc.layers().len();
                // merge layers and objects into the current document
                for l in &state.layers {
                    if self.doc.find_layer(&l.name).is_none() {
                        let i = self.doc.add_layer(l.name.clone());
                        if let Some(nl) = self.doc.layer_mut(i) {
                            *nl = l.clone();
                        }
                    }
                }
                let _ = base;
                for o in state.objects.values() {
                    let layer = state.layers.get(o.attributes.layer).and_then(|l| self.doc.find_layer(&l.name)).unwrap_or(0);
                    self.doc.add_with(o.geometry.clone(), tri_doc::Attributes { layer, ..o.attributes.clone() });
                }
                if skipped > 0 {
                    self.log.push(format!("{skipped} objects of unsupported types were skipped."));
                }
                Ok(n)
            }
            other => anyhow::bail!("unsupported file type: .{other}"),
        }
    }

    pub fn open_path(&mut self, path: PathBuf) {
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if ext == "3dm" {
            match tri_doc::io3dm::load(&path) {
                Ok((state, settings, skipped)) => {
                    let n = state.objects.len();
                    self.doc.load_state(state, settings, Some(path.clone()));
                    self.log.push(format!("Opened {} ({n} objects)", path.display()));
                    if skipped > 0 {
                        self.log.push(format!("{skipped} objects of unsupported types were skipped."));
                    }
                    self.zoom_extents(false, true);
                }
                Err(e) => self.log.push(format!("Open failed: {e:#}")),
            }
            return;
        }
        if ext != io::NATIVE_EXTENSION {
            self.doc = Document::new();
            match self.import_into(&path) {
                Ok(n) => {
                    self.log.push(format!("Opened {} ({n} objects)", path.display()));
                    self.doc.modified = false;
                    self.zoom_extents(false, true);
                }
                Err(e) => self.log.push(format!("Open failed: {e:#}")),
            }
            return;
        }
        match io::load(&path) {
            Ok((state, settings)) => {
                let n = state.objects.len();
                self.doc.load_state(state, settings, Some(path.clone()));
                self.log.push(format!("Opened {} ({n} objects)", path.display()));
                self.zoom_extents(false, true);
            }
            Err(e) => self.log.push(format!("Open failed: {e:#}")),
        }
    }

    fn file_save(&mut self, save_as: bool) {
        let native = self.doc.path.as_ref().is_some_and(|p| {
            let e = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            e == "3dm" || e == io::NATIVE_EXTENSION
        });
        let path = match (&self.doc.path, save_as || !native) {
            (Some(p), false) => p.clone(),
            _ => {
                if self.file_dialog_blocked(if save_as { "SaveAs" } else { "Save" }) { return; }
                let Some(p) = rfd::FileDialog::new()
                    .add_filter("3D Models (*.3dm)", &["3dm"])
                    .add_filter("Triceratops JSON Models (*.tri)", &[io::NATIVE_EXTENSION])
                    .set_file_name("Untitled.3dm")
                    .save_file()
                else {
                    return;
                };
                p
            }
        };
        let path = if path.extension().is_none() { path.with_extension("3dm") } else { path };
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let result = if ext == "3dm" { tri_doc::io3dm::save(&self.doc, &path, false).map(|_| ()) } else { io::save(&self.doc, &path) };
        match result {
            Ok(()) => {
                self.doc.path = Some(path.clone());
                self.doc.modified = false;
                self.log.push(format!("File successfully written as {}", path.display()));
            }
            Err(e) => self.log.push(format!("Save failed: {e:#}")),
        }
    }

    fn filter_hits(&self, hits: Vec<crate::render::scene::PickHit>) -> Vec<crate::render::scene::PickHit> {
        let Some(f) = &self.selection_filter else { return hits };
        hits.into_iter()
            .filter(|h| {
                self.doc.get(h.id).is_some_and(|o| {
                    matches!(
                        (f.as_str(), &o.geometry),
                        ("curve", Geometry::Curve(_)) | ("surface", Geometry::Surface(_)) | ("surface", Geometry::Brep(_)) | ("polysurface", Geometry::Brep(_)) | ("mesh", Geometry::Mesh(_)) | ("point", Geometry::Point(_))
                    )
                })
            })
            .collect()
    }

    /// Save or copy a captured viewport image.
    pub fn finish_capture(&mut self, cr: &CaptureRequest, w: u32, h: u32, data: Vec<u8>) {
        let Some(img) = image::RgbaImage::from_raw(w, h, data) else {
            if let Some(id) = cr.memory {
                self.finished_captures.push((id, Err("the capture failed".into())));
            }
            self.log.push("Capture failed.".into());
            return;
        };
        if let Some(id) = cr.memory {
            let img = match cr.max_size {
                Some((mw, mh)) if img.width() > mw || img.height() > mh => {
                    image::DynamicImage::ImageRgba8(img).resize(mw.max(1), mh.max(1), image::imageops::FilterType::Triangle).into_rgba8()
                }
                _ => img,
            };
            let mut png = Vec::new();
            let result = img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map(|_| png).map_err(|e| format!("cannot encode the capture: {e}"));
            self.finished_captures.push((id, result));
            return;
        }
        let path = if cr.clipboard {
            std::env::temp_dir().join("triceratops-capture.png")
        } else if cr.path.is_empty() {
            if self.file_dialog_blocked("ViewCaptureToFile") { return; }
            match rfd::FileDialog::new().add_filter("PNG image (*.png)", &["png"]).set_file_name("capture.png").save_file() {
                Some(p) => p,
                None => return,
            }
        } else {
            PathBuf::from(&cr.path)
        };
        let path = if path.extension().is_none() { path.with_extension("png") } else { path };
        match img.save(&path) {
            Ok(()) => {
                if cr.clipboard {
                    let copied = std::fs::File::open(&path).ok().and_then(|f| std::process::Command::new("wl-copy").arg("-t").arg("image/png").stdin(f).spawn().ok()).is_some();
                    self.log.push(if copied { "Image copied to the clipboard.".into() } else { format!("Clipboard tool not available; image saved to {}", path.display()) });
                } else {
                    self.log.push(format!("Image saved to {}", path.display()));
                }
            }
            Err(e) => self.log.push(format!("Unable to save image: {e}")),
        }
    }

    fn gumball_op(&mut self, op: &str, points: &[DVec3], value: &str) {
        match op {
            "relocate" => {
                if let Some(o) = points.first() {
                    self.gumball_origin = Some(*o);
                }
                if points.len() > 1 {
                    let x = (points[1] - points[0]).normalize_or(DVec3::X);
                    let cp = self.cplane();
                    let z = if x.cross(cp.z_axis).length() > 1e-6 { cp.z_axis } else { cp.y_axis };
                    let y = z.cross(x).normalize();
                    self.gumball_axes = Some([x, y, x.cross(y)]);
                }
                self.gumball_align = "Object".into();
            }
            "reset" => {
                self.gumball_origin = None;
                self.gumball_axes = None;
            }
            "alignment" => {
                if !value.is_empty() {
                    self.gumball_align = value.to_string();
                    self.gumball_axes = None;
                }
            }
            "scale_mode" => {
                self.memory.insert("Gumball.ScaleMode".into(), value.to_string());
            }
            "auto_reset" => {
                self.gumball_auto_reset = match value { "On" => true, "Off" => false, _ => !self.gumball_auto_reset };
                self.log.push(format!("Gumball auto reset {}.", if self.gumball_auto_reset { "on" } else { "off" }));
            }
            _ => {}
        }
        self.doc.touch_revision();
    }

    fn layer_state(&mut self, op: &str, name: &str) {
        match op {
            "save" => {
                self.layer_states.retain(|(n, _)| n != name);
                self.layer_states.push((name.to_string(), self.doc.layers().to_vec()));
                self.log.push(format!("Layer state \"{name}\" saved."));
            }
            "restore" => {
                let Some((_, layers)) = self.layer_states.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).cloned() else {
                    self.log.push(format!("No layer state named \"{name}\"."));
                    return;
                };
                self.doc.begin_undo("LayerStateManager");
                for l in layers {
                    if let Some(i) = self.doc.find_layer(&l.name) {
                        if let Some(cur) = self.doc.layer_mut(i) {
                            cur.visible = l.visible;
                            cur.locked = l.locked;
                            cur.color = l.color;
                            cur.linetype = l.linetype;
                        }
                    }
                }
                self.doc.touch_revision();
            }
            "delete" => self.layer_states.retain(|(n, _)| !n.eq_ignore_ascii_case(name)),
            "list" => {
                let names: Vec<&str> = self.layer_states.iter().map(|(n, _)| n.as_str()).collect();
                self.log.push(if names.is_empty() { "No layer states saved.".into() } else { format!("Layer states: {}", names.join(", ")) });
            }
            book => {
                // LayerBook: show one layer at a time, in order
                let n = self.doc.layers().len();
                if n == 0 {
                    return;
                }
                self.doc.begin_undo("LayerBook");
                match book {
                    "book_next" => self.layer_book_index = (self.layer_book_index + 1) % n,
                    "book_previous" => self.layer_book_index = (self.layer_book_index + n - 1) % n,
                    "book_first" => self.layer_book_index = 0,
                    _ => {
                        for i in 0..n {
                            if let Some(l) = self.doc.layer_mut(i) {
                                l.visible = true;
                            }
                        }
                        self.doc.touch_revision();
                        return;
                    }
                }
                for i in 0..n {
                    if let Some(l) = self.doc.layer_mut(i) {
                        l.visible = i == self.layer_book_index;
                    }
                }
                self.doc.touch_revision();
                let name = self.doc.layers()[self.layer_book_index].name.clone();
                self.log.push(format!("LayerBook page {}: {name}", self.layer_book_index + 1));
            }
        }
    }

    fn named_selection(&mut self, op: &str, name: &str) {
        match op {
            "save" => {
                let ids = self.doc.selected_ids();
                self.named_selections.retain(|(n, _)| n != name);
                self.named_selections.push((name.to_string(), ids.clone()));
                self.log.push(format!("Named selection \"{name}\" saved ({} objects).", ids.len()));
            }
            "restore" => {
                let Some((_, ids)) = self.named_selections.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).cloned() else {
                    self.log.push(format!("No named selection \"{name}\"."));
                    return;
                };
                self.doc.clear_selection();
                for id in ids {
                    self.doc.select(id, true);
                }
            }
            "delete" => self.named_selections.retain(|(n, _)| !n.eq_ignore_ascii_case(name)),
            _ => {
                let names: Vec<&str> = self.named_selections.iter().map(|(n, _)| n.as_str()).collect();
                self.log.push(if names.is_empty() { "No named selections.".into() } else { format!("Named selections: {}", names.join(", ")) });
            }
        }
    }

    fn cplane_op(&mut self, op: &str) {
        match op {
            "copy_all" => {
                let p = self.viewports[self.active].cplane_override;
                for v in self.viewports.iter_mut() {
                    v.set_cplane(p);
                }
                self.log.push("Construction plane copied to all viewports.".into());
            }
            "sync" => {
                let p = Some(self.viewports[self.active].cplane());
                for v in self.viewports.iter_mut() {
                    v.set_cplane(p);
                }
                self.doc.touch_revision();
            }
            "auto_align" | "one_view" => {
                // align the construction plane to the view: the plane faces the camera
                let v = &mut self.viewports[self.active];
                let (r, u) = v.camera.basis();
                let plane = Plane::from_frame(v.camera.target, r, u);
                v.set_cplane(Some(plane));
                if op == "one_view" {
                    self.layout = Layout::Single;
                    self.log.push("OneView: the construction plane follows the view (run CPlane World to stop).".into());
                }
                self.doc.touch_revision();
            }
            "mplane_detach" => {
                self.mplane_object = None;
                self.log.push("Mobile construction plane detached.".into());
            }
            "mplane_refresh" => {
                if let Some(id) = self.mplane_object {
                    if let Some(o) = self.doc.get(id) {
                        let bb = self.doc.object_bbox(o);
                        let cp = self.cplane();
                        let v = &mut self.viewports[self.active];
                        v.set_cplane(Some(Plane { origin: bb.center(), ..cp }));
                        self.doc.touch_revision();
                    }
                }
            }
            other => {
                if let Some(id) = other.strip_prefix("mplane_attach:").and_then(|s| s.parse::<ObjectId>().ok()) {
                    self.mplane_object = Some(id);
                    self.cplane_op("mplane_refresh");
                    self.log.push(format!("Construction plane attached to object #{id} (MPlane Refresh after moving it)."));
                }
            }
        }
    }

    fn viewports_file(&mut self, path: &str, save: bool) {
        if save {
            let data: Vec<ViewportSave> = self.viewports.iter().map(|v| ViewportSave { name: v.name.clone(), camera: v.camera.clone(), page: v.page.clone(), detail_of: v.detail_of.map(|p| self.viewports[p].name.clone()), detail_rect: v.detail_rect }).collect();
            match serde_json::to_string_pretty(&data).map_err(|e| e.to_string()).and_then(|s| std::fs::write(path, s).map_err(|e| e.to_string())) {
                Ok(()) => self.log.push(format!("{} viewports saved to {path}.", data.len())),
                Err(e) => self.log.push(format!("Unable to write {path}: {e}")),
            }
            return;
        }
        match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|s| serde_json::from_str::<Vec<ViewportSave>>(&s).map_err(|e| e.to_string())) {
            Ok(data) => {
                for vs in data {
                    if let Some(v) = self.viewports.iter_mut().find(|v| v.name.eq_ignore_ascii_case(&vs.name)) {
                        v.camera = vs.camera;
                        continue;
                    }
                    let mut v = Viewport::new(&vs.name, vs.camera, self.viewports[self.active].display);
                    v.rect = self.viewports[self.active].rect;
                    v.page = vs.page.clone();
                    v.detail_rect = vs.detail_rect;
                    if let Some(pn) = &vs.detail_of {
                        v.detail_of = self.viewports.iter().position(|p| p.name.eq_ignore_ascii_case(pn));
                        if v.detail_of.is_none() {
                            continue;
                        }
                    }
                    let is_detail = v.is_detail();
                    self.viewports.push(v);
                    if !is_detail {
                        self.viewport_tab_order.push(self.viewports.len() - 1);
                    }
                    if !is_detail && vs.page.is_none() {
                        self.layout = Layout::Grid;
                    }
                }
                self.sync_batch_vectors();
                self.log.push(format!("Viewports read from {path}."));
            }
            Err(e) => self.log.push(format!("Unable to read {path}: {e}")),
        }
    }

    fn worksession(&mut self, op: &str, path: &str) {
        match op {
            "attach" => {
                let before: std::collections::BTreeSet<ObjectId> = self.doc.objects().map(|o| o.id).collect();
                match self.import_into(std::path::Path::new(path)) {
                    Ok(n) => {
                        let new: Vec<ObjectId> = self.doc.objects().map(|o| o.id).filter(|id| !before.contains(id)).collect();
                        for id in new {
                            if let Some(o) = self.doc.get_mut(id) {
                                o.attributes.locked = true;
                                o.attributes.user_text.push(("Reference".into(), path.to_string()));
                            }
                        }
                        self.doc.touch_revision();
                        self.log.push(format!("{n} reference objects attached from {path} (locked; PurgeRefObjects removes them)."));
                    }
                    Err(e) => self.log.push(format!("Unable to attach {path}: {e:#}")),
                }
            }
            "detach" => {
                let ids: Vec<ObjectId> = self.doc.objects().filter(|o| o.attributes.user_text.iter().any(|(k, _)| k == "Reference")).map(|o| o.id).collect();
                self.doc.begin_undo("Worksession");
                for id in &ids {
                    self.doc.remove(*id);
                }
                self.log.push(format!("{} reference objects detached.", ids.len()));
            }
            _ => {
                let mut files: Vec<String> = self.doc.objects().filter_map(|o| o.attributes.user_text.iter().find(|(k, _)| k == "Reference").map(|(_, v)| v.clone())).collect();
                files.sort();
                files.dedup();
                self.log.push(if files.is_empty() { "No reference files attached.".into() } else { format!("Attached: {}", files.join(", ")) });
            }
        }
    }

    fn export_ids(&mut self, ids: &[ObjectId], path: &str) {
        let mut tmp = Document::new();
        tmp.settings = self.doc.settings.clone();
        tmp.state.layers = self.doc.state.layers.clone();
        tmp.state.blocks = self.doc.state.blocks.clone();
        for id in ids {
            if let Some(o) = self.doc.get(*id) {
                tmp.add_with(o.geometry.clone(), o.attributes.clone());
            }
        }
        let p = std::path::Path::new(path);
        let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let r = match ext.as_str() {
            "dxf" => tri_doc::dxf::export(&tmp, p, false).map_err(|e| format!("{e:#}")),
            "obj" => io::export_obj(&tmp, p, false).map_err(|e| format!("{e:#}")),
            _ => tri_doc::io3dm::save(&tmp, p, false).map_err(|e| format!("{e:#}")),
        };
        match r {
            Ok(n) => self.log.push(format!("{n} objects exported to {path}")),
            Err(e) => self.log.push(format!("Export failed: {e}")),
        }
    }

    fn set_option(&mut self, key: &str, value: &str) {
        let toggle = |cur: bool| match value { "On" => true, "Off" => false, _ => !cur };
        match key {
            "ortho_angle" => {
                if let Ok(v) = value.parse::<f64>() {
                    self.aids.ortho_angle = v.abs().max(1.0);
                }
            }
            "fov" => {
                if let Ok(v) = value.parse::<f64>() {
                    self.viewports[self.active].camera.fov_y = v.clamp(1.0, 170.0).to_radians();
                }
            }
            "lens" => {
                if let Ok(v) = value.parse::<f64>() {
                    self.viewports[self.active].camera.set_lens_length(v.max(1.0));
                }
            }
            "snap" => self.aids.grid_snap = toggle(self.aids.grid_snap),
            "ortho" => self.aids.ortho = toggle(self.aids.ortho),
            "planar" => self.aids.planar = toggle(self.aids.planar),
            "gumball" => self.aids.gumball = toggle(self.aids.gumball),
            "osnap" => self.aids.osnap.disable = !toggle(!self.aids.osnap.disable),
            "active_viewport" => {
                if let Some(i) = self.viewports.iter().position(|v| v.name.eq_ignore_ascii_case(value)) {
                    self.active = i;
                } else {
                    self.log.push(format!("No viewport named \"{value}\"."));
                }
            }
            "record_history" => {
                self.aids.record_history = toggle(self.aids.record_history);
                self.memory.insert("RecordHistory".into(), if self.aids.record_history { "Yes".into() } else { "No".into() });
                self.log.push(if self.aids.record_history { "History recording on.".into() } else { "History recording off.".into() });
            }
            "mapping_widget" => {
                self.mapping_widgets = value == "On";
                self.doc.touch_revision();
            }
            "show_edges" => self.show_edges = toggle(self.show_edges),
            "show_ends" => self.show_ends = toggle(self.show_ends),
            "show_dir" => self.show_dir = toggle(self.show_dir),
            "show_camera" => self.show_camera = toggle(self.show_camera),
            "walk" => {
                self.walk_mode = toggle(self.walk_mode);
                self.log.push(if self.walk_mode { "WalkAbout: W/S move forward/back, A/D strafe, Q/E up/down, arrows turn. Run WalkAbout again to leave.".into() } else { "WalkAbout off.".into() });
            }
            "command_area" => self.show_command_area = toggle(self.show_command_area),
            "render_mesh_hidden" => {
                self.render_mesh_hidden = toggle(self.render_mesh_hidden);
                self.doc.touch_revision();
            }
            "print_display" => {
                self.print_display = toggle(self.print_display);
                self.doc.touch_revision();
                self.log.push(if self.print_display { "Print display on: curves are drawn with their print widths.".into() } else { "Print display off.".into() });
            }
            "alerter" => self.memory.insert("Alerter".into(), value.into()).map(|_| ()).unwrap_or(()),
            "drag_strength" => {
                if let Ok(v) = value.parse::<f64>() {
                    self.drag_strength = (v / 100.0).clamp(0.01, 10.0);
                }
            }
            "drag_mode" => {
                self.drag_mode = value.to_string();
                self.log.push(format!("Drag mode: {value}."));
            }
            "zoom_border" => {
                if let Ok(v) = value.parse::<f64>() {
                    self.zoom_border = v.max(0.0) / 100.0;
                }
            }
            "px_per_mm" => {
                if let Ok(v) = value.parse::<f64>() {
                    self.px_per_mm = v.max(0.1);
                }
            }
            "zoom_1to1" => {
                let v = &mut self.viewports[self.active];
                if v.camera.projection == crate::camera::Projection::Parallel {
                    let units_per_mm = match self.doc.settings.units.to_ascii_lowercase().as_str() { "centimeters" => 0.1, "meters" => 0.001, "inches" => 1.0 / 25.4, "feet" => 1.0 / 304.8, _ => 1.0 };
                    v.camera.ortho_height = v.rect.h * self.px_per_mm.recip() * units_per_mm;
                } else {
                    self.log.push("Zoom 1To1 needs a parallel projection viewport.".into());
                }
            }
            "tilt" => {
                if let Ok(v) = value.parse::<f64>() {
                    let cam = &mut self.viewports[self.active].camera;
                    cam.roll += v.to_radians();
                }
            }
            "dolly" => {
                if let Ok(v) = value.parse::<f64>() {
                    self.viewports[self.active].camera.dolly(v.max(0.01));
                }
            }
            "rotate_camera" => {
                let parts: Vec<f64> = value.split(',').filter_map(|x| x.parse().ok()).collect();
                if parts.len() == 2 {
                    self.viewports[self.active].camera.rotate_camera(parts[0].to_radians(), parts[1].to_radians());
                }
            }
            "lock_viewport" => {
                let v = &mut self.viewports[self.active];
                v.camera.rotatable = !v.camera.rotatable;
                self.log.push(if v.camera.rotatable { "Viewport unlocked.".into() } else { "Viewport locked.".into() });
            }
            _ => self.log.push(format!("Unknown option {key}")),
        }
    }

    fn export_with_origin(&mut self, ids: &[ObjectId], origin: DVec3) {
        if self.file_dialog_blocked("ExportWithOrigin") { return; }
        let Some(p) = rfd::FileDialog::new().add_filter("3D Models (*.3dm)", &["3dm"]).set_file_name("Export.3dm").save_file() else { return };
        let mut tmp = Document::new();
        tmp.settings = self.doc.settings.clone();
        tmp.state.layers = self.doc.state.layers.clone();
        tmp.state.blocks = self.doc.state.blocks.clone();
        let xf = DAffine3::from_translation(-origin);
        for id in ids {
            if let Some(o) = self.doc.get(*id) {
                let mut g = o.geometry.clone();
                g.transform(&xf);
                tmp.add_with(g, o.attributes.clone());
            }
        }
        match tri_doc::io3dm::save(&tmp, &p, false) {
            Ok(n) => self.log.push(format!("{n} objects exported to {}", p.display())),
            Err(e) => self.log.push(format!("Export failed: {e:#}")),
        }
    }

    fn options_file(&mut self, export: bool) {
        if self.file_dialog_blocked(if export { "OptionsExport" } else { "OptionsImport" }) {
            return;
        }
        let Some(src) = options_path() else { return };
        if export {
            self.save_options();
            if let Some(p) = rfd::FileDialog::new().add_filter("Options (*.json)", &["json"]).set_file_name("triceratops-options.json").save_file() {
                match std::fs::copy(&src, &p) {
                    Ok(_) => self.log.push(format!("Options exported to {}", p.display())),
                    Err(e) => self.log.push(format!("Export failed: {e}")),
                }
            }
        } else if let Some(p) = rfd::FileDialog::new().add_filter("Options (*.json)", &["json"]).pick_file() {
            match std::fs::copy(&p, &src) {
                Ok(_) => {
                    self.load_options();
                    self.log.push("Options imported.".into());
                }
                Err(e) => self.log.push(format!("Import failed: {e}")),
            }
        }
    }

    fn export_blocks(&mut self) {
        if self.doc.state.blocks.is_empty() {
            self.log.push("No block definitions in the document.".into());
            return;
        }
        if self.file_dialog_blocked("ExportBlocks") { return; }
        let Some(dir) = rfd::FileDialog::new().pick_folder() else { return };
        for b in &self.doc.state.blocks {
            let mut tmp = Document::new();
            tmp.settings = self.doc.settings.clone();
            tmp.state.layers = self.doc.state.layers.clone();
            for (g, a) in &b.objects {
                tmp.add_with(g.clone(), a.clone());
            }
            let p = dir.join(format!("{}.3dm", b.name.replace('/', "_")));
            match tri_doc::io3dm::save(&tmp, &p, false) {
                Ok(_) => self.log.push(format!("Block \"{}\" written to {}", b.name, p.display())),
                Err(e) => self.log.push(format!("Block \"{}\": {e:#}", b.name)),
            }
        }
    }

    /// Control point selection operations (SelU/SelV/NextU/...).
    fn grip_op(&mut self, op: &str) {
        if self.grips.is_empty() && op != "ShowPt" {
            if !self.doc.selection.is_empty() && (op == "SelControlPoint" || op == "EditPtOn") {
                let ids = self.doc.selected_ids();
                self.points_on = ids;
                self.rebuild_grips(true);
            } else {
                self.log.push("No control points are on. Use PointsOn first.".into());
                return;
            }
        }
        let layout = |app: &App, g: &Grip| -> (usize, usize, usize) {
            match app.doc.get(g.object).map(|o| &o.geometry) {
                Some(Geometry::Surface(s)) => (g.index / s.count_v, g.index % s.count_v, s.count_v),
                _ => (g.index, 0, 1),
            }
        };
        let selected: Vec<(ObjectId, usize, usize)> = self.grips.iter().filter(|g| g.selected).map(|g| { let (i, j, _) = layout(self, g); (g.object, i, j) }).collect();
        let step = |app: &mut App, du: i64, dv: i64, add: bool| {
            let targets: Vec<(ObjectId, usize, usize)> = selected.iter().map(|(o, i, j)| (*o, (*i as i64 + du).max(0) as usize, (*j as i64 + dv).max(0) as usize)).collect();
            if !add {
                for g in app.grips.iter_mut() {
                    g.selected = false;
                }
            }
            let infos: Vec<(usize, ObjectId, usize, usize)> = app.grips.iter().enumerate().map(|(k, g)| { let (i, j, _) = layout(app, g); (k, g.object, i, j) }).collect();
            for (k, o, i, j) in infos {
                if targets.contains(&(o, i, j)) {
                    app.grips[k].selected = true;
                }
            }
        };
        match op {
            "SelControlPoint" | "EditPtOn" => self.grips.iter_mut().for_each(|g| g.selected = true),
            "InvertPt" => self.grips.iter_mut().for_each(|g| g.selected = !g.selected),
            "SelU" | "SelV" | "SelUV" => {
                let infos: Vec<(usize, ObjectId, usize, usize)> = self.grips.iter().enumerate().map(|(k, g)| { let (i, j, _) = layout(self, g); (k, g.object, i, j) }).collect();
                for (k, o, i, j) in infos {
                    let hit = selected.iter().any(|(so, si, sj)| *so == o && ((op != "SelV" && *si == i) || (op != "SelU" && *sj == j)));
                    if hit {
                        self.grips[k].selected = true;
                    }
                }
            }
            "SelConnected" => {
                step(self, 1, 0, true);
                step(self, -1, 0, true);
                step(self, 0, 1, true);
                step(self, 0, -1, true);
            }
            "NextU" => step(self, 1, 0, false),
            "PrevU" => step(self, -1, 0, false),
            "NextV" => step(self, 0, 1, false),
            "PrevV" => step(self, 0, -1, false),
            "AddNextU" => step(self, 1, 0, true),
            "AddPrevU" => step(self, -1, 0, true),
            "AddNextV" => step(self, 0, 1, true),
            "AddPrevV" => step(self, 0, -1, true),
            "HidePt" => self.grips.retain(|g| !g.selected),
            "ShowPt" => self.rebuild_grips(false),
            "SelControlPointRegion" => self.grips.iter_mut().for_each(|g| g.selected = true),
            _ => {}
        }
        let n = self.grips.iter().filter(|g| g.selected).count();
        self.log.push(format!("{n} control points selected."));
    }

    /// Run every non-empty line of a text file as command-line input (`//` starts a comment).
    /// Run command-line entries as if typed: an empty line (or "_Enter") presses Enter in
    /// the running command, "_Cancel" presses Esc, and lines starting with "//" are comments.
    pub fn run_script_lines(&mut self, lines: &[String]) {
        for (k, line) in lines.iter().map(|l| l.trim()).enumerate() {
            if line.starts_with("//") {
                continue;
            }
            if self.runner.job().is_some() {
                if line.eq_ignore_ascii_case("_cancel") || line.eq_ignore_ascii_case("_escape") {
                    self.cancel();
                    continue;
                }
                // the rest runs when the job has ended (`poll_job`)
                self.pending_lines = lines[k..].to_vec();
                self.pending_assistant = self.assistant_mode;
                return;
            }
            if line.eq_ignore_ascii_case("_cancel") || line.eq_ignore_ascii_case("_escape") {
                self.cancel();
                continue;
            }
            if line.is_empty() || line.eq_ignore_ascii_case("_enter") {
                if self.runner.is_active() {
                    self.submit_enter();
                    self.process_requests();
                }
                continue;
            }
            self.cmd_text = line.to_string();
            self.submit();
            self.process_requests();
        }
    }

    /// In assistant mode a file picker would wait for a person: refuse instead.
    fn file_dialog_blocked(&mut self, command: &str) -> bool {
        if self.assistant_mode {
            self.log.push(format!("{command} needs a file path when run by an assistant"));
        }
        self.assistant_mode
    }

    pub fn run_command_file(&mut self, path: &str) {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
                self.log.push(format!("Reading command file {path} ({} lines)", lines.len()));
                self.run_script_lines(&lines);
            }
            Err(e) => self.log.push(format!("Unable to read command file {path}: {e}")),
        }
    }

    /// SaveACopy (dialog) or IncrementalSave (name001.3dm next to the current file); the document path is unchanged.
    fn file_save_copy(&mut self, incremental: bool) {
        let path = if incremental {
            let Some(cur) = self.doc.path.clone() else {
                self.file_save(true);
                return;
            };
            let stem = cur.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled".into());
            let ext = cur.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "3dm".into());
            let base = stem.trim_end_matches(|c: char| c.is_ascii_digit()).to_string();
            let mut n = 1;
            loop {
                let candidate = cur.with_file_name(format!("{base}{n:03}.{ext}"));
                if !candidate.exists() {
                    break candidate;
                }
                n += 1;
            }
        } else {
            if self.file_dialog_blocked("SaveACopy") { return; }
            let Some(p) = rfd::FileDialog::new().add_filter("3D Models (*.3dm)", &["3dm"]).add_filter("Triceratops JSON Models (*.tri)", &[io::NATIVE_EXTENSION]).set_file_name("Copy.3dm").save_file() else {
                return;
            };
            p
        };
        let path = if path.extension().is_none() { path.with_extension("3dm") } else { path };
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let result = if ext == "3dm" { tri_doc::io3dm::save(&self.doc, &path, false).map(|_| ()) } else { io::save(&self.doc, &path) };
        match result {
            Ok(()) => self.log.push(format!("File successfully written as {}", path.display())),
            Err(e) => self.log.push(format!("Save failed: {e:#}")),
        }
    }

    fn file_import(&mut self) {
        if self.file_dialog_blocked("Import") { return; }
        let Some(path) = Self::import_dialog(false).pick_file() else {
            return;
        };
        self.doc.begin_undo("Import");
        match self.import_into(&path) {
            Ok(n) => {
                self.log.push(format!("Imported {n} objects from {}", path.display()));
                self.zoom_extents(false, true);
            }
            Err(e) => self.log.push(format!("Import failed: {e:#}")),
        }
    }

    /// File types offered by Export, in dialog order.
    pub fn export_types() -> Vec<crate::file_dialog::FileType> {
        use crate::file_dialog::FileType;
        let mut types = vec![
            FileType::new("3D Models", &["3dm"]),
            FileType::new("Triceratops", &["tri"]),
            FileType::new("AutoCAD Drawing Exchange", &["dxf"]),
            FileType::new("AutoCAD Drawing", &["dwg"]),
            FileType::new("Wavefront OBJ", &["obj"]),
        ];
        for f in tri_doc::formats::FORMATS.iter().filter(|f| f.export) {
            types.push(FileType::new(f.name, f.extensions));
        }
        types
    }

    fn file_export(&mut self) {
        if self.file_dialog_blocked("Export") {
            return;
        }
        use crate::file_dialog::{save_file, type_for_extension, with_type_extension, SaveRequest};
        let types = Self::export_types();
        let doc_path = self.doc.path.clone();
        let current = type_for_extension(&types, &self.export_memory.last_format).unwrap_or(0);
        let req = SaveRequest {
            title: "Export",
            types: &types,
            current_type: current,
            folder: self.export_memory.folder_for(doc_path.as_deref()),
            name: ExportMemory::name_for(doc_path.as_deref(), &types[current].extensions[0]),
        };
        let Some(choice) = save_file(&req) else { return };
        // the file type picked in the dialog decides the extension (and the writer)
        let t = choice.file_type.unwrap_or(current);
        let path = with_type_extension(&choice.path, &types[t], &types);
        let ext = crate::file_dialog::extension_of(&path).unwrap_or_else(|| types[t].extensions[0].clone());
        // never overwrite the open working file with an export (use Save for that)
        let same = |a: &std::path::Path, b: &std::path::Path| a == b || (a.canonicalize().ok().is_some() && a.canonicalize().ok() == b.canonicalize().ok());
        if doc_path.as_deref().is_some_and(|d| same(d, &path)) {
            self.log.push(format!("Export cancelled: {} is the open working file. Use Save, or export under another name.", path.display()));
            self.message_box = Some("Export would overwrite the open working file.\nUse Save, or choose another name.".into());
            return;
        }
        self.export_to(&path, &ext);
        self.export_memory.remember(doc_path.as_deref(), &path, &ext);
        self.options_dirty = true;
    }

    /// Write the visible (or selected) objects to `path` in the format of `ext`.
    pub fn export_to(&mut self, path: &std::path::Path, ext: &str) {
        let only_selected = !self.doc.selection.is_empty();
        let result = match ext {
            "dxf" | "dwg" => tri_doc::dxf::export(&self.doc, path, only_selected),
            "3dm" => tri_doc::io3dm::save(&self.doc, path, only_selected),
            "obj" => io::export_obj(&self.doc, path, only_selected),
            "tri" => io::save(&self.doc, path).map(|_| self.doc.objects().count()),
            _ => tri_doc::formats::export(&self.doc, path, only_selected),
        };
        match result {
            Ok(n) => self.log.push(format!("Exported {n} objects to {}", path.display())),
            Err(e) => self.log.push(format!("Export failed: {e:#}")),
        }
    }

    /// Text for the status bar Layer pane: current layer, or the selection's layer / "varies".
    pub fn status_layer_text(&self) -> String {
        if self.doc.selection.is_empty() {
            return self.doc.layers()[self.doc.current_layer()].name.clone();
        }
        let mut layer: Option<usize> = None;
        for id in &self.doc.selection {
            if let Some(o) = self.doc.get(*id) {
                match layer {
                    None => layer = Some(o.attributes.layer),
                    Some(l) if l != o.attributes.layer => return "varies".into(),
                    _ => {}
                }
            }
        }
        layer.and_then(|l| self.doc.layers().get(l)).map_or("varies".into(), |l| l.name.clone())
    }

    // ----- batches ---------------------------------------------------------

    fn selection_hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        for id in &self.doc.selection {
            id.hash(&mut h);
        }
        h.finish()
    }

    /// View tolerance tier of the most zoomed-in visible viewport (curves, edges and
    /// isocurves are tessellated to a fraction of its pixel size).
    fn view_tier(&self) -> i32 {
        let wpp = self
            .visible_viewports()
            .into_iter()
            .map(|vp| {
                let v = &self.viewports[vp];
                v.camera.world_per_pixel(v.rect.h)
            })
            .fold(f64::INFINITY, f64::min);
        crate::render::scene::view_tolerance_tier(if wpp.is_finite() { wpp } else { 1.0 })
    }

    fn update_scene(&mut self) {
        let tier = self.view_tier();
        self.scene.update(&self.doc, tier);
    }

    pub fn build_batches(&mut self) -> Vec<bool> {
        self.sync_batch_vectors();
        self.update_scene();
        if self.locust_displays_rev != self.locust.preview_rev || (self.locust_displays_rev == 0 && !self.locust.preview.is_empty()) {
            // the Locust preview: red and translucent like Grasshopper's, or its own colour
            let tier = self.scene.tier.unwrap_or(0);
            self.locust_displays = self
                .locust
                .preview
                .iter()
                .map(|(g, col)| {
                    let (line, fill) = match col {
                        Some(c) => {
                            let rgb = [(c[0].clamp(0.0, 1.0) * 255.0) as u8, (c[1].clamp(0.0, 1.0) * 255.0) as u8, (c[2].clamp(0.0, 1.0) * 255.0) as u8];
                            (lin(rgb), lin_a(rgb, c[3].clamp(0.05, 1.0) as f32))
                        }
                        None => (lin([150, 20, 20]), lin_a([205, 50, 50], 0.55)),
                    };
                    (crate::render::scene::preview_display(g, tier), line, fill)
                })
                .collect();
            self.locust_displays_rev = self.locust.preview_rev;
        }
        if !self.points_on.is_empty() && self.grip_drag.is_none() {
            self.rebuild_grips(false);
        }
        let sel_hash = self.selection_hash();
        let mut changed = vec![false; self.viewports.len()];
        let preview = self.runner.preview(self.cursor.map(|c| c.world), &self.doc);
        for vp in self.visible_viewports() {
            let v = &self.viewports[vp];
            let mut h = DefaultHasher::new();
            self.doc.revision.hash(&mut h);
            self.scene.tier.hash(&mut h);
            self.locust.preview_rev.hash(&mut h);
            sel_hash.hash(&mut h);
            (v.display as u8).hash(&mut h);
            let cp = v.cplane();
            for val in [cp.z_axis.x, cp.z_axis.y, cp.z_axis.z, cp.x_axis.x, cp.x_axis.y, cp.x_axis.z, cp.origin.x, cp.origin.y, cp.origin.z] {
                val.to_bits().hash(&mut h);
            }
            format!("{:?}", self.colors).hash(&mut h);
            let key = h.finish();
            if self.scene_keys[vp] != Some(key) {
                let batch = self.build_scene_batch(vp);
                self.scene_batches[vp] = batch;
                self.scene_keys[vp] = Some(key);
                changed[vp] = true;
            }
            let overlay = self.build_overlay_batch(vp, &preview);
            self.overlay_batches[vp] = overlay;
        }
        changed
    }

    pub fn scene_batch(&self, vp: usize) -> &Batch {
        &self.scene_batches[vp]
    }

    pub fn background_color(&self, vp: usize) -> [u8; 3] {
        let v = &self.viewports[vp];
        if v.display.white_background() {
            [255, 255, 255]
        } else if let Some((top, _)) = v.gradient {
            top
        } else if let (Some((top, _)), true) = (self.theme.viewport_gradient, !v.is_page()) {
            top
        } else {
            self.colors.background
        }
    }

    fn build_scene_batch(&self, vp: usize) -> Batch {
        let v = &self.viewports[vp];
        let mode = v.display;
        let mut b = Batch::default();
        if let Some(p) = &v.page {
            // the paper: a white sheet with a drop shadow, like Triceratops' layout page
            let sheet = tri_geom::Mesh::cuboid(DVec3::new(0.0, 0.0, -0.02), DVec3::new(p.width, p.height, -0.01));
            b.mesh(&sheet, [1.0, 1.0, 1.0, 1.0]);
            let shadow = tri_geom::Mesh::cuboid(DVec3::new(p.width * 0.01, -p.height * 0.01, -0.04), DVec3::new(p.width * 1.01, p.height * 0.99, -0.03));
            b.mesh(&shadow, [0.25, 0.25, 0.25, 1.0]);
            for d in self.details_of(vp) {
                let [x, y, w, h] = self.viewports[d].detail_rect;
                let pts = [DVec3::new(x, y, 0.0), DVec3::new(x + w, y, 0.0), DVec3::new(x + w, y + h, 0.0), DVec3::new(x, y + h, 0.0), DVec3::new(x, y, 0.0)];
                b.polyline(&pts, if self.active == d { [0.9, 0.5, 0.1, 1.0] } else { [0.1, 0.1, 0.1, 1.0] });
            }
        }
        if let (Some(h), true) = (self.ground_plane, mode.shaded()) {
            let bb = self.doc.bounding_box();
            let r = if bb.is_empty() { 50.0 } else { bb.diagonal().max(10.0) * 2.0 };
            let c = if bb.is_empty() { DVec3::ZERO } else { bb.center() };
            let mut m = tri_geom::Mesh::new();
            m.vertices = vec![DVec3::new(c.x - r, c.y - r, h), DVec3::new(c.x + r, c.y - r, h), DVec3::new(c.x + r, c.y + r, h), DVec3::new(c.x - r, c.y + r, h)];
            m.indices = vec![0, 1, 2, 0, 2, 3];
            m.compute_normals();
            b.mesh(&m, [0.6, 0.6, 0.58, 1.0]);
        }
        let mut ordered: Vec<&crate::render::scene::ObjectDisplay> = self.scene.objects.iter().collect();
        ordered.sort_by_key(|od| self.doc.get(od.id).map(|o| o.attributes.draw_order).unwrap_or(0));
        for od in ordered {
            let Some(o) = self.doc.get(od.id) else { continue };
            if !self.doc.is_visible(o) || !self.object_in_viewport(vp, o) {
                continue;
            }
            let mode = o.attributes.user_text.iter().find(|(k, _)| k == "DisplayMode").and_then(|(_, v)| DisplayMode::from_name(v)).unwrap_or(mode);
            let mode = if self.render_mesh_hidden && mode.shaded() && !od.is_curve { DisplayMode::Wireframe } else { mode };
            let selected = self.doc.is_selected(od.id);
            let base: [u8; 3] = if selected {
                self.colors.selected
            } else if self.doc.is_locked(o) {
                self.colors.locked
            } else {
                let c = self.doc.display_color(o);
                [c[0], c[1], c[2]]
            };
            let base = if mode.white_background() && !selected { [0, 0, 0] } else { base };
            let line_color = lin(base);
            let pattern: Vec<f64> = if od.is_curve { self.doc.linetype_pattern(&self.doc.linetype_of(o)).unwrap_or_default() } else { Vec::new() };
            let scale = self.doc.settings.linetype_scale.max(1e-6);
            let wires = |b: &mut Batch, pts: &[DVec3]| {
                let dashed;
                let pts: &[DVec3] = if pattern.is_empty() { pts } else { dashed = dash_segments(pts, &pattern, scale); &dashed };
                if mode == DisplayMode::XRay {
                    for seg in pts.chunks(2) {
                        if seg.len() == 2 {
                            b.xray_line(seg[0], seg[1], line_color);
                        }
                    }
                } else {
                    b.segments(pts, line_color);
                }
            };
            if od.is_curve || mode.isocurves() {
                wires(&mut b, &od.lines);
            }
            wires(&mut b, &od.edges);
            for p in &od.points {
                b.point(*p, 4.0, line_color);
            }
            if mode.shaded() {
                if let Some(m) = &od.mesh {
                    let fill = match mode {
                        DisplayMode::Ghosted => lin_a(if selected { base } else { [230, 230, 230] }, 0.35),
                        DisplayMode::XRay => lin_a(if selected { base } else { [200, 200, 200] }, 0.7),
                        DisplayMode::Pen | DisplayMode::Technical | DisplayMode::Arctic => lin(if selected { base } else { [255, 255, 255] }),
                        DisplayMode::Thickness => {
                            let diag = od.bbox.diagonal().max(1e-6);
                            let uvs: Vec<[f32; 2]> = m.vertices.iter().zip(m.normals.iter()).map(|(p, n)| {
                                let ray = tri_geom::Ray::new(*p - *n * (diag * 1e-5), -*n);
                                let t = m.intersect_ray(&ray).unwrap_or(diag);
                                [(t / diag) as f32, 0.0]
                            }).collect();
                            b.mesh_ex(m, lin([200, 200, 200]), 0.0, Some(&uvs), None);
                            continue;
                        }
                        DisplayMode::Rendered => {
                            let mat = self.doc.material_of(o);
                            let c = if selected { base } else { [mat.color[0], mat.color[1], mat.color[2]] };
                            let mut fill = lin(c);
                            fill[3] = 1.0 - mat.transparency.clamp(0.0, 0.95);
                            if let Some(tex) = mat.texture.clone() {
                                let (kind, scale) = o.attributes.mapping.unwrap_or((tri_doc::MappingKind::Surface, mat.texture_scale));
                                let uvs = texture_uvs(&o.geometry, m, od.bbox, kind, scale.max(1e-9));
                                b.mesh_ex(m, fill, mat.gloss, Some(&uvs), Some(&tex));
                                continue;
                            }
                            b.mesh_ex(m, fill, mat.gloss, None, None);
                            continue;
                        }
                        _ => {
                            if selected {
                                lin(base)
                            } else {
                                // Shaded uses the layer colour; lighten very dark colours a little
                                let c = base;
                                lin([(c[0] as u32 * 60 / 100 + 102) as u8, (c[1] as u32 * 60 / 100 + 102) as u8, (c[2] as u32 * 60 / 100 + 102) as u8])
                            }
                        }
                    };
                    b.mesh(m, fill);
                }
            }
        }
        for (od, line, fill) in &self.locust_displays {
            if od.is_curve || mode.isocurves() {
                b.segments(&od.lines, *line);
            }
            b.segments(&od.edges, *line);
            for p in &od.points {
                b.point(*p, 5.0, *line);
            }
            if mode.shaded() {
                if let Some(m) = &od.mesh {
                    b.mesh(m, *fill);
                }
            }
        }
        b
    }

    /// The construction-plane grid, endless: it is rebuilt for the current view every frame,
    /// covering what the camera sees. Lines too dense to read give way to every `major`-th
    /// one; in perspective views each coarser level reaches further out and fades away
    /// before the horizon.
    fn build_grid(&self, vp: usize, b: &mut Batch) {
        const MIN_PX: f64 = 7.0;
        let v = &self.viewports[vp];
        let cam = &v.camera;
        let cp = v.cplane();
        let s = &self.doc.settings;
        let mode = v.display;
        let every = s.grid_major_every.max(1);
        let ratio = if every >= 2 { every as f64 } else { 10.0 };
        let minor_spacing = s.grid_minor_spacing.max(1e-6);
        let (minor, major) = if mode.white_background() { ([225, 225, 225], [200, 200, 200]) } else { (self.colors.minor_grid, self.colors.major_grid) };
        let (minor, major) = (lin(minor), lin(major));
        let (xcol, ycol) = (lin(self.colors.x_axis), lin(self.colors.y_axis));
        let wpp = cam.world_per_pixel(v.rect.h);
        let to_uv = |p: DVec3| {
            let d = p - cp.origin;
            DVec2::new(d.dot(cp.x_axis), d.dot(cp.y_axis))
        };
        // colour of the grid line `i` steps of `minor_spacing` from the origin
        let colour = |i: i64| if i.rem_euclid(every as i64) == 0 { major } else { minor };
        // levels: (spacing, centre, radius) — lines inside a disc around `centre`
        let mut levels: Vec<(f64, DVec2, f64)> = Vec::new();
        let fade = if cam.projection == Projection::Perspective {
            let eye = cam.eye();
            let height = (eye - cp.origin).dot(cp.z_axis).abs();
            let foot = to_uv(eye);
            let reach = cam.distance * 30.0;
            let mut step = minor_spacing;
            for _ in 0..40 {
                // where this spacing is still MIN_PX on screen
                let r3 = (step * cam.distance / (MIN_PX * wpp)).min(reach);
                if r3 > height {
                    levels.push((step, foot, (r3 * r3 - height * height).sqrt()));
                }
                if r3 >= reach {
                    break;
                }
                step *= ratio;
            }
            true
        } else {
            // the part of the plane the view covers: where the corner rays meet it
            let aspect = v.rect.aspect();
            let (mut lo, mut hi) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
            let centre = to_uv(cam.target);
            let cap = wpp * v.rect.w.hypot(v.rect.h) * 50.0;
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (0.0, 0.0)] {
                let ray = cam.ray_ndc(DVec2::new(x, y), aspect);
                let den = ray.dir.dot(cp.z_axis);
                let q = if den.abs() > 1e-12 {
                    let t = (cp.origin - ray.origin).dot(cp.z_axis) / den;
                    to_uv(ray.at(t))
                } else {
                    centre + to_uv(cp.origin + ray.dir).normalize_or_zero() * cap
                };
                let q = centre + (q - centre).clamp_length_max(cap);
                lo = lo.min(q);
                hi = hi.max(q);
            }
            let mut step = minor_spacing;
            while step / wpp < MIN_PX || (hi - lo).max_element() / step > 3000.0 {
                step *= ratio;
            }
            let c = (lo + hi) * 0.5;
            levels.push((step, c, (hi - lo).length() * 0.5));
            false
        };
        let Some(&(_, outer_c, outer_r)) = levels.last() else { return };
        // a grid line u = c (or v = c) clipped to the disc, faded towards its rim
        let line = |along_v: bool, c: f64, centre: DVec2, r: f64, from: f64, to: f64, col: [f32; 4], b: &mut Batch| {
            let (cc, oc) = if along_v { (centre.x, centre.y) } else { (centre.y, centre.x) };
            let d = c - cc;
            if d.abs() >= r {
                return;
            }
            let half = (r * r - d * d).sqrt();
            let (a, z) = ((oc - half).max(from), (oc + half).min(to));
            if a >= z {
                return;
            }
            let at = |t: f64| if along_v { cp.point_at(c, t) } else { cp.point_at(t, c) };
            if !fade {
                b.line(at(a), at(z), col);
                return;
            }
            let alpha = |t: f64| {
                let rho = (d * d + (t - oc) * (t - oc)).sqrt() / r;
                let k = ((rho - 0.55) / 0.45).clamp(0.0, 1.0);
                1.0 - k * k * (3.0 - 2.0 * k)
            };
            let n = 8;
            for i in 0..n {
                let t0 = a + (z - a) * i as f64 / n as f64;
                let t1 = a + (z - a) * (i + 1) as f64 / n as f64;
                let (c0, c1) = ([col[0], col[1], col[2], col[3] * alpha(t0) as f32], [col[0], col[1], col[2], col[3] * alpha(t1) as f32]);
                b.lines.push(crate::render::LineVertex { pos: at(t0).as_vec3().to_array(), color: c0 });
                b.lines.push(crate::render::LineVertex { pos: at(t1).as_vec3().to_array(), color: c1 });
            }
        };
        if s.show_grid {
            for (k, &(step, centre, r)) in levels.iter().enumerate() {
                let coarser = levels.get(k + 1).map(|l| l.0);
                let m = (step / minor_spacing).round().max(1.0) as i64;
                for along_v in [true, false] {
                    let cc = if along_v { centre.x } else { centre.y };
                    let (i0, i1) = (((cc - r) / step).ceil() as i64, ((cc + r) / step).floor() as i64);
                    for i in i0..=i1 {
                        if i == 0 {
                            continue;
                        }
                        let c = i as f64 * step;
                        // drawn by the coarser level, which reaches further
                        if let Some(cs) = coarser {
                            if ((c / cs).round() * cs - c).abs() < step * 1e-6 {
                                continue;
                            }
                        }
                        line(along_v, c, centre, r, f64::MIN, f64::MAX, colour(i * m), b);
                    }
                }
            }
        }
        if s.show_grid_axes || s.show_grid {
            let (xp, yp) = if s.show_grid_axes { (xcol, ycol) } else { (major, major) };
            line(false, 0.0, outer_c, outer_r, f64::MIN, 0.0, major, b);
            line(false, 0.0, outer_c, outer_r, 0.0, f64::MAX, xp, b);
            line(true, 0.0, outer_c, outer_r, f64::MIN, 0.0, major, b);
            line(true, 0.0, outer_c, outer_r, 0.0, f64::MAX, yp, b);
        }
    }

    fn build_overlay_batch(&self, vp: usize, preview: &[Preview]) -> Batch {
        let mut b = Batch::default();
        let v = &self.viewports[vp];
        if (self.doc.settings.show_grid || self.doc.settings.show_grid_axes) && !v.is_page() {
            self.build_grid(vp, &mut b);
        }
        let col = lin(if v.display.white_background() { [0, 0, 0] } else { self.colors.feedback });
        let tol = v.camera.world_per_pixel(v.rect.h) * 0.5;
        for p in preview {
            match p {
                Preview::Line(a, c) => b.line(*a, *c, col),
                Preview::Polyline(pts) => b.polyline(pts, col),
                Preview::Curve(c) => b.polyline(&c.tessellate_adaptive(tol), col),
                Preview::Surface(s) => {
                    let (mut e, mut i) = (Vec::new(), Vec::new());
                    surface_lines(s, tol, &mut e, &mut i);
                    b.segments(&e, col);
                    b.segments(&i, col);
                }
                Preview::Brep(brep) => {
                    for f in &brep.faces {
                        let (mut e, mut i) = (Vec::new(), Vec::new());
                        crate::render::scene::face_lines(f, tol, &mut e, &mut i);
                        b.segments(&e, col);
                        b.segments(&i, col);
                    }
                }
                Preview::Point(p) => b.point(*p, 4.0, col),
            }
        }
        if let Some(pts) = &self.sketch_points {
            b.polyline(pts, col);
        }
        // construction guides
        for (p, q) in &self.guides {
            let d = (*q - *p).normalize_or(DVec3::X);
            let big = 1e4;
            b.xray_line(*p - d * big, *p + d * big, lin(self.colors.tracking));
        }
        // analysis overlays
        if self.show_edges || self.show_ends || self.show_dir {
            let magenta = lin([255, 0, 255]);
            let dir_col = lin([255, 255, 255]);
            let scale = v.camera.world_per_pixel(v.rect.h) * 25.0;
            for od in &self.scene.objects {
                let Some(o) = self.doc.get(od.id) else { continue };
                if !self.doc.is_visible(o) {
                    continue;
                }
                if self.show_edges {
                    if let Some(m) = &od.mesh {
                        for l in tri_geom::mesh_ops::naked_loops(m) {
                            b.polyline(&l, magenta);
                            if l.len() > 2 {
                                b.line(l[l.len() - 1], l[0], magenta);
                            }
                        }
                    }
                }
                if let Geometry::Curve(c) = &o.geometry {
                    if self.show_ends {
                        b.point(c.start_point(), 6.0, magenta);
                        b.point(c.end_point(), 6.0, magenta);
                    }
                    if self.show_dir {
                        let (t0, t1) = c.domain();
                        for k in 1..4 {
                            let t = t0 + (t1 - t0) * k as f64 / 4.0;
                            let p = c.point_at(t);
                            let d = c.tangent_at(t);
                            let n = v.camera.view_dir().cross(d).normalize_or(DVec3::Y);
                            b.line(p, p + d * scale, dir_col);
                            b.line(p + d * scale, p + d * (scale * 0.6) + n * (scale * 0.25), dir_col);
                            b.line(p + d * scale, p + d * (scale * 0.6) - n * (scale * 0.25), dir_col);
                        }
                    }
                } else if self.show_dir {
                    if let Some(m) = &od.mesh {
                        let step = (m.triangle_count() / 24).max(1);
                        for t in (0..m.triangle_count()).step_by(step) {
                            let (p, q, r) = m.triangle(t);
                            let c = (p + q + r) / 3.0;
                            let n = (q - p).cross(r - p).normalize_or(DVec3::Z);
                            b.line(c, c + n * scale, dir_col);
                        }
                    }
                }
            }
        }
        if self.show_camera && vp != self.active {
            let cam = &self.viewports[self.active].camera;
            let r = &self.viewports[self.active].rect;
            let eye = cam.eye();
            let corners = [(0.0, 0.0), (r.w, 0.0), (r.w, r.h), (0.0, r.h)];
            let far: Vec<DVec3> = corners.iter().map(|(x, y)| { let ray = cam.ray_at(r, r.x + *x, r.y + *y); ray.at(cam.distance * 1.5) }).collect();
            let cc = lin([255, 200, 0]);
            for i in 0..4 {
                b.xray_line(eye, far[i], cc);
                b.xray_line(far[i], far[(i + 1) % 4], cc);
            }
            b.point(eye, 6.0, cc);
            b.point(cam.target, 5.0, cc);
        }
        // texture mapping widgets: bounding boxes of selected objects with a mapping
        if self.mapping_widgets {
            let mcol = lin([230, 120, 30]);
            for id in self.doc.selected_ids() {
                let Some(o) = self.doc.get(id) else { continue };
                if o.attributes.mapping.is_none() && self.doc.material_of(o).texture.is_none() {
                    continue;
                }
                let c = self.doc.object_bbox(o).corners();
                for (i, j) in [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)] {
                    b.xray_line(c[i], c[j], mcol);
                }
            }
        }
        // parametric constraint glyphs (labels at their anchors)
        if self.doc.state.parametric.show_bars && !self.doc.state.parametric.constraints.is_empty() {
            let h = v.camera.world_per_pixel(v.rect.h) * 9.0;
            let plane = self.doc.state.parametric.plane;
            let ok_col = lin([40, 120, 220]);
            let bad_col = lin([220, 60, 40]);
            for (_, label, anchors, satisfied) in self.doc.constraint_glyphs() {
                let Some(a) = anchors.first() else { continue };
                let c = if satisfied { ok_col } else { bad_col };
                b.point(*a, 5.0, c);
                for curve in crate::commands::annotate::text_curves(&label, &plane, *a + plane.x_axis * (h * 0.4) + plane.y_axis * (h * 0.4), h, 0.0) {
                    b.polyline(&curve.control_points, c);
                }
                for q in anchors.iter().skip(1) {
                    b.xray_line(*a, *q, c);
                }
            }
        }
        // drag-move feedback
        if let Some(d) = &self.drag_move {
            let xf = DAffine3::from_translation(d.to - d.from);
            for p in crate::commands::edit::preview_transformed(&self.doc, &d.ids, &xf) {
                match p {
                    Preview::Line(a, c) => b.line(a, c, col),
                    Preview::Curve(c) => b.polyline(&c.tessellate_adaptive(tol), col),
                    Preview::Point(p) => b.point(p, 4.0, col),
                    _ => {}
                }
            }
        }
        // SmartTrack: tracking points and the lines the cursor follows
        if self.cursor.is_some_and(|c| c.viewport == vp) && self.aids.smarttrack && self.wants_point() {
            let track = lin(self.colors.tracking);
            for p in &self.track_points {
                b.point(*p, 5.0, [1.0, 1.0, 1.0, 1.0]);
            }
            let ext = v.camera.world_per_pixel(v.rect.h) * 1e4;
            for (a, p) in &self.track_lines {
                let d = (*p - *a).normalize_or(DVec3::X);
                b.xray_line(*a - d * ext, *a + d * ext, track);
            }
        }
        // tracking lines for constraints
        if let (Some(base), Some(c)) = (self.runner.base_point(), self.cursor) {
            if c.viewport == vp {
                if let Some(e) = self.constraints.elevator {
                    let cp = v.cplane();
                    b.xray_line(e - cp.z_axis * 1e4, e + cp.z_axis * 1e4, lin(self.colors.tracking));
                    b.xray_line(e, c.world, lin(self.colors.tracking));
                }
                if let Some(dir) = self.constraints.direction {
                    b.xray_line(base - dir * 1e4, base + dir * 1e4, lin(self.colors.tracking));
                }
            }
        }
        // control points (PointsOn): control polygon in gray, grips as squares
        if !self.grips.is_empty() {
            let poly_col = lin([150, 150, 150]);
            for id in &self.points_on {
                if let Some(o) = self.doc.get(*id) {
                    match &o.geometry {
                        Geometry::Curve(c) => b.polyline(&c.control_points, poly_col),
                        Geometry::Surface(s) => {
                            for i in 0..s.count_u {
                                let row: Vec<DVec3> = (0..s.count_v).map(|j| s.control_points[i * s.count_v + j]).collect();
                                b.polyline(&row, poly_col);
                            }
                            for j in 0..s.count_v {
                                let colv: Vec<DVec3> = (0..s.count_u).map(|i| s.control_points[i * s.count_v + j]).collect();
                                b.polyline(&colv, poly_col);
                            }
                        }
                        _ => {}
                    }
                }
            }
            let on = lin(self.colors.selected);
            let off = lin(if v.display.white_background() { [0, 0, 0] } else { [255, 255, 255] });
            for g in &self.grips {
                b.point(g.pos, 4.0, if g.selected { on } else { off });
            }
        }
        // world axes icon is drawn by the UI layer
        let _ = Ray::new(DVec3::ZERO, DVec3::Z);
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::Input;

    fn app_with_rect() -> App {
        let mut app = App::new();
        for v in app.viewports.iter_mut() {
            v.rect = ViewRect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };
        }
        app.layout = Layout::Single;
        app.active = 0; // Top view
        app
    }

    fn run(app: &mut App, name: &str, inputs: &[Input]) {
        app.start_command(name);
        for i in inputs {
            app.with_ctx(|r, c| r.feed(i.clone(), c));
        }
        let before: Vec<ObjectId> = Vec::new();
        app.after_command_step(&before);
    }

    fn screen_of(app: &App, p: DVec3) -> DVec2 {
        let v = &app.viewports[app.active];
        v.camera.project(&v.rect, p).unwrap().0
    }

    #[test]
    fn click_selects_closed_polyline() {
        let mut app = app_with_rect();
        let pts = [DVec3::new(0.0, 0.0, 0.0), DVec3::new(10.0, 0.0, 0.0), DVec3::new(10.0, 6.0, 0.0), DVec3::new(0.0, 6.0, 0.0)];
        let mut inputs: Vec<Input> = pts.iter().map(|p| Input::Point(*p)).collect();
        inputs.push(Input::Option("Close".into()));
        run(&mut app, "Polyline", &inputs);
        assert_eq!(app.doc.len(), 1);
        app.zoom_extents(false, true);
        let id = app.doc.objects().next().unwrap().id;
        let s = screen_of(&app, DVec3::new(5.0, 0.0, 0.0));
        app.viewport_click(0, s, false, false);
        assert!(app.selection_menu.is_none(), "single object must not open the selection menu");
        assert!(app.doc.is_selected(id));
        // click on empty space deselects
        // (5,3) is the gumball centre; click away from the gumball handles instead
        let empty = screen_of(&app, DVec3::new(2.5, 1.2, 0.0));
        app.viewport_click(0, empty, false, false);
        assert!(!app.doc.is_selected(id));
        // a tiny drag counts as a click
        app.begin_left_drag(0, s);
        app.update_left_drag(s + DVec2::new(2.0, 1.0));
        app.end_left_drag(false, false);
        assert!(app.doc.is_selected(id));
    }

    #[test]
    fn center_osnap_on_circle_and_closed_polyline() {
        let mut app = app_with_rect();
        run(&mut app, "Circle", &[Input::Point(DVec3::new(3.0, 4.0, 0.0)), Input::Number(2.0)]);
        run(&mut app, "Rectangle", &[Input::Point(DVec3::new(10.0, 10.0, 0.0)), Input::Point(DVec3::new(14.0, 12.0, 0.0))]);
        app.zoom_extents(false, true);
        app.update_scene();
        let s = screen_of(&app, DVec3::new(3.2, 4.1, 0.0));
        let (p, kind) = app.snapped_point(0, s);
        assert_eq!(kind, Some(SnapKind::Cen));
        assert!((p - DVec3::new(3.0, 4.0, 0.0)).length() < 1e-9);
        let s = screen_of(&app, DVec3::new(12.1, 11.0, 0.0));
        let (p, kind) = app.snapped_point(0, s);
        assert_eq!(kind, Some(SnapKind::Cen));
        assert!((p - DVec3::new(12.0, 11.0, 0.0)).length() < 1e-9);
    }

    #[test]
    fn menu_pick_curve_under_surface_edge() {
        let mut app = app_with_rect();
        run(&mut app, "Line", &[Input::Point(DVec3::ZERO), Input::Point(DVec3::new(10.0, 0.0, 0.0))]);
        let curve = app.doc.objects().next().unwrap().id;
        app.doc.select(curve, true);
        run(&mut app, "ExtrudeCrv", &[Input::Number(5.0)]);
        app.doc.clear_selection();
        assert_eq!(app.doc.len(), 2);
        app.active = 2; // Front view: the curve and the bottom edge of the surface coincide
        app.zoom_extents(false, true);
        let s = screen_of(&app, DVec3::new(5.0, 0.0, 0.0));
        app.viewport_click(2, s, false, false);
        let menu = app.selection_menu.clone().expect("menu for coincident curve and edge");
        assert!(menu.candidates.contains(&curve));
        app.selection_menu_pick(Some(curve));
        assert!(app.doc.is_selected(curve));
        assert!(app.selection_menu.is_none());
        let changed = app.build_batches();
        assert!(changed[2]);
    }

    #[test]
    fn selection_menu_for_overlapping_curves() {
        let mut app = app_with_rect();
        run(&mut app, "Line", &[Input::Point(DVec3::ZERO), Input::Point(DVec3::new(10.0, 0.0, 0.0))]);
        run(&mut app, "Line", &[Input::Point(DVec3::ZERO), Input::Point(DVec3::new(10.0, 0.0, 0.0))]);
        app.zoom_extents(false, true);
        let s = screen_of(&app, DVec3::new(5.0, 0.0, 0.0));
        app.viewport_click(0, s, false, false);
        let menu = app.selection_menu.clone().expect("menu");
        assert_eq!(menu.candidates.len(), 2);
        let pick = menu.candidates[1];
        app.selection_menu_pick(Some(pick));
        assert!(app.doc.is_selected(pick));
        assert_eq!(app.doc.selection.len(), 1);
    }
}

#[cfg(test)]
mod export_memory_tests {
    use super::ExportMemory;
    use std::path::Path;

    #[test]
    fn export_defaults_follow_the_working_file() {
        let tmp = std::env::temp_dir();
        let work = tmp.join("triceratops_work").join("bracket.3dm");
        let mut m = ExportMemory::default();
        // unsaved document: no folder yet, name "export"
        assert_eq!(m.folder_for(None), None);
        assert_eq!(ExportMemory::name_for(None, "stl"), "export.stl");
        // saved document: its folder and its name
        assert_eq!(m.folder_for(Some(&work)), work.parent().map(|p| p.to_path_buf()));
        assert_eq!(ExportMemory::name_for(Some(&work), "igs"), "bracket.igs");
        // after an export the folder used is remembered (globally and for that file)
        m.remember(Some(&work), &tmp.join("out.stl"), "stl");
        assert_eq!(m.last_format, "stl");
        assert_eq!(m.folder_for(Some(&work)), Some(tmp.clone()));
        assert_eq!(m.folder_for(None), Some(tmp.clone()));
        // another working file still starts in its own folder
        let other = Path::new("/nonexistent/dir/other.3dm");
        assert_eq!(m.folder_for(Some(other)), Some(Path::new("/nonexistent/dir").to_path_buf()));
    }
}

#[cfg(test)]
mod gumball_tests {
    use super::*;
    use crate::commands::Input;

    #[test]
    fn gumball_move_handle_drags_selection() {
        let mut app = App::new();
        for v in app.viewports.iter_mut() {
            v.rect = ViewRect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };
        }
        app.layout = Layout::Single;
        app.active = 0;
        app.start_command("Point");
        app.with_ctx(|r, c| r.feed(Input::Point(DVec3::new(2.0, 3.0, 0.0)), c));
        let id = app.doc.objects().next().unwrap().id;
        app.doc.select(id, true);
        app.zoom_extents(false, true);
        let g = app.gumball_screen(0).expect("gumball shown for a selection");
        // press on the X arrow shaft (past the scale handle) and drag it 40 px along the arrow
        let mid = g.origin + (g.arrow_end[0] - g.origin) * 0.85;
        app.begin_left_drag(0, mid);
        assert!(app.gumball_drag.is_some());
        let dir = (g.arrow_end[0] - g.origin).normalize();
        app.update_left_drag(mid + dir * 40.0);
        app.end_left_drag(false, false);
        let p = match app.doc.get(id).unwrap().geometry { Geometry::Point(p) => p, _ => unreachable!() };
        assert!(p.x > 2.0 + 1e-6, "moved along +X: {p:?}");
        assert!((p.y - 3.0).abs() < 1e-9 && p.z.abs() < 1e-9);
        assert!(app.doc.can_undo());
    }

    #[test]
    fn page_layouts_with_details_and_spaces() {
        let mut app = App::new();
        for v in app.viewports.iter_mut() {
            v.rect = ViewRect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };
        }
        let before = app.viewports.len();
        app.layout_op("new:297;210;2", "Sheet A");
        assert_eq!(app.viewports.len(), before + 3, "page + two details");
        let page = app.active;
        assert!(app.viewports[page].is_page());
        assert_eq!(app.details_of(page).len(), 2);
        // the visible list contains the page and its details
        let vis = app.visible_viewports();
        assert!(vis.contains(&page) && app.details_of(page).iter().all(|d| vis.contains(d)));
        // the details sit inside the page on screen
        app.viewports[page].rect = ViewRect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };
        let d = app.details_of(page)[0];
        let r = app.detail_screen_rect(d).unwrap();
        assert!(r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= 800.0 && r.y + r.h <= 600.0, "{r:?}");
        // a point drawn while the page is active goes to page space
        app.start_command("Point");
        app.feed_input(Input::Point(DVec3::new(10.0, 10.0, 0.0)));
        let pid = app.doc.objects().next().unwrap().id;
        let o = app.doc.get(pid).unwrap().clone();
        assert!(o.attributes.user_text.iter().any(|(k, v)| k == "Space" && v == "Sheet A"));
        assert!(app.object_in_viewport(page, &o) && !app.object_in_viewport(d, &o) && !app.object_in_viewport(0, &o));
        // a model object is seen by the detail and the model viewports, not by the page
        app.active = 0;
        app.start_command("Point");
        app.feed_input(Input::Point(DVec3::new(1.0, 2.0, 3.0)));
        let mid = app.doc.objects().last().unwrap().id;
        let m = app.doc.get(mid).unwrap().clone();
        assert!(!app.object_in_viewport(page, &m) && app.object_in_viewport(d, &m) && app.object_in_viewport(0, &m));
        // hide it in the detail only
        app.active = d;
        app.doc.select(mid, true);
        app.layout_op("hide_in_detail", "");
        let m = app.doc.get(mid).unwrap().clone();
        assert!(!app.object_in_viewport(d, &m) && app.object_in_viewport(0, &m));
        app.layout_op("show_in_detail", "");
        assert!(app.object_in_viewport(d, &app.doc.get(mid).unwrap().clone()));
        // ChangeSpace to the page and back
        app.active = page;
        app.doc.clear_selection();
        app.doc.select(mid, true);
        app.layout_op("change_space:Page", "");
        assert!(app.object_in_viewport(page, &app.doc.get(mid).unwrap().clone()));
        app.doc.select(mid, true);
        app.layout_op("change_space:Model", "");
        assert!(app.object_in_viewport(0, &app.doc.get(mid).unwrap().clone()));
        // batches build for every visible viewport without panicking
        app.layout = Layout::Single;
        app.build_batches();
        // delete the layout removes page and details
        app.layout_op("delete", "Sheet A");
        assert_eq!(app.viewports.len(), before);
    }

    #[test]
    fn theme_command_switches_ui_and_viewport_colours() {
        let mut app = App::new();
        assert_eq!(app.theme.name, "Triceratops Light");
        let rev = app.theme_revision;
        app.start_command("Theme");
        app.feed_input(Input::Option("Set".into()));
        app.feed_input(Input::Option("Blender".into()));
        app.process_requests();
        assert_eq!(app.theme.name, "Blender");
        assert!(app.dark_mode);
        assert!(app.theme_revision > rev);
        assert_eq!(app.colors.selected, [255, 160, 40], "viewport colours follow the theme");
        assert_eq!(app.background_color(0), [61, 61, 61]);
        assert!(app.gradient_for(0).is_some(), "Blender theme has a GPU gradient background");
        // the egui style follows the theme revision
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |root| crate::ui::draw(&mut app, root));
        out.textures_delta.clear(); // no renderer here: drop the texture uploads on purpose
        assert!(ctx.global_style().visuals.dark_mode);
        assert_eq!(ctx.global_style().visuals.selection.bg_fill, egui::Color32::from_rgb(71, 114, 179));
        // unknown theme is reported, state unchanged
        assert!(!app.set_theme_by_name("Nope"));
        assert_eq!(app.theme.name, "Blender");
        app.set_dark_mode(false);
        assert_eq!(app.theme.name, "Triceratops Light");
        assert!(app.gradient_for(0).is_none());
        // backend option validation
        app.theme_op("backend:gl");
        assert_eq!(app.backend, "gl");
        app.theme_op("backend:dx12");
        assert_eq!(app.backend, "gl");
    }

    /// The Top viewport sized to 800×600, looking at a 10-unit square around the origin.
    fn top_view(app: &mut App) -> usize {
        let vp = app.viewports.iter().position(|v| v.name == "Top").unwrap();
        app.viewports[vp].rect = ViewRect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };
        app.viewports[vp].camera.zoom_to(&BoundingBox::from_points(&[DVec3::ZERO, DVec3::new(10.0, 10.0, 0.0)]));
        vp
    }

    fn screen(app: &App, vp: usize, p: DVec3) -> DVec2 {
        let v = &app.viewports[vp];
        v.camera.project(&v.rect, p).unwrap().0
    }

    #[test]
    fn osnap_finds_the_points_of_a_polyline_in_progress() {
        let mut app = App::new();
        let vp = top_view(&mut app);
        app.aids.osnap.set(SnapKind::End, true);
        app.start_command("Polyline");
        for p in [DVec3::ZERO, DVec3::new(8.0, 0.0, 0.0), DVec3::new(4.0, 6.0, 0.0)] {
            app.with_ctx(|r, c| r.feed(Input::Point(p), c));
        }
        // closing the triangle: the first vertex is still only in the command
        let near_start = screen(&app, vp, DVec3::ZERO) + DVec2::new(4.0, -3.0);
        let (p, kind) = app.snapped_point(vp, near_start);
        assert_eq!(kind, Some(SnapKind::End));
        assert!(p.distance(DVec3::ZERO) < 1e-9);
    }

    #[test]
    fn smarttrack_follows_lines_through_snap_points() {
        let mut app = App::new();
        let vp = top_view(&mut app);
        app.aids.osnap.set(SnapKind::End, true);
        app.aids.smarttrack = true;
        app.start_command("Polyline");
        let (a, b) = (DVec3::new(2.0, 1.0, 0.0), DVec3::new(7.0, 5.0, 0.0));
        app.with_ctx(|r, c| r.feed(Input::Point(a), c));
        app.with_ctx(|r, c| r.feed(Input::Point(b), c));
        // rest on the first vertex: it becomes a tracking point
        app.update_cursor(vp, screen(&app, vp, a));
        assert_eq!(app.track_points, vec![a]);
        // near the vertical through `a`: the point slides onto it
        let off = screen(&app, vp, DVec3::new(2.0, 3.5, 0.0)) + DVec2::new(5.0, 0.0);
        let (p, _) = app.snapped_point(vp, off);
        assert!((p.x - 2.0).abs() < 1e-9 && (p.y - 3.5).abs() < 0.2, "{p}");
        // where the vertical through `a` meets the horizontal through the last point
        let corner = screen(&app, vp, DVec3::new(2.0, 5.0, 0.0)) + DVec2::new(4.0, 3.0);
        let (p, _) = app.snapped_point(vp, corner);
        assert!(p.distance(DVec3::new(2.0, 5.0, 0.0)) < 1e-9, "{p}");
        app.update_cursor(vp, corner);
        assert_eq!(app.track_lines.len(), 2);
    }

    #[test]
    fn guides_named_selections_and_layer_states() {
        let mut app = App::new();
        app.requests.push(AppRequest::Guide(Some(DVec3::ZERO), Some(DVec3::X)));
        app.process_requests();
        assert_eq!(app.guides.len(), 1);
        app.requests.push(AppRequest::Guide(Some(DVec3::new(0.5, 0.1, 0.0)), None));
        app.process_requests();
        assert!(app.guides.is_empty());
        app.start_command("Point");
        app.with_ctx(|r, c| r.feed(Input::Point(DVec3::ZERO), c));
        let id = app.doc.objects().next().unwrap().id;
        app.doc.select(id, true);
        app.named_selection("save", "A");
        app.doc.clear_selection();
        app.named_selection("restore", "A");
        assert!(app.doc.is_selected(id));
        app.doc.add_layer("L2");
        app.layer_state("save", "all on");
        app.doc.layer_mut(1).unwrap().visible = false;
        app.layer_state("restore", "all on");
        assert!(app.doc.layers()[1].visible);
        // UndoSelected restores only the chosen object
        app.doc.begin_undo("move");
        app.doc.transform_objects(&[id], &DAffine3::from_translation(DVec3::X));
        let n = app.doc.undo_selected(&[id]);
        assert_eq!(n, 1);
        assert!(matches!(app.doc.get(id).unwrap().geometry, Geometry::Point(p) if p.length() < 1e-9));
    }
}

#[cfg(test)]
mod grip_tests {
    use super::*;
    use crate::commands::Input;

    #[test]
    fn points_on_and_drag_grip() {
        let mut app = App::new();
        for v in app.viewports.iter_mut() {
            v.rect = ViewRect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };
        }
        app.layout = Layout::Single;
        app.active = 0;
        app.start_command("Line");
        app.with_ctx(|r, c| r.feed(Input::Point(DVec3::ZERO), c));
        app.with_ctx(|r, c| r.feed(Input::Point(DVec3::new(10.0, 0.0, 0.0)), c));
        let id = app.doc.objects().next().unwrap().id;
        app.doc.select(id, true);
        app.start_command("PointsOn");
        app.process_requests();
        assert_eq!(app.grips.len(), 2);
        app.zoom_extents(false, true);
        let (cam, rect) = (app.viewports[0].camera.clone(), app.viewports[0].rect);
        let s = cam.project(&rect, DVec3::new(10.0, 0.0, 0.0)).unwrap().0;
        app.begin_left_drag(0, s);
        let s2 = cam.project(&rect, DVec3::new(10.0, 5.0, 0.0)).unwrap().0;
        app.update_left_drag(s2);
        app.end_left_drag(false, false);
        let c = app.doc.get(id).unwrap().geometry.as_curve().unwrap();
        assert!((c.end_point() - DVec3::new(10.0, 5.0, 0.0)).length() < 1e-6, "end {:?}", c.end_point());
        assert!(app.doc.can_undo());
        app.cancel();
        app.cancel();
        assert!(app.grips.is_empty());
    }
}


/// Dash a sequence of consecutive line segments (pairs of points) with a linetype pattern.
pub fn dash_segments(pts: &[DVec3], pattern: &[f64], scale: f64) -> Vec<DVec3> {
    let pat: Vec<f64> = pattern.iter().map(|d| d * scale).collect();
    let total: f64 = pat.iter().map(|d| d.abs()).sum();
    if total < 1e-9 {
        return pts.to_vec();
    }
    let mut out = Vec::new();
    let mut pi = 0usize; // pattern index
    let mut remaining = pat[0].abs();
    let mut dash = pat[0] >= 0.0;
    for seg in pts.chunks(2) {
        if seg.len() < 2 {
            continue;
        }
        let (a, b) = (seg[0], seg[1]);
        let len = (b - a).length();
        if len < 1e-12 {
            continue;
        }
        let dir = (b - a) / len;
        let mut t = 0.0;
        while t < len {
            let step = remaining.min(len - t);
            if dash && step > 1e-9 {
                out.push(a + dir * t);
                out.push(a + dir * (t + step));
            }
            t += step;
            remaining -= step;
            if remaining <= 1e-9 {
                pi = (pi + 1) % pat.len();
                remaining = pat[pi].abs().max(1e-6);
                dash = pat[pi] >= 0.0;
            }
        }
    }
    out
}

/// Texture coordinates for a display mesh according to a mapping kind.
pub fn texture_uvs(geometry: &Geometry, m: &tri_geom::Mesh, bbox: BoundingBox, kind: tri_doc::MappingKind, scale: f64) -> Vec<[f32; 2]> {
    use tri_doc::MappingKind as K;
    let c = bbox.center();
    let size = (bbox.max - bbox.min).max(DVec3::splat(1e-9));
    let kind = match (kind, geometry) {
        (K::Surface, Geometry::Surface(_)) => K::Surface,
        (K::Surface, _) => K::Box,
        (k, _) => k,
    };
    if kind == K::Surface {
        // grid tessellation: vertices ordered u-major with nv samples per row
        let nv = m.indices.get(1).map(|i| (*i - m.indices[0]) as usize).unwrap_or(1).max(1);
        let nu = (m.vertices.len() / nv).max(1);
        return (0..m.vertices.len()).map(|i| [(i / nv) as f32 / (nu.max(2) - 1) as f32, (i % nv) as f32 / (nv.max(2) - 1) as f32]).collect();
    }
    m.vertices
        .iter()
        .zip(m.normals.iter())
        .map(|(p, n)| {
            let d = *p - c;
            match kind {
                K::Planar => [(d.x / scale) as f32, (d.y / scale) as f32],
                K::Box => {
                    let an = DVec3::new(n.x.abs(), n.y.abs(), n.z.abs());
                    if an.z >= an.x && an.z >= an.y {
                        [(d.x / scale) as f32, (d.y / scale) as f32]
                    } else if an.x >= an.y {
                        [(d.y / scale) as f32, (d.z / scale) as f32]
                    } else {
                        [(d.x / scale) as f32, (d.z / scale) as f32]
                    }
                }
                K::Cylindrical => [(d.y.atan2(d.x) / std::f64::consts::TAU) as f32, (d.z / scale) as f32],
                K::Spherical => {
                    let r = d.length().max(1e-12);
                    [(d.y.atan2(d.x) / std::f64::consts::TAU) as f32, ((d.z / r).clamp(-1.0, 1.0).acos() / std::f64::consts::PI) as f32]
                }
                K::Surface => [(d.x / size.x) as f32, (d.y / size.y) as f32],
            }
        })
        .collect()
}

#[cfg(test)]
mod assistant_tests {
    use super::*;

    fn lines(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn script_lines_build_and_boolean() {
        let mut app = App::new();
        app.run_script_lines(&lines(&["Box", "0,0,0", "10,10,0", "5", "Cylinder", "5,5,-1", "2", "7", "BooleanDifference", "#1", "#2"]));
        app.settle();
        assert_eq!(app.doc.objects().count(), 1);
        assert!(!app.runner.is_active());
    }

    #[test]
    fn assistant_loft_uses_defaults_without_dialog() {
        let mut app = App::new();
        app.assistant_mode = true;
        app.run_script_lines(&lines(&["InterpCrv", "0,0,0", "10,0,2", "20,0,0", "", "InterpCrv", "0,10,1", "10,10,4", "20,10,1", "", "Loft", "#1,#2"]));
        assert!(app.loft_dialog.is_none(), "no dialog for an assistant");
        let surfaces = app.doc.objects().filter(|o| matches!(o.geometry, Geometry::Surface(_) | Geometry::Brep(_))).count();
        assert_eq!(surfaces, 1, "log: {:?}", app.log);
    }

    #[test]
    fn assistant_open_without_path_fails_instead_of_dialog() {
        let mut app = App::new();
        app.assistant_mode = true;
        app.run_script_lines(&lines(&["Open"]));
        assert!(app.log.iter().any(|l| l == "Open needs a file path when run by an assistant"), "log: {:?}", app.log);
        assert!(!app.runner.is_active());
    }

    #[test]
    fn assistant_message_box_goes_to_log() {
        let mut app = App::new();
        app.assistant_mode = true;
        app.requests.push(AppRequest::Message("hi".into()));
        app.process_requests();
        assert!(app.message_box.is_none());
        assert_eq!(app.log.last().map(|s| s.as_str()), Some("hi"));
    }
}
