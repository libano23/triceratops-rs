//! One way to ask "along which direction?", for every command that offers it.
//!
//! A command that cuts, extrudes, projects or mirrors along an axis used to offer its own short
//! list. This is the shared list and the questions behind it: the world axes, the axes of the
//! construction plane, the view, two points, the tangent of a curve and the normal of a surface.
//! A command owns an [`AxisAsk`], opens it with [`AxisAsk::begin`] when its `Direction` option is
//! chosen and routes inputs to it while [`AxisAsk::active`].

use glam::DVec3;
use tri_doc::Geometry;

use super::{opt, CmdCtx, Input, ObjFilter, Prompt, Step};

/// The choices every direction question offers, in the order they are listed.
pub const CHOICES: [&str; 10] = ["CPlaneNormal", "View", "WorldX", "WorldY", "WorldZ", "CPlaneX", "CPlaneY", "TwoPoints", "CurveTangent", "SurfaceNormal"];

/// The direction a named choice stands for, when no further answer is needed.
pub fn named(choice: &str, ctx: &CmdCtx) -> Option<DVec3> {
    let d = match choice {
        "CPlaneNormal" => ctx.cplane.z_axis,
        "View" => ctx.view_dir,
        "WorldX" => DVec3::X,
        "WorldY" => DVec3::Y,
        "WorldZ" => DVec3::Z,
        "CPlaneX" => ctx.cplane.x_axis,
        "CPlaneY" => ctx.cplane.y_axis,
        _ => return None,
    };
    d.try_normalize()
}

/// What a direction question is waiting for.
#[derive(Clone, Copy, Default, PartialEq)]
enum Stage {
    #[default]
    Idle,
    Choose,
    From,
    To(DVec3),
    Curve,
    Surface,
}

/// What asking left to do.
pub enum Asked {
    /// Ask this next.
    Prompt(Prompt),
    /// The direction is chosen.
    Chosen(DVec3),
    /// The question was abandoned or could not be answered.
    Cancelled,
}

#[derive(Default, Clone)]
pub struct AxisAsk {
    stage: Stage,
    /// The choice last made, by name.
    last: &'static str,
    /// The first of the two points of a `TwoPoints` answer: where the direction starts.
    anchor: Option<DVec3>,
}

impl AxisAsk {
    /// Whether a direction question is open: inputs go to [`AxisAsk::input`].
    pub fn active(&self) -> bool {
        self.stage != Stage::Idle
    }

    /// Open the question: the list of choices.
    pub fn begin(&mut self) -> Prompt {
        self.stage = Stage::Choose;
        self.anchor = None;
        Prompt::options("Direction").opts(CHOICES.iter().map(|c| opt(c)))
    }

    /// Where the direction starts when it was given by two points (an axis through them).
    pub fn anchor(&self) -> Option<DVec3> {
        self.anchor
    }

    /// The name of the choice last made (`TwoPoints`, `WorldX`...).
    pub fn last_name(&self) -> &'static str {
        self.last
    }

    /// Feed an answer to the open question.
    pub fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Asked {
        match (self.stage, input) {
            (Stage::Choose, Input::Option(o)) => match o.as_str() {
                name if CHOICES.contains(&name) && !matches!(name, "TwoPoints" | "CurveTangent" | "SurfaceNormal") => {
                    self.last = CHOICES.iter().find(|c| **c == name).copied().unwrap_or("");
                    self.finish(named(name, ctx))
                }
                "TwoPoints" => {
                    self.last = "TwoPoints";
                    self.stage = Stage::From;
                    Asked::Prompt(Prompt::point("Start of direction"))
                }
                "CurveTangent" => {
                    self.last = "CurveTangent";
                    self.stage = Stage::Curve;
                    Asked::Prompt(Prompt::pick("Select a curve: its tangent at the picked point is the direction", ObjFilter::Curve))
                }
                "SurfaceNormal" => {
                    self.last = "SurfaceNormal";
                    self.stage = Stage::Surface;
                    Asked::Prompt(Prompt::pick("Select a surface or solid: its normal at the picked point is the direction", ObjFilter::Any))
                }
                _ => {
                    self.stage = Stage::Idle;
                    Asked::Cancelled
                }
            },
            (Stage::From, Input::Point(p)) => {
                self.anchor = Some(p);
                self.stage = Stage::To(p);
                Asked::Prompt(Prompt::point("End of direction"))
            }
            (Stage::To(a), Input::Point(b)) => self.finish((b - a).try_normalize()),
            (Stage::Curve, Input::Pick(id, at)) => {
                let d = ctx.doc.get(id).and_then(|o| match &o.geometry {
                    Geometry::Curve(c) => Some(c.tangent_at(c.closest_parameter(at))),
                    _ => None,
                });
                self.finish(d.and_then(|d| d.try_normalize()))
            }
            (Stage::Surface, Input::Pick(id, at)) => {
                let d = super::surface_edit::faces_of(ctx.doc, id).filter(|f| !f.is_empty()).map(|faces| super::misc6::pull_point(&faces, at).1);
                self.finish(d.and_then(|d| d.try_normalize()))
            }
            _ => {
                self.stage = Stage::Idle;
                Asked::Cancelled
            }
        }
    }

    fn finish(&mut self, d: Option<DVec3>) -> Asked {
        self.stage = Stage::Idle;
        match d {
            Some(d) => Asked::Chosen(d),
            None => Asked::Cancelled,
        }
    }
}

/// The step a command returns when the question ended without an answer.
pub fn abandoned(ctx: &mut CmdCtx) -> Step {
    ctx.log("The direction could not be taken from that.");
    Step::Cancelled
}
