//! Parametric sketch commands (CADCraft / AutoCAD naming): Parametric, GeomConstraint and the
//! Gc* shortcuts, DimConstraint and the Dc* shortcuts, AutoConstrain, DelConstraint,
//! Parameters, ConstraintBar, ConstraintSettings.

use glam::DVec3;
use tri_doc::ObjectId;

use super::simple::{Ans, Ask, SeqCmd};
use super::{fmt_num, opt, optv, CmdCtx, Command, Input, ObjFilter, Prompt, Step};

fn report(ctx: &mut CmdCtx, r: Result<tri_doc::parametric::SolveOutcome, String>) {
    match r {
        Ok(o) => {
            for m in o.messages {
                ctx.log(m);
            }
            if !o.moved.is_empty() {
                ctx.log(format!("{} objects updated by the constraint solver.", o.moved.len()));
            }
        }
        Err(e) => ctx.log(format!("Constraint solve failed: {e}")),
    }
}

/// Parametric mode on/off and the sketch plane.
pub struct ParametricCmd;

impl Command for ParametricCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        let on = ctx.doc.state.parametric.enabled;
        Prompt::options(if on { "Parametric mode is on" } else { "Parametric mode is off" }).opts([opt("On"), opt("Off"), opt("Toggle"), optv("Plane", "CPlane"), optv("Bars", if ctx.doc.state.parametric.show_bars { "Yes" } else { "No" }), opt("Solve")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Option(o) = input else { return Step::Done };
        match o.as_str() {
            "On" => ctx.doc.state.parametric.enabled = true,
            "Off" => ctx.doc.state.parametric.enabled = false,
            "Toggle" => ctx.doc.state.parametric.enabled = !ctx.doc.state.parametric.enabled,
            "Plane" => {
                ctx.doc.state.parametric.plane = ctx.cplane;
                ctx.log("Sketch plane set to the construction plane.");
            }
            "Bars" => ctx.doc.state.parametric.show_bars = !ctx.doc.state.parametric.show_bars,
            "Solve" => {
                let r = ctx.doc.solve_constraints(&[]);
                report(ctx, r);
            }
            _ => {}
        }
        ctx.doc.touch_revision();
        ctx.log(if ctx.doc.state.parametric.enabled { "Parametric mode on: constraints are re-solved after every edit." } else { "Parametric mode off." });
        Step::Done
    }
}

/// Number of picks each geometric constraint needs and whether the picks are points.
fn geom_spec(kind: &str) -> (usize, bool, &'static str) {
    match kind {
        "Coincident" => (2, true, "Select point"),
        "Collinear" | "Parallel" | "Perpendicular" | "Tangent" | "Smooth" | "Equal" | "Concentric" => (2, false, "Select object"),
        "Symmetric" => (3, false, "Select object (third pick: symmetry line)"),
        _ => (1, false, "Select object"),
    }
}

pub struct GeomConstraintCmd {
    kind: Option<String>,
    picks: Vec<(ObjectId, DVec3)>,
}

impl GeomConstraintCmd {
    pub fn new(kind: Option<&str>) -> Self {
        Self { kind: kind.map(|s| s.to_string()), picks: Vec::new() }
    }
    fn kind_prompt() -> Prompt {
        Prompt::options("Constraint type").opts(["Coincident", "Collinear", "Concentric", "Fix", "Parallel", "Perpendicular", "Horizontal", "Vertical", "Tangent", "Smooth", "Symmetric", "Equal"].iter().map(|k| opt(k)))
    }
    fn pick_prompt(&self) -> Prompt {
        let (n, _, msg) = geom_spec(self.kind.as_deref().unwrap_or(""));
        Prompt::pick(format!("{msg} {} of {n}", self.picks.len() + 1), ObjFilter::Any)
    }
}

