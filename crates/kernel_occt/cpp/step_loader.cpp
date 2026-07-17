// printCAD OCCT shim: STEP import + per-face triangulation pipeline.
//
// Presentation colours: `XCAFDoc_ColorTool` and `XCAFDoc_VisMaterial` (diffuse / PBR base).
// STEP `CONTEXT_DEPENDENT_*` styled items + SHUO (`FindSHUO` / `FindComponent`).
// Per-face RGB: XCAF `GetSubShapes` two-pass walk (solids/shells first, then faces)
// so region colours override broadly-then-specifically.
//
// Normals: non-analytic faces with UVs use `GeomLib::NormEstim` on the face at
// identity from `Poly_Triangulation` UV nodes
// before falling back to stored or triangle-based normals. Analytic planes still get
// one outward `Geom_Plane` / `GeomAbs_Plane` direction. Mixed triangle winding on fans
// is handled in the no-UV path; `HasNormals()` noise is still overridden when geometry
// is flat.
//
// Vertices are emitted per-face, then optionally welded across seams when
// position, face-normal compatibility, and packed RGB all match.

#include "step_loader.h"

#include <BRepAdaptor_Surface.hxx>
#include <BRep_Builder.hxx>
#include <BRep_Tool.hxx>
#include <BRepTools.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <IMeshTools_Parameters.hxx>
#include <Bnd_Box.hxx>
#include <BRepBndLib.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_GTransform.hxx>
#include <BRepClass_FaceClassifier.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeSolid.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepExtrema_DistShapeShape.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepGProp.hxx>
#include <BRepLib.hxx>
#include <BRepOffsetAPI_DraftAngle.hxx>
#include <BRepOffsetAPI_MakeOffset.hxx>
#include <BRepOffsetAPI_MakePipeShell.hxx>
#include <BRepOffsetAPI_MakeThickSolid.hxx>
#include <BRepOffsetAPI_ThruSections.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCone.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeHalfSpace.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>
#include <BRepPrimAPI_MakeTorus.hxx>
#include <BRepPrimAPI_MakeWedge.hxx>
#include <GC_MakeArcOfCircle.hxx>
#include <GC_MakeArcOfEllipse.hxx>
#include <GC_MakeSegment.hxx>
#include <GProp_GProps.hxx>
#include <Geom2d_Line.hxx>
#include <Geom_BSplineCurve.hxx>
#include <Geom_ConicalSurface.hxx>
#include <Geom_CylindricalSurface.hxx>
#include <Geom_Ellipse.hxx>
#include <Geom_TrimmedCurve.hxx>
#include <IntCurvesFace_ShapeIntersector.hxx>
#include <ShapeFix_Face.hxx>
#include <ShapeUpgrade_UnifySameDomain.hxx>
#include <TColStd_Array1OfInteger.hxx>
#include <TColStd_Array1OfReal.hxx>
#include <TColgp_Array1OfPnt.hxx>
#include <gp_Elips.hxx>
#include <gp_GTrsf.hxx>
#include <gp_Lin.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Ax3.hxx>
#include <gp_Circ.hxx>
#include <gp_Pln.hxx>
#include <Standard_Version.hxx>
#include <gp.hxx>
#include <Poly_Triangulation.hxx>
#include <Poly_Triangle.hxx>
#include <Quantity_Color.hxx>
#include <Quantity_ColorRGBA.hxx>
#include <STEPControl_Reader.hxx>
#include <STEPCAFControl_Reader.hxx>
#include <Standard_Failure.hxx>
#include <TDF_ChildIterator.hxx>
#include <TDF_Label.hxx>
#include <TDF_LabelSequence.hxx>
#include <TDF_Tool.hxx>
#include <TDataStd_Name.hxx>
#include <TDocStd_Document.hxx>
#include <TCollection_AsciiString.hxx>
#include <TopAbs_Orientation.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopLoc_Location.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Iterator.hxx>
#include <TopoDS_Shape.hxx>
#include <XCAFApp_Application.hxx>
#include <XCAFDoc_ColorTool.hxx>
#include <XCAFDoc_DocumentTool.hxx>
#include <XCAFDoc_GraphNode.hxx>
#include <XCAFDoc_ShapeTool.hxx>
#include <XCAFDoc_VisMaterial.hxx>
#include <XCAFDoc_VisMaterialTool.hxx>
#include <GeomLib.hxx>
#include <Geom_Plane.hxx>
#include <Geom_Surface.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <Precision.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <sstream>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <utility>
#include <vector>

