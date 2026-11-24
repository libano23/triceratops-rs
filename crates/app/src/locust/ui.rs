//! The Locust editor: a canvas of components and wires in a floating window, with the
//! solve / preview / bake cycle.
//!
//! Mouse: drag a node by its body, drag from a port to wire it (a release near a port snaps
//! to it), drag the background to select with a rectangle, middle-drag to pan, wheel or
//! pinch to zoom. Double-click the background to add a component, a cluster to open it, a
//! comment or group to edit it. Right-click for the context menus.
//! Keys (pointer over the canvas): Delete, F (zoom fit), Ctrl+D/C/V/Z/Y/G/E.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{epaint::CubicBezierShape, Align2, Color32, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, Vec2};
use serde_json::{json, Value as J};
use tri_doc::{Document, Geometry};

use super::components::{registry, Ctx, InputSpec};
use super::data::{DataTree, Kind, Value};
use super::engine::{self, Outputs};
use super::example::build_example;
use super::graph::{Connection, Graph, Node};

const HEADER_H: f32 = 24.0;
const ROW_H: f32 = 20.0;
const NODE_W: f32 = 190.0;
const PORT_R: f32 = 6.0;
const BG: Color32 = Color32::from_rgb(0x23, 0x24, 0x27);
const GRID: Color32 = Color32::from_rgb(0x2b, 0x2c, 0x30);
const NODE_BG: Color32 = Color32::from_rgb(0x2e, 0x2f, 0x34);
const NODE_HEAD: Color32 = Color32::from_rgb(0x3a, 0x3b, 0x41);
const NODE_DISABLED: Color32 = Color32::from_rgb(0x26, 0x27, 0x2b);
const NODE_SEL: Color32 = Color32::from_rgb(0x4a, 0x3f, 0x28);
const NODE_BORDER: Color32 = Color32::from_rgb(0x3d, 0x3e, 0x44);
const ACCENT: Color32 = Color32::from_rgb(0xd9, 0xa4, 0x41);
const WIRE: Color32 = Color32::from_rgb(0x8a, 0x8a, 0x90);
const TEXT: Color32 = Color32::from_rgb(0xcf, 0xd0, 0xd2);
const MUTED: Color32 = Color32::from_rgb(0x9a, 0x9b, 0x9e);

/// Where the canvas is kept between sessions (beside the options; the Python Locust uses
/// the same file).
pub fn autosave_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("triceratops").join("locust.locust"))
}

fn parse_color(s: &str, fallback: Color32) -> Color32 {
    let h = s.trim_start_matches('#');
    if h.len() == 6 {
        if let Ok(v) = u32::from_str_radix(h, 16) {
            return Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
        }
    }
    fallback
}

fn lighter(c: Color32, f: f32) -> Color32 {
    let l = |x: u8| ((x as f32 * f).min(255.0)) as u8;
    Color32::from_rgb(l(c.r()), l(c.g()), l(c.b()))
}

/// What a node shows as ports: (name, kind, spec) for inputs, names for outputs.
struct Ports {
    inputs: Vec<(String, Option<InputSpec>)>,
    outputs: Vec<String>,
    cluster: bool,
}

fn ports_of(graph: &Graph, node: &Node) -> Ports {
    if let Some(c) = registry().get(&node.kind) {
        return Ports { inputs: c.inputs.iter().map(|i| (i.name.to_string(), Some(i.clone()))).collect(), outputs: c.outputs.iter().map(|o| o.to_string()).collect(), cluster: false };
    }
    if graph.is_cluster(node) {
        let (ins, outs) = graph.cluster_ports(node);
        return Ports { inputs: ins.into_iter().map(|(n, _)| (n, None)).collect(), outputs: outs.into_iter().map(|(n, _)| n).collect(), cluster: true };
    }
    Ports { inputs: Vec::new(), outputs: Vec::new(), cluster: false }
}

fn in_y(i: usize) -> f32 {
    HEADER_H + ROW_H * i as f32 + ROW_H / 2.0
}

fn out_y(n_in: usize, i: usize) -> f32 {
    HEADER_H + ROW_H * n_in as f32 + 6.0 + ROW_H * i as f32 + ROW_H / 2.0
}

fn node_height(p: &Ports, display: &str) -> f32 {
    let rows = (p.inputs.len() + p.outputs.len() + 1) as f32;
    let extra = if display.is_empty() { 0.0 } else { ROW_H * display.lines().take(8).count() as f32 + 6.0 };
    HEADER_H + ROW_H * rows + 8.0 + extra
}

#[derive(Clone, Debug)]
enum Dialog {
    Search { pos: Pos2, text: String, sel: usize, focus: bool },
    RenameNode(String, String),
    RenameGroup(String, String),
    EditComment(String, String),
    ClusterName(Vec<String>, String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PortRef {
    out: bool,
    index: usize,
}

/// The editor state, kept by the application.
pub struct Editor {
    pub open: bool,
    loaded: bool,
    /// The definition (top level).
    pub root: Graph,
    /// Clusters being edited: (cluster id, nested graph), innermost last.
    stack: Vec<(String, Graph)>,
    selected: HashSet<String>,
    selected_groups: HashSet<String>,
    selected_comments: HashSet<String>,
    selected_wire: Option<Connection>,
    undo: Vec<String>,
    redo: Vec<String>,
    last_tag: Option<String>,
    clipboard: Option<J>,
    scene_rect: Rect,
    fit_pending: bool,
    /// Wire being dragged: from node, port.
    wire: Option<(String, PortRef)>,
    band: Option<Pos2>,
    dialog: Option<Dialog>,
    dirty: Option<Instant>,
    doc_revision: Option<u64>,
    /// Outputs of the graph on screen (the nested one inside a cluster).
    outputs: Outputs,
    displays: HashMap<String, String>,
    /// Geometry shown in the viewports, with an optional RGBA colour.
    pub preview: Vec<(Arc<Geometry>, Option<[f64; 4]>)>,
    /// Bumped whenever `preview` changes (the viewports rebuild their batches).
    pub preview_rev: u64,
    bake: Vec<Arc<Geometry>>,
    status: String,
    /// The pointer is over the editor: keyboard shortcuts go to the canvas.
    pub hovered: bool,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            open: false,
            loaded: false,
            root: Graph::new(),
            stack: Vec::new(),
            selected: HashSet::new(),
            selected_groups: HashSet::new(),
            selected_comments: HashSet::new(),
            selected_wire: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last_tag: None,
            clipboard: None,
            scene_rect: Rect::NOTHING,
            fit_pending: true,
            wire: None,
            band: None,
            dialog: None,
            dirty: None,
            doc_revision: None,
            outputs: HashMap::new(),
            displays: HashMap::new(),
            preview: Vec::new(),
            preview_rev: 0,
            bake: Vec::new(),
            status: String::new(),
            hovered: false,
        }
    }
}

impl Editor {
    /// Open the window; the first time, the canvas of the last session (or the example).
    pub fn show_window(&mut self) {
        if !self.loaded {
            self.loaded = true;
            let saved = autosave_path().and_then(|p| Graph::load(&p).ok()).filter(|g| !g.nodes.is_empty());
            self.root = saved.unwrap_or_else(build_example);
            self.fit_pending = true;
        }
        self.open = true;
        self.schedule();
    }

