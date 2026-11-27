//! Construction history, cages (free-form deformation) and hole editing.

use glam::{DAffine3, DQuat, DVec3};
use tri_doc::history::CageRecord;
use tri_doc::{Geometry, ObjectId};

use super::misc4::{add_hole_curve, hole_of_curve, recut_holes, register_hole};
use super::simple::{select_where, Ans, Ask, SeqCmd};
use super::{fmt_num, opt, optv, AppRequest, CmdCtx, Command, Input, ObjFilter, Prompt, Step};

pub struct HistoryCmd;

impl Command for HistoryCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        let n = ctx.doc.state.history.records.len();
        let rec = ctx.memory.get("RecordHistory").is_some_and(|v| v == "Yes");
        ctx.log(format!("{n} objects with history in the document."));
        Prompt::options("History").opts([optv("Record", if rec { "Yes" } else { "No" }), opt("Update"), opt("List")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if o == "Record" => {
                let rec = ctx.memory.get("RecordHistory").is_some_and(|v| v == "Yes");
                ctx.requests.push(AppRequest::SetOption { key: "record_history".into(), value: if rec { "Off".into() } else { "On".into() } });
                Step::Done
            }
            Input::Option(o) if o == "Update" => {
                ctx.requests.push(AppRequest::HistoryUpdate(Vec::new()));
                Step::Done
            }
            Input::Option(_) => {
                let rows: Vec<String> = ctx.doc.state.history.records.iter().map(|r| format!("  [{}] {}: parents {:?} -> children {:?}", r.id, r.command, r.parents(), r.outputs)).collect();
                for r in rows {
                    ctx.log(r);
                }
                Step::Done
            }
            _ => Step::Done,
        }
    }
}

pub fn history_update() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to update. Press Enter for all", ObjFilter::Any, 0)], |a, ctx| {
        ctx.requests.push(AppRequest::HistoryUpdate(a[0].ids()));
        Step::Done
    })
}

pub fn history_purge() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to purge history from", ObjFilter::Any, 1)], |a, ctx| {
        let ids = a[0].ids();
        let before = ctx.doc.state.history.records.len();
        ctx.doc.state.history.records.retain(|r| !r.outputs.iter().any(|o| ids.contains(o)) && !r.parents().iter().any(|p| ids.contains(p)));
        ctx.log(format!("{} history records removed.", before - ctx.doc.state.history.records.len()));
        Step::Done
    })
}

pub fn sel_history(kind: &'static str) -> super::simple::FnCmd {
    match kind {
        "children" => super::simple::FnCmd(|ctx| {
            let sel = ctx.doc.selected_ids();
            let kids: Vec<ObjectId> = ctx.doc.state.history.records.iter().filter(|r| r.parents().iter().any(|p| sel.contains(p))).flat_map(|r| r.outputs.clone()).collect();
            select_where(ctx, |_, o| kids.contains(&o.id))
        }),
        "parents" => super::simple::FnCmd(|ctx| {
            let sel = ctx.doc.selected_ids();
            let parents: Vec<ObjectId> = ctx.doc.state.history.records.iter().filter(|r| r.outputs.iter().any(|p| sel.contains(p))).flat_map(|r| r.parents()).collect();
            select_where(ctx, |_, o| parents.contains(&o.id))
        }),
        _ => super::simple::FnCmd(|ctx| {
            let kids: Vec<ObjectId> = ctx.doc.state.history.records.iter().flat_map(|r| r.outputs.clone()).collect();
            select_where(ctx, |_, o| kids.contains(&o.id))
        }),
    }
}

// ---- cages ---------------------------------------------------------------------------------------

