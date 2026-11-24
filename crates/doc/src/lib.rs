//! tri-doc: document model (objects, layers, selection, undo) and file I/O.

pub mod dxf;
pub mod formats;
pub mod io;
pub mod io3dm;
pub mod history;
pub mod parametric;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use glam::{DAffine3, DVec3};
use tri_geom::{BoundingBox, Brep, Mesh, NurbsCurve, NurbsSurface};
use serde::{Deserialize, Serialize};

pub type ObjectId = u64;

/// RGBA colour, 0-255.
pub type Color = [u8; 4];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Geometry {
    Point(DVec3),
    Curve(NurbsCurve),
    Surface(NurbsSurface),
    Brep(Brep),
    Mesh(Mesh),
    /// Instance of a block definition (index into `State::blocks`) with a transform.
    Instance { block: usize, xf: DAffine3 },
}

/// Texture mapping kinds (ApplyPlanarMapping, ApplyBoxMapping, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MappingKind {
    /// Surface parameters (NURBS surfaces) or box mapping for other objects.
    #[default]
    Surface,
    Planar,
    Box,
    Cylindrical,
    Spherical,
}

impl MappingKind {
    pub fn name(&self) -> &'static str {
        match self {
            MappingKind::Surface => "Surface",
            MappingKind::Planar => "Planar",
            MappingKind::Box => "Box",
            MappingKind::Cylindrical => "Cylindrical",
            MappingKind::Spherical => "Spherical",
        }
    }
    pub fn from_name(n: &str) -> Option<Self> {
        Some(match n.to_ascii_lowercase().as_str() {
            "surface" | "ocs" | "custom" => MappingKind::Surface,
            "planar" => MappingKind::Planar,
            "box" => MappingKind::Box,
            "cylindrical" => MappingKind::Cylindrical,
            "spherical" => MappingKind::Spherical,
            _ => return None,
        })
    }
}

/// A render material (Materials panel).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Material {
    pub name: String,
    pub color: Color,
    /// 0 = matte, 1 = mirror-like highlights.
    pub gloss: f32,
    /// 0 = opaque, 1 = invisible.
    pub transparency: f32,
    /// Image file used as the colour texture.
    pub texture: Option<String>,
    /// Texture repeat scale in model units (one repeat every `texture_scale` units).
    pub texture_scale: f64,
}

impl Default for Material {
    fn default() -> Self {
        Self { name: "Default Material".into(), color: [200, 200, 200, 255], gloss: 0.3, transparency: 0.0, texture: None, texture_scale: 10.0 }
    }
}

/// Built-in linetype patterns: dash lengths (positive = dash, negative = gap), repeated.
pub const BUILTIN_LINETYPES: [(&str, &[f64]); 7] = [
    ("Continuous", &[]),
    ("Dashed", &[4.0, -2.0]),
    ("Dots", &[0.4, -2.0]),
    ("DashDot", &[4.0, -2.0, 0.4, -2.0]),
    ("Center", &[8.0, -2.0, 2.0, -2.0]),
    ("Hidden", &[2.0, -2.0]),
    ("Border", &[4.0, -2.0, 4.0, -2.0, 0.4, -2.0]),
];

/// A block definition: geometry stored once and referenced by instances.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlockDef {
    pub name: String,
    pub base: DVec3,
    pub objects: Vec<(Geometry, Attributes)>,
}

impl BlockDef {
    pub fn bounding_box(&self) -> BoundingBox {
        self.objects.iter().fold(BoundingBox::EMPTY, |b, (g, _)| b.union(&g.bounding_box()))
    }
}