impl Command for GeomConstraintCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        if self.kind.is_none() {
            return Self::kind_prompt().step();
        }
        self.pick_prompt().step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if self.kind.is_none() => {
                self.kind = Some(o);
                self.pick_prompt().step()
            }
            Input::Pick(id, p) => {
                self.picks.push((id, p));
                let kind = self.kind.clone().unwrap_or_default();
                let (n, points, _) = geom_spec(&kind);
                if self.picks.len() < n {
                    return self.pick_prompt().step();
                }
                let refs: Vec<(ObjectId, String)> = self.picks.iter().enumerate().map(|(i, (id, p))| {
                    let want_point = points || (kind == "Symmetric" && i < 2 && false);
                    (*id, ctx.doc.sub_at(*id, *p, want_point))
                }).collect();
                ctx.doc.begin_undo("GeomConstraint");
                match ctx.doc.add_constraint(&kind, refs, "", "") {
                    Ok(id) => ctx.log(format!("{kind} constraint #{id} added.")),
                    Err(e) => ctx.log(format!("{kind}: {e}")),
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

pub struct DimConstraintCmd {
    kind: Option<String>,
    picks: Vec<(ObjectId, DVec3)>,
}

impl DimConstraintCmd {
    pub fn new(kind: Option<&str>) -> Self {
        Self { kind: kind.map(|s| s.to_string()), picks: Vec::new() }
    }
    fn needs(kind: &str) -> (usize, bool) {
        match kind {
            "Radius" | "Diameter" => (1, false),
            "Angular" => (2, false),
            _ => (2, true),
        }
    }
    fn pick_prompt(&self) -> Prompt {
        let kind = self.kind.as_deref().unwrap_or("Aligned");
        let (n, pts) = Self::needs(kind);
        Prompt::pick(format!("{} {} of {n}", if pts { "Select point" } else { "Select object" }, self.picks.len() + 1), ObjFilter::Any)
    }
}

impl Command for DimConstraintCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        if self.kind.is_none() {
            return Prompt::options("Dimensional constraint type").opts([opt("Aligned"), opt("HorizontalDistance"), opt("VerticalDistance"), opt("Angular"), opt("Radius"), opt("Diameter")]).step();
        }
        self.pick_prompt().step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if self.kind.is_none() => {
                self.kind = Some(o);
                self.pick_prompt().step()
            }
            Input::Pick(id, p) => {
                self.picks.push((id, p));
                let kind = self.kind.clone().unwrap_or_else(|| "Aligned".into());
                let (n, _) = Self::needs(&kind);
                if self.picks.len() < n {
                    return self.pick_prompt().step();
                }
                Prompt::text("Value or expression (Enter keeps the measured value)").step()
            }
            Input::Text(expr) | Input::Option(expr) if !self.picks.is_empty() => {
                let kind = self.kind.clone().unwrap_or_else(|| "Aligned".into());
                let (_, pts) = Self::needs(&kind);
                let refs: Vec<(ObjectId, String)> = self.picks.iter().map(|(id, p)| (*id, ctx.doc.sub_at(*id, *p, pts))).collect();
                ctx.doc.begin_undo("DimConstraint");
                match ctx.doc.add_constraint(&kind, refs, "", expr.trim()) {
                    Ok(id) => {
                        let name = ctx.doc.state.parametric.constraints.iter().find(|c| c.id == id).map(|c| format!("{}={}", c.name, c.expr)).unwrap_or_default();
                        ctx.log(format!("{kind} constraint #{id} added ({name})."));
                    }
                    Err(e) => ctx.log(format!("{kind}: {e}")),
                }
                Step::Done
            }
            Input::Empty if !self.picks.is_empty() => self.input(Input::Text(String::new()), ctx),
            _ => Step::Cancelled,
        }
    }
}

pub fn auto_constrain() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to constrain automatically", ObjFilter::Any, 1)], |a, ctx| {
        ctx.doc.begin_undo("AutoConstrain");
        let added = ctx.doc.auto_constrain(&a[0].ids());
        ctx.log(format!("{} constraints added.", added.len()));
        Step::Done
    })
}

pub struct DelConstraintCmd;

impl Command for DelConstraintCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to remove constraints from", ObjFilter::Any, 0).opts([opt("All"), opt("Id")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if o == "All" => {
                ctx.doc.begin_undo("DelConstraint");
                let n = ctx.doc.state.parametric.constraints.len();
                ctx.doc.state.parametric.constraints.clear();
                ctx.doc.touch_revision();
                ctx.log(format!("{n} constraints removed."));
                Step::Done
            }
            Input::Option(_) => Prompt::number("Constraint id").step(),
            Input::Number(id) => {
                ctx.doc.begin_undo("DelConstraint");
                let ok = ctx.doc.remove_constraint(id as u32);
                ctx.doc.touch_revision();
                ctx.log(if ok { format!("Constraint #{id} removed.") } else { format!("No constraint #{id}.") });
                Step::Done
            }
            Input::Objects(ids) => {
                ctx.doc.begin_undo("DelConstraint");
                let before = ctx.doc.state.parametric.constraints.len();
                ctx.doc.state.parametric.constraints.retain(|c| !c.refs.iter().any(|(id, _)| ids.contains(id)));
                ctx.doc.touch_revision();
                ctx.log(format!("{} constraints removed.", before - ctx.doc.state.parametric.constraints.len()));
                Step::Done
            }
            Input::Empty => Step::Done,
            _ => Step::Cancelled,
        }
    }
}

