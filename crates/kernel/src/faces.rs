//! A solid's record as `tri_geom` faces: the face surfaces as `NurbsSurface`s in the record's
//! own (u, v), and trim loops sampled from the edges.
//!
//! Every edge is sampled once, at parameters shared by both faces that use it; each face then
//! evaluates its own pcurve at those parameters, so neighbouring trims meet at the same points
//! of the solid (to the kernel's tolerance) and the display has no cracks.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{DVec2, DVec3};
use solidcore_record::brep::{Brep as Record, Curve2, Loop, Shell, Surface};
use solidcore_tess::{curve_points, MeshTolerance};
use tri_geom::{Brep, Face, NurbsCurve, NurbsSurface};

use crate::solid::{refused, KernelError, Solid};

impl Solid {
    /// The solid as a closed `Brep`: one `Face` per record face, `chord` the farthest a trim
    /// segment may stand off its edge (mm). The exact body is attached to it.
    pub fn to_brep(&self, chord: f64) -> Result<Brep, KernelError> {
        let record = self.record()?;
        let faces = faces_of(&record, chord)?;
        let mut b = Brep::new(faces);
        // the kernel is the authority on closure: its own check is the one that decides
        b.closed = closed_solid(&record);
        if !b.closed {
            return Err(refused("the kernel body is not a closed solid"));
        }
        b.attach_exact_measured(Arc::new(record), self.volume().ok(), self.area().ok());
        Ok(b)
    }

    /// The solid behind `brep`'s exact body, only while that body still matches its faces.
    pub fn from_exact(brep: &Brep) -> Option<Solid> {
        let exact = brep.exact_valid()?;
        let (body, _order) = solidcore_ops::Body::from_record(&exact.record, exact.trusted).ok()?;
        if exact.placement != glam::DAffine3::IDENTITY {
            body.transform(&crate::convert::matrix(&exact.placement)).ok()?;
        }
        Some(Solid::new(body))
    }
}

fn closed_solid(r: &Record) -> bool {
    r.regions.iter().skip(1).any(|reg| {
        !reg.void && reg.shells.first().is_some_and(|s| matches!(r.shells.get(*s as usize), Some(Shell::Faces { .. })))
    })
}

fn faces_of(r: &Record, chord: f64) -> Result<Vec<Face>, KernelError> {
    let chord = if chord.is_finite() && chord > 0.0 { chord } else { 0.05 };
    let mut samples: HashMap<u32, usize> = HashMap::new();
    let mut out: Vec<Option<Face>> = vec![None; r.faces.len()];
    for shell in &r.shells {
        let Shell::Faces { faces, .. } = shell else { continue };
        for of in faces {
            let slot = out.get_mut(of.face as usize).ok_or_else(|| refused("the record names a missing face"))?;
            if slot.is_some() {
                continue;
            }
            let mut f = face_of(r, of.face as usize, chord, &mut samples)?;
            if !of.forward {
                f.flip();
            }
            *slot = Some(f);
        }
    }
    out.into_iter().map(|f| f.ok_or_else(|| refused("a record face belongs to no face shell"))).collect()
}

fn face_of(r: &Record, i: usize, chord: f64, counts: &mut HashMap<u32, usize>) -> Result<Face, KernelError> {
    let rf = r.faces.get(i).ok_or_else(|| refused("the record names a missing face"))?;
    let surface = nurbs_surface(rf.nurbs.as_ref().unwrap_or(&rf.surface), rf.domain)?;
    let mut loops: Vec<Vec<DVec2>> = Vec::new();
    for lp in &rf.loops {
        let Loop::Edges { edges } = lp else { continue }; // a pole or collapsed boundary: no trim
        let mut uv: Vec<DVec2> = Vec::new();
        for oe in edges {
            let e = r.edges.get(oe.edge as usize).ok_or_else(|| refused("the record names a missing edge"))?;
            let n = match counts.get(&oe.edge) {
                Some(n) => *n,
                None => {
                    let curve = e.nurbs.as_ref().unwrap_or(&e.curve);
                    let pts = curve_points(curve, e.interval, MeshTolerance::display(chord)).map_err(|x| refused(&x.to_string()))?;
                    let n = pts.len().saturating_sub(1).max(1);
                    counts.insert(oe.edge, n);
                    n
                }
            };
            let pc = pcurve(&oe.pcurve)?;
            // the record says a pcurve runs on its edge's own interval; a different range would
            // misplace every trim point, so it is refused rather than guessed
            let (a, b) = (e.interval[0], e.interval[1]);
            let (pa, pb) = pc.domain();
            let tol = 1e-9 * (b - a).abs().max(1.0);
            if (pa - a).abs() > tol || (pb - b).abs() > tol {
                return Err(refused("a pcurve does not run on its edge's interval"));
            }
            let mut pts: Vec<DVec2> = (0..=n)
                .map(|k| {
                    let p = pc.point_at(a + (b - a) * k as f64 / n as f64);
                    DVec2::new(p.x, p.y)
                })
                .collect();
            if !oe.forward {
                pts.reverse();
            }
            pts.pop(); // the next edge starts here
            uv.extend(pts);
        }
        if uv.len() >= 3 {
            loops.push(uv);
        }
    }
    Ok(Face { surface, loops })
}

