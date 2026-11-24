#include <functional>
#include <cstdlib>
#include <cstdio>
// C shim over openNURBS for the on3dm crate.
#include "opennurbs_public.h"
#include <vector>
#include <string>
#include <cstring>
#include <map>
#include <cmath>

extern "C" {

struct On3dmObject {
    int kind = 0; // 0 unsupported, 1 point, 2 curve, 3 surface, 4 brep, 5 mesh
    int layer = 0;
    std::string name;
    int has_color = 0;
    unsigned color = 0;
    int hidden = 0;
    int locked = 0;
    // point
    double point[3] = {0, 0, 0};
    // curve / surface (surface uses the *_u / *_v fields)
    int rational = 0;
    int order_u = 0, order_v = 0, cvc_u = 0, cvc_v = 0;
    std::vector<double> knots_u, knots_v, cv; // cv: 4 doubles per point (x,y,z,w) Euclidean
    // brep
    struct Face {
        int rational = 0;
        int order_u = 0, order_v = 0, cvc_u = 0, cvc_v = 0;
        std::vector<double> knots_u, knots_v, cv;
        std::vector<std::vector<double>> loops; // uv pairs, outer first
        std::vector<int> loop_outer;
    };
    std::vector<Face> faces;
    int solid = 0;
    // mesh
    std::vector<double> verts;
    std::vector<int> tris;
    // identity / block membership
    std::string uuid;
    std::string name_of_idef; // uuid of the referenced definition (kind 6)
    int idef_member = 0;
    // instance reference (kind 6)
    int idef_index = -1;
    double xform[16] = {1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1};
};

struct On3dmIdef {
    std::string name;
    std::string uuid;
    std::vector<std::string> member_uuids;
    std::vector<int> members; // object indices, resolved after reading
};

struct On3dmLayer {
    int index = 0;
    std::string name;
    unsigned color = 0;
    int visible = 1;
    int locked = 0;
};

struct On3dm {
    ONX_Model model;
    std::vector<On3dmLayer> layers;
    std::vector<On3dmObject> objects;
    std::vector<On3dmIdef> idefs;
    std::string notes;
    ON_Brep* pending_brep = nullptr;
    // writing: block definition being collected
    int idef_mode = 0;
    ON_SimpleArray<ON_UUID> idef_ids;
    std::vector<ON_UUID> idef_uuids;
};

static std::string uuid_str(const ON_UUID& id) {
    char buf[64];
    ON_UuidToString(id, buf);
    return std::string(buf);
}

static std::string to_utf8(const ON_wString& ws) {
    ON_String s(ws);
    const char* a = s.Array();
    return a ? std::string(a) : std::string();
}

static void fill_nurbs_surface(const ON_NurbsSurface& ns, int& rational, int& ou, int& ov, int& cu, int& cvv, std::vector<double>& ku, std::vector<double>& kv, std::vector<double>& cv) {
    rational = ns.IsRational() ? 1 : 0;
    ou = ns.Order(0);
    ov = ns.Order(1);
    cu = ns.CVCount(0);
    cvv = ns.CVCount(1);
    ku.assign(ns.Knot(0), ns.Knot(0) + ns.KnotCount(0));
    kv.assign(ns.Knot(1), ns.Knot(1) + ns.KnotCount(1));
    cv.resize((size_t)cu * cvv * 4);
    for (int i = 0; i < cu; i++) {
        for (int j = 0; j < cvv; j++) {
            ON_4dPoint p;
            ns.GetCV(i, j, ON::point_style::homogeneous_rational, &p.x);
            double w = ns.IsRational() ? p.w : 1.0;
            double* o = &cv[((size_t)i * cvv + j) * 4];
            if (ns.IsRational() && w != 0.0) {
                o[0] = p.x / w; o[1] = p.y / w; o[2] = p.z / w; o[3] = w;
            } else {
                ON_3dPoint q;
                ns.GetCV(i, j, q);
                o[0] = q.x; o[1] = q.y; o[2] = q.z; o[3] = 1.0;
            }
        }
    }
}

static void fill_nurbs_curve(const ON_NurbsCurve& nc, On3dmObject& o) {
    o.rational = nc.IsRational() ? 1 : 0;
    o.order_u = nc.Order();
    o.cvc_u = nc.CVCount();
    o.knots_u.assign(nc.Knot(), nc.Knot() + nc.KnotCount());
    o.cv.resize((size_t)o.cvc_u * 4);
    for (int i = 0; i < o.cvc_u; i++) {
        double* d = &o.cv[(size_t)i * 4];
        if (nc.IsRational()) {
            ON_4dPoint p;
            nc.GetCV(i, p);
            double w = p.w != 0.0 ? p.w : 1.0;
            d[0] = p.x / w; d[1] = p.y / w; d[2] = p.z / w; d[3] = w;
        } else {
            ON_3dPoint p;
            nc.GetCV(i, p);
            d[0] = p.x; d[1] = p.y; d[2] = p.z; d[3] = 1.0;
        }
    }
}

static void fill_brep(const ON_Brep& brep, On3dmObject& o) {
    o.kind = 4;
    o.solid = brep.IsSolid() ? 1 : 0;
    // loop sampling accuracy: well inside the 2e-3 * diagonal used to match edges
    double chord_tol = ON_Max(1e-6, brep.BoundingBox().Diagonal().Length() * 1e-4);
    for (int fi = 0; fi < brep.m_F.Count(); fi++) {
        const ON_BrepFace& face = brep.m_F[fi];
        const ON_Surface* srf = face.SurfaceOf();
        if (!srf) continue;
        ON_NurbsSurface ns;
        int rc = srf->GetNurbForm(ns);
        if (rc == 0) continue;
        On3dmObject::Face f;
        fill_nurbs_surface(ns, f.rational, f.order_u, f.order_v, f.cvc_u, f.cvc_v, f.knots_u, f.knots_v, f.cv);
        // trim loops sampled in parameter space; when the NURBS form has a different
        // parameterisation (rc == 2, e.g. ON_RevSurface) the samples are mapped onto it
        bool untrimmed_rect = (face.LoopCount() == 1);
        for (int li = 0; li < face.LoopCount(); li++) {
            const ON_BrepLoop* loop = face.Loop(li);
            if (!loop) continue;
            std::vector<double> pts;
            for (int ti = 0; ti < loop->TrimCount(); ti++) {
                const ON_BrepTrim* trim = loop->Trim(ti);
                if (!trim) continue;
                const ON_Curve* c2 = trim->TrimCurveOf();
                ON_Interval dom = trim->Domain();
                int n = 2;
                if (c2 && !c2->IsLinear()) {
                    n = ON_Max(8, trim->SpanCount() * 8);
                }
                // Triceratops loops are polylines whose vertices are joined by straight UV
                // segments: subdivide until each segment stays within chord_tol of the true
                // edge in 3-D (a straight trim along a cylinder is still a curved edge)
                auto push_uv = [&](const ON_3dPoint& p) {
                    double nu = p.x, nv = p.y;
                    if (rc == 2 && !srf->GetNurbFormParameterFromSurfaceParameter(p.x, p.y, &nu, &nv)) { nu = p.x; nv = p.y; }
                    pts.push_back(nu);
                    pts.push_back(nv);
                };
                auto at3d = [&](double t) { ON_3dPoint uv = trim->PointAt(t); return srf->PointAt(uv.x, uv.y); };
                std::function<void(double, double, const ON_3dPoint&, const ON_3dPoint&, int)> refine =
                    [&](double t0, double t1, const ON_3dPoint& p0, const ON_3dPoint& p1, int depth) {
                        // emits the samples strictly between t0 and t1
                        if (depth >= 14) return;
                        ON_Line chord(p0, p1);
                        double worst = 0.0;
                        for (double f : {0.25, 0.5, 0.75}) {
                            ON_3dPoint q = at3d(t0 + (t1 - t0) * f);
                            double ct;
                            double d = chord.ClosestPointTo(q, &ct) ? q.DistanceTo(chord.PointAt(ON_Max(0.0, ON_Min(1.0, ct)))) : q.DistanceTo(p0);
                            worst = ON_Max(worst, d);
                        }
                        if (worst <= chord_tol) return;
                        double tm = 0.5 * (t0 + t1);
                        ON_3dPoint pm = at3d(tm);
                        refine(t0, tm, p0, pm, depth + 1);
                        push_uv(trim->PointAt(tm));
                        refine(tm, t1, pm, p1, depth + 1);
                    };
                for (int k = 0; k < n; k++) {
                    double t0 = dom.ParameterAt((double)k / (double)n);
                    double t1 = dom.ParameterAt((double)(k + 1) / (double)n);
                    push_uv(trim->PointAt(t0));
                    refine(t0, t1, at3d(t0), at3d(t1), 0);
                }
            }
            if (loop->m_type != ON_BrepLoop::outer) untrimmed_rect = false;
            if (getenv("TRICERATOPS_3DM_DEBUG") && atoi(getenv("TRICERATOPS_3DM_DEBUG")) >= 2 && pts.size() >= 6) {
                // compare the trim (through the surface) with the 3-D edge it belongs to
                double worst = 0.0;
                for (int ti = 0; ti < loop->TrimCount(); ti++) {
                    const ON_BrepTrim* trim = loop->Trim(ti);
                    const ON_BrepEdge* edge = trim ? trim->Edge() : nullptr;
                    if (!edge) continue;
                    for (int k = 0; k <= 4; k++) {
                        ON_3dPoint uv = trim->PointAt(trim->Domain().ParameterAt(k / 4.0));
                        ON_3dPoint p3 = srf->PointAt(uv.x, uv.y);
                        double best = 1e300;
                        ON_Interval ed = edge->Domain();
                        for (int m = 0; m <= 256; m++) best = ON_Min(best, p3.DistanceTo(edge->PointAt(ed.ParameterAt(m / 256.0))));
                        worst = ON_Max(worst, best);
                    }
                }
                ON_3dPoint q = ns.PointAt(pts[0], pts[1]);
                fprintf(stderr, "  face %d loop %d: first uv (%.4f,%.4f) -> (%.4f,%.4f,%.4f), trim-to-edge max %.5f, srf %s, nurb rc %d\n", fi, li, pts[0], pts[1], q.x, q.y, q.z, worst, srf->ClassId()->ClassName(), rc);
            }
            f.loops.push_back(pts);
            f.loop_outer.push_back(loop->m_type == ON_BrepLoop::outer ? 1 : 0);
        }
        if (untrimmed_rect) {
            // an outer loop that is the surface boundary rectangle: treat as untrimmed
            ON_BoundingBox bb;
            const std::vector<double>& pts = f.loops[0];
            double u0 = ns.Domain(0)[0], u1 = ns.Domain(0)[1], v0 = ns.Domain(1)[0], v1 = ns.Domain(1)[1];
            bool on_border = true;
            double tol = 1e-6 * (1.0 + fabs(u1 - u0) + fabs(v1 - v0));
            for (size_t k = 0; k + 1 < pts.size(); k += 2) {
                double u = pts[k], v = pts[k + 1];
                bool border = fabs(u - u0) < tol || fabs(u - u1) < tol || fabs(v - v0) < tol || fabs(v - v1) < tol;
                if (!border) { on_border = false; break; }
            }
            if (on_border) { f.loops.clear(); f.loop_outer.clear(); }
        }
        if (face.m_bRev) {
            // reversed face: swap u and v so the normal flips (loops are swapped too)
            On3dmObject::Face g = f;
            g.order_u = f.order_v; g.order_v = f.order_u; g.cvc_u = f.cvc_v; g.cvc_v = f.cvc_u;
            g.knots_u = f.knots_v; g.knots_v = f.knots_u;
            g.cv.resize(f.cv.size());
            for (int i = 0; i < f.cvc_u; i++) for (int j = 0; j < f.cvc_v; j++)
                for (int c = 0; c < 4; c++) g.cv[((size_t)j * f.cvc_u + i) * 4 + c] = f.cv[((size_t)i * f.cvc_v + j) * 4 + c];
            for (auto& lp : g.loops) {
                std::vector<double> sw;
                for (size_t k = 0; k + 1 < lp.size(); k += 2) { sw.push_back(lp[k + 1]); sw.push_back(lp[k]); }
                // reverse order to keep orientation conventions
                std::vector<double> rev;
                for (size_t k = sw.size(); k >= 2; k -= 2) { rev.push_back(sw[k - 2]); rev.push_back(sw[k - 1]); }
                lp = rev;
            }
            f = g;
        }
        o.faces.push_back(f);
    }
}

static void fill_mesh(const ON_Mesh& m, On3dmObject& o) {
    o.kind = 5;
    int vc = m.VertexCount();
    o.verts.resize((size_t)vc * 3);
    for (int i = 0; i < vc; i++) {
        ON_3dPoint p = m.Vertex(i);
        o.verts[(size_t)i * 3] = p.x; o.verts[(size_t)i * 3 + 1] = p.y; o.verts[(size_t)i * 3 + 2] = p.z;
    }
    for (int i = 0; i < m.m_F.Count(); i++) {
        const ON_MeshFace& f = m.m_F[i];
        o.tris.push_back(f.vi[0]); o.tris.push_back(f.vi[1]); o.tris.push_back(f.vi[2]);
        if (f.IsQuad()) { o.tris.push_back(f.vi[0]); o.tris.push_back(f.vi[2]); o.tris.push_back(f.vi[3]); }
    }
}

static void convert_geometry(const ON_Geometry* g, On3dmObject& o) {
    if (!g) return;
    if (getenv("TRICERATOPS_3DM_DEBUG")) {
        const ON_ClassId* id = g->ClassId();
        const ON_Brep* db = ON_Brep::Cast(g);
        const ON_Extrusion* de = ON_Extrusion::Cast(g);
        fprintf(stderr, "3dm object: %s%s%s\n", id ? id->ClassName() : "?", db ? (db->IsSolid() ? " (solid)" : " (open)") : "",
                de ? (de->IsSolid() ? " (capped solid)" : de->IsCapped() ? " (capped)" : " (uncapped)") : "");
    }
    if (const ON_Point* p = ON_Point::Cast(g)) {
        o.kind = 1; o.point[0] = p->point.x; o.point[1] = p->point.y; o.point[2] = p->point.z; return;
    }
    if (const ON_Curve* c = ON_Curve::Cast(g)) {
        ON_NurbsCurve* nc = c->NurbsCurve();
        if (nc) { o.kind = 2; fill_nurbs_curve(*nc, o); delete nc; }
        return;
    }
    // ON_Extrusion derives from ON_Surface: test it first, or a capped extrusion (solid)
    // would be read as its bare side wall
    if (const ON_Extrusion* e = ON_Extrusion::Cast(g)) {
        ON_Brep* b = e->BrepForm(nullptr);
        if (b) { fill_brep(*b, o); delete b; }
        return;
    }
    if (const ON_Surface* s = ON_Surface::Cast(g)) {
        ON_NurbsSurface* ns = s->NurbsSurface();
        if (ns) { o.kind = 3; fill_nurbs_surface(*ns, o.rational, o.order_u, o.order_v, o.cvc_u, o.cvc_v, o.knots_u, o.knots_v, o.cv); delete ns; }
        return;
    }
    if (const ON_Brep* b = ON_Brep::Cast(g)) { fill_brep(*b, o); return; }
    if (const ON_Mesh* m = ON_Mesh::Cast(g)) { fill_mesh(*m, o); return; }
    if (const ON_InstanceRef* r = ON_InstanceRef::Cast(g)) {
        o.kind = 6;
        for (int i = 0; i < 4; i++) for (int j = 0; j < 4; j++) o.xform[i * 4 + j] = r->m_xform.m_xform[i][j];
        o.name_of_idef = uuid_str(r->m_instance_definition_uuid);
        return;
    }
}

On3dm* on3dm_open(const char* path) {
    On3dm* h = new On3dm();
    if (!h->model.Read(path, nullptr)) { delete h; return nullptr; }
    h->notes = to_utf8(h->model.m_properties.m_Notes.m_notes);
    {
        ONX_ModelComponentIterator it(h->model, ON_ModelComponent::Type::Layer);
        for (ON_ModelComponentReference r = it.FirstComponentReference(); !r.IsEmpty(); r = it.NextComponentReference()) {
            const ON_Layer* layer = ON_Layer::Cast(r.ModelComponent());
            if (!layer) continue;
            On3dmLayer l;
            l.index = layer->Index();
            l.name = to_utf8(layer->Name());
            ON_Color c = layer->Color();
            l.color = ((unsigned)c.Red() << 16) | ((unsigned)c.Green() << 8) | (unsigned)c.Blue();
            l.visible = layer->IsVisible() ? 1 : 0;
            l.locked = layer->IsLocked() ? 1 : 0;
            h->layers.push_back(l);
        }
    }
    {
        ONX_ModelComponentIterator it(h->model, ON_ModelComponent::Type::InstanceDefinition);
        for (ON_ModelComponentReference r = it.FirstComponentReference(); !r.IsEmpty(); r = it.NextComponentReference()) {
            const ON_InstanceDefinition* d = ON_InstanceDefinition::Cast(r.ModelComponent());
            if (!d) continue;
            On3dmIdef def;
            def.name = to_utf8(d->Name());
            def.uuid = uuid_str(d->Id());
            const ON_SimpleArray<ON_UUID>& ids = d->InstanceGeometryIdList();
            for (int k = 0; k < ids.Count(); k++) def.member_uuids.push_back(uuid_str(ids[k]));
            h->idefs.push_back(def);
        }
    }
    {
        ONX_ModelComponentIterator it(h->model, ON_ModelComponent::Type::ModelGeometry);
        for (ON_ModelComponentReference r = it.FirstComponentReference(); !r.IsEmpty(); r = it.NextComponentReference()) {
            const ON_ModelGeometryComponent* mg = ON_ModelGeometryComponent::Cast(r.ModelComponent());
            if (!mg) continue;
            On3dmObject o;
            o.uuid = uuid_str(mg->Id());
            const ON_3dmObjectAttributes* a = mg->Attributes(nullptr);
            if (a) {
                o.layer = a->m_layer_index;
                o.name = to_utf8(a->m_name);
                if (a->ColorSource() == ON::color_from_object) {
                    o.has_color = 1;
                    o.color = ((unsigned)a->m_color.Red() << 16) | ((unsigned)a->m_color.Green() << 8) | (unsigned)a->m_color.Blue();
                }
                o.hidden = a->IsVisible() ? 0 : 1;
                o.locked = (a->Mode() == ON::locked_object) ? 1 : 0;
                o.idef_member = a->IsInstanceDefinitionObject() ? 1 : 0;
            }
            convert_geometry(mg->Geometry(nullptr), o);
            if (o.kind == 6) {
                for (size_t k = 0; k < h->idefs.size(); k++) if (h->idefs[k].uuid == o.name_of_idef) o.idef_index = (int)k;
                if (o.idef_index < 0) o.kind = 0;
            }
            h->objects.push_back(o);
        }
    }
    // resolve definition members to object indices
    {
        std::map<std::string, int> by_uuid;
        for (size_t i = 0; i < h->objects.size(); i++) by_uuid[h->objects[i].uuid] = (int)i;
        for (auto& d : h->idefs) {
            for (auto& u : d.member_uuids) {
                auto f = by_uuid.find(u);
                if (f != by_uuid.end()) { d.members.push_back(f->second); h->objects[f->second].idef_member = 1; }
            }
        }
    }
    return h;
}

int on3dm_idef_count(On3dm* h) { return (int)h->idefs.size(); }
const char* on3dm_idef_name(On3dm* h, int i) { return h->idefs[i].name.c_str(); }
int on3dm_idef_member_count(On3dm* h, int i) { return (int)h->idefs[i].members.size(); }
int on3dm_idef_member(On3dm* h, int i, int k) { return h->idefs[i].members[k]; }
int on3dm_object_idef_member(On3dm* h, int i) { return h->objects[i].idef_member; }
int on3dm_object_instance(On3dm* h, int i, int* idef, double* xf16) {
    On3dmObject& o = h->objects[i];
    if (o.kind != 6) return 0;
    *idef = o.idef_index;
    for (int k = 0; k < 16; k++) xf16[k] = o.xform[k];
    return 1;
}

On3dm* on3dm_new(void) {
    On3dm* h = new On3dm();
    h->model.m_settings.m_ModelUnitsAndTolerances.m_unit_system = ON_UnitSystem::Millimeters;
    h->model.m_settings.m_ModelUnitsAndTolerances.m_absolute_tolerance = 0.001;
    return h;
}

void on3dm_free(On3dm* h) { if (h) { delete h->pending_brep; delete h; } }

int on3dm_write(On3dm* h, const char* path, int version) {
    return h->model.Write(path, version, nullptr) ? 1 : 0;
}

int on3dm_units(On3dm* h) { return (int)h->model.m_settings.m_ModelUnitsAndTolerances.m_unit_system.UnitSystem(); }
double on3dm_abs_tol(On3dm* h) { return h->model.m_settings.m_ModelUnitsAndTolerances.m_absolute_tolerance; }
void on3dm_set_units(On3dm* h, int unit_system, double abs_tol) {
    h->model.m_settings.m_ModelUnitsAndTolerances.m_unit_system = ON_UnitSystem((ON::LengthUnitSystem)unit_system);
    h->model.m_settings.m_ModelUnitsAndTolerances.m_absolute_tolerance = abs_tol;
}
const char* on3dm_notes(On3dm* h) { return h->notes.c_str(); }
void on3dm_set_notes(On3dm* h, const char* notes) {
    h->model.m_properties.m_Notes.m_notes = ON_wString(notes);
    h->model.m_properties.m_Notes.m_bVisible = false;
}

int on3dm_layer_count(On3dm* h) { return (int)h->layers.size(); }
int on3dm_layer_index(On3dm* h, int i) { return h->layers[i].index; }
const char* on3dm_layer_name(On3dm* h, int i) { return h->layers[i].name.c_str(); }
unsigned on3dm_layer_color(On3dm* h, int i) { return h->layers[i].color; }
int on3dm_layer_visible(On3dm* h, int i) { return h->layers[i].visible; }
int on3dm_layer_locked(On3dm* h, int i) { return h->layers[i].locked; }

int on3dm_add_layer(On3dm* h, const char* name, unsigned rgb, int visible, int locked) {
    ON_wString wname(name);
    ON_Color c((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
    int idx = h->model.AddLayer(wname, c);
    if (idx >= 0 && (!visible || locked)) {
        ONX_ModelComponentIterator it(h->model, ON_ModelComponent::Type::Layer);
        for (ON_ModelComponentReference r = it.FirstComponentReference(); !r.IsEmpty(); r = it.NextComponentReference()) {
            const ON_Layer* layer = ON_Layer::Cast(r.ModelComponent());
            if (layer && layer->Index() == idx) {
                ON_Layer* l = const_cast<ON_Layer*>(layer);
                l->SetVisible(visible != 0);
                l->SetLocked(locked != 0);
            }
        }
    }
    return idx;
}

int on3dm_object_count(On3dm* h) { return (int)h->objects.size(); }
int on3dm_object_kind(On3dm* h, int i) { return h->objects[i].kind; }
int on3dm_object_layer(On3dm* h, int i) { return h->objects[i].layer; }
const char* on3dm_object_name(On3dm* h, int i) { return h->objects[i].name.c_str(); }
int on3dm_object_color(On3dm* h, int i, unsigned* rgb) { *rgb = h->objects[i].color; return h->objects[i].has_color; }
int on3dm_object_hidden(On3dm* h, int i) { return h->objects[i].hidden; }
int on3dm_object_locked(On3dm* h, int i) { return h->objects[i].locked; }
const double* on3dm_object_point(On3dm* h, int i) { return h->objects[i].point; }

int on3dm_object_curve(On3dm* h, int i, int* rational, int* order, int* cv_count, int* knot_count, const double** knots, const double** cv) {
    On3dmObject& o = h->objects[i];
    if (o.kind != 2) return 0;
    *rational = o.rational; *order = o.order_u; *cv_count = o.cvc_u; *knot_count = (int)o.knots_u.size();
    *knots = o.knots_u.data(); *cv = o.cv.data();
    return 1;
}

static int surface_out(const int rational, int ou, int ov, int cu, int cvv, const std::vector<double>& ku, const std::vector<double>& kv, const std::vector<double>& cv,
                       int* p_rational, int* p_ou, int* p_ov, int* p_cu, int* p_cv, int* p_ku_count, int* p_kv_count, const double** p_ku, const double** p_kv, const double** p_cvs) {
    *p_rational = rational; *p_ou = ou; *p_ov = ov; *p_cu = cu; *p_cv = cvv;
    *p_ku_count = (int)ku.size(); *p_kv_count = (int)kv.size();
    *p_ku = ku.data(); *p_kv = kv.data(); *p_cvs = cv.data();
    return 1;
}

int on3dm_object_surface(On3dm* h, int i, int* rational, int* ou, int* ov, int* cu, int* cvv, int* ku_count, int* kv_count, const double** ku, const double** kv, const double** cv) {
    On3dmObject& o = h->objects[i];
    if (o.kind != 3) return 0;
    return surface_out(o.rational, o.order_u, o.order_v, o.cvc_u, o.cvc_v, o.knots_u, o.knots_v, o.cv, rational, ou, ov, cu, cvv, ku_count, kv_count, ku, kv, cv);
}

int on3dm_brep_face_count(On3dm* h, int i) { return h->objects[i].kind == 4 ? (int)h->objects[i].faces.size() : 0; }
int on3dm_brep_is_solid(On3dm* h, int i) { return h->objects[i].solid; }
int on3dm_brep_face_surface(On3dm* h, int i, int f, int* rational, int* ou, int* ov, int* cu, int* cvv, int* ku_count, int* kv_count, const double** ku, const double** kv, const double** cv) {
    On3dmObject::Face& fc = h->objects[i].faces[f];
    return surface_out(fc.rational, fc.order_u, fc.order_v, fc.cvc_u, fc.cvc_v, fc.knots_u, fc.knots_v, fc.cv, rational, ou, ov, cu, cvv, ku_count, kv_count, ku, kv, cv);
}
int on3dm_brep_face_loop_count(On3dm* h, int i, int f) { return (int)h->objects[i].faces[f].loops.size(); }
int on3dm_brep_face_loop(On3dm* h, int i, int f, int l, int* is_outer, int* count, const double** uv) {
    On3dmObject::Face& fc = h->objects[i].faces[f];
    *is_outer = fc.loop_outer[l]; *count = (int)fc.loops[l].size() / 2; *uv = fc.loops[l].data();
    return 1;
}
int on3dm_object_mesh(On3dm* h, int i, int* vcount, int* tcount, const double** verts, const int** tris) {
    On3dmObject& o = h->objects[i];
    if (o.kind != 5) return 0;
    *vcount = (int)o.verts.size() / 3; *tcount = (int)o.tris.size() / 3; *verts = o.verts.data(); *tris = o.tris.data();
    return 1;
}

// ---- writing -------------------------------------------------------------

static int g_idef_mode = 0;

static void track(On3dm* h, const ON_ModelComponentReference& r) {
    if (h->idef_mode && !r.IsEmpty() && r.ModelComponent()) h->idef_ids.Append(r.ModelComponent()->Id());
}

static ON_3dmObjectAttributes* make_attrs(int layer, const char* name, int has_color, unsigned rgb, int hidden, int locked) {
    ON_3dmObjectAttributes* a = new ON_3dmObjectAttributes();
    if (g_idef_mode) a->SetMode(ON::idef_object);
    a->m_layer_index = layer;
    if (name && *name) a->m_name = ON_wString(name);
    if (has_color) {
        a->m_color = ON_Color((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
        a->SetColorSource(ON::color_from_object);
    }
    if (hidden) a->SetVisible(false);
    if (locked && !g_idef_mode) a->SetMode(ON::locked_object);
    return a;
}

void on3dm_idef_begin(On3dm* h) {
    h->idef_mode = 1;
    g_idef_mode = 1;
    h->idef_ids.SetCount(0);
}

int on3dm_idef_end(On3dm* h, const char* name) {
    h->idef_mode = 0;
    g_idef_mode = 0;
    ON_InstanceDefinition* d = new ON_InstanceDefinition();
    d->SetName(ON_wString(name));
    d->SetInstanceGeometryIdList(h->idef_ids);
    ON_ModelComponentReference r = h->model.AddManagedModelComponent(d, true);
    if (r.IsEmpty() || !r.ModelComponent()) return -1;
    h->idef_uuids.push_back(r.ModelComponent()->Id());
    return (int)h->idef_uuids.size() - 1;
}

int on3dm_add_instance(On3dm* h, int idef, const double* xf16, int layer, const char* name, int has_color, unsigned rgb, int hidden, int locked) {
    if (idef < 0 || idef >= (int)h->idef_uuids.size()) return 0;
    ON_InstanceRef* ref = new ON_InstanceRef();
    ref->m_instance_definition_uuid = h->idef_uuids[idef];
    for (int i = 0; i < 4; i++) for (int j = 0; j < 4; j++) ref->m_xform.m_xform[i][j] = xf16[i * 4 + j];
    track(h, h->model.AddManagedModelGeometryComponent(ref, make_attrs(layer, name, has_color, rgb, hidden, locked)));
    return 1;
}

static ON_NurbsCurve* make_curve(int rational, int order, int cv_count, const double* knots, const double* cv) {
    ON_NurbsCurve* c = new ON_NurbsCurve(3, rational != 0, order, cv_count);
    for (int k = 0; k < c->KnotCount(); k++) c->SetKnot(k, knots[k]);
    for (int i = 0; i < cv_count; i++) {
        const double* d = &cv[(size_t)i * 4];
        if (rational) c->SetCV(i, ON_4dPoint(d[0] * d[3], d[1] * d[3], d[2] * d[3], d[3]));
        else c->SetCV(i, ON_3dPoint(d[0], d[1], d[2]));
    }
    return c;
}

static ON_NurbsSurface* make_surface(int rational, int ou, int ov, int cu, int cvv, const double* ku, const double* kv, const double* cv) {
    ON_NurbsSurface* s = new ON_NurbsSurface(3, rational != 0, ou, ov, cu, cvv);
    for (int k = 0; k < s->KnotCount(0); k++) s->SetKnot(0, k, ku[k]);
    for (int k = 0; k < s->KnotCount(1); k++) s->SetKnot(1, k, kv[k]);
    for (int i = 0; i < cu; i++) for (int j = 0; j < cvv; j++) {
        const double* d = &cv[((size_t)i * cvv + j) * 4];
        if (rational) s->SetCV(i, j, ON_4dPoint(d[0] * d[3], d[1] * d[3], d[2] * d[3], d[3]));
        else s->SetCV(i, j, ON_3dPoint(d[0], d[1], d[2]));
    }
    return s;
}

int on3dm_add_point(On3dm* h, const double* xyz, int layer, const char* name, int has_color, unsigned rgb, int hidden, int locked) {
    ON_Point* p = new ON_Point(ON_3dPoint(xyz[0], xyz[1], xyz[2]));
    track(h, h->model.AddManagedModelGeometryComponent(p, make_attrs(layer, name, has_color, rgb, hidden, locked)));
    return 1;
}

int on3dm_add_curve(On3dm* h, int rational, int order, int cv_count, const double* knots, const double* cv, int layer, const char* name, int has_color, unsigned rgb, int hidden, int locked) {
    ON_NurbsCurve* c = make_curve(rational, order, cv_count, knots, cv);
    if (!c->IsValid()) { delete c; return 0; }
    track(h, h->model.AddManagedModelGeometryComponent(c, make_attrs(layer, name, has_color, rgb, hidden, locked)));
    return 1;
}

int on3dm_add_surface(On3dm* h, int rational, int ou, int ov, int cu, int cvv, const double* ku, const double* kv, const double* cv, int layer, const char* name, int has_color, unsigned rgb, int hidden, int locked) {
    ON_NurbsSurface* s = make_surface(rational, ou, ov, cu, cvv, ku, kv, cv);
    if (!s->IsValid()) { delete s; return 0; }
    track(h, h->model.AddManagedModelGeometryComponent(s, make_attrs(layer, name, has_color, rgb, hidden, locked)));
    return 1;
}

struct TopoKey {
    long long a, b, c;
    bool operator<(const TopoKey& o) const { return a != o.a ? a < o.a : (b != o.b ? b < o.b : c < o.c); }
};

static TopoKey quantize(const ON_3dPoint& p, double q) {
    TopoKey k;
    k.a = (long long)llround(p.x / q);
    k.b = (long long)llround(p.y / q);
    k.c = (long long)llround(p.z / q);
    return k;
}

struct BrepBuilder {
    ON_Brep* brep = nullptr;
    double q = 1e-6;
    std::map<TopoKey, int> vertices;
    struct EdgeKey { TopoKey v0, v1, mid; bool operator<(const EdgeKey& o) const { return v0 < o.v0 ? true : (o.v0 < v0 ? false : (v1 < o.v1 ? true : (o.v1 < v1 ? false : mid < o.mid))); } };
    std::map<EdgeKey, int> edges;

    int vertex(const ON_3dPoint& p) {
        TopoKey k = quantize(p, q);
        auto it = vertices.find(k);
        if (it != vertices.end()) return it->second;
        ON_BrepVertex& v = brep->NewVertex(p, 0.0);
        vertices[k] = v.m_vertex_index;
        return v.m_vertex_index;
    }

    // Returns the edge index for a 3d curve, reusing a coincident one; sets `rev` when the
    // existing edge runs opposite to `c3`.
    int edge(ON_Curve* c3, bool& rev) {
        ON_3dPoint a = c3->PointAtStart(), b = c3->PointAtEnd(), m = c3->PointAt(c3->Domain().Mid());
        TopoKey ka = quantize(a, q), kb = quantize(b, q), km = quantize(m, q);
        EdgeKey fwd{ka, kb, km};
        EdgeKey bwd{kb, ka, km};
        auto it = edges.find(fwd);
        if (it != edges.end()) { rev = false; delete c3; return it->second; }
        it = edges.find(bwd);
        if (it != edges.end()) { rev = true; delete c3; return it->second; }
        int vi0 = vertex(a);
        int vi1 = vertex(b);
        int c3i = brep->AddEdgeCurve(c3);
        ON_BrepEdge& e = brep->NewEdge(brep->m_V[vi0], brep->m_V[vi1], c3i);
        e.m_tolerance = 0.0;
        edges[fwd] = e.m_edge_index;
        rev = false;
        return e.m_edge_index;
    }

    void add_trim(ON_BrepLoop& loop, ON_Curve* c2, ON_Curve* c3) {
        // degenerate 3d curve: singular trim
        ON_3dPoint a = c3->PointAtStart(), b = c3->PointAtEnd(), m = c3->PointAt(c3->Domain().Mid());
        if (a.DistanceTo(b) < q && a.DistanceTo(m) < q) {
            int vi = vertex(a);
            int c2i = brep->AddTrimCurve(c2);
            ON_BrepTrim& t = brep->NewSingularTrim(brep->m_V[vi], loop, ON_Surface::not_iso, c2i);
            t.m_tolerance[0] = 0.0; t.m_tolerance[1] = 0.0;
            delete c3;
            return;
        }
        bool rev = false;
        int ei = edge(c3, rev);
        int c2i = brep->AddTrimCurve(c2);
        ON_BrepTrim& t = brep->NewTrim(brep->m_E[ei], rev, loop, c2i);
        t.m_tolerance[0] = 0.0; t.m_tolerance[1] = 0.0;
    }
};

static BrepBuilder* g_builder = nullptr;

int on3dm_brep_begin(On3dm* h) {
    delete h->pending_brep;
    h->pending_brep = new ON_Brep();
    delete g_builder;
    g_builder = new BrepBuilder();
    g_builder->brep = h->pending_brep;
    return 1;
}

// loops: concatenated uv pairs; loop_sizes = point counts per loop; first loop is the outer one.
int on3dm_brep_add_face(On3dm* h, int rational, int ou, int ov, int cu, int cvv, const double* ku, const double* kv, const double* cv, int loop_count, const int* loop_sizes, const double* loop_uv) {
    if (!h->pending_brep || !g_builder) return 0;
    ON_Brep& brep = *h->pending_brep;
    BrepBuilder& B = *g_builder;
    ON_NurbsSurface* s = make_surface(rational, ou, ov, cu, cvv, ku, kv, cv);
    if (!s->IsValid()) { delete s; return 0; }
    ON_BoundingBox bb = s->BoundingBox();
    B.q = ON_Max(1e-9, 1e-6 * (1.0 + bb.Diagonal().Length()));
    int si = brep.AddSurface(s);
    ON_BrepFace& face = brep.NewFace(si);
    if (loop_count == 0) {
        // untrimmed: outer loop around the surface border, exact iso curves as edges
        ON_Interval du = s->Domain(0), dv = s->Domain(1);
        ON_BrepLoop& loop = brep.NewLoop(ON_BrepLoop::outer, face);
        // south: v = v0, u increasing
        { ON_Curve* c3 = s->IsoCurve(0, dv[0]); ON_LineCurve* c2 = new ON_LineCurve(ON_2dPoint(du[0], dv[0]), ON_2dPoint(du[1], dv[0])); B.add_trim(loop, c2, c3); }
        // east: u = u1, v increasing
        { ON_Curve* c3 = s->IsoCurve(1, du[1]); ON_LineCurve* c2 = new ON_LineCurve(ON_2dPoint(du[1], dv[0]), ON_2dPoint(du[1], dv[1])); B.add_trim(loop, c2, c3); }
        // north: v = v1, u decreasing
        { ON_Curve* c3 = s->IsoCurve(0, dv[1]); c3->Reverse(); ON_LineCurve* c2 = new ON_LineCurve(ON_2dPoint(du[1], dv[1]), ON_2dPoint(du[0], dv[1])); B.add_trim(loop, c2, c3); }
        // west: u = u0, v decreasing
        { ON_Curve* c3 = s->IsoCurve(1, du[0]); c3->Reverse(); ON_LineCurve* c2 = new ON_LineCurve(ON_2dPoint(du[0], dv[1]), ON_2dPoint(du[0], dv[0])); B.add_trim(loop, c2, c3); }
        return 1;
    }
    size_t off = 0;
    for (int li = 0; li < loop_count; li++) {
        int n = loop_sizes[li];
        std::vector<ON_2dPoint> uv;
        for (int k = 0; k < n; k++) uv.push_back(ON_2dPoint(loop_uv[(off + k) * 2], loop_uv[(off + k) * 2 + 1]));
        off += n;
        if (n < 3) continue;
        ON_BrepLoop& loop = brep.NewLoop(li == 0 ? ON_BrepLoop::outer : ON_BrepLoop::inner, face);
        for (int k = 0; k < n; k++) {
            int k1 = (k + 1) % n;
            ON_LineCurve* c2 = new ON_LineCurve(uv[k], uv[k1]);
            ON_3dPoint a = s->PointAt(uv[k].x, uv[k].y);
            ON_3dPoint b = s->PointAt(uv[k1].x, uv[k1].y);
            ON_LineCurve* c3 = new ON_LineCurve(a, b);
            B.add_trim(loop, c2, c3);
        }
    }
    return 1;
}

int on3dm_brep_end(On3dm* h, int layer, const char* name, int has_color, unsigned rgb, int hidden, int locked) {
    if (!h->pending_brep) return 0;
    ON_Brep* brep = h->pending_brep;
    h->pending_brep = nullptr;
    delete g_builder;
    g_builder = nullptr;
    brep->SetTrimIsoFlags();
    brep->SetTrimTypeFlags();
    brep->SetTolerancesBoxesAndFlags();
    for (int ei = 0; ei < brep->m_E.Count(); ei++) if (!(brep->m_E[ei].m_tolerance >= 0.0)) brep->m_E[ei].m_tolerance = 0.0;
    for (int vi = 0; vi < brep->m_V.Count(); vi++) if (!(brep->m_V[vi].m_tolerance >= 0.0)) brep->m_V[vi].m_tolerance = 0.0;
    for (int ti = 0; ti < brep->m_T.Count(); ti++) { ON_BrepTrim& t = brep->m_T[ti]; if (!(t.m_tolerance[0] >= 0.0)) t.m_tolerance[0] = 0.0; if (!(t.m_tolerance[1] >= 0.0)) t.m_tolerance[1] = 0.0; }
    ON_TextLog log;
    if (!brep->IsValid(&log)) {
        // fall back: write each face surface untrimmed
        for (int fi = 0; fi < brep->m_F.Count(); fi++) {
            const ON_Surface* srf = brep->m_F[fi].SurfaceOf();
            if (!srf) continue;
            ON_NurbsSurface* ns = srf->NurbsSurface();
            if (ns) track(h, h->model.AddManagedModelGeometryComponent(ns, make_attrs(layer, name, has_color, rgb, hidden, locked)));
        }
        delete brep;
        return 2;
    }
    track(h, h->model.AddManagedModelGeometryComponent(brep, make_attrs(layer, name, has_color, rgb, hidden, locked)));
    return 1;
}

int on3dm_add_mesh(On3dm* h, int vcount, const double* verts, int tcount, const int* tris, int layer, const char* name, int has_color, unsigned rgb, int hidden, int locked) {
    ON_Mesh* m = new ON_Mesh(tcount, vcount, true, false);
    for (int i = 0; i < vcount; i++) m->SetVertex(i, ON_3dPoint(verts[(size_t)i * 3], verts[(size_t)i * 3 + 1], verts[(size_t)i * 3 + 2]));
    for (int i = 0; i < tcount; i++) m->SetTriangle(i, tris[(size_t)i * 3], tris[(size_t)i * 3 + 1], tris[(size_t)i * 3 + 2]);
    m->ComputeVertexNormals();
    track(h, h->model.AddManagedModelGeometryComponent(m, make_attrs(layer, name, has_color, rgb, hidden, locked)));
    return 1;
}

} // extern "C"

// Test helper: add a capped cylinder stored as an ON_Extrusion (what ExtrudeCrv / Cylinder save).
extern "C" int on3dm_add_test_extrusion(On3dm* h, double radius, double height) {
    ON_Cylinder cyl(ON_Circle(ON_Plane::World_xy, radius), height);
    ON_Extrusion* e = ON_Extrusion::Cylinder(cyl, true, true, nullptr);
    if (!e) return 0;
    track(h, h->model.AddManagedModelGeometryComponent(e, make_attrs(0, "", 0, 0, 0, 0)));
    return 1;
}
