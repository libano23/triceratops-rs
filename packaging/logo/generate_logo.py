#!/usr/bin/env python3
"""Generate the Triceratops low-poly wireframe logo as SVG.

    python3 packaging/logo/generate_logo.py

Writes packaging/logo/logo.svg (transparent background),
packaging/logo/icon.svg (app icon on a dark rounded square) and
packaging/logo/icon-small.svg (the same, simplified for 16-48 px). The silhouette
is made of hand-placed outlines; each is filled with a Delaunay
triangulation of its outline plus a few interior points, so editing SHAPES
reshapes the mesh. Only the standard library is used.
"""
import math
import os
import random

HERE = os.path.dirname(os.path.abspath(__file__))

# Side view, facing left, in a 512 x 512 box (y grows downwards).
# Each shape is (outline, interior points); shapes are triangulated
# separately so thin parts such as horns keep all their faces.
BODY = [
    (30, 300), (60, 280),                     # beak
    (74, 268), (92, 258), (124, 244),         # snout
    (150, 226), (150, 180), (160, 140),       # frill front edge
    (176, 108), (172, 82), (200, 84),         # frill crest with spikes
    (212, 60), (232, 70), (254, 52), (264, 74),
    (294, 66), (292, 96), (324, 102), (310, 130),
    (338, 148), (316, 170), (306, 206),       # frill back edge
    (340, 190), (390, 178), (436, 186),       # back
    (470, 208), (510, 250),                   # tail
    (472, 252), (450, 262),
    (450, 320), (456, 376), (420, 378), (414, 320),   # hind leg
    (388, 312), (350, 316),                   # belly
    (330, 322), (334, 378), (298, 380), (292, 322),   # front leg
    (262, 316), (220, 320),
    (170, 328), (120, 328), (80, 320), (50, 310),     # jaw
]
BODY_INTERIOR = [
    (96, 290), (146, 292), (196, 286), (182, 236), (190, 180), (214, 130),
    (252, 104), (270, 146), (232, 214), (268, 186), (250, 262), (296, 252),
    (330, 230), (372, 220), (414, 220), (446, 236), (336, 284), (384, 272),
    (424, 290), (290, 296), (204, 252), (232, 162),
]
NOSE_HORN = [(66, 276), (58, 226), (96, 262)]
BROW_HORN = [(146, 230), (96, 192), (40, 138), (110, 178), (170, 214)]
SHAPES = [(BODY, BODY_INTERIOR), (NOSE_HORN, []), (BROW_HORN, [(118, 204)])]
EYE = (130, 262)
BBOX_Y = (52, 380)


def point_in_polygon(p, poly):
    x, y = p
    inside = False
    j = len(poly) - 1
    for i in range(len(poly)):
        xi, yi = poly[i]
        xj, yj = poly[j]
        if (yi > y) != (yj > y) and x < (xj - xi) * (y - yi) / (yj - yi) + xi:
            inside = not inside
        j = i
    return inside


def circumcircle(a, b, c):
    ax, ay = a
    bx, by = b
    cx, cy = c
    d = 2 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by))
    if abs(d) < 1e-12:
        return (0, 0), float("inf")
    ux = ((ax * ax + ay * ay) * (by - cy) + (bx * bx + by * by) * (cy - ay) + (cx * cx + cy * cy) * (ay - by)) / d
    uy = ((ax * ax + ay * ay) * (cx - bx) + (bx * bx + by * by) * (ax - cx) + (cx * cx + cy * cy) * (bx - ax)) / d
    return (ux, uy), (ax - ux) ** 2 + (ay - uy) ** 2


def delaunay(points):
    """Bowyer-Watson. Returns triangles as index triples into points."""
    pts = list(points) + [(-10000, -10000), (10000, -10000), (0, 10000)]
    n = len(points)
    tris = [(n, n + 1, n + 2)]
    for i in range(n):
        p = pts[i]
        bad = []
        for t in tris:
            centre, r2 = circumcircle(pts[t[0]], pts[t[1]], pts[t[2]])
            if (p[0] - centre[0]) ** 2 + (p[1] - centre[1]) ** 2 < r2:
                bad.append(t)
        edges = {}
        for t in bad:
            for e in ((t[0], t[1]), (t[1], t[2]), (t[2], t[0])):
                k = tuple(sorted(e))
                edges[k] = edges.get(k, 0) + 1
        tris = [t for t in tris if t not in bad]
        for (a, b), count in edges.items():
            if count == 1:
                tris.append((a, b, i))
    return [t for t in tris if max(t) < n]


def segments_cross(p1, p2, p3, p4):
    def orient(a, b, c):
        return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    d1, d2 = orient(p3, p4, p1), orient(p3, p4, p2)
    d3, d4 = orient(p1, p2, p3), orient(p1, p2, p4)
    return d1 * d2 < 0 and d3 * d4 < 0