    /// Close the window: keep the canvas for the next session and clear the preview.
    pub fn close(&mut self) {
        self.open = false;
        self.save_autosave();
        if !self.preview.is_empty() {
            self.preview.clear();
            self.preview_rev += 1;
        }
    }

    pub fn save_autosave(&self) {
        if self.loaded {
            if let Some(p) = autosave_path() {
                let _ = self.root.save(&p);
            }
        }
    }

    // ------------------------------------------------------------ graphs

    pub fn graph(&self) -> &Graph {
        self.stack.last().map(|(_, g)| g).unwrap_or(&self.root)
    }

    fn graph_mut(&mut self) -> &mut Graph {
        match self.stack.last_mut() {
            Some((_, g)) => g,
            None => &mut self.root,
        }
    }

    /// Write the nested graphs back into their parents' cluster definitions.
    fn sync(&mut self) {
        for i in (0..self.stack.len()).rev() {
            let json = self.stack[i].1.to_json();
            let id = self.stack[i].0.clone();
            let parent = if i == 0 { &mut self.root } else { &mut self.stack[i - 1].1 };
            if let Some(def) = parent.clusters.get_mut(&id) {
                def["graph"] = json;
            }
        }
    }

    fn changed(&mut self) {
        self.sync();
        self.schedule();
    }

    fn schedule(&mut self) {
        self.dirty = Some(Instant::now());
    }

    fn snapshot(&mut self) -> String {
        self.sync();
        serde_json::to_string(&self.root.to_json()).unwrap_or_default()
    }

    /// Record an undo step (`tag` coalesces repeated edits of one value, like a slider drag).
    fn record(&mut self, tag: Option<String>) {
        if tag.is_some() && tag == self.last_tag && !self.undo.is_empty() {
            return;
        }
        let s = self.snapshot();
        self.undo.push(s);
        if self.undo.len() > 120 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.last_tag = tag;
    }

    fn restore(&mut self, snap: &str) {
        if let Ok(j) = serde_json::from_str::<J>(snap) {
            self.root = Graph::from_json(&j);
            self.stack.clear();
            self.clear_selection();
            self.schedule();
        }
    }

