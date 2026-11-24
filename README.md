# Triceratops

Triceratops is a NURBS 3D modeller for Linux written in Rust. It follows the interaction
model of professional NURBS CAD tools: a command line with prompts and clickable options,
object snaps, four synchronised viewports, a gumball, layers, and exact NURBS curves,
surfaces and polysurfaces (B-reps).

- **Modelling:** points, curves (lines, polylines, arcs, circles, ellipses, interpolated and
  control-point curves), surfaces (extrude, revolve, loft, sweep, network, edge surfaces,
  patch, blend), solids, SubD-style meshes and meshes. About 1,070 commands are registered.
- **NURBS kernel:** exact surface/surface intersections, Booleans that keep the original
  surfaces and only add trim curves, projection of curves onto any surface, trimming and
  splitting by any number of curves.
- **Interface:** dockable and floating panels and toolbars (including torn-off flyouts),
  light, dark and Blender-style themes, persistent settings and window layouts.
- **Files:** native `.3dm` through openNURBS, plus STEP, IGES, DXF/DWG, OBJ, STL, glTF and about
  twenty more formats.

## Build and run

Requirements: a recent stable Rust toolchain, CMake and a C++ compiler (for the vendored
openNURBS), and a GPU driver with Vulkan or OpenGL ES support.

```sh
cargo run --release                    # start Triceratops
cargo run --release -- model.3dm       # open a file
cargo run --release -- --script s.txt  # run command lines from a file at start-up
cargo test --workspace                 # run the test suites
```

`./triceratops.sh` does the same as `cargo run --release`: it rebuilds when the sources changed,
then starts the program with any arguments you pass (`./triceratops.sh model.3dm`,
`./triceratops.sh --script s.txt`). Pass `--no-build` first to skip the build check.

The first build compiles openNURBS and takes a few minutes. Triceratops has been tested on
Arch Linux under Wayland (niri) with an AMD GPU.

## Using it

Type a command name in the command line (`Box`, `Loft`, `BooleanDifference`, `WireCut`, ...)
or click a toolbar button. Prompts show their options in parentheses; click an option or type
its name. Enter repeats the last command. Middle-click in a viewport opens the popup toolbar.

- **Panels and toolbars:** drag a tab out of its container to float it, or use the `▾` menu to
  dock it on any side. Flyouts, such as Array or Scale, can be torn off with their `⇱` button
  or by dragging their title.
- **Settings:** osnaps, Ortho, Grid Snap, Planar, Gumball, the window layout, the theme and the
  export folders are saved automatically. Options > Appearance can save or load a layout file
  and reset everything to the defaults.
- **Themes:** `Theme` or View > Theme switches between Triceratops Light, Triceratops Dark and
  Blender. User themes are JSON files.
- **Scripts:** a script file contains one command-line entry per line. An empty line presses
  Enter. Objects are referenced as `#12`, `#12@x,y,z` (a pick at a point) or `last`.

## File formats

| Kind | Read and write | Write only |
|------|----------------|------------|
| Native | 3DM (openNURBS), TRI (JSON) | |
| CAD | STEP (AP214 B-rep), IGES (trimmed surfaces), VDA-FS, DXF, DWG | |
| Meshes | OBJ, STL, PLY, OFF, 3DS, glTF/GLB, DAE, WRL, X3D, X, AMF, 3MF, GTS, RAW, FBX (ASCII), SLC | POV-Ray, RIB |
| 2D | SVG | PDF, EPS, AI |
| Points | PTS, XYZ, ASC, CSV, TXT | |

Export proposes the working file's folder and name, remembers the last folder and file type,
and always uses the extension of the file type chosen in the dialog. Proprietary formats that
cannot be read (SKP, SAT, DGN, E57, binary FBX, ...) give a message naming an alternative.

## Configuration

| Path or variable | Purpose |
|------------------|---------|
| `~/.config/triceratops/options.json` | Settings, window layout, export memory |
| `~/.config/triceratops/themes/*.json` | User themes |
| `TRICERATOPS_BACKEND=auto\|gl\|vulkan` | Graphics backend |
| `TRICERATOPS_BOOL_DEBUG=1` | Diagnostics for Booleans, projections and trims |

## Project layout

| Crate | Contents |
|-------|----------|
| `crates/geom` (`tri-geom`) | NURBS kernel: curves, surfaces, trimmed faces and B-reps, meshes, exact Booleans (`brep_boolean_exact.rs`), surface projection and region splitting (`surface_ops.rs`) |
| `crates/doc` (`tri-doc`) | Document model, layers, undo, history, the native format, and the import/export registry in `formats/` |
| `crates/app` (`triceratops`) | The application: wgpu renderer, egui interface, docking, themes, the command system and every command |
| `crates/on3dm` | Rust bindings to the vendored openNURBS library for `.3dm` files |

`docs/FEATURES.md` describes the implemented features command by command.
`docs/ui-reference.md` and `docs/toolbars-reference.txt` record the interaction conventions
being followed. `docs/commands.json` is the command catalogue.

## Tests

`cargo test --workspace` runs:

- geometry tests, including Booleans checked for closure, volume and edge accuracy;
- format round trips for every reader and writer;
- command-flow tests that drive commands through their prompts as a user would;
- the openNURBS round trip.

## Limitations

- Trim curves from Booleans and projections are polylines in the surface parameter space,
  refined to the model tolerance; they are not analytic curves.
- Shell works on solids with planar faces only.
- FilletEdge works on planar faces only.
- SubD objects are smoothed meshes.
- Digitizer support, visual scripting, licensing and plug-in management are not available.
  Those commands say so and name the closest alternative.

## License

Triceratops is released under the MIT license. openNURBS, in `third_party/opennurbs`, is
distributed under its own license (see `third_party/opennurbs/LICENSE`).
