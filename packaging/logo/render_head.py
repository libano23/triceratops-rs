"""Render the Triceratops icon: a 3D triceratops head, built procedurally.

    blender -b --python packaging/logo/render_head.py -- packaging/logo/head.png [size]

The head faces +X: ellipsoids for the skull, snout, beak, jaw and cheeks and
a scalloped disc for the frill are joined and voxel-remeshed into one smooth
skin; the horns are bent cones. Cycles renders it on a transparent background with a key light and
teal / violet rim lights. Only Blender's bundled Python is needed.
"""
import math
import sys

import bmesh
import bpy
from mathutils import Euler, Vector

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
OUT = argv[0] if argv else "head.png"
SIZE = int(argv[1]) if len(argv) > 1 else 1024

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene


def material(name, color, rough=0.38, metal=0.15, coat=0.4):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    p = m.node_tree.nodes["Principled BSDF"]
    p.inputs["Base Color"].default_value = (*color, 1.0)
    p.inputs["Roughness"].default_value = rough
    p.inputs["Metallic"].default_value = metal
    p.inputs["Coat Weight"].default_value = coat
    p.inputs["Coat Roughness"].default_value = 0.2
    return m


SKIN = material("skin", (0.22, 0.40, 0.46))
BEAK = material("beak", (0.16, 0.17, 0.19), rough=0.3, metal=0.0, coat=0.6)
HORN = material("horn", (0.86, 0.84, 0.78), rough=0.3, metal=0.0, coat=0.6)
EYE = material("eye", (0.01, 0.012, 0.02), rough=0.08, metal=0.0, coat=1.0)


def link(obj, mat):
    obj.data.materials.append(mat)
    scene.collection.objects.link(obj) if obj.name not in scene.collection.objects else None
    return obj


# ---- skull, snout, beak, jaw and frill: one remeshed skin -----------------------
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
ellipsoid((1.48, 0.0, -0.76), (0.45, 0.21, 0.22), (0, -0.35, 0))  # lower beak
for s in (-1, 1):
    ellipsoid((0.10, 0.42 * s, -0.36), (0.60, 0.30, 0.48))       # cheeks
    ellipsoid((0.48, 0.34 * s, 0.30), (0.42, 0.22, 0.18))        # brow ridges


def frill():
    bm = bmesh.new()
    n, R = 160, 1.3
    ring = []
    for i in range(n):
        t = 2 * math.pi * i / n
        # rounded scallops (epoccipitals) along the edge
        r = R * (1 + 0.12 * max(0.0, math.cos(10 * t)) ** 2)
        ring.append(bm.verts.new((r * math.cos(t), r * 0.9 * math.sin(t), 0)))
    bm.faces.new(ring)
    me = bpy.data.meshes.new("frill")
    bm.to_mesh(me)
    ob = bpy.data.objects.new("frill", me)
    scene.collection.objects.link(ob)
    sol = ob.modifiers.new("solid", "SOLIDIFY")
    sol.thickness = 0.2
    sol.offset = 0
    # stand the disc up behind the skull, its face turned forward and up
    ob.rotation_euler = (math.radians(90), math.radians(-32), math.radians(90))
    ob.location = (-0.72, 0.0, 0.78)
    bpy.context.view_layer.objects.active = ob
    ob.select_set(True)
    bpy.ops.object.modifier_apply(modifier="solid")
    parts.append(ob)


frill()

bpy.ops.object.select_all(action="DESELECT")
for ob in parts:
    ob.select_set(True)
bpy.context.view_layer.objects.active = parts[0]
bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)
bpy.ops.object.join()
head = bpy.context.active_object
rem = head.modifiers.new("remesh", "REMESH")
rem.mode = "VOXEL"
rem.voxel_size = 0.02
smooth = head.modifiers.new("smooth", "CORRECTIVE_SMOOTH")
smooth.factor = 0.8
smooth.iterations = 30
smooth.smooth_type = "LENGTH_WEIGHTED"
for p in head.data.polygons:
    p.use_smooth = True
head.data.materials.clear()
head.data.materials.append(SKIN)
bpy.context.view_layer.objects.active = head
bpy.ops.object.modifier_apply(modifier="remesh")
bpy.ops.object.modifier_apply(modifier="smooth")
sub = head.modifiers.new("sub", "SUBSURF")
sub.levels = sub.render_levels = 1
bpy.ops.object.shade_smooth()


