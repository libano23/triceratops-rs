//! Safe wrapper over a small C shim of the openNURBS toolkit: read and write
//! Triceratops `.3dm` files (points, NURBS curves and surfaces, polysurfaces with
//! trim loops, meshes, layers, units and notes).

use std::ffi::{c_char, c_double, c_int, c_uint, CStr, CString};
use std::path::Path;

#[repr(C)]
struct On3dm {
    _private: [u8; 0],
}

extern "C" {
    fn on3dm_open(path: *const c_char) -> *mut On3dm;
    fn on3dm_new() -> *mut On3dm;
    fn on3dm_free(h: *mut On3dm);
    fn on3dm_write(h: *mut On3dm, path: *const c_char, version: c_int) -> c_int;
    fn on3dm_units(h: *mut On3dm) -> c_int;
    fn on3dm_abs_tol(h: *mut On3dm) -> c_double;
    fn on3dm_set_units(h: *mut On3dm, unit_system: c_int, abs_tol: c_double);
    fn on3dm_notes(h: *mut On3dm) -> *const c_char;
    fn on3dm_set_notes(h: *mut On3dm, notes: *const c_char);
    fn on3dm_layer_count(h: *mut On3dm) -> c_int;
    fn on3dm_layer_index(h: *mut On3dm, i: c_int) -> c_int;
    fn on3dm_layer_name(h: *mut On3dm, i: c_int) -> *const c_char;
    fn on3dm_layer_color(h: *mut On3dm, i: c_int) -> c_uint;
    fn on3dm_layer_visible(h: *mut On3dm, i: c_int) -> c_int;
    fn on3dm_layer_locked(h: *mut On3dm, i: c_int) -> c_int;
    fn on3dm_add_layer(h: *mut On3dm, name: *const c_char, rgb: c_uint, visible: c_int, locked: c_int) -> c_int;
    fn on3dm_object_count(h: *mut On3dm) -> c_int;
    fn on3dm_object_kind(h: *mut On3dm, i: c_int) -> c_int;
    fn on3dm_object_layer(h: *mut On3dm, i: c_int) -> c_int;
    fn on3dm_object_name(h: *mut On3dm, i: c_int) -> *const c_char;
    fn on3dm_object_color(h: *mut On3dm, i: c_int, rgb: *mut c_uint) -> c_int;
    fn on3dm_object_hidden(h: *mut On3dm, i: c_int) -> c_int;
    fn on3dm_object_locked(h: *mut On3dm, i: c_int) -> c_int;
    fn on3dm_object_point(h: *mut On3dm, i: c_int) -> *const c_double;
    fn on3dm_object_curve(h: *mut On3dm, i: c_int, rational: *mut c_int, order: *mut c_int, cv_count: *mut c_int, knot_count: *mut c_int, knots: *mut *const c_double, cv: *mut *const c_double) -> c_int;
    fn on3dm_object_surface(h: *mut On3dm, i: c_int, rational: *mut c_int, ou: *mut c_int, ov: *mut c_int, cu: *mut c_int, cvv: *mut c_int, ku_count: *mut c_int, kv_count: *mut c_int, ku: *mut *const c_double, kv: *mut *const c_double, cv: *mut *const c_double) -> c_int;
    fn on3dm_brep_face_count(h: *mut On3dm, i: c_int) -> c_int;
    fn on3dm_brep_is_solid(h: *mut On3dm, i: c_int) -> c_int;
    fn on3dm_brep_face_surface(h: *mut On3dm, i: c_int, f: c_int, rational: *mut c_int, ou: *mut c_int, ov: *mut c_int, cu: *mut c_int, cvv: *mut c_int, ku_count: *mut c_int, kv_count: *mut c_int, ku: *mut *const c_double, kv: *mut *const c_double, cv: *mut *const c_double) -> c_int;
    fn on3dm_brep_face_loop_count(h: *mut On3dm, i: c_int, f: c_int) -> c_int;
    fn on3dm_brep_face_loop(h: *mut On3dm, i: c_int, f: c_int, l: c_int, is_outer: *mut c_int, count: *mut c_int, uv: *mut *const c_double) -> c_int;
    fn on3dm_object_mesh(h: *mut On3dm, i: c_int, vcount: *mut c_int, tcount: *mut c_int, verts: *mut *const c_double, tris: *mut *const c_int) -> c_int;
    fn on3dm_add_point(h: *mut On3dm, xyz: *const c_double, layer: c_int, name: *const c_char, has_color: c_int, rgb: c_uint, hidden: c_int, locked: c_int) -> c_int;
    fn on3dm_add_curve(h: *mut On3dm, rational: c_int, order: c_int, cv_count: c_int, knots: *const c_double, cv: *const c_double, layer: c_int, name: *const c_char, has_color: c_int, rgb: c_uint, hidden: c_int, locked: c_int) -> c_int;
    fn on3dm_add_surface(h: *mut On3dm, rational: c_int, ou: c_int, ov: c_int, cu: c_int, cvv: c_int, ku: *const c_double, kv: *const c_double, cv: *const c_double, layer: c_int, name: *const c_char, has_color: c_int, rgb: c_uint, hidden: c_int, locked: c_int) -> c_int;
    fn on3dm_brep_begin(h: *mut On3dm) -> c_int;
    fn on3dm_brep_add_face(h: *mut On3dm, rational: c_int, ou: c_int, ov: c_int, cu: c_int, cvv: c_int, ku: *const c_double, kv: *const c_double, cv: *const c_double, loop_count: c_int, loop_sizes: *const c_int, loop_uv: *const c_double) -> c_int;
    fn on3dm_brep_end(h: *mut On3dm, layer: c_int, name: *const c_char, has_color: c_int, rgb: c_uint, hidden: c_int, locked: c_int) -> c_int;
    fn on3dm_add_mesh(h: *mut On3dm, vcount: c_int, verts: *const c_double, tcount: c_int, tris: *const c_int, layer: c_int, name: *const c_char, has_color: c_int, rgb: c_uint, hidden: c_int, locked: c_int) -> c_int;
    fn on3dm_idef_count(h: *mut On3dm) -> c_int;
    fn on3dm_idef_name(h: *mut On3dm, i: c_int) -> *const c_char;
    fn on3dm_idef_member_count(h: *mut On3dm, i: c_int) -> c_int;
    fn on3dm_idef_member(h: *mut On3dm, i: c_int, k: c_int) -> c_int;
    fn on3dm_object_idef_member(h: *mut On3dm, i: c_int) -> c_int;
    fn on3dm_object_instance(h: *mut On3dm, i: c_int, idef: *mut c_int, xf16: *mut c_double) -> c_int;
    fn on3dm_idef_begin(h: *mut On3dm);
    fn on3dm_idef_end(h: *mut On3dm, name: *const c_char) -> c_int;
    fn on3dm_add_instance(h: *mut On3dm, idef: c_int, xf16: *const c_double, layer: c_int, name: *const c_char, has_color: c_int, rgb: c_uint, hidden: c_int, locked: c_int) -> c_int;
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("openNURBS could not read {0}")]
    Read(String),
    #[error("openNURBS could not write {0}")]
    Write(String),
    #[error("invalid path")]
    Path,
}

