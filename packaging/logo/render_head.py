"""Render the Triceratops icon: a papercraft-style low-poly triceratops head.

    blender -b --python packaging/logo/render_head.py -- packaging/logo/head.png [size]

Every surface is built from explicit low-poly sections, like a folded-paper
model, so the facets are large, planar-looking and mirror-symmetric:

- the head is a loft through elliptical 12-sided sections, from the beak tip
  to the back of the skull, triangulated with mirrored diagonals;
- the horns are lofts of pentagons along curved paths, tapering to a point;
- the frill is a radial dome with pyramid spikes on its rim.

The head faces +X. A gradient along the head keeps it solid and dissolves the
top and back of the frill into a glowing wireframe. Cycles renders it on a
transparent background. Only Blender's bundled Python is needed.
"""
import math
import sys

import bpy
from mathutils import Euler, Matrix, Vector

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
OUT = argv[0] if argv else "head.png"
SIZE = int(argv[1]) if len(argv) > 1 else 1024

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene

# fade: solid at the front, wire at the top/back of the frill
FADE_DIR = Vector((0.55, 0.0, -1.0)).normalized()
FADE_WIRE, FADE_SOLID = -2.15, -1.6


# ---- materials --------------------------------------------------------------------
def fade_factor(nt):
    """0 where the frill turns to wire, 1 on the solid head."""
    geo = nt.nodes.new("ShaderNodeNewGeometry")
    dot = nt.nodes.new("ShaderNodeVectorMath")
    dot.operation = "DOT_PRODUCT"
    dot.inputs[1].default_value = FADE_DIR
    nt.links.new(geo.outputs["Position"], dot.inputs[0])
    rng = nt.nodes.new("ShaderNodeMapRange")
    rng.interpolation_type = "SMOOTHSTEP"
    rng.inputs["From Min"].default_value = FADE_WIRE
    rng.inputs["From Max"].default_value = FADE_SOLID
    nt.links.new(dot.outputs["Value"], rng.inputs["Value"])
    return rng.outputs["Result"]


def solid(name, color, rough=0.62, coat=0.0, spec=0.35):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    nt = m.node_tree
    p = nt.nodes["Principled BSDF"]
    p.inputs["Base Color"].default_value = (*color, 1.0)
    p.inputs["Roughness"].default_value = rough
    p.inputs["Coat Weight"].default_value = coat
    p.inputs["Specular IOR Level"].default_value = spec
    mix = nt.nodes.new("ShaderNodeMixShader")
    clear = nt.nodes.new("ShaderNodeBsdfTransparent")
    nt.links.new(fade_factor(nt), mix.inputs["Fac"])
    nt.links.new(clear.outputs[0], mix.inputs[1])
    nt.links.new(p.outputs[0], mix.inputs[2])
    nt.links.new(mix.outputs[0], nt.nodes["Material Output"].inputs["Surface"])
    return m


def wire_material():
    m = bpy.data.materials.new("wire")
    m.use_nodes = True
    nt = m.node_tree
    nt.nodes.remove(nt.nodes["Principled BSDF"])
    f = fade_factor(nt)
    ramp = nt.nodes.new("ShaderNodeValToRGB")
    ramp.color_ramp.elements[0].color = (0.66, 0.50, 1.0, 1)     # violet where only wire is left
    ramp.color_ramp.elements[1].color = (1.0, 0.62, 0.45, 1)     # coral where the faces begin
    nt.links.new(f, ramp.inputs["Fac"])
    em = nt.nodes.new("ShaderNodeEmission")
    em.inputs["Strength"].default_value = 2.4
    nt.links.new(ramp.outputs["Color"], em.inputs["Color"])
    vis = nt.nodes.new("ShaderNodeMath")      # visible only where the faces fade out
    vis.operation = "SUBTRACT"
    vis.inputs[0].default_value = 1.0
    nt.links.new(f, vis.inputs[1])
    mix = nt.nodes.new("ShaderNodeMixShader")
    clear = nt.nodes.new("ShaderNodeBsdfTransparent")
    nt.links.new(vis.outputs[0], mix.inputs["Fac"])
    nt.links.new(clear.outputs[0], mix.inputs[1])
    nt.links.new(em.outputs[0], mix.inputs[2])
    nt.links.new(mix.outputs[0], nt.nodes["Material Output"].inputs["Surface"])
    return m


