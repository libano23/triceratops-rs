"""Render the Triceratops icon: a low-poly triceratops head that fades into wireframe.

    blender -b --python packaging/logo/render_head.py -- packaging/logo/head.png [size]

The head faces +X. Ellipsoids for the skull, snout, beak, jaw and cheeks and  are
joined, voxel-remeshed into one skin and decimated to a few hundred flat
faces, like folded paper; the frill is a low-poly radial dome. Horns and frill spikes are low-poly
cones. A gradient along the head (beak = solid, frill = wire) drives the
opacity of the faces and the brightness of a wireframe copy, so the rendered
head dissolves into its mesh towards the back. Cycles renders it on a
transparent background. Only Blender's bundled Python is needed.
"""
import math
import sys

import bmesh
import bpy
from mathutils import Euler, Matrix, Vector

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
OUT = argv[0] if argv else "head.png"
SIZE = int(argv[1]) if len(argv) > 1 else 1024
FACES = 380                       # target face count of the low-poly skin

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene

# fade axis: from the frill (wire) to the beak (solid)
FADE_DIR = Vector((1.0, 0.0, -0.45)).normalized()
FADE_WIRE, FADE_SOLID = -1.05, 0.15


# ---- materials ----------------------------------------------------------------------
def fade_factor(nt):
    """Node output in [0, 1]: 0 at the back of the head (wire), 1 at the front (solid)."""
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


def solid_material(name, color, rough=0.55, coat=0.0, fade=True):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    nt = m.node_tree
    out = nt.nodes["Material Output"]
    p = nt.nodes["Principled BSDF"]
    p.inputs["Base Color"].default_value = (*color, 1.0)
    p.inputs["Roughness"].default_value = rough
    p.inputs["Coat Weight"].default_value = coat
    if not fade:
        return m
    mix = nt.nodes.new("ShaderNodeMixShader")
    clear = nt.nodes.new("ShaderNodeBsdfTransparent")
    nt.links.new(fade_factor(nt), mix.inputs["Fac"])
    nt.links.new(clear.outputs[0], mix.inputs[1])
    nt.links.new(p.outputs[0], mix.inputs[2])
    nt.links.new(mix.outputs[0], out.inputs["Surface"])
    return m


def wire_material():
    m = bpy.data.materials.new("wire")
    m.use_nodes = True
    nt = m.node_tree
    nt.nodes.remove(nt.nodes["Principled BSDF"])
    out = nt.nodes["Material Output"]
    f = fade_factor(nt)
    # colour runs from violet at the back to warm coral where the faces begin
    ramp = nt.nodes.new("ShaderNodeValToRGB")
    ramp.color_ramp.elements[0].color = (0.62, 0.45, 1.0, 1)
    ramp.color_ramp.elements[1].color = (1.0, 0.55, 0.42, 1)
    nt.links.new(f, ramp.inputs["Fac"])
    em = nt.nodes.new("ShaderNodeEmission")
    em.inputs["Strength"].default_value = 2.2
    nt.links.new(ramp.outputs["Color"], em.inputs["Color"])
    # bright where the faces have gone, faint over the solid part
    inv = nt.nodes.new("ShaderNodeMath")
    inv.operation = "MULTIPLY_ADD"
    inv.inputs[1].default_value = -0.96
    inv.inputs[2].default_value = 1.0
    nt.links.new(f, inv.inputs[0])
    mix = nt.nodes.new("ShaderNodeMixShader")
    clear = nt.nodes.new("ShaderNodeBsdfTransparent")
    nt.links.new(inv.outputs[0], mix.inputs["Fac"])
    nt.links.new(clear.outputs[0], mix.inputs[1])
    nt.links.new(em.outputs[0], mix.inputs[2])
    nt.links.new(mix.outputs[0], out.inputs["Surface"])
    return m


SKIN = solid_material("skin", (0.92, 0.33, 0.22), rough=0.7)
HORN = solid_material("horn", (0.045, 0.035, 0.035), rough=0.35, coat=0.5)
BEAK = solid_material("beak", (0.88, 0.62, 0.18), rough=0.4, coat=0.3)
LIP = solid_material("lip", (0.85, 0.08, 0.14), rough=0.35, coat=0.4)
EYE = solid_material("eye", (0.01, 0.01, 0.012), rough=0.1, coat=1.0, fade=False)
WIRE = wire_material()


def flat(ob):
    for p in ob.data.polygons:
        p.use_smooth = False


# ---- skin: ellipsoids + frill disc, remeshed and decimated ------------------------
parts = []


def ellipsoid(co, radii, rot=(0, 0, 0)):
    bpy.ops.mesh.primitive_uv_sphere_add(segments=48, ring_count=24, radius=1.0, location=co)
    ob = bpy.context.active_object
    ob.scale = radii
    ob.rotation_euler = rot
    parts.append(ob)


