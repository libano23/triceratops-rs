# Changelog

What changed in each version of Triceratops, newest first. The section of a version
becomes its release notes and is what the program shows when it offers an update:
before tagging a release, rename "Unreleased" to the version and date (see
[docs/RELEASING.md](docs/RELEASING.md)).

## Unreleased

## 0.5.0 - 2026-11-27

### New

* **Exact solids.** Box, Cylinder, Cone, Sphere and Torus (and the prisms behind WireCut, holes,
  Boss and Rib) now carry an exact B-rep from the solidcore kernel. A Boolean between two such
  solids runs on the kernel: sphere − two cylinders (+11 % off before) and torus − a bore along
  its tube (+5.9 % off before) are within 0.1 % of the true volume, and tangent or coincident
  surfaces no longer break it. Volume and area readouts of such solids come from the kernel.
  Move, Rotate, Mirror, Copy and arrays keep the exact body; a kernel refusal is reported with its
  message and leaves both objects as they were. Sphere and Torus now create a solid polysurface
  instead of a closed surface. Old files open as before. The program is about 40 MB larger.
* **Interactive previews, for almost every command.** WireCut shows the flattened wire, the cutter
  and the depth planes while you choose the cut depth. Any command that asks a distance, radius,
  angle or scale factor now accepts a click for it and shows what it would make while the cursor
  moves (PushPull, Pipe, Ribbon, Contour, Slab, ScaleNU, Twist, Maelstrom, mesh Inset/Offset and
  about forty more); a typed number still works. Questions that are not lengths in the model (page
  sizes, camera lenses, settings) stay typed.

### Fixed

* **WireCut leaves a smooth cut.** The cutter used to be the curve sampled into flat facets,
  so every cut on a solid ended in a wall of tiny planes (a mesh look). It is now the curve itself
  extruded, so the wall is one smooth face.
* A Boolean that is refused inside Boss, Rib, Shell, holes, BooleanSplit or WireCut now says why
  in the command's log instead of silently doing nothing.
* A curve with a repeated knot at the end of its domain no longer evaluates to garbage.

## 0.4.1 - 2026-11-27

### Fixed

* An AppImage kept in a folder Triceratops cannot write to is no longer left without an
  update: the new version is downloaded to Downloads for you to put in its place.
* Help › About shows the real version.
* A curve with a repeated knot at the end of its domain (what knot insertion leaves on a
  closed spline) no longer evaluates to garbage.

### Under the hood

* Groundwork for **exact solids** on a real B-rep kernel (solidcore): Booleans between solids
  made by Box, Cylinder, Cone, Sphere and Torus would give exact results where the mesh-based
  Boolean is off by up to 11 % in volume. It is switched off in this build and does not change
  anything you use today; the design and the numbers are in `docs/FEATURES.md`.

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
