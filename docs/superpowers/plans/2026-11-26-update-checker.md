# Update checker, unsaved-changes guard, autosave — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ask before discarding unsaved changes, autosave to a separate file with recovery, and check GitHub for new versions with a changelog, self-updating the AppImage and downloading the Windows zip.

**Architecture:** One discard guard in `App` that every destructive action goes through; an autosave module that snapshots `doc.state`/`doc.settings` to a background writer; a new `update` module (pure release logic + `ureq` networking on threads, results over channels) with its UI in `ui/dialogs.rs` and the status bar; `CHANGELOG.md` feeding the release body.

**Tech Stack:** Rust, egui 0.37 / winit, `ureq` 3 (rustls), `sha2` 0.11, Python for the release script.

**Spec:** `docs/superpowers/specs/2026-11-26-update-checker-design.md` (including its Addendum).

## Global Constraints

- Requests: `https://api.github.com/repos/libano23/triceratops-rs/releases?per_page=20`, header `User-Agent: Triceratops/<CARGO_PKG_VERSION>` only; base URL overridable by `TRICERATOPS_UPDATE_URL`.
- Assets: name ends with `x86_64.AppImage` (Linux) / `windows-x64.zip` (Windows); verify `size` and `digest` (`sha256:<hex>`, when present).
- No automatic update check in tests, with `--script`, or with `TRICERATOPS_NO_UPDATE_CHECK=1`; automatic checks at most every 24 h.
- New options (serde defaults): `check_updates: true`, `last_update_check: 0`, `skipped_version: ""`, `notified_version: ""`, `autosave: true`, `autosave_minutes: 5` (clamped 1–60).
- Autosave path: `<config>/triceratops/autosave/<stem>.autosave.3dm`, `Untitled` for unsaved models.
- Network and file writing never block the UI thread; automatic-check failures are silent.
- English code/UI/commits; commits as Attilio Mandola <libanesevolante@pm.me>; clippy `-D warnings` and `cargo test --workspace` clean; no macOS anything.

## Review Focus

- Closing the window while the Save file dialog is cancelled: nothing is lost and the app keeps running.
- A download interrupted (network drop, Cancel) leaves no `.download` file and never touches the AppImage.
- A release list with a tag like `nightly` or `v1.0.0-rc1`, or a body of `null`: ignored or shown empty, no panic.
- The autosave folder unwritable or the disk full: one log line, no repeat spam every interval, the session continues.
- Two Triceratops windows open at once: each writes its own model's autosave file (named by the model), and recovery at start does not offer the file the other running window is still writing — recovery only lists files older than 2× the autosave interval.

---

### Task 1: Ask before discarding unsaved changes

**Files:** Modify `crates/app/src/app.rs` (guard, `AppRequest::Quit|FileNew|FileOpen` handling, `file_save` result), `crates/app/src/main.rs:242` (close event), `crates/app/src/ui/dialogs.rs` (dialog).

**Interfaces — Produces:**
- `pub enum Discard { Quit, New, Open, Restart(std::path::PathBuf) }` in `app.rs`.
- `App::request_discard(&mut self, action: Discard)` — runs it now if `!doc.modified`, else sets `self.save_prompt = Some(action)`.
- `App::answer_save_prompt(&mut self, answer: SaveAnswer)` with `pub enum SaveAnswer { Save, DontSave, Cancel }`.
- `fn file_save(&mut self, save_as: bool) -> bool` (true when written).
- `App::run_discard(&mut self, action: Discard)` performs it (Quit: cancel job, delete autosave [Task 2 hooks in], save options, `quit = true`; New; Open; Restart: spawn the path then Quit).

- [ ] **Step 1: Tests** in `app.rs` tests: `unmodified_quit_is_immediate` (fresh app, `request_discard(Discard::Quit)` → `quit`); `modified_quit_asks` (set `doc.modified = true` → `save_prompt == Some(Quit)`, `!quit`); `dont_save_proceeds` (→ `quit`); `cancel_keeps_everything` (→ `save_prompt == None`, `!quit`, `doc.modified`); `new_after_dont_save_clears_the_model` (add a point object, modified, `New`, `DontSave` → zero objects). Save with a path: give `doc.path` a temp `.3dm`, `answer_save_prompt(Save)` → file exists, `quit`.
- [ ] **Step 2: Run** `cargo test -p triceratops --bin triceratops discard` — Expected: FAIL (types missing).
- [ ] **Step 3: Implement.** `AppRequest::Quit/FileNew/FileOpen` call `request_discard`; `main.rs` `CloseRequested` sends `AppRequest::Quit` through `request_discard` instead of exiting (the frame loop already exits on `app.quit`). Dialog in `dialogs.rs`: modal-looking `egui::Window` "Unsaved changes", text `Save changes to {title}?`, buttons Save / Don't Save / Cancel, Esc = Cancel.
- [ ] **Step 4: Run** tests + clippy — Expected: PASS.
- [ ] **Step 5: Commit** "Ask before discarding unsaved changes".

