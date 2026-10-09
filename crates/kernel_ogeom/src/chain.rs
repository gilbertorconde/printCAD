//! `execute_solid_chain`: threads one in-memory `(Model, Shape)` through a
//! body's `SolidOp` list. Native-format blobs appear only at the boundaries:
//! the final result out, and `SolidOp::Boolean`'s external tool in. Tool
//! snapshots for patterns are in-model `Shape`s, with no per-op serialization.

use kernel_api::{
    BoolKind, BooleanOp, ChainError, ChainProbe, FeaturePreview, ProbeAnswer, SolidBuildResult,
    SolidOp, TessellationSettings,
};
use kernel_api::{Profile, TopoName, naming as names};
use ogeom::math::{Point, Vector};
use ogeom::topo::{Model, Shape};

use crate::naming::{self, NameMap};
use crate::ops::{self, pattern};
use crate::{progress, tess};

#[derive(Clone)]
struct ToolSnapshot {
    op: SolidOp,
    subtractive: bool,
    /// The tool itself, for an op a pattern cannot re-run elsewhere: one
    /// that sweeps a face of the solid it was built on.
    solid: Option<Shape>,
}

impl ToolSnapshot {
    /// The tool a pattern can repeat: one that adds or cuts. A tool that
    /// keeps only what it shares with the body has none, since each copy
    /// would keep less of what the one before left.
    fn of(op: &SolidOp, boolean: BooleanOp, solid: Option<Shape>) -> Option<Self> {
        (boolean != BooleanOp::Common).then(|| ToolSnapshot {
            op: op.clone(),
            subtractive: boolean == BooleanOp::Cut,
            solid,
        })
    }
}

/// How many states a [`ChainCache`] keeps.
const KEPT_STATES: usize = 4;

/// The chain as it stood at the start of one of its ops: what a later
/// build whose history agrees up to there resumes from.
struct Saved {
    /// How many ops came before, and their key (see [`prefix_keys`]).
    at: usize,
    key: u64,
    model: Model,
    current: Option<Shape>,
    names: NameMap,
    tools: Vec<Option<ToolSnapshot>>,
    /// The faces the op before `at` made or changed, which a refine at
    /// `at` looks around.
    touched: Option<Vec<Shape>>,
    /// The probes asked of the solid before `at`, with their answers.
    answered: Vec<(ChainProbe, Result<ProbeAnswer, String>)>,
}

/// What one body's builds keep between them, so the next build replays
/// only the history after what changed: the chain's state at the start of
/// the op the last edit changed, and at the end of the chain. A build
/// whose ops agree with a kept state's up to its point resumes there.
#[derive(Default)]
pub struct ChainCache {
    /// Newest last.
    saved: Vec<Saved>,
    /// Each op's key in the last build.
    last: Vec<u64>,
    /// How many ops the last build took from a kept state.
    resumed: usize,
    /// The last build's edited op, what came of it and what followed.
    after_edit: Option<AfterEdit>,
    /// The last build's edited op made the solid the build before's did,
    /// and the rest of its result was that build's.
    reused: bool,
    /// The meshes of the faces the last build drew.
    faces: crate::reuse::FaceMeshes,
    /// Keep every edited op's solid for the next build to compare, whatever
    /// the build's timings say it is worth.
    every_edit: bool,
}

/// What a build made of its edited op (the first that differs from the
/// build before) and of everything after it: when the next build's same op
/// makes the same solid and nothing after it differs, the rest is the same
/// too, and its result is this one's.
struct AfterEdit {
    at: usize,
    /// The ops after `at`, the meshing settings and the probes.
    tail: u64,
    /// The snapshot of the solid the op made, hashed; `None` when the ops
    /// after it took less than twice as long as writing a snapshot, too
    /// little to be worth the comparison (unless the cache keeps every
    /// edit).
    solid: Option<u64>,
    result: SolidBuildResult,
}

impl ChainCache {
    /// Where a build of `prefix` (see [`prefix_keys`]) can resume: the
    /// kept state furthest along that agrees with it, no later than the
    /// previewed feature's start and with every probe asked before it
    /// answered.
    fn resume_point(
        &self,
        prefix: &[u64],
        preview: Option<&std::ops::Range<usize>>,
        probes: &[ChainProbe],
    ) -> Option<&Saved> {
        self.saved
            .iter()
            .filter(|s| s.at > 0 && prefix.get(s.at) == Some(&s.key))
            .filter(|s| preview.is_none_or(|r| s.at <= r.start))
            .filter(|s| {
                probes
                    .iter()
                    .filter(|p| p.after_op < s.at)
                    .all(|p| s.answered.iter().any(|(q, _)| q == p))
            })
            .max_by_key(|s| s.at)
    }

    /// Keep `state`, in place of one with the same key; the oldest goes
    /// when there are too many.
    fn keep(&mut self, state: Saved) {
        self.saved.retain(|s| s.key != state.key);
        self.saved.push(state);
        if self.saved.len() > KEPT_STATES {
            self.saved.remove(0);
        }
    }