SKIN = solid("skin", (1.0, 0.42, 0.22))
FRILL = solid("frill", (1.0, 0.48, 0.26))
HORN = solid("horn", (0.05, 0.035, 0.03), rough=0.32, coat=0.6, spec=0.6)
BEAK = solid("beak", (0.93, 0.66, 0.20), rough=0.45, coat=0.3)
LIP = solid("lip", (0.88, 0.07, 0.16), rough=0.4, coat=0.4)
EYE = solid("eye", (0.01, 0.01, 0.012), rough=0.1, coat=1.0, spec=0.8)
WIRE = wire_material()


def make_object(name, verts, faces, mat):
    me = bpy.data.meshes.new(name)
    me.from_pydata([tuple(v) for v in verts], [], faces)
    me.validate()
    me.update()
    ob = bpy.data.objects.new(name, me)
    scene.collection.objects.link(ob)
    ob.data.materials.append(mat)
    for p in me.polygons:
        p.use_smooth = False
    return ob


def fix_normals(ob):
    bpy.context.view_layer.objects.active = ob
    bpy.ops.object.select_all(action="DESELECT")
    ob.select_set(True)
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.mesh.normals_make_consistent(inside=False)
    bpy.ops.object.mode_set(mode="OBJECT")


# ---- head: loft through 12-sided sections -------------------------------------------
HALF = [-90, -55, -20, 15, 50, 90]     # section angles on one side, bottom to top


def section(x, cz, w, h, top=1.0):
    """12 points around an ellipse: bottom, right side up to top, left side down."""
    right = []
    for a in HALF:
        r = math.radians(a)
        k = top if a > 0 else 1.0
        right.append(Vector((x, w * math.cos(r), cz + h * math.sin(r) * k)))
    left = [Vector((v.x, -v.y, v.z)) for v in reversed(right[1:-1])]
    return right + left


# (x, centre z, half width, half height, top scale) from the beak to the back
SECTIONS = [
    (2.05, -0.40, 0.12, 0.22, 1.0),    # upper beak
    (1.72, -0.24, 0.24, 0.40, 1.0),    # snout
    (1.30, -0.10, 0.33, 0.52, 1.05),   # nose horn
    (0.85, -0.02, 0.44, 0.62, 1.05),   # muzzle
    (0.40, -0.02, 0.58, 0.78, 1.0),    # eyes and brows
    (-0.05, -0.06, 0.72, 0.84, 0.92),  # cheeks and jaw
    (-0.45, 0.06, 0.58, 0.64, 0.95),   # back of the skull
]
rings = [section(*s) for s in SECTIONS]
N = len(rings[0])
verts = [v for ring in rings for v in ring]
tip = len(verts)
verts.append(Vector((2.32, 0.0, -0.58)))       # beak tip
back = len(verts)
verts.append(Vector((-0.70, 0.0, 0.06)))       # back cap centre
faces = []
for k in range(len(rings) - 1):
    for i in range(N):
        j = (i + 1) % N
        a, b = k * N + i, k * N + j                  # front section
        c, d = (k + 1) * N + i, (k + 1) * N + j      # back section
        # mirrored diagonals: the two sides of the head fold the same way
        if i < N // 2:
            faces += [(a, c, d), (a, d, b)]
        else:
            faces += [(a, c, b), (b, c, d)]
for i in range(N):
    j = (i + 1) % N
    faces.append((tip, j, i))
    faces.append((back, (len(rings) - 1) * N + i, (len(rings) - 1) * N + j))
head = make_object("head", verts, faces, SKIN)
fix_normals(head)