### Task 2: Autosave and recovery

**Files:** Create `crates/app/src/autosave.rs`; modify `app.rs` (fields, options, `AppRequest::Autosave`, per-frame tick, Save/Quit hooks), `ui/dialogs.rs` (General options page, recovery dialog), `main.rs` (mod).

**Interfaces — Produces:**
- `autosave::path_for(doc_path: Option<&Path>) -> Option<PathBuf>` (in the config dir; `Untitled` when `None`).
- `autosave::leftovers(dir: &Path, older_than: Duration) -> Vec<(PathBuf, SystemTime)>`.
- `struct Autosaver { last_revision: u64, last_time: Instant, writing: Option<Receiver<Result<PathBuf, String>>>, failed_once: bool }` with `fn tick(&mut self, now: Instant, doc: &Document, minutes: u32, enabled: bool) -> Option<SaveJob>` (pure decision: due, modified, revision changed, not already writing) and `fn spawn_write(job: SaveJob)` writing on a thread via `tri_doc::io3dm::save` of a temp `Document` built with `load_state(state.clone(), settings.clone(), None)`.
- `App::recovered: Vec<(PathBuf, SystemTime)>` shown by the recovery dialog.

- [ ] **Step 1: Tests** in `autosave.rs`: `path_uses_the_file_stem` (`/x/plate.3dm` → `.../autosave/plate.autosave.3dm`; `None` → `Untitled.autosave.3dm`); `tick_waits_for_the_interval_and_a_change` (not due before N minutes, not when revision unchanged, not when disabled, due after); `leftovers_skip_recent_files` (temp dir, one old file via `filetime`-free approach: compare against a `now` parameter — make `leftovers` take `now: SystemTime`). In `app.rs`: `autosave_options_round_trip` (autosave false, minutes 12) and clamping (0 → 1, 500 → 60); `save_deletes_the_autosave` (write the autosave file for a temp path, `file_save(false)` → file gone).
- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement**; the existing `Autosave` command writes `path_for(doc.path)` at once. The unwritable case logs once (`failed_once`) until a write succeeds. Recovery dialog at start (not in tests / `--script`): list name + local time, Open (via `open_path`, then `doc.path = None`, `modified = true`, log "Recovered <stem>"), Delete, Later. General page: checkbox + `DragValue` minutes.
- [ ] **Step 4: Run** tests + clippy — Expected: PASS.
- [ ] **Step 5: Commit** "Autosave to a separate file, with recovery at start".

### Task 3: Release logic and downloads (`update` module)

**Files:** Create `crates/app/src/update.rs`; modify `crates/app/Cargo.toml` (`ureq = { version = "3", default-features = false, features = ["rustls"] }`, `sha2 = "0.11"`), `main.rs` (mod).

**Interfaces — Produces:**
- `pub struct Version(u64, u64, u64)` with `Ord`; `pub fn parse_version(tag: &str) -> Option<Version>` (optional `v`, exactly three numeric parts; `-rc1` etc. → `None`).
- `#[derive(Deserialize)] pub struct Release { tag_name: String, name: Option<String>, body: Option<String>, draft: bool, prerelease: bool, html_url: String, assets: Vec<Asset> }`, `Asset { name, size: u64, browser_download_url, digest: Option<String> }`.
- `pub struct Offer { version: Version, tag: String, changelog: String, html_url: String, asset: Option<Asset> }`.
- `pub fn offer(releases: &[Release], current: &Version, kind: Install) -> Option<Offer>` — newest non-draft, non-prerelease, parseable, newer than `current`; changelog = for each newer release newest first `"## <tag>\n\n<body>\n\n"`.
- `pub enum Install { AppImage(PathBuf), WindowsZip, ReleasePage }`; `pub fn install_kind() -> Install` (`APPIMAGE` env → AppImage; `cfg!(windows)` → WindowsZip; else ReleasePage); asset suffix per kind.
- `pub fn fetch_releases(base: &str) -> Result<Vec<Release>, String>`.
- `pub fn download(asset: &Asset, dest: &Path, progress: &dyn Fn(u64), cancel: &AtomicBool) -> Result<(), String>` — streams to `dest`, deletes it on error/cancel, checks size and digest.
- `pub fn replace_appimage(downloaded: &Path, target: &Path) -> Result<(), String>` — chmod 755, rename over target.
- `pub struct Checker` running `fetch_releases`/`download` on threads, results via `std::sync::mpsc`, progress via `Arc<AtomicU64>`.