impl Geometry {
    pub fn kind(&self) -> &'static str {
        match self {
            Geometry::Point(_) => "point",
            Geometry::Curve(_) => "curve",
            Geometry::Surface(_) => "surface",
            Geometry::Brep(_) => "polysurface",
            Geometry::Mesh(_) => "mesh",
            Geometry::Instance { .. } => "block instance",
        }
    }

    /// Bounding box; instances need the document to resolve their definition
    /// (see `Document::object_bbox`), here they report an empty box.
    pub fn bounding_box(&self) -> BoundingBox {
        match self {
            Geometry::Point(p) => BoundingBox::new(*p, *p),
            Geometry::Curve(c) => c.bounding_box(),
            Geometry::Surface(s) => s.bounding_box(),
            Geometry::Brep(b) => b.bounding_box(),
            Geometry::Mesh(m) => m.bounding_box(),
            Geometry::Instance { .. } => BoundingBox::EMPTY,
        }
    }

    pub fn transform(&mut self, xf: &DAffine3) {
        match self {
            Geometry::Point(p) => *p = xf.transform_point3(*p),
            Geometry::Curve(c) => c.transform(xf),
            Geometry::Surface(s) => s.transform(xf),
            Geometry::Brep(b) => b.transform(xf),
            Geometry::Mesh(m) => m.transform(xf),
            Geometry::Instance { xf: own, .. } => *own = *xf * *own,
        }
    }

    pub fn is_curve(&self) -> bool {
        matches!(self, Geometry::Curve(_))
    }

    /// Triceratops' object type wording: "closed curve", "open surface", "closed solid polysurface", ...
    pub fn type_name(&self) -> String {
        match self {
            Geometry::Point(_) => "point".into(),
            Geometry::Curve(c) => if c.is_closed() { "closed curve".into() } else { "open curve".into() },
            Geometry::Surface(s) => if surface_is_closed(s) { "closed surface".into() } else { "open surface".into() },
            Geometry::Brep(b) => if b.closed { "closed solid polysurface".into() } else { "open polysurface".into() },
            Geometry::Mesh(m) => if m.is_closed() { "closed mesh".into() } else { "open mesh".into() },
            Geometry::Instance { .. } => "block instance".into(),
        }
    }

    /// Solids: closed surfaces and closed polysurfaces.
    pub fn is_solid(&self) -> bool {
        match self {
            Geometry::Surface(s) => surface_is_closed(s),
            Geometry::Brep(b) => b.closed,
            Geometry::Mesh(m) => m.is_closed(),
            _ => false,
        }
    }

    pub fn as_curve(&self) -> Option<&NurbsCurve> {
        if let Geometry::Curve(c) = self {
            Some(c)
        } else {
            None
        }
    }
}