namespace {

double ms_between(
    std::chrono::steady_clock::time_point a,
    std::chrono::steady_clock::time_point b) {
    using namespace std::chrono;
    return duration<double, std::milli>(b - a).count();
}

struct MeshBuffer {
    std::vector<float> positions;
    std::vector<float> normals;
    std::vector<float> colors;
    std::vector<uint32_t> indices;
    std::vector<size_t> face_starts;
    std::vector<std::array<float, 3>> face_normals;
};

char* duplicate_to_malloc(const std::string& input) {
    char* out = static_cast<char*>(std::malloc(input.size() + 1));
    if (out == nullptr) {
        return nullptr;
    }
    std::memcpy(out, input.data(), input.size());
    out[input.size()] = '\0';
    return out;
}

PrintcadOcctImportResult make_error(const std::string& message) {
    PrintcadOcctImportResult result{};
    result.bodies = nullptr;
    result.body_count = 0;
    result.nodes = nullptr;
    result.node_count = 0;
    result.error = duplicate_to_malloc(message);
    return result;
}

std::string label_name(const TDF_Label& label) {
    if (label.IsNull()) {
        return {};
    }
    Handle(TDataStd_Name) name_attr;
    if (!label.FindAttribute(TDataStd_Name::GetID(), name_attr) || name_attr.IsNull()) {
        return {};
    }
    const TCollection_AsciiString ascii(name_attr->Get());
    return std::string(ascii.ToCString());
}

bool label_visible(const TDF_Label& label, const Handle(XCAFDoc_ColorTool)& color_tool) {
    if (label.IsNull() || color_tool.IsNull()) {
        return true;
    }
    return color_tool->IsVisible(label);
}

void trsf_to_row_major(const gp_Trsf& trsf, float* out16) {
    if (out16 == nullptr) {
        return;
    }
    out16[0] = static_cast<float>(trsf.Value(1, 1));
    out16[1] = static_cast<float>(trsf.Value(1, 2));
    out16[2] = static_cast<float>(trsf.Value(1, 3));
    out16[3] = static_cast<float>(trsf.Value(1, 4));
    out16[4] = static_cast<float>(trsf.Value(2, 1));
    out16[5] = static_cast<float>(trsf.Value(2, 2));
    out16[6] = static_cast<float>(trsf.Value(2, 3));
    out16[7] = static_cast<float>(trsf.Value(2, 4));
    out16[8] = static_cast<float>(trsf.Value(3, 1));
    out16[9] = static_cast<float>(trsf.Value(3, 2));
    out16[10] = static_cast<float>(trsf.Value(3, 3));
    out16[11] = static_cast<float>(trsf.Value(3, 4));
    out16[12] = 0.f;
    out16[13] = 0.f;
    out16[14] = 0.f;
    out16[15] = 1.f;
}

bool trsf_is_identity(const gp_Trsf& trsf) {
    return trsf.Form() == gp_Identity;
}

// Globally unique OCAF label key (e.g. "0:1:1:1:3"). `TDF_Label::Tag()` returns
// only the local integer within the parent label and collides across branches,
// so we use the full entry string for tree↔body mapping.
std::string label_entry(const TDF_Label& label) {
    if (label.IsNull()) {
        return {};
    }
    TCollection_AsciiString ascii;
    TDF_Tool::Entry(label, ascii);
    return std::string(ascii.ToCString());
}

void collect_body_labels_recursive(
    const TDF_Label& label,
    const Handle(XCAFDoc_ShapeTool)& shape_tool,
    std::unordered_set<std::string>& seen_label_entries,
    std::vector<TDF_Label>& out_body_labels) {
    if (label.IsNull() || shape_tool.IsNull()) {
        return;
    }

    if (shape_tool->IsAssembly(label)) {
        TDF_LabelSequence children;
        shape_tool->GetComponents(label, children, Standard_False);
        for (Standard_Integer i = 1; i <= children.Length(); ++i) {
            collect_body_labels_recursive(children.Value(i), shape_tool, seen_label_entries, out_body_labels);
        }
        return;
    }

    const TopoDS_Shape shape = shape_tool->GetShape(label);
    if (shape.IsNull()) {
        return;
    }
    if (seen_label_entries.insert(label_entry(label)).second) {
        out_body_labels.push_back(label);
    }
}

int node_kind_for_label(const TDF_Label& label, const Handle(XCAFDoc_ShapeTool)& shape_tool) {
    if (!shape_tool.IsNull()) {
        if (shape_tool->IsReference(label) || shape_tool->IsComponent(label)) {
            return 2; // instance
        }
        if (shape_tool->IsAssembly(label)) {
            return 0; // assembly
        }
    }
    return 1; // part
}

void append_node_recursive(
    const TDF_Label& label,
    int64_t parent_id,
    const Handle(XCAFDoc_ShapeTool)& shape_tool,
    const Handle(XCAFDoc_ColorTool)& color_tool,
    const std::unordered_map<std::string, int64_t>& body_index_by_label_entry,
    std::vector<PrintcadOcctImportNode>& out_nodes,
    uint64_t& next_id) {
    if (label.IsNull() || shape_tool.IsNull()) {
        return;
    }

    PrintcadOcctImportNode node{};
    node.id = next_id++;
    node.parent_id = parent_id;
    node.name = duplicate_to_malloc(label_name(label));
    node.kind = node_kind_for_label(label, shape_tool);
    node.visible = label_visible(label, color_tool) ? 1 : 0;
    node.body_index = -1;
    node.has_local_transform = 0;
    for (float& v : node.local_transform) {
        v = 0.f;
    }

    const auto it_body = body_index_by_label_entry.find(label_entry(label));
    if (it_body != body_index_by_label_entry.end()) {
        node.body_index = it_body->second;
    }

    const TopLoc_Location loc = XCAFDoc_ShapeTool::GetLocation(label);
    const gp_Trsf trsf = loc.Transformation();
    if (!trsf_is_identity(trsf)) {
        node.has_local_transform = 1;
        trsf_to_row_major(trsf, node.local_transform);
    }

    out_nodes.push_back(node);
    const int64_t this_id = static_cast<int64_t>(node.id);

    if (shape_tool->IsAssembly(label)) {
        TDF_LabelSequence children;
        shape_tool->GetComponents(label, children, Standard_False);
        for (Standard_Integer i = 1; i <= children.Length(); ++i) {
            append_node_recursive(
                children.Value(i),
                this_id,
                shape_tool,
                color_tool,
                body_index_by_label_entry,
                out_nodes,
                next_id);
        }
        return;
    }

    if (shape_tool->IsReference(label)) {
        TDF_Label referred;
        XCAFDoc_ShapeTool::GetReferredShape(label, referred);
        if (!referred.IsNull()) {
            append_node_recursive(
                referred,
                this_id,
                shape_tool,
                color_tool,
                body_index_by_label_entry,
                out_nodes,
                next_id);
        }
    }
}

std::vector<PrintcadOcctImportNode> build_import_nodes(
    const std::vector<TDF_Label>& body_labels,
    const Handle(XCAFDoc_ShapeTool)& shape_tool,
    const Handle(XCAFDoc_ColorTool)& color_tool) {
    std::vector<PrintcadOcctImportNode> out;
    if (shape_tool.IsNull()) {
        return out;
    }

    std::unordered_map<std::string, int64_t> body_index_by_label_entry;
    body_index_by_label_entry.reserve(body_labels.size());
    for (size_t i = 0; i < body_labels.size(); ++i) {
        body_index_by_label_entry.emplace(label_entry(body_labels[i]), static_cast<int64_t>(i));
    }

    TDF_LabelSequence roots;
    shape_tool->GetFreeShapes(roots);
    if (roots.Length() <= 0) {
        return out;
    }
    uint64_t next_id = 1;
    for (Standard_Integer i = 1; i <= roots.Length(); ++i) {
        append_node_recursive(
            roots.Value(i),
            -1,
            shape_tool,
            color_tool,
            body_index_by_label_entry,
            out,
            next_id);
    }
    return out;
}

static bool try_shape_color(
    const TopoDS_Shape& sh,
    const Handle(XCAFDoc_ColorTool)& color_tool,
    Quantity_Color& qc) {
    if (color_tool.IsNull()) {
        return Standard_False;
    }
    Quantity_ColorRGBA rgba;
    if (color_tool->GetInstanceColor(sh, XCAFDoc_ColorSurf, qc)) {
        return Standard_True;
    }
    if (color_tool->GetInstanceColor(sh, XCAFDoc_ColorGen, qc)) {
        return Standard_True;
    }
    if (color_tool->GetColor(sh, XCAFDoc_ColorSurf, qc)) {
        return Standard_True;
    }
    if (color_tool->GetColor(sh, XCAFDoc_ColorGen, qc)) {
        return Standard_True;
    }
    if (color_tool->GetColor(sh, XCAFDoc_ColorSurf, rgba)) {
        qc = rgba.GetRGB();
        return Standard_True;
    }
    if (color_tool->GetColor(sh, XCAFDoc_ColorGen, rgba)) {
        qc = rgba.GetRGB();
        return Standard_True;
    }
    if (color_tool->GetInstanceColor(sh, XCAFDoc_ColorSurf, rgba)) {
        qc = rgba.GetRGB();
        return Standard_True;
    }
    if (color_tool->GetInstanceColor(sh, XCAFDoc_ColorGen, rgba)) {
        qc = rgba.GetRGB();
        return Standard_True;
    }
    // Some STEP writers (incl. certain PCB exports) tag RGB as "curve" presentation.
    if (color_tool->GetColor(sh, XCAFDoc_ColorCurv, qc)) {
        return Standard_True;
    }
    if (color_tool->GetColor(sh, XCAFDoc_ColorCurv, rgba)) {
        qc = rgba.GetRGB();
        return Standard_True;
    }
    if (color_tool->GetInstanceColor(sh, XCAFDoc_ColorCurv, qc)) {
        return Standard_True;
    }
    if (color_tool->GetInstanceColor(sh, XCAFDoc_ColorCurv, rgba)) {
        qc = rgba.GetRGB();
        return Standard_True;
    }
    return Standard_False;
}

static bool vis_material_to_quantity_color(const Handle(XCAFDoc_VisMaterial)& mat, Quantity_Color& qc) {
    if (mat.IsNull() || mat->IsEmpty()) {
        return Standard_False;
    }
    if (mat->HasPbrMaterial() && mat->PbrMaterial().IsDefined) {
        qc = mat->PbrMaterial().BaseColor.GetRGB();
        return Standard_True;
    }
    if (mat->HasCommonMaterial() && mat->CommonMaterial().IsDefined) {
        qc = mat->CommonMaterial().DiffuseColor;
        return Standard_True;
    }
    const Quantity_ColorRGBA rgba = mat->BaseColor();
    qc = rgba.GetRGB();
    return Standard_True;
}

static bool try_vis_material_shape_label(
    const TDF_Label& shape_label,
    const Handle(XCAFDoc_VisMaterialTool)& vis_tool,
    Quantity_Color& qc) {
    if (vis_tool.IsNull()) {
        return Standard_False;
    }
    TDF_Label matL;
    if (!XCAFDoc_VisMaterialTool::GetShapeMaterial(shape_label, matL)) {
        return Standard_False;
    }
    return vis_material_to_quantity_color(vis_tool->GetMaterial(matL), qc);
}

//! Walk label → Father() chain for `XCAFDoc_ColorTool` / vis material bindings.
static bool walk_label_ancestors_for_color(
    TDF_Label L,
    const Handle(XCAFDoc_ColorTool)& color_tool,
    const Handle(XCAFDoc_VisMaterialTool)& vis_tool,
    Quantity_Color& qc) {
    Quantity_ColorRGBA rgba;
    for (; !L.IsNull(); L = L.Father()) {
        if (!color_tool.IsNull()) {
            if (color_tool->GetColor(L, XCAFDoc_ColorSurf, qc)) {
                return Standard_True;
            }
            if (color_tool->GetColor(L, XCAFDoc_ColorGen, qc)) {
                return Standard_True;
            }
            if (color_tool->GetColor(L, XCAFDoc_ColorSurf, rgba)) {
                qc = rgba.GetRGB();
                return Standard_True;
            }
            if (color_tool->GetColor(L, XCAFDoc_ColorGen, rgba)) {
                qc = rgba.GetRGB();
                return Standard_True;
            }
            if (color_tool->GetColor(L, XCAFDoc_ColorCurv, qc)) {
                return Standard_True;
            }
            if (color_tool->GetColor(L, XCAFDoc_ColorCurv, rgba)) {
                qc = rgba.GetRGB();
                return Standard_True;
            }
        }
        if (try_vis_material_shape_label(L, vis_tool, qc)) {
            return Standard_True;
        }
    }
    return Standard_False;
}

static bool try_vis_material_shape(
    const TopoDS_Shape& sh,
    const Handle(XCAFDoc_VisMaterialTool)& vis_tool,
    Quantity_Color& qc) {
    if (vis_tool.IsNull()) {
        return Standard_False;
    }
    return vis_material_to_quantity_color(vis_tool->GetShapeMaterial(sh), qc);
}

static bool try_subshape_label_colors(
    const TopoDS_Shape& face_sh,
    const TDF_Label& xcaf_root_l,
    const Handle(XCAFDoc_ShapeTool)& shape_tool,
    const Handle(XCAFDoc_ColorTool)& color_tool,
    const Handle(XCAFDoc_VisMaterialTool)& vis_tool,
    Quantity_Color& qc) {
    if (shape_tool.IsNull()) {
        return Standard_False;
    }
    if (color_tool.IsNull() && vis_tool.IsNull()) {
        return Standard_False;
    }
    auto try_on_root = [&](const TDF_Label& root_l) -> Standard_Boolean {
        if (root_l.IsNull()) {
            return Standard_False;
        }
        TDF_Label sub_l;
        if (shape_tool->FindSubShape(root_l, face_sh, sub_l) && !sub_l.IsNull()) {
            if (walk_label_ancestors_for_color(sub_l, color_tool, vis_tool, qc)) {
                return Standard_True;
            }
        }
        TopoDS_Shape face_noloc = face_sh;
        face_noloc.Location(TopLoc_Location());
        if (shape_tool->FindSubShape(root_l, face_noloc, sub_l) && !sub_l.IsNull()) {
            if (walk_label_ancestors_for_color(sub_l, color_tool, vis_tool, qc)) {
                return Standard_True;
            }
        }
        return Standard_False;
    };

    if (try_on_root(xcaf_root_l)) {
        return Standard_True;
    }
    const TDF_Label main_l = shape_tool->FindMainShape(face_sh);
    if (!main_l.IsNull() && main_l != xcaf_root_l && try_on_root(main_l)) {
        return Standard_True;
    }
    return Standard_False;
}

// CONTEXT_DEPENDENT_* styled items: colours live on instance / SHUO paths; subshape
// labels under the free-shape root (`FindSubShape`) often carry per-face RGB from
// STEPCAF. `Search` on `TopoDS_Face` alone misses those.
static bool try_find_component_path_colors(
    const TopoDS_Shape& sh,
    const Handle(XCAFDoc_ShapeTool)& shape_tool,
    const Handle(XCAFDoc_ColorTool)& color_tool,
    const Handle(XCAFDoc_VisMaterialTool)& vis_tool,
    Quantity_Color& qc) {
    if (shape_tool.IsNull()) {
        return Standard_False;
    }
    if (color_tool.IsNull() && vis_tool.IsNull()) {
        return Standard_False;
    }
    TDF_LabelSequence path;
    path.Clear();
    if (!shape_tool->FindComponent(sh, path) || path.IsEmpty()) {
        return Standard_False;
    }
    // AP242 / styled assemblies: context-dependent presentation can sit on SHUO.
    Handle(XCAFDoc_GraphNode) shuo;
    if (XCAFDoc_ShapeTool::FindSHUO(path, shuo) && !shuo.IsNull()) {
        const TDF_Label& shuo_lab = shuo->Label();
        if (walk_label_ancestors_for_color(shuo_lab, color_tool, vis_tool, qc)) {
            return Standard_True;
        }
        for (TDF_ChildIterator it(shuo_lab); it.More(); it.Next()) {
            if (walk_label_ancestors_for_color(it.Value(), color_tool, vis_tool, qc)) {
                return Standard_True;
            }
        }
    }
    for (Standard_Integer pi = path.Length(); pi >= 1; --pi) {
        if (walk_label_ancestors_for_color(path.Value(pi), color_tool, vis_tool, qc)) {
            return Standard_True;
        }
    }
    return Standard_False;
}

static bool try_label_tree_from_shape(
    const TopoDS_Shape& sh,
    const Handle(XCAFDoc_ShapeTool)& shape_tool,
    const Handle(XCAFDoc_ColorTool)& color_tool,
    const Handle(XCAFDoc_VisMaterialTool)& vis_tool,
    Quantity_Color& qc) {
    if (shape_tool.IsNull()) {
        return Standard_False;
    }
    if (color_tool.IsNull() && vis_tool.IsNull()) {
        return Standard_False;
    }
    TDF_Label lab;
    if (!shape_tool->Search(sh, lab, Standard_True, Standard_True, Standard_True)) {
        return Standard_False;
    }
    return walk_label_ancestors_for_color(lab, color_tool, vis_tool, qc);
}

static bool try_label_tree_color(
    const TopoDS_Face& face,
    const Handle(XCAFDoc_ShapeTool)& shape_tool,
    const Handle(XCAFDoc_ColorTool)& color_tool,
    const Handle(XCAFDoc_VisMaterialTool)& vis_tool,
    Quantity_Color& qc) {
    return try_label_tree_from_shape(face, shape_tool, color_tool, vis_tool, qc);
}

static bool try_ancestor_list_colors(
    const Standard_Integer idx,
    const TopTools_IndexedDataMapOfShapeListOfShape& map,
    const Handle(XCAFDoc_ColorTool)& color_tool,
    const Handle(XCAFDoc_ShapeTool)& shape_tool,
    const Handle(XCAFDoc_VisMaterialTool)& vis_tool,
    Quantity_Color& qc) {
    if (idx <= 0) {
        return Standard_False;
    }
    const TopTools_ListOfShape& lst = map.FindFromIndex(idx);
    for (TopTools_ListOfShape::Iterator it(lst); it.More(); it.Next()) {
        const TopoDS_Shape& ancestor = it.Value();
        if (try_shape_color(ancestor, color_tool, qc)) {
            return Standard_True;
        }
        if (try_vis_material_shape(ancestor, vis_tool, qc)) {
            return Standard_True;
        }
        if (try_find_component_path_colors(ancestor, shape_tool, color_tool, vis_tool, qc)) {
            return Standard_True;
        }
        if (try_label_tree_from_shape(ancestor, shape_tool, color_tool, vis_tool, qc)) {
            return Standard_True;
        }
    }
    return Standard_False;
}

// Resolve presentation colour: XCAF colour tool, visualization materials (diffuse /
// base colour from STEP AP242-style data), label inheritance, then ancestors.
static std::array<float, 3> face_albedo(
    const TopoDS_Face& face,
    const TDF_Label& xcaf_root_label,
    const Handle(XCAFDoc_ColorTool)& color_tool,
    const Handle(XCAFDoc_VisMaterialTool)& vis_tool,
    const Handle(XCAFDoc_ShapeTool)& shape_tool,
    const TopoDS_Shape& body_root,
    const TopTools_IndexedDataMapOfShapeListOfShape& face_to_solids,
    const TopTools_IndexedDataMapOfShapeListOfShape& face_to_shells,
    const TopTools_IndexedDataMapOfShapeListOfShape& face_to_compounds) {
    std::array<float, 3> rgb{{1.0f, 1.0f, 1.0f}};
    if (color_tool.IsNull() && vis_tool.IsNull()) {
        return rgb;
    }
    Quantity_Color qc;
    if (!xcaf_root_label.IsNull()
        && try_subshape_label_colors(face, xcaf_root_label, shape_tool, color_tool, vis_tool, qc)) {
        goto have_color;
    }
    if (try_shape_color(face, color_tool, qc)) {
        goto have_color;
    }
    if (try_vis_material_shape(face, vis_tool, qc)) {
        goto have_color;
    }
    if (try_find_component_path_colors(face, shape_tool, color_tool, vis_tool, qc)) {
        goto have_color;
    }
    {
        TopoDS_Shape face_noloc = face;
        face_noloc.Location(TopLoc_Location());
        if (try_shape_color(face_noloc, color_tool, qc)) {
            goto have_color;
        }
        if (try_vis_material_shape(face_noloc, vis_tool, qc)) {
            goto have_color;
        }
        if (try_find_component_path_colors(face_noloc, shape_tool, color_tool, vis_tool, qc)) {
            goto have_color;
        }
    }
    if (try_label_tree_color(face, shape_tool, color_tool, vis_tool, qc)) {
        goto have_color;
    }
    {
        const Standard_Integer isolid_path = face_to_solids.FindIndex(face);
        if (isolid_path > 0) {
            const TopTools_ListOfShape& solids = face_to_solids.FindFromIndex(isolid_path);
            for (TopTools_ListOfShape::Iterator it(solids); it.More(); it.Next()) {
                if (try_find_component_path_colors(
                        it.Value(), shape_tool, color_tool, vis_tool, qc)) {
                    goto have_color;
                }
            }
        }
    }
    {
        const Standard_Integer isolid = face_to_solids.FindIndex(face);
        if (try_ancestor_list_colors(
                isolid,
                face_to_solids,
                color_tool,
                shape_tool,
                vis_tool,
                qc)) {
            goto have_color;
        }
        const Standard_Integer ishell = face_to_shells.FindIndex(face);
        if (try_ancestor_list_colors(
                ishell,
                face_to_shells,
                color_tool,
                shape_tool,
                vis_tool,
                qc)) {
            goto have_color;
        }
        const Standard_Integer icomp = face_to_compounds.FindIndex(face);
        if (try_ancestor_list_colors(
                icomp,
                face_to_compounds,
                color_tool,
                shape_tool,
                vis_tool,
                qc)) {
            goto have_color;
        }
    }
    if (try_shape_color(body_root, color_tool, qc)) {
        goto have_color;
    }
    if (try_vis_material_shape(body_root, vis_tool, qc)) {
        goto have_color;
    }
    if (try_label_tree_from_shape(body_root, shape_tool, color_tool, vis_tool, qc)) {
        goto have_color;
    }
    if (try_find_component_path_colors(body_root, shape_tool, color_tool, vis_tool, qc)) {
        goto have_color;
    }
    return rgb;

have_color:
    rgb[0] = static_cast<float>(qc.Red());
    rgb[1] = static_cast<float>(qc.Green());
    rgb[2] = static_cast<float>(qc.Blue());
    return rgb;
}

static std::array<float, 3> rgb_from_quantity(const Quantity_Color& qc) {
    return {
        static_cast<float>(qc.Red()),
        static_cast<float>(qc.Green()),
        static_cast<float>(qc.Blue()),
    };
}

// Two-pass `GetSubShapes` colouring: solids/shells first, then face/edge labels.
// Supports assembly STEP where colour sits on solids/shells rather than every face leaf.
static bool fill_face_colors_from_xcaf_subshapes(
    const TopoDS_Shape& shape,
    const TDF_Label& xcaf_root_label,
    const Handle(XCAFDoc_ShapeTool)& shape_tool,
    const Handle(XCAFDoc_ColorTool)& color_tool,
    const Handle(XCAFDoc_VisMaterialTool)& vis_tool,
    TopTools_IndexedMapOfShape& face_map,
    std::vector<std::array<float, 3>>& out_per_face_rgb) {
    out_per_face_rgb.clear();
    face_map.Clear();
    if (xcaf_root_label.IsNull() || shape_tool.IsNull()) {
        return false;
    }
    TDF_LabelSequence seq;
    if (!shape_tool->GetSubShapes(xcaf_root_label, seq) || seq.Length() <= 0) {
        return false;
    }

    TopExp::MapShapes(shape, TopAbs_FACE, face_map);
    const Standard_Integer nfaces = face_map.Extent();
    if (nfaces <= 0) {
        return false;
    }
    out_per_face_rgb.resize(static_cast<size_t>(nfaces));

    Quantity_Color qc_body;
    std::array<float, 3> default_rgb{{1.0f, 1.0f, 1.0f}};
    if (try_shape_color(shape, color_tool, qc_body)
        || try_vis_material_shape(shape, vis_tool, qc_body)
        || try_label_tree_from_shape(shape, shape_tool, color_tool, vis_tool, qc_body)
        || try_find_component_path_colors(shape, shape_tool, color_tool, vis_tool, qc_body)) {
        default_rgb = rgb_from_quantity(qc_body);
    }
    for (auto& c : out_per_face_rgb) {
        c = default_rgb;
    }

    for (Standard_Integer pass = 0; pass < 2; ++pass) {
        for (Standard_Integer si = 1; si <= seq.Length(); ++si) {
            const TDF_Label& lab = seq.Value(si);
            const TopoDS_Shape sub_shape = shape_tool->GetShape(lab);
            if (sub_shape.IsNull()) {
                continue;
            }
            const TopAbs_ShapeEnum st = sub_shape.ShapeType();
            if (st == TopAbs_FACE || st == TopAbs_EDGE) {
                if (pass == 0) {
                    continue;
                }
            } else if (pass != 0) {
                continue;
            }
            // `TopExp_Explorer` on a compound/compsolid visits every descendant face. If that
            // aggregate appears *after* leaf solids in `GetSubShapes` order, it overwrites their
            // colours with the parent style (often neutral white) — typical PCB assembly break.
            // Colours on compounds still contribute via `default_rgb` (`try_shape_color(shape)`
            // + label walks) and pass 1 face labels; solids/shells carry per-body STEP colours.
            if (pass == 0 && (st == TopAbs_COMPOUND || st == TopAbs_COMPSOLID)) {
                continue;
            }

            Quantity_Color qc;
            bool have = false;
            Quantity_ColorRGBA rgba;
            if (!color_tool.IsNull()) {
                if (color_tool->GetColor(lab, XCAFDoc_ColorSurf, rgba)
                    || color_tool->GetColor(lab, XCAFDoc_ColorGen, rgba)) {
                    qc = rgba.GetRGB();
                    have = true;
                } else if (
                    color_tool->GetColor(lab, XCAFDoc_ColorSurf, qc)
                    || color_tool->GetColor(lab, XCAFDoc_ColorGen, qc)) {
                    have = true;
                }
            }
            if (!have) {
                have = walk_label_ancestors_for_color(lab, color_tool, vis_tool, qc);
            }
            if (!have) {
                have = try_shape_color(sub_shape, color_tool, qc);
            }
            if (!have && !vis_tool.IsNull()) {
                have = try_vis_material_shape(sub_shape, vis_tool, qc);
            }
            if (!have && !vis_tool.IsNull() && try_vis_material_shape_label(lab, vis_tool, qc)) {
                have = true;
            }
            if (!have) {
                continue;
            }
            const std::array<float, 3> fr = rgb_from_quantity(qc);
            for (TopExp_Explorer exp(sub_shape, TopAbs_FACE); exp.More(); exp.Next()) {
                const Standard_Integer fi = face_map.FindIndex(exp.Current());
                if (fi >= 1) {
                    out_per_face_rgb[static_cast<size_t>(fi - 1)] = fr;
                }
            }
        }
    }
    return true;
}

uint32_t pack_rgb_key(const float* rgb) {
    auto ch = [](float v) -> uint32_t {
        const double x = std::max(0.0, std::min(1.0, static_cast<double>(v)));
        return static_cast<uint32_t>(std::llround(x * 255.0)) & 0xffu;
    };
    return (ch(rgb[0]) << 16) | (ch(rgb[1]) << 8) | ch(rgb[2]);
}

// `GeomLib::NormEstim` at each UV node on the face at identity, then reverse for
// `TopAbs_REVERSED`, then transform by the triangulation placement (same as mesh vertices).
bool fill_normals_from_surface_uv(
    const TopoDS_Face& face,
    bool reversed,
    const Handle(Poly_Triangulation)& triangulation,
    const gp_Trsf& trsf,
    uint32_t base_index,
    int node_count,
    MeshBuffer& out,
    std::array<float, 3>& face_normal) {
    if (!triangulation->HasUVNodes()) {
        return false;
    }
    TopoDS_Face face0 = TopoDS::Face(face.Located(TopLoc_Location()));
    Handle(Geom_Surface) surf0 = BRep_Tool::Surface(face0);
    if (surf0.IsNull()) {
        return false;
    }
    const Standard_Real tol = Precision::Confusion();
    std::vector<gp_Dir> dirs(static_cast<size_t>(node_count));
    for (int i = 1; i <= node_count; ++i) {
        gp_Dir n_est;
        if (GeomLib::NormEstim(surf0, triangulation->UVNode(i), tol, n_est) > 1) {
            return false;
        }
        if (reversed) {
            n_est.Reverse();
        }
        dirs[static_cast<size_t>(i - 1)] = n_est;
    }

    double fn_acc_x = 0.0;
    double fn_acc_y = 0.0;
    double fn_acc_z = 0.0;
    for (int i = 1; i <= node_count; ++i) {
        gp_Dir dn = dirs[static_cast<size_t>(i - 1)];
        dn.Transform(trsf);
        const float nx = static_cast<float>(dn.X());
        const float ny = static_cast<float>(dn.Y());
        const float nz = static_cast<float>(dn.Z());
        const uint32_t vi = base_index + static_cast<uint32_t>(i - 1);
        out.normals[vi * 3 + 0] = nx;
        out.normals[vi * 3 + 1] = ny;
        out.normals[vi * 3 + 2] = nz;
        fn_acc_x += static_cast<double>(nx);
        fn_acc_y += static_cast<double>(ny);
        fn_acc_z += static_cast<double>(nz);
    }
    const double face_len_acc =
        std::sqrt(fn_acc_x * fn_acc_x + fn_acc_y * fn_acc_y + fn_acc_z * fn_acc_z);
    if (face_len_acc > 1e-12) {
        face_normal[0] = static_cast<float>(fn_acc_x / face_len_acc);
        face_normal[1] = static_cast<float>(fn_acc_y / face_len_acc);
        face_normal[2] = static_cast<float>(fn_acc_z / face_len_acc);
    }
    return true;
}

bool append_face(
    const TopoDS_Face& face,
    const std::array<float, 3>& face_rgb,
    MeshBuffer& out) {
    TopLoc_Location location;
    Handle(Poly_Triangulation) triangulation = BRep_Tool::Triangulation(face, location);
    if (triangulation.IsNull()) {
        return false;
    }

    const int node_count = triangulation->NbNodes();
    const int triangle_count = triangulation->NbTriangles();
    if (node_count <= 0 || triangle_count <= 0) {
        return false;
    }

    const gp_Trsf& trsf = location.Transformation();
    const bool reversed = face.Orientation() == TopAbs_REVERSED;

    const uint32_t base_index = static_cast<uint32_t>(out.positions.size() / 3);

    out.positions.reserve(out.positions.size() + static_cast<size_t>(node_count) * 3);
    out.colors.reserve(out.colors.size() + static_cast<size_t>(node_count) * 3);
    for (int i = 1; i <= node_count; ++i) {
        gp_Pnt p = triangulation->Node(i);
        p.Transform(trsf);
        out.positions.push_back(static_cast<float>(p.X()));
        out.positions.push_back(static_cast<float>(p.Y()));
        out.positions.push_back(static_cast<float>(p.Z()));
        out.colors.push_back(face_rgb[0]);
        out.colors.push_back(face_rgb[1]);
        out.colors.push_back(face_rgb[2]);
    }

    out.normals.resize(out.positions.size(), 0.0f);

    // `BRepAdaptor_Surface` is usually enough, but some STEP planes (incl. holed plates)
    // still surface as non–`GeomAbs_Plane` while the underlying geometry is `Geom_Plane`.
    TopLoc_Location surf_loc;
    Handle(Geom_Surface) geom_surf = BRep_Tool::Surface(face, surf_loc);
    Handle(Geom_Plane) gpl = Handle(Geom_Plane)::DownCast(geom_surf);
    bool is_geom_plane = false;
    gp_Dir plane_dir_geom;
    if (!gpl.IsNull()) {
        gp_Pln pln = gpl->Pln();
        pln.Transform(surf_loc.Transformation());
        plane_dir_geom = pln.Axis().Direction();
        if (reversed) {
            plane_dir_geom.Reverse();
        }
        is_geom_plane = true;
    }

    BRepAdaptor_Surface surface(face);
    const bool is_plane_adaptor = surface.GetType() == GeomAbs_Plane;
    const bool use_analytic_plane = is_plane_adaptor || is_geom_plane;

    double face_nx = 0.0;
    double face_ny = 0.0;
    double face_nz = 0.0;

    std::array<float, 3> face_normal{0.0f, 0.0f, 1.0f};

    out.indices.reserve(out.indices.size() + static_cast<size_t>(triangle_count) * 3);
    if (use_analytic_plane) {
        gp_Dir dn = is_plane_adaptor ? surface.Plane().Axis().Direction() : plane_dir_geom;
        if (is_plane_adaptor && reversed) {
            dn.Reverse();
        }
        const float fnx = static_cast<float>(dn.X());
        const float fny = static_cast<float>(dn.Y());
        const float fnz = static_cast<float>(dn.Z());
        face_normal[0] = fnx;
        face_normal[1] = fny;
        face_normal[2] = fnz;

        for (int i = 1; i <= node_count; ++i) {
            const uint32_t vi = base_index + static_cast<uint32_t>(i - 1);
            out.normals[vi * 3 + 0] = fnx;
            out.normals[vi * 3 + 1] = fny;
            out.normals[vi * 3 + 2] = fnz;
        }

        for (int t = 1; t <= triangle_count; ++t) {
            const Poly_Triangle& tri = triangulation->Triangle(t);
            int n1, n2, n3;
            tri.Get(n1, n2, n3);
            if (reversed) {
                std::swap(n2, n3);
            }
            const uint32_t a = base_index + static_cast<uint32_t>(n1 - 1);
            const uint32_t b = base_index + static_cast<uint32_t>(n2 - 1);
            const uint32_t c = base_index + static_cast<uint32_t>(n3 - 1);
            out.indices.push_back(a);
            out.indices.push_back(b);
            out.indices.push_back(c);
        }
    } else {
        auto emit_triangle_indices = [&]() {
            for (int t = 1; t <= triangle_count; ++t) {
                const Poly_Triangle& tri = triangulation->Triangle(t);
                int n1, n2, n3;
                tri.Get(n1, n2, n3);
                if (reversed) {
                    std::swap(n2, n3);
                }
                const uint32_t a = base_index + static_cast<uint32_t>(n1 - 1);
                const uint32_t b = base_index + static_cast<uint32_t>(n2 - 1);
                const uint32_t c = base_index + static_cast<uint32_t>(n3 - 1);
                out.indices.push_back(a);
                out.indices.push_back(b);
                out.indices.push_back(c);
            }
        };

        if (fill_normals_from_surface_uv(
                face,
                reversed,
                triangulation,
                trsf,
                base_index,
                node_count,
                out,
                face_normal)) {
            emit_triangle_indices();
        } else if (triangulation->HasNormals()) {
            double fn_acc_x = 0.0;
            double fn_acc_y = 0.0;
            double fn_acc_z = 0.0;
            for (int i = 1; i <= node_count; ++i) {
                gp_Dir dn = triangulation->Normal(i);
                dn.Transform(trsf);
                if (reversed) {
                    dn.Reverse();
                }
                const uint32_t vi = base_index + static_cast<uint32_t>(i - 1);
                const float nx = static_cast<float>(dn.X());
                const float ny = static_cast<float>(dn.Y());
                const float nz = static_cast<float>(dn.Z());
                out.normals[vi * 3 + 0] = nx;
                out.normals[vi * 3 + 1] = ny;
                out.normals[vi * 3 + 2] = nz;
                fn_acc_x += static_cast<double>(nx);
                fn_acc_y += static_cast<double>(ny);
                fn_acc_z += static_cast<double>(nz);
            }
            const double face_len_acc =
                std::sqrt(fn_acc_x * fn_acc_x + fn_acc_y * fn_acc_y + fn_acc_z * fn_acc_z);
            if (face_len_acc > 1e-12) {
                face_normal[0] = static_cast<float>(fn_acc_x / face_len_acc);
                face_normal[1] = static_cast<float>(fn_acc_y / face_len_acc);
                face_normal[2] = static_cast<float>(fn_acc_z / face_len_acc);
            }

            emit_triangle_indices();
        } else {
        std::vector<std::array<float, 3>> tri_unit_normals;
        tri_unit_normals.reserve(static_cast<size_t>(triangle_count));

        for (int t = 1; t <= triangle_count; ++t) {
            const Poly_Triangle& tri = triangulation->Triangle(t);
            int n1, n2, n3;
            tri.Get(n1, n2, n3);
            if (reversed) {
                std::swap(n2, n3);
            }
            const uint32_t a = base_index + static_cast<uint32_t>(n1 - 1);
            const uint32_t b = base_index + static_cast<uint32_t>(n2 - 1);
            const uint32_t c = base_index + static_cast<uint32_t>(n3 - 1);

            const float ax = out.positions[a * 3 + 0];
            const float ay = out.positions[a * 3 + 1];
            const float az = out.positions[a * 3 + 2];
            const float bx = out.positions[b * 3 + 0];
            const float by = out.positions[b * 3 + 1];
            const float bz = out.positions[b * 3 + 2];
            const float cx = out.positions[c * 3 + 0];
            const float cy = out.positions[c * 3 + 1];
            const float cz = out.positions[c * 3 + 2];

            const float ux = bx - ax, uy = by - ay, uz = bz - az;
            const float vx = cx - ax, vy = cy - ay, vz = cz - az;
            const float nx = uy * vz - uz * vy;
            const float ny = uz * vx - ux * vz;
            const float nz = ux * vy - uy * vx;

            out.normals[a * 3 + 0] += nx;
            out.normals[a * 3 + 1] += ny;
            out.normals[a * 3 + 2] += nz;
            out.normals[b * 3 + 0] += nx;
            out.normals[b * 3 + 1] += ny;
            out.normals[b * 3 + 2] += nz;
            out.normals[c * 3 + 0] += nx;
            out.normals[c * 3 + 1] += ny;
            out.normals[c * 3 + 2] += nz;

            face_nx += nx;
            face_ny += ny;
            face_nz += nz;

            const float tlen = std::sqrt(nx * nx + ny * ny + nz * nz);
            if (tlen > 1e-20f) {
                tri_unit_normals.push_back({nx / tlen, ny / tlen, nz / tlen});
            } else {
                tri_unit_normals.push_back({0.0f, 0.0f, 1.0f});
            }
        }

            emit_triangle_indices();

        for (size_t i = base_index * 3; i < out.normals.size(); i += 3) {
            const float nx = out.normals[i];
            const float ny = out.normals[i + 1];
            const float nz = out.normals[i + 2];
            const float len = std::sqrt(nx * nx + ny * ny + nz * nz);
            if (len > 1e-8f) {
                out.normals[i] = nx / len;
                out.normals[i + 1] = ny / len;
                out.normals[i + 2] = nz / len;
            } else {
                out.normals[i] = 0.0f;
                out.normals[i + 1] = 0.0f;
                out.normals[i + 2] = 1.0f;
            }
        }

        const double face_len =
            std::sqrt(face_nx * face_nx + face_ny * face_ny + face_nz * face_nz);
        if (face_len > 1e-12) {
            face_normal[0] = static_cast<float>(face_nx / face_len);
            face_normal[1] = static_cast<float>(face_ny / face_len);
            face_normal[2] = static_cast<float>(face_nz / face_len);
        }

        // Non-analytic but geometrically flat panels: unify normals when triangle normals
        // agree (mean direction + |dot| handles winding noise). Relaxed threshold catches
        // tessellation that is almost coplanar but numerically imperfect.
        // Flip inconsistent winding first so the mean direction is not diluted (fan
        // triangulation from corners to circular holes often mixes orientations).
        constexpr float k_flat_dot = 0.992f;
        std::array<float, 3> ref_n{0.0f, 0.0f, 1.0f};
        bool have_ref = false;
        for (const auto& tn : tri_unit_normals) {
            const float tlen2 = tn[0] * tn[0] + tn[1] * tn[1] + tn[2] * tn[2];
            if (tlen2 > 1e-12f) {
                ref_n = tn;
                have_ref = true;
                break;
            }
        }
        std::vector<std::array<float, 3>> aligned_tris;
        aligned_tris.reserve(tri_unit_normals.size());
        float mx = 0.0f;
        float my = 0.0f;
        float mz = 0.0f;
        for (auto tn : tri_unit_normals) {
            if (have_ref) {
                const float s =
                    tn[0] * ref_n[0] + tn[1] * ref_n[1] + tn[2] * ref_n[2];
                if (s < 0.0f) {
                    tn[0] = -tn[0];
                    tn[1] = -tn[1];
                    tn[2] = -tn[2];
                }
            }
            aligned_tris.push_back(tn);
            mx += tn[0];
            my += tn[1];
            mz += tn[2];
        }
        const float mlen = std::sqrt(mx * mx + my * my + mz * mz);
        if (mlen > 1e-8f) {
            mx /= mlen;
            my /= mlen;
            mz /= mlen;
        } else {
            mx = face_normal[0];
            my = face_normal[1];
            mz = face_normal[2];
        }

        bool flat_enough =
            face_len > 1e-12 && aligned_tris.size() == static_cast<size_t>(triangle_count);
        for (size_t ti = 0; flat_enough && ti < aligned_tris.size(); ++ti) {
            const auto& tn = aligned_tris[ti];
            const float d = std::abs(tn[0] * mx + tn[1] * my + tn[2] * mz);
            if (d < k_flat_dot) {
                flat_enough = false;
            }
        }
        if (flat_enough) {
            face_normal[0] = mx;
            face_normal[1] = my;
            face_normal[2] = mz;
            for (int i = 1; i <= node_count; ++i) {
                const uint32_t vi = base_index + static_cast<uint32_t>(i - 1);
                out.normals[vi * 3 + 0] = mx;
                out.normals[vi * 3 + 1] = my;
                out.normals[vi * 3 + 2] = mz;
            }
        }
        }
    }

    // Spline or other non-analytic surfaces that are still geometrically planar
    // (common for plates with holes) often hit `HasNormals()` with perturbed UV /
    // trim normals — rebuild a direction purely from triangle geometry.
    if (!use_analytic_plane && triangle_count > 0) {
        std::vector<std::array<float, 3>> tri_unit_normals_geom;
        tri_unit_normals_geom.reserve(static_cast<size_t>(triangle_count));
        double gnx = 0.0;
        double gny = 0.0;
        double gnz = 0.0;
        for (int t = 1; t <= triangle_count; ++t) {
            const Poly_Triangle& tri = triangulation->Triangle(t);
            int n1, n2, n3;
            tri.Get(n1, n2, n3);
            if (reversed) {
                std::swap(n2, n3);
            }
            const uint32_t a = base_index + static_cast<uint32_t>(n1 - 1);
            const uint32_t b = base_index + static_cast<uint32_t>(n2 - 1);
            const uint32_t c = base_index + static_cast<uint32_t>(n3 - 1);

            const float ax = out.positions[a * 3 + 0];
            const float ay = out.positions[a * 3 + 1];
            const float az = out.positions[a * 3 + 2];
            const float bx = out.positions[b * 3 + 0];
            const float by = out.positions[b * 3 + 1];
            const float bz = out.positions[b * 3 + 2];
            const float cx = out.positions[c * 3 + 0];
            const float cy = out.positions[c * 3 + 1];
            const float cz = out.positions[c * 3 + 2];

            const float ux = bx - ax, uy = by - ay, uz = bz - az;
            const float vx = cx - ax, vy = cy - ay, vz = cz - az;
            const float nx = uy * vz - uz * vy;
            const float ny = uz * vx - ux * vz;
            const float nz = ux * vy - uy * vx;

            gnx += static_cast<double>(nx);
            gny += static_cast<double>(ny);
            gnz += static_cast<double>(nz);
            const float tlen = std::sqrt(nx * nx + ny * ny + nz * nz);
            if (tlen > 1e-20f) {
                tri_unit_normals_geom.push_back({nx / tlen, ny / tlen, nz / tlen});
            } else {
                tri_unit_normals_geom.push_back({0.0f, 0.0f, 1.0f});
            }
        }
        constexpr float k_flat_dot_geom = 0.992f;
        std::array<float, 3> ref_g{0.0f, 0.0f, 1.0f};
        bool have_ref_g = false;
        for (const auto& tn : tri_unit_normals_geom) {
            const float tlen2 = tn[0] * tn[0] + tn[1] * tn[1] + tn[2] * tn[2];
            if (tlen2 > 1e-12f) {
                ref_g = tn;
                have_ref_g = true;
                break;
            }
        }
        std::vector<std::array<float, 3>> aligned_geom;
        aligned_geom.reserve(tri_unit_normals_geom.size());
        float mx = 0.0f;
        float my = 0.0f;
        float mz = 0.0f;
        for (auto tn : tri_unit_normals_geom) {
            if (have_ref_g) {
                const float s =
                    tn[0] * ref_g[0] + tn[1] * ref_g[1] + tn[2] * ref_g[2];
                if (s < 0.0f) {
                    tn[0] = -tn[0];
                    tn[1] = -tn[1];
                    tn[2] = -tn[2];
                }
            }
            aligned_geom.push_back(tn);
            mx += tn[0];
            my += tn[1];
            mz += tn[2];
        }
        const float mlen = std::sqrt(mx * mx + my * my + mz * mz);
        if (mlen > 1e-8f) {
            mx /= mlen;
            my /= mlen;
            mz /= mlen;
        }
        const double face_len_geom = std::sqrt(gnx * gnx + gny * gny + gnz * gnz);
        bool flat_geom = face_len_geom > 1e-12
            && aligned_geom.size() == static_cast<size_t>(triangle_count);
        for (size_t ti = 0; flat_geom && ti < aligned_geom.size(); ++ti) {
            const auto& tn = aligned_geom[ti];
            const float d = std::abs(tn[0] * mx + tn[1] * my + tn[2] * mz);
            if (d < k_flat_dot_geom) {
                flat_geom = false;
            }
        }
        if (flat_geom) {
            face_normal[0] = mx;
            face_normal[1] = my;
            face_normal[2] = mz;
            for (int i = 1; i <= node_count; ++i) {
                const uint32_t vi = base_index + static_cast<uint32_t>(i - 1);
                out.normals[vi * 3 + 0] = mx;
                out.normals[vi * 3 + 1] = my;
                out.normals[vi * 3 + 2] = mz;
            }
        }
    }

    out.face_normals.push_back(face_normal);

    return true;
}

std::vector<uint32_t> extract_boundary_edges(const std::vector<uint32_t>& indices) {
    if (indices.size() < 3) {
        return {};
    }

    struct EdgeRecord {
        uint32_t a;
        uint32_t b;
        uint32_t count;
    };

    struct PairHash {
        size_t operator()(const std::pair<uint32_t, uint32_t>& p) const noexcept {
            return std::hash<uint64_t>{}((static_cast<uint64_t>(p.first) << 32) | p.second);
        }
    };

    std::unordered_map<std::pair<uint32_t, uint32_t>, EdgeRecord, PairHash> edges;
    edges.reserve(indices.size());

    auto bump = [&](uint32_t a, uint32_t b) {
        if (a == b) {
            return;
        }
        std::pair<uint32_t, uint32_t> key = a < b ? std::make_pair(a, b) : std::make_pair(b, a);
        auto it = edges.find(key);
        if (it == edges.end()) {
            edges.emplace(key, EdgeRecord{a, b, 1});
        } else {
            it->second.count += 1;
        }
    };

    for (size_t i = 0; i + 2 < indices.size(); i += 3) {
        const uint32_t a = indices[i];
        const uint32_t b = indices[i + 1];
        const uint32_t c = indices[i + 2];
        bump(a, b);
        bump(b, c);
        bump(c, a);
    }

    std::vector<uint32_t> out;
    out.reserve(edges.size() * 2);
    for (const auto& kv : edges) {
        if (kv.second.count == 1) {
            out.push_back(kv.second.a);
            out.push_back(kv.second.b);
        }
    }
    return out;
}

std::vector<uint32_t> weld_vertices(
    MeshBuffer& mesh,
    const std::vector<uint32_t>& vertex_face,
    float angle_cos_threshold) {
    const size_t old_vertex_count = mesh.positions.size() / 3;
    if (mesh.colors.size() != mesh.positions.size()) {
        mesh.colors.resize(mesh.positions.size(), 1.0f);
    }

    constexpr double kQuantize = 1.0e5;

    struct PositionKey {
        int64_t x, y, z;
        bool operator==(const PositionKey& other) const noexcept {
            return x == other.x && y == other.y && z == other.z;
        }
    };
    struct PositionKeyHash {
        size_t operator()(const PositionKey& k) const noexcept {
            const uint64_t hx = static_cast<uint64_t>(k.x);
            const uint64_t hy = static_cast<uint64_t>(k.y);
            const uint64_t hz = static_cast<uint64_t>(k.z);
            uint64_t h = hx;
            h = h * 0x9E3779B97F4A7C15ULL ^ hy;
            h = h * 0x9E3779B97F4A7C15ULL ^ hz;
            return static_cast<size_t>(h);
        }
    };

    auto key_of = [](const float* p) {
        return PositionKey{
            static_cast<int64_t>(std::llround(static_cast<double>(p[0]) * kQuantize)),
            static_cast<int64_t>(std::llround(static_cast<double>(p[1]) * kQuantize)),
            static_cast<int64_t>(std::llround(static_cast<double>(p[2]) * kQuantize)),
        };
    };

    struct BucketEntry {
        uint32_t canonical_index;
        std::array<float, 3> face_normal;
        uint32_t color_packed;
    };

    std::unordered_map<PositionKey, std::vector<BucketEntry>, PositionKeyHash> buckets;
    buckets.reserve(old_vertex_count);

    std::vector<float> new_positions;
    std::vector<float> new_colors;
    std::vector<double> normal_accum;
    std::vector<uint32_t> normal_weights;
    new_positions.reserve(mesh.positions.size());
    new_colors.reserve(mesh.colors.size());
    normal_accum.reserve(mesh.normals.size());
    normal_weights.reserve(old_vertex_count);

    std::vector<uint32_t> remap(old_vertex_count, 0);

    for (size_t i = 0; i < old_vertex_count; ++i) {
        const float* pos = &mesh.positions[i * 3];
        const float* nrm = &mesh.normals[i * 3];
        const float* col = &mesh.colors[i * 3];
        const uint32_t fid = vertex_face[i];
        const std::array<float, 3>& face_n = mesh.face_normals[fid];
        const uint32_t pcol = pack_rgb_key(col);

        const PositionKey key = key_of(pos);
        auto& bucket = buckets[key];

        uint32_t canonical = UINT32_MAX;
        for (auto& entry : bucket) {
            if (entry.color_packed != pcol) {
                continue;
            }
            const float dot = entry.face_normal[0] * face_n[0]
                            + entry.face_normal[1] * face_n[1]
                            + entry.face_normal[2] * face_n[2];
            if (dot >= angle_cos_threshold) {
                canonical = entry.canonical_index;
                const float w = static_cast<float>(normal_weights[canonical]);
                const float inv = 1.0f / (w + 1.0f);
                entry.face_normal = {
                    (entry.face_normal[0] * w + face_n[0]) * inv,
                    (entry.face_normal[1] * w + face_n[1]) * inv,
                    (entry.face_normal[2] * w + face_n[2]) * inv,
                };
                const float len = std::sqrt(
                    entry.face_normal[0] * entry.face_normal[0]
                    + entry.face_normal[1] * entry.face_normal[1]
                    + entry.face_normal[2] * entry.face_normal[2]);
                if (len > 1e-8f) {
                    entry.face_normal[0] /= len;
                    entry.face_normal[1] /= len;
                    entry.face_normal[2] /= len;
                }
                break;
            }
        }

        if (canonical == UINT32_MAX) {
            canonical = static_cast<uint32_t>(new_positions.size() / 3);
            new_positions.push_back(pos[0]);
            new_positions.push_back(pos[1]);
            new_positions.push_back(pos[2]);
            new_colors.push_back(col[0]);
            new_colors.push_back(col[1]);
            new_colors.push_back(col[2]);
            normal_accum.push_back(static_cast<double>(nrm[0]));
            normal_accum.push_back(static_cast<double>(nrm[1]));
            normal_accum.push_back(static_cast<double>(nrm[2]));
            normal_weights.push_back(1);
            bucket.push_back(BucketEntry{canonical, face_n, pcol});
        } else {
            normal_accum[canonical * 3 + 0] += static_cast<double>(nrm[0]);
            normal_accum[canonical * 3 + 1] += static_cast<double>(nrm[1]);
            normal_accum[canonical * 3 + 2] += static_cast<double>(nrm[2]);
            normal_weights[canonical] += 1;
        }

        remap[i] = canonical;
    }

    std::vector<float> new_normals;
    new_normals.resize(new_positions.size(), 0.0f);
    const size_t new_vertex_count = new_positions.size() / 3;
    for (size_t i = 0; i < new_vertex_count; ++i) {
        const double w = static_cast<double>(normal_weights[i]);
        const double inv = w > 0.0 ? 1.0 / w : 0.0;
        const double nx = normal_accum[i * 3 + 0] * inv;
        const double ny = normal_accum[i * 3 + 1] * inv;
        const double nz = normal_accum[i * 3 + 2] * inv;
        const double len = std::sqrt(nx * nx + ny * ny + nz * nz);
        if (len > 1e-12) {
            new_normals[i * 3 + 0] = static_cast<float>(nx / len);
            new_normals[i * 3 + 1] = static_cast<float>(ny / len);
            new_normals[i * 3 + 2] = static_cast<float>(nz / len);
        } else {
            new_normals[i * 3 + 0] = 0.0f;
            new_normals[i * 3 + 1] = 0.0f;
            new_normals[i * 3 + 2] = 1.0f;
        }
    }

    for (auto& idx : mesh.indices) {
        idx = remap[idx];
    }

    mesh.positions = std::move(new_positions);
    mesh.normals = std::move(new_normals);
    mesh.colors = std::move(new_colors);
    return remap;
}

bool finalize_body(
    MeshBuffer& buffer,
    const std::string& name,
    const std::vector<uint32_t>& edges,
    std::vector<PrintcadOcctBody>& out) {
    if (buffer.indices.empty() || buffer.positions.empty()) {
        return false;
    }

    const size_t vn = buffer.positions.size() / 3;
    if (buffer.colors.size() != buffer.positions.size()) {
        buffer.colors.resize(buffer.positions.size(), 1.0f);
    }
    if (buffer.normals.size() != buffer.positions.size()) {
        return false;
    }

    PrintcadOcctBody body{};
    body.name = name.empty() ? nullptr : duplicate_to_malloc(name);

    body.vertex_count = vn;
    body.index_count = buffer.indices.size();
    body.edge_count = edges.size() / 2;

    body.positions = static_cast<float*>(std::malloc(buffer.positions.size() * sizeof(float)));
    body.normals = static_cast<float*>(std::malloc(buffer.normals.size() * sizeof(float)));
    body.colors = static_cast<float*>(std::malloc(buffer.colors.size() * sizeof(float)));
    body.indices = static_cast<uint32_t*>(std::malloc(buffer.indices.size() * sizeof(uint32_t)));
    body.edges = edges.empty()
                     ? nullptr
                     : static_cast<uint32_t*>(std::malloc(edges.size() * sizeof(uint32_t)));

    if (body.positions == nullptr || body.normals == nullptr || body.colors == nullptr
        || body.indices == nullptr || (!edges.empty() && body.edges == nullptr)) {
        std::free(body.positions);
        std::free(body.normals);
        std::free(body.colors);
        std::free(body.indices);
        std::free(body.edges);
        std::free(body.name);
        return false;
    }
    std::memcpy(body.positions, buffer.positions.data(), buffer.positions.size() * sizeof(float));
    std::memcpy(body.normals, buffer.normals.data(), buffer.normals.size() * sizeof(float));
    std::memcpy(body.colors, buffer.colors.data(), buffer.colors.size() * sizeof(float));
    std::memcpy(body.indices, buffer.indices.data(), buffer.indices.size() * sizeof(uint32_t));
    if (!edges.empty()) {
        std::memcpy(body.edges, edges.data(), edges.size() * sizeof(uint32_t));
    }
    out.push_back(body);
    return true;
}

std::vector<std::array<float, 3>> collect_face_display_rgbs(
    const TopoDS_Shape& shape,
    const TDF_Label& xcaf_root_label,
    const Handle(XCAFDoc_ColorTool)& color_tool,
    const Handle(XCAFDoc_VisMaterialTool)& vis_tool,
    const Handle(XCAFDoc_ShapeTool)& shape_tool) {
    TopTools_IndexedDataMapOfShapeListOfShape face_to_solids;
    TopTools_IndexedDataMapOfShapeListOfShape face_to_shells;
    TopTools_IndexedDataMapOfShapeListOfShape face_to_compounds;
    if (!color_tool.IsNull() || !vis_tool.IsNull()) {
        TopExp::MapShapesAndAncestors(shape, TopAbs_FACE, TopAbs_SOLID, face_to_solids);
        TopExp::MapShapesAndAncestors(shape, TopAbs_FACE, TopAbs_SHELL, face_to_shells);
        TopExp::MapShapesAndAncestors(shape, TopAbs_FACE, TopAbs_COMPOUND, face_to_compounds);
    }

    TopTools_IndexedMapOfShape xcaf_subshape_face_map;
    std::vector<std::array<float, 3>> xcaf_subshape_face_rgb;
    const bool use_subshape_face_palette =
        (!color_tool.IsNull() || !vis_tool.IsNull())
        && fill_face_colors_from_xcaf_subshapes(
            shape,
            xcaf_root_label,
            shape_tool,
            color_tool,
            vis_tool,
            xcaf_subshape_face_map,
            xcaf_subshape_face_rgb);

    std::vector<std::array<float, 3>> rgbs;
    for (TopExp_Explorer it(shape, TopAbs_FACE); it.More(); it.Next()) {
        const TopoDS_Face& face = TopoDS::Face(it.Current());
        const std::array<float, 3> frgb_alb = face_albedo(
            face,
            xcaf_root_label,
            color_tool,
            vis_tool,
            shape_tool,
            shape,
            face_to_solids,
            face_to_shells,
            face_to_compounds);
        std::array<float, 3> frgb = frgb_alb;
        if (use_subshape_face_palette) {
            const Standard_Integer fi = xcaf_subshape_face_map.FindIndex(face);
            const std::array<float, 3> frgb_pal =
                (fi >= 1) ? xcaf_subshape_face_rgb[static_cast<size_t>(fi - 1)]
                          : std::array<float, 3>{{1.0f, 1.0f, 1.0f}};
            auto nearly_white = [](const std::array<float, 3>& c) -> bool {
                return c[0] >= 0.999f && c[1] >= 0.999f && c[2] >= 0.999f;
            };
            const bool pal_w = nearly_white(frgb_pal);
            const bool alb_w = nearly_white(frgb_alb);
            if (!pal_w && alb_w) {
                frgb = frgb_pal;
            } else if (pal_w && !alb_w) {
                frgb = frgb_alb;
            } else if (!pal_w && !alb_w) {
                frgb = frgb_pal;
            } else {
                frgb = frgb_alb;
            }
        }
        rgbs.push_back(frgb);
    }
    return rgbs;
}

void mesh_shape_from_precolored_faces(
    const TopoDS_Shape& shape,
    const std::vector<std::array<float, 3>>& face_rgbs,
    std::vector<PrintcadOcctBody>& out,
    int index,
    bool weld_cross_face,
    float weld_angle_cos_threshold,
    bool generate_boundary_edges) {
    if (face_rgbs.empty()) {
        return;
    }
    size_t face_idx = 0;
    MeshBuffer buffer;
    buffer.face_starts.push_back(0);
    for (TopExp_Explorer it(shape, TopAbs_FACE); it.More(); it.Next()) {
        if (face_idx >= face_rgbs.size()) {
            return;
        }
        const TopoDS_Face& face = TopoDS::Face(it.Current());
        if (append_face(face, face_rgbs[face_idx], buffer)) {
            buffer.face_starts.push_back(buffer.indices.size());
        }
        ++face_idx;
    }
    if (face_idx != face_rgbs.size()) {
        return;
    }

    std::vector<uint32_t> edges;
    if (generate_boundary_edges) {
        edges = extract_boundary_edges(buffer.indices);
    }

    if (weld_cross_face && !buffer.indices.empty()) {
        const size_t vertex_count = buffer.positions.size() / 3;
        std::vector<uint32_t> vertex_face(vertex_count, 0);

        const size_t face_count = buffer.face_normals.size();
        for (size_t f = 0; f < face_count; ++f) {
            const size_t start = buffer.face_starts[f];
            const size_t end = buffer.face_starts[f + 1];
            for (size_t i = start; i < end; ++i) {
                vertex_face[buffer.indices[i]] = static_cast<uint32_t>(f);
            }
        }

        std::vector<uint32_t> remap =
            weld_vertices(buffer, vertex_face, weld_angle_cos_threshold);

        for (auto& v : edges) {
            v = remap[v];
        }

        std::vector<uint32_t> filtered;
        filtered.reserve(edges.size());
        for (size_t i = 0; i + 1 < edges.size(); i += 2) {
            if (edges[i] != edges[i + 1]) {
                filtered.push_back(edges[i]);
                filtered.push_back(edges[i + 1]);
            }
        }
        edges = std::move(filtered);
    }

    std::string name = "Body " + std::to_string(index + 1);
    finalize_body(buffer, name, edges, out);
}

void process_top_level(
    const TopoDS_Shape& shape,
    const TDF_Label& xcaf_root_label,
    std::vector<PrintcadOcctBody>& out,
    int index,
    bool weld_cross_face,
    float weld_angle_cos_threshold,
    bool generate_boundary_edges,
    const Handle(XCAFDoc_ColorTool)& color_tool,
    const Handle(XCAFDoc_VisMaterialTool)& vis_tool,
    const Handle(XCAFDoc_ShapeTool)& shape_tool) {
    std::vector<std::array<float, 3>> rgbs =
        collect_face_display_rgbs(shape, xcaf_root_label, color_tool, vis_tool, shape_tool);
    mesh_shape_from_precolored_faces(
        shape, rgbs, out, index, weld_cross_face, weld_angle_cos_threshold, generate_boundary_edges);
}

static void shape_bbox_floats(const TopoDS_Shape& shape, float* bbox_min, float* bbox_max) {
    Bnd_Box box;
    BRepBndLib::Add(shape, box);
    Standard_Real xmin;
    Standard_Real ymin;
    Standard_Real zmin;
    Standard_Real xmax;
    Standard_Real ymax;
    Standard_Real zmax;
    box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
    bbox_min[0] = static_cast<float>(xmin);
    bbox_min[1] = static_cast<float>(ymin);
    bbox_min[2] = static_cast<float>(zmin);
    bbox_max[0] = static_cast<float>(xmax);
    bbox_max[1] = static_cast<float>(ymax);
    bbox_max[2] = static_cast<float>(zmax);
}

static bool brep_write_to_vector(const TopoDS_Shape& shape, std::vector<uint8_t>& out) {
    std::ostringstream oss;
    BRepTools::Write(shape, oss);
    const std::string& s = oss.str();
    if (s.empty()) {
        return false;
    }
    out.assign(reinterpret_cast<const uint8_t*>(s.data()),
               reinterpret_cast<const uint8_t*>(s.data()) + s.size());
    return true;
}

static bool brep_read_from_bytes(const uint8_t* data, size_t len, TopoDS_Shape& shape) {
    if (data == nullptr || len == 0) {
        return false;
    }
    const std::string s(reinterpret_cast<const char*>(data), len);
    std::istringstream iss(s);
    BRep_Builder builder;
    BRepTools::Read(shape, iss, builder);
    return !shape.IsNull();
}

static PrintcadOcctBrepImportResult make_brep_error(const std::string& message) {
    PrintcadOcctBrepImportResult result{};
    result.bodies = nullptr;
    result.body_count = 0;
    result.nodes = nullptr;
    result.node_count = 0;
    result.error = duplicate_to_malloc(message);
    return result;
}

static size_t count_faces(const TopoDS_Shape& shape) {
    size_t n = 0;
    for (TopExp_Explorer it(shape, TopAbs_FACE); it.More(); it.Next()) {
        ++n;
    }
    return n;
}

bool try_read_step_xcaf(
    const char* utf8_path,
    std::vector<TopoDS_Shape>& out_shapes,
    std::vector<TDF_Label>& out_root_labels,
    Handle(TDocStd_Document)& out_doc,
    Handle(XCAFDoc_ColorTool)& out_colors,
    Handle(XCAFDoc_ShapeTool)& out_shape_tool,
    Handle(XCAFDoc_VisMaterialTool)& out_vis_tool) {
    out_shapes.clear();
    out_root_labels.clear();
    out_doc.Nullify();
    out_colors.Nullify();
    out_shape_tool.Nullify();
    out_vis_tool.Nullify();

    Handle(XCAFApp_Application) app = XCAFApp_Application::GetApplication();
    Handle(TDocStd_Document) doc;
    app->NewDocument("MDTV-XCAF", doc);

    STEPCAFControl_Reader reader;
    reader.SetColorMode(Standard_True);
    reader.SetNameMode(Standard_True);
    reader.SetLayerMode(Standard_True);
    reader.SetPropsMode(Standard_True);
    reader.SetMatMode(Standard_True);
    reader.SetViewMode(Standard_True);
    reader.SetSHUOMode(Standard_True);
    const IFSelect_ReturnStatus st = reader.ReadFile(utf8_path);
    if (st != IFSelect_RetDone) {
        return false;
    }
    if (!reader.Transfer(doc)) {
        return false;
    }

    Handle(XCAFDoc_ShapeTool) shape_tool = XCAFDoc_DocumentTool::ShapeTool(doc->Main());
    // Synchronise assembly locations / composites so `FindComponent` and instance
    // colours resolve the same as OCCT desktop viewers after AP242 transfer.
    shape_tool->UpdateAssemblies();
    out_doc = doc;
    out_shape_tool = shape_tool;
    out_colors = XCAFDoc_DocumentTool::ColorTool(doc->Main());
    out_vis_tool = XCAFDoc_DocumentTool::VisMaterialTool(doc->Main());

    TDF_LabelSequence labels;
    shape_tool->GetFreeShapes(labels);
    if (labels.Length() <= 0) {
        out_shapes.clear();
        out_root_labels.clear();
        out_doc.Nullify();
        out_colors.Nullify();
        out_shape_tool.Nullify();
        out_vis_tool.Nullify();
        return false;
    }

    // Build the subshape↔label map under each free-shape root so
    // `FindSubShape` / per-face colour attributes from the STEP CAF tree
    // resolve (same as most OCCT-based STEP viewers after transfer).
    for (Standard_Integer i = 1; i <= labels.Length(); ++i) {
        shape_tool->ComputeShapes(labels.Value(i));
    }

    std::vector<TDF_Label> body_labels;
    std::unordered_set<std::string> seen_label_entries;
    for (Standard_Integer i = 1; i <= labels.Length(); ++i) {
        collect_body_labels_recursive(labels.Value(i), shape_tool, seen_label_entries, body_labels);
    }
    if (body_labels.empty()) {
        for (Standard_Integer i = 1; i <= labels.Length(); ++i) {
            body_labels.push_back(labels.Value(i));
        }
    }

    for (const TDF_Label& body_label : body_labels) {
        const TopoDS_Shape sh = shape_tool->GetShape(body_label);
        if (!sh.IsNull()) {
            out_shapes.push_back(sh);
            out_root_labels.push_back(body_label);
        }
    }
    if (out_shapes.empty()) {
        out_root_labels.clear();
        out_doc.Nullify();
        out_colors.Nullify();
        out_shape_tool.Nullify();
        out_vis_tool.Nullify();
        return false;
    }
    return true;
}

// Bounding-box scaled linear deflection: (dx + dy + dz) / 300 × multiplier.
// Yields a chord height that scales with model size so coarse parts and
// detailed parts share comparable triangle density without per-model tuning.
static double bbox_linear_deflection(const TopoDS_Shape& shape, double mesh_deviation) {
    Bnd_Box bounds;
    BRepBndLib::Add(shape, bounds);
    bounds.SetGap(0.0);
    Standard_Real xMin = 0.0, yMin = 0.0, zMin = 0.0, xMax = 0.0, yMax = 0.0, zMax = 0.0;
    bounds.Get(xMin, yMin, zMin, xMax, yMax, zMax);
    const double dx = static_cast<double>(xMax - xMin);
    const double dy = static_cast<double>(yMax - yMin);
    const double dz = static_cast<double>(zMax - zMin);
    return ((dx + dy + dz) / 300.0) * mesh_deviation;
}

// Incremental BRep mesher tuned for viewport display: clean any stale poly
// representation, then run BRepMesh with absolute deflection + angular limit
// in parallel. Quality decrease is allowed so OCCT can pull back on
// pathological faces rather than spinning indefinitely.
static void brepmesh_incremental(TopoDS_Shape& shape, double linear_abs, double angular_rad) {
#if OCC_VERSION_HEX >= 0x070600
    BRepTools::Clean(shape, Standard_True);
#else
    BRepTools::Clean(shape);
#endif
    Standard_Real deflection = linear_abs;
    if (deflection < gp::Resolution()) {
        deflection = Precision::Confusion();
    }
    const Standard_Real angle =
        angular_rad > 0.0 ? static_cast<Standard_Real>(angular_rad) : 0.5;

    IMeshTools_Parameters mesh_params;
    mesh_params.Deflection = deflection;
    mesh_params.Relative = Standard_False;
    mesh_params.Angle = angle;
    mesh_params.InParallel = Standard_True;
    mesh_params.AllowQualityDecrease = Standard_True;

    BRepMesh_IncrementalMesh mesher(shape, mesh_params);
    mesher.Perform();
}

// linear_deflection_mode: 0 = bbox-scaled (linear_value = mesh deviation pref), 1 = absolute.
static double resolve_linear_deflection_abs(
    const TopoDS_Shape& shape,
    int linear_deflection_mode,
    double linear_value) {
    if (linear_deflection_mode != 0) {
        return linear_value > 0.0 ? linear_value : 0.5;
    }
    const double mult = linear_value > 0.0 ? linear_value : 0.2;
    return bbox_linear_deflection(shape, mult);
}

// ---- Sketch-profile solid sweeps (pad/pocket/revolve/groove) ----

static PrintcadOcctSweepResult make_sweep_error(const std::string& message) {
    PrintcadOcctSweepResult result{};
    result.error = duplicate_to_malloc(message);
    return result;
}

// Map a sketch-plane (u, v) coordinate to world space via the profile plane.
static gp_Pnt profile_point(const PcadProfilePlane& plane, double u, double v) {
    return gp_Pnt(
        plane.origin[0] + u * plane.x_axis[0] + v * plane.y_axis[0],
        plane.origin[1] + u * plane.x_axis[1] + v * plane.y_axis[1],
        plane.origin[2] + u * plane.x_axis[2] + v * plane.y_axis[2]);
}

// Ellipse frame for segment kinds 3/4: world-space gp_Elips from a sketch-uv
// center, major-vertex vector, and minor/major ratio.
static bool profile_ellipse(
    const PcadProfilePlane& plane,
    const gp_Dir& normal_dir,
    const PcadProfileSegment& seg,
    gp_Elips& out,
    std::string& err) {
    const double major_len = std::hypot(seg.d[2], seg.d[3]);
    const double ratio = seg.d[4];
    if (!(major_len > Precision::Confusion()) || !(ratio > 0.0) || ratio > 1.0) {
        err = "profile ellipse needs a positive major axis and ratio in (0, 1]";
        return false;
    }
    const gp_Pnt center = profile_point(plane, seg.d[0], seg.d[1]);
    const gp_Pnt major_vertex =
        profile_point(plane, seg.d[0] + seg.d[2], seg.d[1] + seg.d[3]);
    const gp_Dir major_dir(gp_Vec(center, major_vertex));
    out = gp_Elips(gp_Ax2(center, normal_dir, major_dir), major_len, major_len * ratio);
    return true;
}

// Cubic (or lower-degree) B-spline edge through flat (u, v) control points.
static bool profile_bspline_edge(
    const PcadProfilePlane& plane,
    const PcadProfileSegment& seg,
    TopoDS_Edge& out,
    std::string& err) {
    const bool periodic = seg.d[0] != 0.0;
    const size_t n = seg.extra_count / 2;
    if (seg.extra == nullptr || n < 2 || (periodic && n < 3)) {
        err = "profile B-spline needs at least 2 control points (3 when periodic)";
        return false;
    }
    TColgp_Array1OfPnt poles(1, static_cast<Standard_Integer>(n));
    for (size_t i = 0; i < n; ++i) {
        poles.SetValue(
            static_cast<Standard_Integer>(i + 1),
            profile_point(plane, seg.extra[2 * i], seg.extra[2 * i + 1]));
    }
    const Standard_Integer degree =
        std::min<Standard_Integer>(3, static_cast<Standard_Integer>(n) - (periodic ? 0 : 1));
    Handle(Geom_BSplineCurve) curve;
    if (periodic) {
        // Periodic form: one knot per pole plus the wrap knot, all simple.
        const Standard_Integer nk = static_cast<Standard_Integer>(n) + 1;
        TColStd_Array1OfReal knots(1, nk);
        TColStd_Array1OfInteger mults(1, nk);
        for (Standard_Integer i = 1; i <= nk; ++i) {
            knots.SetValue(i, static_cast<Standard_Real>(i - 1));
            mults.SetValue(i, 1);
        }
        curve = new Geom_BSplineCurve(poles, knots, mults, degree, Standard_True);
    } else {
        // Clamped uniform: end knots at multiplicity degree+1.
        const Standard_Integer nk = static_cast<Standard_Integer>(n) - degree + 1;
        TColStd_Array1OfReal knots(1, nk);
        TColStd_Array1OfInteger mults(1, nk);
        for (Standard_Integer i = 1; i <= nk; ++i) {
            knots.SetValue(i, static_cast<Standard_Real>(i - 1) / (nk - 1));
            mults.SetValue(i, (i == 1 || i == nk) ? degree + 1 : 1);
        }
        curve = new Geom_BSplineCurve(poles, knots, mults, degree, Standard_False);
    }
    out = BRepBuilderAPI_MakeEdge(curve).Edge();
    return !out.IsNull();
}

// Build one TopoDS_Wire from a profile wire description. Returns false with a
// diagnostic in `err` on degenerate or disconnected segments, or — unless
// `require_closed` is false (spine paths) — an open loop.
static bool build_profile_wire(
    const PcadProfilePlane& plane,
    const gp_Dir& normal_dir,
    const gp_Dir& x_dir,
    const PcadProfileWire& wire_in,
    TopoDS_Wire& out_wire,
    std::string& err,
    bool require_closed = true) {
    if (wire_in.segments == nullptr || wire_in.count == 0) {
        err = "profile wire has no segments";
        return false;
    }
    BRepBuilderAPI_MakeWire maker;
    for (size_t i = 0; i < wire_in.count; ++i) {
        const PcadProfileSegment& seg = wire_in.segments[i];
        TopoDS_Edge edge;
        switch (seg.kind) {
            case 0: { // line: start -> end
                const gp_Pnt p1 = profile_point(plane, seg.d[0], seg.d[1]);
                const gp_Pnt p2 = profile_point(plane, seg.d[2], seg.d[3]);
                if (p1.Distance(p2) <= Precision::Confusion()) {
                    err = "degenerate (zero-length) line segment in profile";
                    return false;
                }
                GC_MakeSegment mk(p1, p2);
                if (!mk.IsDone()) {
                    err = "failed to build line segment for profile";
                    return false;
                }
                edge = BRepBuilderAPI_MakeEdge(mk.Value()).Edge();
                break;
            }
            case 1: { // arc through three on-curve points: start -> mid -> end
                const gp_Pnt p1 = profile_point(plane, seg.d[0], seg.d[1]);
                const gp_Pnt pm = profile_point(plane, seg.d[2], seg.d[3]);
                const gp_Pnt p2 = profile_point(plane, seg.d[4], seg.d[5]);
                GC_MakeArcOfCircle mk(p1, pm, p2);
                if (!mk.IsDone()) {
                    err = "failed to build arc segment for profile "
                          "(points may be collinear or coincident)";
                    return false;
                }
                edge = BRepBuilderAPI_MakeEdge(mk.Value()).Edge();
                break;
            }
            case 2: { // full circle: center + radius
                const double radius = seg.d[2];
                if (!(radius > Precision::Confusion())) {
                    err = "profile circle radius must be positive";
                    return false;
                }
                const gp_Pnt center = profile_point(plane, seg.d[0], seg.d[1]);
                const gp_Circ circ(gp_Ax2(center, normal_dir, x_dir), radius);
                edge = BRepBuilderAPI_MakeEdge(circ).Edge();
                break;
            }
            case 3: { // full ellipse
                gp_Elips elips;
                if (!profile_ellipse(plane, normal_dir, seg, elips, err)) {
                    return false;
                }
                edge = BRepBuilderAPI_MakeEdge(elips).Edge();
                break;
            }
            case 4: { // elliptical arc between two parameter angles
                if (seg.extra == nullptr || seg.extra_count < 2) {
                    err = "elliptical arc segment is missing its parameter range";
                    return false;
                }
                gp_Elips elips;
                if (!profile_ellipse(plane, normal_dir, seg, elips, err)) {
                    return false;
                }
                GC_MakeArcOfEllipse mk(elips, seg.extra[0], seg.extra[1], Standard_True);
                if (!mk.IsDone()) {
                    err = "failed to build elliptical arc segment for profile";
                    return false;
                }
                edge = BRepBuilderAPI_MakeEdge(mk.Value()).Edge();
                break;
            }
            case 5: { // cubic B-spline through control points
                if (!profile_bspline_edge(plane, seg, edge, err)) {
                    return false;
                }
                break;
            }
            default:
                err = "unknown profile segment kind " + std::to_string(seg.kind);
                return false;
        }
        maker.Add(edge);
        if (!maker.IsDone()) {
            err = "profile segments do not form a connected wire";
            return false;
        }
    }
    TopoDS_Wire wire = maker.Wire();
    if (wire.IsNull()) {
        err = "profile wire construction failed";
        return false;
    }
    if (require_closed && !BRep_Tool::IsClosed(wire)) {
        err = "profile wire is not closed";
        return false;
    }
    out_wire = wire;
    return true;
}

// Absolute enclosed area of a single closed planar wire (0 when the face
// cannot be built).
static double planar_wire_area(const gp_Pln& pln, const TopoDS_Wire& wire) {
    BRepBuilderAPI_MakeFace maker(pln, wire, Standard_True);
    if (!maker.IsDone()) {
        return 0.0;
    }
    GProp_GProps props;
    BRepGProp::SurfaceProperties(maker.Face(), props);
    return std::abs(props.Mass());
}

// One outer boundary and the hole wires directly nested inside it.
struct WireGroup {
    TopoDS_Wire outer;
    std::vector<TopoDS_Wire> holes;
};

// True when `inner` lies inside the face bounded by `container` alone.
static bool wire_inside_wire(
    const gp_Pln& pln,
    const TopoDS_Wire& inner,
    const TopoDS_Wire& container) {
    BRepBuilderAPI_MakeFace maker(pln, container, Standard_True);
    if (!maker.IsDone()) {
        return false;
    }
    TopExp_Explorer vertex_it(inner, TopAbs_VERTEX);
    if (!vertex_it.More()) {
        return false;
    }
    const gp_Pnt probe = BRep_Tool::Pnt(TopoDS::Vertex(vertex_it.Current()));
    BRepClass_FaceClassifier classifier(maker.Face(), probe, Precision::Confusion());
    return classifier.State() == TopAbs_IN;
}

// Group wires by containment: a wire nested at even depth is an outer
// boundary (disjoint wires become separate solids); odd depth makes it a
// hole of its immediate container.
static bool group_profile_wires(
    const gp_Pln& pln,
    const std::vector<TopoDS_Wire>& wires,
    std::vector<WireGroup>& out_groups,
    std::string& err) {
    const size_t n = wires.size();
    std::vector<double> areas(n);
    for (size_t i = 0; i < n; ++i) {
        areas[i] = planar_wire_area(pln, wires[i]);
        if (!(areas[i] > 0.0)) {
            err = "a profile wire encloses no area";
            return false;
        }
    }

    // Immediate container = smallest-area wire strictly containing this one.
    std::vector<int> parent(n, -1);
    std::vector<int> depth(n, 0);
    for (size_t i = 0; i < n; ++i) {
        for (size_t j = 0; j < n; ++j) {
            if (i == j || areas[j] <= areas[i]) {
                continue;
            }
            if (!wire_inside_wire(pln, wires[i], wires[j])) {
                continue;
            }
            if (parent[i] < 0 || areas[j] < areas[static_cast<size_t>(parent[i])]) {
                parent[i] = static_cast<int>(j);
            }
        }
    }
    for (size_t i = 0; i < n; ++i) {
        int p = parent[i];
        while (p >= 0) {
            ++depth[i];
            p = parent[static_cast<size_t>(p)];
        }
    }

    std::vector<int> group_of(n, -1);
    for (size_t i = 0; i < n; ++i) {
        if (depth[i] % 2 == 0) {
            group_of[i] = static_cast<int>(out_groups.size());
            out_groups.push_back(WireGroup{wires[i], {}});
        }
    }
    for (size_t i = 0; i < n; ++i) {
        if (depth[i] % 2 == 1) {
            const int container = parent[i];
            out_groups[static_cast<size_t>(group_of[static_cast<size_t>(container)])]
                .holes.push_back(wires[i]);
        }
    }
    return true;
}

// Face for one wire group (outer boundary + its holes), hole orientation
// fixed by ShapeFix_Face.
static bool build_group_face(
    const gp_Pln& pln,
    const WireGroup& group,
    TopoDS_Face& out_face,
    std::string& err) {
    BRepBuilderAPI_MakeFace maker(pln, group.outer, Standard_True);
    if (!maker.IsDone()) {
        err = "failed to build profile face from outer wire";
        return false;
    }
    for (const TopoDS_Wire& hole : group.holes) {
        maker.Add(hole);
        if (!maker.IsDone()) {
            err = "failed to add hole wire to profile face";
            return false;
        }
    }
    TopoDS_Face face = maker.Face();

    ShapeFix_Face fixer(face);
    fixer.Perform();
    if (!fixer.Face().IsNull()) {
        face = fixer.Face();
    }
    if (face.IsNull()) {
        err = "profile face construction produced a null face";
        return false;
    }
    out_face = face;
    return true;
}

// Build the profile as one face per outer wire (a compound when the profile
// has several disjoint regions).
static bool build_profile_face(
    const gp_Pln& pln,
    const std::vector<TopoDS_Wire>& wires,
    TopoDS_Shape& out_shape,
    std::vector<WireGroup>& out_groups,
    std::string& err) {
    if (!group_profile_wires(pln, wires, out_groups, err)) {
        return false;
    }
    if (out_groups.size() == 1) {
        TopoDS_Face face;
        if (!build_group_face(pln, out_groups[0], face, err)) {
            return false;
        }
        out_shape = face;
        return true;
    }
    TopoDS_Compound compound;
    BRep_Builder builder;
    builder.MakeCompound(compound);
    for (const WireGroup& group : out_groups) {
        TopoDS_Face face;
        if (!build_group_face(pln, group, face, err)) {
            return false;
        }
        builder.Add(compound, face);
    }
    out_shape = compound;
    return true;
}

} // namespace

