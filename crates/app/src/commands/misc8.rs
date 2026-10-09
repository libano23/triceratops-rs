//! Final batch: view capture / Render / Print, Turntable, Sun, GroundPlane, draw order, BoxEdit,
//! CalcRPN, DupFaceBorder, JoinCopy, PointCloud, SplitFace, Zoom helpers.

use glam::DVec3;
use tri_doc::{Geometry, ObjectId};
use tri_geom::{mesh_ops, NurbsCurve};

use super::simple::{add_all, Ans, Ask, SeqCmd};
use super::surface_edit::{face_under, faces_of, split_faces};
use super::{fmt_num, opt, optv, AppRequest, CmdCtx, Command, Input, ObjFilter, Prompt, Step};

pub fn capture_cmd(label: &'static str, clipboard: bool, whole: bool) -> SeqCmd {
    let asks = if clipboard { vec![] } else { vec![Ask::Text("File name (empty for a dialog)")] };
    SeqCmd::new(asks, move |a, ctx| {
        let mut path = a.first().map(|t| t.text()).unwrap_or_default();
        if whole {
            path = format!("screen:{path}");
        }
        ctx.log(format!("{label}: capturing the active viewport."));
        ctx.requests.push(AppRequest::ViewCapture { path, clipboard });
        Step::Done
    })
}

pub fn render_cmd() -> SeqCmd {
    SeqCmd::new(vec![], |_, ctx| {
        ctx.log("Render: the active viewport is captured with its current display mode (set Rendered mode for shading with colours).");
        ctx.requests.push(AppRequest::ViewCapture { path: String::new(), clipboard: false });
        Step::Done
    })
}

pub fn batch_render() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Output folder")], |a, ctx| {
        let dir = a[0].text();
        if dir.is_empty() {
            return Step::Cancelled;
        }
        let _ = std::fs::create_dir_all(&dir);
        ctx.requests.push(AppRequest::BatchRenderNamedViews(dir));
        Step::Done
    })
}

pub struct TurntableCmd {
    frames: f64,
    record: bool,
}

impl Default for TurntableCmd {
    fn default() -> Self {
        Self { frames: 72.0, record: false }
    }
}

impl Command for TurntableCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.frames = ctx.mem_num("Turntable.Frames", 72.0);
        Prompt::options("Press Enter to start the turntable").opts([optv("Frames", fmt_num(self.frames)), optv("Record", if self.record { "Yes" } else { "No" })]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if o == "Frames" => Prompt::number("Number of frames").default_num(self.frames).step(),
            Input::Option(o) if o == "Record" => {
                self.record = !self.record;
                Prompt::options("Press Enter to start the turntable").opts([optv("Frames", fmt_num(self.frames)), optv("Record", if self.record { "Yes" } else { "No" })]).step()
            }
            Input::Option(_) => Prompt::options("Press Enter to start the turntable").step(),
            Input::Number(n) => {
                self.frames = n.round().clamp(2.0, 3600.0);
                ctx.mem_set("Turntable.Frames", fmt_num(self.frames));
                Prompt::options("Press Enter to start the turntable").opts([optv("Frames", fmt_num(self.frames))]).step()
            }
            Input::Empty => {
                let record = if self.record {
                    let dir = std::env::temp_dir().join("triceratops-turntable");
                    let _ = std::fs::create_dir_all(&dir);
                    ctx.log(format!("Recording frames to {}", dir.display()));
                    Some(dir.to_string_lossy().to_string())
                } else {
                    None
                };
                ctx.requests.push(AppRequest::Turntable { frames: self.frames as usize, record });
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

pub struct SunCmd;

impl Command for SunCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        Prompt::options("Sun").opts([opt("On"), opt("Off"), optv("Azimuth", fmt_num(ctx.mem_num("Sun.Azimuth", 135.0))), optv("Altitude", fmt_num(ctx.mem_num("Sun.Altitude", 45.0)))]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if o == "Azimuth" => {
                ctx.mem_set("Sun.Asking", "Azimuth");
                Prompt::number("Azimuth (degrees from north, clockwise)").default_num(ctx.mem_num("Sun.Azimuth", 135.0)).step()
            }
            Input::Option(o) if o == "Altitude" => {
                ctx.mem_set("Sun.Asking", "Altitude");
                Prompt::number("Altitude (degrees above the horizon)").default_num(ctx.mem_num("Sun.Altitude", 45.0)).step()
            }
            Input::Number(v) => {
                let key = if ctx.memory.remove("Sun.Asking").as_deref() == Some("Azimuth") { "Sun.Azimuth" } else { "Sun.Altitude" };
                ctx.mem_set(key, fmt_num(v));
                ctx.requests.push(AppRequest::Sun(Some((ctx.mem_num("Sun.Azimuth", 135.0), ctx.mem_num("Sun.Altitude", 45.0)))));
                self.start(ctx)
            }
            Input::Option(o) if o == "On" => {
                ctx.requests.push(AppRequest::Sun(Some((ctx.mem_num("Sun.Azimuth", 135.0), ctx.mem_num("Sun.Altitude", 45.0)))));
                Step::Done
            }
            Input::Option(_) => {
                ctx.requests.push(AppRequest::Sun(None));
                Step::Done
            }
            _ => Step::Done,
        }
    }
}