# ---- horns and beak: lofted polygons along a curve ----------------------------------
def horn(name, base, direction, length, radius, curl=(0, 0, 1), bend=0.0, sides=5, mat=HORN, steps=4):
    d = Vector(direction).normalized()
    up = Vector(curl)
    up = (up - d * up.dot(d)).normalized()
    verts, faces, rings = [], [], []
    for s in range(steps):
        t = s / steps
        centre = Vector(base) + d * length * t + up * bend * length * t * t
        tan = (d + up * 2 * bend * t).normalized()   # keep sections square to the path
        u = (up - tan * up.dot(tan)).normalized()
        w = tan.cross(u)
        r = radius * (1 - t) ** 0.85
        ring = []
        for k in range(sides):
            a = 2 * math.pi * (k + 0.5) / sides
            ring.append(len(verts))
            verts.append(centre + (u * math.cos(a) + w * math.sin(a)) * r)
        rings.append(ring)
    tipi = len(verts)
    verts.append(Vector(base) + d * length + up * bend * length)
    for a_ring, b_ring in zip(rings, rings[1:]):
        for k in range(sides):
            n = (k + 1) % sides
            faces.append((a_ring[k], a_ring[n], b_ring[n], b_ring[k]))
    for k in range(sides):
        faces.append((rings[-1][k], rings[-1][(k + 1) % sides], tipi))
    faces.append(tuple(reversed(rings[0])))
    ob = make_object(name, verts, faces, mat)
    ob.modifiers.new("tri", "TRIANGULATE")
    fix_normals(ob)
    return ob


horn("nose", (1.30, 0.0, 0.32), (0.45, 0.0, 1.0), 0.50, 0.17, curl=(1, 0, 0), bend=0.25)
horn("beak", (1.86, 0.0, -0.20), (1.0, 0.0, -0.45), 0.62, 0.22, curl=(0, 0, -1), bend=0.35,
     sides=4, mat=BEAK)
horn("lip", (1.62, 0.0, -0.66), (1.0, 0.0, -0.15), 0.52, 0.16, sides=4, mat=LIP)
for s in (-1, 1):
    # brow horns: long, sweeping forward and curling up at the tip
    horn(f"brow{s}", (0.42, 0.30 * s, 0.58), (1.0, 0.18 * s, 0.38), 1.85, 0.21,
         curl=(0, 0, 1), bend=0.3, sides=6)
    # small cheek horns pointing out and down
    horn(f"cheek{s}", (-0.02, 0.66 * s, -0.42), (0.1, 1.0 * s, -0.55), 0.38, 0.13,
         curl=(1, 0, 0), bend=0.1)

# ---- frill: radial dome with spikes ----------------------------------------------------
SEG, R = 14, 1.55
FRILL_M = (Matrix.Translation((-0.62, 0.0, 0.78))
           @ Euler((math.radians(90), math.radians(-34), math.radians(90))).to_matrix().to_4x4())


def frill_pt(t, r, z):
    return FRILL_M @ Vector((r * math.sin(t), r * 0.92 * math.cos(t), z))


fv, ff = [FRILL_M @ Vector((0, 0, 0.42))], []
inner = [1 + i for i in range(SEG)]
fv += [frill_pt(2 * math.pi * (i + 0.5) / SEG, 0.55 * R, 0.26) for i in range(SEG)]
outer = [1 + SEG + i for i in range(SEG)]
fv += [frill_pt(2 * math.pi * i / SEG, R, 0.0) for i in range(SEG)]
for i in range(SEG):
    j = (i + 1) % SEG
    ff += [(0, inner[i], inner[j]), (inner[i], outer[j], inner[j]), (inner[i], outer[i], outer[j])]
frill = make_object("frill", fv, ff, FRILL)
fix_normals(frill)
sol = frill.modifiers.new("solid", "SOLIDIFY")
sol.thickness, sol.offset = 0.12, 0.0