# ---- horns: bent cones --------------------------------------------------------
def horn(base, direction, length, radius, bend, mat=None):
    bpy.ops.mesh.primitive_cone_add(vertices=48, radius1=radius, radius2=0.0, depth=length,
                                    end_fill_type="NOTHING", location=(0, 0, 0))
    ob = bpy.context.active_object
    me = ob.data
    for v in me.vertices:            # base at z=0, tip at z=length, then bend
        v.co.z += length / 2
        t = v.co.z / length
        v.co.x += bend * length * t * t
    me.update()
    for p in me.polygons:
        p.use_smooth = True
    d = Vector(direction).normalized()
    ob.rotation_mode = "QUATERNION"
    ob.rotation_quaternion = Vector((0, 0, 1)).rotation_difference(d)
    ob.location = base
    sub = ob.modifiers.new("sub", "SUBSURF")
    sub.levels = sub.render_levels = 1
    ob.data.materials.append(mat or HORN)
    return ob


horn((1.22, 0.0, 0.16), (0.35, 0.0, 1.0), 0.48, 0.16, -0.3)             # nose horn
horn((1.88, 0.0, -0.36), (1.0, 0.0, -0.9), 0.5, 0.21, 0.45, BEAK)     # hooked beak
for s in (-1, 1):
    horn((0.42, 0.33 * s, 0.42), (1.0, 0.13 * s, 0.62), 1.75, 0.18, -0.2)   # brow horns
    horn((0.12, 0.70 * s, -0.60), (0.15, 0.9 * s, -0.45), 0.32, 0.11, 0.0)  # cheek horns

# ---- eyes ---------------------------------------------------------------------
for s in (-1, 1):
    bpy.ops.mesh.primitive_uv_sphere_add(segments=32, ring_count=16, radius=0.11, location=(0.66, 0.50 * s, 0.13))
    e = bpy.context.active_object
    bpy.ops.object.shade_smooth()
    e.data.materials.append(EYE)

# ---- camera and lights ----------------------------------------------------------
cam_data = bpy.data.cameras.new("cam")
cam_data.type = "ORTHO"
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
view_dir = Vector((-7.6, 5.6, -2.4)).normalized()   # from front-left, a little above
target = Vector((0.45, 0.0, 0.2))
cam.location = target - view_dir * 12
cam.rotation_euler = view_dir.to_track_quat("-Z", "Y").to_euler()
scene.camera = cam
bpy.context.view_layer.update()

# fit the orthographic frame to everything that renders, with a small margin
inv = cam.matrix_world.inverted()
pts = []
deps = bpy.context.evaluated_depsgraph_get()
for ob in scene.objects:
    if ob.type == "MESH":
        me = ob.evaluated_get(deps).to_mesh()
        pts += [inv @ (ob.matrix_world @ v.co) for v in me.vertices]
xs, ys = [p.x for p in pts], [p.y for p in pts]
cx, cy = (min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2
cam.location = cam.matrix_world @ Vector((cx, cy, 0))
cam_data.ortho_scale = max(max(xs) - min(xs), max(ys) - min(ys)) * 1.08
target = cam.location + view_dir * 12


def light(name, kind, loc, energy, color, size=2.0):
    ld = bpy.data.lights.new(name, kind)
    ld.energy = energy
    ld.color = color
    if kind == "AREA":
        ld.size = size
    ob = bpy.data.objects.new(name, ld)
    ob.location = loc
    ob.rotation_euler = (target - Vector(loc)).to_track_quat("-Z", "Y").to_euler()
    scene.collection.objects.link(ob)


light("key", "AREA", (4.0, -5.0, 6.0), 450, (1.0, 0.97, 0.92), 4.0)
light("fill", "AREA", (-2.0, -6.0, 1.0), 120, (0.75, 0.85, 1.0), 5.0)
light("rim_teal", "AREA", (-4.0, 3.0, 3.0), 700, (0.37, 0.92, 0.83), 3.0)
light("rim_violet", "AREA", (3.0, 5.0, -1.0), 500, (0.65, 0.55, 0.98), 3.0)

world = bpy.data.worlds.new("world")
world.use_nodes = True
world.node_tree.nodes["Background"].inputs["Color"].default_value = (0.03, 0.04, 0.06, 1)
world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.6
scene.world = world

scene.render.engine = "CYCLES"
scene.cycles.samples = 96
scene.cycles.use_denoising = True
scene.cycles.device = "CPU"
scene.render.film_transparent = True
scene.render.resolution_x = scene.render.resolution_y = SIZE
scene.render.image_settings.file_format = "PNG"
scene.render.image_settings.color_mode = "RGBA"
scene.view_settings.view_transform = "AgX"
scene.render.filepath = OUT
bpy.ops.render.render(write_still=True)
print("wrote", OUT)
