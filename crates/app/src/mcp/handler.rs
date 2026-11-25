//! Executes assistant requests against the application state. No window or GPU is
//! involved, so every method is unit-tested on a headless `App`.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use serde_json::{json, Value};
use tri_doc::{Geometry, Object, ObjectId};

use crate::app::App;
use crate::commands::{self, Prompt, PromptKind};

/// What a request produced: an answer now, or an answer once the capture with this id
/// has been rendered (see [`take_capture`]).
pub enum Outcome {
    Reply(Result<Value, String>),
    Deferred(u64),
}

fn ok(v: Value) -> Outcome {
    Outcome::Reply(Ok(v))
}

fn fail(e: impl Into<String>) -> Outcome {
    Outcome::Reply(Err(e.into()))
}

/// Execute one request. Commands run exactly as typed ones do, with `assistant_mode` set
/// so that nothing waits for a person.
pub fn handle(app: &mut App, method: &str, params: &Value) -> Outcome {
    if matches!(method, "run_commands" | "undo" | "redo") && busy_with_user(app) {
        return fail("the user is running a command in Triceratops");
    }
    match method {
        "hello" => ok(json!({ "version": crate::mcp::protocol::VERSION })),
        "run_commands" => run_commands(app, params),
        "get_state" => ok(state(app)),
        "list_objects" => ok(list_objects(app, params)),
        "object_info" => object_info(app, params),
        "find_commands" => ok(find_commands(params)),
        "undo" | "redo" => {
            let steps = params["steps"].as_u64().unwrap_or(1).clamp(1, 100) as usize;
            let cmd = if method == "undo" { "Undo" } else { "Redo" };
            let start = app.log.len();
            run_lines(app, &vec![cmd.to_string(); steps]);
            ok(json!({ "log": log_since(app, start) }))
        }
        "capture_view" => capture_view(app, params),
        other => fail(format!("unknown method {other}")),
    }
}

/// The user is typing, or running a command they started: leave them alone.
pub fn busy_with_user(app: &App) -> bool {
    !app.cmd_text.trim().is_empty() || (app.runner.is_active() && !app.assistant_owns_command)
}

fn run_lines(app: &mut App, lines: &[String]) {
    let was = app.assistant_mode;
    app.assistant_mode = true;
    app.run_script_lines(lines);
    app.assistant_mode = was;
}

fn log_since(app: &App, start: usize) -> Vec<String> {
    app.log.get(start.min(app.log.len())..).unwrap_or_default().to_vec()
}

/// A fingerprint of each object's geometry and attributes, to tell which ones a command changed.
fn fingerprints(app: &App) -> HashMap<ObjectId, u64> {
    app.doc
        .objects()
        .map(|o| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            serde_json::to_vec(&o.geometry).unwrap_or_default().hash(&mut h);
            serde_json::to_vec(&o.attributes).unwrap_or_default().hash(&mut h);
            (o.id, h.finish())
        })
        .collect()
}

fn run_commands(app: &mut App, params: &Value) -> Outcome {
    let Some(lines) = params["lines"].as_array() else { return fail("run_commands needs \"lines\": an array of strings") };
    let lines: Vec<String> = lines.iter().map(|l| l.as_str().map(str::to_string).unwrap_or_else(|| l.to_string())).collect();
    let before = fingerprints(app);
    let start = app.log.len();
    run_lines(app, &lines);
    let after = fingerprints(app);
    let mut added: Vec<ObjectId> = after.keys().filter(|id| !before.contains_key(id)).copied().collect();
    let mut changed: Vec<ObjectId> = after.iter().filter(|(id, h)| before.get(id).is_some_and(|b| b != *h)).map(|(id, _)| *id).collect();
    let mut deleted: Vec<ObjectId> = before.keys().filter(|id| !after.contains_key(id)).copied().collect();
    added.sort_unstable();
    changed.sort_unstable();
    deleted.sort_unstable();
    ok(json!({
        "log": log_since(app, start),
        "added": added,
        "changed": changed,
        "deleted": deleted,
        "prompt": prompt_json(app.runner.prompt()),
    }))
}

