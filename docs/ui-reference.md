# UI and interaction reference

Compiled 2026-10-08 from the public help of the reference NURBS modeller (version 8) whose interaction conventions Triceratops follows.
triceratops follows this document; when in doubt, copy Triceratops.

## Window layout (top to bottom)

1. **Title bar**: `<file name> - Triceratops` (file size optional).
2. **Menu bar**: File, Edit, View, Curve, Surface, SubD, Solid, Mesh, Dimension,
   Transform, Tools, Analyze, Render, Panels, Help.
3. **Command area**: command-line *history* (2 lines visible by default, resizable,
   right-click context menu with recent commands / Clear) and the command-line
   *prompt* (`Command:`). Prompts echo to history. Options appear in parentheses:
   `Start of line ( BothSides  Normal  Angled  Vertical  FourPoint  Bisector  Perpendicular  Tangent  Extension ):`
   Options with values show as `Copy=Yes`. Click an option, type its full name
   or its underlined letter. Enter, Space, or a right-click in a viewport confirm
   typed input. Enter with no command active repeats the last command. Esc cancels.
   Autocomplete: typing shows a dropdown with matching commands (exact prefix
   matches first, then fuzzy matches).
4. **Toolbar container** with tabs: Standard, CPlanes, Set View, Display, Select,
   Viewport Layout, Visibility, Transform, Curve Tools, Surface Tools, SubD Tools,
   Solid Tools, Mesh Tools, Render Tools, Drafting, New.
   Buttons: left click = main command, right click = secondary command; a small
   black triangle = flyout toolbar (click and hold, or click the triangle).
5. **Left sidebar** ("Main" toolbar, two columns, read row by row):
   Cancel | Point, Polyline | Curve, Circle | Ellipse, Arc | Rectangle,
   Polygon | Fillet, SrfPt | FilletSrf, Box | BooleanUnion, Project | Mesh,
   Join | Explode, Trim | Split, Group | Ungroup, EditPtOn | PointsOn,
   TextObject | Move, Copy | Rotate, Scale | Dir, Array | Align,
   FlowAlongSrf | PushPull, Check | Block, History.
   (Secondary commands and flyouts: see docs/toolbars-reference.txt.)
6. **Viewport area**: default 4View layout: Top (top-left), Perspective (top-right),
   Front (bottom-left), Right (bottom-right). Viewport title in the upper-left
   corner with a small triangle; click title = menu, double-click = maximize /
   restore, right-click = menu. Active viewport title is highlighted.
   World axes icon in the lower-left corner. Construction plane grid.
   **Viewport tabs** below the viewports: Perspective, Top, Front, Right, (+).
7. **Right container**: tabbed panels (default tabs: Properties, Layers, Display,
   Help, Notes, Rendering, Materials...). Layers panel columns: Name, Current,
   On, Lock, Color, Material, Linetype, Print Color, Print Width; toolbar:
   New Layer, New Sublayer, Delete, Change Object Layer, Copy Objects to Layer,
   Move Up, Move Down, Move Up One Parent, Filters, Columns, Tools.
   Properties panel: Object page (type, name, layer, display color, ...),
   or View page when nothing is selected (title, width, height, projection,
   camera location, target, lens length, wallpaper).
8. **Osnap control** (above the status bar): End, Near, Point, Mid, Cen, Int,
   Perp, Tan, Quad, Knot, Vertex, Project, Disable. Shift+click a box = one-shot.
   Alt held while picking suspends osnaps. Grouped with Selection Filters.
9. **Status bar** (left to right): [left sidebar toggle] CPlane/World toggle,
   x, y, z, Units (Millimeters) / Delta while picking, Layer pane (current
   layer, or layer of selection / "varies"), Grid Snap, Ortho, Planar, Osnap,
   SmartTrack, Auto CPlane, Gumball, Record History, Filter, Info,
   [right sidebar toggle]. Panes light up when on. Menu item descriptions show
   in the status bar on hover.

## Mouse

- Left click: select (clears previous selection). Shift+click add, Ctrl+click
  remove, Ctrl+Shift+click sub-object.