/// A NURBS curve in tri-geom conventions: full knot vector (count + order), Euclidean
/// control points with separate weights.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub degree: usize,
    pub control_points: Vec<[f64; 3]>,
    pub weights: Vec<f64>,
    pub knots: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub degree_u: usize,
    pub degree_v: usize,
    pub count_u: usize,
    pub count_v: usize,
    /// row-major: index = i * count_v + j
    pub control_points: Vec<[f64; 3]>,
    pub weights: Vec<f64>,
    pub knots_u: Vec<f64>,
    pub knots_v: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub surface: Surface,
    /// UV loops; the first is the outer boundary. Empty = untrimmed.
    pub loops: Vec<Vec<[f64; 2]>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Geometry {
    Point([f64; 3]),
    Curve(Curve),
    Surface(Surface),
    Brep { faces: Vec<Face>, solid: bool },
    Mesh { vertices: Vec<[f64; 3]>, triangles: Vec<[u32; 3]> },
    /// Block instance: index into `Model::blocks` and a row-major 4x4 transform.
    Instance { block: usize, xform: [f64; 16] },
}

/// A block (instance) definition: name and the objects it contains, in definition space.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockDef {
    pub name: String,
    pub objects: Vec<Object>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub geometry: Geometry,
    /// Layer index as stored in the file (matches `Layer::index`).
    pub layer: i32,
    pub name: String,
    pub color: Option<[u8; 3]>,
    pub hidden: bool,
    pub locked: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub index: i32,
    pub name: String,
    pub color: [u8; 3],
    pub visible: bool,
    pub locked: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub layers: Vec<Layer>,
    pub objects: Vec<Object>,
    /// Block definitions; instances refer to them by index.
    pub blocks: Vec<BlockDef>,
    /// ON::LengthUnitSystem value (2 = millimeters, 3 = centimeters, 4 = meters, 8 = inches, 9 = feet).
    pub unit_system: i32,
    pub absolute_tolerance: f64,
    pub notes: String,
    /// Objects in the file that could not be converted.
    pub unsupported: usize,
}

