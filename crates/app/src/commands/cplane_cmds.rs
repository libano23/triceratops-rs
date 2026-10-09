//! Construction plane commands (CPlane with World/3Point/Object/View/Previous options),
//! rounded rectangles, Line sub-options and surface Rebuild.

use glam::DVec3;
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{NurbsCurve, NurbsSurface, Plane};

use super::{fmt_num, opt, optv, AppRequest, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

#[derive(Default)]
pub struct CPlaneCmd {
    pts: Vec<DVec3>,
    mode: Option<&'static str>,
}

impl CPlaneCmd {
    fn main_prompt(&self) -> Prompt {
        Prompt::point("CPlane origin").opts([
            opt("3Point"),
            opt("Object"),
            opt("View"),
            opt("World"),
            opt("Previous"),
            opt("Next"),
            opt("Elevation"),
        ])
    }
}

impl Command for CPlaneCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        self.main_prompt().step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.mode, input) {
            (_, Input::Option(o)) => match o.as_str() {
                "World" => Prompt::options("World construction plane").opts([opt("Top"), opt("Bottom"), opt("Front"), opt("Back"), opt("Right"), opt("Left")]).step(),
                "Top" | "Bottom" | "Front" | "Back" | "Right" | "Left" => {
                    let plane = match o.as_str() {
                        "Top" => Plane::world_xy(),
                        "Bottom" => Plane { x_axis: DVec3::X, y_axis: -DVec3::Y, z_axis: -DVec3::Z, ..Plane::world_xy() },
                        "Front" => Plane::world_xz(),
                        "Back" => Plane { x_axis: -DVec3::X, y_axis: DVec3::Z, z_axis: DVec3::Y, ..Plane::world_xy() },
                        "Right" => Plane::world_yz(),
                        _ => Plane { x_axis: -DVec3::Y, y_axis: DVec3::Z, z_axis: -DVec3::X, ..Plane::world_xy() },
                    };
                    ctx.requests.push(AppRequest::SetCPlane(Some(plane)));
                    Step::Done
                }
                "3Point" => {
                    self.mode = Some("3Point");
                    Prompt::point("CPlane origin").step()
                }
                "Object" => {
                    self.mode = Some("Object");
                    Prompt::objects("Select object to align CPlane to", ObjFilter::Any, 1).step()
                }
                "View" => {
                    ctx.requests.push(AppRequest::CPlaneToView);
                    Step::Done
                }
                "Previous" => {
                    ctx.requests.push(AppRequest::SetCPlane(None));
                    Step::Done
                }
                "Elevation" => {
                    self.mode = Some("Elevation");
                    Prompt::number("Elevation").default_num(0.0).step()
                }
                _ => self.main_prompt().step(),
            },
            (Some("Elevation"), Input::Number(e)) => {
                let mut p = ctx.cplane;
                p.origin += p.z_axis * e;
                ctx.requests.push(AppRequest::SetCPlane(Some(p)));
                Step::Done
            }
            (Some("Object"), Input::Objects(ids)) => {
                let Some(o) = ids.first().and_then(|id| ctx.doc.get(*id)) else { return Step::Cancelled };
                let plane = match &o.geometry {
                    Geometry::Curve(c) => tri_geom::curve_plane(c).map(|p| Plane { origin: c.start_point(), ..p }),
                    Geometry::Surface(s) => {
                        let (u0, u1) = s.domain_u();
                        let (v0, v1) = s.domain_v();
                        let (p, su, sv) = s.evaluate((u0 + u1) * 0.5, (v0 + v1) * 0.5);
                        Some(Plane::from_frame(p, su, sv))
                    }
                    Geometry::Brep(b) => b.faces.first().map(|f| {
                        let (p, n) = f.sample();
                        Plane::from_normal(p, n)
                    }),
                    _ => None,
                };
                match plane {
                    Some(p) => {
                        ctx.requests.push(AppRequest::SetCPlane(Some(p)));
                        Step::Done
                    }
                    None => {
                        ctx.log("Unable to determine a plane from the object.");
                        Step::Cancelled
                    }
                }
            }
            (mode, Input::Point(p)) => {
                self.pts.push(p);
                let three = mode == Some("3Point");
                match self.pts.len() {
                    1 => Prompt::point("X axis direction").step(),
                    2 if three => Prompt::point("CPlane orientation (point on positive Y side)").step(),
                    2 => {
                        let x = self.pts[1] - self.pts[0];
                        let z = ctx.cplane.z_axis;
                        let plane = Plane::from_frame(self.pts[0], x, z.cross(x));
                        ctx.requests.push(AppRequest::SetCPlane(Some(plane)));
                        Step::Done
                    }
                    _ => {
                        let plane = Plane::from_frame(self.pts[0], self.pts[1] - self.pts[0], self.pts[2] - self.pts[0]);
                        ctx.requests.push(AppRequest::SetCPlane(Some(plane)));
                        Step::Done
                    }
                }
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.pts.first(), cursor) {
            (Some(o), Some(c)) => vec![Preview::Line(*o, c)],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

/// Rectangle with rounded corners (arcs of `radius`) from two corners on `plane`.
pub fn rounded_rectangle(plane: &Plane, a: DVec3, b: DVec3, radius: f64) -> Option<NurbsCurve> {
    let (u0, v0, w) = plane.closest_parameters(a);
    let (u1, v1, _) = plane.closest_parameters(b);
    let (umin, umax) = (u0.min(u1), u0.max(u1));
    let (vmin, vmax) = (v0.min(v1), v0.max(v1));
    let r = radius.min((umax - umin) * 0.5).min((vmax - vmin) * 0.5).max(0.0);
    let p = |u: f64, v: f64| plane.point_at_3(u, v, w);
    if r < 1e-9 {
        return NurbsCurve::polyline(vec![p(umin, vmin), p(umax, vmin), p(umax, vmax), p(umin, vmax), p(umin, vmin)]).ok();
    }
    let pl = |u: f64, v: f64| Plane { origin: p(u, v), ..*plane };
    use std::f64::consts::FRAC_PI_2;
    let segs: Vec<NurbsCurve> = vec![
        NurbsCurve::line(p(umin + r, vmin), p(umax - r, vmin)).ok()?,
        NurbsCurve::arc(&pl(umax - r, vmin + r), r, -FRAC_PI_2, 0.0).ok()?,
        NurbsCurve::line(p(umax, vmin + r), p(umax, vmax - r)).ok()?,
        NurbsCurve::arc(&pl(umax - r, vmax - r), r, 0.0, FRAC_PI_2).ok()?,
        NurbsCurve::line(p(umax - r, vmax), p(umin + r, vmax)).ok()?,
        NurbsCurve::arc(&pl(umin + r, vmax - r), r, FRAC_PI_2, std::f64::consts::PI).ok()?,
        NurbsCurve::line(p(umin, vmax - r), p(umin, vmin + r)).ok()?,
        NurbsCurve::arc(&pl(umin + r, vmin + r), r, std::f64::consts::PI, 3.0 * FRAC_PI_2).ok()?,
    ];
    let mut out = segs[0].clone();
    for s in &segs[1..] {
        out = out.join_any(s).ok()?;
    }
    Some(out)
}

#[derive(Default)]
pub struct RoundedRectCmd {
    corner: Option<DVec3>,
    other: Option<DVec3>,
    plane: Plane,
}

impl Command for RoundedRectCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        Prompt::point("First corner of rectangle").opts([optv("Rounded", "Yes")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.corner, self.other, input) {
            (None, _, Input::Point(p)) => {
                self.corner = Some(p);
                Prompt::point("Other corner or length").step()
            }
            (Some(_), None, Input::Point(p)) => {
                self.other = Some(p);
                Prompt::point_or_number("Radius or point for corner").default_num(ctx.mem_num("Rectangle.Radius", 1.0)).opts([optv("Corner", "Arc")]).step()
            }
            (Some(a), Some(b), inp @ (Input::Point(_) | Input::Number(_))) => {
                let r = match inp {
                    Input::Number(v) => v.abs(),
                    Input::Point(p) => {
                        let (u, _, _) = self.plane.closest_parameters(p);
                        let (ua, _, _) = self.plane.closest_parameters(a);
                        let (ub, _, _) = self.plane.closest_parameters(b);
                        (u - ua.min(ub)).abs().min((ua.max(ub) - u).abs())
                    }
                    _ => 1.0,
                };
                ctx.mem_set("Rectangle.Radius", fmt_num(r));
                match rounded_rectangle(&self.plane, a, b, r) {
                    Some(c) => {
                        ctx.doc.begin_undo("Rectangle");
                        ctx.doc.add(Geometry::Curve(c));
                        Step::Done
                    }
                    None => Step::Cancelled,
                }
            }
            (_, _, Input::Option(_)) => Prompt::point("First corner of rectangle").step(),
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.corner, self.other, cursor) {
            (Some(a), None, Some(c)) => vec![Preview::Polyline(super::create::rect_points(&self.plane, a, c))],
            (Some(a), Some(b), Some(c)) => {
                let (u, _, _) = self.plane.closest_parameters(c);
                let (ua, _, _) = self.plane.closest_parameters(a);
                let (ub, _, _) = self.plane.closest_parameters(b);
                let r = (u - ua.min(ub)).abs().min((ua.max(ub) - u).abs());
                rounded_rectangle(&self.plane, a, b, r).map(|cv| vec![Preview::Curve(cv)]).unwrap_or_default()
            }
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.other.or(self.corner)
    }
}

/// Rebuild surfaces with a point count in U and V (Triceratops' Rebuild for surfaces).
pub struct RebuildSrfCmd {
    ids: Vec<ObjectId>,
    counts: (usize, usize),
    degree: (usize, usize),
    asking: Option<&'static str>,
}

impl Default for RebuildSrfCmd {
    fn default() -> Self {
        Self { ids: Vec::new(), counts: (10, 10), degree: (3, 3), asking: None }
    }
}

impl RebuildSrfCmd {
    fn prompt(&self) -> Prompt {
        Prompt::options("Rebuild options. Press Enter to rebuild").opts([
            optv("PointCountU", self.counts.0),
            optv("PointCountV", self.counts.1),
            optv("DegreeU", self.degree.0),
            optv("DegreeV", self.degree.1),
            optv("DeleteInput", "Yes"),
        ])
    }
}

impl Command for RebuildSrfCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select surfaces to rebuild", ObjFilter::Surface, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                self.prompt().step()
            }
            Input::Option(o) => {
                let (label, def) = match o.as_str() {
                    "PointCountU" => ("Point count in U", self.counts.0 as f64),
                    "PointCountV" => ("Point count in V", self.counts.1 as f64),
                    "DegreeU" => ("Degree in U", self.degree.0 as f64),
                    "DegreeV" => ("Degree in V", self.degree.1 as f64),
                    _ => return self.prompt().step(),
                };
                self.asking = Some(match o.as_str() {
                    "PointCountU" => "PointCountU",
                    "PointCountV" => "PointCountV",
                    "DegreeU" => "DegreeU",
                    _ => "DegreeV",
                });
                Prompt::number(label).default_num(def).step()
            }
            Input::Number(v) => {
                let n = v.round().max(1.0) as usize;
                match self.asking.take() {
                    Some("PointCountU") => self.counts.0 = n.max(2),
                    Some("PointCountV") => self.counts.1 = n.max(2),
                    Some("DegreeU") => self.degree.0 = n.min(11),
                    Some("DegreeV") => self.degree.1 = n.min(11),
                    _ => {}
                }
                self.prompt().step()
            }
            Input::Empty if self.asking.is_some() => {
                self.asking = None;
                self.prompt().step()
            }
            Input::Empty => {
                ctx.doc.begin_undo("Rebuild");
                for id in &self.ids {
                    let Some(Geometry::Surface(s)) = ctx.doc.get(*id).map(|o| o.geometry.clone()) else { continue };
                    let (u0, u1) = s.domain_u();
                    let (v0, v1) = s.domain_v();
                    let (nu, nv) = self.counts;
                    let grid: Vec<Vec<DVec3>> = (0..nu)
                        .map(|i| (0..nv).map(|j| s.point_at(u0 + (u1 - u0) * i as f64 / (nu - 1) as f64, v0 + (v1 - v0) * j as f64 / (nv - 1) as f64)).collect())
                        .collect();
                    match NurbsSurface::interpolate_grid(&grid, self.degree.0, self.degree.1) {
                        Ok(ns) => ctx.doc.replace_geometry(*id, Geometry::Surface(ns)),
                        Err(e) => {
                            ctx.log(format!("Rebuild failed: {e}"));
                            false
                        }
                    };
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

/// Triceratops' Rebuild handles curves and surfaces; route by the selected object types.
pub struct RebuildAnyCmd {
    curves: super::curve_edit::RebuildCmd,
    surfaces: RebuildSrfCmd,
    routed: Option<bool>,
}

impl Default for RebuildAnyCmd {
    fn default() -> Self {
        Self { curves: super::curve_edit::RebuildCmd::default(), surfaces: RebuildSrfCmd::default(), routed: None }
    }
}

impl Command for RebuildAnyCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves or surfaces to rebuild", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.routed, input) {
            (None, Input::Objects(ids)) => {
                let any_srf = ids.iter().filter_map(|id| ctx.doc.get(*id)).any(|o| matches!(o.geometry, Geometry::Surface(_)));
                self.routed = Some(any_srf);
                if any_srf {
                    let _ = self.surfaces.start(ctx);
                    self.surfaces.input(Input::Objects(ids), ctx)
                } else {
                    let _ = self.curves.start(ctx);
                    self.curves.input(Input::Objects(ids), ctx)
                }
            }
            (Some(true), inp) => self.surfaces.input(inp, ctx),
            (Some(false), inp) => self.curves.input(inp, ctx),
            _ => Step::Cancelled,
        }
    }
}