ellipsoid((0.0, 0.0, 0.05), (1.0, 0.62, 0.72))                  # skull
ellipsoid((0.95, 0.0, -0.16), (0.88, 0.40, 0.50), (0, 0.12, 0))   # snout
ellipsoid((1.68, 0.0, -0.34), (0.50, 0.24, 0.38), (0, 0.55, 0))   # upper beak
ellipsoid((0.55, 0.0, -0.70), (1.0, 0.42, 0.30), (0, -0.06, 0))   # lower jaw
ellipsoid((1.40, 0.0, -0.74), (0.45, 0.21, 0.22), (0, -0.35, 0))  # lower beak
for s in (-1, 1):
    ellipsoid((0.10, 0.44 * s, -0.36), (0.60, 0.32, 0.48))       # cheeks
    ellipsoid((0.48, 0.36 * s, 0.30), (0.42, 0.24, 0.20))        # brow ridges

FRILL_R = 1.45
FRILL_MATRIX = (Matrix.Translation((-0.70, 0.0, 0.80))
                @ Euler((math.radians(90), math.radians(-30), math.radians(90))).to_matrix().to_4x4())


def frill_point(t, r=FRILL_R):
    return FRILL_MATRIX @ Vector((r * math.cos(t), r * 0.92 * math.sin(t), -0.02))


bpy.ops.object.select_all(action="DESELECT")
for ob in parts:
    ob.select_set(True)
bpy.context.view_layer.objects.active = parts[0]
bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
bpy.ops.object.join()
skin = bpy.context.active_object
rem = skin.modifiers.new("remesh", "REMESH")
rem.mode = "VOXEL"
rem.voxel_size = 0.025
bpy.ops.object.modifier_apply(modifier="remesh")
sm = skin.modifiers.new("smooth", "CORRECTIVE_SMOOTH")
sm.factor, sm.iterations = 0.8, 20
bpy.ops.object.modifier_apply(modifier="smooth")
dec = skin.modifiers.new("decimate", "DECIMATE")
dec.ratio = min(1.0, FACES / max(1, len(skin.data.polygons)))
dec.use_symmetry = True
dec.symmetry_axis = "Y"
bpy.ops.object.modifier_apply(modifier="decimate")
skin.data.materials.clear()
skin.data.materials.append(SKIN)
flat(skin)


# ---- frill: a low-poly radial dome (two rings of 14 segments) ----------------------
SEG = 14
bm = bmesh.new()
centre = bm.verts.new((0, 0, 0.18))
inner = [bm.verts.new((0.55 * FRILL_R * math.cos(t), 0.55 * FRILL_R * 0.92 * math.sin(t), 0.12))
         for t in (2 * math.pi * (i + 0.5) / SEG for i in range(SEG))]
outer = [bm.verts.new((FRILL_R * math.cos(t), FRILL_R * 0.92 * math.sin(t), -0.02))
         for t in (2 * math.pi * i / SEG for i in range(SEG))]
for i in range(SEG):
    j = (i + 1) % SEG
    bm.faces.new((centre, inner[i], inner[j]))
    bm.faces.new((inner[i], outer[j], inner[j]))
    bm.faces.new((inner[i], outer[i], outer[j]))
me = bpy.data.meshes.new("frill")
bm.to_mesh(me)
frill = bpy.data.objects.new("frill", me)
scene.collection.objects.link(frill)
frill.matrix_world = FRILL_MATRIX
sol = frill.modifiers.new("solid", "SOLIDIFY")
sol.thickness = 0.12
sol.offset = 0
frill.data.materials.append(SKIN)
flat(frill)


# ---- horns, beak and frill spikes: low-poly cones -----------------------------------
def cone(base, direction, length, radius, bend=0.0, mat=HORN, sides=7, bend_to=(0, 0, 1)):
    """A cone from base along direction; the tip curls towards the world direction bend_to."""
    bpy.ops.mesh.primitive_cone_add(vertices=sides, radius1=radius, radius2=0.0, depth=length,
                                    end_fill_type="NGON", location=(0, 0, 0))
    ob = bpy.context.active_object
    d = Vector(direction).normalized()
    rot = Vector((0, 0, 1)).rotation_difference(d)
    curl = rot.inverted() @ Vector(bend_to)
    curl.z = 0                       # bend sideways in the cone's own frame
    for v in ob.data.vertices:       # base at z=0, tip at z=length, then curve the tip
        v.co.z += length / 2
        t = v.co.z / length
        v.co += curl * bend * length * t * t
    ob.data.update()
    ob.rotation_mode = "QUATERNION"
    ob.rotation_quaternion = rot
    ob.location = base
    ob.data.materials.append(mat)
    flat(ob)
    return ob