# pyramid spikes on the upper rim, pointing outwards in the frill's plane
for i in range(SEG):
    t = 2 * math.pi * i / SEG
    base = frill_pt(t, R - 0.04, 0.0)
    if base.z < 0.55:
        continue
    out = (frill_pt(t, R + 1, 0.0) - frill_pt(t, R, 0.0)).normalized()
    normal = (FRILL_M.to_3x3() @ Vector((0, 0, 1))).normalized()
    horn(f"spike{i}", base, out, 0.36, 0.13, curl=normal, sides=4)

# ---- eyes --------------------------------------------------------------------------------
for s in (-1, 1):
    bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=1, radius=0.1, location=(0.52, 0.55 * s, 0.28))
    bpy.context.active_object.data.materials.append(EYE)

# ---- wireframe copies (visible only where the faces fade out) ----------------------------
for ob in [o for o in scene.objects if o.type == "MESH"]:
    w = ob.copy()
    w.data = ob.data.copy()
    scene.collection.objects.link(w)
    w.data.materials.clear()
    w.data.materials.append(WIRE)
    for m in list(w.modifiers):
        if m.type == "SOLIDIFY":
            w.modifiers.remove(m)
    mod = w.modifiers.new("wire", "WIREFRAME")
    mod.thickness = 0.02
    mod.use_even_offset = False
    mod.use_replace = True
    w["wire"] = True

# ---- camera: 3/4 view, beak pointing left ----------------------------------------------
cam_data = bpy.data.cameras.new("cam")
cam_data.type = "ORTHO"
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
view_dir = Vector((-6.0, -7.0, -0.9)).normalized()
target = Vector((0.45, 0.0, 0.3))
cam.location = target - view_dir * 12
cam.rotation_euler = view_dir.to_track_quat("-Z", "Y").to_euler()
scene.camera = cam
bpy.context.view_layer.update()

inv = cam.matrix_world.inverted()
pts = []
deps = bpy.context.evaluated_depsgraph_get()
for ob in scene.objects:
    if ob.type == "MESH" and not ob.get("wire"):
        me = ob.evaluated_get(deps).to_mesh()
        pts += [inv @ (ob.matrix_world @ v.co) for v in me.vertices]
xs, ys = [p.x for p in pts], [p.y for p in pts]
cam.location = cam.matrix_world @ Vector(((min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2, 0))
cam_data.ortho_scale = max(max(xs) - min(xs), max(ys) - min(ys)) * 1.06
target = cam.location + view_dir * 12


def light(name, loc, energy, color, size):
    ld = bpy.data.lights.new(name, "AREA")
    ld.energy, ld.color, ld.size = energy, color, size
    ob = bpy.data.objects.new(name, ld)
    ob.location = loc
    ob.rotation_euler = (target - Vector(loc)).to_track_quat("-Z", "Y").to_euler()
    scene.collection.objects.link(ob)


light("key", (4.0, -6.0, 5.5), 700, (1.0, 0.95, 0.88), 3.0)       # warm key, front-left above
light("fill", (1.0, -5.0, -3.0), 260, (1.0, 0.35, 0.75), 4.0)     # magenta from below
light("side", (5.0, 4.0, 2.0), 250, (1.0, 0.80, 0.70), 3.0)       # soft right side
light("rim", (-5.0, 1.0, 4.0), 600, (0.80, 0.65, 1.0), 3.0)        # violet rim on the frill

world = bpy.data.worlds.new("world")
world.use_nodes = True
world.node_tree.nodes["Background"].inputs["Color"].default_value = (0.05, 0.045, 0.07, 1)
world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.6
scene.world = world

scene.render.engine = "CYCLES"
scene.cycles.samples = 128
scene.cycles.transparent_max_bounces = 32
scene.cycles.use_denoising = True
scene.cycles.device = "CPU"
scene.render.film_transparent = True
scene.render.resolution_x = scene.render.resolution_y = SIZE
scene.render.image_settings.file_format = "PNG"
scene.render.image_settings.color_mode = "RGBA"
scene.view_settings.view_transform = "Standard"
scene.render.filepath = OUT
bpy.ops.render.render(write_still=True)
print("wrote", OUT)
