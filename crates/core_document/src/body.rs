//! A body's own types (its look, faces, material, mirror plane) and the
//! document's mutators for them.

use super::*;

/// What a scan body was read as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScanInfo {
    /// The units its file was read in, when the file did not say and they
    /// were not millimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_in: Option<kernel_api::LengthUnit>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Body {
    pub id: BodyId,
    pub name: String,
    pub created_at: i64,
    /// Feature exposed as the body's shape. `None` means the last feature in
    /// the history; setting an earlier feature previews that history state
    /// (features after the tip are excluded from the build).
    #[serde(default)]
    pub tip: Option<FeatureId>,
    /// How the body is drawn when the user chose, instead of the material
    /// colour it came with.
    #[serde(default)]
    pub display: Option<BodyDisplay>,
    /// The user asked for the kernel's repair on this body's imported
    /// shape. The repaired shape is derived from it, like the rest of an
    /// import's geometry.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub repair_requested: bool,
    /// The asset the body's shape is read from in place of the one it
    /// was imported with (op `ReplaceBodyShape`): its first solid becomes
    /// the body's base, or its shape when it has no history. Derived from
    /// the asset, like an import's geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape_asset: Option<Uuid>,
    /// The user asked for this mesh body to become a B-rep solid. The
    /// solid is derived from it, like the rest of an import's geometry.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub solid_requested: bool,
    /// The user asked for this converted solid's facets to be rebuilt on
    /// the surfaces they approximate. Derived from it, like the rest of an
    /// import's geometry.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub refine_requested: bool,
    /// Kept out of the scene: not drawn, picked or framed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// Where the body's own geometry sits in the document. Its features,
    /// sketches and kernel shape stay in the body's frame; the mesh the
    /// scene draws and picks is placed.
    #[serde(default, skip_serializing_if = "BodyPlacement::is_identity")]
    pub placement: BodyPlacement,
    /// A linked copy: the body whose shape this one takes, placed on its
    /// own. Its geometry is derived from the source's and follows every
    /// change to it; it takes no features.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_of: Option<BodyId>,
    /// A linked copy that is the source's mirror image: the plane, in the
    /// source's own frame, it is mirrored across.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<MirrorPlane>,
    /// A part linked from another printCAD file: its shape is that body's
    /// there, read again when the file is reloaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<FileLink>,
    /// The component the body sits in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<ComponentId>,
    /// Kept as it stands: its features are not rebuilt until it thaws.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub frozen: bool,
    /// How a mesh body's triangles are shaded.
    #[serde(default, skip_serializing_if = "is_as_meshed")]
    pub shading: Shading,
    /// A scan: a measured mesh that inspection and mesh editing work on,
    /// with what it was read as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan: Option<ScanInfo>,
    /// Drawn but never picked: clicks go to what is behind it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unselectable: bool,
    /// What it is made of, for its mass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<Material>,
    /// Colours chosen for single faces, over the body's own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub face_colors: Vec<FaceColor>,
    /// Textures pressed into its faces for printing: drawn in the view and
    /// baked into the meshes written for the slicer, the exact solid left
    /// as it is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub textures: Vec<FaceTexture>,
    /// Notes pinned to points on it, in its own frame.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
    /// A shape that stands in for the body's solid while its history does
    /// not build (op `SetBodyStandIn`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stand_in: Option<StandIn>,
}

/// A shape kept to stand in for a body's solid: the asset it is read from,
/// shown while `feature` is the body's tip and its build fails, until a
/// build succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandIn {
    pub feature: FeatureId,
    pub asset: Uuid,
}