    pub fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            let cur = self.snapshot();
            self.redo.push(cur);
            self.restore(&s);
            self.last_tag = None;
        }
    }

    pub fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            let cur = self.snapshot();
            self.undo.push(cur);
            self.restore(&s);
            self.last_tag = None;
        }
    }

    fn clear_selection(&mut self) {
        self.selected.clear();
        self.selected_groups.clear();
        self.selected_comments.clear();
        self.selected_wire = None;
    }

    fn set_root(&mut self, g: Graph) {
        self.root = g;
        self.stack.clear();
        self.clear_selection();
        self.fit_pending = true;
        self.schedule();
    }

    // ------------------------------------------------------------ editing

    pub fn add_node(&mut self, kind: &str, pos: Pos2) -> String {
        self.record(None);
        let uid = self.graph_mut().add_node(kind, pos.x as f64, pos.y as f64);
        self.clear_selection();
        self.selected.insert(uid.clone());
        self.changed();
        uid
    }

    fn set_value(&mut self, uid: &str, name: &str, v: J) {
        self.record(Some(format!("value:{uid}:{name}")));
        self.graph_mut().set_value(uid, name, v);
        self.changed();
    }

    pub fn delete_selected(&mut self) {
        if self.selected.is_empty() && self.selected_groups.is_empty() && self.selected_comments.is_empty() && self.selected_wire.is_none() {
            return;
        }
        self.record(None);
        let (nodes, groups, comments, wire) = (self.selected.clone(), self.selected_groups.clone(), self.selected_comments.clone(), self.selected_wire.take());
        let g = self.graph_mut();
        for u in &nodes {
            g.remove_node(u);
        }
        for u in &groups {
            g.remove_group(u);
        }
        for u in &comments {
            g.remove_comment(u);
        }
        if let Some(c) = wire {
            g.disconnect(&c.dst, c.dst_port);
        }
        self.clear_selection();
        self.changed();
    }

    pub fn duplicate_selected(&mut self) {
        let uids: Vec<String> = self.sorted_selection();
        if uids.is_empty() {
            return;
        }
        self.record(None);
        let new = self.graph_mut().duplicate(&uids, 40.0, 40.0);
        self.selected = new.into_iter().collect();
        self.changed();
    }

    fn sorted_selection(&self) -> Vec<String> {
        self.graph().nodes.iter().filter(|n| self.selected.contains(&n.uid)).map(|n| n.uid.clone()).collect()
    }

    pub fn group_selected(&mut self) {
        let uids = self.sorted_selection();
        if uids.is_empty() {
            return;
        }
        self.record(None);
        self.graph_mut().add_group(&uids, "Group");
        self.changed();
    }

    fn cluster(&mut self, uids: Vec<String>, name: &str) {
        if uids.is_empty() {
            return;
        }
        self.record(None);
        if let Some(c) = self.graph_mut().make_cluster(&uids, if name.is_empty() { "Cluster" } else { name }) {
            self.clear_selection();
            self.selected.insert(c);
        }
        self.changed();
    }

    pub fn toggle_disable(&mut self, uids: &[String]) {
        if uids.is_empty() {
            return;
        }
        self.record(None);
        for u in uids {
            if let Some(n) = self.graph_mut().node_mut(u) {
                n.disabled = !n.disabled;
            }
        }
        self.changed();
    }

    pub fn copy(&mut self) {
        let keep: HashSet<String> = self.sorted_selection().into_iter().collect();
        if keep.is_empty() {
            return;
        }
        let j = self.graph().to_json();
        let nodes: Vec<J> = j["nodes"].as_array().cloned().unwrap_or_default().into_iter().filter(|n| n["uid"].as_str().is_some_and(|u| keep.contains(u))).collect();
        let conns: Vec<J> = j["connections"].as_array().cloned().unwrap_or_default().into_iter().filter(|c| c["src"].as_str().is_some_and(|u| keep.contains(u)) && c["dst"].as_str().is_some_and(|u| keep.contains(u))).collect();
        // clusters travel with their definitions
        let clusters: serde_json::Map<String, J> = self.graph().nodes.iter().filter(|n| keep.contains(&n.uid)).filter_map(|n| n.kind.strip_prefix("Cluster:").map(|id| (id.to_string(), self.graph().clusters.get(id).cloned().unwrap_or(J::Null)))).collect();
        self.clipboard = Some(json!({"nodes": nodes, "connections": conns, "clusters": clusters}));
    }

    pub fn paste(&mut self) {
        let Some(clip) = self.clipboard.clone() else { return };
        self.record(None);
        let sub = Graph::from_json(&clip);
        let g = self.graph_mut();
        for (k, v) in &sub.clusters {
            g.clusters.entry(k.clone()).or_insert_with(|| v.clone());
        }
        let mut map = HashMap::new();
        for n in &sub.nodes {
            let uid = g.add_node_with(&n.kind, n.x + 40.0, n.y + 40.0, n.values.clone());
            if let Some(nn) = g.node_mut(&uid) {
                nn.nickname = n.nickname.clone();
                nn.disabled = n.disabled;
            }
            map.insert(n.uid.clone(), uid);
        }
        for c in &sub.connections {
            g.connect(&map[&c.src], c.src_port, &map[&c.dst], c.dst_port);
        }
        self.selected = map.into_values().collect();
        self.changed();
    }

    fn open_cluster(&mut self, uid: &str) {
        let Some(node) = self.graph().node(uid).cloned() else { return };
        let Some(id) = node.kind.strip_prefix("Cluster:").map(String::from) else { return };
        let Some(def) = self.graph().clusters.get(&id) else { return };
        let inner = Graph::from_json(&def["graph"]);
        self.sync();
        self.stack.push((id, inner));
        self.clear_selection();
        self.fit_pending = true;
        self.schedule();
    }

    fn leave_cluster(&mut self) {
        if !self.stack.is_empty() {
            self.sync();
            self.stack.pop();
            self.clear_selection();
            self.fit_pending = true;
            self.schedule();
        }
    }

    fn breadcrumb(&self) -> String {
        let mut parts = vec!["Locust".to_string()];
        let mut g = &self.root;
        for (id, inner) in &self.stack {
            parts.push(g.clusters.get(id).and_then(|d| d["name"].as_str()).unwrap_or("Cluster").to_string());
            g = inner;
        }
        parts.join(" › ")
    }

    // ------------------------------------------------------------ solving

    /// Solve when something changed (debounced) or when the document changed under a
    /// Geometry parameter. Returns true when the preview changed.
    pub fn tick(&mut self, doc: &Document) -> bool {
        if !self.open {
            return false;
        }
        let refs_doc = self.root.nodes.iter().any(|n| n.kind == "Geometry") || !self.root.clusters.is_empty();
        if refs_doc && self.doc_revision != Some(doc.revision) && self.dirty.is_none() {
            self.dirty = Some(Instant::now() - Duration::from_secs(1));
        }
        match self.dirty {
            Some(t) if t.elapsed() >= Duration::from_millis(60) => {
                self.solve(doc);
                true
            }
            _ => false,
        }
    }

    pub fn needs_repaint(&self) -> bool {
        self.dirty.is_some()
    }

    pub fn solve(&mut self, doc: &Document) {
        self.dirty = None;
        self.doc_revision = Some(doc.revision);
        self.sync();
        let mut ctx = Ctx::with_doc(doc);
        let root_out = engine::solve(&self.root, &mut ctx);
        self.preview = ctx.preview;
        self.bake = ctx.bake;
        self.preview_rev += 1;
        // what the canvas shows: the nested graph solves on its own inside a cluster
        self.outputs = if self.stack.is_empty() { root_out } else { engine::solve(self.graph(), &mut Ctx::with_doc(doc)) };
        self.displays = self.graph().nodes.iter().filter(|n| n.kind == "Panel").map(|n| {
            let tree = engine::input_preview(self.graph(), &n.uid, 0, &self.outputs);
            (n.uid.clone(), tree.format(6, 5))
        }).collect();
        self.status = format!("{} components · {} preview object(s) · {} to bake", self.root.nodes.len(), self.preview.len(), self.bake.len());
    }

    /// Put the pending Bake geometry into the document. Returns the number of objects.
    pub fn bake_into(&mut self, doc: &mut Document) -> usize {
        if self.bake.is_empty() {
            self.status = "Nothing to bake — connect an Output › Bake component.".into();
            return 0;
        }
        doc.begin_undo("Locust Bake");
        doc.clear_selection();
        let n = self.bake.len();
        for g in &self.bake {
            let id = doc.add(g.as_ref().clone());
            doc.select(id, true);
        }
        self.status = format!("Baked {n} object(s) into the document.");
        n
    }

    // ------------------------------------------------------------ files

    fn open_file(&mut self) {
        if let Some(p) = rfd::FileDialog::new().set_title("Open Locust definition").add_filter("Locust (*.locust, *.json)", &["locust", "json"]).pick_file() {
            match Graph::load(&p) {
                Ok(g) => {
                    self.record(None);
                    self.set_root(g);
                    self.status = format!("Opened {}", p.display());
                }
                Err(e) => self.status = format!("Cannot open {}: {e}", p.display()),
            }
        }
    }

    fn save_file(&mut self) {
        self.sync();
        if let Some(mut p) = rfd::FileDialog::new().set_title("Save Locust definition").add_filter("Locust (*.locust)", &["locust"]).set_file_name("definition.locust").save_file() {
            if !matches!(p.extension().and_then(|e| e.to_str()), Some("locust" | "json")) {
                p.set_extension("locust");
            }
            self.status = match self.root.save(&p) {
                Ok(()) => format!("Saved {}", p.display()),
                Err(e) => format!("Cannot save {}: {e}", p.display()),
            };
        }
    }
}

// =============================================================== drawing

/// The editor window. `bake` is set when the Bake button was pressed.
pub fn window(ed: &mut Editor, ctx: &egui::Context) -> bool {
    if !ed.open {
        ed.hovered = false;
        return false;
    }
    let mut open = true;
    let mut bake = false;
    let title = ed.breadcrumb();
    let screen = ctx.screen_rect();
    let size = Vec2::new((screen.width() * 0.55).clamp(420.0, 1080.0), (screen.height() * 0.5).clamp(260.0, 740.0));
    let resp = egui::Window::new("Locust")
        .id(egui::Id::new("locust_window"))
        .title_bar(true)
        .open(&mut open)
        .default_size(size)
        .default_pos(screen.right_bottom() - size - Vec2::new(40.0, 80.0))
        .max_size(screen.size() * 0.95)
        .min_size([420.0, 260.0])
        .resizable(true)
        .collapsible(true)
        .show(ctx, |ui| {
            toolbar(ed, ui, &mut bake);
            ui.separator();
            if !ed.stack.is_empty() {
                ui.horizontal(|ui| {
                    if ui.button("⬑ Back").on_hover_text("Leave the cluster").clicked() {
                        ed.leave_cluster();
                    }
                    ui.label(egui::RichText::new(&title).strong());
                });
            }
            let avail = ui.available_size();
            let size = Vec2::new(avail.x.max(100.0), (avail.y - 22.0).max(80.0));
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.set_clip_rect(rect);
                canvas(ed, ui)
            });
            ui.separator();
            ui.label(egui::RichText::new(&ed.status).small().color(MUTED));
        });
    ed.hovered = resp.as_ref().is_some_and(|r| r.response.contains_pointer());
    if ed.hovered {
        shortcuts(ed, ctx);
    }
    dialogs(ed, ctx);
    if !open {
        ed.close();
    }
    if ed.needs_repaint() {
        ctx.request_repaint_after(Duration::from_millis(30));
    }
    bake
}