pub const UNITS_MILLIMETERS: i32 = 2;
pub const UNITS_CENTIMETERS: i32 = 3;
pub const UNITS_METERS: i32 = 4;
pub const UNITS_INCHES: i32 = 8;
pub const UNITS_FEET: i32 = 9;

fn rgb(v: u32) -> [u8; 3] {
    [((v >> 16) & 255) as u8, ((v >> 8) & 255) as u8, (v & 255) as u8]
}

fn pack(c: [u8; 3]) -> u32 {
    ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32
}

unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

/// openNURBS knot vectors omit the two superfluous end knots; add them back.
fn full_knots(on_knots: &[f64]) -> Vec<f64> {
    let mut k = Vec::with_capacity(on_knots.len() + 2);
    k.push(on_knots[0]);
    k.extend_from_slice(on_knots);
    k.push(on_knots[on_knots.len() - 1]);
    k
}

fn on_knots(full: &[f64]) -> Vec<f64> {
    full[1..full.len() - 1].to_vec()
}

unsafe fn read_surface(rational: c_int, ou: c_int, ov: c_int, cu: c_int, cvv: c_int, kuc: c_int, kvc: c_int, ku: *const f64, kv: *const f64, cv: *const f64) -> Surface {
    let n = (cu * cvv) as usize;
    let cvs = std::slice::from_raw_parts(cv, n * 4);
    let mut control_points = Vec::with_capacity(n);
    let mut weights = Vec::with_capacity(n);
    for i in 0..n {
        control_points.push([cvs[i * 4], cvs[i * 4 + 1], cvs[i * 4 + 2]]);
        weights.push(if rational != 0 { cvs[i * 4 + 3] } else { 1.0 });
    }
    Surface {
        degree_u: (ou - 1) as usize,
        degree_v: (ov - 1) as usize,
        count_u: cu as usize,
        count_v: cvv as usize,
        control_points,
        weights,
        knots_u: full_knots(std::slice::from_raw_parts(ku, kuc as usize)),
        knots_v: full_knots(std::slice::from_raw_parts(kv, kvc as usize)),
    }
}