pub struct GroundPlaneCmd;

impl Command for GroundPlaneCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        Prompt::options("Ground plane").opts([opt("On"), opt("Off"), optv("Height", fmt_num(ctx.mem_num("GroundPlane.Height", 0.0)))]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if o == "Height" => Prompt::number("Ground plane height").default_num(ctx.mem_num("GroundPlane.Height", 0.0)).step(),
            Input::Number(h) => {
                ctx.mem_set("GroundPlane.Height", fmt_num(h));
                ctx.requests.push(AppRequest::GroundPlane(Some(h)));
                Step::Done
            }
            Input::Option(o) if o == "On" => {
                ctx.requests.push(AppRequest::GroundPlane(Some(ctx.mem_num("GroundPlane.Height", 0.0))));
                Step::Done
            }
            Input::Option(_) => {
                ctx.requests.push(AppRequest::GroundPlane(None));
                Step::Done
            }
            _ => Step::Done,
        }
    }
}

pub fn draw_order(delta: i32, absolute: bool) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects", ObjFilter::Any, 1)], move |a, ctx| {
        let (lo, hi) = ctx.doc.objects().fold((0i32, 0i32), |(l, h), o| (l.min(o.attributes.draw_order), h.max(o.attributes.draw_order)));
        ctx.doc.begin_undo("DrawOrder");
        for id in a[0].ids() {
            if let Some(o) = ctx.doc.get_mut(id) {
                o.attributes.draw_order = match (absolute, delta) {
                    (true, 0) => 0,
                    (true, d) if d > 0 => hi + 1,
                    (true, _) => lo - 1,
                    (false, d) => o.attributes.draw_order + d,
                };
            }
        }
        Step::Done
    })
}

pub fn box_edit() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to edit", ObjFilter::Any, 1), Ask::Number("Size X (Enter keeps)", 0.0), Ask::Number("Size Y (Enter keeps)", 0.0), Ask::Number("Size Z (Enter keeps)", 0.0), Ask::Number("Position X of the bounding box corner (Enter keeps)", f64::NAN), Ask::Number("Position Y (Enter keeps)", f64::NAN), Ask::Number("Position Z (Enter keeps)", f64::NAN)], |a, ctx| {
        let ids = a[0].ids();
        let bb = ids.iter().filter_map(|id| ctx.doc.get(*id)).fold(tri_geom::BoundingBox::EMPTY, |b, o| b.union(&ctx.doc.object_bbox(o)));
        if bb.is_empty() {
            return Step::Cancelled;
        }
        let size = bb.max - bb.min;
        let want = |i: usize, cur: f64| { let v = a[i].number(); if v > 0.0 { v } else { cur } };
        let s = DVec3::new(want(1, size.x) / size.x.max(1e-12), want(2, size.y) / size.y.max(1e-12), want(3, size.z) / size.z.max(1e-12));
        let pos = |i: usize, cur: f64| { let v = a[i].number(); if v.is_nan() { cur } else { v } };
        let target = DVec3::new(pos(4, bb.min.x), pos(5, bb.min.y), pos(6, bb.min.z));
        let xf = glam::DAffine3::from_translation(target) * glam::DAffine3::from_scale(s) * glam::DAffine3::from_translation(-bb.min);
        ctx.doc.begin_undo("BoxEdit");
        ctx.doc.transform_objects(&ids, &xf);
        Step::Done
    })
}

pub fn calc_rpn() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("RPN expression (e.g. 3 4 + 2 *)")], |a, ctx| {
        let mut stack: Vec<f64> = Vec::new();
        for tok in a[0].text().split_whitespace() {
            match tok {
                "+" | "-" | "*" | "/" | "^" => {
                    let (Some(y), Some(x)) = (stack.pop(), stack.pop()) else { ctx.log("Stack underflow."); return Step::Done };
                    stack.push(match tok { "+" => x + y, "-" => x - y, "*" => x * y, "/" => x / y, _ => x.powf(y) });
                }
                "sqrt" | "sin" | "cos" | "tan" | "neg" => {
                    let Some(x) = stack.pop() else { ctx.log("Stack underflow."); return Step::Done };
                    stack.push(match tok { "sqrt" => x.sqrt(), "sin" => x.to_radians().sin(), "cos" => x.to_radians().cos(), "tan" => x.to_radians().tan(), _ => -x });
                }
                n => match n.parse::<f64>() {
                    Ok(v) => stack.push(v),
                    Err(_) => {
                        ctx.log(format!("Unknown token {n}"));
                        return Step::Done;
                    }
                },
            }
        }
        match stack.last() {
            Some(v) => ctx.log(format!("= {}", fmt_num(*v))),
            None => ctx.log("Empty expression."),
        }
        Step::Done
    })
}

