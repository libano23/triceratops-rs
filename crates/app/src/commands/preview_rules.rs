//! The interaction rule (from Serpentine3D), kept by a test over the command sources: a command
//! that asks for a length, radius, angle or factor in model space must let the mouse answer it,
//! and then it shows what the click would make (`SeqCmd` asks `Distance`/`Radius`/`Factor`/`Angle`
//! or `PointOrNumber`; the commands with their own state offer a trial copy through
//! `ghost_by_trial!`). A question that is not a length in the model says why in the table.

#![cfg(test)]

use std::collections::BTreeSet;

const SPATIAL: [&str; 21] = [
    "radius", "height", "length", "distance", "width", "depth", "size", "diameter", "offset", "thickness", "spacing", "gap", "extrude", "dist", "amount", "clearance", "falloff", "bulge", "scale", "factor", "angle",
];

/// (function, why the mouse cannot answer it)
const SEQ_EXEMPT: &[(&str, &str)] = &[
    ("layout", "the size of the sheet of paper, not a length in the model"),
    ("layout_properties", "the size of the sheet of paper, not a length in the model"),
    ("set_linetype_scale", "a display scale, no geometry on screen to measure it against"),
    ("apply_mapping", "texture repeat in model units, a material setting with no geometry to show"),
    ("ortho_angle", "a viewport setting (the angle ortho mode snaps to)"),
    ("snap_size", "a grid setting"),
    ("perspective_angle", "a camera setting"),
    ("zoom_lens", "a camera lens, not a length in the scene"),
    ("set_camera", "a camera lens, not a length in the scene"),
    ("tilt_view", "a camera angle"),
    ("dolly_zoom", "a camera lens change"),
    ("rotate_camera", "a camera angle"),
    ("orient_camera_to_srf", "a camera distance"),
    ("zoom_border", "a percentage of the view"),
    ("earth_anchor", "a compass direction for the site, no geometry"),
    ("convert_dots", "annotation text height, as printed"),
    ("hatch_scale", "pattern density as printed"),
    ("scale_text_height", "annotation text height, as printed"),
    ("hydrostatics", "a report; nothing is made to ghost"),
    ("clash", "selects the objects that clash: a selection, not geometry"),
    ("truncated_pyramid", "the height is answered by a click; 'Top scale' is a typed 0..1 ratio"),
];

/// (command, why the mouse cannot answer it)
const CUSTOM_EXEMPT: &[(&str, &str)] = &[
    ("HatchCmd", "pattern scale, as printed"),
    ("SmoothCmd", "a 0..1 strength, not a length"),
    ("FilletCmd", "a setting asked from an option before the curves are picked: there is no result yet"),
    ("ChamferCmd", "a setting asked from an option before the curves are picked: there is no result yet"),
    ("FilletSrfCmd", "a setting asked from an option before the surfaces are picked: there is no result yet"),
    ("OffsetMultipleCmd", "a setting asked from an option before the curves are picked: there is no result yet"),
    ("ZoomCmd", "a view factor"),
    ("GridCmd", "grid settings"),
    ("CurvatureGraphCmd", "a display scale"),
    ("GroundPlaneCmd", "a display setting asked from an option"),
    ("ConstraintSettingsCmd", "tolerances of the solver, settings"),
    ("LinetypesCmd", "a display scale"),
    ("ViewOpCmd", "view rotation and pan settings"),
    ("TextCmd", "annotation text height, as printed, asked from an option"),
    ("InsertCmd", "scale and rotation of a block, asked from options before it is placed"),
    ("PolylineCmd", "a length-lock option: the cursor's direction decides the next segment"),
    ("OffsetCmd", "a setting asked from an option before the curve is picked: there is no result yet"),
    ("ExtrudeTaperedCmd", "the draft angle, a setting asked from an option; the distance is answered by the mouse"),
    ("ExtrudeSrfVariantCmd", "the draft angle, a setting asked from an option; the distance is answered by the mouse"),
    ("DimRotatedCmd", "asked first, before any point: the dimension is placed afterwards with previews"),
    ("BlendSrfCmd", "the bulge, a setting asked from an option"),
    ("ArrayPolarCmd", "step angle and Z offset, settings asked from options; the angle to fill takes points"),
];