/// Read a `.3dm` file.
pub fn read(path: &Path) -> Result<Model, Error> {
    let cpath = CString::new(path.to_string_lossy().as_bytes()).map_err(|_| Error::Path)?;
    unsafe {
        let h = on3dm_open(cpath.as_ptr());
        if h.is_null() {
            return Err(Error::Read(path.display().to_string()));
        }
        let mut layers = Vec::new();
        for i in 0..on3dm_layer_count(h) {
            layers.push(Layer {
                index: on3dm_layer_index(h, i),
                name: cstr(on3dm_layer_name(h, i)),
                color: rgb(on3dm_layer_color(h, i)),
                visible: on3dm_layer_visible(h, i) != 0,
                locked: on3dm_layer_locked(h, i) != 0,
            });
        }
        let mut all: Vec<Option<Object>> = Vec::new();
        let mut unsupported = 0;
        for i in 0..on3dm_object_count(h) {
            let kind = on3dm_object_kind(h, i);
            let geometry = match kind {
                6 => {
                    let mut idef: c_int = -1;
                    let mut xf = [0.0f64; 16];
                    on3dm_object_instance(h, i, &mut idef, xf.as_mut_ptr());
                    Geometry::Instance { block: idef as usize, xform: xf }
                }
                1 => {
                    let p = on3dm_object_point(h, i);
                    Geometry::Point([*p, *p.add(1), *p.add(2)])
                }
                2 => {
                    let (mut rational, mut order, mut cvc, mut kc) = (0, 0, 0, 0);
                    let (mut knots, mut cv): (*const f64, *const f64) = (std::ptr::null(), std::ptr::null());
                    on3dm_object_curve(h, i, &mut rational, &mut order, &mut cvc, &mut kc, &mut knots, &mut cv);
                    let cvs = std::slice::from_raw_parts(cv, cvc as usize * 4);
                    let mut control_points = Vec::new();
                    let mut weights = Vec::new();
                    for k in 0..cvc as usize {
                        control_points.push([cvs[k * 4], cvs[k * 4 + 1], cvs[k * 4 + 2]]);
                        weights.push(if rational != 0 { cvs[k * 4 + 3] } else { 1.0 });
                    }
                    Geometry::Curve(Curve { degree: (order - 1) as usize, control_points, weights, knots: full_knots(std::slice::from_raw_parts(knots, kc as usize)) })
                }
                3 => {
                    let (mut r, mut ou, mut ov, mut cu, mut cvv, mut kuc, mut kvc) = (0, 0, 0, 0, 0, 0, 0);
                    let (mut ku, mut kv, mut cv): (*const f64, *const f64, *const f64) = (std::ptr::null(), std::ptr::null(), std::ptr::null());
                    on3dm_object_surface(h, i, &mut r, &mut ou, &mut ov, &mut cu, &mut cvv, &mut kuc, &mut kvc, &mut ku, &mut kv, &mut cv);
                    Geometry::Surface(read_surface(r, ou, ov, cu, cvv, kuc, kvc, ku, kv, cv))
                }
                4 => {
                    let mut faces = Vec::new();
                    for f in 0..on3dm_brep_face_count(h, i) {
                        let (mut r, mut ou, mut ov, mut cu, mut cvv, mut kuc, mut kvc) = (0, 0, 0, 0, 0, 0, 0);
                        let (mut ku, mut kv, mut cv): (*const f64, *const f64, *const f64) = (std::ptr::null(), std::ptr::null(), std::ptr::null());
                        on3dm_brep_face_surface(h, i, f, &mut r, &mut ou, &mut ov, &mut cu, &mut cvv, &mut kuc, &mut kvc, &mut ku, &mut kv, &mut cv);
                        let surface = read_surface(r, ou, ov, cu, cvv, kuc, kvc, ku, kv, cv);
                        let mut loops = Vec::new();
                        let mut outer_first: Vec<(bool, Vec<[f64; 2]>)> = Vec::new();
                        for l in 0..on3dm_brep_face_loop_count(h, i, f) {
                            let (mut is_outer, mut count) = (0, 0);
                            let mut uv: *const f64 = std::ptr::null();
                            on3dm_brep_face_loop(h, i, f, l, &mut is_outer, &mut count, &mut uv);
                            let s = std::slice::from_raw_parts(uv, count as usize * 2);
                            let pts: Vec<[f64; 2]> = (0..count as usize).map(|k| [s[k * 2], s[k * 2 + 1]]).collect();
                            outer_first.push((is_outer != 0, pts));
                        }
                        outer_first.sort_by_key(|(o, _)| !*o);
                        for (_, pts) in outer_first {
                            if pts.len() >= 3 {
                                loops.push(pts);
                            }
                        }
                        faces.push(Face { surface, loops });
                    }
                    Geometry::Brep { faces, solid: on3dm_brep_is_solid(h, i) != 0 }
                }
                5 => {
                    let (mut vc, mut tc) = (0, 0);
                    let mut verts: *const f64 = std::ptr::null();
                    let mut tris: *const c_int = std::ptr::null();
                    on3dm_object_mesh(h, i, &mut vc, &mut tc, &mut verts, &mut tris);
                    let vs = std::slice::from_raw_parts(verts, vc as usize * 3);
                    let ts = std::slice::from_raw_parts(tris, tc as usize * 3);
                    Geometry::Mesh {
                        vertices: (0..vc as usize).map(|k| [vs[k * 3], vs[k * 3 + 1], vs[k * 3 + 2]]).collect(),
                        triangles: (0..tc as usize).map(|k| [ts[k * 3] as u32, ts[k * 3 + 1] as u32, ts[k * 3 + 2] as u32]).collect(),
                    }
                }
                _ => {
                    unsupported += 1;
                    all.push(None);
                    continue;
                }
            };
            let mut c = 0u32;
            let has_color = on3dm_object_color(h, i, &mut c) != 0;
            all.push(Some(Object {
                geometry,
                layer: on3dm_object_layer(h, i),
                name: cstr(on3dm_object_name(h, i)),
                color: has_color.then(|| rgb(c)),
                hidden: on3dm_object_hidden(h, i) != 0,
                locked: on3dm_object_locked(h, i) != 0,
            }));
        }
        let mut blocks = Vec::new();
        for d in 0..on3dm_idef_count(h) {
            let mut members = Vec::new();
            for k in 0..on3dm_idef_member_count(h, d) {
                let idx = on3dm_idef_member(h, d, k) as usize;
                if let Some(Some(o)) = all.get(idx) {
                    members.push(o.clone());
                }
            }
            blocks.push(BlockDef { name: cstr(on3dm_idef_name(h, d)), objects: members });
        }
        let objects: Vec<Object> = all
            .into_iter()
            .enumerate()
            .filter(|(i, _)| on3dm_object_idef_member(h, *i as c_int) == 0)
            .filter_map(|(_, o)| o)
            .collect();
        let model = Model {
            layers,
            objects,
            blocks,
            unit_system: on3dm_units(h),
            absolute_tolerance: on3dm_abs_tol(h),
            notes: cstr(on3dm_notes(h)),
            unsupported,
        };
        on3dm_free(h);
        Ok(model)
    }
}