    /// How many ops the last build took from a kept state rather than
    /// building them: zero for a build from the first op.
    pub fn resumed(&self) -> usize {
        self.resumed
    }

    /// A cache that keeps every edited op's solid for comparison, rather than
    /// only those whose following ops took long enough to be worth it:
    /// what it reuses then depends on the history alone, not on how fast
    /// the machine built it.
    pub fn keeping_every_edit() -> Self {
        Self {
            every_edit: true,
            ..Self::default()
        }
    }

    /// Whether the last build stopped at its edited op, which made the
    /// solid the build before's did, and took the rest from that build.
    pub fn reused(&self) -> bool {
        self.reused
    }

    /// How many faces' meshes the last build left for the next.
    pub fn faces_kept(&self) -> usize {
        self.faces.len()
    }

    /// How many states it keeps.
    pub fn len(&self) -> usize {
        self.saved.len()
    }

    /// Whether it keeps none.
    pub fn is_empty(&self) -> bool {
        self.saved.is_empty()
    }
}

/// Hash a value by its JSON. One that did not serialize hashes as a
/// number never used before, so its key matches nothing cached.
fn hash_json(json: serde_json::Result<Vec<u8>>, hasher: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    static UNMATCHED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    match json {
        Ok(bytes) => bytes.hash(hasher),
        Err(_) => UNMATCHED
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .hash(hasher),
    }
}

/// Each op's key: the op and its tag.
fn op_keys(ops_list: &[SolidOp], tags: &[TopoName]) -> Vec<u64> {
    use std::hash::{Hash, Hasher};
    ops_list
        .iter()
        .zip(tags)
        .map(|(op, tag)| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            hash_json(serde_json::to_vec(op), &mut hasher);
            tag.hash(&mut hasher);
            hasher.finish()
        })
        .collect()
}

/// The key of every prefix of a chain: `[k]` stands for its first `k`
/// ops, one more entry than ops.
fn prefix_keys(op_keys: &[u64]) -> Vec<u64> {
    use std::hash::{Hash, Hasher};
    let mut keys = Vec::with_capacity(op_keys.len() + 1);
    keys.push(0);
    for op in op_keys {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        keys.last().hash(&mut hasher);
        op.hash(&mut hasher);
        keys.push(hasher.finish());
    }
    keys
}

pub fn execute(
    ops_list: &[SolidOp],
    detail: &TessellationSettings,
) -> Result<SolidBuildResult, ChainError> {
    execute_previewing(ops_list, detail, None)
}

/// [`execute`], and with `preview` (the ops of the feature being edited)
/// what that feature does: its tool solid and the body without it, in
/// [`SolidBuildResult::preview`]. A feature with no tool of its own (a
/// dress-up, a pattern) has no preview.
pub fn execute_previewing(
    ops_list: &[SolidOp],
    detail: &TessellationSettings,
    preview: Option<std::ops::Range<usize>>,
) -> Result<SolidBuildResult, ChainError> {
    execute_probing(ops_list, detail, preview, &[])
}

/// [`execute_previewing`], and the answers to `probes`, each asked of the
/// solid as it stands after its `after_op` ops, in
/// [`SolidBuildResult::probes`]. A probe asked before any op, or past the
/// chain's end, is answered with an error.
pub fn execute_probing(
    ops_list: &[SolidOp],
    detail: &TessellationSettings,
    preview: Option<std::ops::Range<usize>>,
    probes: &[ChainProbe],
) -> Result<SolidBuildResult, ChainError> {
    execute_named(ops_list, &[], detail, preview, probes)
}

/// The name each op's faces are named under: `tags[index]` (the feature
/// the op builds, [`kernel_api::naming::name_of_id`] of its id), the second
/// and later ops of one feature told apart by their count; an op without
/// a tag is named by where it stands in the chain.
fn op_tags(ops_list: &[SolidOp], tags: &[TopoName]) -> Vec<TopoName> {
    let mut seen: Vec<(TopoName, u32)> = Vec::new();
    (0..ops_list.len())
        .map(|index| {
            let tag = tags
                .get(index)
                .copied()
                .filter(|t| *t != 0)
                .unwrap_or_else(|| names::name_of(format!("op {index}").as_bytes()));
            let count = match seen.iter_mut().find(|(t, _)| *t == tag) {
                Some((_, n)) => {
                    *n += 1;
                    *n
                }
                None => {
                    seen.push((tag, 0));
                    0
                }
            };
            if count == 0 {
                tag
            } else {
                names::child(tag, &count.to_le_bytes())
            }
        })
        .collect()
}

/// The names of a sweep's tool: its walls after the profile's segments
/// (every sweep's wall passes through the segment it sweeps) and its ends
/// after where they stand along the profile's normal; the rest afresh.
fn sweep_names(model: &Model, tool: &Shape, tag: TopoName, profile: &Profile) -> NameMap {
    let plane = &profile.plane;
    let origin = Point::new(plane.origin[0], plane.origin[1], plane.origin[2]);
    let normal = Vector::new(plane.normal[0], plane.normal[1], plane.normal[2]);
    naming::tool_names(
        model,
        tool,
        tag,
        &naming::profile_segments(profile),
        origin,
        normal,
    )
}