fn make_cage(ctx: &mut CmdCtx, min: DVec3, max: DVec3, dims: (usize, usize, usize)) -> (String, Vec<ObjectId>) {
    let n = ctx.memory.get("GroupCounter").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0) + 1;
    ctx.mem_set("GroupCounter", n);
    let name = format!("Cage{n:02}");
    let mut ids = Vec::new();
    let size = max - min;
    for i in 0..dims.0 {
        for j in 0..dims.1 {
            for k in 0..dims.2 {
                let p = DVec3::new(
                    min.x + size.x * i as f64 / (dims.0 - 1).max(1) as f64,
                    min.y + size.y * j as f64 / (dims.1 - 1).max(1) as f64,
                    min.z + size.z * k as f64 / (dims.2 - 1).max(1) as f64,
                );
                let mut a = tri_doc::Attributes { layer: ctx.doc.current_layer(), name: Some(name.clone()), ..Default::default() };
                a.groups.push(name.clone());
                a.user_text.push(("CageIndex".into(), format!("{i},{j},{k}")));
                a.user_text.push(("CageDims".into(), format!("{},{},{}", dims.0, dims.1, dims.2)));
                ids.push(ctx.doc.add_with(Geometry::Point(p), a));
            }
        }
    }
    (name, ids)
}

pub fn cage_cmd() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("First corner of cage"), Ask::Point("Other corner"), Ask::PointOrNumber("Height", 10.0), Ask::Number("Points in X", 4.0), Ask::Number("Points in Y", 4.0), Ask::Number("Points in Z", 4.0)], |a, ctx| {
        let (p0, p1) = (a[0].point(), a[1].point());
        let h = match &a[2] { Ans::Number(n) => *n, Ans::Point(p) => (*p - p1).dot(ctx.cplane.z_axis), _ => 10.0 };
        let min = p0.min(p1);
        let max = p0.max(p1) + ctx.cplane.z_axis * h;
        let dims = (a[3].number().round().clamp(2.0, 10.0) as usize, a[4].number().round().clamp(2.0, 10.0) as usize, a[5].number().round().clamp(2.0, 10.0) as usize);
        ctx.doc.begin_undo("Cage");
        ctx.doc.clear_selection();
        let (name, ids) = make_cage(ctx, min.min(max), min.max(max), dims);
        for id in ids {
            ctx.doc.select(id, true);
        }
        ctx.log(format!("Cage {name} created; use CageEdit to capture objects, then move its points (PointsOn is not needed: the points are objects)."));
        Step::Done
    })
}

/// Cage points of the group containing `pick`, ordered by index, with the dimensions.
fn cage_points(ctx: &CmdCtx, pick: ObjectId) -> Option<(Vec<ObjectId>, (usize, usize, usize))> {
    let o = ctx.doc.get(pick)?;
    let group = o.attributes.groups.iter().find(|g| g.starts_with("Cage"))?.clone();
    let dims = o.attributes.user_text.iter().find(|(k, _)| k == "CageDims").map(|(_, v)| { let n: Vec<usize> = v.split(',').filter_map(|x| x.parse().ok()).collect(); (n[0], n[1], n[2]) })?;
    let mut pts: Vec<(usize, ObjectId)> = ctx.doc.objects().filter(|ob| ob.attributes.groups.contains(&group)).filter_map(|ob| {
        let idx = ob.attributes.user_text.iter().find(|(k, _)| k == "CageIndex").map(|(_, v)| { let n: Vec<usize> = v.split(',').filter_map(|x| x.parse().ok()).collect(); (n[0] * dims.1 + n[1]) * dims.2 + n[2] })?;
        Some((idx, ob.id))
    }).collect();
    pts.sort();
    if pts.len() != dims.0 * dims.1 * dims.2 {
        return None;
    }
    Some((pts.into_iter().map(|(_, id)| id).collect(), dims))
}

#[derive(Default)]
pub struct CageEditCmd {
    captives: Vec<ObjectId>,
}


impl Command for CageEditCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select captive objects", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) if self.captives.is_empty() => {
                self.captives = ids;
                ctx.doc.clear_selection();
                Prompt::pick("Select control object (a cage point). Press Enter for a bounding box cage", ObjFilter::Point).opts([opt("BoundingBox")]).step()
            }
            Input::Pick(id, _) => {
                let Some((points, dims)) = cage_points(ctx, id) else {
                    ctx.log("The picked point is not part of a cage (use the Cage command).");
                    return Step::Cancelled;
                };
                self.capture(ctx, points, dims)
            }
            Input::Option(_) | Input::Empty => {
                let bb = self.captives.iter().filter_map(|id| ctx.doc.get(*id)).fold(tri_geom::BoundingBox::EMPTY, |b, o| b.union(&ctx.doc.object_bbox(o)));
                if bb.is_empty() {
                    return Step::Cancelled;
                }
                let pad = bb.diagonal() * 0.05;
                ctx.doc.begin_undo("CageEdit");
                let (_, points) = make_cage(ctx, bb.min - DVec3::splat(pad), bb.max + DVec3::splat(pad), (4, 4, 4));
                self.capture(ctx, points, (4, 4, 4))
            }
            _ => Step::Cancelled,
        }
    }
}

