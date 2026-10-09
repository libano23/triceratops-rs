//! Page layouts: Layout, Layouts, LayoutProperties, Detail, ChangeSpace, CopyLayout, ImportLayout,
//! Hide/ShowInDetail, Hide/ShowLayersInDetail, CopyDetailToViewport, CopyViewportToDetail, SelDetail.

use super::simple::{Ask, SeqCmd};
use super::{fmt_num, opt, AppRequest, CmdCtx, Step};

fn lay(ctx: &mut CmdCtx, op: impl Into<String>, name: impl Into<String>) -> Step {
    ctx.requests.push(AppRequest::LayoutOp { op: op.into(), name: name.into() });
    Step::Done
}

pub fn layout() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Layout name (empty for Page n)"), Ask::Number("Page width", 297.0), Ask::Number("Page height", 210.0), Ask::Number("Initial detail count (0-4)", 1.0)], |a, ctx| {
        let name = a[0].text();
        lay(ctx, format!("new:{};{};{}", a[1].number().abs().max(1.0), a[2].number().abs().max(1.0), a[3].number().round().clamp(0.0, 4.0) as usize), name)
    })
}

pub fn layouts() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("Layouts", vec![opt("List"), opt("Activate"), opt("Delete"), opt("Rename"), opt("New")])], |a, ctx| {
        let op = a[0].text();
        ctx.mem_set("Layouts.Op", op.clone());
        match op.as_str() {
            "Activate" => Step::Prompt(super::Prompt::text("Layout name to activate")),
            "Delete" => Step::Prompt(super::Prompt::text("Layout name to delete")),
            "Rename" => Step::Prompt(super::Prompt::text("New name for the active layout")),
            "New" => lay(ctx, "new:297;210;1", ""),
            _ => lay(ctx, "list", ""),
        }
    })
    .then(|inp, ctx| {
        let super::Input::Text(t) = inp else { return Step::Cancelled };
        match ctx.memory.get("Layouts.Op").map(|s| s.as_str()) {
            Some("Delete") => lay(ctx, "delete", t.trim()),
            Some("Rename") => lay(ctx, format!("rename:{}", t.trim()), ""),
            _ => lay(ctx, "activate", t.trim()),
        }
    })
}

pub fn layout_properties() -> SeqCmd {
    SeqCmd::new(vec![Ask::Number("Page width", 297.0), Ask::Number("Page height", 210.0), Ask::Text("Name (empty keeps the current name)")], |a, ctx| {
        ctx.requests.push(AppRequest::LayoutOp { op: format!("size:{};{}", a[0].number().abs().max(1.0), a[1].number().abs().max(1.0)), name: String::new() });
        let name = a[2].text();
        if !name.trim().is_empty() {
            ctx.requests.push(AppRequest::LayoutOp { op: format!("rename:{}", name.trim()), name: String::new() });
        }
        Step::Done
    })
}

pub fn detail() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("First corner of detail (on the page)"), Ask::Point("Other corner")], |a, ctx| {
        let (p, q) = (a[0].point(), a[1].point());
        let (x0, x1) = (p.x.min(q.x), p.x.max(q.x));
        let (y0, y1) = (p.y.min(q.y), p.y.max(q.y));
        if x1 - x0 < 1e-6 || y1 - y0 < 1e-6 {
            return Step::Cancelled;
        }
        lay(ctx, format!("detail:{};{};{};{}", fmt_num(x0), fmt_num(y0), fmt_num(x1 - x0), fmt_num(y1 - y0)), "")
    })
}

pub fn change_space() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to move between model and page space", super::ObjFilter::Any, 1), Ask::Options("Move to", vec![opt("Page"), opt("Model")])], |a, ctx| {
        for id in a[0].ids() {
            ctx.doc.select(id, true);
        }
        lay(ctx, format!("change_space:{}", if a[1].text() == "Model" { "Model" } else { "Page" }), "")
    })
}

pub fn copy_layout() -> SeqCmd {
    SeqCmd::new(vec![], |_, ctx| lay(ctx, "copy", ""))
}

pub fn import_layout() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Viewport file saved with SaveViewports / ReadViewportsFromFile format")], |a, ctx| {
        let p = a[0].text();
        if p.is_empty() {
            return Step::Cancelled;
        }
        ctx.requests.push(AppRequest::ViewportsFile { path: p, save: false });
        Step::Done
    })
}

pub fn detail_visibility(op: &'static str) -> SeqCmd {
    let asks = match op {
        "hide_in_detail" | "show_selected_in_detail" => vec![Ask::Objects("Select objects", super::ObjFilter::Any, 1)],
        "hide_layers" | "show_layers" => vec![Ask::Text("Layer names separated by ; (empty = all)")],
        _ => vec![],
    };
    SeqCmd::new(asks, move |a, ctx| {
        match op {
            "hide_in_detail" | "show_selected_in_detail" => {
                for id in a[0].ids() {
                    ctx.doc.select(id, true);
                }
                lay(ctx, op, "")
            }
            "hide_layers" | "show_layers" => lay(ctx, format!("{op}:{}", a[0].text()), ""),
            _ => lay(ctx, op, ""),
        }
    })
}

pub fn detail_viewport_copy(to_viewport: bool) -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("Model viewport", vec![opt("Top"), opt("Front"), opt("Right"), opt("Perspective")])], move |a, ctx| lay(ctx, format!("{}:{}", if to_viewport { "detail_to_viewport" } else { "viewport_to_detail" }, a[0].text()), ""))
}

pub fn sel_detail() -> SeqCmd {
    SeqCmd::new(vec![], |_, ctx| lay(ctx, "sel_detail", ""))
}