/// A tool's faces named afresh, by their surfaces and where they face.
fn tool_names_fresh(model: &Model, tool: &Shape, tag: TopoName) -> NameMap {
    NameMap::assign(model, tool, tag, |_| Vec::new())
}

/// [`execute_probing`], naming the faces of the solid as it builds under
/// `tags`, one per op (see [`op_tags`]): the result's mesh carries each
/// face's name and each edge's faces' names, and references with names
/// find their faces by them.
pub fn execute_named(
    ops_list: &[SolidOp],
    tags: &[TopoName],
    detail: &TessellationSettings,
    preview: Option<std::ops::Range<usize>>,
    probes: &[ChainProbe],
) -> Result<SolidBuildResult, ChainError> {
    execute_cached(ops_list, tags, detail, preview, probes, None)
}

/// [`execute_named`], resuming from the state `cache` kept where the
/// history agrees with the last builds', and keeping the states the next
/// build may resume from: at the start of the first op that differs from
/// the last build's (where the user is editing) and at the end.
pub fn execute_cached(
    ops_list: &[SolidOp],
    tags: &[TopoName],
    detail: &TessellationSettings,
    preview: Option<std::ops::Range<usize>>,
    probes: &[ChainProbe],
    mut cache: Option<&mut ChainCache>,
) -> Result<SolidBuildResult, ChainError> {
    let chain_err = |op_index: usize, message: String| ChainError { op_index, message };

    if ops_list.is_empty() {
        return Err(chain_err(0, "solid-op chain is empty".into()));
    }
    if !ops_list[0].starts_shape() {
        let message = match ops_list[0].boolean_op() {
            Some(_) => "first solid op in a chain must be NewSolid",
            None => "first solid op in a chain must produce a shape",
        };
        return Err(chain_err(0, message.into()));
    }
    for (index, op) in ops_list.iter().enumerate().skip(1) {
        if op.boolean_op() == Some(BooleanOp::NewSolid) {
            return Err(chain_err(
                index,
                "only the first op in a chain may be NewSolid".into(),
            ));
        }
    }

    let mut model = Model::with_tolerances(tess::tolerances());
    let mut current: Option<Shape> = None;
    let op_tags = op_tags(ops_list, tags);
    let mut names = NameMap::default();
    let mut tools: Vec<Option<ToolSnapshot>> = Vec::with_capacity(ops_list.len());
    // The previewed feature: the body before it, after it, and its tools.
    let previewing = |index: usize| preview.as_ref().is_some_and(|r| r.contains(&index));
    let mut before: Option<Shape> = None;
    let mut after: Option<Shape> = None;
    let mut preview_tools: Vec<Shape> = Vec::new();
    let mut preview_cuts = false;
    let mut answers: Vec<Result<ProbeAnswer, String>> = probes
        .iter()
        .map(|p| {
            Err(if p.after_op == 0 {
                "there is no solid before it".to_string()
            } else {
                "it stands past the end of the history".to_string()
            })
        })
        .collect();

    // Where the last builds left a state this one agrees with, it starts
    // there. The op that differs from the last build's is where the next
    // edit most likely is, so the state there is kept; with no build
    // before, the last feature is the likeliest.
    let mut start = 0;
    let mut keep_at = None;
    // The faces the op before made or changed, as its kernel operations
    // said; unknown after an import or an op that said nothing.
    let mut touched: Option<Vec<Shape>> = None;
    let mut edit: Option<(usize, u64)> = None;
    let keys = cache.as_ref().map(|_| op_keys(ops_list, &op_tags));
    let prefix = keys.as_deref().map(prefix_keys);
    if let (Some(cache), Some(prefix), Some(keys)) = (cache.as_deref_mut(), prefix.as_ref(), keys) {
        if let Some(saved) = cache.resume_point(prefix, preview.as_ref(), probes) {
            start = saved.at;
            model = saved.model.clone();
            current = saved.current.clone();
            names = saved.names.clone();
            tools = saved.tools.clone();
            touched = saved.touched.clone();
            for (probe, answer) in probes.iter().zip(answers.iter_mut()) {
                if let Some((_, known)) = saved.answered.iter().find(|(q, _)| q == probe) {
                    *answer = known.clone();
                }
            }
            tracing::debug!(
                target: "printcad.chain",
                "resumed after {start} of {} ops",
                ops_list.len()
            );
        }
        cache.resumed = start;
        cache.reused = false;
        let edited = if cache.last.is_empty() {
            last_feature_start(tags, ops_list.len())
        } else {
            keys.iter()
                .zip(&cache.last)
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| keys.len().min(cache.last.len()))
        };
        keep_at = Some(edited).filter(|&at| at > start && at < ops_list.len());
        if edited < ops_list.len() && preview.is_none() {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            keys[edited + 1..].hash(&mut hasher);
            hash_json(serde_json::to_vec(&(detail, probes)), &mut hasher);
            edit = Some((edited, hasher.finish()));
        }
        cache.last = keys;
    }
    // The edited op's solid, and when the ops after it began.
    let mut edit_solid: Option<Shape> = None;
    let mut tail_started: Option<web_time::Instant> = None;

    for (index, solid_op) in ops_list.iter().enumerate().skip(start) {
        if keep_at == Some(index)
            && let (Some(cache), Some(prefix)) = (cache.as_deref_mut(), prefix.as_ref())
        {
            // The model keeps every intermediate result; what the chain
            // still reaches is all it needs, and a clone costs what is kept.
            let mut roots: Vec<Shape> = [&current, &before, &after, &edit_solid]
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            roots.extend(preview_tools.iter().cloned());
            roots.extend(tools.iter().flatten().filter_map(|t| t.solid.clone()));
            compact(&mut model, &roots);
            cache.keep(Saved {
                at: index,
                key: prefix[index],
                model: model.clone(),
                current: current.clone(),
                names: names.clone(),
                tools: tools.clone(),
                touched: touched.clone(),
                answered: answered_before(probes, &answers, index),
            });
        }
        let tag = op_tags[index];
        // What the op and the probes asked before it look up by name is
        // found in the solid as it stands.
        let named = naming::set_current(std::mem::take(&mut names));
        if index > 0
            && let Some(shape) = current.as_ref()
        {
            ask(&mut model, shape, index, probes, &mut answers);
        }
        let mut tool_names: Option<NameMap> = None;
        progress::context(format_args!(
            "{} {}/{}",
            progress::op_label(solid_op),
            index + 1,
            ops_list.len()
        ));
        let err = |message: String| ChainError {
            op_index: index,
            message,
        };
        progress::checkpoint().map_err(&err)?;
        let op_started = web_time::Instant::now();
        let base = current.clone();
        if preview.as_ref().is_some_and(|r| r.start == index) {
            before = base.clone();
        }
        let mut tool_snapshot: Option<ToolSnapshot> = None;
        // Keep a feature's tool when it is the one previewed.
        let mut keep = |tool: &Shape, op: BooleanOp| {
            if previewing(index) {
                preview_tools.push(tool.clone());
                preview_cuts |= matches!(op, BooleanOp::Cut | BooleanOp::Common);
            }
        };

        let next = match solid_op {
            SolidOp::Shape { brep } => absorb_shape(&mut model, brep).map_err(&err)?,
            SolidOp::Sweep { profile, kind, op } => {
                let tool = ops::sweep::build_tool(&mut model, base.as_ref(), profile, kind)
                    .map_err(&err)?;
                tool_names = Some(sweep_names(&model, &tool, tag, profile));
                tool_snapshot = ToolSnapshot::of(solid_op, *op, None);
                keep(&tool, *op);

                combine(&mut model, base.as_ref(), tool, *op).map_err(&err)?
            }
            SolidOp::SweepFace { face, kind, op } => {
                let tool = ops::sweep::build_face_tool(&mut model, base.as_ref(), face, kind)
                    .map_err(&err)?;
                tool_names = Some(tool_names_fresh(&model, &tool, tag));
                tool_snapshot = ToolSnapshot::of(solid_op, *op, Some(tool.clone()));
                keep(&tool, *op);

                combine(&mut model, base.as_ref(), tool, *op).map_err(&err)?
            }
            SolidOp::SweepFaceOf {
                shape,
                transform,
                point,
                kind,
                op,
            } => {
                let tool = ops::sweep::build_face_of_tool(
                    &mut model,
                    base.as_ref(),
                    shape,
                    transform.as_deref(),
                    *point,
                    kind,
                )
                .map_err(&err)?;
                tool_names = Some(tool_names_fresh(&model, &tool, tag));
                tool_snapshot = ToolSnapshot::of(solid_op, *op, Some(tool.clone()));
                keep(&tool, *op);

                combine(&mut model, base.as_ref(), tool, *op).map_err(&err)?
            }
            SolidOp::RevolveToFaceOf {
                profile,
                kind,
                shape,
                transform,
                point,
                op,
            } => {
                let tool = ops::sweep::build_revolve_to_face_of_tool(
                    &mut model,
                    profile,
                    kind,
                    shape,
                    transform.as_deref(),
                    *point,
                )
                .map_err(&err)?;
                tool_names = Some(sweep_names(&model, &tool, tag, profile));
                tool_snapshot = ToolSnapshot::of(solid_op, *op, Some(tool.clone()));
                keep(&tool, *op);

                combine(&mut model, base.as_ref(), tool, *op).map_err(&err)?
            }
            SolidOp::Primitive {
                kind,
                placement,
                op,
            } => {
                let tool = ops::primitive::build_tool(&mut model, kind, placement).map_err(&err)?;
                tool_names = Some(tool_names_fresh(&model, &tool, tag));
                tool_snapshot = ToolSnapshot::of(solid_op, *op, None);
                keep(&tool, *op);

                combine(&mut model, base.as_ref(), tool, *op).map_err(&err)?
            }
            SolidOp::LoftThrough {
                sections,
                ruled,
                closed,
                op,
            } => {
                let tool = ops::loft_pipe::loft_through_tool(
                    &mut model,
                    base.as_ref(),
                    sections,
                    *ruled,
                    *closed,
                )
                .map_err(&err)?;
                tool_names = Some(tool_names_fresh(&model, &tool, tag));
                tool_snapshot = ToolSnapshot::of(solid_op, *op, Some(tool.clone()));
                keep(&tool, *op);

                combine(&mut model, base.as_ref(), tool, *op).map_err(&err)?
            }
            SolidOp::Loft {
                sections,
                ruled,
                closed,
                op,
            } => {
                let tool = ops::loft_pipe::loft_tool(&mut model, sections, *ruled, *closed)
                    .map_err(&err)?;
                tool_names = Some(match sections.first() {
                    Some(first) if !*closed => {
                        let plane = &first.plane;
                        naming::tool_names(
                            &model,
                            &tool,
                            tag,
                            &naming::profile_segments(first),
                            Point::new(plane.origin[0], plane.origin[1], plane.origin[2]),
                            Vector::new(plane.normal[0], plane.normal[1], plane.normal[2]),
                        )
                    }
                    _ => tool_names_fresh(&model, &tool, tag),
                });
                tool_snapshot = ToolSnapshot::of(solid_op, *op, None);
                keep(&tool, *op);

                combine(&mut model, base.as_ref(), tool, *op).map_err(&err)?
            }
            SolidOp::Pipe {
                profile,
                spine,
                frame,
                corner,
                sections,
                op,
            } => {
                let tool =
                    ops::loft_pipe::pipe_tool(&mut model, profile, spine, frame, *corner, sections)
                        .map_err(&err)?;
                tool_names = Some(tool_names_fresh(&model, &tool, tag));
                tool_snapshot = ToolSnapshot::of(solid_op, *op, None);
                keep(&tool, *op);

                combine(&mut model, base.as_ref(), tool, *op).map_err(&err)?
            }
            SolidOp::PipeThrough {
                profile,
                path,
                frame,
                corner,
                sections,
                op,
            } => {
                let tool = ops::loft_pipe::pipe_through_tool(
                    &mut model,
                    base.as_ref(),
                    profile,
                    path,
                    frame,
                    *corner,
                    sections,
                )
                .map_err(&err)?;
                tool_names = Some(tool_names_fresh(&model, &tool, tag));
                tool_snapshot = ToolSnapshot::of(solid_op, *op, Some(tool.clone()));
                keep(&tool, *op);

                combine(&mut model, base.as_ref(), tool, *op).map_err(&err)?
            }
            SolidOp::Fillet {
                radius,
                edges,
                follow_tangent,
            } => {
                let solid = base.ok_or_else(|| err("fillet needs an existing solid".into()))?;
                ops::dressup::fillet(&mut model, &solid, *radius, edges, *follow_tangent)
                    .map_err(&err)?
            }
            SolidOp::Chamfer {
                spec,
                flip,
                edges,
                follow_tangent,
            } => {
                let solid = base.ok_or_else(|| err("chamfer needs an existing solid".into()))?;
                ops::dressup::chamfer(&mut model, &solid, spec, *flip, edges, *follow_tangent)
                    .map_err(&err)?
            }
            SolidOp::Draft {
                angle_deg,
                neutral_point,
                neutral_normal,
                pull_dir,
                faces,
                face_names,
            } => {
                let solid = base.ok_or_else(|| err("draft needs an existing solid".into()))?;
                ops::dressup::draft(
                    &mut model,
                    &solid,
                    *angle_deg,
                    *neutral_point,
                    *neutral_normal,
                    *pull_dir,
                    &faces
                        .iter()
                        .enumerate()
                        .map(|(i, p)| (*p, face_names.get(i).copied().unwrap_or(0)))
                        .collect::<Vec<_>>(),
                )
                .map_err(&err)?
            }
            SolidOp::Refine => {
                let solid = base.ok_or_else(|| err("refine needs an existing solid".into()))?;
                // After a feature, only the faces it made or changed are
                // merged with their neighbours: the rest of the part was
                // refined when it was made.
                match touched.as_deref() {
                    Some(around) => ogeom::heal::unify_same_domain_around(
                        &mut model,
                        &solid,
                        around,
                        ops::tol(),
                    ),
                    None => ogeom::heal::unify_same_domain(&mut model, &solid, ops::tol()),
                }
                .map(|(built, _)| {
                    naming::record(&built.history);
                    built.shape
                })
                .map_err(|e| err(format!("refine failed: {e}")))?
            }
            SolidOp::OffsetFaces {
                faces,
                face_names,
                distance,
            } => {
                let solid =
                    base.ok_or_else(|| err("offsetting faces needs an existing solid".into()))?;
                ops::dressup::offset_faces_of(&mut model, &solid, faces, face_names, *distance)
                    .map_err(&err)?
            }
            SolidOp::MoveFaces {
                faces,
                face_names,
                transform,
            } => {
                let solid =
                    base.ok_or_else(|| err("moving faces needs an existing solid".into()))?;
                ops::dressup::move_faces_of(&mut model, &solid, faces, face_names, transform)
                    .map_err(&err)?
            }
            SolidOp::RemoveFaces { faces, face_names } => {
                let solid =
                    base.ok_or_else(|| err("removing faces needs an existing solid".into()))?;
                ops::dressup::remove_faces(&mut model, &solid, faces, face_names).map_err(&err)?
            }
            SolidOp::Thickness {
                value,
                open_faces,
                open_face_names,
                inward,
                join,
                both_sides,
            } => {
                let solid = base.ok_or_else(|| err("thickness needs an existing solid".into()))?;
                ops::dressup::thickness(
                    &mut model,
                    &solid,
                    *value,
                    open_faces,
                    open_face_names,
                    if *both_sides { None } else { Some(*inward) },
                    *join,
                )
                .map_err(&err)?
            }
            SolidOp::Transform {
                transforms,
                originals,
            } => {
                let solid = base.ok_or_else(|| err("pattern needs an existing solid".into()))?;
                let instances: Vec<pattern::ToolInstance> = if originals.is_empty() {
                    vec![pattern::ToolInstance {
                        tool: pattern::PatternTool::Solid(solid.clone()),
                        subtractive: false,
                    }]
                } else {
                    originals
                        .iter()
                        .map(|&orig| {
                            tools
                                .get(orig)
                                .and_then(|t| t.as_ref())
                                .map(|t| pattern::ToolInstance {
                                    tool: match &t.solid {
                                        Some(solid) => pattern::PatternTool::Solid(solid.clone()),
                                        None => pattern::PatternTool::Op(Box::new(t.op.clone())),
                                    },
                                    subtractive: t.subtractive,
                                })
                                .ok_or_else(|| {
                                    err("pattern references an op with no reusable tool solid"
                                        .into())
                                })
                        })
                        .collect::<Result<_, _>>()?
                };
                pattern::apply(&mut model, solid, &instances, transforms).map_err(&err)?
            }
            SolidOp::Boolean {
                tool_brep,
                kind,
                tool_transform,
            } => {
                let solid = base.ok_or_else(|| err("boolean needs an existing solid".into()))?;
                let tool =
                    external_tool(&mut model, tool_brep, tool_transform.as_ref()).map_err(&err)?;
                let cuts = *kind == BoolKind::Cut;
                keep(
                    &tool,
                    if cuts {
                        BooleanOp::Cut
                    } else {
                        BooleanOp::Fuse
                    },
                );
                ops::combine_solids(&mut model, &solid, &tool, *kind).map_err(&err)?
            }
            SolidOp::Surface(op) => {
                let made = ops::surface::apply(&mut model, base.as_ref(), op).map_err(&err)?;
                if let Some(sheet) = &made.sheet {
                    tool_names = Some(tool_names_fresh(&model, sheet, tag));
                }
                made.shape
            }
        };

        // The result's faces take the names of the faces they came from.
        let (before_names, histories) = named.take_with_histories();
        touched = match current.as_ref() {
            Some(before) if !histories.is_empty() => {
                Some(naming::touched_faces(&model, before, &next, &histories))
            }
            _ => None,
        };
        let naming_started = web_time::Instant::now();
        names = match solid_op {
            SolidOp::Shape { .. } => tool_names_fresh(&model, &next, tag),
            _ => {
                let mut sources: Vec<&NameMap> = vec![&before_names];
                if let Some(tool) = &tool_names {
                    sources.push(tool);
                }
                NameMap::carry(&model, &next, &sources, tag, &histories)
            }
        };
        current = Some(next);
        tracing::debug!(
            target: "printcad.chain",
            "op {index} {}: {:.1} ms, naming {:.1} ms",
            progress::op_label(solid_op),
            op_started.elapsed().as_secs_f64() * 1000.0,
            naming_started.elapsed().as_secs_f64() * 1000.0
        );
        if let Some((at, tail)) = edit
            && at == index
        {
            // The edited op made the solid the last build's did, and what
            // follows is the same: so is the result.
            let earlier = cache.as_deref().and_then(|c| c.after_edit.as_ref());
            if let Some(earlier) = earlier.filter(|e| e.at == at && e.tail == tail)
                && let Some(solid) = earlier.solid
                && current.as_ref().and_then(|s| fingerprint(&model, s)) == Some(solid)
            {
                tracing::debug!(
                    target: "printcad.chain",
                    "op {index} made the same solid as before: the rest is as built"
                );
                let result = earlier.result.clone();
                if let Some(cache) = cache.as_deref_mut() {
                    cache.reused = true;
                }
                return Ok(result);
            }
            edit_solid = current.clone();
            tail_started = Some(web_time::Instant::now());
        }
        if preview.as_ref().is_some_and(|r| r.end == index + 1) {
            after = current.clone();
        }
        tools.push(tool_snapshot);
    }

    let final_shape = current.expect("chain validated non-empty");
    let kept_names = cache.is_some().then(|| names.clone());
    let named = naming::set_current(names);
    ask(
        &mut model,
        &final_shape,
        ops_list.len(),
        probes,
        &mut answers,
    );
    let names = named.take();
    let meshing = web_time::Instant::now();
    let reuse = cache.as_deref_mut().map(|c| &mut c.faces);
    let mesh = tess::mesh_named(&model, &final_shape, detail, &names, reuse).map_err(|e| {
        chain_err(
            ops_list.len() - 1,
            format!("meshing the result failed: {e}"),
        )
    })?;
    if mesh.positions.is_empty() || mesh.indices.is_empty() {
        return Err(chain_err(
            ops_list.len() - 1,
            "solid-op chain produced an empty render mesh".into(),
        ));
    }
    tracing::debug!(
        target: "printcad.chain",
        "meshing: {:.1} ms",
        meshing.elapsed().as_secs_f64() * 1000.0
    );
    let tail_took = tail_started.map(|t| t.elapsed());
    let writing = web_time::Instant::now();
    let brep_blob = tess::write_blob(&model, &final_shape).map_err(|e| {
        chain_err(
            ops_list.len() - 1,
            format!("serializing the result failed: {e}"),
        )
    })?;

    let snapshot_took = writing.elapsed();
    tracing::debug!(
        target: "printcad.chain",
        "snapshot: {:.1} ms",
        snapshot_took.as_secs_f64() * 1000.0
    );
    // The box the faces kept and measured make, else the solid measured.
    let bounds_mm = cache
        .as_deref_mut()
        .and_then(|c| c.faces.take_bounds())
        .map(|(lo, hi)| {
            (
                [lo.x as f32, lo.y as f32, lo.z as f32],
                [hi.x as f32, hi.y as f32, hi.z as f32],
            )
        })
        .or_else(|| tess::solid_bounds(&model, &final_shape, &mesh));
    // A preview that cannot be made leaves the build as it is.
    let preview = (!preview_tools.is_empty())
        .then(|| {
            let shown = if preview_cuts { after } else { before };
            feature_preview(&mut model, preview_tools, shown, preview_cuts, detail)
        })
        .flatten()
        .map(Box::new);
    let result = SolidBuildResult {
        brep_blob,
        mesh,
        bounds_mm,
        preview,
        probes: answers,
    };
    if let (Some(cache), Some((at, tail))) = (cache.as_deref_mut(), edit) {
        let worth = cache.every_edit || tail_took.is_some_and(|t| t > snapshot_took * 2);
        cache.after_edit = Some(AfterEdit {
            at,
            tail,
            solid: edit_solid
                .filter(|_| worth)
                .and_then(|s| fingerprint(&model, &s)),
            result: result.clone(),
        });
    }
    // The state at the end, for a build that changes nothing but the
    // meshing, or adds a feature after it.
    if let (Some(cache), Some(prefix), Some(names)) = (cache, prefix.as_ref(), kept_names)
        && start < ops_list.len()
    {
        let answered = answered_before(probes, &result.probes, ops_list.len());
        let mut model = model;
        let mut roots = vec![final_shape.clone()];
        roots.extend(tools.iter().flatten().filter_map(|t| t.solid.clone()));
        compact(&mut model, &roots);
        cache.keep(Saved {
            at: ops_list.len(),
            key: prefix[ops_list.len()],
            model,
            current: Some(final_shape),
            names,
            tools,
            touched,
            answered,
        });
    }
    Ok(result)
}