impl CageEditCmd {
    fn capture(&self, ctx: &mut CmdCtx, points: Vec<ObjectId>, dims: (usize, usize, usize)) -> Step {
        let pos: Vec<DVec3> = points.iter().filter_map(|id| ctx.doc.get(*id)).filter_map(|o| if let Geometry::Point(p) = o.geometry { Some(p) } else { None }).collect();
        if pos.len() != points.len() {
            return Step::Cancelled;
        }
        let min = pos.iter().fold(DVec3::splat(f64::INFINITY), |a, p| a.min(*p));
        let max = pos.iter().fold(DVec3::splat(f64::NEG_INFINITY), |a, p| a.max(*p));
        let captives: Vec<(ObjectId, Geometry)> = self.captives.iter().filter(|id| !points.contains(id)).filter_map(|id| ctx.doc.get(*id).map(|o| (*id, o.geometry.clone()))).collect();
        let id = ctx.doc.state.history.next_id();
        ctx.doc.state.history.cages.push(CageRecord { id, points: points.clone(), dims, origin_points: pos, min, max, captives });
        ctx.doc.clear_selection();
        for p in points {
            ctx.doc.select(p, true);
        }
        ctx.log("Captives recorded: move the cage points (drag, Move, gumball) to deform them. ReleaseFromCage ends the link.");
        Step::Done
    }
}

pub fn release_from_cage() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select captive objects to release", ObjFilter::Any, 1)], |a, ctx| {
        let ids = a[0].ids();
        for c in ctx.doc.state.history.cages.iter_mut() {
            c.captives.retain(|(id, _)| !ids.contains(id));
        }
        ctx.doc.state.history.cages.retain(|c| !c.captives.is_empty());
        Step::Done
    })
}

pub fn extract_original_captives() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select captive objects", ObjFilter::Any, 1)], |a, ctx| {
        let ids = a[0].ids();
        let originals: Vec<Geometry> = ctx.doc.state.history.cages.iter().flat_map(|c| c.captives.iter().filter(|(id, _)| ids.contains(id)).map(|(_, g)| g.clone())).collect();
        if originals.is_empty() {
            ctx.log("The objects are not cage captives.");
            return Step::Done;
        }
        super::simple::add_all(ctx, "ExtractOriginalCaptives", originals)
    })
}

pub fn sel_cage(kind: &'static str) -> super::simple::FnCmd {
    match kind {
        "captives" => super::simple::FnCmd(|ctx| {
            let ids: Vec<ObjectId> = ctx.doc.state.history.cages.iter().flat_map(|c| c.captives.iter().map(|(id, _)| *id)).collect();
            select_where(ctx, |_, o| ids.contains(&o.id))
        }),
        _ => super::simple::FnCmd(|ctx| select_where(ctx, |_, o| o.attributes.user_text.iter().any(|(k, _)| k == "CageIndex"))),
    }
}

// ---- holes --------------------------------------------------------------------------------------------------