/// A surface is closed (a solid) when it has no naked edges: closed in both
/// directions, or closed in one and collapsed to a point in the other (sphere).
pub fn surface_is_closed(s: &NurbsSurface) -> bool {
    let (u0, u1) = s.domain_u();
    let (v0, v1) = s.domain_v();
    let tol = 1e-6 * (1.0 + s.bounding_box().diagonal());
    let same = |a: DVec3, b: DVec3| (a - b).length() < tol;
    let closed_u = (0..=8).all(|i| {
        let v = v0 + (v1 - v0) * i as f64 / 8.0;
        same(s.point_at(u0, v), s.point_at(u1, v))
    });
    let closed_v = (0..=8).all(|i| {
        let u = u0 + (u1 - u0) * i as f64 / 8.0;
        same(s.point_at(u, v0), s.point_at(u, v1))
    });
    let degenerate = |fixed_v: f64| (0..=8).all(|i| {
        let u = u0 + (u1 - u0) * i as f64 / 8.0;
        same(s.point_at(u, fixed_v), s.point_at(u0, fixed_v))
    });
    let degenerate_u = |fixed_u: f64| (0..=8).all(|i| {
        let v = v0 + (v1 - v0) * i as f64 / 8.0;
        same(s.point_at(fixed_u, v), s.point_at(fixed_u, v0))
    });
    (closed_u && closed_v)
        || (closed_u && degenerate(v0) && degenerate(v1))
        || (closed_v && degenerate_u(u0) && degenerate_u(u1))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Attributes {
    pub layer: usize,
    pub name: Option<String>,
    /// Per-object colour override; `None` means "by layer".
    pub color: Option<Color>,
    pub hidden: bool,
    pub locked: bool,
    /// Group names this object belongs to (innermost last).
    pub groups: Vec<String>,
    /// User text key/value pairs (SetUserText).
    #[serde(default)]
    pub user_text: Vec<(String, String)>,
    /// Draw order (BringToFront / SendToBack); higher draws later.
    #[serde(default)]
    pub draw_order: i32,
    /// Linetype name; `None` means "by layer".
    #[serde(default)]
    pub linetype: Option<String>,
    /// Material name; `None` means "by layer".
    #[serde(default)]
    pub material: Option<String>,
    /// Texture mapping override (kind, scale); `None` uses the material's default (surface/box).
    #[serde(default)]
    pub mapping: Option<(MappingKind, f64)>,
}


#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Object {
    pub id: ObjectId,
    pub geometry: Geometry,
    pub attributes: Attributes,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layer {
    pub name: String,
    pub color: Color,
    pub visible: bool,
    pub locked: bool,
    pub linetype: String,
    pub material: Option<String>,
}

impl Default for Layer {
    fn default() -> Self {
        Self { name: "Default".into(), color: [0, 0, 0, 255], visible: true, locked: false, linetype: "Continuous".into(), material: None }
    }
}

impl Layer {
    pub fn new(name: impl Into<String>, color: Color) -> Self {
        Self { name: name.into(), color, ..Default::default() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub units: String,
    pub absolute_tolerance: f64,
    pub angle_tolerance_deg: f64,
    /// Document Properties > Grid
    pub grid_line_count: i32,
    pub grid_minor_spacing: f64,
    pub grid_major_every: i32,
    pub grid_snap_spacing: f64,
    pub show_grid: bool,
    pub show_grid_axes: bool,
    pub show_world_axes: bool,
    pub notes: String,
    /// Document user text (SetDocumentUserText).
    #[serde(default)]
    pub user_text: Vec<(String, String)>,
    /// Render materials (Materials panel).
    #[serde(default)]
    pub materials: Vec<Material>,
    /// Global linetype scale.
    #[serde(default = "one")]
    pub linetype_scale: f64,
    /// Custom linetypes: name and dash pattern.
    #[serde(default)]
    pub linetypes: Vec<(String, Vec<f64>)>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            units: "Millimeters".into(),
            absolute_tolerance: 0.001,
            angle_tolerance_deg: 1.0,
            grid_line_count: 70,
            grid_minor_spacing: 1.0,
            grid_major_every: 5,
            grid_snap_spacing: 1.0,
            show_grid: true,
            show_grid_axes: true,
            show_world_axes: true,
            notes: String::new(),
            user_text: Vec::new(),
            materials: Vec::new(),
            linetype_scale: 1.0,
            linetypes: Vec::new(),
        }
    }
}

fn one() -> f64 {
    1.0
}

/// Everything that undo/redo restores.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub objects: BTreeMap<ObjectId, Object>,
    pub layers: Vec<Layer>,
    pub current_layer: usize,
    pub blocks: Vec<BlockDef>,
    /// Parametric sketch data (constraints, parameters, sketch plane).
    pub parametric: parametric::Parametric,
    /// Construction history, cages and holes.
    pub history: history::History,
}

impl Default for State {
    fn default() -> Self {
        Self { objects: BTreeMap::new(), layers: vec![Layer::new("Default", DEFAULT_LAYER_COLORS[0])], current_layer: 0, blocks: Vec::new(), parametric: Default::default(), history: Default::default() }
    }
}

#[derive(Clone, Debug)]
struct UndoRecord {
    label: String,
    state: State,
    selection: BTreeSet<ObjectId>,
}

/// A light object: id, kind, position, direction, colour, intensity, on.
pub type LightInfo = (ObjectId, String, DVec3, DVec3, [f32; 3], f32, bool);

