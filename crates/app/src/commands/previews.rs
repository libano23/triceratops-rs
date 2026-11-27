//! Ghosts for sequential commands, made by running the command itself.
//!
//! The convention (taken from Serpentine3D): a question the mouse can answer shows what the
//! click would make while the cursor moves. A `SeqCmd` already holds everything needed: its
//! `finish` closure turns the answers into geometry. A dry run feeds it the cursor as the pending
//! answer, on a scratch document holding copies of the objects the answers name, and shows
//! whatever it added or changed.

use std::time::{Duration, Instant};

use glam::DVec3;
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{BoundingBox, Plane};

use super::simple::{Ans, Ask};
use super::{CmdCtx, Preview, Step};

/// The longest a dry run may take before the command stops being previewed (the window must not
/// stutter), and how long a worker job is waited for.
const BUDGET: Duration = Duration::from_millis(150);

/// The last ghost made for a command: the cursor, how many answers it had, when, and the ghost.
pub type LastGhost = Option<(DVec3, usize, Instant, Vec<Preview>)>;

/// What a dry run or a trial needs from the context it cannot ask for: the construction plane, the
/// view and the values the session remembers (the last angle, option states).
#[derive(Clone)]
pub struct Ambient {
    pub cplane: Plane,
    pub view_dir: DVec3,
    pub view_eye: DVec3,
    pub memory: std::collections::HashMap<String, String>,
}

impl Ambient {
    pub fn of(ctx: &CmdCtx) -> Ambient {
        Ambient { cplane: ctx.cplane, view_dir: ctx.view_dir, view_eye: ctx.view_eye, memory: ctx.memory.clone() }
    }
}

/// What a distance, radius, factor or angle is measured from.
pub enum Reference {
    /// A point the user gave: the last point or pick among the answers.
    Point(DVec3),
    /// Objects the user selected: the box around them.
    Box(BoundingBox),
    None,
}

impl Reference {
    pub fn of(answers: &[Ans], doc: &Document) -> Reference {
        if let Some(p) = answers.iter().rev().find_map(|a| match a {
            Ans::Point(p) | Ans::Pick(_, p) => Some(*p),
            _ => None,
        }) {
            // a pick names an object too: measure from the object, not from the spot clicked
            if let Some(Ans::Pick(id, _)) = answers.iter().rev().find(|a| matches!(a, Ans::Pick(..) | Ans::Point(_))) {
                if let Some(o) = doc.get(*id) {
                    return Reference::Box(o.geometry.bounding_box());
                }
            }
            return Reference::Point(p);
        }
        let mut bb = BoundingBox::EMPTY;
        for id in object_ids(answers) {
            if let Some(o) = doc.get(id) {
                bb = bb.union(&o.geometry.bounding_box());
            }
        }
        if bb.min.x <= bb.max.x {
            Reference::Box(bb)
        } else {
            Reference::None
        }
    }

    fn centre(&self) -> DVec3 {
        match self {
            Reference::Point(p) => *p,
            Reference::Box(b) => b.center(),
            Reference::None => DVec3::ZERO,
        }
    }

    /// The distance from `p` to the reference: to the point, or to the surface of the box (0
    /// inside it, where the distance to the centre is used instead so the cursor still answers).
    fn distance(&self, p: DVec3) -> f64 {
        match self {
            Reference::Point(q) => (p - *q).length(),
            Reference::Box(b) => {
                let d = (b.min - p).max(p - b.max).max(DVec3::ZERO).length();
                if d > 0.0 {
                    d
                } else {
                    (p - b.center()).length()
                }
            }
            Reference::None => p.length(),
        }
    }

    /// Half the size of what is referenced: the unit a factor is measured in.
    fn unit(&self) -> f64 {
        match self {
            Reference::Box(b) => (b.diagonal() * 0.5).max(1e-9),
            _ => 1.0,
        }
    }
}

pub fn object_ids(answers: &[Ans]) -> Vec<ObjectId> {
    answers.iter().flat_map(|a| a.ids()).collect()
}