fn pack_cvs(points: &[[f64; 3]], weights: &[f64]) -> Vec<f64> {
    let mut v = Vec::with_capacity(points.len() * 4);
    for (p, w) in points.iter().zip(weights) {
        v.extend_from_slice(&[p[0], p[1], p[2], *w]);
    }
    v
}

fn is_rational(weights: &[f64]) -> bool {
    weights.iter().any(|w| (w - 1.0).abs() > 1e-12)
}

/// Write a model to a `.3dm` file (Triceratops 7/8 archive). Returns the number of objects written.
pub fn write(model: &Model, path: &Path) -> Result<usize, Error> {
    let cpath = CString::new(path.to_string_lossy().as_bytes()).map_err(|_| Error::Path)?;
    unsafe {
        let h = on3dm_new();
        on3dm_set_units(h, model.unit_system, model.absolute_tolerance);
        if !model.notes.is_empty() {
            if let Ok(n) = CString::new(model.notes.as_str()) {
                on3dm_set_notes(h, n.as_ptr());
            }
        }
        // layers: file index assigned by openNURBS; map by our order
        let mut layer_map: Vec<i32> = Vec::new();
        for l in &model.layers {
            let name = CString::new(l.name.as_str()).unwrap_or_else(|_| CString::new("Layer").unwrap());
            let idx = on3dm_add_layer(h, name.as_ptr(), pack(l.color), l.visible as c_int, l.locked as c_int);
            layer_map.push(idx);
        }
        let mut written = 0;
        for b in &model.blocks {
            on3dm_idef_begin(h);
            for o in &b.objects {
                write_object(h, o, &layer_map);
            }
            let name = CString::new(b.name.as_str()).unwrap_or_else(|_| CString::new("Block").unwrap());
            on3dm_idef_end(h, name.as_ptr());
        }
        for o in &model.objects {
            if write_object(h, o, &layer_map) {
                written += 1;
            }
        }
        let ok = on3dm_write(h, cpath.as_ptr(), 0);
        on3dm_free(h);
        if ok == 0 {
            return Err(Error::Write(path.display().to_string()));
        }
        Ok(written)
    }
}