impl Body {
    /// A body as `CreateBody` makes it: named, placed where its own frame
    /// is, holding nothing.
    pub fn new(id: BodyId, name: String, created_at: i64) -> Self {
        Self {
            component: None,
            frozen: false,
            shading: Shading::AsMeshed,
            scan: None,
            unselectable: false,
            material: None,
            face_colors: Vec::new(),
            textures: Vec::new(),
            notes: Vec::new(),
            stand_in: None,
            link: None,
            mirror: None,
            id,
            name,
            created_at,
            tip: None,
            display: None,
            repair_requested: false,
            shape_asset: None,
            solid_requested: false,
            refine_requested: false,
            hidden: false,
            placement: BodyPlacement::IDENTITY,
            copy_of: None,
        }
    }
}

/// A texture on some faces of a body. Each face is found by its name when
/// the mesh names it, so the texture follows the face through a rebuild,
/// else by its index; no faces means every face.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaceTexture {
    pub texture: surface_texture::Texture,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub faces: Vec<FaceKey>,
}

/// A face of a body's mesh: its name, when the mesh has one, and its index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FaceKey {
    pub name: kernel_api::naming::TopoName,
    pub index: u32,
}

impl FaceKey {
    /// Face `index` of `mesh`, keyed as it stands.
    pub fn of(mesh: &TriMesh, index: u32) -> Self {
        Self {
            name: mesh.face_names.get(index as usize).copied().unwrap_or(0),
            index,
        }
    }

    /// Whether this is face `index` of `mesh`.
    pub fn is_face(&self, mesh: &TriMesh, index: u32) -> bool {
        match mesh.face_names.get(index as usize) {
            Some(name) if *name != 0 && self.name != 0 => *name == self.name,
            _ => index == self.index,
        }
    }
}

impl FaceTexture {
    /// The faces of `mesh` it covers, by index; empty for all of them.
    pub fn face_indices(&self, mesh: &TriMesh) -> Vec<u32> {
        if self.faces.is_empty() {
            return Vec::new();
        }
        let count = mesh.faces.iter().max().map_or(0, |m| m + 1);
        (0..count)
            .filter(|i| self.faces.iter().any(|k| k.is_face(mesh, *i)))
            .collect()
    }
}

/// A material: its name and density.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    /// Grams per cubic centimetre.
    pub density: f32,
}

/// A face's own colour. The face is found by its name when the mesh
/// names it (so the colour follows the face through a rebuild), else by
/// its index.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FaceColor {
    #[serde(flatten)]
    pub face: FaceKey,
    pub color: [f32; 3],
}

impl FaceColor {
    /// Whether this colour is for face `index` of `mesh`.
    pub fn is_face(&self, mesh: &TriMesh, index: u32) -> bool {
        self.face.is_face(mesh, index)
    }
}

/// `mesh` with every face `colors` names in its colour, the rest in
/// `base` (or the colours the mesh has). Vertices a coloured face shares
/// with another face are split off, so the colour ends at the face's edge.
pub fn mesh_with_face_colors(mesh: &TriMesh, colors: &[FaceColor], base: [f32; 3]) -> TriMesh {
    let mut out = mesh.clone();
    if colors.is_empty() || mesh.faces.len() * 3 != mesh.indices.len() {
        return out;
    }
    if out.colors.len() != out.positions.len() {
        out.colors = vec![base; out.positions.len()];
    }
    // Each vertex belongs to the first face that uses it.
    let mut owner: Vec<Option<u32>> = vec![None; out.positions.len()];
    for (t, face) in mesh.faces.iter().enumerate() {
        for k in 0..3 {
            let v = out.indices[t * 3 + k] as usize;
            owner[v].get_or_insert(*face);
        }
    }
    for (t, face) in mesh.faces.iter().enumerate() {
        let Some(color) = colors.iter().find(|c| c.is_face(mesh, *face)) else {
            continue;
        };
        for k in 0..3 {
            let v = out.indices[t * 3 + k] as usize;
            let v = match owner[v] {
                Some(f) if f != *face => {
                    // Shared with another face: a copy of its own.
                    out.positions.push(out.positions[v]);
                    if let Some(n) = out.normals.get(v).copied() {
                        out.normals.push(n);
                    }
                    out.colors.push(color.color);
                    owner.push(Some(*face));
                    let copy = out.positions.len() - 1;
                    out.indices[t * 3 + k] = copy as u32;
                    copy
                }
                _ => v,
            };
            owner[v] = Some(*face);
            out.colors[v] = color.color;
        }
    }
    out
}