#[derive(Debug)]
pub struct Document {
    pub state: State,
    pub settings: Settings,
    pub selection: BTreeSet<ObjectId>,
    pub path: Option<PathBuf>,
    pub modified: bool,
    next_id: ObjectId,
    undo_stack: Vec<UndoRecord>,
    redo_stack: Vec<UndoRecord>,
    /// Bumped on every change so renderers can cache tessellations.
    pub revision: u64,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

pub const DEFAULT_LAYER_COLORS: [Color; 6] = [
    [0, 0, 0, 255],
    [255, 0, 0, 255],
    [0, 0, 255, 255],
    [0, 128, 0, 255],
    [255, 0, 255, 255],
    [0, 128, 128, 255],
];

impl Document {
    pub fn new() -> Self {
        Self {
            state: State {
                objects: BTreeMap::new(),
                layers: vec![
                    Layer::new("Default", DEFAULT_LAYER_COLORS[0]),
                    Layer::new("Layer 01", DEFAULT_LAYER_COLORS[1]),
                    Layer::new("Layer 02", DEFAULT_LAYER_COLORS[2]),
                    Layer::new("Layer 03", DEFAULT_LAYER_COLORS[3]),
                    Layer::new("Layer 04", DEFAULT_LAYER_COLORS[4]),
                    Layer::new("Layer 05", DEFAULT_LAYER_COLORS[5]),
                ],
                current_layer: 0,
                blocks: Vec::new(),
                parametric: Default::default(),
                history: Default::default(),
            },
            settings: Settings::default(),
            selection: BTreeSet::new(),
            path: None,
            modified: false,
            next_id: 1,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            revision: 0,
        }
    }

    // ----- objects -------------------------------------------------------

    pub fn objects(&self) -> impl Iterator<Item = &Object> {
        self.state.objects.values()
    }

    pub fn get(&self, id: ObjectId) -> Option<&Object> {
        self.state.objects.get(&id)
    }

    pub fn get_mut(&mut self, id: ObjectId) -> Option<&mut Object> {
        self.touch();
        self.state.objects.get_mut(&id)
    }

    pub fn len(&self) -> usize {
        self.state.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.state.objects.is_empty()
    }

    pub fn add(&mut self, geometry: Geometry) -> ObjectId {
        let attributes = Attributes { layer: self.state.current_layer, ..Default::default() };
        self.add_with(geometry, attributes)
    }

    pub fn add_with(&mut self, geometry: Geometry, attributes: Attributes) -> ObjectId {
        let id = self.next_id;
        self.next_id += 1;
        self.state.objects.insert(id, Object { id, geometry, attributes });
        self.touch();
        id
    }

    pub fn remove(&mut self, id: ObjectId) -> Option<Object> {
        self.selection.remove(&id);
        let o = self.state.objects.remove(&id);
        if o.is_some() {
            self.touch();
        }
        o
    }

    pub fn replace_geometry(&mut self, id: ObjectId, geometry: Geometry) -> bool {
        if let Some(o) = self.state.objects.get_mut(&id) {
            o.geometry = geometry;
            self.touch();
            true
        } else {
            false
        }
    }

    pub fn transform_objects(&mut self, ids: &[ObjectId], xf: &DAffine3) {
        for id in ids {
            if let Some(o) = self.state.objects.get_mut(id) {
                o.geometry.transform(xf);
            }
        }
        self.touch();
    }

    /// Whether an object is currently visible (own flag and layer visibility).
    pub fn is_visible(&self, o: &Object) -> bool {
        !o.attributes.hidden && self.state.layers.get(o.attributes.layer).is_none_or(|l| l.visible)
    }

    pub fn is_locked(&self, o: &Object) -> bool {
        o.attributes.locked || self.state.layers.get(o.attributes.layer).is_some_and(|l| l.locked)
    }

    pub fn is_selectable(&self, o: &Object) -> bool {
        self.is_visible(o) && !self.is_locked(o)
    }

    pub fn display_color(&self, o: &Object) -> Color {
        o.attributes.color.unwrap_or_else(|| {
            self.state.layers.get(o.attributes.layer).map_or([0, 0, 0, 255], |l| l.color)
        })
    }