def triangulate(outline, interior):
    points = outline + interior
    tris = []
    boundary = list(zip(outline, outline[1:] + outline[:1]))
    for t in delaunay(points):
        a, b, c = (points[i] for i in t)
        centroid = ((a[0] + b[0] + c[0]) / 3, (a[1] + b[1] + c[1]) / 3)
        if not point_in_polygon(centroid, outline):
            continue
        # drop triangles whose edges leave the silhouette at concave corners
        ok = True
        for u, v in ((a, b), (b, c), (c, a)):
            if (u, v) in boundary or (v, u) in boundary:
                continue
            mid = ((u[0] + v[0]) / 2, (u[1] + v[1]) / 2)
            if not point_in_polygon(mid, outline):
                ok = False
            for s0, s1 in boundary:
                if len({u, v, s0, s1}) == 4 and segments_cross(u, v, s0, s1):
                    ok = False
        if ok:
            tris.append((a, b, c))
    return tris, boundary


def build_mesh():
    """Returns (triangles, edges, vertices), all in absolute coordinates."""
    tris, edges = [], set()
    for outline, interior in SHAPES:
        shape_tris, boundary = triangulate(outline, interior)
        tris += shape_tris
        for a, b, c in shape_tris:
            for e in ((a, b), (b, c), (c, a)):
                edges.add(tuple(sorted(e)))
        for e in boundary:  # always draw the full silhouette
            edges.add(tuple(sorted(e)))
    vertices = sorted({p for e in edges for p in e})
    return tris, sorted(edges), vertices


def svg(tris, edges, vertices, background, stroke=3.0, dots=True):
    rng = random.Random(7)
    light = (-0.5, -0.8)
    faces = []
    for a, b, c in tris:
        # fake per-face shading: orientation of the face's longest edge
        # against a light direction, plus a little noise
        ex, ey = b[0] - a[0] + c[0] - a[0], b[1] - a[1] + c[1] - a[1]
        length = math.hypot(ex, ey) or 1
        shade = 0.5 + 0.5 * ((ex * light[0] + ey * light[1]) / length)
        shade = min(1, max(0, 0.15 + 0.6 * shade + rng.uniform(-0.12, 0.12)))
        faces.append((a, b, c, shade))

    pad = 28 if background else 8
    scale = (512 - 2 * pad) / 512
    out = ['<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512">',
           '<defs>',
           '<linearGradient id="g" x1="0" y1="0" x2="1" y2="1">'
           '<stop offset="0" stop-color="#5eead4"/><stop offset="0.55" stop-color="#38bdf8"/>'
           '<stop offset="1" stop-color="#a78bfa"/></linearGradient>',
           '<radialGradient id="bg" cx="0.3" cy="0.25" r="0.9">'
           '<stop offset="0" stop-color="#1e293b"/><stop offset="1" stop-color="#0b1120"/></radialGradient>',
           '</defs>']
    if background:
        out.append('<rect x="0" y="0" width="512" height="512" rx="112" fill="url(#bg)"/>')
    dy = (512 - (BBOX_Y[0] + BBOX_Y[1])) / 2
    out.append(f'<g transform="translate({pad} {pad + dy * scale}) scale({scale:.4f})">')
    out.append('<g>')
    for a, b, c, shade in faces:
        out.append(f'<path d="M{a[0]} {a[1]}L{b[0]} {b[1]}L{c[0]} {c[1]}Z" fill="url(#g)" fill-opacity="{0.08 + 0.32 * shade:.3f}"/>')
    out.append('</g>')
    out.append(f'<g stroke="url(#g)" stroke-width="{stroke}" stroke-linecap="round" stroke-linejoin="round" fill="none">')
    for a, b in edges:
        out.append(f'<line x1="{a[0]}" y1="{a[1]}" x2="{b[0]}" y2="{b[1]}"/>')
    out.append('</g>')
    if dots:
        # light dots on the dark icon; gradient dots on the transparent logo
        # so they read on light and dark pages alike
        out.append('<g fill="#e0f2fe">' if background else '<g fill="url(#g)">')
        for x, y in vertices:
            out.append(f'<circle cx="{x}" cy="{y}" r="5"/>')
        out.append('</g>')
    out.append(f'<circle cx="{EYE[0]}" cy="{EYE[1]}" r="9" fill="#0b1120" stroke="{'#e0f2fe' if background else '#38bdf8'}" stroke-width="3"/>')  # eye
    out.append('</g></svg>')
    return "\n".join(out)


def main():
    tris, edges, vertices = build_mesh()
    variants = {
        "logo.svg": dict(background=False),
        "icon.svg": dict(background=True),
        # 16-48 px icons: thicker lines, no vertex dots
        "icon-small.svg": dict(background=True, stroke=7.0, dots=False),
    }
    for name, opts in variants.items():
        with open(os.path.join(HERE, name), "w") as f:
            f.write(svg(tris, edges, vertices, **opts) + "\n")
    print(f"{len(tris)} triangles, {len(vertices)} vertices")


if __name__ == "__main__":
    main()
