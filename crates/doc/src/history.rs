//! Construction history (Record History / HistoryUpdate), cage editing records and hole
//! records: the data needed to replay a command, re-deform captives or re-cut holes when
//! their parents change.

use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::{Geometry, ObjectId};

/// A command input as recorded for history replay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum HistInput {
    Point(DVec3),
    Objects(Vec<ObjectId>),
    Pick(ObjectId, DVec3),
    Number(f64),
    Text(String),
    Option(String),
    Empty,
}

impl HistInput {
    pub fn object_ids(&self) -> Vec<ObjectId> {
        match self {
            HistInput::Objects(v) => v.clone(),
            HistInput::Pick(id, _) => vec![*id],
            _ => Vec::new(),
        }
    }
}

/// One recorded command: when any input object changes the command is replayed and the
/// outputs replaced.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub id: u32,
    pub command: String,
    pub inputs: Vec<HistInput>,
    pub outputs: Vec<ObjectId>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

impl HistoryRecord {
    pub fn parents(&self) -> Vec<ObjectId> {
        let mut v: Vec<ObjectId> = self.inputs.iter().flat_map(|i| i.object_ids()).collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// Free-form deformation record: a cage of point objects and the objects it deforms.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CageRecord {
    pub id: u32,
    /// Cage point objects in (i, j, k) order.
    pub points: Vec<ObjectId>,
    pub dims: (usize, usize, usize),
    /// Cage point positions when the captives were captured.
    pub origin_points: Vec<DVec3>,
    /// Box of the undeformed cage (local coordinates are relative to it).
    pub min: DVec3,
    pub max: DVec3,
    /// Captive objects with their undeformed geometry.
    pub captives: Vec<(ObjectId, Geometry)>,
}

fn bernstein(n: usize, i: usize, t: f64) -> f64 {
    // n = degree
    let mut c = 1.0;
    for k in 0..i {
        c *= (n - k) as f64 / (k + 1) as f64;
    }
    c * t.powi(i as i32) * (1.0 - t).powi((n - i) as i32)
}

impl CageRecord {
    /// Map a point through the deformed cage (Bézier free-form deformation).
    pub fn deform_point(&self, current: &[DVec3], p: DVec3) -> DVec3 {
        let size = (self.max - self.min).max(DVec3::splat(1e-12));
        let s = ((p.x - self.min.x) / size.x, (p.y - self.min.y) / size.y, (p.z - self.min.z) / size.z);
        let (nu, nv, nw) = self.dims;
        let mut out = DVec3::ZERO;
        for i in 0..nu {
            let bi = bernstein(nu - 1, i, s.0);
            for j in 0..nv {
                let bj = bernstein(nv - 1, j, s.1);
                for k in 0..nw {
                    let bk = bernstein(nw - 1, k, s.2);
                    let idx = (i * nv + j) * nw + k;
                    let q = current.get(idx).copied().unwrap_or(DVec3::ZERO);
                    out += q * (bi * bj * bk);
                }
            }
        }
        out
    }

    pub fn deform_geometry(&self, current: &[DVec3], g: &Geometry) -> Geometry {
        let mut g = g.clone();
        let f = |p: DVec3| self.deform_point(current, p);
        match &mut g {
            Geometry::Point(p) => *p = f(*p),
            Geometry::Curve(c) => c.control_points.iter_mut().for_each(|p| *p = f(*p)),
            Geometry::Surface(s) => s.control_points.iter_mut().for_each(|p| *p = f(*p)),
            Geometry::Brep(b) => b.faces.iter_mut().for_each(|fc| fc.surface.control_points.iter_mut().for_each(|p| *p = f(*p))),
            Geometry::Mesh(m) => {
                m.vertices.iter_mut().for_each(|p| *p = f(*p));
                m.compute_normals();
            }
            Geometry::Instance { xf, .. } => xf.translation = f(xf.translation),
        }
        g
    }
}

/// A hole cut into an object: the target keeps its original geometry so holes can be moved,
/// copied or removed and the cut repeated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoleRecord {
    pub target: ObjectId,
    pub original: Geometry,
    /// Closed planar curves (objects, usually hidden) that define the holes.
    pub curves: Vec<ObjectId>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    pub records: Vec<HistoryRecord>,
    pub cages: Vec<CageRecord>,
    pub holes: Vec<HoleRecord>,
}

impl History {
    pub fn next_id(&self) -> u32 {
        self.records.iter().map(|r| r.id).chain(self.cages.iter().map(|c| c.id)).max().map_or(1, |m| m + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffd_identity_and_translation() {
        let mut pts = Vec::new();
        for i in 0..2 {
            for j in 0..2 {
                for k in 0..2 {
                    pts.push(DVec3::new(i as f64, j as f64, k as f64));
                }
            }
        }
        let rec = CageRecord { id: 1, points: vec![], dims: (2, 2, 2), origin_points: pts.clone(), min: DVec3::ZERO, max: DVec3::ONE, captives: vec![] };
        let p = DVec3::new(0.25, 0.5, 0.75);
        assert!((rec.deform_point(&pts, p) - p).length() < 1e-12);
        let moved: Vec<DVec3> = pts.iter().map(|q| *q + DVec3::X).collect();
        assert!((rec.deform_point(&moved, p) - (p + DVec3::X)).length() < 1e-12);
    }
}