- Left drag from empty space: window selection left-to-right (solid rectangle,
  only objects completely inside), crossing selection right-to-left (dashed
  rectangle, anything touched). Shift adds, Ctrl removes.
- Left drag on a selected object moves it (type a distance to constrain).
- Overlapping candidates: selection menu pops up listing the candidates; the
  hovered entry highlights; `None` cancels.
- Right drag: rotate view in perspective viewports, pan in parallel viewports.
  Shift+right drag: pan. Ctrl+right drag: zoom. Ctrl+Shift+right drag: rotate
  around object. Wheel: zoom at cursor. Right click (no drag): Enter.
- Middle button: pops up the "Popup" toolbar by default.
- Double-click viewport title: maximize/restore.

## Keyboard

- F1 Help, F2 CommandHistory, F3 Properties, F4 (none), F6 Camera, F7 Grid,
  F8 Ortho, F9 Snap, F10 PointsOn, F11 PointsOff, F12 DigClick.
- Ctrl+F1 Top, Ctrl+F2 Front, Ctrl+F3 Right, Ctrl+F4 Perspective.
- Ctrl+A SelAll, Ctrl+C Copy to clipboard, Ctrl+V Paste, Ctrl+X Cut,
  Ctrl+Z Undo, Ctrl+Y Redo, Ctrl+N New, Ctrl+O Open, Ctrl+S Save,
  Ctrl+P Print, Ctrl+H Hide, Ctrl+L Lock, Ctrl+J Join, Ctrl+G Group,
  Ctrl+M MaxViewport, Ctrl+B Block, Ctrl+I Insert, Ctrl+F FindText,
  Ctrl+Tab next viewport, Delete = Delete, Esc = cancel / deselect,
  Space/Enter = Enter.
- Arrow keys rotate perspective views, Shift+arrows pan parallel views,
  PageUp/PageDown zoom, Ctrl+PageUp/Down pan perspective in/out,
  Home/End = UndoView/RedoView, Alt+arrows = nudge objects (Alt+PageUp/Down in z).
- Tab while picking a point: lock direction along first point -> cursor.
  Second Tab: circle lock.

## Coordinate input at a point prompt

- `x,y` or `x,y,z`: construction-plane coordinates of the active viewport.
- `wx,y,z`: world coordinates. `rx,y` or `@x,y`: relative to the last point.
  `wr` world relative.
- Polar: `dist<angle` (`4<45`), relative `@4<45` / `r4<45`, with z `17<45,8`.
- Distance constraint: type a number -> the marker moves around the last point
  at that distance. Negative number = incremental distances.
- Angle constraint: `<angle` -> cursor moves along a line at that angle.
- Elevator mode: Ctrl+left click -> the point is constrained to the CPlane
  normal through the picked point; type an elevation or pick.
- Point filters: `.x .y .z .xy .xz .yz` (not implemented yet).
- Command-line calculator: expressions with + - * / ( ) sin cos tan sqrt pi.

## Modeling aids

- Grid: 70 minor lines from the origin to the edge, minor spacing 1 unit,
  major line every 5 minor lines, snap spacing 1 (Small Objects - Millimeters).
- Grid snap (F9), Ortho (F8, 90 degrees; Shift toggles temporarily), Planar.
- Units: Millimeters, absolute tolerance 0.001, angle tolerance 1 degree.

## Default colours (Options > Appearance > Colors; best-effort values)

Background 140,140,140 | Major grid 129,134,140 | Minor grid 147,153,160 |
X axis 150,75,75 | Y axis 75,150,75 | Z axis 0,0,150 | Selected 255,255,0 |
Locked 100,100,100 | New layer 0,0,0 | Feedback 0,0,0 | Tracking 0,0,0 |
World axes icon X 255,0,0 Y 0,255,0 Z 0,0,255 | Command prompt black on white |
Hover colour blue. Template layers: Default (black), Layer 01 red, Layer 02 blue,
Layer 03 green, Layer 04 magenta, Layer 05 cyan.

## Display modes

Wireframe, Shaded (default in Perspective), Rendered, Ghosted, X-Ray, Technical,
Artistic, Pen, Monochrome, Arctic, Raytraced. Shaded uses layer colours for the
fill, shows isocurves and surface edges.

