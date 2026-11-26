# One-line command area, icon size, MicroStation key-ins — design

Date: 2026-11-26. Target: the release after 0.3.0.

## Intent

The user wants more room for the viewports and the habits of MicroStation available
when they want them, without changing how Triceratops works for anyone else:

1. the command area can shrink to a single line (just the prompt);
2. the toolbar icon size is a setting;
3. MicroStation precision key-ins work at point prompts, on top of today's syntax.

Non-goals: AccuDraw (the compass, smart locks), changing or removing any existing input
syntax (`x,y,z`, `r`/`@` relative, `w` world, `d<a` polar, distance and angle
constraints), rearranging the toolbars.

Anti-goal: anything a current user would notice without asking for it.

## Approaches (coordinates)

- **A. Always recognised, no setting — chosen.** No existing syntax contains `=`, so the
  key-ins cannot collide with it; whoever does not type them sees no difference.
- B. Behind an option, off by default: same parser, plus a switch that only guards
  against a conflict that does not exist. Rejected as unnecessary.

The other two items have one sensible way each.

## Design

### 1. Command area down to one line

`ui/command_area.rs`: the top panel's minimum size drops from 40 px to one prompt line
(the line height plus the frame margins). When the panel is too short for any history,
the history scroll area is skipped and the prompt line alone is drawn. Hovering the
prompt line then shows the last few log lines in a tooltip, so messages are not lost.
The height is remembered as today.

### 2. Icon size

A new saved option `icon_size` (px, default 24, range 16–40), with a slider in
Options › Appearance. It replaces the `ICON` constant wherever toolbar, flyout and
sidebar buttons are sized (`ui/toolbars.rs`, `ui/icons.rs` callers), so the setting
applies everywhere at once and live. Icons are drawn as vectors, so every size stays
sharp. Older settings files without the field load with 24.

### 3. MicroStation key-ins

`input.rs::parse_point_input` recognises, case-insensitively and before the existing
rules:

| Key-in | Meaning |
|--------|---------|
| `XY=x,y[,z]` | absolute point in world coordinates |
| `DL=dx,dy[,dz]` | offset from the last point, world axes |
| `DX=dx,dy[,dz]` | offset from the last point, axes of the active construction plane |
| `DI=distance,angle` | polar from the last point; the angle in degrees in the construction plane |

Each number may be a calculator expression, as elsewhere (`dl=10/3,0`). `DL`, `DX` and
`DI` need a last point and are refused without one, like `@` today, with a message in
the command history. Spaces around `=` and `,` are allowed. The command-line help text
(`ui/panels.rs`) and the Docs page list the key-ins.

## Testing

- Parser unit tests for each key-in: world vs construction plane, a rotated plane, z
  omitted, expressions, upper/lower case, refusal without a last point, and that every
  existing syntax still parses as before.
- A command-script test drawing a polyline with `XY=`, `DL=`, `DX=`, `DI=`.
- The icon size round-trips through the options file; the toolbar uses it.
- The command area renders at one-line height without panicking.

## Left to the builder

Exact tooltip content for the one-line mode, slider step, wording of the help text.