static void free_import_nodes_vec(std::vector<PrintcadOcctImportNode>& nodes);

extern "C" PrintcadOcctImportResult printcad_occt_import_step(
    const char* utf8_path,
    int linear_deflection_mode,
    double linear_value,
    double angular_deflection_rad,
    int weld_cross_face,
    double weld_angle_threshold_rad,
    int generate_boundary_edges) {
    if (utf8_path == nullptr) {
        return make_error("STEP path is null");
    }

    try {
        using clock = std::chrono::steady_clock;
        const auto t_cpp_start = clock::now();

        std::vector<TopoDS_Shape> top_level;
        std::vector<TDF_Label> xcaf_root_labels;
        Handle(TDocStd_Document) xcaf_doc;
        Handle(XCAFDoc_ColorTool) color_tool;
        Handle(XCAFDoc_ShapeTool) shape_tool;
        Handle(XCAFDoc_VisMaterialTool) vis_tool;
        const bool xcaf_ok = try_read_step_xcaf(
            utf8_path,
            top_level,
            xcaf_root_labels,
            xcaf_doc,
            color_tool,
            shape_tool,
            vis_tool);

        if (!xcaf_ok) {
            STEPControl_Reader reader;
            const IFSelect_ReturnStatus status = reader.ReadFile(utf8_path);
            if (status != IFSelect_RetDone) {
                return make_error(
                    std::string("STEP read failed (status ")
                    + std::to_string(static_cast<int>(status)) + ")");
            }

            const Standard_Integer transferred = reader.TransferRoots();
            if (transferred <= 0) {
                return make_error("STEP file contained no transferable roots");
            }

            const Standard_Integer shape_count = reader.NbShapes();
            if (shape_count <= 0) {
                return make_error("STEP file produced no shapes");
            }

            top_level.reserve(static_cast<size_t>(shape_count));
            for (Standard_Integer i = 1; i <= shape_count; ++i) {
                top_level.push_back(reader.Shape(i));
            }
            xcaf_root_labels.clear();
            color_tool.Nullify();
            shape_tool.Nullify();
            vis_tool.Nullify();
        }

        const auto t_after_read_transfer = clock::now();

        for (auto& shape : top_level) {
            const double linear_abs = resolve_linear_deflection_abs(
                shape,
                linear_deflection_mode,
                linear_value);
            const double ang =
                angular_deflection_rad > 0.0 ? angular_deflection_rad : 0.5;
            brepmesh_incremental(shape, linear_abs, ang);
        }
        const auto t_after_brepmesh = clock::now();

        const float weld_angle_cos =
            static_cast<float>(std::cos(std::max(0.0, weld_angle_threshold_rad)));
        const bool do_weld = weld_cross_face != 0;
        const bool do_edges = generate_boundary_edges != 0;

        std::vector<PrintcadOcctBody> bodies;
        bodies.reserve(top_level.size());
        for (size_t i = 0; i < top_level.size(); ++i) {
            const TDF_Label root_lab =
                (xcaf_ok && i < xcaf_root_labels.size()) ? xcaf_root_labels[i] : TDF_Label();
            process_top_level(
                top_level[i],
                root_lab,
                bodies,
                static_cast<int>(i),
                do_weld,
                weld_angle_cos,
                do_edges,
                color_tool,
                vis_tool,
                shape_tool);
        }
        const auto t_after_extract = clock::now();

        const double read_ms = ms_between(t_cpp_start, t_after_read_transfer);
        const double brepmesh_ms = ms_between(t_after_read_transfer, t_after_brepmesh);
        const double extract_ms = ms_between(t_after_brepmesh, t_after_extract);
        const double total_cpp_ms = ms_between(t_cpp_start, t_after_extract);
        std::fprintf(
            stderr,
            "[printcad_import_cpp] read_transfer=%.1fms brepmesh=%.1fms "
            "tessellate_weld_extract=%.1fms total_cpp=%.1fms xcaf=%d file=%s\n",
            read_ms,
            brepmesh_ms,
            extract_ms,
            total_cpp_ms,
            xcaf_ok ? 1 : 0,
            utf8_path);

        if (bodies.empty()) {
            return make_error("STEP file produced no triangulated geometry");
        }

        std::vector<PrintcadOcctImportNode> nodes =
            build_import_nodes(xcaf_root_labels, shape_tool, color_tool);

        PrintcadOcctImportResult result{};
        result.body_count = bodies.size();
        result.bodies = static_cast<PrintcadOcctBody*>(
            std::malloc(bodies.size() * sizeof(PrintcadOcctBody)));
        result.nodes = nullptr;
        result.node_count = 0;
        if (result.bodies == nullptr) {
            for (auto& body : bodies) {
                std::free(body.positions);
                std::free(body.normals);
                std::free(body.colors);
                std::free(body.indices);
                std::free(body.edges);
                std::free(body.name);
            }
            free_import_nodes_vec(nodes);
            return make_error("Out of memory while building STEP import result");
        }
        std::memcpy(result.bodies, bodies.data(), bodies.size() * sizeof(PrintcadOcctBody));
        result.node_count = nodes.size();
        result.nodes = nullptr;
        if (!nodes.empty()) {
            result.nodes = static_cast<PrintcadOcctImportNode*>(
                std::malloc(nodes.size() * sizeof(PrintcadOcctImportNode)));
            if (result.nodes == nullptr) {
                for (auto& body : bodies) {
                    std::free(body.positions);
                    std::free(body.normals);
                    std::free(body.colors);
                    std::free(body.indices);
                    std::free(body.edges);
                    std::free(body.name);
                }
                std::free(result.bodies);
                free_import_nodes_vec(nodes);
                return make_error("Out of memory while building STEP import nodes");
            }
            std::memcpy(result.nodes, nodes.data(), nodes.size() * sizeof(PrintcadOcctImportNode));
        }
        result.error = nullptr;
        return result;
    } catch (Standard_Failure const& ex) {
        return make_error(std::string("OCCT exception: ") + ex.GetMessageString());
    } catch (std::exception const& ex) {
        return make_error(std::string("std::exception: ") + ex.what());
    } catch (...) {
        return make_error("Unknown exception while importing STEP file");
    }
}