fn toolbar(ed: &mut Editor, ui: &mut egui::Ui, bake: &mut bool) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("Undo").clicked() {
            ed.undo();
        }
        if ui.button("Redo").clicked() {
            ed.redo();
        }
        ui.separator();
        if ui.button("Solve").clicked() {
            ed.dirty = Some(Instant::now() - Duration::from_secs(1));
        }
        if ui.button("Bake").on_hover_text("Put the geometry reaching Bake components into the document").clicked() {
            *bake = true;
        }
        ui.separator();
        if ui.button("Add component").clicked() {
            let c = ed.scene_rect.center();
            ed.dialog = Some(Dialog::Search { pos: c, text: String::new(), sel: 0, focus: true });
        }
        if ui.button("Add comment").clicked() {
            let c = ed.scene_rect.center();
            ed.record(None);
            ed.graph_mut().add_comment(c.x as f64, c.y as f64, "Comment");
            ed.changed();
        }
        if ui.button("Example").clicked() {
            ed.record(None);
            ed.set_root(build_example());
        }
        if ui.button("Group").clicked() {
            ed.group_selected();
        }
        if ui.button("Cluster").clicked() {
            let uids = ed.sorted_selection();
            if !uids.is_empty() {
                ed.dialog = Some(Dialog::ClusterName(uids, "Cluster".into()));
            }
        }
        if ui.button("Zoom fit").clicked() {
            ed.fit_pending = true;
        }
        ui.separator();
        if ui.button("New").clicked() {
            ed.record(None);
            ed.set_root(Graph::new());
        }
        if ui.button("Open…").clicked() {
            ed.open_file();
        }
        if ui.button("Save…").clicked() {
            ed.save_file();
        }
    });
}

fn shortcuts(ed: &mut Editor, ctx: &egui::Context) {
    if ed.dialog.is_some() || ctx.wants_keyboard_input() {
        return;
    }
    let mut actions: Vec<&str> = Vec::new();
    ctx.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace) {
            actions.push("delete");
        }
        if i.consume_key(Modifiers::NONE, Key::F) {
            actions.push("fit");
        }
        for (k, a) in [(Key::D, "dup"), (Key::C, "copy"), (Key::V, "paste"), (Key::Z, "undo"), (Key::Y, "redo"), (Key::G, "group"), (Key::E, "disable")] {
            if i.consume_key(Modifiers::COMMAND, k) {
                actions.push(a);
            }
        }
    });
    for a in actions {
        match a {
            "delete" => ed.delete_selected(),
            "fit" => ed.fit_pending = true,
            "dup" => ed.duplicate_selected(),
            "copy" => ed.copy(),
            "paste" => ed.paste(),
            "undo" => ed.undo(),
            "redo" => ed.redo(),
            "group" => ed.group_selected(),
            "disable" => {
                let u = ed.sorted_selection();
                ed.toggle_disable(&u);
            }
            _ => {}
        }
    }
}

/// Bounding rectangle of everything on the canvas.
fn content_rect(ed: &Editor) -> Rect {
    let g = ed.graph();
    let mut r = Rect::NOTHING;
    for n in &g.nodes {
        let p = ports_of(g, n);
        let h = node_height(&p, ed.displays.get(&n.uid).map(|s| s.as_str()).unwrap_or(""));
        r = r.union(Rect::from_min_size(Pos2::new(n.x as f32, n.y as f32), Vec2::new(NODE_W, h)));
    }
    for c in &g.comments {
        r = r.union(Rect::from_min_size(Pos2::new(c.x as f32, c.y as f32), Vec2::new(c.w as f32, c.h as f32)));
    }
    r
}

fn node_rect(ed: &Editor, n: &Node) -> Rect {
    let p = ports_of(ed.graph(), n);
    Rect::from_min_size(Pos2::new(n.x as f32, n.y as f32), Vec2::new(NODE_W, node_height(&p, ed.displays.get(&n.uid).map(|s| s.as_str()).unwrap_or(""))))
}

/// Positions of every port, in canvas coordinates.
fn port_positions(ed: &Editor) -> Vec<(String, PortRef, Pos2)> {
    let g = ed.graph();
    let mut out = Vec::new();
    for n in &g.nodes {
        let p = ports_of(g, n);
        let o = Pos2::new(n.x as f32, n.y as f32);
        for i in 0..p.inputs.len() {
            out.push((n.uid.clone(), PortRef { out: false, index: i }, o + Vec2::new(0.0, in_y(i))));
        }
        for i in 0..p.outputs.len() {
            out.push((n.uid.clone(), PortRef { out: true, index: i }, o + Vec2::new(NODE_W, out_y(p.inputs.len(), i))));
        }
    }
    out
}

fn port_at(ports: &[(String, PortRef, Pos2)], pos: Pos2, exclude: Option<(&str, PortRef)>, snap: f32) -> Option<(String, PortRef)> {
    ports
        .iter()
        .filter(|(u, r, _)| exclude.is_none_or(|(eu, er)| !(eu == u && er == *r)))
        .map(|(u, r, p)| (u, r, (p.x - pos.x).abs() + (p.y - pos.y).abs()))
        .filter(|(_, _, d)| *d < snap)
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(u, r, _)| (u.clone(), *r))
}

fn wire_shape(a: Pos2, b: Pos2, stroke: Stroke) -> CubicBezierShape {
    let dx = ((b.x - a.x).abs() * 0.5).max(30.0);
    CubicBezierShape::from_points_stroke([a, a + Vec2::new(dx, 0.0), b - Vec2::new(dx, 0.0), b], false, Color32::TRANSPARENT, stroke)
}

fn near_wire(a: Pos2, b: Pos2, p: Pos2, tol: f32) -> bool {
    let shape = wire_shape(a, b, Stroke::NONE);
    let pts = shape.flatten(Some(0.5));
    pts.windows(2).any(|w| {
        let (s, e) = (w[0], w[1]);
        let d = e - s;
        let t = if d.length_sq() < 1e-6 { 0.0 } else { ((p - s).dot(d) / d.length_sq()).clamp(0.0, 1.0) };
        (s + d * t - p).length() < tol
    })
}

fn canvas(ed: &mut Editor, ui: &mut egui::Ui) {
    if ed.fit_pending {
        let r = content_rect(ed);
        ed.scene_rect = if r.is_positive() { r.expand(60.0) } else { Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0)) };
        ed.fit_pending = false;
    }
    let mut scene_rect = ed.scene_rect;
    let outer = ui.max_rect();
    ui.painter().rect_filled(outer, 0.0, BG);
    let scene = egui::Scene::new().max_inner_size(outer.size()).zoom_range(0.08..=2.5).drag_pan_buttons(egui::containers::DragPanButtons::MIDDLE | egui::containers::DragPanButtons::SECONDARY);
    scene.show(ui, &mut scene_rect, |ui| canvas_contents(ed, ui));
    ed.scene_rect = scene_rect;
    let _ = outer;
}