/// A plane a shape is mirrored across: a point on it and its normal.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MirrorPlane {
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

impl MirrorPlane {
    /// `p` mirrored across the plane.
    pub fn point_of(&self, p: [f32; 3]) -> [f32; 3] {
        let n = glam::Vec3::from_array(self.normal).normalize_or_zero();
        let p = glam::Vec3::from_array(p);
        let d = (p - glam::Vec3::from_array(self.point)).dot(n);
        (p - 2.0 * d * n).to_array()
    }

    /// A direction mirrored across the plane.
    pub fn direction_of(&self, v: [f32; 3]) -> [f32; 3] {
        let n = glam::Vec3::from_array(self.normal).normalize_or_zero();
        let v = glam::Vec3::from_array(v);
        (v - 2.0 * v.dot(n) * n).to_array()
    }

    /// A mesh mirrored across the plane, its triangles wound the other way
    /// so they still face out.
    pub fn mesh(&self, mesh: &TriMesh) -> TriMesh {
        let mut out = mesh.clone();
        for p in &mut out.positions {
            *p = self.point_of(*p);
        }
        for n in &mut out.normals {
            *n = self.direction_of(*n);
        }
        for tri in out.indices.as_chunks_mut::<3>().0 {
            tri.swap(1, 2);
        }
        for surface in &mut out.face_surfaces {
            *surface = surface.moved(|p| self.point_of(p), |d| self.direction_of(d));
        }
        out
    }
}

/// A user-chosen look for a body: its colour and how much of it shows.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BodyDisplay {
    pub color: [f32; 3],
    /// 1 is solid; less lets what is behind show through.
    pub opacity: f32,
}

fn is_as_meshed(shading: &Shading) -> bool {
    *shading == Shading::AsMeshed
}

/// How a mesh body's triangles are shaded in the view: as its mesh came
/// (welded where normals agree within 30°), every triangle flat, or
/// smooth across the triangles meeting at a point that turn less than a
/// crease angle from each other.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum Shading {
    #[default]
    AsMeshed,
    Sharp,
    Smooth {
        crease_deg: f32,
    },
}

impl Default for BodyDisplay {
    /// A neutral slate grey: mid-toned, so white and coloured lines, the
    /// dark edges and the selection paint all stand out on it.
    fn default() -> Self {
        Self {
            color: [0.58, 0.61, 0.66],
            opacity: 1.0,
        }
    }
}

impl Document {
    /// Shade a mesh body's triangles as `shading` says.
    pub fn set_body_shading(&mut self, body: BodyId, shading: Shading) {
        if self.body(body).is_some_and(|b| b.shading != shading) {
            self.record_and_apply(op::DocumentOp::SetBodyShading { id: body, shading });
        }
    }

    /// Keep a body as it stands, its features not rebuilt, or let it
    /// rebuild again.
    pub fn set_body_frozen(&mut self, body: BodyId, frozen: bool) {
        if self.body(body).is_some_and(|b| b.frozen != frozen) {
            self.record_and_apply(op::DocumentOp::SetBodyFrozen { id: body, frozen });
        }
    }

    pub fn body_frozen(&self, body: BodyId) -> bool {
        self.body(body).is_some_and(|b| b.frozen)
    }

    /// Mark a mesh body as a scan, with what it was read as, or (`None`)
    /// as an ordinary mesh again. Whether anything changed: only a mesh
    /// body can be a scan.
    pub fn set_body_scan(&mut self, body: BodyId, scan: Option<ScanInfo>) -> bool {
        let changes = self.body(body).is_some_and(|b| b.scan != scan);
        if !changes || (scan.is_some() && !self.is_mesh_body(body)) {
            return false;
        }
        self.record_and_apply(op::DocumentOp::SetBodyScan { id: body, scan });
        true
    }