static void free_brep_bodies_vec(std::vector<PrintcadOcctBrepBody>& bodies) {
    for (auto& b : bodies) {
        std::free(b.name);
        std::free(b.brep_blob);
        std::free(b.face_colors);
        std::free(b.mesh_positions);
        std::free(b.mesh_normals);
        std::free(b.mesh_colors);
        std::free(b.mesh_indices);
        std::free(b.mesh_edges);
    }
    bodies.clear();
}

static void free_import_nodes_vec(std::vector<PrintcadOcctImportNode>& nodes) {
    for (auto& n : nodes) {
        std::free(n.name);
        n.name = nullptr;
    }
    nodes.clear();
}

extern "C" PrintcadOcctBrepImportResult printcad_occt_import_step_brep(
    const char* utf8_path,
    int serialize_brep,
    int linear_deflection_mode,
    double linear_value,
    double angular_deflection_rad,
    int weld_cross_face,
    double weld_angle_threshold_rad,
    int generate_boundary_edges) {
    if (utf8_path == nullptr) {
        return make_brep_error("STEP path is null");
    }

    try {
        using clock = std::chrono::steady_clock;
        const auto t_cpp_start = clock::now();

        std::vector<TopoDS_Shape> top_level;
        std::vector<TDF_Label> xcaf_root_labels;
        Handle(TDocStd_Document) xcaf_doc;
        Handle(XCAFDoc_ColorTool) color_tool;
        Handle(XCAFDoc_ShapeTool) shape_tool;
        Handle(XCAFDoc_VisMaterialTool) vis_tool;
        const bool xcaf_ok = try_read_step_xcaf(
            utf8_path,
            top_level,
            xcaf_root_labels,
            xcaf_doc,
            color_tool,
            shape_tool,
            vis_tool);

        if (!xcaf_ok) {
            STEPControl_Reader reader;
            const IFSelect_ReturnStatus status = reader.ReadFile(utf8_path);
            if (status != IFSelect_RetDone) {
                return make_brep_error(
                    std::string("STEP read failed (status ")
                    + std::to_string(static_cast<int>(status)) + ")");
            }

            const Standard_Integer transferred = reader.TransferRoots();
            if (transferred <= 0) {
                return make_brep_error("STEP file contained no transferable roots");
            }

            const Standard_Integer shape_count = reader.NbShapes();
            if (shape_count <= 0) {
                return make_brep_error("STEP file produced no shapes");
            }

            top_level.reserve(static_cast<size_t>(shape_count));
            for (Standard_Integer i = 1; i <= shape_count; ++i) {
                top_level.push_back(reader.Shape(i));
            }
            xcaf_root_labels.clear();
            color_tool.Nullify();
            shape_tool.Nullify();
            vis_tool.Nullify();
        }

        const auto t_after_read_transfer = clock::now();

        const float weld_angle_cos =
            static_cast<float>(std::cos(std::max(0.0, weld_angle_threshold_rad)));
        const bool do_weld = weld_cross_face != 0;
        const bool do_edges = generate_boundary_edges != 0;

        std::vector<PrintcadOcctBrepBody> bodies;
        bodies.reserve(top_level.size());
        for (size_t i = 0; i < top_level.size(); ++i) {
            const TDF_Label root_lab =
                (xcaf_ok && i < xcaf_root_labels.size()) ? xcaf_root_labels[i] : TDF_Label();
            std::vector<std::array<float, 3>> rgbs = collect_face_display_rgbs(
                top_level[i], root_lab, color_tool, vis_tool, shape_tool);

            PrintcadOcctBrepBody entry{};
            shape_bbox_floats(top_level[i], entry.bbox_min, entry.bbox_max);

            const std::string name = "Body " + std::to_string(i + 1);
            entry.name = duplicate_to_malloc(name);
            if (entry.name == nullptr) {
                free_brep_bodies_vec(bodies);
                return make_brep_error("Out of memory while duplicating body name");
            }

            entry.face_count = rgbs.size();
            entry.face_colors =
                entry.face_count == 0
                    ? nullptr
                    : static_cast<float*>(std::malloc(entry.face_count * 3 * sizeof(float)));
            if (entry.face_count > 0 && entry.face_colors == nullptr) {
                std::free(entry.name);
                free_brep_bodies_vec(bodies);
                return make_brep_error("Out of memory while allocating face colour snapshot");
            }
            for (size_t f = 0; f < rgbs.size(); ++f) {
                entry.face_colors[f * 3 + 0] = rgbs[f][0];
                entry.face_colors[f * 3 + 1] = rgbs[f][1];
                entry.face_colors[f * 3 + 2] = rgbs[f][2];
            }

            if (serialize_brep != 0) {
                std::vector<uint8_t> brep_bin;
                if (!brep_write_to_vector(top_level[i], brep_bin)) {
                    std::free(entry.face_colors);
                    std::free(entry.name);
                    free_brep_bodies_vec(bodies);
                    return make_brep_error("BRepTools::Write failed while serializing a body");
                }
                entry.brep_len = brep_bin.size();
                entry.brep_blob =
                    brep_bin.empty() ? nullptr
                                     : static_cast<uint8_t*>(std::malloc(brep_bin.size()));
                if (!brep_bin.empty() && entry.brep_blob == nullptr) {
                    std::free(entry.face_colors);
                    std::free(entry.name);
                    free_brep_bodies_vec(bodies);
                    return make_brep_error("Out of memory while allocating BRep blob");
                }
                if (!brep_bin.empty()) {
                    std::memcpy(entry.brep_blob, brep_bin.data(), brep_bin.size());
                }
            } else {
                entry.brep_blob = nullptr;
                entry.brep_len = 0;
                TopoDS_Shape& sh = top_level[i];
                const double linear_abs =
                    resolve_linear_deflection_abs(sh, linear_deflection_mode, linear_value);
                const double ang =
                    angular_deflection_rad > 0.0 ? angular_deflection_rad : 0.5;
                brepmesh_incremental(sh, linear_abs, ang);

                std::vector<PrintcadOcctBody> meshed_bodies;
                mesh_shape_from_precolored_faces(
                    sh,
                    rgbs,
                    meshed_bodies,
                    static_cast<int>(i),
                    do_weld,
                    weld_angle_cos,
                    do_edges);
                if (meshed_bodies.size() != 1 || meshed_bodies[0].vertex_count == 0) {
                    std::free(entry.face_colors);
                    std::free(entry.name);
                    free_brep_bodies_vec(bodies);
                    return make_brep_error("inline tessellation produced no mesh geometry");
                }
                PrintcadOcctBody& mb = meshed_bodies[0];
                std::free(mb.name);
                mb.name = nullptr;

                entry.mesh_positions = mb.positions;
                entry.mesh_normals = mb.normals;
                entry.mesh_colors = mb.colors;
                entry.mesh_indices = mb.indices;
                entry.mesh_edges = mb.edges;
                entry.mesh_vertex_count = mb.vertex_count;
                entry.mesh_index_count = mb.index_count;
                entry.mesh_edge_count = mb.edge_count;

                mb.positions = nullptr;
                mb.normals = nullptr;
                mb.colors = nullptr;
                mb.indices = nullptr;
                mb.edges = nullptr;
                mb.vertex_count = 0;
                mb.index_count = 0;
                mb.edge_count = 0;
            }

            bodies.push_back(entry);
        }

        const auto t_after_snapshot = clock::now();
        const double read_ms = ms_between(t_cpp_start, t_after_read_transfer);
        const double extra_ms = ms_between(t_after_read_transfer, t_after_snapshot);
        const double total_cpp_ms = ms_between(t_cpp_start, t_after_snapshot);
        if (serialize_brep != 0) {
            std::fprintf(
                stderr,
                "[printcad_import_brep_cpp] read_transfer=%.1fms brep_snapshot_serialize=%.1fms "
                "total_cpp=%.1fms xcaf=%d file=%s\n",
                read_ms,
                extra_ms,
                total_cpp_ms,
                xcaf_ok ? 1 : 0,
                utf8_path);
        } else {
            std::fprintf(
                stderr,
                "[printcad_import_brep_cpp] read_transfer=%.1fms inline_mesh_ms=%.1fms "
                "total_cpp=%.1fms xcaf=%d file=%s\n",
                read_ms,
                extra_ms,
                total_cpp_ms,
                xcaf_ok ? 1 : 0,
                utf8_path);
        }

        if (bodies.empty()) {
            return make_brep_error("STEP file produced no bodies");
        }

        std::vector<PrintcadOcctImportNode> nodes =
            build_import_nodes(xcaf_root_labels, shape_tool, color_tool);

        PrintcadOcctBrepImportResult result{};
        result.body_count = bodies.size();
        result.bodies = static_cast<PrintcadOcctBrepBody*>(
            std::malloc(bodies.size() * sizeof(PrintcadOcctBrepBody)));
        if (result.bodies == nullptr) {
            free_brep_bodies_vec(bodies);
            free_import_nodes_vec(nodes);
            return make_brep_error("Out of memory while building BRep import result");
        }
        std::memcpy(result.bodies, bodies.data(), bodies.size() * sizeof(PrintcadOcctBrepBody));
        result.node_count = nodes.size();
        result.nodes = nullptr;
        if (!nodes.empty()) {
            result.nodes = static_cast<PrintcadOcctImportNode*>(
                std::malloc(nodes.size() * sizeof(PrintcadOcctImportNode)));
            if (result.nodes == nullptr) {
                free_brep_bodies_vec(bodies);
                free_import_nodes_vec(nodes);
                std::free(result.bodies);
                return make_brep_error("Out of memory while building BRep import nodes");
            }
            std::memcpy(result.nodes, nodes.data(), nodes.size() * sizeof(PrintcadOcctImportNode));
        }
        result.error = nullptr;
        return result;
    } catch (Standard_Failure const& ex) {
        return make_brep_error(std::string("OCCT exception: ") + ex.GetMessageString());
    } catch (std::exception const& ex) {
        return make_brep_error(std::string("std::exception: ") + ex.what());
    } catch (...) {
        return make_brep_error("Unknown exception while importing STEP file (brep)");
    }
}