fn canvas_contents(ed: &mut Editor, ui: &mut egui::Ui) {
    let painter = ui.painter().clone();
    let view = ui.clip_rect();
    let from_global = ui.ctx().layer_transform_from_global(ui.layer_id()).unwrap_or_default();
    let pointer = ui.ctx().pointer_latest_pos().map(|p| from_global * p);
    // grid
    let step = 24.0;
    let mut x = (view.min.x / step).floor() * step;
    while x < view.max.x {
        painter.line_segment([Pos2::new(x, view.min.y), Pos2::new(x, view.max.y)], Stroke::new(1.0_f32, GRID));
        x += step;
    }
    let mut y = (view.min.y / step).floor() * step;
    while y < view.max.y {
        painter.line_segment([Pos2::new(view.min.x, y), Pos2::new(view.max.x, y)], Stroke::new(1.0_f32, GRID));
        y += step;
    }
    // rubber band selection on the background (primary drag)
    let band_resp = ui.interact(view, ui.id().with("band"), Sense::click_and_drag());
    if band_resp.drag_started_by(egui::PointerButton::Primary) {
        ed.band = pointer;
    }
    if band_resp.double_clicked() {
        if let Some(p) = pointer {
            ed.dialog = Some(Dialog::Search { pos: p, text: String::new(), sel: 0, focus: true });
        }
    } else if band_resp.clicked() {
        ed.clear_selection();
    }
    let menu_pos = pointer.unwrap_or(view.center());
    band_resp.context_menu(|ui| {
        if ui.button("Add component…").clicked() {
            ed.dialog = Some(Dialog::Search { pos: menu_pos, text: String::new(), sel: 0, focus: true });
            ui.close();
        }
        if ui.button("Add comment").clicked() {
            ed.record(None);
            ed.graph_mut().add_comment(menu_pos.x as f64, menu_pos.y as f64, "Comment");
            ed.changed();
            ui.close();
        }
        ui.separator();
        if ui.add_enabled(ed.clipboard.is_some(), egui::Button::new("Paste")).clicked() {
            ed.paste();
            ui.close();
        }
        if ui.button("Zoom fit").clicked() {
            ed.fit_pending = true;
            ui.close();
        }
        if !ed.stack.is_empty() && ui.button("Leave cluster").clicked() {
            ed.leave_cluster();
            ui.close();
        }
    });

    groups(ed, ui, pointer);
    comments(ed, ui);
    wires(ed, ui, pointer);
    let uids: Vec<String> = ed.graph().nodes.iter().map(|n| n.uid.clone()).collect();
    for uid in uids {
        node(ed, ui, &uid);
    }
    // dragging a new wire
    if let Some((uid, port)) = ed.wire.clone() {
        let ports = port_positions(ed);
        if let (Some(start), Some(p)) = (ports.iter().find(|(u, r, _)| *u == uid && *r == port).map(|(_, _, p)| *p), pointer) {
            painter.line_segment([start, p], Stroke::new(1.6_f32, ACCENT));
            if let Some((tu, tr)) = port_at(&ports, p, Some((&uid, port)), 24.0) {
                if tr.out != port.out {
                    if let Some((_, _, tp)) = ports.iter().find(|(u, r, _)| *u == tu && *r == tr) {
                        painter.circle_stroke(*tp, PORT_R + 3.0, Stroke::new(2.0_f32, ACCENT));
                    }
                }
            }
        }
        if ui.input(|i| i.pointer.any_released()) {
            if let Some(p) = pointer {
                if let Some((tu, tr)) = port_at(&ports, p, Some((&uid, port)), 24.0) {
                    if tr.out != port.out {
                        let (src, sp, dst, dp) = if port.out { (uid.clone(), port.index, tu, tr.index) } else { (tu, tr.index, uid.clone(), port.index) };
                        ed.record(None);
                        if ed.graph_mut().connect(&src, sp, &dst, dp) {
                            ed.changed();
                        } else {
                            ed.status = "That wire would make a loop.".into();
                        }
                    }
                }
            }
            ed.wire = None;
        }
    }
    // the rubber band
    if let (Some(a), Some(b)) = (ed.band, pointer) {
        let r = Rect::from_two_pos(a, b);
        painter.rect(r, 0.0, Color32::from_rgba_unmultiplied(0xd9, 0xa4, 0x41, 24), Stroke::new(1.0_f32, ACCENT), egui::StrokeKind::Inside);
        if ui.input(|i| i.pointer.any_released()) {
            let add = ui.input(|i| i.modifiers.shift || i.modifiers.command);
            if !add {
                ed.clear_selection();
            }
            let hits: Vec<String> = ed.graph().nodes.iter().filter(|n| node_rect(ed, n).intersects(r)).map(|n| n.uid.clone()).collect();
            ed.selected.extend(hits);
            ed.band = None;
        }
    }
}

fn groups(ed: &mut Editor, ui: &mut egui::Ui, _pointer: Option<Pos2>) {
    let painter = ui.painter().clone();
    let groups = ed.graph().groups.clone();
    for gr in groups {
        let members: Vec<Rect> = ed.graph().nodes.iter().filter(|n| gr.nodes.contains(&n.uid)).map(|n| node_rect(ed, n)).collect();
        if members.is_empty() {
            continue;
        }
        let r = members.iter().fold(Rect::NOTHING, |a, b| a.union(*b));
        let r = Rect::from_min_max(r.min - Vec2::new(14.0, 30.0), r.max + Vec2::new(14.0, 14.0));
        let col = parse_color(&gr.color, Color32::from_rgb(0x4a, 0x3f, 0x28));
        let sel = ed.selected_groups.contains(&gr.uid);
        painter.rect(r, 8.0, lighter(col, 1.1).gamma_multiply(0.6), Stroke::new(1.4_f32, if sel { ACCENT } else { lighter(col, 1.4) }), egui::StrokeKind::Inside);
        let head = Rect::from_min_size(r.min, Vec2::new(r.width(), 20.0));
        painter.rect_filled(head, 8.0, lighter(col, 1.5));
        painter.text(head.left_center() + Vec2::new(8.0, 0.0), Align2::LEFT_CENTER, &gr.name, FontId::proportional(12.0), TEXT);
        let resp = ui.interact(head, ui.id().with(("group", &gr.uid)), Sense::click_and_drag());
        if resp.drag_started() {
            ed.record(None);
        }
        if resp.dragged() {
            let d = resp.drag_delta();
            for m in &gr.nodes {
                if let Some(n) = ed.graph_mut().node_mut(m) {
                    n.x += d.x as f64;
                    n.y += d.y as f64;
                }
            }
        }
        if resp.drag_stopped() {
            ed.sync();
        }
        if resp.clicked() {
            if !ui.input(|i| i.modifiers.shift) {
                ed.clear_selection();
            }
            ed.selected_groups.insert(gr.uid.clone());
        }
        if resp.double_clicked() {
            ed.dialog = Some(Dialog::RenameGroup(gr.uid.clone(), gr.name.clone()));
        }
        resp.context_menu(|ui| {
            if ui.button("Rename…").clicked() {
                ed.dialog = Some(Dialog::RenameGroup(gr.uid.clone(), gr.name.clone()));
                ui.close();
            }
            if ui.button("Ungroup").clicked() {
                ed.record(None);
                ed.graph_mut().remove_group(&gr.uid);
                ed.changed();
                ui.close();
            }
        });
    }
}