/// Drop from `model` what `roots` do not reach, the handles into what they
/// do kept as they are. A model the kernel will not compact (one whose
/// tolerances were widened) is left whole: it is only larger.
fn compact(model: &mut Model, roots: &[Shape]) {
    if let Err(e) = model.retain_reachable(roots) {
        tracing::debug!(target: "printcad.chain", "the model was left whole: {e}");
    }
}

/// A solid's geometry, hashed: every vertex, three points along every
/// edge and every face's kept box, each to a micrometre, with the way
/// each face faces and the kind of its surface. Equal for the same solid however its curves and surfaces
/// record their parameter ranges, which a snapshot would tell apart.
fn fingerprint(model: &Model, shape: &Shape) -> Option<u64> {
    use ogeom::topo::{NodeData, ShapeType, explore_unique};
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let micrometres = |p: Point| [p.x, p.y, p.z].map(|c| (c * 1e3).round() as i64);
    for vertex in explore_unique(model, shape, ShapeType::Vertex).ok()? {
        let Some(NodeData::Vertex(data)) = model.node(&vertex).map(|n| n.data()) else {
            return None;
        };
        let placement = vertex.transform(model.datums()).ok()?;
        micrometres(placement.apply(data.point)).hash(&mut hasher);
    }
    for edge in explore_unique(model, shape, ShapeType::Edge).ok()? {
        // A degenerate edge (a cone's apex) has no curve to sample.
        if let Ok(points) = crate::queries::edge_samples(model, &edge, &[0.25, 0.5, 0.75]) {
            for p in points {
                micrometres(p).hash(&mut hasher);
            }
        }
    }
    for face in explore_unique(model, shape, ShapeType::Face).ok()? {
        let bounds = ogeom::algo::face_bounds(model, &face).ok()?;
        let (lo, hi) = (bounds.low()?, bounds.high()?);
        micrometres(lo).hash(&mut hasher);
        micrometres(hi).hash(&mut hasher);
        format!("{:?}", face.orientation()).hash(&mut hasher);
        crate::naming::surface_kind(model, &face).hash(&mut hasher);
    }
    Some(hasher.finish())
}

