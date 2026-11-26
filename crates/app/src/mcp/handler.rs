//! Executes assistant requests against the application state. No window or GPU is
//! involved, so every method is unit-tested on a headless `App`.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use serde_json::{json, Value};
use tri_doc::{Geometry, Object, ObjectId};

use crate::app::App;
use crate::commands::{self, Prompt, PromptKind};

/// What a request produced: an answer now, an answer once the capture with this id has
/// been rendered (see [`take_capture`]), or one once the job the commands started has ended
/// (see [`finish_run`]).
pub enum Outcome {
    Reply(Result<Value, String>),
    Deferred(u64),
    Run(RunState),
}

/// A request waiting for a job: a `run_commands` (with what the document looked like
/// before it) or a Locust solve.
pub struct RunState {
    before: HashMap<ObjectId, u64>,
    start: usize,
    locust: bool,
}

impl RunState {
    fn locust() -> Self {
        RunState { before: HashMap::new(), start: 0, locust: true }
    }
}

/// Methods that change the model, refused while a job runs.
fn changes_model(method: &str) -> bool {
    matches!(method, "run_commands" | "undo" | "redo") || method.starts_with("locust_")
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
    if changes_model(method) {
        if let Some(job) = app.job_status().filter(|_| app.assistant_owns_command) {
            return fail(format!("busy: {} running for {} s (call cancel to stop it)", job.label, job.elapsed.as_secs()));
        }
    }
    if changes_model(method) && busy_with_user(app) {
        return fail("the user is running a command in Triceratops");
    }
    if method == "locust_bake" {
        if let Some(t) = app.locust.solving() {
            return fail(format!("busy: Locust solve running for {} s (wait for it, or call cancel)", t.as_secs()));
        }
    }
    if matches!(method, "locust_set_graph" | "locust_set_values" | "locust_bake") && app.locust.busy() {
        return fail("the user is working in Locust — try again shortly");
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
            if app.runner.is_active() {
                // the busy guard has made sure it is the assistant's own command: leave it,
                // or "Undo" would be taken as that command's option
                app.cancel();
            }
            run_lines(app, &vec![cmd.to_string(); steps]);
            ok(json!({ "log": log_since(app, start) }))
        }
        "capture_view" => capture_view(app, params),
        "locust_components" => ok(locust_components(params)),
        "locust_get_graph" => {
            app.locust.ensure_loaded();
            ok(app.locust.root.to_json())
        }
        "locust_set_graph" => match crate::locust::authoring::prepare(&params["graph"]) {
            Ok(g) => {
                app.locust.replace_root(g);
                locust_solve(app)
            }
            Err(e) => fail(e),
        },
        "locust_set_values" => {
            let Some(values) = params["values"].as_object() else { return fail("locust_set_values needs \"values\": an object of input → value") };
            match app.locust.set_inputs(values) {
                Ok(_) => locust_solve(app),
                Err(e) => fail(e),
            }
        }
        "locust_solve" => {
            app.locust.ensure_loaded();
            locust_solve(app)
        }
        "locust_bake" => {
            let before: std::collections::HashSet<ObjectId> = app.doc.objects().map(|o| o.id).collect();
            let n = app.locust.bake_into(&mut app.doc);
            if n == 0 {
                return fail("nothing to bake: wire geometry into a Bake component");
            }
            app.log.push(format!("Locust: baked {n} object(s)."));
            let mut added: Vec<ObjectId> = app.doc.objects().map(|o| o.id).filter(|id| !before.contains(id)).collect();
            added.sort_unstable();
            ok(json!({ "added": added }))
        }
        "cancel" => ok(json!({ "cancelled": (app.assistant_owns_command && app.cancel_job("cancelled")) || app.locust.cancel_solve() })),
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
    let state = RunState { before: fingerprints(app), start: app.log.len(), locust: false };
    run_lines(app, &lines);
    match finish_run(app, &state) {
        Some(v) => ok(v),
        None => Outcome::Run(state),
    }
}