fn comments(ed: &mut Editor, ui: &mut egui::Ui) {
    let painter = ui.painter().clone();
    let comments = ed.graph().comments.clone();
    for c in comments {
        let r = Rect::from_min_size(Pos2::new(c.x as f32, c.y as f32), Vec2::new(c.w as f32, c.h as f32));
        let sel = ed.selected_comments.contains(&c.uid);
        painter.rect(r, 5.0, parse_color(&c.color, Color32::from_rgb(0x2b, 0x2c, 0x30)), Stroke::new(1.2_f32, if sel { ACCENT } else { NODE_BORDER }), egui::StrokeKind::Inside);
        let galley = painter.layout(c.text.clone(), FontId::proportional(12.0), TEXT, r.width() - 16.0);
        painter.galley(r.min + Vec2::new(8.0, 6.0), galley, TEXT);
        let resp = ui.interact(r, ui.id().with(("comment", &c.uid)), Sense::click_and_drag());
        if resp.drag_started() {
            ed.record(None);
        }
        if resp.dragged() {
            let d = resp.drag_delta();
            if let Some(cm) = ed.graph_mut().comments.iter_mut().find(|x| x.uid == c.uid) {
                cm.x += d.x as f64;
                cm.y += d.y as f64;
            }
        }
        if resp.drag_stopped() {
            ed.sync();
        }
        if resp.clicked() {
            if !ui.input(|i| i.modifiers.shift) {
                ed.clear_selection();
            }
            ed.selected_comments.insert(c.uid.clone());
        }
        if resp.double_clicked() {
            ed.dialog = Some(Dialog::EditComment(c.uid.clone(), c.text.clone()));
        }
        resp.context_menu(|ui| {
            if ui.button("Edit…").clicked() {
                ed.dialog = Some(Dialog::EditComment(c.uid.clone(), c.text.clone()));
                ui.close();
            }
            if ui.button("Delete").clicked() {
                ed.record(None);
                ed.graph_mut().remove_comment(&c.uid);
                ed.changed();
                ui.close();
            }
        });
    }
}

fn wires(ed: &mut Editor, ui: &mut egui::Ui, pointer: Option<Pos2>) {
    let painter = ui.painter().clone();
    let ports = port_positions(ed);
    let find = |u: &str, out: bool, i: usize| ports.iter().find(|(pu, r, _)| pu == u && r.out == out && r.index == i).map(|(_, _, p)| *p);
    let clicked = ui.input(|i| i.pointer.primary_clicked());
    let conns = ed.graph().connections.clone();
    let mut hit: Option<Connection> = None;
    for c in conns {
        let (Some(a), Some(b)) = (find(&c.src, true, c.src_port), find(&c.dst, false, c.dst_port)) else { continue };
        let sel = ed.selected_wire.as_ref() == Some(&c);
        painter.add(wire_shape(a, b, Stroke::new(if sel { 2.4_f32 } else { 1.6 }, if sel { ACCENT } else { WIRE })));
        if clicked && hit.is_none() && ed.wire.is_none() {
            if let Some(p) = pointer {
                if near_wire(a, b, p, 5.0) {
                    hit = Some(c.clone());
                }
            }
        }
    }
    if let Some(c) = hit {
        // a click on a wire selects it (Delete removes it)
        let over_node = pointer.is_some_and(|p| ed.graph().nodes.iter().any(|n| node_rect(ed, n).expand(PORT_R).contains(p)));
        if !over_node {
            ed.clear_selection();
            ed.selected_wire = Some(c);
        }
    }
}