extern "C" PrintcadOcctImportResult printcad_occt_tessellate_brep(
    const uint8_t* brep_bytes,
    size_t brep_len,
    const float* face_colors,
    size_t face_color_count,
    int linear_deflection_mode,
    double linear_value,
    double angular_deflection_rad,
    int weld_cross_face,
    double weld_angle_threshold_rad,
    int generate_boundary_edges) {
    if (brep_bytes == nullptr || brep_len == 0) {
        return make_error("BRep blob is null or empty");
    }
    if (face_colors == nullptr && face_color_count > 0) {
        return make_error("Face colours are null but face_count > 0");
    }

    try {
        using clock = std::chrono::steady_clock;
        const auto t_cpp_start = clock::now();

        TopoDS_Shape shape;
        if (!brep_read_from_bytes(brep_bytes, brep_len, shape) || shape.IsNull()) {
            return make_error("BRepTools::Read failed (invalid or unsupported BRep blob)");
        }

        const size_t n_faces = count_faces(shape);
        if (n_faces != face_color_count) {
            return make_error(
                std::string("face colour count (") + std::to_string(face_color_count)
                + ") does not match BRep face count (" + std::to_string(n_faces) + ")");
        }

        const auto t_after_read = clock::now();

        const double linear_abs =
            resolve_linear_deflection_abs(shape, linear_deflection_mode, linear_value);
        const double ang =
            angular_deflection_rad > 0.0 ? angular_deflection_rad : 0.5;
        brepmesh_incremental(shape, linear_abs, ang);

        const auto t_after_brepmesh = clock::now();

        std::vector<std::array<float, 3>> rgbs;
        rgbs.reserve(face_color_count);
        for (size_t i = 0; i < face_color_count; ++i) {
            rgbs.push_back(std::array<float, 3>{
                face_colors[i * 3 + 0], face_colors[i * 3 + 1], face_colors[i * 3 + 2]});
        }

        const float weld_angle_cos =
            static_cast<float>(std::cos(std::max(0.0, weld_angle_threshold_rad)));
        const bool do_weld = weld_cross_face != 0;
        const bool do_edges = generate_boundary_edges != 0;

        std::vector<PrintcadOcctBody> bodies;
        mesh_shape_from_precolored_faces(
            shape, rgbs, bodies, 0, do_weld, weld_angle_cos, do_edges);

        const auto t_after_extract = clock::now();

        const double read_ms = ms_between(t_cpp_start, t_after_read);
        const double brepmesh_ms = ms_between(t_after_read, t_after_brepmesh);
        const double extract_ms = ms_between(t_after_brepmesh, t_after_extract);
        const double total_cpp_ms = ms_between(t_cpp_start, t_after_extract);
        std::fprintf(
            stderr,
            "[printcad_tessellate_brep_cpp] brep_read=%.1fms brepmesh=%.1fms "
            "tessellate_weld_extract=%.1fms total_cpp=%.1fms\n",
            read_ms,
            brepmesh_ms,
            extract_ms,
            total_cpp_ms);

        if (bodies.empty()) {
            return make_error("Tessellation produced no mesh geometry");
        }

        PrintcadOcctImportResult result{};
        result.body_count = bodies.size();
        result.bodies = static_cast<PrintcadOcctBody*>(
            std::malloc(bodies.size() * sizeof(PrintcadOcctBody)));
        if (result.bodies == nullptr) {
            for (auto& body : bodies) {
                std::free(body.positions);
                std::free(body.normals);
                std::free(body.colors);
                std::free(body.indices);
                std::free(body.edges);
                std::free(body.name);
            }
            return make_error("Out of memory while building tessellation result");
        }
        std::memcpy(result.bodies, bodies.data(), bodies.size() * sizeof(PrintcadOcctBody));
        result.error = nullptr;
        return result;
    } catch (Standard_Failure const& ex) {
        return make_error(std::string("OCCT exception: ") + ex.GetMessageString());
    } catch (std::exception const& ex) {
        return make_error(std::string("std::exception: ") + ex.what());
    } catch (...) {
        return make_error("Unknown exception while tessellating BRep blob");
    }
}