cone((1.22, 0.0, 0.16), (0.55, 0.0, 1.0), 0.45, 0.17, 0.2, sides=6, bend_to=(1, 0, 0))  # nose horn
cone((1.80, 0.0, -0.28), (1.0, 0.0, -0.75), 0.75, 0.30, 0.45, BEAK, 6, bend_to=(0, 0, -1))  # upper beak
cone((1.55, 0.0, -0.80), (1.0, 0.0, 0.2), 0.42, 0.18, 0.0, LIP, 6)        # lower beak
for s in (-1, 1):
    # long brow horns that sweep forward and curl up at the tip
    cone((0.40, 0.33 * s, 0.42), (1.0, 0.16 * s, 0.12), 2.0, 0.22, 0.4, sides=8)
    cone((0.12, 0.72 * s, -0.62), (0.15, 0.9 * s, -0.45), 0.34, 0.12, sides=5)  # cheek horns

# spikes (epoccipitals) around the top of the frill, pointing outwards
for i in range(SEG):
    t = 2 * math.pi * i / SEG
    base = frill_point(t, FRILL_R - 0.06)
    if base.z < 0.35:
        continue
    out = (frill_point(t, FRILL_R + 1) - frill_point(t, FRILL_R)).normalized()
    cone(base, out, 0.32, 0.12, sides=5)

# ---- eyes ---------------------------------------------------------------------------
for s in (-1, 1):
    bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=1, radius=0.12, location=(0.68, 0.50 * s, 0.14))
    e = bpy.context.active_object
    e.data.materials.append(EYE)
    flat(e)

# ---- wireframe copy of everything that fades --------------------------------------------
for ob in [o for o in scene.objects if o.type == "MESH" and o.data.materials and o.data.materials[0] != EYE]:
    w = ob.copy()
    w.data = ob.data.copy()
    scene.collection.objects.link(w)
    w.data.materials.clear()
    w.data.materials.append(WIRE)
    mod = w.modifiers.new("wire", "WIREFRAME")
    mod.thickness = 0.022
    mod.use_even_offset = False   # decimated faces can be degenerate; even offset blows them up
    mod.use_replace = True
    w["wire"] = True

# ---- camera: 3/4 view from front-right, so the beak points left ----------------------
cam_data = bpy.data.cameras.new("cam")
cam_data.type = "ORTHO"
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
view_dir = Vector((-7.0, -6.2, -1.6)).normalized()
target = Vector((0.45, 0.0, 0.3))
cam.location = target - view_dir * 12
cam.rotation_euler = view_dir.to_track_quat("-Z", "Y").to_euler()
scene.camera = cam
bpy.context.view_layer.update()

# fit the orthographic frame to everything that renders, with a small margin
inv = cam.matrix_world.inverted()
pts = []
deps = bpy.context.evaluated_depsgraph_get()
for ob in scene.objects:
    if ob.type == "MESH" and not ob.get("wire"):
        me = ob.evaluated_get(deps).to_mesh()
        pts += [inv @ (ob.matrix_world @ v.co) for v in me.vertices]
xs, ys = [p.x for p in pts], [p.y for p in pts]
cx, cy = (min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2
cam.location = cam.matrix_world @ Vector((cx, cy, 0))
cam_data.ortho_scale = max(max(xs) - min(xs), max(ys) - min(ys)) * 1.06
target = cam.location + view_dir * 12


def light(name, loc, energy, color, size):
    ld = bpy.data.lights.new(name, "AREA")
    ld.energy, ld.color, ld.size = energy, color, size
    ob = bpy.data.objects.new(name, ld)
    ob.location = loc
    ob.rotation_euler = (target - Vector(loc)).to_track_quat("-Z", "Y").to_euler()
    scene.collection.objects.link(ob)


light("key", (5.0, 3.0, 6.0), 800, (1.0, 0.94, 0.86), 3.0)        # warm, from above front
light("fill", (3.0, 7.0, -1.0), 260, (0.95, 0.45, 0.85), 5.0)      # magenta shadows
light("rim", (-5.0, 2.0, 4.0), 500, (0.75, 0.6, 1.0), 3.0)           # violet rim on the frill

world = bpy.data.worlds.new("world")
world.use_nodes = True
world.node_tree.nodes["Background"].inputs["Color"].default_value = (0.05, 0.04, 0.07, 1)
world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.5
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
scene.view_settings.view_transform = "AgX"
scene.view_settings.look = "AgX - Punchy"
scene.render.filepath = OUT
bpy.ops.render.render(write_still=True)
print("wrote", OUT)