fn is_spatial(prompt: &str) -> bool {
    let p = prompt.to_lowercase();
    SPATIAL.iter().any(|w| p.contains(w)) && !p.contains("number of")
}

fn sources() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        if name.ends_with(".rs") && name != "flow_tests.rs" && name != "preview_rules.rs" {
            out.push((name, std::fs::read_to_string(&p).unwrap()));
        }
    }
    out
}

/// Every `Ask::Number("..")` or `Prompt::number("..")` whose text names a spatial quantity, with
/// the function (SeqCmd) or command it sits in.
fn typed_only_spatial_questions() -> (BTreeSet<String>, BTreeSet<String>) {
    let (mut seq, mut custom) = (BTreeSet::new(), BTreeSet::new());
    for (_file, src) in sources() {
        // items start at a line beginning with `pub fn`, `fn`, `impl` or `pub struct`
        let mut starts: Vec<usize> = Vec::new();
        for (i, _) in src.match_indices('\n') {
            let rest = &src[i + 1..];
            if rest.starts_with("pub fn ") || rest.starts_with("fn ") || rest.starts_with("impl ") || rest.starts_with("pub struct ") {
                starts.push(i + 1);
            }
        }
        starts.push(src.len());
        for w in starts.windows(2) {
            let item = &src[w[0]..w[1]];
            let head = item.lines().next().unwrap_or("");
            if let Some(name) = head.strip_prefix("pub fn ").and_then(|r| r.split('(').next()).filter(|_| head.contains("-> SeqCmd")) {
                if item.match_indices("Ask::Number(\"").any(|(i, _)| is_spatial(item[i + 13..].split('"').next().unwrap_or(""))) {
                    seq.insert(name.to_string());
                }
            }
            if let Some(name) = head.strip_prefix("impl ").and_then(|r| r.split(" for ").nth(1)).map(|r| r.split(' ').next().unwrap_or("").trim_start_matches("super::").to_string()).filter(|_| head.contains("Command for ")) {
                if item.match_indices("Prompt::number(\"").any(|(i, _)| is_spatial(item[i + 16..].split('"').next().unwrap_or(""))) {
                    custom.insert(name);
                }
            }
        }
    }
    (seq, custom)
}

#[test]
fn every_spatial_question_accepts_the_mouse_or_says_why_not() {
    let (seq, custom) = typed_only_spatial_questions();
    let seq_ok: BTreeSet<&str> = SEQ_EXEMPT.iter().map(|(n, _)| *n).collect();
    let custom_ok: BTreeSet<&str> = CUSTOM_EXEMPT.iter().map(|(n, _)| *n).collect();
    let missing_seq: Vec<&String> = seq.iter().filter(|n| !seq_ok.contains(n.as_str())).collect();
    let missing_custom: Vec<&String> = custom.iter().filter(|n| !custom_ok.contains(n.as_str())).collect();
    assert!(missing_seq.is_empty(), "SeqCmd functions that ask a length typed-only and are not exempt (use Ask::Distance/Radius/Factor/Angle or PointOrNumber): {missing_seq:?}");
    assert!(missing_custom.is_empty(), "commands that ask a length typed-only and are not exempt (use Prompt::point_or_number, point_as_number and ghost_by_trial!): {missing_custom:?}");
}

#[test]
fn the_exemption_tables_name_only_questions_that_still_exist() {
    let (seq, custom) = typed_only_spatial_questions();
    let stale_seq: Vec<&str> = SEQ_EXEMPT.iter().map(|(n, _)| *n).filter(|n| !seq.contains(*n)).collect();
    let stale_custom: Vec<&str> = CUSTOM_EXEMPT.iter().map(|(n, _)| *n).filter(|n| !custom.contains(*n)).collect();
    assert!(stale_seq.is_empty() && stale_custom.is_empty(), "no longer typed-only (remove from the tables): {stale_seq:?} {stale_custom:?}");
}
