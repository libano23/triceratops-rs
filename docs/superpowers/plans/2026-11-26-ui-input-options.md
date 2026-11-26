# One-line command area, icon size, MicroStation key-ins — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** MicroStation precision key-ins at point prompts, a saved toolbar icon size, and a command area that shrinks to one line, with no change for users who do not use them.

**Architecture:** Key-ins are new rules at the top of `input::parse_point_input`; they return the existing `PointInput::Point`, plus a new `PointInput::NeedsBase` so the app can say why a relative key-in was refused. Icon size is a field of `UserOptions` copied into `App`, read by the toolbar code in place of the `ICON` constant. The command area drops its 40 px floor and draws only the prompt line when there is no room for history.

**Tech Stack:** Rust, egui 0.37, serde (options.json).

**Spec:** `docs/superpowers/specs/2026-11-26-ui-input-options-design.md`

## Global Constraints

- Every existing point syntax parses exactly as before: `x,y[,z]`, `r`/`@` relative, `w` world, `wr`, `d<a`, `@d<a`, a lone number (distance constraint), `<a` (angle constraint).
- Key-ins are case-insensitive, allow spaces around `=` and `,`, and accept calculator expressions for each number (`parse_number`).
- `XY=x,y[,z]` absolute world; `DL=dx,dy[,dz]` world offset from the last point; `DX=dx,dy[,dz]` offset in the active construction plane's axes; `DI=distance,angle` polar from the last point, angle in degrees in the construction plane.
- Icon size: px, default 24, range 16–40; older options files load with 24.
- Code, comments, UI text and commits in English; commits as Attilio Mandola <libanesevolante@pm.me>.
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` stay clean.

## Review Focus

- `DL=`/`DX=`/`DI=` typed as the first point of a command (no last point): refused with "DL= needs a previous point", not "Unknown command".
- A malformed key-in (`dl=`, `dl=1`, `xy=1,2,3,4`, `di=5`) is refused, never panics or yields a partial point.
- A key-in typed at a `PointOrNumber` prompt is a point, not a number.
- An options file with an absurd `icon_size` (0, 500) is clamped into 16–40.
- Dragging the command area to its minimum still shows the prompt and accepts typing.

---

### Task 1: MicroStation key-ins

**Files:**
- Modify: `crates/app/src/input.rs` (enum `PointInput`, `parse_point_input`, tests)
- Modify: `crates/app/src/app.rs` (match on `parse_point_input` near line 1825)
- Modify: `crates/app/src/ui/panels.rs:437` (help line), `site/docs.html` (command line section), `docs/FEATURES.md` (input section, one line)

**Interfaces:**
- Produces: `PointInput::NeedsBase(&'static str)` — the key-in name (`"DL="`, `"DX="`, `"DI="`) refused for lack of a last point.

- [ ] **Step 1: Write the failing tests** in `input.rs` tests, using the existing `cp` fixture (a construction plane whose axes differ from world) and a rotated plane:

```rust
#[test]
fn microstation_keyins() {
    let cp = /* the fixture used by the existing test */;
    let b = Some(DVec3::new(5.0, 5.0, 5.0));
    assert_eq!(parse_point_input("XY=1,2,3", &cp, None), Some(PointInput::Point(DVec3::new(1.0, 2.0, 3.0))));
    assert_eq!(parse_point_input("xy = 1 , 2", &cp, None), Some(PointInput::Point(DVec3::new(1.0, 2.0, 0.0))));
    assert_eq!(parse_point_input("dl=1,2,3", &cp, b), Some(PointInput::Point(DVec3::new(6.0, 7.0, 8.0))));
    assert_eq!(parse_point_input("DX=1,0", &cp, b), Some(PointInput::Point(DVec3::new(5.0, 5.0, 5.0) + cp.x_axis)));
    assert_eq!(parse_point_input("dx=0,1,2", &cp, b), Some(PointInput::Point(DVec3::new(5.0, 5.0, 5.0) + cp.y_axis + cp.z_axis * 2.0)));
    // DI: distance 2 at 90° in the plane = 2 along the plane's y axis
    let p = parse_point_input("di=2,90", &cp, b);
    // compare within 1e-12 to b + cp.y_axis * 2
    assert_eq!(parse_point_input("dl=10/2,0", &cp, b), Some(PointInput::Point(DVec3::new(10.0, 5.0, 5.0))));
    for k in ["dl=1,2", "dx=1,2", "di=3,0"] {
        assert!(matches!(parse_point_input(k, &cp, None), Some(PointInput::NeedsBase(_))), "{k}");
    }
    for bad in ["dl=", "dl=1", "xy=1,2,3,4", "di=5", "di=1,2,3", "zz=1,2", "xy=a,b"] {
        assert_eq!(parse_point_input(bad, &cp, b), None, "{bad}");
    }
}
```

Also add `existing_syntax_unchanged`: the assertions already in the module's current test, plus `"@3<90"` and `"w1,2,3"` with a base, must keep their values (copy them verbatim; this guards the new prefix rules from swallowing `w`/`r`).

- [ ] **Step 2: Run** `cargo test -p triceratops input::` — Expected: FAIL (`NeedsBase` not defined).

- [ ] **Step 3: Implement.** Add `NeedsBase(&'static str)` to `PointInput` (doc comment: a relative key-in typed with no previous point). In `parse_point_input`, before the existing prefix loop: strip all whitespace, lowercase; if the text matches `^(xy|dl|dx|di)=(.*)$`, split the rest on `,`, `parse_number` each part, and return per the Global Constraints (`xy`/`dl`/`dx` take 2 or 3 numbers, `di` exactly 2; anything else `None`). For `dl`/`dx`/`di` with `base == None` return `NeedsBase("DL=")` etc. Never fall through to the old rules once the `xx=` shape matched.

- [ ] **Step 4: Handle `NeedsBase` in `app.rs`** next to the `None` arm: log `format!("{k} needs a previous point")` and return `None` (prompt stays). Add an app-level test in `app.rs` tests: `App::new()`, start `Polyline`, set `app.cmd_text` and call `app.submit()` for `"xy=0,0,0"`, `"dl=10,0"`, `"dx=0,10"`, `"di=10,180"`, then `""` (Enter); assert the new curve's points are (0,0,0), (10,0,0), (10,10,0), (0,10,0) within 1e-9 (default Top view plane = world XY). A second test: `Polyline` then `"dl=1,1"` first → the log's last line is `"DL= needs a previous point"` and the command still waits for a point.

- [ ] **Step 5: Docs.** Help line in `ui/panels.rs:437` gains `  XY=  DL=  DX=  DI= (MicroStation)`. In `site/docs.html` "The command line" section, add a bullet: "MicroStation key-ins work too: `XY=x,y,z` (absolute, world), `DL=dx,dy,dz` (from the last point, world axes), `DX=dx,dy,dz` (from the last point, construction-plane axes) and `DI=distance,angle`." Same sentence, shortened, in `docs/FEATURES.md` where point input is described.

- [ ] **Step 6: Run** `cargo test -p triceratops` and clippy — Expected: all pass, 0 warnings.

- [ ] **Step 7: Commit** "MicroStation key-ins at point prompts: XY=, DL=, DX=, DI=".

### Task 2: Icon size setting

**Files:**
- Modify: `crates/app/src/app.rs` (`UserOptions`, `App` field, `load_options`, `save_options`)
- Modify: `crates/app/src/ui/icons.rs:6` (`ICON` becomes the default only), `crates/app/src/ui/toolbars.rs:435,476,527,529,573,575`
- Modify: `crates/app/src/ui/dialogs.rs` ("Appearance" page)

**Interfaces:**
- Produces: `App::icon_size: f32` (always within 16–40); `UserOptions::icon_size: f32` with serde default 24; `pub const ICON_DEFAULT: f32 = 24.0`, `ICON_MIN = 16.0`, `ICON_MAX = 40.0` in `ui/icons.rs`; `fn clamp_icon_size(f32) -> f32` there.

- [ ] **Step 1: Failing tests** in `app.rs` tests: `icon_size_round_trips` — set `app.icon_size = 32.0`, build the `UserOptions` the way `save_options` does (factor that into `fn user_options(&self) -> UserOptions` if it is not already separate), serialize to JSON, deserialize, apply with the same code `load_options` uses (`fn apply_user_options(&mut self, o: UserOptions)`), assert 32. `icon_size_defaults_and_clamps` — JSON without the field gives 24; `"icon_size": 0` gives 16; `500` gives 40.

- [ ] **Step 2: Run** — Expected: FAIL.

- [ ] **Step 3: Implement** the field, the clamp on load, and replace every `ICON` use in `toolbars.rs` with `app.icon_size` (thread `size: f32` into helpers that lack `app`). Slider in Appearance: `egui::Slider::new(&mut app.icon_size, ICON_MIN..=ICON_MAX).step_by(2.0).text("Icon size")`, saved like the other Appearance controls.

- [ ] **Step 4: Run** tests and clippy — Expected: pass. Launch the app (`./triceratops.sh`), set 16 and 40, screenshot the toolbars with `shoot.sh`-style capture and look at them: icons and flyouts scale, the sidebar's column count adapts.

- [ ] **Step 5: Commit** "Toolbar icon size is a setting (Options › Appearance)".

### Task 3: One-line command area

**Files:**
- Modify: `crates/app/src/ui/command_area.rs`

**Interfaces:**
- Produces: `pub const ONE_LINE: f32` in `command_area.rs` — the panel height that fits the prompt line alone (line height 18 + frame margins 2+2 + 4 spacing = 26).

- [ ] **Step 1: Implement.** `.min_size(ONE_LINE)`. When `ui.available_height() < 2.0 * line_h + 6.0`, skip the history `ScrollArea`; the prompt row's response gets `.on_hover_ui` listing the last 8 `app.log` lines (oldest first) when history is hidden.

- [ ] **Step 2: Test.** egui headless test in `command_area.rs` (or `app.rs` tests if UI tests live there): run one frame of `show` inside an `egui::Context::run` with a screen 30 px tall for the panel area — no panic, and `app.cmd_text` typed via events still reaches the field. If a headless frame cannot size the panel, cover it manually: launch, drag the area to its minimum, screenshot, type `Line` + Enter and see it start.

- [ ] **Step 3: Run** full tests and clippy — Expected: pass.

- [ ] **Step 4: Commit** "The command area shrinks to the prompt line alone".