    /// Dash pattern of a linetype name (empty = continuous, None = unknown).
    pub fn linetype_pattern(&self, name: &str) -> Option<Vec<f64>> {
        if let Some((_, p)) = BUILTIN_LINETYPES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            return Some(p.to_vec());
        }
        if let Some((_, p)) = self.settings.linetypes.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            return Some(p.clone());
        }
        if let Some(rest) = name.strip_prefix("Custom:") {
            return Some(rest.split(',').filter_map(|x| x.trim().parse().ok()).collect());
        }
        None
    }

    /// Effective linetype name of an object (object override or layer).
    pub fn linetype_of(&self, o: &Object) -> String {
        o.attributes.linetype.clone().unwrap_or_else(|| self.state.layers.get(o.attributes.layer).map(|l| l.linetype.clone()).unwrap_or_else(|| "Continuous".into()))
    }

    pub fn material_named(&self, name: &str) -> Option<&Material> {
        self.settings.materials.iter().find(|m| m.name.eq_ignore_ascii_case(name))
    }

    /// Effective material of an object: object override, then layer, then a default built from
    /// the display colour.
    pub fn material_of(&self, o: &Object) -> Material {
        let name = o.attributes.material.clone().or_else(|| self.state.layers.get(o.attributes.layer).and_then(|l| l.material.clone()));
        if let Some(m) = name.as_deref().and_then(|n| self.material_named(n)) {
            return m.clone();
        }
        Material { name: String::new(), color: self.display_color(o), ..Default::default() }
    }

    /// Lights: point objects tagged with the `Light` user text key (kind, direction, colour, intensity, on).
    pub fn lights(&self) -> Vec<LightInfo> {
        self.objects()
            .filter(|o| self.is_visible(o))
            .filter_map(|o| {
                let Geometry::Point(p) = o.geometry else { return None };
                let get = |k: &str| o.attributes.user_text.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone());
                let kind = get("Light")?;
                let dir = get("LightDir").map(|s| { let v: Vec<f64> = s.split(',').filter_map(|x| x.parse().ok()).collect(); if v.len() == 3 { DVec3::new(v[0], v[1], v[2]) } else { -DVec3::Z } }).unwrap_or(-DVec3::Z);
                let color = get("LightColor").map(|s| { let v: Vec<f32> = s.split(',').filter_map(|x| x.parse().ok()).collect(); if v.len() == 3 { [v[0], v[1], v[2]] } else { [1.0, 1.0, 1.0] } }).unwrap_or([1.0, 1.0, 1.0]);
                let intensity = get("LightIntensity").and_then(|s| s.parse().ok()).unwrap_or(1.0);
                let on = get("LightOn").is_none_or(|s| s != "No");
                Some((o.id, kind, p, dir, color, intensity, on))
            })
            .collect()
    }

    /// Bounding box of an object, resolving block instances.
    pub fn object_bbox(&self, o: &Object) -> BoundingBox {
        match &o.geometry {
            Geometry::Instance { block, xf } => match self.state.blocks.get(*block) {
                Some(def) => {
                    let mut bb = BoundingBox::EMPTY;
                    for (g, _) in &def.objects {
                        let mut g2 = g.clone();
                        g2.transform(xf);
                        bb = bb.union(&g2.bounding_box());
                    }
                    bb
                }
                None => BoundingBox::EMPTY,
            },
            g => g.bounding_box(),
        }
    }

    /// Geometry of a block instance, instantiated in world coordinates.
    pub fn instance_geometry(&self, g: &Geometry) -> Vec<(Geometry, Attributes)> {
        match g {
            Geometry::Instance { block, xf } => match self.state.blocks.get(*block) {
                Some(def) => def
                    .objects
                    .iter()
                    .flat_map(|(g, a)| {
                        if matches!(g, Geometry::Instance { .. }) {
                            let mut g2 = g.clone();
                            g2.transform(xf);
                            self.instance_geometry(&g2)
                        } else {
                            let mut g2 = g.clone();
                            g2.transform(xf);
                            vec![(g2, a.clone())]
                        }
                    })
                    .collect(),
                None => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    pub fn find_block(&self, name: &str) -> Option<usize> {
        self.state.blocks.iter().position(|b| b.name.eq_ignore_ascii_case(name))
    }

    pub fn add_block(&mut self, def: BlockDef) -> usize {
        self.state.blocks.push(def);
        self.touch();
        self.state.blocks.len() - 1
    }

    pub fn bounding_box(&self) -> BoundingBox {
        self.objects()
            .filter(|o| self.is_visible(o))
            .fold(BoundingBox::EMPTY, |b, o| b.union(&self.object_bbox(o)))
    }

    // ----- layers --------------------------------------------------------

    pub fn layers(&self) -> &[Layer] {
        &self.state.layers
    }

    pub fn current_layer(&self) -> usize {
        self.state.current_layer
    }

    pub fn set_current_layer(&mut self, idx: usize) {
        if idx < self.state.layers.len() {
            self.state.current_layer = idx;
            self.touch();
        }
    }

    pub fn add_layer(&mut self, name: impl Into<String>) -> usize {
        let idx = self.state.layers.len();
        let color = DEFAULT_LAYER_COLORS[idx % DEFAULT_LAYER_COLORS.len()];
        self.state.layers.push(Layer::new(name, color));
        self.touch();
        idx
    }

    pub fn layer_mut(&mut self, idx: usize) -> Option<&mut Layer> {
        self.touch();
        self.state.layers.get_mut(idx)
    }

    pub fn find_layer(&self, name: &str) -> Option<usize> {
        self.state.layers.iter().position(|l| l.name.eq_ignore_ascii_case(name))
    }

    /// Delete a layer; its objects move to layer 0. Layer 0 cannot be deleted.
    pub fn delete_layer(&mut self, idx: usize) -> bool {
        if idx == 0 || idx >= self.state.layers.len() {
            return false;
        }
        self.state.layers.remove(idx);
        for o in self.state.objects.values_mut() {
            if o.attributes.layer == idx {
                o.attributes.layer = 0;
            } else if o.attributes.layer > idx {
                o.attributes.layer -= 1;
            }
        }
        if self.state.current_layer >= self.state.layers.len() {
            self.state.current_layer = 0;
        }
        self.touch();
        true
    }

    // ----- selection -----------------------------------------------------

    pub fn select(&mut self, id: ObjectId, selected: bool) {
        if selected {
            if self.get(id).is_some_and(|o| self.is_selectable(o)) {
                self.selection.insert(id);
            }
        } else {
            self.selection.remove(&id);
        }
    }

    /// Objects sharing the outermost group of `id` (including `id`).
    pub fn group_members(&self, id: ObjectId) -> Vec<ObjectId> {
        let Some(o) = self.get(id) else { return vec![id] };
        let Some(top) = o.attributes.groups.first().cloned() else { return vec![id] };
        self.objects().filter(|x| x.attributes.groups.first() == Some(&top)).map(|x| x.id).collect()
    }

    /// Area of a planar closed curve, face or polysurface (via tessellation).
    pub fn area_of(g: &Geometry) -> Option<f64> {
        let tri_area = |m: &Mesh| -> f64 {
            (0..m.triangle_count()).map(|t| { let (a, b, c) = m.triangle(t); (b - a).cross(c - a).length() * 0.5 }).sum()
        };
        match g {
            Geometry::Curve(c) if c.is_closed() => {
                let pts = c.tessellate(16);
                let n = DVec3::ZERO;
                let mut nn = n;
                for i in 0..pts.len() {
                    let a = pts[i];
                    let b = pts[(i + 1) % pts.len()];
                    nn += a.cross(b);
                }
                Some(nn.length() * 0.5)
            }
            Geometry::Surface(s) => Some(tri_area(&s.tessellate(16, 16))),
            Geometry::Brep(b) => Some(tri_area(&b.tessellate(16, 16))),
            Geometry::Mesh(m) => Some(tri_area(m)),
            _ => None,
        }
    }

    /// Signed volume of a closed mesh (divergence theorem) and its centroid.
    pub fn volume_of(g: &Geometry) -> Option<(f64, DVec3)> {
        let mesh = match g {
            Geometry::Surface(s) if surface_is_closed(s) => s.tessellate(16, 16),
            Geometry::Brep(b) if b.closed => b.tessellate(16, 16),
            Geometry::Mesh(m) if m.is_closed() => m.clone(),
            _ => return None,
        };
        let mut v = 0.0;
        let mut c = DVec3::ZERO;
        for t in 0..mesh.triangle_count() {
            let (a, b, d) = mesh.triangle(t);
            let vt = a.dot(b.cross(d)) / 6.0;
            v += vt;
            c += (a + b + d) / 4.0 * vt;
        }
        if v.abs() < 1e-18 {
            return None;
        }
        Some((v.abs(), c / v))
    }

    pub fn toggle_select(&mut self, id: ObjectId) {
        let s = !self.selection.contains(&id);
        self.select(id, s);
    }

    pub fn select_all(&mut self) {
        let ids: Vec<ObjectId> = self.objects().filter(|o| self.is_selectable(o)).map(|o| o.id).collect();
        self.selection.extend(ids);
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    pub fn selected_ids(&self) -> Vec<ObjectId> {
        self.selection.iter().copied().collect()
    }

    pub fn is_selected(&self, id: ObjectId) -> bool {
        self.selection.contains(&id)
    }

    // ----- undo / redo ---------------------------------------------------

    /// Record the current state before a mutating operation.
    pub fn begin_undo(&mut self, label: impl Into<String>) {
        self.undo_stack.push(UndoRecord {
            label: label.into(),
            state: self.state.clone(),
            selection: self.selection.clone(),
        });
        if self.undo_stack.len() > 200 {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) -> Option<String> {
        let rec = self.undo_stack.pop()?;
        let current = UndoRecord { label: rec.label.clone(), state: self.state.clone(), selection: self.selection.clone() };
        self.redo_stack.push(current);
        self.state = rec.state;
        self.selection = rec.selection;
        self.selection.retain(|id| self.state.objects.contains_key(id));
        self.touch();
        Some(rec.label)
    }

    /// Restore only the given objects from the most recent undo record (UndoSelected).
    pub fn undo_selected(&mut self, ids: &[ObjectId]) -> usize {
        let Some(rec) = self.undo_stack.last().cloned() else { return 0 };
        self.begin_undo("UndoSelected");
        let mut n = 0;
        for id in ids {
            match rec.state.objects.get(id) {
                Some(old) => {
                    self.state.objects.insert(*id, old.clone());
                    n += 1;
                }
                None => {
                    if self.state.objects.remove(id).is_some() {
                        n += 1;
                    }
                }
            }
        }
        self.selection.retain(|id| self.state.objects.contains_key(id));
        self.touch();
        n
    }

    pub fn redo(&mut self) -> Option<String> {
        let rec = self.redo_stack.pop()?;
        let current = UndoRecord { label: rec.label.clone(), state: self.state.clone(), selection: self.selection.clone() };
        self.undo_stack.push(current);
        self.state = rec.state;
        self.selection = rec.selection;
        self.selection.retain(|id| self.state.objects.contains_key(id));
        self.touch();
        Some(rec.label)
    }

    pub fn clear_undo(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Bump the revision so display caches rebuild (used by display-only changes).
    pub fn touch_revision(&mut self) {
        self.revision += 1;
    }

    fn touch(&mut self) {
        self.revision += 1;
        self.modified = true;
    }

    /// Replace the whole document (used when opening a file).
    pub fn load_state(&mut self, state: State, settings: Settings, path: Option<PathBuf>) {
        self.next_id = state.objects.keys().max().map_or(1, |m| m + 1);
        self.state = state;
        self.settings = settings;
        self.selection.clear();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.path = path;
        self.revision += 1;
        self.modified = false;
    }

    pub fn title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".into());
        if self.modified {
            format!("{name}*")
        } else {
            name
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_roundtrip() {
        let mut d = Document::new();
        d.begin_undo("add");
        let id = d.add(Geometry::Point(DVec3::ZERO));
        assert_eq!(d.len(), 1);
        assert_eq!(d.undo().as_deref(), Some("add"));
        assert_eq!(d.len(), 0);
        assert!(d.redo().is_some());
        assert!(d.get(id).is_some());
    }

    #[test]
    fn layer_deletion_reassigns_objects() {
        let mut d = Document::new();
        let l = d.add_layer("Walls");
        d.set_current_layer(l);
        let id = d.add(Geometry::Point(DVec3::ONE));
        assert_eq!(d.get(id).unwrap().attributes.layer, l);
        assert!(d.delete_layer(l));
        assert_eq!(d.get(id).unwrap().attributes.layer, 0);
        assert_eq!(d.current_layer(), 0);
    }
}