/// A pcurve as a 2D curve (the third coordinate is 0).
fn pcurve(c: &Curve2) -> Result<NurbsCurve, KernelError> {
    let bad = |e: tri_geom::GeomError| refused(&e.to_string());
    match c {
        Curve2::Spline { knots, points } => {
            let degree = knots.len().checked_sub(points.len() + 1).ok_or_else(|| refused("a pcurve has too few knots"))?;
            let cps = points.iter().map(|p| DVec3::new(p[0], p[1], 0.0)).collect::<Vec<_>>();
            NurbsCurve::new(degree, cps, vec![1.0; points.len()], knots.clone()).map_err(bad)
        }
        Curve2::RationalSpline { knots, homogeneous } => {
            let degree = knots.len().checked_sub(homogeneous.len() + 1).ok_or_else(|| refused("a pcurve has too few knots"))?;
            let cps = homogeneous.iter().map(|h| DVec3::new(h[0] / h[2], h[1] / h[2], 0.0)).collect::<Vec<_>>();
            NurbsCurve::new(degree, cps, homogeneous.iter().map(|h| h[2]).collect(), knots.clone()).map_err(bad)
        }
    }
}

/// The face's surface in the parameters its loops are in.
fn nurbs_surface(s: &Surface, domain: [[f64; 2]; 2]) -> Result<NurbsSurface, KernelError> {
    let bad = |e: tri_geom::GeomError| refused(&e.to_string());
    let v3 = |p: &[f64; 3]| DVec3::new(p[0], p[1], p[2]);
    match s {
        Surface::Spline { u_knots, v_knots, points } => {
            let (cu, cv) = (points.len(), points.first().map_or(0, Vec::len));
            let du = u_knots.len().checked_sub(cu + 1).ok_or_else(|| refused("a surface has too few knots"))?;
            let dv = v_knots.len().checked_sub(cv + 1).ok_or_else(|| refused("a surface has too few knots"))?;
            let cps = points.iter().flat_map(|row| row.iter().map(v3)).collect::<Vec<_>>();
            NurbsSurface::new(du, dv, cu, cv, cps, vec![1.0; cu * cv], u_knots.clone(), v_knots.clone()).map_err(bad)
        }
        Surface::RationalSpline { u_knots, v_knots, homogeneous } => {
            let (cu, cv) = (homogeneous.len(), homogeneous.first().map_or(0, Vec::len));
            let du = u_knots.len().checked_sub(cu + 1).ok_or_else(|| refused("a surface has too few knots"))?;
            let dv = v_knots.len().checked_sub(cv + 1).ok_or_else(|| refused("a surface has too few knots"))?;
            let hs = homogeneous.iter().flatten().collect::<Vec<_>>();
            let cps = hs.iter().map(|h| DVec3::new(h[0] / h[3], h[1] / h[3], h[2] / h[3])).collect::<Vec<_>>();
            let ws = hs.iter().map(|h| h[3]).collect::<Vec<_>>();
            NurbsSurface::new(du, dv, cu, cv, cps, ws, u_knots.clone(), v_knots.clone()).map_err(bad)
        }
        Surface::Plane { origin, u_axis, v_axis } => plane_patch(v3(origin), v3(u_axis), v3(v_axis), domain),
        Surface::PlaneThrough { origin, u_point, v_point } => {
            let o = v3(origin);
            plane_patch(o, v3(u_point) - o, v3(v_point) - o, domain)
        }
        _ => Err(refused("a face surface has no NURBS form in the record")),
    }
}

/// A plane over `domain` as a bilinear patch whose parameters are the plane's own.
fn plane_patch(o: DVec3, ua: DVec3, va: DVec3, d: [[f64; 2]; 2]) -> Result<NurbsSurface, KernelError> {
    let at = |u: f64, v: f64| o + ua * u + va * v;
    let cps = vec![at(d[0][0], d[1][0]), at(d[0][0], d[1][1]), at(d[0][1], d[1][0]), at(d[0][1], d[1][1])];
    NurbsSurface::new(
        1,
        1,
        2,
        2,
        cps,
        vec![1.0; 4],
        vec![d[0][0], d[0][0], d[0][1], d[0][1]],
        vec![d[1][0], d[1][0], d[1][1], d[1][1]],
    )
    .map_err(|e| refused(&e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use solidcore_record::brep::Curve2;

    fn box_record() -> Record {
        Solid::cuboid(DVec3::ZERO, DVec3::new(10.0, 20.0, 30.0)).unwrap().record().unwrap()
    }

    #[test]
    fn an_untouched_record_converts() {
        assert_eq!(faces_of(&box_record(), 0.05).unwrap().len(), 6);
    }

    #[test]
    fn a_pcurve_on_a_different_range_than_its_edge_is_refused_not_misplaced() {
        let mut r = box_record();
        let oe = &mut r.faces[0].loops.iter_mut().find_map(|l| if let Loop::Edges { edges } = l { edges.first_mut() } else { None }).unwrap();
        match &mut oe.pcurve {
            Curve2::Spline { knots, .. } | Curve2::RationalSpline { knots, .. } => knots.iter_mut().for_each(|k| *k *= 2.0),
        }
        let e = faces_of(&r, 0.05).err().expect("a pcurve that does not follow its edge's interval must be refused");
        assert!(e.to_string().contains("pcurve"), "{e}");
    }
}