## Command prompts (exact wording)

- Line: `Start of line ( BothSides  Normal  Angled  Vertical  FourPoint  Bisector  Perpendicular  Tangent  Extension )`, `End of line ( BothSides )`.
- Polyline: `Start of polyline ( PersistentClose=No  Mode=Line  Helpers=No )`,
  `Next point of polyline ( PersistentClose=No  Mode=Line  Helpers=No  Length  Undo )`,
  after three points `... ( ... Close  Undo )`. Press Enter when done.
- Rectangle: `First corner of rectangle ( 3Point  Vertical  Center  Rounded )`, `Other corner or length`.
- Circle: `Center of circle ( Deformable  Vertical  2Point  3Point  Tangent  AroundCurve  FitPoints )`, `Radius <1.000> ( Diameter  Orientation  Circumference  Area )`.
- Arc: `Center of arc ( Deformable  StartPoint  Tangent  Extension )`, `Start of arc`, `End of arc or angle ( Length  Tilted )`.
  Arc StartPoint: `Start of arc`, `End of arc`, `Point on arc ( Direction  ThroughPoint  Center )`.
- Polygon: `Center of polygon ( NumSides=6  Mode=Inscribed  Vertical  AroundCurve )`, `Corner of polygon`.
- Curve: `Start of curve ( Degree=3  PersistentClose=No  SubDFriendly=No )`, `Next point ( Degree=3  PersistentClose=No  Undo )`, `Next point. Press Enter when done ( ... Close  Sharp  Undo )`.
- InterpCrv: `Start of curve ( Degree=3  Knots=SqrtChord  PersistentClose=No  StartTangent  SubDFriendly=No )`, `Next point of curve ( ... Undo )`, `... Press Enter when done ( ... Close  Sharp  EndTangent  Undo )`.
- Box: `First corner of base ( Diagonal  3Point  Vertical  Center )`, `Other corner of base or length`, `Height. Press Enter to use width`.
- Sphere: `Center of sphere ( 2Point  3Point  Tangent  AroundCurve  4Point  FitPoints )`, `Radius <1.000> ( Diameter  Orientation  Circumference  Area )`.
- Cylinder: `Base of cylinder ( DirectionConstraint=Vertical  Solid=Yes  2Point  3Point  Tangent  FitPoints )`, `Radius <1.000> ( Diameter ... )`, `End of cylinder ( DirectionConstraint=Vertical  BothSides=No  Solid=Yes )`.
- ExtrudeCrv: `Select curves to extrude`, `Extrusion distance <1.000> ( Direction  BothSides=No  Solid=Yes  DeleteInput=No  ToBoundary  SetBasePoint )`.
- Revolve: `Select curves to revolve`, `Start of revolve axis`, `End of revolve axis`, `Start angle <0> ( FullCircle  Deformable=No  DeleteInput=No )`, `Revolution angle <360>`.
- Loft: `Select curves to loft ( Point )`, `Select curves to loft. Press Enter when done`, then Loft Options dialog (Style, Closed loft, Match start/end tangent, Do not simplify / Rebuild with N control points / Refit within tolerance).
- Move: `Select objects to move`, `Point to move from ( Vertical=No )`, `Point to move to`.
- Copy: `Select objects to copy`, `Point to copy from ( Vertical=No  InPlace )`, `Point to copy to ( FromLastPoint=No  UseLastDistance=No  UseLastDirection=No )`, repeat until Enter.
- Rotate: `Select objects to rotate`, `Center of rotation ( Copy=No  UseLastCenter )`, `Angle or first reference point ( Copy=No )`, `Second reference point ( Copy=No )`.
- Scale: `Select objects to scale`, `Origin point ( Copy=No  Rigid=No )`, `Scale factor or first reference point <1.000> ( Copy=No  Rigid=No )`, `Second reference point`.
- Mirror: `Select objects to mirror`, `Start of mirror plane ( 3Point  Copy=Yes  XAxis  YAxis  ZAxis  Object )`, `End of mirror plane ( Copy=Yes )`.
- Delete: `Select objects to delete`.