// ---- Solid-op suite ----

static constexpr double kPi = 3.14159265358979323846;

static double deg_to_rad(double deg) {
    return deg * kPi / 180.0;
}

// Serialize the result (and optional tool solid) and tessellate on request.
// Consumes nothing; on failure returns an error result with everything freed.
static PrintcadOcctSweepResult finish_solid_result(
    TopoDS_Shape result_shape,
    const TopoDS_Shape* tool_shape,
    const PcadMeshOptions* mesh) {
    if (count_faces(result_shape) == 0) {
        return make_sweep_error(
            "solid op result contains no faces (boolean removed all material?)");
    }

    std::vector<uint8_t> blob;
    if (!brep_write_to_vector(result_shape, blob)) {
        return make_sweep_error("BRepTools::Write failed while serializing solid result");
    }

    PrintcadOcctSweepResult result{};
    result.brep_len = blob.size();
    result.brep_blob = static_cast<uint8_t*>(std::malloc(blob.size()));
    if (result.brep_blob == nullptr) {
        return make_sweep_error("Out of memory while allocating solid BRep blob");
    }
    std::memcpy(result.brep_blob, blob.data(), blob.size());

    if (tool_shape != nullptr && !tool_shape->IsNull()) {
        std::vector<uint8_t> tool_bytes;
        if (brep_write_to_vector(*tool_shape, tool_bytes)) {
            result.tool_len = tool_bytes.size();
            result.tool_blob = static_cast<uint8_t*>(std::malloc(tool_bytes.size()));
            if (result.tool_blob != nullptr) {
                std::memcpy(result.tool_blob, tool_bytes.data(), tool_bytes.size());
            } else {
                result.tool_len = 0;
            }
        }
    }

    if (mesh != nullptr && mesh->want_mesh != 0) {
        const double linear_abs = resolve_linear_deflection_abs(
            result_shape, mesh->linear_deflection_mode, mesh->linear_value);
        const double ang =
            mesh->angular_deflection_rad > 0.0 ? mesh->angular_deflection_rad : 0.5;
        brepmesh_incremental(result_shape, linear_abs, ang);

        const float weld_angle_cos = static_cast<float>(
            std::cos(std::max(0.0, mesh->weld_angle_threshold_rad)));
        const std::vector<std::array<float, 3>> rgbs(
            count_faces(result_shape), std::array<float, 3>{{1.0f, 1.0f, 1.0f}});
        std::vector<PrintcadOcctBody> bodies;
        mesh_shape_from_precolored_faces(
            result_shape,
            rgbs,
            bodies,
            0,
            mesh->weld_cross_face != 0,
            weld_angle_cos,
            mesh->generate_boundary_edges != 0);
        if (bodies.size() != 1 || bodies[0].vertex_count == 0) {
            for (auto& body : bodies) {
                std::free(body.positions);
                std::free(body.normals);
                std::free(body.colors);
                std::free(body.indices);
                std::free(body.edges);
                std::free(body.name);
            }
            std::free(result.brep_blob);
            std::free(result.tool_blob);
            return make_sweep_error(
                "tessellation of solid result produced no mesh geometry");
        }
        PrintcadOcctBody& mb = bodies[0];
        std::free(mb.name);
        std::free(mb.colors);
        result.mesh_positions = mb.positions;
        result.mesh_normals = mb.normals;
        result.mesh_indices = mb.indices;
        result.mesh_edges = mb.edges;
        result.mesh_vertex_count = mb.vertex_count;
        result.mesh_index_count = mb.index_count;
        result.mesh_edge_count = mb.edge_count;
    }

    result.error = nullptr;
    return result;
}

// Validate the boolean role and (for fuse/cut) load the base solid.
static bool load_base_for_op(
    int32_t op,
    const uint8_t* base_brep,
    size_t base_brep_len,
    TopoDS_Shape& out_base,
    std::string& err) {
    if (op < 0 || op > 2) {
        err = "unknown boolean op " + std::to_string(op)
              + " (expected 0=new, 1=fuse, 2=cut)";
        return false;
    }
    if (op == 0) {
        if (base_brep != nullptr) {
            err = "new-solid op must not receive a base BRep";
            return false;
        }
        return true;
    }
    if (base_brep == nullptr || base_brep_len == 0) {
        err = "fuse/cut op requires a non-empty base BRep";
        return false;
    }
    if (!brep_read_from_bytes(base_brep, base_brep_len, out_base)) {
        err = "BRepTools::Read failed (invalid or unsupported base BRep blob)";
        return false;
    }
    return true;
}

// Merge coplanar faces / colinear edges a boolean left behind so later
// features (fillets, further booleans) see clean topology.
static TopoDS_Shape unify_domain(const TopoDS_Shape& shape) {
    ShapeUpgrade_UnifySameDomain unify(shape, Standard_True, Standard_True, Standard_False);
    unify.Build();
    return unify.Shape().IsNull() ? shape : unify.Shape();
}

static bool combine_with_base(
    int32_t op,
    const TopoDS_Shape& base,
    const TopoDS_Shape& tool,
    TopoDS_Shape& out,
    std::string& err) {
    if (op == 0) {
        out = tool;
        return true;
    }
    TopoDS_Shape combined;
    if (op == 1) {
        BRepAlgoAPI_Fuse boolean_op(base, tool);
        if (!boolean_op.IsDone()) {
            err = "fuse boolean operation failed";
            return false;
        }
        combined = boolean_op.Shape();
    } else {
        BRepAlgoAPI_Cut boolean_op(base, tool);
        if (!boolean_op.IsDone()) {
            err = "cut boolean operation failed";
            return false;
        }
        combined = boolean_op.Shape();
    }
    if (combined.IsNull()) {
        err = "boolean operation produced a null shape";
        return false;
    }
    out = unify_domain(combined);
    return true;
}

// Parse plane + wires into OCCT form. `require_closed=false` admits open
// spine paths.
static bool parse_profile(
    const PcadProfilePlane* plane,
    const PcadProfileWire* wires,
    size_t wire_count,
    gp_Pln& out_pln,
    gp_Dir& out_normal,
    std::vector<TopoDS_Wire>& out_wires,
    std::string& err,
    bool require_closed = true) {
    if (plane == nullptr) {
        err = "profile plane is null";
        return false;
    }
    if (wires == nullptr || wire_count == 0) {
        err = "solid op has no profile wires";
        return false;
    }
    const gp_Dir normal_dir(plane->normal[0], plane->normal[1], plane->normal[2]);
    const gp_Dir x_dir(plane->x_axis[0], plane->x_axis[1], plane->x_axis[2]);
    const gp_Pnt origin(plane->origin[0], plane->origin[1], plane->origin[2]);
    out_pln = gp_Pln(gp_Ax3(origin, normal_dir, x_dir));
    out_normal = normal_dir;

    out_wires.clear();
    out_wires.reserve(wire_count);
    for (size_t i = 0; i < wire_count; ++i) {
        TopoDS_Wire wire;
        if (!build_profile_wire(*plane, normal_dir, x_dir, wires[i], wire, err, require_closed)) {
            return false;
        }
        out_wires.push_back(wire);
    }
    return true;
}

static gp_Pnt face_centroid(const TopoDS_Shape& face) {
    GProp_GProps props;
    BRepGProp::SurfaceProperties(face, props);
    return props.CentreOfMass();
}

static double shape_bbox_diagonal(const TopoDS_Shape& shape) {
    Bnd_Box box;
    BRepBndLib::Add(shape, box);
    if (box.IsVoid()) {
        return 1.0;
    }
    Standard_Real xmin, ymin, zmin, xmax, ymax, zmax;
    box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
    return gp_Pnt(xmin, ymin, zmin).Distance(gp_Pnt(xmax, ymax, zmax));
}

// Length that guarantees a prism from `from` along `dir` passes fully through
// `solid`.
static double through_all_length(const TopoDS_Shape& solid, const gp_Pnt& from, const gp_Dir& dir) {
    Bnd_Box box;
    BRepBndLib::Add(solid, box);
    double furthest = 0.0;
    if (!box.IsVoid()) {
        Standard_Real xmin, ymin, zmin, xmax, ymax, zmax;
        box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
        const double xs[2] = {xmin, xmax};
        const double ys[2] = {ymin, ymax};
        const double zs[2] = {zmin, zmax};
        for (int i = 0; i < 2; ++i) {
            for (int j = 0; j < 2; ++j) {
                for (int k = 0; k < 2; ++k) {
                    const gp_Vec to_corner(from, gp_Pnt(xs[i], ys[j], zs[k]));
                    furthest = std::max(furthest, to_corner.Dot(gp_Vec(dir)));
                }
            }
        }
    }
    return std::max(furthest, 0.0) + shape_bbox_diagonal(solid) * 0.1 + 1.0;
}

// Keep only the material on `keep_point`'s side of `cut_pln`.
static bool trim_with_halfspace(
    const TopoDS_Shape& shape,
    const gp_Pln& cut_pln,
    const gp_Pnt& keep_point,
    TopoDS_Shape& out,
    std::string& err) {
    const TopoDS_Face plane_face = BRepBuilderAPI_MakeFace(cut_pln).Face();
    BRepPrimAPI_MakeHalfSpace half_maker(plane_face, keep_point);
    BRepAlgoAPI_Common common(shape, half_maker.Solid());
    if (!common.IsDone() || common.Shape().IsNull()) {
        err = "trimming the sweep at the target plane failed";
        return false;
    }
    out = common.Shape();
    return true;
}

// Straight or tapered prism of the whole profile from the sketch plane along
// `dir` by `distance`. A tapered prism lofts each wire to its planar offset
// at the far end: outer boundaries grow, holes shrink.
static bool prism_solid(
    const gp_Pln& pln,
    const std::vector<WireGroup>& groups,
    const TopoDS_Shape& profile_shape,
    const gp_Dir& dir,
    double distance,
    double taper_deg,
    TopoDS_Shape& out,
    std::string& err) {
    if (!(distance > Precision::Confusion())) {
        err = "extrusion distance must be positive";
        return false;
    }
    const gp_Vec sweep = gp_Vec(dir) * distance;
    if (std::abs(taper_deg) <= 1e-9) {
        BRepPrimAPI_MakePrism prism(profile_shape, sweep);
        if (!prism.IsDone() || prism.Shape().IsNull()) {
            err = "prism extrusion failed";
            return false;
        }
        out = prism.Shape();
        return true;
    }
    if (std::abs(taper_deg) >= 89.9) {
        err = "taper angle must be below 90 degrees";
        return false;
    }

    gp_Trsf move;
    move.SetTranslation(sweep);
    auto tapered_wire_loft = [&](const TopoDS_Wire& wire, double grow,
                                 TopoDS_Shape& solid) -> bool {
        BRepOffsetAPI_MakeOffset offsetter(
            BRepBuilderAPI_MakeFace(pln, wire, Standard_True).Face(),
            GeomAbs_Intersection);
        offsetter.Perform(grow);
        if (!offsetter.IsDone() || offsetter.Shape().IsNull()) {
            err = "taper offset failed (the profile may collapse at this angle)";
            return false;
        }
        TopExp_Explorer wire_it(offsetter.Shape(), TopAbs_WIRE);
        if (!wire_it.More()) {
            err = "taper offset produced no wire (the profile collapses at this angle)";
            return false;
        }
        const TopoDS_Wire far_wire = TopoDS::Wire(
            BRepBuilderAPI_Transform(wire_it.Current(), move, Standard_True).Shape());

        BRepOffsetAPI_ThruSections loft(Standard_True, Standard_True);
        loft.CheckCompatibility(Standard_False);
        loft.AddWire(wire);
        loft.AddWire(far_wire);
        loft.Build();
        if (!loft.IsDone() || loft.Shape().IsNull()) {
            err = "tapered prism loft failed";
            return false;
        }
        solid = loft.Shape();
        return true;
    };

    const double grow = std::tan(deg_to_rad(taper_deg)) * distance;
    TopoDS_Shape result;
    for (const WireGroup& group : groups) {
        TopoDS_Shape group_solid;
        if (!tapered_wire_loft(group.outer, grow, group_solid)) {
            return false;
        }
        for (const TopoDS_Wire& hole : group.holes) {
            TopoDS_Shape hole_solid;
            if (!tapered_wire_loft(hole, -grow, hole_solid)) {
                return false;
            }
            BRepAlgoAPI_Cut cut(group_solid, hole_solid);
            if (!cut.IsDone() || cut.Shape().IsNull()) {
                err = "subtracting a tapered hole from the prism failed";
                return false;
            }
            group_solid = cut.Shape();
        }
        if (result.IsNull()) {
            result = group_solid;
        } else {
            BRepAlgoAPI_Fuse fuse(result, group_solid);
            if (!fuse.IsDone() || fuse.Shape().IsNull()) {
                err = "fusing tapered profile regions failed";
                return false;
            }
            result = fuse.Shape();
        }
    }
    out = result;
    return true;
}

// Build the solid for one extrusion side. `base` may be null except for
// through-all / to-first / to-last terminations.
static bool extrude_one_side(
    const gp_Pln& pln,
    const std::vector<WireGroup>& groups,
    const TopoDS_Shape& profile_shape,
    const gp_Dir& dir,
    const PcadTermination& term,
    double taper_deg,
    const TopoDS_Shape* base,
    TopoDS_Shape& out,
    std::string& err) {
    const gp_Pnt centroid = face_centroid(profile_shape);
    switch (term.kind) {
        case 0: // blind
            return prism_solid(
                pln, groups, profile_shape, dir, term.distance, taper_deg, out, err);
        case 1: { // through all
            if (base == nullptr || base->IsNull()) {
                err = "a through-all extrusion needs existing material to pass through";
                return false;
            }
            const double d = through_all_length(*base, centroid, dir);
            return prism_solid(pln, groups, profile_shape, dir, d, taper_deg, out, err);
        }
        case 2: { // up to plane
            const gp_Dir plane_normal(
                term.plane_normal[0], term.plane_normal[1], term.plane_normal[2]);
            const gp_Pnt plane_point =
                gp_Pnt(term.plane_point[0], term.plane_point[1], term.plane_point[2])
                    .Translated(gp_Vec(plane_normal) * term.offset);
            const double denom = gp_Vec(dir).Dot(gp_Vec(plane_normal));
            if (std::abs(denom) <= 1e-9) {
                err = "the target plane is parallel to the extrusion direction";
                return false;
            }
            const double t = gp_Vec(centroid, plane_point).Dot(gp_Vec(plane_normal)) / denom;
            if (t <= Precision::Confusion()) {
                err = "the target plane lies behind the sketch along the extrusion direction";
                return false;
            }
            const double reach = t + shape_bbox_diagonal(profile_shape) + 1.0;
            TopoDS_Shape long_prism;
            if (!prism_solid(
                    pln, groups, profile_shape, dir, reach, taper_deg, long_prism, err)) {
                return false;
            }
            return trim_with_halfspace(
                long_prism, gp_Pln(plane_point, plane_normal), centroid, out, err);
        }
        case 3:
        case 4: { // to first / to last base face hit along the direction
            if (base == nullptr || base->IsNull()) {
                err = "a to-first/to-last extrusion needs existing material to stop at";
                return false;
            }
            IntCurvesFace_ShapeIntersector intersector;
            intersector.Load(*base, Precision::Confusion());
            intersector.Perform(gp_Lin(centroid, dir), Precision::Confusion(), RealLast());
            int best = -1;
            double best_w = 0.0;
            for (Standard_Integer i = 1; i <= intersector.NbPnt(); ++i) {
                const double w = intersector.WParameter(i);
                const bool better = best < 0
                    || (term.kind == 3 ? w < best_w : w > best_w);
                if (w > Precision::Confusion() && better) {
                    best = i;
                    best_w = w;
                }
            }
            if (best < 0) {
                err = "the extrusion direction does not hit the existing material";
                return false;
            }
            BRepAdaptor_Surface hit_surface(intersector.Face(best));
            if (hit_surface.GetType() == GeomAbs_Plane) {
                PcadTermination up_to{};
                up_to.kind = 2;
                const gp_Pln hit_pln = hit_surface.Plane();
                const gp_Pnt loc = hit_pln.Location();
                const gp_Dir n = hit_pln.Axis().Direction();
                up_to.plane_point[0] = loc.X();
                up_to.plane_point[1] = loc.Y();
                up_to.plane_point[2] = loc.Z();
                up_to.plane_normal[0] = n.X();
                up_to.plane_normal[1] = n.Y();
                up_to.plane_normal[2] = n.Z();
                return extrude_one_side(
                    pln, groups, profile_shape, dir, up_to, taper_deg, base, out, err);
            }
            // Curved terminating face: stop at the centroid hit distance.
            return prism_solid(pln, groups, profile_shape, dir, best_w, taper_deg, out, err);
        }
        default:
            err = "unknown extrusion termination kind " + std::to_string(term.kind);
            return false;
    }
}

static bool fuse_shapes(
    const TopoDS_Shape& a,
    const TopoDS_Shape& b,
    TopoDS_Shape& out,
    std::string& err) {
    BRepAlgoAPI_Fuse fuse(a, b);
    if (!fuse.IsDone() || fuse.Shape().IsNull()) {
        err = "fusing the two sweep sides failed";
        return false;
    }
    out = fuse.Shape();
    return true;
}

// World-space axis from a sketch-plane (uv point, uv direction) pair.
static bool sketch_plane_axis(
    const PcadProfilePlane& plane,
    const double origin_uv[2],
    const double dir_uv[2],
    gp_Ax1& out,
    std::string& err) {
    const gp_Vec axis_vec(
        dir_uv[0] * plane.x_axis[0] + dir_uv[1] * plane.y_axis[0],
        dir_uv[0] * plane.x_axis[1] + dir_uv[1] * plane.y_axis[1],
        dir_uv[0] * plane.x_axis[2] + dir_uv[1] * plane.y_axis[2]);
    if (axis_vec.Magnitude() <= Precision::Confusion()) {
        err = "axis direction is (near) zero in the sketch plane";
        return false;
    }
    out = gp_Ax1(profile_point(plane, origin_uv[0], origin_uv[1]), gp_Dir(axis_vec));
    return true;
}

static bool revolve_face(
    const TopoDS_Shape& face,
    const gp_Ax1& axis,
    double angle_deg,
    TopoDS_Shape& out,
    std::string& err) {
    if (!(angle_deg > 0.0) || angle_deg > 360.0) {
        err = "revolve angle must be in (0, 360] degrees, got " + std::to_string(angle_deg);
        return false;
    }
    // Treat anything within a millidegree of a full turn as 360°.
    if (angle_deg >= 359.999) {
        BRepPrimAPI_MakeRevol revol(face, axis);
        if (!revol.IsDone() || revol.Shape().IsNull()) {
            err = "revolve operation failed";
            return false;
        }
        out = revol.Shape();
        return true;
    }
    BRepPrimAPI_MakeRevol revol(face, axis, deg_to_rad(angle_deg));
    if (!revol.IsDone() || revol.Shape().IsNull()) {
        err = "revolve operation failed";
        return false;
    }
    out = revol.Shape();
    return true;
}