/// The number a point answers for a question the mouse measures.
pub fn number_from_point(ask: &Ask, answers: &[Ans], doc: &Document, p: DVec3, cplane: &Plane) -> Option<f64> {
    let r = Reference::of(answers, doc);
    Some(match ask {
        Ask::Distance(..) => r.distance(p),
        Ask::Radius(..) => (p - r.centre()).length(),
        Ask::Factor(..) => (p - r.centre()).length() / r.unit(),
        Ask::Angle(..) => {
            let d = p - r.centre();
            d.dot(cplane.y_axis).atan2(d.dot(cplane.x_axis)).to_degrees()
        }
        _ => return None,
    })
}

/// The answer the cursor would give to `ask`, or `None` when the mouse cannot answer it.
pub fn candidate(ask: &Ask, answers: &[Ans], doc: &Document, cursor: DVec3, cplane: &Plane) -> Option<Ans> {
    match ask {
        Ask::Point(_) | Ask::PointOrNumber(..) => Some(Ans::Point(cursor)),
        Ask::Distance(..) | Ask::Radius(..) | Ask::Factor(..) | Ask::Angle(..) => number_from_point(ask, answers, doc, cursor, cplane).map(Ans::Number),
        _ => None,
    }
}

/// What a question answered by pressing Enter gives.
pub fn default_answer(ask: &Ask) -> Option<Ans> {
    Some(match ask {
        Ask::Number(_, d) | Ask::PointOrNumber(_, d) | Ask::Distance(_, d) | Ask::Radius(_, d) | Ask::Factor(_, d) | Ask::Angle(_, d) => Ans::Number(*d),
        Ask::Text(_) => Ans::Text(String::new()),
        Ask::Options(..) => Ans::Option(String::new()),
        Ask::Objects(..) | Ask::Pick(..) | Ask::Point(_) => return None,
    })
}

/// A scratch document with copies of the objects `ids` name and of the selection.
fn scratch_of(doc: &Document, ids: &[ObjectId]) -> Document {
    let mut state = tri_doc::State::default();
    for id in ids.iter().copied() {
        if let Some(o) = doc.get(id) {
            state.objects.insert(id, o.clone());
        }
    }
    // the selection the command may read
    for id in doc.selected_ids() {
        if let Some(o) = doc.get(id) {
            state.objects.entry(id).or_insert_with(|| o.clone());
        }
    }
    state.layers = doc.state.layers.clone();
    let mut scratch = Document::new();
    scratch.load_state(state, doc.settings.clone(), None);
    for id in doc.selected_ids() {
        scratch.select(id, true);
    }
    scratch
}

/// The ghost of an object: its geometry as previews.
pub fn geometry_previews(g: &Geometry) -> Vec<Preview> {
    match g {
        Geometry::Curve(c) => vec![Preview::Curve(c.clone())],
        Geometry::Surface(s) => vec![Preview::Surface(s.clone())],
        Geometry::Brep(b) => vec![Preview::Brep(b.clone())],
        Geometry::Mesh(m) => vec![Preview::Mesh(m.clone())],
        Geometry::Point(p) => vec![Preview::Point(*p)],
        Geometry::Instance { .. } => Vec::new(),
    }
}