unsafe fn write_object(h: *mut On3dm, o: &Object, layer_map: &[i32]) -> bool {
    {
        {
            let layer = layer_map.get(o.layer as usize).copied().unwrap_or(0);
            let name = CString::new(o.name.as_str()).unwrap_or_else(|_| CString::new("").unwrap());
            let (has_color, rgbv) = match o.color {
                Some(c) => (1, pack(c)),
                None => (0, 0),
            };
            let ok = match &o.geometry {
                Geometry::Point(p) => on3dm_add_point(h, p.as_ptr(), layer, name.as_ptr(), has_color, rgbv, o.hidden as c_int, o.locked as c_int),
                Geometry::Curve(c) => {
                    let cv = pack_cvs(&c.control_points, &c.weights);
                    let k = on_knots(&c.knots);
                    on3dm_add_curve(h, is_rational(&c.weights) as c_int, (c.degree + 1) as c_int, c.control_points.len() as c_int, k.as_ptr(), cv.as_ptr(), layer, name.as_ptr(), has_color, rgbv, o.hidden as c_int, o.locked as c_int)
                }
                Geometry::Surface(s) => {
                    let cv = pack_cvs(&s.control_points, &s.weights);
                    let (ku, kv) = (on_knots(&s.knots_u), on_knots(&s.knots_v));
                    on3dm_add_surface(h, is_rational(&s.weights) as c_int, (s.degree_u + 1) as c_int, (s.degree_v + 1) as c_int, s.count_u as c_int, s.count_v as c_int, ku.as_ptr(), kv.as_ptr(), cv.as_ptr(), layer, name.as_ptr(), has_color, rgbv, o.hidden as c_int, o.locked as c_int)
                }
                Geometry::Brep { faces, .. } => {
                    on3dm_brep_begin(h);
                    for f in faces {
                        let s = &f.surface;
                        let cv = pack_cvs(&s.control_points, &s.weights);
                        let (ku, kv) = (on_knots(&s.knots_u), on_knots(&s.knots_v));
                        let sizes: Vec<c_int> = f.loops.iter().map(|l| l.len() as c_int).collect();
                        let uv: Vec<f64> = f.loops.iter().flat_map(|l| l.iter().flat_map(|p| [p[0], p[1]])).collect();
                        on3dm_brep_add_face(h, is_rational(&s.weights) as c_int, (s.degree_u + 1) as c_int, (s.degree_v + 1) as c_int, s.count_u as c_int, s.count_v as c_int, ku.as_ptr(), kv.as_ptr(), cv.as_ptr(), f.loops.len() as c_int, sizes.as_ptr(), uv.as_ptr());
                    }
                    on3dm_brep_end(h, layer, name.as_ptr(), has_color, rgbv, o.hidden as c_int, o.locked as c_int)
                }
                Geometry::Mesh { vertices, triangles } => {
                    let vs: Vec<f64> = vertices.iter().flat_map(|v| *v).collect();
                    let ts: Vec<c_int> = triangles.iter().flat_map(|t| [t[0] as c_int, t[1] as c_int, t[2] as c_int]).collect();
                    on3dm_add_mesh(h, vertices.len() as c_int, vs.as_ptr(), triangles.len() as c_int, ts.as_ptr(), layer, name.as_ptr(), has_color, rgbv, o.hidden as c_int, o.locked as c_int)
                }
                Geometry::Instance { block, xform } => on3dm_add_instance(h, *block as c_int, xform.as_ptr(), layer, name.as_ptr(), has_color, rgbv, o.hidden as c_int, o.locked as c_int),
            };
            ok != 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_point_curve_surface_mesh() {
        let dir = std::env::temp_dir().join(format!("on3dm-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.3dm");
        let curve = Curve {
            degree: 2,
            control_points: vec![[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [2.0, 0.0, 0.0]],
            weights: vec![1.0, 0.5, 1.0],
            knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        };
        let surface = Surface {
            degree_u: 1,
            degree_v: 1,
            count_u: 2,
            count_v: 2,
            control_points: vec![[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
            weights: vec![1.0; 4],
            knots_u: vec![0.0, 0.0, 1.0, 1.0],
            knots_v: vec![0.0, 0.0, 1.0, 1.0],
        };
        let model = Model {
            layers: vec![Layer { index: 0, name: "Default".into(), color: [0, 0, 0], visible: true, locked: false }, Layer { index: 1, name: "Red".into(), color: [255, 0, 0], visible: true, locked: false }],
            blocks: vec![BlockDef { name: "B".into(), objects: vec![Object { geometry: Geometry::Point([0.5, 0.0, 0.0]), layer: 0, name: String::new(), color: None, hidden: false, locked: false }] }],
            objects: vec![
                Object { geometry: Geometry::Instance { block: 0, xform: [1.0, 0.0, 0.0, 10.0, 0.0, 1.0, 0.0, 20.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0] }, layer: 0, name: "inst".into(), color: None, hidden: false, locked: false },
                Object { geometry: Geometry::Point([1.0, 2.0, 3.0]), layer: 1, name: "pt".into(), color: None, hidden: false, locked: false },
                Object { geometry: Geometry::Curve(curve.clone()), layer: 0, name: String::new(), color: Some([0, 128, 255]), hidden: false, locked: false },
                Object { geometry: Geometry::Surface(surface.clone()), layer: 0, name: String::new(), color: None, hidden: false, locked: false },
                Object { geometry: Geometry::Brep { faces: vec![Face { surface: surface.clone(), loops: vec![vec![[0.1, 0.1], [0.9, 0.1], [0.9, 0.9], [0.1, 0.9]]] }], solid: false }, layer: 0, name: String::new(), color: None, hidden: false, locked: false },
                Object { geometry: Geometry::Mesh { vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], triangles: vec![[0, 1, 2]] }, layer: 0, name: String::new(), color: None, hidden: true, locked: false },
            ],
            unit_system: UNITS_MILLIMETERS,
            absolute_tolerance: 0.001,
            notes: "hello".into(),
            unsupported: 0,
        };
        assert_eq!(write(&model, &p).unwrap(), 6);
        let m = read(&p).unwrap();
        assert_eq!(m.unit_system, UNITS_MILLIMETERS);
        assert_eq!(m.notes, "hello");
        assert!(m.layers.iter().any(|l| l.name == "Red" && l.color == [255, 0, 0]));
        assert_eq!(m.objects.len(), 6, "block members are not top-level objects");
        assert_eq!(m.blocks.len(), 1);
        assert_eq!(m.blocks[0].name, "B");
        assert_eq!(m.blocks[0].objects.len(), 1);
        let inst = m.objects.iter().find_map(|o| if let Geometry::Instance { block, xform } = &o.geometry { Some((*block, *xform)) } else { None }).unwrap();
        assert_eq!(inst.0, 0);
        assert!((inst.1[3] - 10.0).abs() < 1e-12 && (inst.1[7] - 20.0).abs() < 1e-12);
        let c = m.objects.iter().find_map(|o| if let Geometry::Curve(c) = &o.geometry { Some(c) } else { None }).unwrap();
        assert_eq!(c.degree, 2);
        assert!((c.weights[1] - 0.5).abs() < 1e-12);
        assert_eq!(c.knots, curve.knots);
        let b = m.objects.iter().find_map(|o| if let Geometry::Brep { faces, .. } = &o.geometry { Some(faces) } else { None }).unwrap();
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].loops.len(), 1);
        assert!(m.objects.iter().any(|o| o.hidden));
        std::fs::remove_dir_all(dir).ok();
    }
}