// Sweep every profile wire group along the spine (outer sweeps minus hole
// sweeps, disjoint regions fused). `binormal` fixes the profile orientation
// (helix); otherwise `frenet` selects the Frenet vs corrected frame.
static bool sweep_wires_along_spine(
    const TopoDS_Wire& spine,
    const std::vector<WireGroup>& groups,
    const gp_Dir* binormal,
    bool frenet,
    TopoDS_Shape& out,
    std::string& err) {
    auto pipe_one = [&](const TopoDS_Wire& wire, TopoDS_Shape& solid) -> bool {
        BRepOffsetAPI_MakePipeShell shell(spine);
        if (binormal != nullptr) {
            shell.SetMode(*binormal);
        } else {
            shell.SetMode(frenet ? Standard_True : Standard_False);
        }
        shell.SetTransitionMode(BRepBuilderAPI_RightCorner);
        shell.Add(wire);
        shell.Build();
        if (!shell.IsDone() || shell.Shape().IsNull()) {
            err = "pipe sweep failed (the profile may self-intersect along the path)";
            return false;
        }
        if (!shell.MakeSolid()) {
            err = "pipe sweep could not be closed into a solid";
            return false;
        }
        solid = shell.Shape();
        return true;
    };

    TopoDS_Shape result;
    for (const WireGroup& group : groups) {
        TopoDS_Shape group_solid;
        if (!pipe_one(group.outer, group_solid)) {
            return false;
        }
        for (const TopoDS_Wire& hole : group.holes) {
            TopoDS_Shape hole_solid;
            if (!pipe_one(hole, hole_solid)) {
                return false;
            }
            BRepAlgoAPI_Cut cut(group_solid, hole_solid);
            if (!cut.IsDone() || cut.Shape().IsNull()) {
                err = "subtracting a hole sweep from the pipe failed";
                return false;
            }
            group_solid = cut.Shape();
        }
        if (result.IsNull()) {
            result = group_solid;
        } else {
            BRepAlgoAPI_Fuse fuse(result, group_solid);
            if (!fuse.IsDone() || fuse.Shape().IsNull()) {
                err = "fusing pipe profile regions failed";
                return false;
            }
            result = fuse.Shape();
        }
    }
    out = result;
    return true;
}

// Helix spine on a cylinder/cone around `axis`, starting at the closest point
// on the axis to `start` (profile centroid), running `height` with `pitch`
// per turn.
static bool build_helix_spine(
    const gp_Ax1& axis,
    const gp_Pnt& start,
    double pitch,
    double height,
    double cone_angle_deg,
    bool left_handed,
    bool reversed,
    TopoDS_Wire& out,
    std::string& err) {
    if (!(pitch > Precision::Confusion()) || !(height > Precision::Confusion())) {
        err = "helix pitch and height must be positive";
        return false;
    }
    const gp_Vec to_start(axis.Location(), start);
    const gp_Vec along = gp_Vec(axis.Direction()) * to_start.Dot(gp_Vec(axis.Direction()));
    const gp_Vec radial = to_start - along;
    const double radius = radial.Magnitude();
    if (radius <= Precision::Confusion()) {
        err = "the profile sits on the helix axis (zero radius)";
        return false;
    }

    gp_Dir axis_dir = axis.Direction();
    if (reversed) {
        axis_dir.Reverse();
    }
    const gp_Pnt base_point = start.Translated(-radial);
    const gp_Ax3 frame(base_point, axis_dir, gp_Dir(radial));

    const double turns = height / pitch;
    const double du = 2.0 * kPi * turns * (left_handed ? -1.0 : 1.0);
    Handle(Geom_Surface) surface;
    double dv;
    if (std::abs(cone_angle_deg) <= 1e-9) {
        surface = new Geom_CylindricalSurface(frame, radius);
        dv = height;
    } else {
        if (std::abs(cone_angle_deg) >= 89.0) {
            err = "helix cone angle must be below 89 degrees";
            return false;
        }
        const double semi = deg_to_rad(cone_angle_deg);
        surface = new Geom_ConicalSurface(frame, semi, radius);
        // The cone's V parameter runs along the slant, not the axis.
        dv = height / std::cos(semi);
    }

    const gp_Dir2d dir2d(du, dv);
    Handle(Geom2d_Line) param_line = new Geom2d_Line(gp_Pnt2d(0.0, 0.0), dir2d);
    const double param_len = std::hypot(du, dv);
    TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(param_line, surface, 0.0, param_len).Edge();
    BRepLib::BuildCurves3d(edge);
    out = BRepBuilderAPI_MakeWire(edge).Wire();
    if (out.IsNull()) {
        err = "building the helix spine failed";
        return false;
    }
    return true;
}

extern "C" PrintcadOcctSweepResult printcad_occt_solid_sweep(
    const uint8_t* base_brep,
    size_t base_brep_len,
    const PcadProfilePlane* plane,
    const PcadProfileWire* wires,
    size_t wire_count,
    const PcadSweepDesc* desc,
    int32_t op,
    const PcadMeshOptions* mesh) {
    if (desc == nullptr) {
        return make_sweep_error("sweep description is null");
    }
    try {
        std::string err;
        TopoDS_Shape base;
        if (!load_base_for_op(op, base_brep, base_brep_len, base, err)) {
            return make_sweep_error(err);
        }

        gp_Pln pln;
        gp_Dir normal_dir;
        std::vector<TopoDS_Wire> topo_wires;
        if (!parse_profile(plane, wires, wire_count, pln, normal_dir, topo_wires, err)) {
            return make_sweep_error(err);
        }
        TopoDS_Shape face;
        std::vector<WireGroup> groups;
        if (!build_profile_face(pln, topo_wires, face, groups, err)) {
            return make_sweep_error(err);
        }
        const TopoDS_Shape* base_ptr = base.IsNull() ? nullptr : &base;

        TopoDS_Shape tool;
        switch (desc->kind) {
            case 0: { // extrude
                gp_Dir dir = normal_dir;
                if (desc->has_direction != 0) {
                    const gp_Vec custom(
                        desc->direction[0], desc->direction[1], desc->direction[2]);
                    if (custom.Magnitude() <= Precision::Confusion()) {
                        return make_sweep_error("custom extrusion direction is (near) zero");
                    }
                    if (std::abs(custom.Normalized().Dot(gp_Vec(normal_dir)))
                        <= Precision::Confusion()) {
                        return make_sweep_error(
                            "custom extrusion direction is parallel to the sketch plane");
                    }
                    dir = gp_Dir(custom);
                }
                if (desc->reversed != 0) {
                    dir.Reverse();
                }

                if (!extrude_one_side(
                        pln, groups, face, dir, desc->term, desc->taper_deg,
                        base_ptr, tool, err)) {
                    return make_sweep_error(err);
                }
                if (desc->has_term2 != 0) {
                    TopoDS_Shape back;
                    if (!extrude_one_side(
                            pln, groups, face, dir.Reversed(), desc->term2,
                            desc->taper_deg, base_ptr, back, err)) {
                        return make_sweep_error(err);
                    }
                    if (!fuse_shapes(tool, back, tool, err)) {
                        return make_sweep_error(err);
                    }
                } else if (desc->symmetric != 0 && desc->term.kind == 0) {
                    gp_Trsf shift;
                    shift.SetTranslation(gp_Vec(dir) * (-desc->term.distance * 0.5));
                    tool = BRepBuilderAPI_Transform(tool, shift, Standard_True).Shape();
                }
                break;
            }
            case 1: { // revolve
                gp_Ax1 axis;
                if (!sketch_plane_axis(*plane, desc->axis_origin, desc->axis_dir, axis, err)) {
                    return make_sweep_error(err);
                }
                if (desc->reversed != 0) {
                    axis.Reverse();
                }
                double forward = desc->angle_deg;
                double backward = desc->has_angle2 != 0 ? desc->angle2_deg : 0.0;
                if (desc->midplane != 0) {
                    forward = desc->angle_deg * 0.5;
                    backward = desc->angle_deg * 0.5;
                }
                TopoDS_Shape sweep_face = face;
                if (backward > 0.0) {
                    // Pre-rotate so the sweep covers [-backward, +forward].
                    gp_Trsf pre;
                    pre.SetRotation(axis, -deg_to_rad(backward));
                    sweep_face = BRepBuilderAPI_Transform(face, pre, Standard_True).Shape();
                }
                if (!revolve_face(sweep_face, axis, forward + backward, tool, err)) {
                    return make_sweep_error(err);
                }
                break;
            }
            case 2: { // helix
                gp_Ax1 axis;
                if (!sketch_plane_axis(*plane, desc->axis_origin, desc->axis_dir, axis, err)) {
                    return make_sweep_error(err);
                }
                TopoDS_Wire spine;
                if (!build_helix_spine(
                        axis, face_centroid(face), desc->pitch, desc->height,
                        desc->cone_angle_deg, desc->left_handed != 0,
                        desc->reversed != 0, spine, err)) {
                    return make_sweep_error(err);
                }
                const gp_Dir binormal = axis.Direction();
                if (!sweep_wires_along_spine(spine, groups, &binormal, false, tool, err)) {
                    return make_sweep_error(err);
                }
                break;
            }
            default:
                return make_sweep_error(
                    "unknown sweep kind " + std::to_string(desc->kind)
                    + " (expected 0=extrude, 1=revolve, 2=helix)");
        }
        if (tool.IsNull()) {
            return make_sweep_error("sweep produced no shape");
        }

        TopoDS_Shape result_shape;
        if (!combine_with_base(op, base, tool, result_shape, err)) {
            return make_sweep_error(err);
        }
        return finish_solid_result(result_shape, &tool, mesh);
    } catch (Standard_Failure const& ex) {
        return make_sweep_error(std::string("OCCT exception: ") + ex.GetMessageString());
    } catch (std::exception const& ex) {
        return make_sweep_error(std::string("std::exception: ") + ex.what());
    } catch (...) {
        return make_sweep_error("Unknown exception while sweeping profile");
    }
}

extern "C" PrintcadOcctSweepResult printcad_occt_solid_loft(
    const uint8_t* base_brep,
    size_t base_brep_len,
    const PcadProfilePlane* planes,
    const PcadProfileWire* wires,
    const size_t* wire_offsets,
    const size_t* wire_counts,
    size_t section_count,
    int ruled,
    int closed,
    int32_t op,
    const PcadMeshOptions* mesh) {
    if (planes == nullptr || wires == nullptr || wire_offsets == nullptr
        || wire_counts == nullptr) {
        return make_sweep_error("loft input arrays are null");
    }
    if (section_count < 2) {
        return make_sweep_error("a loft needs at least two sections");
    }
    try {
        std::string err;
        TopoDS_Shape base;
        if (!load_base_for_op(op, base_brep, base_brep_len, base, err)) {
            return make_sweep_error(err);
        }

        // Per section: wires sorted by descending area so index 0 is the
        // outer boundary and hole ranks correspond across sections.
        std::vector<std::vector<TopoDS_Wire>> sections(section_count);
        bool holes_matched = true;
        for (size_t s = 0; s < section_count; ++s) {
            gp_Pln pln;
            gp_Dir normal;
            if (!parse_profile(
                    &planes[s], wires + wire_offsets[s], wire_counts[s], pln, normal,
                    sections[s], err)) {
                return make_sweep_error("loft section " + std::to_string(s + 1) + ": " + err);
            }
            std::sort(
                sections[s].begin(), sections[s].end(),
                [&pln](const TopoDS_Wire& a, const TopoDS_Wire& b) {
                    return planar_wire_area(pln, a) > planar_wire_area(pln, b);
                });
            if (sections[s].size() != sections[0].size()) {
                holes_matched = false;
            }
        }

        const size_t rank_count = holes_matched ? sections[0].size() : 1;
        TopoDS_Shape tool;
        std::vector<TopoDS_Shape> hole_solids;
        for (size_t rank = 0; rank < rank_count; ++rank) {
            BRepOffsetAPI_ThruSections loft(Standard_True, ruled != 0);
            loft.CheckCompatibility(Standard_False);
            for (size_t s = 0; s < section_count; ++s) {
                loft.AddWire(sections[s][rank]);
            }
            if (closed != 0) {
                loft.AddWire(sections[0][rank]);
            }
            loft.Build();
            if (!loft.IsDone() || loft.Shape().IsNull()) {
                return make_sweep_error(
                    "loft failed (check section compatibility and ordering)");
            }
            if (rank == 0) {
                tool = loft.Shape();
            } else {
                hole_solids.push_back(loft.Shape());
            }
        }
        for (const TopoDS_Shape& hole : hole_solids) {
            BRepAlgoAPI_Cut cut(tool, hole);
            if (!cut.IsDone() || cut.Shape().IsNull()) {
                return make_sweep_error("subtracting a hole loft failed");
            }
            tool = cut.Shape();
        }

        TopoDS_Shape result_shape;
        if (!combine_with_base(op, base, tool, result_shape, err)) {
            return make_sweep_error(err);
        }
        return finish_solid_result(result_shape, &tool, mesh);
    } catch (Standard_Failure const& ex) {
        return make_sweep_error(std::string("OCCT exception: ") + ex.GetMessageString());
    } catch (std::exception const& ex) {
        return make_sweep_error(std::string("std::exception: ") + ex.what());
    } catch (...) {
        return make_sweep_error("Unknown exception while lofting");
    }
}

extern "C" PrintcadOcctSweepResult printcad_occt_solid_pipe(
    const uint8_t* base_brep,
    size_t base_brep_len,
    const PcadProfilePlane* profile_plane,
    const PcadProfileWire* profile_wires,
    size_t profile_wire_count,
    const PcadProfilePlane* spine_plane,
    const PcadProfileWire* spine_wire,
    int frenet,
    int32_t op,
    const PcadMeshOptions* mesh) {
    try {
        std::string err;
        TopoDS_Shape base;
        if (!load_base_for_op(op, base_brep, base_brep_len, base, err)) {
            return make_sweep_error(err);
        }

        gp_Pln profile_pln;
        gp_Dir profile_normal;
        std::vector<TopoDS_Wire> topo_wires;
        if (!parse_profile(
                profile_plane, profile_wires, profile_wire_count, profile_pln,
                profile_normal, topo_wires, err)) {
            return make_sweep_error(err);
        }
        std::vector<WireGroup> groups;
        if (!group_profile_wires(profile_pln, topo_wires, groups, err)) {
            return make_sweep_error(err);
        }

        gp_Pln spine_pln;
        gp_Dir spine_normal;
        std::vector<TopoDS_Wire> spine_wires;
        if (!parse_profile(
                spine_plane, spine_wire, 1, spine_pln, spine_normal, spine_wires, err,
                /*require_closed=*/false)) {
            return make_sweep_error("pipe spine: " + err);
        }

        TopoDS_Shape tool;
        if (!sweep_wires_along_spine(
                spine_wires[0], groups, nullptr, frenet != 0, tool, err)) {
            return make_sweep_error(err);
        }

        TopoDS_Shape result_shape;
        if (!combine_with_base(op, base, tool, result_shape, err)) {
            return make_sweep_error(err);
        }
        return finish_solid_result(result_shape, &tool, mesh);
    } catch (Standard_Failure const& ex) {
        return make_sweep_error(std::string("OCCT exception: ") + ex.GetMessageString());
    } catch (std::exception const& ex) {
        return make_sweep_error(std::string("std::exception: ") + ex.what());
    } catch (...) {
        return make_sweep_error("Unknown exception while sweeping along a path");
    }
}

extern "C" PrintcadOcctSweepResult printcad_occt_solid_primitive(
    const uint8_t* base_brep,
    size_t base_brep_len,
    int32_t kind,
    const double* params,
    size_t param_count,
    const double placement[9],
    int32_t op,
    const PcadMeshOptions* mesh) {
    if (params == nullptr || placement == nullptr) {
        return make_sweep_error("primitive parameters are null");
    }
    try {
        std::string err;
        TopoDS_Shape base;
        if (!load_base_for_op(op, base_brep, base_brep_len, base, err)) {
            return make_sweep_error(err);
        }

        const gp_Pnt origin(placement[0], placement[1], placement[2]);
        const gp_Vec x_raw(placement[3], placement[4], placement[5]);
        const gp_Vec z_raw(placement[6], placement[7], placement[8]);
        if (x_raw.Magnitude() <= Precision::Confusion()
            || z_raw.Magnitude() <= Precision::Confusion()) {
            return make_sweep_error("primitive placement axes are (near) zero");
        }
        const gp_Dir z_dir(z_raw);
        // Re-orthogonalize X against Z so slightly-off inputs still work.
        gp_Vec x_ortho = x_raw - gp_Vec(z_dir) * x_raw.Dot(gp_Vec(z_dir));
        if (x_ortho.Magnitude() <= Precision::Confusion()) {
            return make_sweep_error("primitive placement axes are (near) parallel");
        }
        const gp_Ax2 frame(origin, z_dir, gp_Dir(x_ortho));

        auto need = [&](size_t n) -> bool { return param_count >= n; };
        auto positive = [](double v) -> bool { return v > Precision::Confusion(); };

        TopoDS_Shape tool;
        switch (kind) {
            case 0: { // box
                if (!need(3) || !positive(params[0]) || !positive(params[1])
                    || !positive(params[2])) {
                    return make_sweep_error("box needs positive length/width/height");
                }
                tool = BRepPrimAPI_MakeBox(frame, params[0], params[1], params[2]).Shape();
                break;
            }
            case 1: { // cylinder
                if (!need(3) || !positive(params[0]) || !positive(params[1])) {
                    return make_sweep_error("cylinder needs positive radius/height");
                }
                const double angle = params[2];
                if (angle >= 359.999 || angle <= 0.0) {
                    tool = BRepPrimAPI_MakeCylinder(frame, params[0], params[1]).Shape();
                } else {
                    tool = BRepPrimAPI_MakeCylinder(
                               frame, params[0], params[1], deg_to_rad(angle))
                               .Shape();
                }
                break;
            }
            case 2: { // sphere
                if (!need(4) || !positive(params[0])) {
                    return make_sweep_error("sphere needs a positive radius");
                }
                const double a1 = deg_to_rad(params[1]);
                const double a2 = deg_to_rad(params[2]);
                const double a3 = params[3];
                const bool full_lat = params[1] <= -89.999 && params[2] >= 89.999;
                if (full_lat && a3 >= 359.999) {
                    tool = BRepPrimAPI_MakeSphere(frame, params[0]).Shape();
                } else if (full_lat) {
                    tool = BRepPrimAPI_MakeSphere(frame, params[0], deg_to_rad(a3)).Shape();
                } else {
                    tool = BRepPrimAPI_MakeSphere(frame, params[0], a1, a2, deg_to_rad(a3))
                               .Shape();
                }
                break;
            }
            case 3: { // cone
                if (!need(4) || params[0] < 0.0 || params[1] < 0.0 || !positive(params[2])) {
                    return make_sweep_error(
                        "cone needs non-negative radii and a positive height");
                }
                if (std::abs(params[0] - params[1]) <= Precision::Confusion()) {
                    return make_sweep_error(
                        "cone radii must differ (use a cylinder for equal radii)");
                }
                const double angle = params[3];
                if (angle >= 359.999 || angle <= 0.0) {
                    tool = BRepPrimAPI_MakeCone(frame, params[0], params[1], params[2]).Shape();
                } else {
                    tool = BRepPrimAPI_MakeCone(
                               frame, params[0], params[1], params[2], deg_to_rad(angle))
                               .Shape();
                }
                break;
            }
            case 4: { // torus
                if (!need(5) || !positive(params[0]) || !positive(params[1])) {
                    return make_sweep_error("torus needs positive radii");
                }
                const double a1 = params[2];
                const double a2 = params[3];
                const double a3 = params[4];
                const bool full_section = a1 <= -179.999 && a2 >= 179.999;
                if (full_section && a3 >= 359.999) {
                    tool = BRepPrimAPI_MakeTorus(frame, params[0], params[1]).Shape();
                } else if (full_section) {
                    tool = BRepPrimAPI_MakeTorus(frame, params[0], params[1], deg_to_rad(a3))
                               .Shape();
                } else {
                    tool = BRepPrimAPI_MakeTorus(
                               frame, params[0], params[1], deg_to_rad(a1), deg_to_rad(a2),
                               deg_to_rad(a3))
                               .Shape();
                }
                break;
            }
            case 5: { // ellipsoid: unit sphere scaled per-axis, then placed
                if (!need(3) || !positive(params[0]) || !positive(params[1])
                    || !positive(params[2])) {
                    return make_sweep_error("ellipsoid needs three positive radii");
                }
                const TopoDS_Shape sphere = BRepPrimAPI_MakeSphere(1.0).Shape();
                gp_GTrsf scale;
                scale.SetVectorialPart(gp_Mat(
                    params[0], 0.0, 0.0,
                    0.0, params[1], 0.0,
                    0.0, 0.0, params[2]));
                BRepBuilderAPI_GTransform scaler(sphere, scale, Standard_True);
                if (!scaler.IsDone() || scaler.Shape().IsNull()) {
                    return make_sweep_error("ellipsoid scaling failed");
                }
                gp_Trsf place;
                place.SetTransformation(gp_Ax3(frame), gp_Ax3());
                tool = BRepBuilderAPI_Transform(scaler.Shape(), place, Standard_True).Shape();
                break;
            }
            case 6: { // regular prism
                if (!need(3) || params[0] < 3.0 || !positive(params[1])
                    || !positive(params[2])) {
                    return make_sweep_error(
                        "prism needs at least 3 sides and positive radius/height");
                }
                const int sides = static_cast<int>(params[0]);
                BRepBuilderAPI_MakePolygon polygon;
                for (int i = 0; i < sides; ++i) {
                    const double a = 2.0 * kPi * i / sides;
                    const gp_Vec offset =
                        gp_Vec(frame.XDirection()) * (std::cos(a) * params[1])
                        + gp_Vec(frame.YDirection()) * (std::sin(a) * params[1]);
                    polygon.Add(origin.Translated(offset));
                }
                polygon.Close();
                const TopoDS_Face face =
                    BRepBuilderAPI_MakeFace(gp_Pln(origin, z_dir), polygon.Wire()).Face();
                BRepPrimAPI_MakePrism prism(face, gp_Vec(z_dir) * params[2]);
                if (!prism.IsDone() || prism.Shape().IsNull()) {
                    return make_sweep_error("prism extrusion failed");
                }
                tool = prism.Shape();
                break;
            }
            case 7: { // wedge
                if (!need(10)) {
                    return make_sweep_error("wedge needs 10 span parameters");
                }
                const double dx = params[1] - params[0];
                const double dy = params[3] - params[2];
                const double dz = params[5] - params[4];
                if (!positive(dx) || !positive(dy) || !positive(dz)) {
                    return make_sweep_error("wedge spans must have positive extent");
                }
                const double top_xmin = params[6] - params[0];
                const double top_xmax = params[7] - params[0];
                const double top_zmin = params[8] - params[4];
                const double top_zmax = params[9] - params[4];
                if (top_xmax < top_xmin || top_zmax < top_zmin) {
                    return make_sweep_error("wedge top spans are inverted");
                }
                BRepPrimAPI_MakeWedge wedge(
                    frame, dx, dy, dz, top_xmin, top_zmin, top_xmax, top_zmax);
                tool = wedge.Shape();
                // Shift so the placement origin corresponds to (0,0,0) of the
                // requested min corner, matching the span-based input.
                gp_Trsf shift;
                shift.SetTranslation(
                    gp_Vec(frame.XDirection()) * params[0]
                    + gp_Vec(frame.YDirection()) * params[2]
                    + gp_Vec(z_dir) * params[4]);
                tool = BRepBuilderAPI_Transform(tool, shift, Standard_True).Shape();
                break;
            }
            default:
                return make_sweep_error("unknown primitive kind " + std::to_string(kind));
        }
        if (tool.IsNull()) {
            return make_sweep_error("primitive construction produced no shape");
        }

        TopoDS_Shape result_shape;
        if (!combine_with_base(op, base, tool, result_shape, err)) {
            return make_sweep_error(err);
        }
        return finish_solid_result(result_shape, &tool, mesh);
    } catch (Standard_Failure const& ex) {
        return make_sweep_error(std::string("OCCT exception: ") + ex.GetMessageString());
    } catch (std::exception const& ex) {
        return make_sweep_error(std::string("std::exception: ") + ex.what());
    } catch (...) {
        return make_sweep_error("Unknown exception while building a primitive");
    }
}