fn prompt_json(p: Option<&Prompt>) -> Value {
    let Some(p) = p else { return Value::Null };
    let kind = match &p.kind {
        PromptKind::Point => "point",
        PromptKind::PointOrNumber => "point_or_number",
        PromptKind::Objects { .. } => "objects",
        PromptKind::ObjectPick { .. } => "pick",
        PromptKind::Number => "number",
        PromptKind::Text => "text",
        PromptKind::Options => "options",
    };
    let options: Vec<Value> = p.options.iter().map(|o| json!({ "name": o.name, "value": o.value })).collect();
    json!({ "message": p.msg, "kind": kind, "options": options, "default": p.default })
}

fn state(app: &App) -> Value {
    let current = app.doc.current_layer();
    let layers: Vec<Value> = app
        .doc
        .layers()
        .iter()
        .enumerate()
        .map(|(i, l)| json!({ "name": l.name, "visible": l.visible, "locked": l.locked, "current": i == current }))
        .collect();
    json!({
        "prompt": prompt_json(app.runner.prompt()),
        "selected": app.doc.selected_ids(),
        "active_viewport": app.viewports.get(app.active).map(|v| v.name.clone()),
        "viewports": app.viewports.iter().map(|v| v.name.clone()).collect::<Vec<_>>(),
        "units": app.doc.settings.units,
        "layers": layers,
    })
}

fn bbox_json(app: &App, o: &Object) -> Value {
    let b = app.doc.object_bbox(o);
    json!([[b.min.x, b.min.y, b.min.z], [b.max.x, b.max.y, b.max.z]])
}

fn is_closed(g: &Geometry) -> bool {
    match g {
        Geometry::Curve(c) => c.is_closed(),
        other => other.is_solid(),
    }
}

fn summary(app: &App, o: &Object) -> Value {
    let layer = app.doc.layers().get(o.attributes.layer).map(|l| l.name.clone());
    json!({
        "id": o.id,
        "type": o.geometry.type_name(),
        "layer": layer,
        "name": o.attributes.name,
        "bbox": bbox_json(app, o),
        "closed": is_closed(&o.geometry),
    })
}

fn list_objects(app: &App, params: &Value) -> Value {
    let ty = params["type"].as_str().map(str::to_ascii_lowercase);
    let layer = params["layer"].as_str();
    let ids: Option<Vec<u64>> = params["ids"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect());
    let limit = params["limit"].as_u64().unwrap_or(200) as usize;
    let matching: Vec<&Object> = app
        .doc
        .objects()
        .filter(|o| ty.as_ref().is_none_or(|t| o.geometry.type_name().contains(t.as_str())))
        .filter(|o| layer.is_none_or(|l| app.doc.layers().get(o.attributes.layer).is_some_and(|x| x.name.eq_ignore_ascii_case(l))))
        .filter(|o| ids.as_ref().is_none_or(|ids| ids.contains(&o.id)))
        .collect();
    let objects: Vec<Value> = matching.iter().take(limit).map(|o| summary(app, o)).collect();
    json!({ "objects": objects, "total": matching.len() })
}

fn object_info(app: &App, params: &Value) -> Outcome {
    let Some(id) = params["id"].as_u64() else { return fail("object_info needs \"id\"") };
    let Some(o) = app.doc.get(id) else { return fail(format!("no object #{id}")) };
    let mut v = summary(app, o);
    if let Some(a) = tri_doc::Document::area_of(&o.geometry) {
        v["area"] = json!(a);
    }
    if let Some((vol, c)) = tri_doc::Document::volume_of(&o.geometry) {
        v["volume"] = json!(vol);
        v["centroid"] = json!([c.x, c.y, c.z]);
    }
    ok(v)
}