/// Where the last feature's ops start: the trailing run of one tag.
fn last_feature_start(tags: &[TopoName], ops: usize) -> usize {
    match tags.get(..ops).and_then(<[_]>::last) {
        Some(last) => ops - tags[..ops].iter().rev().take_while(|t| *t == last).count(),
        None => ops,
    }
}

/// The probes asked of the solid before op `at`, with their answers.
fn answered_before(
    probes: &[ChainProbe],
    answers: &[Result<ProbeAnswer, String>],
    at: usize,
) -> Vec<(ChainProbe, Result<ProbeAnswer, String>)> {
    probes
        .iter()
        .zip(answers)
        .filter(|(p, _)| p.after_op < at)
        .map(|(p, a)| (*p, a.clone()))
        .collect()
}

/// Answer the probes asked of the solid after `after_op` ops.
fn ask(
    model: &mut Model,
    shape: &Shape,
    after_op: usize,
    probes: &[ChainProbe],
    answers: &mut [Result<ProbeAnswer, String>],
) {
    for (probe, answer) in probes.iter().zip(answers.iter_mut()) {
        if probe.after_op == after_op {
            *answer = crate::probe::answer(model, shape, &probe.probe);
        }
    }
}

/// The preview of a feature: its tools meshed as one, and the body to show
/// beside them.
fn feature_preview(
    model: &mut Model,
    tools: Vec<Shape>,
    shown: Option<Shape>,
    cuts: bool,
    detail: &TessellationSettings,
) -> Option<FeaturePreview> {
    let tool = match tools.as_slice() {
        [one] => one.clone(),
        many => model.add_compound(many).ok()?,
    };
    let tool = tess::mesh_shape(model, &tool, &[], detail).ok()?;
    let shown = match shown {
        Some(shape) => {
            let mesh = tess::mesh_shape(model, &shape, &[], detail).ok()?;
            let brep_blob = tess::write_blob(model, &shape).ok()?;
            let bounds_mm = tess::solid_bounds(model, &shape, &mesh);
            Some(Box::new(SolidBuildResult {
                brep_blob,
                mesh,
                bounds_mm,
                preview: None,
                probes: Vec::new(),
            }))
        }
        None => None,
    };
    Some(FeaturePreview { shown, tool, cuts })
}