/// The `run_commands` reply once no job runs and no script lines wait for one; `None` before.
pub fn finish_run(app: &mut App, state: &RunState) -> Option<Value> {
    if state.locust {
        return app.locust.solving().is_none().then(|| app.locust.report());
    }
    if app.runner.job().is_some() || !app.pending_lines.is_empty() {
        return None;
    }
    let (before, start) = (&state.before, state.start);
    let after = fingerprints(app);
    let mut added: Vec<ObjectId> = after.keys().filter(|id| !before.contains_key(id)).copied().collect();
    let mut changed: Vec<ObjectId> = after.iter().filter(|(id, h)| before.get(id).is_some_and(|b| b != *h)).map(|(id, _)| *id).collect();
    let mut deleted: Vec<ObjectId> = before.keys().filter(|id| !after.contains_key(id)).copied().collect();
    added.sort_unstable();
    changed.sort_unstable();
    deleted.sort_unstable();
    Some(json!({
        "log": log_since(app, start),
        "added": added,
        "changed": changed,
        "deleted": deleted,
        "prompt": prompt_json(app.runner.prompt()),
    }))
}

/// Start solving the Locust definition; the report follows when the solve ends.
fn locust_solve(app: &mut App) -> Outcome {
    app.locust.solve(&app.doc);
    Outcome::Run(RunState::locust())
}

/// The Locust component catalogue, filtered by words (all must appear in the name, category
/// or description) and category.
fn locust_components(params: &Value) -> Value {
    let words: Vec<String> = params["query"].as_str().unwrap_or("").split_whitespace().map(str::to_lowercase).collect();
    let category = params["category"].as_str().map(str::to_lowercase);
    let limit = params["limit"].as_u64().unwrap_or(40).clamp(1, 1000) as usize;
    let list: Vec<Value> = crate::locust::components::registry()
        .components
        .values()
        .filter(|c| category.as_ref().is_none_or(|k| c.category.to_lowercase() == *k))
        .filter(|c| {
            let hay = format!("{} {} {}", c.name, c.category, c.description).to_lowercase();
            words.iter().all(|w| hay.contains(w))
        })
        .take(limit)
        .map(|c| {
            let inputs: Vec<Value> = c
                .inputs
                .iter()
                .map(|i| {
                    let mut v = json!({ "name": i.name, "kind": i.kind.name() });
                    if let Some(d) = &i.default {
                        v["default"] = json!(d.repr());
                    }
                    if let Some((lo, hi)) = i.slider {
                        v["slider"] = json!([lo, hi]);
                    }
                    if i.optional {
                        v["optional"] = json!(true);
                    }
                    v
                })
                .collect();
            json!({ "name": c.name, "category": c.category, "description": c.description, "inputs": inputs, "outputs": c.outputs })
        })
        .collect();
    Value::Array(list)
}

/// The running job for `get_state` and progress frames, or null.
pub fn busy_json(app: &App) -> Value {
    if let Some(j) = app.job_status() {
        return json!({ "label": j.label, "elapsed_s": j.elapsed.as_secs(), "stage": j.stage, "count": j.count, "cancelling": j.cancelling });
    }
    match app.locust.solve_progress() {
        Some((t, stage, count)) => json!({ "label": "Locust solve", "elapsed_s": t.as_secs(), "stage": stage, "count": count, "cancelling": false }),
        None => Value::Null,
    }
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
        "busy": busy_json(app),
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

/// Commands whose name, aliases or description contain every word of the query: names
/// starting with the query first, then names containing it, then descriptions;
/// implemented commands before placeholders.
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
            let place = if name.starts_with(words[0]) && words.iter().all(|w| name.contains(w)) {
                0 // "boolean" → BooleanDifference, BooleanUnion, ...
            } else if words.iter().all(|w| name.contains(w)) {
                1
            } else {
                2
            };
            let rank = place + if commands::is_implemented(c.name) { 0 } else { 3 };
            Some((rank, c))
        })
        .collect();
    hits.sort_by_key(|(rank, c)| (*rank, c.name));
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
            Outcome::Run(_) => panic!("job"),
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

#[cfg(test)]
mod acceptance_tests {
    use super::*;
    use crate::app::App;
    use serde_json::json;

    fn run(app: &mut App, lines: &[&str]) -> Value {
        run_json(app, &json!(lines))
    }

    /// run_commands as the host answers it: a run that started a job waits for the job.
    fn run_json(app: &mut App, lines: &Value) -> Value {
        match handle(app, "run_commands", &json!({ "lines": lines })) {
            Outcome::Reply(Ok(v)) => v,
            Outcome::Reply(Err(e)) => panic!("error reply: {e}"),
            Outcome::Deferred(_) => panic!("deferred"),
            Outcome::Run(state) => {
                app.settle();
                finish_run(app, &state).expect("finished")
            }
        }
    }