/// Queue a capture of a viewport (the active one by default) into memory; the reply
/// follows once a frame has rendered it.
fn capture_view(app: &mut App, params: &Value) -> Outcome {
    let viewport = match params["viewport"].as_str() {
        None => None,
        Some(name) => match app.viewports.iter().position(|v| v.name.eq_ignore_ascii_case(name)) {
            Some(i) if !app.visible_viewports().contains(&i) => {
                return fail(format!("viewport {} is not shown in the current layout; shown: {}", app.viewports[i].name, shown_names(app)));
            }
            Some(i) => Some(i),
            None => {
                let names: Vec<&str> = app.viewports.iter().map(|v| v.name.as_str()).collect();
                return fail(format!("no viewport named {name}; viewports: {}", names.join(", ")));
            }
        },
    };
    let w = params["width"].as_u64().map(|v| v.clamp(16, 4096) as u32);
    let h = params["height"].as_u64().map(|v| v.clamp(16, 4096) as u32);
    let max_size = match (w, h) {
        (None, None) => Some((1280, 1280)),
        (w, h) => Some((w.unwrap_or(u32::MAX), h.unwrap_or(u32::MAX))),
    };
    let id = app.next_capture_id;
    app.next_capture_id += 1;
    app.capture_requests.push(crate::app::CaptureRequest { viewport, memory: Some(id), max_size, ..Default::default() });
    Outcome::Deferred(id)
}

fn shown_names(app: &App) -> String {
    app.visible_viewports().iter().map(|&i| app.viewports[i].name.clone()).collect::<Vec<_>>().join(", ")
}

/// The finished capture with this id, if it has been rendered: `{png_base64, width, height}`.
pub fn take_capture(app: &mut App, id: u64) -> Option<Result<Value, String>> {
    let i = app.finished_captures.iter().position(|(cid, _)| *cid == id)?;
    let (_, result) = app.finished_captures.remove(i);
    Some(result.and_then(|png| {
        let img = image::load_from_memory(&png).map_err(|e| e.to_string())?;
        Ok(json!({
            "png_base64": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png),
            "width": img.width(),
            "height": img.height(),
        }))
    }))
}