fn node(ed: &mut Editor, ui: &mut egui::Ui, uid: &str) {
    let Some(n) = ed.graph().node(uid).cloned() else { return };
    let ports = ports_of(ed.graph(), &n);
    let display = ed.displays.get(uid).cloned().unwrap_or_default();
    let rect = node_rect(ed, &n);
    let sel = ed.selected.contains(uid);
    let painter = ui.painter().clone();
    let fill = if sel { NODE_SEL } else if n.disabled { NODE_DISABLED } else { NODE_BG };
    // body interaction first, so the ports and widgets drawn on top take precedence
    let body = ui.interact(rect, ui.id().with(("node", uid)), Sense::click_and_drag());
    painter.rect(rect, 6.0, fill, Stroke::new(1.4_f32, if sel { ACCENT } else { NODE_BORDER }), egui::StrokeKind::Inside);
    let head = Rect::from_min_size(rect.min, Vec2::new(NODE_W, HEADER_H));
    painter.rect_filled(head, egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, if n.disabled { Color32::from_rgb(0x2a, 0x2b, 0x2f) } else { NODE_HEAD });
    let title = if ports.cluster { format!("⊞ {}", n.label()) } else { n.label().to_string() };
    let unknown = ports.inputs.is_empty() && ports.outputs.is_empty() && registry().get(&n.kind).is_none() && !ports.cluster;
    painter.text(head.left_center() + Vec2::new(8.0, 0.0), Align2::LEFT_CENTER, if unknown { format!("{title} (missing)") } else { title }, FontId::proportional(13.0), if n.disabled { MUTED } else { TEXT });
    let font = FontId::proportional(12.0);
    // inputs
    for (i, (name, spec)) in ports.inputs.iter().enumerate() {
        let y = rect.min.y + in_y(i);
        painter.text(Pos2::new(rect.min.x + 12.0, y), Align2::LEFT_CENTER, name, font.clone(), MUTED);
        let connected = ed.graph().source_of(uid, i).is_some();
        if let (Some(spec), false) = (spec, connected) {
            if spec.kind.is_scalar() {
                let wr = Rect::from_min_size(Pos2::new(rect.min.x + 84.0, y - (ROW_H - 4.0) / 2.0), Vec2::new(NODE_W - 92.0, ROW_H - 4.0));
                inline_widget(ed, ui, &n, spec, wr);
            }
        }
    }
    for (i, name) in ports.outputs.iter().enumerate() {
        let y = rect.min.y + out_y(ports.inputs.len(), i);
        painter.text(Pos2::new(rect.max.x - 12.0, y), Align2::RIGHT_CENTER, name, font.clone(), MUTED);
    }
    if !display.is_empty() {
        let base = rect.min.y + HEADER_H + ROW_H * (ports.inputs.len() + ports.outputs.len()) as f32 + 8.0;
        for (k, line) in display.lines().take(8).enumerate() {
            let text: String = line.chars().take(34).collect();
            painter.text(Pos2::new(rect.min.x + 8.0, base + ROW_H * k as f32 + ROW_H / 2.0), Align2::LEFT_CENTER, text, FontId::monospace(11.0), TEXT);
        }
    }
    // ports (drawn and sensed last)
    let wire_target = ed.wire.clone();
    for (k, out) in [(ports.inputs.len(), false), (ports.outputs.len(), true)] {
        for i in 0..k {
            let p = if out { Pos2::new(rect.max.x, rect.min.y + out_y(ports.inputs.len(), i)) } else { Pos2::new(rect.min.x, rect.min.y + in_y(i)) };
            let r = Rect::from_center_size(p, Vec2::splat(PORT_R * 2.0 + 8.0));
            let pr = ui.interact(r, ui.id().with(("port", uid, out, i)), Sense::click_and_drag());
            let hot = pr.hovered() || wire_target.as_ref().is_some_and(|(u, w)| u == uid && w.out == out && w.index == i);
            painter.circle(p, PORT_R, if hot { ACCENT } else if out { Color32::from_rgb(0xc8, 0xc9, 0xcc) } else { Color32::from_rgb(0x8f, 0x90, 0x94) }, Stroke::new(1.0_f32, NODE_BORDER));
            if pr.drag_started() {
                ed.wire = Some((uid.to_string(), PortRef { out, index: i }));
            }
            let tree: DataTree = if out { ed.outputs.get(uid).and_then(|o| o.get(i)).cloned().unwrap_or_default() } else { engine::input_preview(ed.graph(), uid, i, &ed.outputs) };
            let name = if out { ports.outputs.get(i).cloned().unwrap_or_default() } else { ports.inputs.get(i).map(|x| x.0.clone()).unwrap_or_default() };
            let kind = if out { String::new() } else { ports.inputs.get(i).and_then(|x| x.1.as_ref()).map(|s| format!(" ({})", s.kind.name())).unwrap_or_default() };
            let pr = pr.on_hover_text(format!("{name}{kind}\n{} item(s) in {} branch(es)\n{}", tree.count(), tree.branch_count(), tree.format(6, 5)));
            if !out && ed.graph().source_of(uid, i).is_some() {
                pr.context_menu(|ui| {
                    if ui.button("Disconnect").clicked() {
                        ed.record(None);
                        ed.graph_mut().disconnect(uid, i);
                        ed.changed();
                        ui.close();
                    }
                });
            }
        }
    }
    // body: select, drag, double-click, context menu
    if body.drag_started() && ed.wire.is_none() {
        ed.record(None);
        if !sel {
            if !ui.input(|i| i.modifiers.shift) {
                ed.clear_selection();
            }
            ed.selected.insert(uid.to_string());
        }
    }
    if body.dragged() && ed.wire.is_none() {
        let d = body.drag_delta();
        let moving: Vec<String> = if ed.selected.contains(uid) { ed.selected.iter().cloned().collect() } else { vec![uid.to_string()] };
        for m in moving {
            if let Some(nn) = ed.graph_mut().node_mut(&m) {
                nn.x += d.x as f64;
                nn.y += d.y as f64;
            }
        }
    }
    if body.drag_stopped() {
        ed.sync();
    }
    if body.clicked() {
        let shift = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        if shift {
            if !ed.selected.remove(uid) {
                ed.selected.insert(uid.to_string());
            }
        } else {
            ed.clear_selection();
            ed.selected.insert(uid.to_string());
        }
    }
    if body.double_clicked() && ports.cluster {
        ed.open_cluster(uid);
        return;
    }
    let desc = registry().get(&n.kind).map(|c| if c.description.is_empty() { format!("{} · {}", c.category, c.name) } else { format!("{} · {}\n{}", c.category, c.name, c.description) });
    let body = match desc {
        Some(d) => body.on_hover_text(d),
        None => body,
    };
    body.context_menu(|ui| {
        if !ed.selected.contains(uid) {
            ed.clear_selection();
            ed.selected.insert(uid.to_string());
        }
        if ui.button("Delete").clicked() {
            ed.delete_selected();
            ui.close();
        }
        if ui.button("Duplicate").clicked() {
            ed.duplicate_selected();
            ui.close();
        }
        if ui.button(if n.disabled { "Enable" } else { "Disable" }).clicked() {
            let u = ed.sorted_selection();
            ed.toggle_disable(&u);
            ui.close();
        }
        if ui.button("Rename…").clicked() {
            ed.dialog = Some(Dialog::RenameNode(uid.to_string(), n.label().to_string()));
            ui.close();
        }
        ui.separator();
        if ui.button("Group selected").clicked() {
            ed.group_selected();
            ui.close();
        }
        if ui.button("Cluster selected…").clicked() {
            ed.dialog = Some(Dialog::ClusterName(ed.sorted_selection(), "Cluster".into()));
            ui.close();
        }
        if ports.cluster && ui.button("Open cluster").clicked() {
            ed.open_cluster(uid);
            ui.close();
        }
    });
}

/// The value widget of an unconnected scalar input.
fn inline_widget(ed: &mut Editor, ui: &mut egui::Ui, n: &Node, spec: &InputSpec, rect: Rect) {
    let current = n.values.get(spec.name).map(Value::from_json).or_else(|| spec.default.clone()).unwrap_or_default();
    let id = (n.uid.as_str(), spec.name);
    let mut new: Option<J> = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).id_salt(id), |ui| {
        ui.spacing_mut().slider_width = (rect.width() - 52.0).max(30.0);
        ui.spacing_mut().interact_size.y = rect.height();
        match spec.kind {
            Kind::Bool => {
                let mut b = super::data::truthy(&current);
                if ui.checkbox(&mut b, "").changed() {
                    new = Some(json!(b));
                }
            }
            Kind::Int => {
                let mut v = super::data::as_int(&current);
                let w = if let Some((lo, hi)) = spec.slider { ui.add(egui::Slider::new(&mut v, lo as i64..=hi as i64).clamping(egui::SliderClamping::Never)) } else { ui.add_sized(rect.size(), egui::DragValue::new(&mut v).speed(0.1)) };
                if w.changed() {
                    new = Some(json!(v));
                }
            }
            Kind::Text => {
                let mut s = current.to_text();
                if matches!(current, Value::Null) {
                    s.clear();
                }
                if ui.add_sized(rect.size(), egui::TextEdit::singleline(&mut s).font(egui::TextStyle::Small)).changed() {
                    new = Some(json!(s));
                }
            }
            _ => {
                let mut v = super::data::as_number(&current);
                let w = if let Some((lo, hi)) = spec.slider { ui.add(egui::Slider::new(&mut v, lo..=hi).clamping(egui::SliderClamping::Never).max_decimals(3)) } else { ui.add_sized(rect.size(), egui::DragValue::new(&mut v).speed(0.1).max_decimals(4)) };
                if w.changed() {
                    new = Some(json!(v));
                }
            }
        }
    });
    if let Some(v) = new {
        ed.set_value(&n.uid.clone(), spec.name, v);
    }
}

// ------------------------------------------------------------ dialogs