fn hole_xf(label: &'static str, asks: Vec<Ask>, copy: bool, f: fn(&[Ans], &CmdCtx) -> Vec<DAffine3>) -> SeqCmd {
    let mut all = vec![Ask::Pick("Select hole (its definition curve)", ObjFilter::Curve)];
    all.extend(asks);
    SeqCmd::new(all, move |a, ctx| {
        let cid = a[0].ids()[0];
        let Some(target) = hole_of_curve(ctx, cid) else {
            ctx.log("The curve is not a hole definition (use MakeHole or RoundHole first).");
            return Step::Cancelled;
        };
        let Some(c) = ctx.doc.get(cid).and_then(|o| o.geometry.as_curve().cloned()) else { return Step::Cancelled };
        let xfs = f(a, ctx);
        ctx.doc.begin_undo(label);
        if copy {
            let mut new_ids = Vec::new();
            for xf in &xfs {
                new_ids.push(add_hole_curve(ctx, c.transformed(xf)));
            }
            register_hole(ctx, target, new_ids);
        } else if let Some(xf) = xfs.first() {
            ctx.doc.replace_geometry(cid, Geometry::Curve(c.transformed(xf)));
        }
        recut_holes(ctx, target, label)
    })
}

pub fn copy_hole() -> SeqCmd {
    hole_xf("CopyHole", vec![Ask::Point("Point to copy from"), Ask::Point("Point to copy to")], true, |a, _| vec![DAffine3::from_translation(a[2].point() - a[1].point())])
}

pub fn move_hole() -> SeqCmd {
    hole_xf("MoveHole", vec![Ask::Point("Point to move from"), Ask::Point("Point to move to")], false, |a, _| vec![DAffine3::from_translation(a[2].point() - a[1].point())])
}

pub fn rotate_hole() -> SeqCmd {
    hole_xf("RotateHole", vec![Ask::Point("Center of rotation"), Ask::Angle("Angle", 90.0)], false, |a, ctx| {
        let c = a[1].point();
        vec![DAffine3::from_translation(c) * DAffine3::from_quat(DQuat::from_axis_angle(ctx.cplane.z_axis, a[2].number().to_radians())) * DAffine3::from_translation(-c)]
    })
}

pub fn mirror_hole() -> SeqCmd {
    hole_xf("MirrorHole", vec![Ask::Point("Start of mirror plane"), Ask::Point("End of mirror plane")], true, |a, ctx| {
        let (p, q) = (a[1].point(), a[2].point());
        let n = ctx.cplane.z_axis.cross(q - p).normalize_or(ctx.cplane.y_axis);
        let m = glam::DMat3::IDENTITY - glam::DMat3::from_cols(n * (2.0 * n.x), n * (2.0 * n.y), n * (2.0 * n.z));
        vec![DAffine3::from_translation(p) * DAffine3::from_mat3(m) * DAffine3::from_translation(-p)]
    })
}

pub fn array_hole() -> SeqCmd {
    hole_xf("ArrayHole", vec![Ask::Number("Number in X", 3.0), Ask::Number("Number in Y", 1.0), Ask::Distance("X spacing", 5.0), Ask::Distance("Y spacing", 5.0)], true, |a, ctx| {
        let (nx, ny) = (a[1].number().round().max(1.0) as usize, a[2].number().round().max(1.0) as usize);
        let (dx, dy) = (a[3].number(), a[4].number());
        let mut out = Vec::new();
        for i in 0..nx {
            for j in 0..ny {
                if i == 0 && j == 0 {
                    continue;
                }
                out.push(DAffine3::from_translation(ctx.cplane.x_axis * (dx * i as f64) + ctx.cplane.y_axis * (dy * j as f64)));
            }
        }
        out
    })
}

pub fn array_hole_polar() -> SeqCmd {
    hole_xf("ArrayHolePolar", vec![Ask::Point("Center of polar array"), Ask::Number("Number of items", 4.0), Ask::Angle("Angle to fill", 360.0)], true, |a, ctx| {
        let c = a[1].point();
        let n = a[2].number().round().max(1.0) as usize;
        let total = a[3].number().to_radians();
        let step = if (total - std::f64::consts::TAU).abs() < 1e-9 { total / n as f64 } else { total / (n.max(2) - 1) as f64 };
        (1..n).map(|i| DAffine3::from_translation(c) * DAffine3::from_quat(DQuat::from_axis_angle(ctx.cplane.z_axis, step * i as f64)) * DAffine3::from_translation(-c)).collect()
    })
}

pub fn place_hole() -> super::misc4::MakeHoleCmd {
    super::misc4::MakeHoleCmd::default()
}

#[allow(dead_code)]
fn unused() -> String {
    fmt_num(0.0)
}