    /// What the body was read as, when it is a scan.
    pub fn body_scan(&self, body: BodyId) -> Option<ScanInfo> {
        self.body(body)?.scan
    }

    /// Let clicks pick a body, or pass through it.
    pub fn set_body_selectable(&mut self, body: BodyId, selectable: bool) {
        if self
            .body(body)
            .is_some_and(|b| b.unselectable == selectable)
        {
            self.record_and_apply(op::DocumentOp::SetBodySelectable {
                id: body,
                selectable,
            });
        }
    }

    /// Whether clicks pick a body.
    pub fn body_selectable(&self, body: BodyId) -> bool {
        !self.body(body).is_some_and(|b| b.unselectable)
    }

    /// Set the textures pressed into `body`'s faces, all of them at once.
    pub fn set_body_textures(&mut self, body: BodyId, textures: Vec<FaceTexture>) {
        if self.body(body).is_some_and(|b| b.textures != textures) {
            self.record_and_apply(op::DocumentOp::SetBodyTextures { id: body, textures });
        }
    }

    /// Say what a body is made of, or `None` for nothing said.
    pub fn set_body_material(&mut self, body: BodyId, material: Option<Material>) {
        if self.body(body).is_some_and(|b| b.material != material) {
            self.record_and_apply(op::DocumentOp::SetBodyMaterial { id: body, material });
        }
    }

    /// Colour face `index` of `body` (named `name` in its mesh, 0 for
    /// none), or give it back the body's colour with `None`.
    pub fn set_face_color(
        &mut self,
        body: BodyId,
        index: u32,
        name: kernel_api::naming::TopoName,
        color: Option<[f32; 3]>,
    ) {
        let Some(entry) = self.body(body) else {
            return;
        };
        let face = FaceKey { name, index };
        let now = entry
            .face_colors
            .iter()
            .find(|c| c.face == face)
            .map(|c| c.color);
        if now != color {
            self.record_and_apply(op::DocumentOp::SetFaceColor {
                id: body,
                face,
                color,
            });
        }
    }

    /// Colour face `index` of `body`'s mesh as it stands (`None` gives it
    /// the body's colour again), kept under the face's name so it follows
    /// the face through a rebuild. Returns false for a face the body lacks.
    pub fn color_face(&mut self, body: BodyId, index: u32, color: Option<[f32; 3]>) -> bool {
        let Some(mesh) = self.imported_geometry(body).map(|g| Arc::clone(&g.mesh)) else {
            return false;
        };
        if !mesh.faces.contains(&index) {
            return false;
        }
        let name = mesh.face_names.get(index as usize).copied().unwrap_or(0);
        // A colour kept for this face under an older index goes.
        let stale: Vec<FaceColor> = self
            .body(body)
            .into_iter()
            .flat_map(|b| b.face_colors.iter().copied())
            .filter(|c| c.is_face(&mesh, index) && c.face != FaceKey { name, index })
            .collect();
        for c in stale {
            self.set_face_color(body, c.face.index, c.face.name, None);
        }
        self.set_face_color(body, index, name, color);
        true
    }

    /// The colour face `index` of `body` has of its own, if any.
    pub fn face_color(&self, body: BodyId, index: u32) -> Option<[f32; 3]> {
        let mesh = &self.imported_geometry(body)?.mesh;
        self.body(body)?
            .face_colors
            .iter()
            .find(|c| c.is_face(mesh, index))
            .map(|c| c.color)
    }

    /// Give a body a look of its own, or `None` for the one it came with.
    pub fn set_body_display(&mut self, body: BodyId, display: Option<BodyDisplay>) {
        if let Some(entry) = self.body(body)
            && entry.display != display
        {
            self.record_and_apply(op::DocumentOp::SetBodyDisplay { id: body, display });
        }
    }
}
