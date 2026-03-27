#!/usr/bin/env python3
"""Generate test mesh sequences for seaview integration tests.

Produces two additional sequences alongside the existing cube_rotation:
  - sphere_pulse/   : expanding/contracting icosphere (30 frames)
  - twin_cube/      : two cubes moving apart (30 frames)

All files are binary STL (minimal valid format).
"""

import struct
import math
import os

def write_binary_stl(path, triangles):
    """Write a binary STL file. triangles = list of (normal, v0, v1, v2)."""
    with open(path, 'wb') as f:
        f.write(b'\x00' * 80)  # header
        f.write(struct.pack('<I', len(triangles)))
        for normal, v0, v1, v2 in triangles:
            f.write(struct.pack('<3f', *normal))
            f.write(struct.pack('<3f', *v0))
            f.write(struct.pack('<3f', *v1))
            f.write(struct.pack('<3f', *v2))
            f.write(struct.pack('<H', 0))  # attribute byte count

def cube_triangles(cx=0, cy=0, cz=0, s=1.0):
    """Return triangles for an axis-aligned cube centered at (cx,cy,cz) with half-size s."""
    verts = [
        (cx-s, cy-s, cz-s), (cx+s, cy-s, cz-s),
        (cx+s, cy+s, cz-s), (cx-s, cy+s, cz-s),
        (cx-s, cy-s, cz+s), (cx+s, cy-s, cz+s),
        (cx+s, cy+s, cz+s), (cx-s, cy+s, cz+s),
    ]
    faces = [
        (4,5,6,7,  (0,0,1)),   # +Z
        (0,3,2,1,  (0,0,-1)),  # -Z
        (1,2,6,5,  (1,0,0)),   # +X
        (0,4,7,3,  (-1,0,0)),  # -X
        (3,7,6,2,  (0,1,0)),   # +Y
        (0,1,5,4,  (0,-1,0)),  # -Y
    ]
    tris = []
    for i0,i1,i2,i3,n in faces:
        tris.append((n, verts[i0], verts[i1], verts[i2]))
        tris.append((n, verts[i0], verts[i2], verts[i3]))
    return tris

def icosphere_triangles(cx=0, cy=0, cz=0, r=1.0, subdivisions=1):
    """Return triangles for an icosphere."""
    t = (1.0 + math.sqrt(5.0)) / 2.0
    verts = [
        (-1,  t,  0), ( 1,  t,  0), (-1, -t,  0), ( 1, -t,  0),
        ( 0, -1,  t), ( 0,  1,  t), ( 0, -1, -t), ( 0,  1, -t),
        ( t,  0, -1), ( t,  0,  1), (-t,  0, -1), (-t,  0,  1),
    ]
    # Normalise to unit sphere
    verts = [(x/math.sqrt(x*x+y*y+z*z), y/math.sqrt(x*x+y*y+z*z), z/math.sqrt(x*x+y*y+z*z))
             for x,y,z in verts]
    faces = [
        (0,11,5),(0,5,1),(0,1,7),(0,7,10),(0,10,11),
        (1,5,9),(5,11,4),(11,10,2),(10,7,6),(7,1,8),
        (3,9,4),(3,4,2),(3,2,6),(3,6,8),(3,8,9),
        (4,9,5),(2,4,11),(6,2,10),(8,6,7),(9,8,1),
    ]
    for _ in range(subdivisions):
        new_faces = []
        midpoints = {}
        def mid(a, b):
            key = (min(a,b), max(a,b))
            if key not in midpoints:
                va, vb = verts[a], verts[b]
                m = ((va[0]+vb[0])/2, (va[1]+vb[1])/2, (va[2]+vb[2])/2)
                n = math.sqrt(m[0]**2+m[1]**2+m[2]**2)
                midpoints[key] = len(verts)
                verts.append((m[0]/n, m[1]/n, m[2]/n))
            return midpoints[key]
        for a,b,c in faces:
            ab,bc,ca = mid(a,b),mid(b,c),mid(c,a)
            new_faces += [(a,ab,ca),(b,bc,ab),(c,ca,bc),(ab,bc,ca)]
        faces = new_faces

    tris = []
    for a,b,c in faces:
        va = (verts[a][0]*r+cx, verts[a][1]*r+cy, verts[a][2]*r+cz)
        vb = (verts[b][0]*r+cx, verts[b][1]*r+cy, verts[b][2]*r+cz)
        vc = (verts[c][0]*r+cx, verts[c][1]*r+cy, verts[c][2]*r+cz)
        # face normal
        ab = (vb[0]-va[0], vb[1]-va[1], vb[2]-va[2])
        ac = (vc[0]-va[0], vc[1]-va[1], vc[2]-va[2])
        n = (ab[1]*ac[2]-ab[2]*ac[1], ab[2]*ac[0]-ab[0]*ac[2], ab[0]*ac[1]-ab[1]*ac[0])
        nl = math.sqrt(n[0]**2+n[1]**2+n[2]**2) or 1
        n = (n[0]/nl, n[1]/nl, n[2]/nl)
        tris.append((n, va, vb, vc))
    return tris


BASE = os.path.join(os.path.dirname(__file__), '..', 'assets', 'test_sequences')

# --- sphere_pulse: pulsing icosphere, 30 frames ---
out = os.path.join(BASE, 'sphere_pulse')
os.makedirs(out, exist_ok=True)
for i in range(30):
    r = 0.5 + 0.5 * math.sin(2 * math.pi * i / 30)
    tris = icosphere_triangles(r=max(r, 0.05), subdivisions=1)
    write_binary_stl(os.path.join(out, f'sphere_frame_{i:04d}.stl'), tris)
print(f"Generated sphere_pulse: 30 frames")

# --- twin_cube: two cubes moving apart, 30 frames ---
out = os.path.join(BASE, 'twin_cube')
os.makedirs(out, exist_ok=True)
for i in range(30):
    offset = i * 0.1
    tris = cube_triangles(cx=-1.0 - offset, s=0.4)
    tris += cube_triangles(cx=1.0 + offset, s=0.4)
    write_binary_stl(os.path.join(out, f'twin_frame_{i:04d}.stl'), tris)
print(f"Generated twin_cube: 30 frames")

print("Done.")
