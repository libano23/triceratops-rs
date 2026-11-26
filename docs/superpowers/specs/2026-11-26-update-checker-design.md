# Update checker — design

Date: 2026-11-26. Target: the release after 0.3.0.

## Intent

Users should learn about new Triceratops versions without visiting the website, see what
changed, and get the new version with as little effort as the platform allows.

Goals:
- check GitHub releases of `libano23/triceratops-rs` automatically and on demand;
- show the changes of every version newer than the running one, in readable words;
- Linux AppImage: download, verify, replace itself, offer a restart;
- Windows zip: download and verify into the Downloads folder, show it.

Non-goals: macOS (dropped after 0.3.0); updating builds made from source or installed by
a distribution package; delta updates; a background service; pre-release channels.

Anti-goals: an update that can leave a broken program; nagging (a dialog at every start);
sending anything about the user to the network beyond a plain request for the release
list; blocking the window while checking or downloading.

## Approaches (decided)

When the user accepts an update:
- A. open the release page in the browser — simplest, all the work left to the user;
- B. download the right file to Downloads, verified, and show it — the user replaces it;
- C. full self-update: download, replace, restart — most convenient, most delicate.

**Chosen: C for the AppImage** (one file, replaced by an atomic rename next to it) and
**B for Windows** (a running .exe cannot be overwritten without a helper program).
Builds that are neither (source builds, distro packages) fall back to A.

Changelog source: GitHub's generated notes say only "PR title by @user in #N". Chosen:
a hand-written `CHANGELOG.md` in the repository, one section per version; the release
workflow puts that version's section into the release body, and the program shows the
bodies of the newer releases.

## Design

### Network

- New dependencies (app crate): `ureq` with rustls (no OpenSSL; works in the AppImage and
  on Windows), `sha2` for the digest. Requests go to `https://api.github.com` with
  `User-Agent: Triceratops/<version>` and nothing else identifying.
- The base URL can be overridden with `TRICERATOPS_UPDATE_URL` (tests use a local HTTP
  server; also useful to test a release by hand).
