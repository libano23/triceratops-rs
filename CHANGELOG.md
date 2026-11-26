# Changelog

What changed in each version of Triceratops, newest first. The section of a version
becomes its release notes and is what the program shows when it offers an update:
before tagging a release, rename "Unreleased" to the version and date (see
[docs/RELEASING.md](docs/RELEASING.md)).

## Unreleased

### Fixed

* An AppImage kept in a folder Triceratops cannot write to is no longer left without an
  update: the new version is downloaded to Downloads for you to put in its place.
* Help › About shows the real version.

## 0.4.0 - 2026-11-26

### New

* **Updates:** Triceratops checks once a day for a newer version and shows what changed.
  The AppImage updates itself and restarts; on Windows the new zip is downloaded and
  checked for you. Help › Check for Updates… checks at any time; Options › General
  switches the automatic check off.
* **Save before closing:** closing the window, Exit, New and Open now ask whether to save
  unsaved changes.
* **Autosave:** every 5 minutes (Options › General) a modified model is saved to its own
  file in the settings folder, never over yours. After a crash, Triceratops offers to
  recover it at the next start.
* **MicroStation key-ins** at point prompts: `XY=x,y,z` (absolute), `DL=dx,dy,dz` (from the
  last point), `DX=dx,dy,dz` (along the construction plane) and `DI=distance,angle`.
* **Icon size:** a slider in Options › Appearance, from 16 to 40 pixels.
* The command area can be shrunk to a single line; hover the prompt to see the last
  messages.

### Changed

* macOS builds are no longer made (0.3.0 was the last one).
* Opening folders and web pages from Triceratops works on Windows.

## 0.3.0 - 2026-11-26

### New

* **Locust through AI assistants:** an assistant can build parametric Locust definitions
  for you, or model directly — you choose.
* **Script and Expression nodes** in Locust (Rhai code and formulas).
* **Examples:** an M10 screw and nut, an involute gear, a surface from an equation,
  Voronoi on areas and surfaces, a dovetail joint.
* **Long operations run in the background:** Booleans, solves and history updates show
  their progress in the status bar and stop with Esc.
* A new logo, and a Docs page on the website.

### Fixed

* Booleans of swept threads (screw with its core, nut minus screw) work.
* A Boolean of dense meshes no longer runs out of memory.
* Long lofts are no longer taken for closed solids; Volume and Area measure what is shown.