// Nearest sub-shape of `type` in `shape` to a world point.
static TopoDS_Shape nearest_subshape(
    const TopoDS_Shape& shape,
    TopAbs_ShapeEnum type,
    const gp_Pnt& point) {
    const TopoDS_Shape probe = BRepBuilderAPI_MakeVertex(point).Shape();
    TopoDS_Shape best;
    double best_dist = 0.0;
    for (TopExp_Explorer it(shape, type); it.More(); it.Next()) {
        BRepExtrema_DistShapeShape dist(probe, it.Current());
        if (!dist.IsDone() || dist.NbSolution() == 0) {
            continue;
        }
        if (best.IsNull() || dist.Value() < best_dist) {
            best = it.Current();
            best_dist = dist.Value();
        }
    }
    return best;
}

// Resolve a dress-up edge selection into a deduplicated edge list.
static bool collect_dressup_edges(
    const TopoDS_Shape& base,
    int32_t selection_mode,
    const double* points,
    size_t point_count,
    TopTools_IndexedMapOfShape& out_edges,
    std::string& err) {
    if (selection_mode == 0) {
        TopExp::MapShapes(base, TopAbs_EDGE, out_edges);
    } else {
        if (points == nullptr || point_count == 0) {
            err = "edge selection needs at least one reference point";
            return false;
        }
        for (size_t i = 0; i < point_count; ++i) {
            const gp_Pnt p(points[3 * i], points[3 * i + 1], points[3 * i + 2]);
            if (selection_mode == 1) {
                const TopoDS_Shape face = nearest_subshape(base, TopAbs_FACE, p);
                if (face.IsNull()) {
                    err = "no face found near an edge-selection reference point";
                    return false;
                }
                for (TopExp_Explorer it(face, TopAbs_EDGE); it.More(); it.Next()) {
                    out_edges.Add(it.Current());
                }
            } else {
                const TopoDS_Shape edge = nearest_subshape(base, TopAbs_EDGE, p);
                if (edge.IsNull()) {
                    err = "no edge found near an edge-selection reference point";
                    return false;
                }
                out_edges.Add(edge);
            }
        }
    }
    if (out_edges.IsEmpty()) {
        err = "edge selection resolved to no edges";
        return false;
    }
    return true;
}

extern "C" PrintcadOcctSweepResult printcad_occt_dressup(
    const uint8_t* base_brep,
    size_t base_brep_len,
    int32_t kind,
    const double* params,
    size_t param_count,
    int32_t selection_mode,
    const double* points,
    size_t point_count,
    const PcadMeshOptions* mesh) {
    if (params == nullptr || param_count == 0) {
        return make_sweep_error("dress-up parameters are null");
    }
    try {
        std::string err;
        TopoDS_Shape base;
        if (!load_base_for_op(1, base_brep, base_brep_len, base, err)) {
            return make_sweep_error(err);
        }

        TopTools_IndexedMapOfShape edges;
        if (!collect_dressup_edges(base, selection_mode, points, point_count, edges, err)) {
            return make_sweep_error(err);
        }

        TopoDS_Shape result_shape;
        if (kind == 0) { // fillet
            const double radius = params[0];
            if (!(radius > Precision::Confusion())) {
                return make_sweep_error("fillet radius must be positive");
            }
            BRepFilletAPI_MakeFillet fillet(base);
            for (Standard_Integer i = 1; i <= edges.Extent(); ++i) {
                fillet.Add(radius, TopoDS::Edge(edges.FindKey(i)));
            }
            fillet.Build();
            if (!fillet.IsDone() || fillet.Shape().IsNull()) {
                return make_sweep_error(
                    "fillet failed (radius may be too large for these edges)");
            }
            result_shape = fillet.Shape();
        } else if (kind == 1) { // chamfer
            if (param_count < 4) {
                return make_sweep_error("chamfer needs mode, two sizes, and a flip flag");
            }
            const int mode = static_cast<int>(params[0]);
            const double d1 = params[1];
            const double d2_or_angle = params[2];
            const bool flip = params[3] != 0.0;
            if (!(d1 > Precision::Confusion())) {
                return make_sweep_error("chamfer size must be positive");
            }

            TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
            TopExp::MapShapesAndAncestors(base, TopAbs_EDGE, TopAbs_FACE, edge_faces);
            BRepFilletAPI_MakeChamfer chamfer(base);
            for (Standard_Integer i = 1; i <= edges.Extent(); ++i) {
                const TopoDS_Edge edge = TopoDS::Edge(edges.FindKey(i));
                if (mode == 0) {
                    chamfer.Add(d1, edge);
                    continue;
                }
                const Standard_Integer fi = edge_faces.FindIndex(edge);
                if (fi < 1 || edge_faces.FindFromIndex(fi).IsEmpty()) {
                    return make_sweep_error("chamfer could not find a face for an edge");
                }
                const TopTools_ListOfShape& faces = edge_faces.FindFromIndex(fi);
                TopoDS_Face ref_face = TopoDS::Face(faces.First());
                if (flip && faces.Extent() > 1) {
                    ref_face = TopoDS::Face(faces.Last());
                }
                if (mode == 1) {
                    const double d2 = flip ? d1 : d2_or_angle;
                    const double d1_eff = flip ? d2_or_angle : d1;
                    chamfer.Add(d1_eff, d2, edge, ref_face);
                } else {
                    chamfer.AddDA(d1, deg_to_rad(d2_or_angle), edge, ref_face);
                }
            }
            chamfer.Build();
            if (!chamfer.IsDone() || chamfer.Shape().IsNull()) {
                return make_sweep_error(
                    "chamfer failed (size may be too large for these edges)");
            }
            result_shape = chamfer.Shape();
        } else {
            return make_sweep_error("unknown dress-up kind " + std::to_string(kind));
        }

        return finish_solid_result(result_shape, nullptr, mesh);
    } catch (Standard_Failure const& ex) {
        return make_sweep_error(std::string("OCCT exception: ") + ex.GetMessageString());
    } catch (std::exception const& ex) {
        return make_sweep_error(std::string("std::exception: ") + ex.what());
    } catch (...) {
        return make_sweep_error("Unknown exception while applying a dress-up");
    }
}

extern "C" PrintcadOcctSweepResult printcad_occt_draft(
    const uint8_t* base_brep,
    size_t base_brep_len,
    double angle_deg,
    const double neutral_point[3],
    const double neutral_normal[3],
    const double* pull_dir,
    const double* face_points,
    size_t face_point_count,
    const PcadMeshOptions* mesh) {
    if (neutral_point == nullptr || neutral_normal == nullptr) {
        return make_sweep_error("draft neutral plane is null");
    }
    if (face_points == nullptr || face_point_count == 0) {
        return make_sweep_error("draft needs at least one face reference point");
    }
    try {
        std::string err;
        TopoDS_Shape base;
        if (!load_base_for_op(1, base_brep, base_brep_len, base, err)) {
            return make_sweep_error(err);
        }

        const gp_Dir neutral_dir(neutral_normal[0], neutral_normal[1], neutral_normal[2]);
        const gp_Pln neutral_pln(
            gp_Pnt(neutral_point[0], neutral_point[1], neutral_point[2]), neutral_dir);
        const gp_Dir pull =
            pull_dir != nullptr ? gp_Dir(pull_dir[0], pull_dir[1], pull_dir[2]) : neutral_dir;

        BRepOffsetAPI_DraftAngle draft(base);
        for (size_t i = 0; i < face_point_count; ++i) {
            const gp_Pnt p(face_points[3 * i], face_points[3 * i + 1], face_points[3 * i + 2]);
            const TopoDS_Shape face = nearest_subshape(base, TopAbs_FACE, p);
            if (face.IsNull()) {
                return make_sweep_error("no face found near a draft reference point");
            }
            draft.Add(TopoDS::Face(face), pull, deg_to_rad(angle_deg), neutral_pln);
            if (!draft.AddDone()) {
                return make_sweep_error(
                    "draft rejected a face (it may be tangent-connected or parallel "
                    "to the pull direction)");
            }
        }
        draft.Build();
        if (!draft.IsDone() || draft.Shape().IsNull()) {
            return make_sweep_error("draft failed");
        }
        return finish_solid_result(draft.Shape(), nullptr, mesh);
    } catch (Standard_Failure const& ex) {
        return make_sweep_error(std::string("OCCT exception: ") + ex.GetMessageString());
    } catch (std::exception const& ex) {
        return make_sweep_error(std::string("std::exception: ") + ex.what());
    } catch (...) {
        return make_sweep_error("Unknown exception while drafting faces");
    }
}

extern "C" PrintcadOcctSweepResult printcad_occt_thickness(
    const uint8_t* base_brep,
    size_t base_brep_len,
    double value,
    int inward,
    const double* face_points,
    size_t face_point_count,
    const PcadMeshOptions* mesh) {
    if (face_points == nullptr || face_point_count == 0) {
        return make_sweep_error("thickness needs at least one face to open");
    }
    if (!(value > Precision::Confusion())) {
        return make_sweep_error("wall thickness must be positive");
    }
    try {
        std::string err;
        TopoDS_Shape base;
        if (!load_base_for_op(1, base_brep, base_brep_len, base, err)) {
            return make_sweep_error(err);
        }

        TopTools_ListOfShape open_faces;
        TopTools_IndexedMapOfShape seen;
        for (size_t i = 0; i < face_point_count; ++i) {
            const gp_Pnt p(face_points[3 * i], face_points[3 * i + 1], face_points[3 * i + 2]);
            const TopoDS_Shape face = nearest_subshape(base, TopAbs_FACE, p);
            if (face.IsNull()) {
                return make_sweep_error("no face found near a thickness reference point");
            }
            if (seen.Add(face) > 0) {
                open_faces.Append(face);
            }
        }

        BRepOffsetAPI_MakeThickSolid thick;
        thick.MakeThickSolidByJoin(
            base, open_faces, inward != 0 ? -value : value, 1e-3,
            BRepOffset_Skin, Standard_False, Standard_False, GeomAbs_Arc);
        if (!thick.IsDone() || thick.Shape().IsNull()) {
            return make_sweep_error(
                "hollowing failed (wall may be too thick for this shape)");
        }
        return finish_solid_result(thick.Shape(), nullptr, mesh);
    } catch (Standard_Failure const& ex) {
        return make_sweep_error(std::string("OCCT exception: ") + ex.GetMessageString());
    } catch (std::exception const& ex) {
        return make_sweep_error(std::string("std::exception: ") + ex.what());
    } catch (...) {
        return make_sweep_error("Unknown exception while hollowing");
    }
}

// Row-major 4x4 -> OCCT transform; scaling/shear falls back to GTransform.
static TopoDS_Shape transform_shape_4x4(
    const TopoDS_Shape& shape,
    const double* m,
    std::string& err) {
    // Rigid check: the 3x3 part times its transpose should be identity.
    bool rigid = true;
    for (int r = 0; r < 3 && rigid; ++r) {
        for (int c = 0; c < 3 && rigid; ++c) {
            double dot = 0.0;
            for (int k = 0; k < 3; ++k) {
                dot += m[r * 4 + k] * m[c * 4 + k];
            }
            if (std::abs(dot - (r == c ? 1.0 : 0.0)) > 1e-7) {
                rigid = false;
            }
        }
    }
    if (rigid) {
        gp_Trsf trsf;
        trsf.SetValues(
            m[0], m[1], m[2], m[3],
            m[4], m[5], m[6], m[7],
            m[8], m[9], m[10], m[11]);
        BRepBuilderAPI_Transform mover(shape, trsf, Standard_True);
        if (!mover.IsDone() || mover.Shape().IsNull()) {
            err = "applying a pattern transform failed";
            return TopoDS_Shape();
        }
        return mover.Shape();
    }
    gp_GTrsf gtrsf;
    for (int r = 0; r < 3; ++r) {
        for (int c = 0; c < 4; ++c) {
            gtrsf.SetValue(r + 1, c + 1, m[r * 4 + c]);
        }
    }
    BRepBuilderAPI_GTransform mover(shape, gtrsf, Standard_True);
    if (!mover.IsDone() || mover.Shape().IsNull()) {
        err = "applying a scaling pattern transform failed";
        return TopoDS_Shape();
    }
    return mover.Shape();
}

extern "C" PrintcadOcctSweepResult printcad_occt_pattern(
    const uint8_t* base_brep,
    size_t base_brep_len,
    const PcadToolSolid* tools,
    size_t tool_count,
    const double* transforms,
    size_t transform_count,
    const PcadMeshOptions* mesh) {
    if (tools == nullptr || tool_count == 0) {
        return make_sweep_error("pattern has no tool solids");
    }
    if (transforms == nullptr || transform_count == 0) {
        return make_sweep_error("pattern has no transforms");
    }
    try {
        std::string err;
        TopoDS_Shape base;
        if (!load_base_for_op(1, base_brep, base_brep_len, base, err)) {
            return make_sweep_error(err);
        }

        std::vector<std::pair<TopoDS_Shape, bool>> tool_shapes;
        tool_shapes.reserve(tool_count);
        for (size_t i = 0; i < tool_count; ++i) {
            TopoDS_Shape shape;
            if (!brep_read_from_bytes(tools[i].brep, tools[i].len, shape)) {
                return make_sweep_error("failed to read a pattern tool BRep");
            }
            tool_shapes.emplace_back(shape, tools[i].subtractive != 0);
        }

        TopTools_ListOfShape fuse_list;
        TopTools_ListOfShape cut_list;
        for (size_t t = 0; t < transform_count; ++t) {
            for (const auto& [shape, subtractive] : tool_shapes) {
                const TopoDS_Shape moved =
                    transform_shape_4x4(shape, transforms + 16 * t, err);
                if (moved.IsNull()) {
                    return make_sweep_error(err);
                }
                (subtractive ? cut_list : fuse_list).Append(moved);
            }
        }

        TopoDS_Shape result_shape = base;
        if (!fuse_list.IsEmpty()) {
            BRepAlgoAPI_Fuse fuse;
            TopTools_ListOfShape args;
            args.Append(result_shape);
            fuse.SetArguments(args);
            fuse.SetTools(fuse_list);
            fuse.Build();
            if (!fuse.IsDone() || fuse.Shape().IsNull()) {
                return make_sweep_error("fusing pattern occurrences failed");
            }
            result_shape = fuse.Shape();
        }
        if (!cut_list.IsEmpty()) {
            BRepAlgoAPI_Cut cut;
            TopTools_ListOfShape args;
            args.Append(result_shape);
            cut.SetArguments(args);
            cut.SetTools(cut_list);
            cut.Build();
            if (!cut.IsDone() || cut.Shape().IsNull()) {
                return make_sweep_error("cutting pattern occurrences failed");
            }
            result_shape = cut.Shape();
        }

        return finish_solid_result(unify_domain(result_shape), nullptr, mesh);
    } catch (Standard_Failure const& ex) {
        return make_sweep_error(std::string("OCCT exception: ") + ex.GetMessageString());
    } catch (std::exception const& ex) {
        return make_sweep_error(std::string("std::exception: ") + ex.what());
    } catch (...) {
        return make_sweep_error("Unknown exception while patterning");
    }
}

extern "C" PrintcadOcctSweepResult printcad_occt_boolean(
    const uint8_t* base_brep,
    size_t base_brep_len,
    const uint8_t* tool_brep,
    size_t tool_len,
    int32_t kind,
    const PcadMeshOptions* mesh) {
    try {
        std::string err;
        TopoDS_Shape base;
        if (!load_base_for_op(1, base_brep, base_brep_len, base, err)) {
            return make_sweep_error(err);
        }
        TopoDS_Shape tool;
        if (!brep_read_from_bytes(tool_brep, tool_len, tool)) {
            return make_sweep_error("failed to read the boolean tool BRep");
        }

        TopoDS_Shape result_shape;
        switch (kind) {
            case 0: {
                BRepAlgoAPI_Fuse boolean_op(base, tool);
                if (!boolean_op.IsDone() || boolean_op.Shape().IsNull()) {
                    return make_sweep_error("fuse boolean operation failed");
                }
                result_shape = boolean_op.Shape();
                break;
            }
            case 1: {
                BRepAlgoAPI_Cut boolean_op(base, tool);
                if (!boolean_op.IsDone() || boolean_op.Shape().IsNull()) {
                    return make_sweep_error("cut boolean operation failed");
                }
                result_shape = boolean_op.Shape();
                break;
            }
            case 2: {
                BRepAlgoAPI_Common boolean_op(base, tool);
                if (!boolean_op.IsDone() || boolean_op.Shape().IsNull()) {
                    return make_sweep_error("common boolean operation failed");
                }
                result_shape = boolean_op.Shape();
                break;
            }
            default:
                return make_sweep_error(
                    "unknown boolean kind " + std::to_string(kind)
                    + " (expected 0=fuse, 1=cut, 2=common)");
        }
        return finish_solid_result(unify_domain(result_shape), nullptr, mesh);
    } catch (Standard_Failure const& ex) {
        return make_sweep_error(std::string("OCCT exception: ") + ex.GetMessageString());
    } catch (std::exception const& ex) {
        return make_sweep_error(std::string("std::exception: ") + ex.what());
    } catch (...) {
        return make_sweep_error("Unknown exception while combining bodies");
    }
}

extern "C" void printcad_occt_free_sweep_result(PrintcadOcctSweepResult result) {
    std::free(result.brep_blob);
    std::free(result.tool_blob);
    std::free(result.mesh_positions);
    std::free(result.mesh_normals);
    std::free(result.mesh_indices);
    std::free(result.mesh_edges);
    std::free(result.error);
}

extern "C" void printcad_occt_free_string(char* str) {
    if (str != nullptr) {
        std::free(str);
    }
}

extern "C" void printcad_occt_free_result(PrintcadOcctImportResult result) {
    if (result.bodies != nullptr) {
        for (size_t i = 0; i < result.body_count; ++i) {
            PrintcadOcctBody& body = result.bodies[i];
            std::free(body.positions);
            std::free(body.normals);
            std::free(body.colors);
            std::free(body.indices);
            std::free(body.edges);
            std::free(body.name);
        }
        std::free(result.bodies);
    }
    if (result.nodes != nullptr) {
        for (size_t i = 0; i < result.node_count; ++i) {
            std::free(result.nodes[i].name);
        }
        std::free(result.nodes);
    }
    std::free(result.error);
}

extern "C" void printcad_occt_free_brep_import_result(PrintcadOcctBrepImportResult result) {
    if (result.bodies != nullptr) {
        for (size_t i = 0; i < result.body_count; ++i) {
            PrintcadOcctBrepBody& b = result.bodies[i];
            std::free(b.name);
            std::free(b.brep_blob);
            std::free(b.face_colors);
            std::free(b.mesh_positions);
            std::free(b.mesh_normals);
            std::free(b.mesh_colors);
            std::free(b.mesh_indices);
            std::free(b.mesh_edges);
        }
        std::free(result.bodies);
    }
    if (result.nodes != nullptr) {
        for (size_t i = 0; i < result.node_count; ++i) {
            std::free(result.nodes[i].name);
        }
        std::free(result.nodes);
    }
    std::free(result.error);
}