- `GET /repos/libano23/triceratops-rs/releases?per_page=20`: drafts and pre-releases are
  ignored; versions are compared numerically from the tag (`v0.4.0` → 0.4.0) against
  `CARGO_PKG_VERSION`. Newer releases, newest first, give the version offered (the newest)
  and the changelog (each newer release's body under its version heading).
- All network work runs on a background thread; the UI polls a channel each frame. Any
  failure of an automatic check is silent (logged with `RUST_LOG` only); a manual check
  says what went wrong in the dialog.

### When it checks

- At start, in the background, if automatic checks are on and the last check is more than
  24 hours old. The time of the last check is saved in the options. No automatic check
  when started with `--script` (screenshots, automation) or with
  `TRICERATOPS_NO_UPDATE_CHECK=1`, nor in tests. *(Builder's-check call.)*
- On demand: Help › Check for Updates… and the command `CheckForUpdates`, which always
  checks and always opens the dialog (also to say "Triceratops 0.4.0 is up to date").
- New options (saved with the others, all with serde defaults): `check_updates: bool`
  (default true), `last_update_check: u64` (Unix seconds), `skipped_version: String`,
  `notified_version: String` (the last version whose dialog opened by itself).
  A new Options page "General" holds the checkbox, the last check time and "Check now"
  (and the autosave settings below).

### What the user sees

- A newer version found by an automatic check: the dialog opens by itself only if that
  version differs from `notified_version` (then saved), so it opens once per version
  across restarts; otherwise, and after "Later", a small "Update available" item in the
  status bar opens it. A version equal to `skipped_version` ("Skip this version") gets
  neither the dialog nor the status item from automatic checks. *(Builder's-check call:
  persistence across restarts.)*
- The dialog: "Triceratops 0.4.0 is available (you have 0.3.0)", then the changelog in a
  scrolling area (Markdown headings, bullet lists and inline code shown plainly), then
  the buttons:
  - AppImage: **Update and restart**, Skip this version, Later;
  - Windows: **Download**, Skip this version, Later;
  - other builds: **Open the release page**, Skip this version, Later.
  While downloading: a progress bar with bytes and a Cancel button; the window stays usable.

### Installing

- Asset choice: the AppImage whose name ends in `x86_64.AppImage`; the zip whose name ends
  in `windows-x64.zip`. A release without the right asset offers the release page instead.
- Verification: the downloaded size must equal the asset's `size`, and its SHA-256 must
  equal the asset's `digest` (`sha256:<hex>`) when GitHub provides it. A mismatch deletes
  the file and says so; nothing is replaced.
- AppImage (`APPIMAGE` is set): download to `<AppImage path>.download` in the same folder,
  verify, `chmod 755`, rename over the running AppImage (atomic on the same filesystem; the
  running process keeps its mounted copy). Then "Restart now" starts the new AppImage with
  no arguments and closes this window after asking about unsaved changes as on exit; "Later"
  leaves the new version for the next start. If the folder is not writable, fall back to
  the Windows behaviour (download to Downloads) and say why.
- Windows: download to the Downloads folder (`dirs::download_dir`, else the temp folder)
  under the asset's name, verify, then open Explorer with the file selected and say
  "Unzip it over your current Triceratops folder".

### Release workflow

- `CHANGELOG.md` at the repository root, newest first: `## 0.4.0 — 2026-12-…` then
  bullets written for users. It starts with the sections for 0.3.0 and the unreleased
  next version (MicroStation key-ins, icon size, one-line command area, update checker,
  macOS builds dropped).
- `release.yml`: a step extracts the section of the tag's version
  (`scripts/changelog_section.py <version>`) into the release body instead of
  `generate_release_notes`; the job fails if the section is missing, so a release cannot
  go out without its changelog.

## Addendum (2026-11-26): unsaved changes and autosave

Asked by the user while approving this design. Today nothing asks before discarding the
model: closing the window, `Exit`, `New` and `Open` all drop unsaved changes silently,
and the existing `Autosave` command only writes on request to a fixed file.

### Ask before discarding unsaved changes

- Closing the window (the window manager's close request), `Exit`, `New`, `Open` and the
  updater's "Restart now" go through one guard. If `doc.modified` is false the action runs
  at once; otherwise a modal dialog asks "Save changes to <title>?" with **Save**,
  **Don't Save** and **Cancel**. Save runs the normal Save (with its file dialog for a new
  model) and continues only if the model is then saved; Cancel (or Esc) abandons the
  action. A background job still running is cancelled before the action continues.
- Scripted runs (`--script`) and assistants are not blocked by the dialog: in a
  `--script` run the program never quits on its own anyway, and an assistant's `Exit`,
  `New` or `Open` shows the same dialog to the user, who decides.

### Autosave to a separate file

- Options › General: "Autosave every N minutes", on by default, N = 5 (1–60).
- Every N minutes, if the model changed since the last save or autosave (document
  revision), a snapshot (clones of `doc.state` and `doc.settings`) is written on a
  background thread to `<config>/triceratops/autosave/<file stem>.autosave.3dm`
  (`Untitled.autosave.3dm` for a model never saved). The working file is never touched.
- A successful Save, and a clean exit, delete the current model's autosave file.
- At start, autosave files left in that folder mean a session ended badly: a dialog lists
  them (name, time) with **Open** (opens it as an unsaved model, title "<stem> (recovered)"),
  **Delete** and **Later** (keeps them for the next start).
- The existing `Autosave` command writes the same file at once.

## Testing

- Version parsing and comparison (`0.10.0 > 0.9.1`, `v` prefix, malformed tags ignored).
- Release list → offer: newer releases only, drafts/pre-releases skipped, changelog of
  several versions in order, no offer when up to date, the skipped version not offered
  automatically but offered by a manual check.
- Asset choice per platform and the fallback when missing.
- Download against a local HTTP server: progress reported, cancel stops and deletes the
  partial file, size and digest mismatch rejected.
- AppImage replace in a temp folder: the target is replaced only after verification, keeps
  mode 755, and a failed verification leaves it untouched.
- `changelog_section.py`: extracts one section, fails on a missing one.
- Options round-trip for the four new fields; the 24-hour rule; no automatic check with
  `--script` or the environment variable; the dialog opens once per version.
- Discard guard: an unmodified model quits/opens/clears at once; a modified one shows the
  dialog; Don't Save proceeds, Cancel leaves everything as it was; Save proceeds only once
  saved.
- Autosave: written after the interval only when modified, to the right path, not when
  disabled; deleted after Save; leftover files are found at start; the option round-trips.

## Left to the builder

Exact wording, dialog layout, the status-bar item's look, thread/channel structure,
Markdown rendering detail, how progress is throttled.