/// Commands whose name, aliases or description contain every word of the query; name
/// matches first, implemented commands before placeholders.
fn find_commands(params: &Value) -> Value {
    let query = params["query"].as_str().unwrap_or("").to_ascii_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    let limit = params["limit"].as_u64().unwrap_or(20) as usize;
    let mut hits: Vec<(u8, commands::CommandInfo)> = commands::all_commands()
        .into_iter()
        .filter_map(|c| {
            let name = c.name.to_ascii_lowercase();
            let text = format!("{} {} {}", name, c.aliases.join(" "), c.description.to_ascii_lowercase());
            if words.is_empty() || !words.iter().all(|w| text.contains(w)) {
                return None;
            }
            let rank = if words.iter().all(|w| name.contains(w)) { 0 } else { 1 } + if commands::is_implemented(c.name) { 0 } else { 2 };
            Some((rank, c))
        })
        .collect();
    hits.sort_by_key(|(rank, c)| (*rank, c.name.len(), c.name));
    let list: Vec<Value> = hits
        .into_iter()
        .take(limit)
        .map(|(_, c)| json!({ "name": c.name, "aliases": c.aliases, "description": c.description, "implemented": commands::is_implemented(c.name) }))
        .collect();
    json!({ "commands": list })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use serde_json::{json, Value};

    fn reply(o: Outcome) -> Value {
        match o {
            Outcome::Reply(Ok(v)) => v,
            Outcome::Reply(Err(e)) => panic!("error reply: {e}"),
            Outcome::Deferred(_) => panic!("deferred"),
        }
    }

    fn run(app: &mut App, lines: &[&str]) -> Value {
        reply(handle(app, "run_commands", &json!({ "lines": lines })))
    }

    fn ids(v: &Value) -> Vec<u64> {
        v.as_array().unwrap().iter().map(|x| x.as_u64().unwrap()).collect()
    }

    #[test]
    fn run_commands_reports_added_and_log() {
        let r = run(&mut App::new(), &["Sphere", "0,0,0", "5"]);
        assert_eq!(ids(&r["added"]).len(), 1);
        assert!(r["prompt"].is_null());
        assert!(r["log"].to_string().contains("Sphere"));
    }

    #[test]
    fn run_commands_returns_pending_prompt_and_continues() {
        let mut app = App::new();
        let r = run(&mut app, &["Box", "0,0,0"]);
        assert!(!r["prompt"]["message"].as_str().unwrap().is_empty());
        assert!(r["prompt"]["kind"].is_string());
        let r = run(&mut app, &["10,10,0", "5"]);
        assert_eq!(ids(&r["added"]).len(), 1);
        assert!(r["prompt"].is_null());
    }

    #[test]
    fn bad_input_is_logged_not_an_error() {
        let r = run(&mut App::new(), &["Frobnicate", "Move", "#999"]);
        let log = r["log"].to_string();
        assert!(log.contains("Unknown command: Frobnicate"), "{log}");
        assert!(log.contains("No object matches #999"), "{log}");
    }

    #[test]
    fn changed_and_deleted_are_detected() {
        let mut app = App::new();
        let r = run(&mut app, &["Box", "0,0,0", "10,10,0", "5"]);
        let id = ids(&r["added"])[0];
        let r = run(&mut app, &["Move", &format!("#{id}"), "", "0,0,0", "0,0,5"]);
        assert_eq!(ids(&r["changed"]), vec![id], "{r}");
        let r = run(&mut app, &["Delete", &format!("#{id}")]);
        assert_eq!(ids(&r["deleted"]), vec![id], "{r}");
    }

    #[test]
    fn list_objects_and_object_info() {
        let mut app = App::new();
        let r = run(&mut app, &["Box", "0,0,0", "10,10,0", "5", "Sphere", "50,0,0", "2"]);
        assert_eq!(ids(&r["added"]).len(), 2);
        let l = reply(handle(&mut app, "list_objects", &json!({})));
        assert_eq!(l["total"], 2);
        let b = &l["objects"][0];
        assert!(b["type"].as_str().unwrap().contains("polysurface"), "{b}");
        assert_eq!(b["closed"], true);
        let info = reply(handle(&mut app, "object_info", &json!({ "id": b["id"] })));
        assert!((info["volume"].as_f64().unwrap() - 500.0).abs() < 1.0, "{info}");
        let limited = reply(handle(&mut app, "list_objects", &json!({ "limit": 1 })));
        assert_eq!(limited["objects"].as_array().unwrap().len(), 1);
        assert_eq!(limited["total"], 2);
        match handle(&mut app, "object_info", &json!({ "id": 999 })) {
            Outcome::Reply(Err(e)) => assert_eq!(e, "no object #999"),
            _ => panic!("expected an error"),
        }
    }

    #[test]
    fn find_commands_matches_descriptions() {
        let r = reply(handle(&mut App::new(), "find_commands", &json!({ "query": "fillet" })));
        let names: Vec<&str> = r["commands"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"FilletEdge"), "{names:?}");
    }

    #[test]
    fn undo_removes_last_object() {
        let mut app = App::new();
        run(&mut app, &["Sphere", "0,0,0", "5"]);
        reply(handle(&mut app, "undo", &json!({})));
        assert_eq!(app.doc.objects().count(), 0);
        reply(handle(&mut app, "redo", &json!({})));
        assert_eq!(app.doc.objects().count(), 1);
    }

    #[test]
    fn get_state_reports_prompt_and_layers() {
        let mut app = App::new();
        run(&mut app, &["Circle"]);
        let s = reply(handle(&mut app, "get_state", &json!({})));
        assert!(s["prompt"]["message"].is_string());
        assert!(s["layers"].as_array().unwrap().iter().any(|l| l["current"] == true));
        assert!(s["units"].is_string());
    }

    #[test]
    fn unknown_method_is_an_error() {
        assert!(matches!(handle(&mut App::new(), "explode", &json!({})), Outcome::Reply(Err(_))));
    }

    #[test]
    fn request_and_response_round_trip() {
        let line = serde_json::to_string(&crate::mcp::protocol::Request { id: 7, method: "get_state".into(), params: json!({}) }).unwrap();
        let back: crate::mcp::protocol::Request = serde_json::from_str(&line).unwrap();
        assert_eq!(back.id, 7);
        let ok = crate::mcp::protocol::Response::ok(7, json!({"a": 1}));
        assert_eq!(serde_json::to_value(&ok).unwrap(), json!({"id": 7, "result": {"a": 1}}));
        let err = crate::mcp::protocol::Response::err(7, "boom");
        assert_eq!(serde_json::to_value(&err).unwrap(), json!({"id": 7, "error": "boom"}));
    }

    #[test]
    fn capture_view_defers_and_returns_png() {
        let mut app = App::new();
        let vp = app.viewports.iter().position(|v| v.name == "Top").unwrap();
        let Outcome::Deferred(id) = handle(&mut app, "capture_view", &json!({ "viewport": "top" })) else { panic!("expected deferred") };
        let req = app.capture_requests.last().cloned().expect("a capture request");
        assert_eq!(req.memory, Some(id));
        assert_eq!(req.viewport, Some(vp));
        assert!(take_capture(&mut app, id).is_none(), "nothing rendered yet");
        app.finish_capture(&req, 4, 2, vec![255; 32]);
        let v = take_capture(&mut app, id).expect("finished").expect("ok");
        assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(4), Some(2)));
        let png = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, v["png_base64"].as_str().unwrap()).unwrap();
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!((img.width(), img.height()), (4, 2));
    }

    #[test]
    fn capture_view_scales_to_requested_width() {
        let mut app = App::new();
        // widths below 16 px are raised to 16
        let Outcome::Deferred(id) = handle(&mut app, "capture_view", &json!({ "width": 2 })) else { panic!() };
        let req = app.capture_requests.last().cloned().unwrap();
        app.finish_capture(&req, 64, 32, vec![255; 64 * 32 * 4]);
        let v = take_capture(&mut app, id).unwrap().unwrap();
        assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(16), Some(8)));
    }

    #[test]
    fn capture_view_unknown_viewport_lists_the_real_ones() {
        match handle(&mut App::new(), "capture_view", &json!({ "viewport": "Bottom" })) {
            Outcome::Reply(Err(e)) => assert!(e.starts_with("no viewport named Bottom; viewports:") && e.contains("Perspective"), "{e}"),
            _ => panic!("expected an error"),
        }
    }

    #[test]
    fn capture_view_of_a_hidden_viewport_is_refused() {
        let mut app = App::new();
        run(&mut app, &["MaxViewport"]);
        let hidden = app.viewports.iter().enumerate().find(|(i, _)| !app.visible_viewports().contains(i)).map(|(_, v)| v.name.clone()).expect("a hidden viewport");
        match handle(&mut app, "capture_view", &json!({ "viewport": hidden })) {
            Outcome::Reply(Err(e)) => assert!(e.contains("not shown"), "{e}"),
            _ => panic!("expected an error"),
        }
    }

    fn err(o: Outcome) -> String {
        match o {
            Outcome::Reply(Err(e)) => e,
            _ => panic!("expected an error"),
        }
    }

    #[test]
    fn request_refused_while_user_types() {
        let mut app = App::new();
        app.cmd_text = "Cir".into();
        assert_eq!(err(handle(&mut app, "run_commands", &json!({ "lines": ["Sphere"] }))), "the user is running a command in Triceratops");
        assert_eq!(app.cmd_text, "Cir");
        assert_eq!(app.doc.objects().count(), 0);
    }

    #[test]
    fn request_refused_while_user_runs_own_command() {
        let mut app = App::new();
        // the assistant leaves a command waiting, the user cancels it and starts their own
        run(&mut app, &["Box", "0,0,0"]);
        app.cancel();
        app.cmd_text = "Circle".into();
        app.submit();
        assert!(app.runner.is_active());
        assert_eq!(err(handle(&mut app, "run_commands", &json!({ "lines": ["5,5,0"] }))), "the user is running a command in Triceratops");
        assert_eq!(err(handle(&mut app, "undo", &json!({}))), "the user is running a command in Triceratops");
    }

    #[test]
    fn assistant_continues_its_own_pending_command() {
        let mut app = App::new();
        run(&mut app, &["Circle", "0,0,0"]);
        let r = run(&mut app, &["5"]);
        assert_eq!(ids(&r["added"]).len(), 1);
    }
}