pub struct ParametersCmd {
    asking: Option<String>,
}

impl Default for ParametersCmd {
    fn default() -> Self {
        Self { asking: None }
    }
}

impl ParametersCmd {
    fn list(ctx: &mut CmdCtx) {
        let rows = ctx.doc.parameter_rows();
        if rows.is_empty() {
            ctx.log("No parameters. Dimensional constraints create d1, rad1, ang1 ...; Set adds user parameters.");
        }
        for (name, expr, value, kind) in rows {
            ctx.log(format!("  {name} = {expr}  ({}) [{kind}]", value.map(fmt_num).unwrap_or_else(|| "error".into())));
        }
    }
}

impl Command for ParametersCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        Self::list(ctx);
        Prompt::options("Parameters").opts([opt("Set"), opt("Delete"), opt("List")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.asking.clone(), input) {
            (None, Input::Option(o)) if o == "Set" => {
                self.asking = Some("name".into());
                Prompt::text("Parameter name").step()
            }
            (None, Input::Option(o)) if o == "Delete" => {
                self.asking = Some("delete".into());
                Prompt::text("Parameter name to delete").step()
            }
            (None, Input::Option(_)) => {
                Self::list(ctx);
                Step::Done
            }
            (Some(a), Input::Text(t)) if a == "name" => {
                if t.trim().is_empty() {
                    return Step::Cancelled;
                }
                ctx.mem_set("Parameters.Name", t.trim());
                self.asking = Some("expr".into());
                Prompt::text(format!("Expression for {}", t.trim())).step()
            }
            (Some(a), Input::Text(t)) if a == "expr" => {
                let name = ctx.memory.get("Parameters.Name").cloned().unwrap_or_default();
                ctx.doc.begin_undo("Parameters");
                let r = ctx.doc.set_parameter(&name, t.trim());
                report(ctx, r);
                ctx.doc.touch_revision();
                Step::Done
            }
            (Some(a), Input::Text(t)) if a == "delete" => {
                ctx.doc.begin_undo("Parameters");
                let r = ctx.doc.set_parameter(t.trim(), "");
                report(ctx, r);
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

pub struct ConstraintBarCmd;

impl Command for ConstraintBarCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        let cs = ctx.doc.state.parametric.constraints.clone();
        if cs.is_empty() {
            ctx.log("No constraints in the document.");
        }
        for c in cs {
            let refs: Vec<String> = c.refs.iter().map(|(id, s)| format!("#{id}:{s}")).collect();
            let dim = if c.name.is_empty() { String::new() } else { format!("  {} = {}", c.name, c.expr) };
            ctx.log(format!("  [{}] {} {}{}", c.id, c.kind, refs.join(" "), dim));
        }
        Prompt::options("Constraint bars").opts([opt("Show"), opt("Hide")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        if let Input::Option(o) = input {
            ctx.doc.state.parametric.show_bars = o == "Show";
            ctx.doc.touch_revision();
        }
        Step::Done
    }
}

pub struct ConstraintSettingsCmd;

impl Command for ConstraintSettingsCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        let p = &ctx.doc.state.parametric;
        Prompt::options("Constraint settings").opts([optv("DistanceTolerance", fmt_num(p.distance_tolerance)), optv("AngleTolerance", fmt_num(p.angle_tolerance)), optv("Infer", if ctx.memory.get("Parametric.Infer").map_or(false, |v| v == "Yes") { "Yes" } else { "No" })]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if o == "DistanceTolerance" => {
                ctx.mem_set("Constraint.Asking", "d");
                Prompt::number("Distance tolerance").default_num(ctx.doc.state.parametric.distance_tolerance).step()
            }
            Input::Option(o) if o == "AngleTolerance" => {
                ctx.mem_set("Constraint.Asking", "a");
                Prompt::number("Angle tolerance (degrees)").default_num(ctx.doc.state.parametric.angle_tolerance).step()
            }
            Input::Option(o) if o == "Infer" => {
                let cur = ctx.memory.get("Parametric.Infer").map_or(false, |v| v == "Yes");
                ctx.mem_set("Parametric.Infer", if cur { "No" } else { "Yes" });
                self.start(ctx)
            }
            Input::Number(v) => {
                if ctx.memory.remove("Constraint.Asking").as_deref() == Some("d") {
                    ctx.doc.state.parametric.distance_tolerance = v.abs();
                } else {
                    ctx.doc.state.parametric.angle_tolerance = v.abs();
                }
                self.start(ctx)
            }
            _ => Step::Done,
        }
    }
}

#[allow(dead_code)]
fn unused(_: Ans) {}