/// Run `step` on a scratch copy of the objects `ids` (and the selection) and return what it added
/// or changed as previews. `None` when it does not finish within the budget or shows nothing.
pub fn in_scratch(doc: &Document, ids: &[ObjectId], ambient: &Ambient, step: impl FnOnce(&mut CmdCtx) -> Step) -> Option<Vec<Preview>> {
    let started = Instant::now();
    let mut scratch = scratch_of(doc, ids);
    let before: std::collections::HashMap<ObjectId, Geometry> = scratch.objects().map(|o| (o.id, o.geometry.clone())).collect();
    let (mut log, mut requests, mut memory) = (Vec::new(), Vec::new(), ambient.memory.clone());
    let mut ctx = CmdCtx { doc: &mut scratch, log: &mut log, cplane: ambient.cplane, view_dir: ambient.view_dir, view_eye: ambient.view_eye, requests: &mut requests, memory: &mut memory };
    // a command run half-way with guessed answers may panic: a preview must never take the app down
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| step(&mut ctx))).ok()?;
    if let Step::Work(job) = result {
        // a worker job (a Boolean, a long mesh operation): wait a little, then give up on it
        let super::Job { token, handle, apply, .. } = job;
        while !handle.is_finished() {
            if started.elapsed() > BUDGET {
                token.cancel();
                return None;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let result = handle.join().ok()?;
        apply(result, &mut ctx);
    }
    if started.elapsed() > BUDGET * 2 {
        return None;
    }
    let mut out = Vec::new();
    for o in scratch.objects() {
        if before.get(&o.id) != Some(&o.geometry) {
            out.extend(geometry_previews(&o.geometry));
        }
    }
    (!out.is_empty()).then_some(out)
}

/// A sequential command's `finish` run with `answers`.
pub fn dry_run(finish: &dyn Fn(&[Ans], &mut CmdCtx) -> Step, answers: &[Ans], doc: &Document, ambient: &Ambient) -> Option<Vec<Preview>> {
    in_scratch(doc, &object_ids(answers), ambient, |ctx| finish(answers, ctx))
}

/// A command with its own state: a copy of it fed the cursor (as the number or point the pending
/// question means), on a scratch document.
pub fn trial_run(cmd: &dyn super::Command, cursor: DVec3, doc: &Document, ambient: &Ambient) -> Option<Vec<Preview>> {
    let mut copy = cmd.trial()?;
    let input = match cmd.point_as_number(cursor, doc, ambient) {
        Some(n) => super::Input::Number(n),
        None => super::Input::Point(cursor),
    };
    in_scratch(doc, &cmd.trial_objects(), ambient, move |ctx| copy.input(input, ctx))
}

/// The cost of a dry run that took `took`: past the budget the command is no longer previewed.
pub fn too_slow(took: Duration) -> bool {
    took > BUDGET
}

/// The distance from `p` to the nearest of the objects: to a curve along the curve, to anything
/// else to its box (0 inside it).
pub fn distance_to_objects(ids: &[ObjectId], doc: &Document, p: DVec3) -> f64 {
    ids.iter()
        .filter_map(|id| doc.get(*id))
        .map(|o| match &o.geometry {
            Geometry::Curve(c) => (c.point_at(c.closest_parameter(p)) - p).length(),
            g => Reference::Box(g.bounding_box()).distance(p),
        })
        .fold(f64::INFINITY, f64::min)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ambient() -> Ambient {
        Ambient { cplane: Plane::world_xy(), view_dir: -DVec3::Z, view_eye: DVec3::new(0.0, 0.0, 100.0), memory: Default::default() }
    }

    #[test]
    fn a_panicking_command_gives_no_ghost_instead_of_taking_the_app_down() {
        let doc = Document::new();
        let ghost = in_scratch(&doc, &[], &ambient(), |_ctx| panic!("a command that cannot run on guessed answers"));
        assert!(ghost.is_none());
    }

    #[test]
    fn what_a_command_adds_is_the_ghost_and_the_real_document_is_untouched() {
        let doc = Document::new();
        let ghost = in_scratch(&doc, &[], &ambient(), |ctx| {
            ctx.doc.add(Geometry::Point(DVec3::new(1.0, 2.0, 3.0)));
            Step::Done
        })
        .expect("something was added");
        assert!(matches!(ghost.as_slice(), [Preview::Point(p)] if *p == DVec3::new(1.0, 2.0, 3.0)));
        assert_eq!(doc.len(), 0);
    }

    #[test]
    fn a_command_that_changes_nothing_has_no_ghost() {
        let doc = Document::new();
        assert!(in_scratch(&doc, &[], &ambient(), |_ctx| Step::Done).is_none());
    }
}