fn dialogs(ed: &mut Editor, ctx: &egui::Context) {
    let Some(d) = ed.dialog.clone() else { return };
    let mut keep = true;
    match d {
        Dialog::Search { pos, mut text, mut sel, focus } => {
            let q = text.to_lowercase();
            let mut names: Vec<(&str, &str)> = registry().components.values().filter(|c| q.is_empty() || c.name.to_lowercase().contains(q.trim()) || c.category.to_lowercase().contains(q.trim())).map(|c| (c.category, c.name)).collect();
            names.sort();
            let mut chosen: Option<String> = None;
            egui::Window::new("Add component").id(egui::Id::new("locust_search")).collapsible(false).resizable(false).default_width(340.0).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                let te = ui.add(egui::TextEdit::singleline(&mut text).hint_text("Search…").desired_width(320.0));
                if focus {
                    te.request_focus();
                }
                ui.input_mut(|i| {
                    if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                        sel = (sel + 1).min(names.len().saturating_sub(1));
                    }
                    if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                        sel = sel.saturating_sub(1);
                    }
                });
                if te.changed() {
                    sel = 0;
                }
                egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                    for (k, (cat, name)) in names.iter().enumerate() {
                        let r = ui.selectable_label(k == sel, format!("{cat}  ·  {name}"));
                        if k == sel {
                            r.scroll_to_me(None);
                        }
                        if r.clicked() {
                            sel = k;
                        }
                        if r.double_clicked() {
                            chosen = Some(name.to_string());
                        }
                    }
                });
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    chosen = names.get(sel).map(|(_, n)| n.to_string());
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        keep = false;
                    }
                    if ui.button("Add").clicked() {
                        chosen = names.get(sel).map(|(_, n)| n.to_string());
                    }
                });
            });
            if let Some(name) = chosen {
                ed.add_node(&name, pos);
                keep = false;
            } else if keep {
                ed.dialog = Some(Dialog::Search { pos, text, sel, focus: false });
            }
        }
        other => {
            let (title, multiline) = match &other {
                Dialog::RenameNode(..) => ("Rename", false),
                Dialog::RenameGroup(..) => ("Group", false),
                Dialog::EditComment(..) => ("Comment", true),
                Dialog::ClusterName(..) => ("Cluster", false),
                Dialog::Search { .. } => unreachable!(),
            };
            let mut text = match &other {
                Dialog::RenameNode(_, s) | Dialog::RenameGroup(_, s) | Dialog::EditComment(_, s) | Dialog::ClusterName(_, s) => s.clone(),
                Dialog::Search { .. } => unreachable!(),
            };
            let mut ok = false;
            egui::Window::new(title).id(egui::Id::new("locust_text_dialog")).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                let te = if multiline { ui.add(egui::TextEdit::multiline(&mut text).desired_rows(5).desired_width(320.0)) } else { ui.add(egui::TextEdit::singleline(&mut text).desired_width(260.0)) };
                if !te.has_focus() && !te.lost_focus() {
                    te.request_focus();
                }
                if !multiline && te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    ok = true;
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        keep = false;
                    }
                    if ui.button("OK").clicked() {
                        ok = true;
                    }
                });
            });
            if ok {
                ed.record(None);
                match &other {
                    Dialog::RenameNode(uid, _) => {
                        let label = text.trim().to_string();
                        if let Some(n) = ed.graph_mut().node_mut(uid) {
                            n.nickname = if label.is_empty() || label == n.kind { None } else { Some(label) };
                        }
                    }
                    Dialog::RenameGroup(uid, _) => {
                        if let Some(g) = ed.graph_mut().groups.iter_mut().find(|g| g.uid == *uid) {
                            g.name = if text.trim().is_empty() { "Group".into() } else { text.trim().to_string() };
                        }
                    }
                    Dialog::EditComment(uid, _) => {
                        if let Some(c) = ed.graph_mut().comments.iter_mut().find(|c| c.uid == *uid) {
                            c.text = text.clone();
                        }
                    }
                    Dialog::ClusterName(uids, _) => {
                        let uids = uids.clone();
                        ed.undo.pop();
                        ed.cluster(uids, text.trim());
                    }
                    Dialog::Search { .. } => {}
                }
                ed.changed();
                keep = false;
            } else if keep {
                ed.dialog = Some(match other {
                    Dialog::RenameNode(u, _) => Dialog::RenameNode(u, text),
                    Dialog::RenameGroup(u, _) => Dialog::RenameGroup(u, text),
                    Dialog::EditComment(u, _) => Dialog::EditComment(u, text),
                    Dialog::ClusterName(u, _) => Dialog::ClusterName(u, text),
                    s => s,
                });
            }
        }
    }
    if !keep {
        ed.dialog = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_undo_redo_disable_copy_paste_and_clusters() {
        let mut ed = Editor { loaded: true, ..Default::default() };
        let doc = Document::new();
        ed.add_node("Number", Pos2::ZERO);
        let add = ed.add_node("Add", Pos2::new(200.0, 0.0));
        assert_eq!(ed.root.nodes.len(), 2);
        ed.undo();
        assert_eq!(ed.root.nodes.len(), 1);
        ed.redo();
        assert_eq!(ed.root.nodes.len(), 2);
        ed.toggle_disable(std::slice::from_ref(&add));
        assert!(ed.root.node(&add).unwrap().disabled);
        // copy / paste
        ed.selected = [add.clone()].into_iter().collect();
        ed.copy();
        ed.paste();
        assert_eq!(ed.root.nodes.len(), 3);
        // slider drags coalesce into one undo step
        let steps = ed.undo.len();
        let num = ed.root.nodes[0].uid.clone();
        for v in [1.0, 2.0, 3.0] {
            ed.set_value(&num, "Value", json!(v));
        }
        assert_eq!(ed.undo.len(), steps + 1);
        // cluster, open it, edit inside, leave: the edit lands in the definition
        ed.cluster(vec![num.clone()], "Inputs");
        let cl = ed.root.nodes.iter().find(|n| n.kind.starts_with("Cluster:")).unwrap().uid.clone();
        ed.open_cluster(&cl);
        assert_eq!(ed.breadcrumb(), "Locust › Inputs");
        ed.add_node("Text", Pos2::ZERO);
        ed.leave_cluster();
        let node = ed.root.node(&cl).unwrap().clone();
        let inner = Graph::from_json(&ed.root.cluster_of(&node).unwrap()["graph"]);
        assert!(inner.nodes.iter().any(|n| n.kind == "Text"));
        ed.solve(&doc);
        assert!(ed.status.contains("components"));
    }

    #[test]
    fn editor_previews_and_bakes_the_example() {
        let mut ed = Editor { loaded: true, ..Default::default() };
        ed.root = build_example();
        let mut doc = Document::new();
        ed.open = true;
        ed.solve(&doc);
        assert_eq!(ed.preview.len(), 1);
        let panel = ed.root.nodes.iter().find(|n| n.kind == "Panel").unwrap().uid.clone();
        assert!(ed.displays[&panel].starts_with("{}  "), "{:?}", ed.displays[&panel]);
        assert_eq!(ed.bake_into(&mut doc), 1);
        assert_eq!(doc.objects().count(), 1);
        assert!(doc.objects().next().unwrap().geometry.is_solid());
    }

    #[test]
    fn wire_snapping_finds_the_nearest_port() {
        let mut ed = Editor { loaded: true, ..Default::default() };
        let a = ed.add_node("Number", Pos2::ZERO);
        let ports = port_positions(&ed);
        let (_, r, p) = ports.iter().find(|(u, r, _)| *u == a && r.out).unwrap().clone();
        let hit = port_at(&ports, p + Vec2::new(10.0, 6.0), None, 24.0).unwrap();
        assert_eq!(hit, (a, r));
        assert!(port_at(&ports, p + Vec2::new(60.0, 0.0), None, 24.0).is_none());
    }
}