- [ ] **Step 1: Tests:** `versions_compare_numerically` (`0.10.0 > 0.9.1`, `v0.3.0 == 0.3.0`, `nightly`/`1.0`/`v1.0.0-rc1` → None); `offer_lists_newer_releases_only` (fixture JSON with 0.5.0, 0.4.0, 0.4.0-rc1 prerelease, a draft 0.6.0, 0.3.0, `body: null`; current 0.3.0 → version 0.5.0, changelog has `## v0.5.0` before `## v0.4.0`, no 0.3.0/rc/draft; current 0.5.0 → None); `assets_by_install_kind` (AppImage/zip chosen, missing → `asset: None`); `download_verifies_and_cancels` against a `std::net::TcpListener` thread serving fixed bytes over HTTP/1.1 (good digest → Ok and file present; wrong digest → Err and file absent; cancel set before start → Err "cancelled", file absent; progress called with the final byte count); `replace_appimage_swaps_only_after_verification` (temp dir: target replaced, mode 0o755 on unix).
- [ ] **Step 2: Run** `cargo test -p triceratops --bin triceratops update::` — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** tests + clippy — Expected: PASS. Also `cargo build --release` and check the AppImage still has no OpenSSL dependency (`ldd target/release/triceratops | grep -i ssl` → nothing).
- [ ] **Step 5: Commit** "Update module: release list, offers, verified downloads".

### Task 4: Update UI, options and scheduling

**Files:** Modify `app.rs` (options, start-up check, `Checker` polling, `CheckForUpdates`), `commands/registry.rs` (`"CheckForUpdates", [], "Check for a newer version of Triceratops"` + catalog entry), `ui/menus.rs` (Help › Check for Updates…), `ui/dialogs.rs` (update dialog, General page section), `ui/status.rs` (item).

**Interfaces — Consumes:** Task 3's `Checker`, `Offer`, `Install`, `download`, `replace_appimage`; Task 1's `request_discard(Discard::Restart(path))`; Task 2's General page.
**Produces:** `App::update: UpdateState { checker: Option<Checker>, offer: Option<Offer>, show: bool, manual: bool, status: String, downloading: Option<(Arc<AtomicU64>, u64, Arc<AtomicBool>)> }`; `pub fn should_auto_check(opts: &UserOptions, now_secs: u64, script: bool, env_off: bool) -> bool`; `pub fn should_open_dialog(offer: &Version, notified: &str, skipped: &str, manual: bool) -> (bool /*open*/, bool /*status item*/)`.

- [ ] **Step 1: Tests:** `auto_check_rules` (off, `<24 h`, script, env → false; otherwise true); `dialog_once_per_version` (new version → open + item; same as notified → no open, item; skipped → neither unless manual; manual → open); options round-trip for the four fields.
- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement**; the dialog per the spec ("Triceratops X is available (you have Y)", changelog in a `ScrollArea` with `##` lines bold and `* `/`- ` bullets, buttons by `Install`, progress bar + Cancel, errors shown in red, "Restart now" after an AppImage swap); manual "up to date" message; General page section (checkbox, last check as local date/time, Check now). Start-up check from `main.rs` after options load (skip per `should_auto_check`).
- [ ] **Step 4: Run** tests + clippy — Expected: PASS. Manual: `TRICERATOPS_UPDATE_URL` at a local `python3 -m http.server` serving a fixture `releases` JSON with a newer version → dialog shows the changelog; Download against a fixture asset writes and verifies it.
- [ ] **Step 5: Commit** "Check for updates: dialog, changelog, status item, options".

### Task 5: Changelog and release workflow

**Files:** Create `CHANGELOG.md`, `scripts/changelog_section.py`; modify `.github/workflows/release.yml`, `README.md` (Packaging/release note: write the CHANGELOG section first).

- [ ] **Step 1: Test** `scripts/changelog_section.py` with a doctest-style self-test (`python3 scripts/changelog_section.py --self-test`): extracts `## 0.4.0 …` up to the next `## `, exits 1 with a message when the version is missing.
- [ ] **Step 2: Implement**; `CHANGELOG.md` sections: `## Unreleased` (MicroStation key-ins, icon size, one-line command area, update check, save prompt, autosave, macOS builds dropped) and `## 0.3.0 — 2026-11-26` (Locust through MCP, Script/Expression, examples, background jobs + Esc, kernel fixes, new logo, Docs page). `release.yml`: before the release step, `python3 scripts/changelog_section.py "${GITHUB_REF_NAME#v}" > notes.md`, and `body_path: notes.md` instead of `generate_release_notes`.
- [ ] **Step 3: Run** the self-test and `python3 scripts/changelog_section.py 0.3.0` — Expected: the 0.3.0 section; `0.9.9` → exit 1.
- [ ] **Step 4: Commit** "Changelog file feeds the release notes".