pub fn dup_face_border() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select face", ObjFilter::Surface)], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(faces) = faces_of(ctx.doc, *id) else { return Step::Cancelled };
        let f = &faces[face_under(&faces, *p)];
        let tol = f.bounding_box().diagonal().max(1e-6) * 1e-3;
        let curves: Vec<Geometry> = f.edge_curves(tol).into_iter().map(Geometry::Curve).collect();
        add_all(ctx, "DupFaceBorder", curves)
    })
}

pub fn join_copy() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select curves to copy and join", ObjFilter::Curve, 2)], |a, ctx| {
        let curves: Vec<NurbsCurve> = a[0].ids().iter().filter_map(|id| ctx.doc.get(*id)).filter_map(|o| o.geometry.as_curve().cloned()).collect();
        if curves.is_empty() {
            return Step::Cancelled;
        }
        let mut joined = curves[0].clone();
        let mut rest: Vec<NurbsCurve> = curves[1..].to_vec();
        let mut progress = true;
        while progress && !rest.is_empty() {
            progress = false;
            for i in 0..rest.len() {
                let mut r = rest[i].clone();
                let mut rr = r.clone();
                rr.reverse();
                if let Ok(j) = joined.join_any(&r).or_else(|_| joined.join_any(&rr)).or_else(|_| r.join_any(&joined)).or_else(|_| { r = rr.clone(); rr.join_any(&joined) }) {
                    joined = j;
                    rest.remove(i);
                    progress = true;
                    break;
                }
            }
        }
        let mut out = vec![Geometry::Curve(joined)];
        out.extend(rest.into_iter().map(Geometry::Curve));
        add_all(ctx, "JoinCopy", out)
    })
}

pub fn point_cloud() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select point objects", ObjFilter::Point, 1)], |a, ctx| {
        let n = ctx.memory.get("GroupCounter").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0) + 1;
        ctx.mem_set("GroupCounter", n);
        let name = format!("PointCloud{n:02}");
        ctx.doc.begin_undo("PointCloud");
        for id in a[0].ids() {
            if let Some(o) = ctx.doc.get_mut(id) {
                o.attributes.groups.retain(|g| !g.starts_with("PointCloud"));
                o.attributes.groups.push(name.clone());
            }
        }
        ctx.log(format!("Point cloud {name} created (points grouped)."));
        Step::Done
    })
}

pub fn reduce_point_cloud() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select point cloud points", ObjFilter::Point, 1), Ask::Number("Percentage of points to remove", 50.0)], |a, ctx| {
        let pct = a[1].number().clamp(0.0, 100.0) / 100.0;
        let ids = a[0].ids();
        ctx.doc.begin_undo("ReducePointCloud");
        let mut removed = 0;
        for (i, id) in ids.iter().enumerate() {
            // deterministic pseudo-random thinning
            let h = ((i as u64).wrapping_mul(2654435761) % 1000) as f64 / 1000.0;
            if h < pct {
                ctx.doc.remove(*id);
                removed += 1;
            }
        }
        ctx.log(format!("{removed} points removed."));
        Step::Done
    })
}

pub fn split_face() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select face to split", ObjFilter::Surface), Ask::Point("Start of split line"), Ask::Point("End of split line")], |a, ctx| {
        let Ans::Pick(id, _) = &a[0] else { return Step::Cancelled };
        let (p, q) = (a[1].point(), a[2].point());
        let d = (q - p).normalize_or(DVec3::X) * (q - p).length() * 10.0;
        let Ok(line) = NurbsCurve::line(p - d, q + d) else { return Step::Cancelled };
        ctx.doc.begin_undo("SplitFace");
        let n = split_faces(ctx, *id, &[line]);
        if n == 0 {
            ctx.log("The line does not split the face.");
        }
        Step::Done
    })
}

pub fn zoom_special(kind: &'static str) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects", ObjFilter::Any, 1)], move |a, ctx| {
        let mut bb = tri_geom::BoundingBox::EMPTY;
        for id in a[0].ids() {
            let Some(o) = ctx.doc.get(id) else { continue };
            match (kind, &o.geometry) {
                ("ZoomEnds", Geometry::Curve(c)) => {
                    bb = bb.union(&tri_geom::BoundingBox::from_points(&[c.start_point(), c.end_point()]));
                }
                ("ZoomNaked", Geometry::Mesh(m)) => {
                    for l in mesh_ops::naked_loops(m) {
                        bb = bb.union(&tri_geom::BoundingBox::from_points(&l));
                    }
                }
                ("ZoomNaked", Geometry::Brep(b)) => {
                    for (_, l) in b.naked_planar_loops() {
                        bb = bb.union(&tri_geom::BoundingBox::from_points(&l));
                    }
                }
                _ => bb = bb.union(&ctx.doc.object_bbox(o)),
            }
        }
        if bb.is_empty() {
            ctx.log("Nothing to zoom to.");
            return Step::Done;
        }
        let pad = bb.diagonal().max(1.0) * 0.1;
        ctx.requests.push(AppRequest::ZoomWindowPoints(bb.min - DVec3::splat(pad), bb.max + DVec3::splat(pad)));
        Step::Done
    })
}

#[allow(dead_code)]
fn unused(_: ObjectId) {}