    /// Found by the acceptance run: the union result stayed selected, BooleanDifference took
    /// it as preselection, and every explicit #id shifted to the following prompt.
    #[test]
    fn assistant_commands_ignore_leftover_selection() {
        let mut app = App::new();
        for lines in [
            json!(["Box", "0,0,0", "60,40,0", "8"]),
            json!(["Box", "10,32,8", "50,40,8", "30", "BooleanUnion", "#1,#2", ""]),
            json!(["Cylinder", "10,16,-1", "3", "10", "Cylinder", "50,16,-1", "3", "10"]),
            json!(["BooleanDifference", "#3", "#4,#5"]),
        ] {
            let _ = run_json(&mut app, &lines);
        }
        assert_eq!(app.doc.objects().count(), 1, "log: {:?}", app.log);
        let vol = tri_doc::Document::volume_of(&app.doc.objects().next().unwrap().geometry).unwrap().0;
        let expect = 60.0 * 40.0 * 8.0 + 40.0 * 8.0 * 30.0 - 2.0 * std::f64::consts::PI * 9.0 * 8.0;
        assert!((vol - expect).abs() < expect * 0.02, "volume {vol} vs {expect}");
    }

    /// Found by the acceptance run: "boolean" listed CurveBoolean and Boolean2Objects
    /// before BooleanDifference, which fell outside the first five.
    #[test]
    fn find_commands_puts_name_prefix_matches_first() {
        let r = match handle(&mut App::new(), "find_commands", &json!({ "query": "boolean", "limit": 5 })) { Outcome::Reply(Ok(v)) => v, _ => panic!() };
        let names: Vec<&str> = r["commands"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
        for n in ["BooleanDifference", "BooleanUnion", "BooleanIntersection"] {
            assert!(names.contains(&n), "{names:?}");
        }
    }

    /// Review: an assistant could not leave a command it started.
    #[test]
    fn assistant_can_cancel_its_own_pending_command() {
        let mut app = App::new();
        let r = run(&mut app, &["Polyline", "0,0,0", "10,0,0"]);
        assert!(!r["prompt"].is_null(), "Polyline still waits for points");
        let r = run(&mut app, &["_Cancel", "Sphere", "0,0,0", "5"]);
        assert_eq!(r["added"].as_array().unwrap().len(), 1, "only the sphere: {r}");
        assert!(r["prompt"].is_null(), "{r}");
        assert!(!r["log"].to_string().contains("Unknown command: _Cancel"), "{r}");
    }

    /// Review: undo fed "Undo" into a pending Polyline instead of undoing the document.
    #[test]
    fn undo_tool_undoes_the_document_even_with_a_pending_command() {
        let mut app = App::new();
        run(&mut app, &["Sphere", "0,0,0", "5"]);
        run(&mut app, &["Polyline", "0,0,0", "10,0,0"]);
        assert!(app.runner.is_active());
        match handle(&mut app, "undo", &json!({})) {
            Outcome::Reply(Ok(_)) => {}
            _ => panic!("undo failed"),
        }
        assert!(!app.runner.is_active(), "the pending command was cancelled");
        assert_eq!(app.doc.objects().count(), 0, "the sphere was undone");
    }

    /// Review: Render queued a capture that opened a Save dialog after assistant mode ended.
    #[test]
    fn render_from_an_assistant_never_reaches_a_file_dialog() {
        let mut app = App::new();
        let r = run(&mut app, &["Render"]);
        assert!(app.capture_requests.is_empty(), "nothing queued for a later dialog");
        assert!(r["log"].to_string().contains("needs a file path when run by an assistant"), "{r}");
    }

    /// The screw thread modelled in the 0.2.0 session (once seen to run for 11 minutes):
    /// it must finish quickly with a solid or a clear Boolean message.
    #[test]
    fn screw_thread_boolean_is_bounded() {
        let mut app = App::new();
        let t = std::time::Instant::now();
        for lines in [
            json!(["Cylinder", "0,0,0", "5", "20"]),
            json!(["Helix", "0,0,0", "0,0,20", "Turns", "10", "5"]),
            json!(["Polyline", "5,0,-0.8", "5.9,0,0", "5,0,0.8", "Close"]),
            json!(["Sweep1", "#2", "#3", ""]),
            json!(["Cap", "last"]),
        ] {
            let _ = run_json(&mut app, &lines);
        }
        let r = run(&mut app, &["BooleanUnion", "#1,#4", ""]);
        let log = r["log"].to_string();
        assert!(r["added"].as_array().map_or(0, |a| a.len()) == 1 || log.contains("Boolean"), "{r}");
        assert!(t.elapsed().as_secs() < 60, "took {:?}", t.elapsed());
    }

    #[test]
    fn mcp_run_commands_waits_for_job() {
        let mut app = App::new();
        let _ = handle(&mut app, "run_commands", &json!({ "lines": ["Box", "0,0,0", "10,10,0", "10", "Box", "5,5,5", "15,15,5", "10"] }));
        let Outcome::Run(state) = handle(&mut app, "run_commands", &json!({ "lines": ["BooleanUnion", "#1,#2", ""] })) else { panic!("expected a deferred run") };
        assert!(finish_run(&mut app, &state).is_none(), "still running");
        app.settle();
        let r = finish_run(&mut app, &state).expect("finished");
        assert_eq!(r["added"].as_array().unwrap().len(), 1, "{r}");
        assert_eq!(r["deleted"].as_array().unwrap().len(), 2, "{r}");
    }

    #[test]
    fn mcp_busy_refusal_and_state() {
        let mut app = App::new();
        crate::jobs::testing::heavy_spheres(&mut app);
        let Outcome::Run(state) = handle(&mut app, "run_commands", &json!({ "lines": ["BooleanUnion", "#1,#2", ""] })) else { panic!("expected a deferred run") };
        let Outcome::Reply(Err(e)) = handle(&mut app, "run_commands", &json!({ "lines": ["Sphere", "0,0,0", "5"] })) else { panic!("expected a refusal") };
        assert!(e.starts_with("busy: BooleanUnion running for "), "{e}");
        assert!(e.contains("call cancel to stop it"), "{e}");
        let Outcome::Reply(Ok(st)) = handle(&mut app, "get_state", &json!({})) else { panic!() };
        assert_eq!(st["busy"]["label"], "BooleanUnion", "{st}");
        let Outcome::Reply(Ok(c)) = handle(&mut app, "cancel", &json!({})) else { panic!() };
        assert_eq!(c["cancelled"], true);
        app.settle();
        let r = finish_run(&mut app, &state).expect("finished");
        assert!(r["log"].to_string().contains("BooleanUnion cancelled"), "{r}");
        assert_eq!(r["added"].as_array().unwrap().len(), 0);
        let Outcome::Reply(Ok(st)) = handle(&mut app, "get_state", &json!({})) else { panic!() };
        assert!(st["busy"].is_null(), "{st}");
        let Outcome::Reply(Ok(c)) = handle(&mut app, "cancel", &json!({})) else { panic!() };
        assert_eq!(c["cancelled"], false, "nothing left to cancel");
    }

    // ---------------------------------------------------------------- Locust

    fn ok_reply(o: Outcome) -> Value {
        match o {
            Outcome::Reply(Ok(v)) => v,
            Outcome::Reply(Err(e)) => panic!("error reply: {e}"),
            _ => panic!("not an immediate reply"),
        }
    }

    /// A request as the host answers it: a solve that runs as a job is waited for.
    fn ok_settled(app: &mut App, method: &str, params: &Value) -> Value {
        match handle(app, method, params) {
            Outcome::Run(state) => {
                app.settle();
                finish_run(app, &state).expect("finished")
            }
            o => ok_reply(o),
        }
    }

    #[test]
    fn mcp_locust_set_graph_deferred_answer() {
        let mut app = App::new();
        let Outcome::Run(state) = handle(&mut app, "locust_set_graph", &json!({ "graph": cylinder_graph() })) else { panic!("the solve runs as a job") };
        assert_eq!(busy_json(&app)["label"], "Locust solve");
        let e = err_reply(handle(&mut app, "locust_bake", &json!({})));
        assert!(e.starts_with("busy: Locust solve running"), "{e}");
        app.settle();
        let r = finish_run(&mut app, &state).expect("answered once solved");
        assert_eq!(r["errors"], 0, "{r}");
    }

    fn err_reply(o: Outcome) -> String {
        match o {
            Outcome::Reply(Err(e)) => e,
            Outcome::Reply(Ok(v)) => panic!("expected an error, got {v}"),
            _ => panic!("not an immediate reply"),
        }
    }

    fn cylinder_graph() -> Value {
        json!({
            "nodes": [
                { "type": "Number", "nickname": "Radius", "values": { "Value": 6 } },
                { "type": "Number", "nickname": "Height", "values": { "Value": 12 } },
                { "type": "Circle" },
                { "type": "Extrude" },
                { "type": "Preview" },
                { "type": "Bake" }
            ],
            "connections": [
                { "from": "Radius", "to": "Circle.Radius" },
                { "from": "Circle", "to": "Extrude.Curve" },
                { "from": "Height", "to": "Extrude.Distance" },
                { "from": "Extrude", "to": "Preview.Geometry" },
                { "from": "Extrude", "to": "Bake.Geometry" }
            ]
        })
    }

    fn volume_of(report: &Value) -> f64 {
        report["results"].as_array().unwrap().iter().find_map(|r| r["volume"].as_f64()).unwrap_or_else(|| panic!("no volume in {report}"))
    }

    #[test]
    fn locust_set_graph_solves_and_reports() {
        let mut app = App::new();
        let r = ok_settled(&mut app, "locust_set_graph", &json!({ "graph": cylinder_graph() }));
        assert_eq!(r["errors"], 0, "{r}");
        assert!(r["results"].as_array().unwrap().iter().any(|x| x["kinds"]["brep"] == 1), "{r}");
        let v = volume_of(&r);
        let expect = std::f64::consts::PI * 36.0 * 12.0;
        assert!((v - expect).abs() < expect * 0.01, "volume {v}");
        assert!(app.locust.open, "the editor shows the definition");
    }

    #[test]
    fn locust_set_values_by_nickname() {
        let mut app = App::new();
        ok_settled(&mut app, "locust_set_graph", &json!({ "graph": cylinder_graph() }));
        let r = ok_settled(&mut app, "locust_set_values", &json!({ "values": { "Radius": 3, "Extrude.Cap": true } }));
        let expect = std::f64::consts::PI * 9.0 * 12.0;
        assert!((volume_of(&r) - expect).abs() < expect * 0.01, "{r}");
        let e = err_reply(handle(&mut app, "locust_set_values", &json!({ "values": { "Nope": 3 } })));
        assert_eq!(e, "unknown node \"Nope\"");
        let e = err_reply(handle(&mut app, "locust_set_values", &json!({ "values": { "Circle.Radius": 3 } })));
        assert!(e.contains("is wired"), "{e}");
    }

    #[test]
    fn locust_bad_graph_leaves_canvas() {
        let mut app = App::new();
        ok_settled(&mut app, "locust_set_graph", &json!({ "graph": cylinder_graph() }));
        let before = ok_settled(&mut app, "locust_get_graph", &json!({}));
        let e = err_reply(handle(&mut app, "locust_set_graph", &json!({ "graph": { "nodes": [{ "type": "Extrud" }] } })));
        assert!(e.contains("did you mean"), "{e}");
        assert_eq!(ok_settled(&mut app, "locust_get_graph", &json!({})), before);
    }

    #[test]
    fn locust_refused_while_user_busy() {
        let mut app = App::new();
        app.locust.set_busy_for_test(true);
        let e = err_reply(handle(&mut app, "locust_set_graph", &json!({ "graph": cylinder_graph() })));
        assert_eq!(e, "the user is working in Locust — try again shortly");
    }

    #[test]
    fn locust_bake_adds_objects() {
        let mut app = App::new();
        ok_settled(&mut app, "locust_set_graph", &json!({ "graph": cylinder_graph() }));
        let r = ok_settled(&mut app, "locust_bake", &json!({}));
        assert_eq!(r["added"].as_array().unwrap().len(), 1, "{r}");
        assert_eq!(app.doc.objects().count(), 1);
        run(&mut app, &["Undo"]);
        assert_eq!(app.doc.objects().count(), 0, "one undo step");
    }

    #[test]
    fn locust_components_lists_script_and_expression() {
        let r = ok_reply(handle(&mut App::new(), "locust_components", &json!({ "category": "Script" })));
        let names: Vec<&str> = r.as_array().unwrap().iter().filter_map(|c| c["name"].as_str()).collect();
        assert_eq!(names, ["Expression", "Script"]);
        let script = &r[1]["description"];
        assert!(script.as_str().unwrap().contains("_inputs"), "{script}");
    }

    #[test]
    fn locust_components_search() {
        let r = ok_reply(handle(&mut App::new(), "locust_components", &json!({ "query": "extrude" })));
        let list = r.as_array().unwrap();
        let ex = list.iter().find(|c| c["name"] == "Extrude").expect("Extrude listed");
        assert_eq!(ex["inputs"][0]["name"], "Curve");
        assert_eq!(ex["outputs"][0], "Extrusion");
        assert!(list.len() < 20, "a search, not the catalogue");
        let r = ok_reply(handle(&mut App::new(), "locust_components", &json!({ "category": "Math", "limit": 500 })));
        assert!(r.as_array().unwrap().iter().all(|c| c["category"] == "Math"));
    }
}