/// Combine a freshly built tool with the running solid.
fn combine(
    model: &mut Model,
    base: Option<&Shape>,
    tool: Shape,
    op: BooleanOp,
) -> Result<Shape, String> {
    match op {
        BooleanOp::NewSolid => Ok(tool),
        BooleanOp::Fuse => {
            let base =
                base.ok_or_else(|| "fuse requires existing material in the body".to_string())?;
            if !ops::bounds_overlap(model, base, &tool) {
                return ops::fuse_or_compound(model, base, &tool);
            }
            ops::combine_solids(model, base, &tool, BoolKind::Fuse)
        }
        BooleanOp::Cut => {
            let base =
                base.ok_or_else(|| "cut requires existing material in the body".to_string())?;
            ops::combine_solids(model, base, &tool, BoolKind::Cut)
        }
        BooleanOp::Common => {
            let base = base.ok_or_else(|| {
                "keeping the intersection requires existing material in the body".to_string()
            })?;
            ops::combine_solids(model, base, &tool, BoolKind::Common)
        }
    }
}

/// A native-format snapshot read into the model: the shape it holds.
pub(crate) fn absorb_shape(model: &mut Model, brep: &[u8]) -> Result<Shape, String> {
    let text =
        std::str::from_utf8(brep).map_err(|_| "solid snapshot is not valid UTF-8".to_string())?;
    let absorbed = ogeom::io::native::read_into(model, text)
        .map_err(|e| format!("importing the solid snapshot failed: {e}"))?;
    absorbed
        .shapes
        .first()
        .cloned()
        .ok_or_else(|| "solid snapshot holds no shape".to_string())
}

/// Another body's solid as a boolean's tool, where it sits relative to
/// this one.
fn external_tool(
    model: &mut Model,
    tool_brep: &[u8],
    tool_transform: Option<&[[f64; 4]; 4]>,
) -> Result<Shape, String> {
    let mut tool = absorb_shape(model, tool_brep)?;
    if let Some(matrix) = tool_transform {
        tool = pattern::moved(model, &tool, matrix)?;
    }
    Ok(tool)
}
