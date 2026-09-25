//! The registry side of the workbench seam: what the host learns about a
//! bench from its descriptor alone.

use core_document::{
    CameraOrientRequest, Document, DocumentError, DocumentResult, DocumentService, FeatureId,
    FeatureNode, HookOutcome, HostRequest, MenuItem, MenuScope, PassiveGeometry, PropertyHints,
    ViewportPick, Workbench, WorkbenchContext, WorkbenchDescriptor, WorkbenchFeature, WorkbenchId,
    WorkbenchRuntimeContext,
};
use uuid::Uuid;

/// A bench that declares itself and, when asked to, answers picks with a
/// fixed distance and draws a one-vertex mesh for every feature it owns.
struct FakeBench {
    id: &'static str,
    kinds: Vec<&'static str>,
    modal: bool,
    pick_distance: Option<f32>,
    draws: bool,
    items: Vec<&'static str>,
    lengths: Vec<&'static str>,
}

impl FakeBench {
    fn new(id: &'static str) -> Self {
        Self {
            id,
            kinds: Vec::new(),
            modal: false,
            pick_distance: None,
            draws: false,
            items: Vec::new(),
            lengths: Vec::new(),
        }
    }
    fn offering(mut self, items: &[&'static str]) -> Self {
        self.items = items.to_vec();
        self
    }
    fn measuring(mut self, keys: &[&'static str]) -> Self {
        self.lengths = keys.to_vec();
        self
    }
    fn picking_at(mut self, distance: f32) -> Self {
        self.pick_distance = Some(distance);
        self
    }
    fn drawing(mut self) -> Self {
        self.draws = true;
        self
    }
    fn claiming(mut self, kinds: &[&'static str]) -> Self {
        self.kinds = kinds.to_vec();
        self
    }
    fn modal(mut self) -> Self {
        self.modal = true;
        self
    }
}

impl Workbench for FakeBench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        let d = WorkbenchDescriptor::new(self.id, self.id, "")
            .feature_kinds(self.kinds.iter().copied());
        if self.modal { d.modal() } else { d }
    }
    fn configure(&self, _context: &mut WorkbenchContext) {}
    fn passive_geometry(
        &self,
        _document: &Document,
        _id: FeatureId,
        _node: &FeatureNode,
    ) -> Option<PassiveGeometry> {
        self.draws.then(|| PassiveGeometry {
            mesh: kernel_api::TriMesh {
                positions: vec![[0.0, 0.0, 0.0]],
                ..Default::default()
            },
            revision: 1,
        })
    }
    fn pick_feature(
        &self,
        _document: &Document,
        _id: FeatureId,
        _node: &FeatureNode,
        _pick: &ViewportPick,
    ) -> Option<f32> {
        self.pick_distance
    }
    fn menu_items(&self, _scope: &MenuScope, _document: &Document) -> Vec<MenuItem> {
        self.items
            .iter()
            .map(|id| MenuItem::new(*id, *id))
            .collect()
    }
    fn on_command(
        &mut self,
        id: &str,
        _scope: &MenuScope,
        _ctx: &mut WorkbenchRuntimeContext,
    ) -> bool {
        self.items.contains(&id)
    }
    fn property_hints(&self) -> PropertyHints {
        PropertyHints {
            length_keys: self.lengths.clone(),
            reference_keys: Vec::new(),
        }
    }
}

/// A feature of a second kind, for a second bench.
struct Marker2;

impl WorkbenchFeature for Marker2 {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from("kind.marker2")
    }
    fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
    fn from_json(_value: &serde_json::Value) -> DocumentResult<Self> {
        Ok(Marker2)
    }
    fn dependencies(&self) -> Vec<FeatureId> {
        Vec::new()
    }
    fn name(&self) -> &str {
        "marker2"
    }
}

fn pick() -> ViewportPick {
    ViewportPick {
        view_proj: [[0.0; 4]; 4],
        viewport: (0, 0, 1, 1),
        cursor: (0.0, 0.0),
    }
}

/// A feature of a kind that is not a bench id.
struct Marker;

impl WorkbenchFeature for Marker {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from("kind.marker")
    }
    fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
    fn from_json(_value: &serde_json::Value) -> DocumentResult<Self> {
        Ok(Marker)
    }
    fn dependencies(&self) -> Vec<FeatureId> {
        Vec::new()
    }
    fn name(&self) -> &str {
        "marker"
    }
}

fn registry(benches: Vec<FakeBench>) -> DocumentService {
    let mut registry = DocumentService::default();
    for bench in benches {
        registry.register_workbench(Box::new(bench)).unwrap();
    }
    registry
}

#[test]
fn a_kind_claimed_twice_is_refused() {
    let mut registry = registry(vec![FakeBench::new("a").claiming(&["kind.x"])]);
    let err = registry
        .register_workbench(Box::new(FakeBench::new("b").claiming(&["kind.x"])))
        .unwrap_err();
    assert!(matches!(
        err,
        DocumentError::FeatureKindClaimed { ref kind, ref by } if kind == "kind.x" && by == "a"
    ));
    assert_eq!(registry.ids(), &[WorkbenchId::from("a")]);
}

#[test]
fn a_kind_resolves_to_the_bench_that_claimed_it() {
    let registry = registry(vec![
        FakeBench::new("a").claiming(&["a", "kind.marker"]),
        FakeBench::new("b").claiming(&["b"]),
    ]);
    assert_eq!(
        registry.owner_id_of(&WorkbenchId::from("kind.marker")),
        Some(&WorkbenchId::from("a"))
    );
    assert_eq!(
        registry.owner_id_of(&WorkbenchId::from("b")),
        Some(&WorkbenchId::from("b"))
    );
    assert_eq!(registry.owner_id_of(&WorkbenchId::from("nobody")), None);
}

#[test]
fn the_landing_bench_is_the_first_registered_that_is_not_modal() {
    let registry = registry(vec![
        FakeBench::new("edit").modal(),
        FakeBench::new("model"),
        FakeBench::new("other"),
    ]);
    assert_eq!(
        registry.landing_workbench(),
        Some(WorkbenchId::from("model"))
    );
    assert!(registry.is_modal(&WorkbenchId::from("edit")));
    assert!(!registry.is_modal(&WorkbenchId::from("model")));
    assert_eq!(
        registry.ids(),
        &[
            WorkbenchId::from("edit"),
            WorkbenchId::from("model"),
            WorkbenchId::from("other")
        ]
    );
}

#[test]
fn a_registry_of_only_edit_sessions_has_nowhere_to_land() {
    let registry = registry(vec![FakeBench::new("edit").modal()]);
    assert_eq!(registry.landing_workbench(), None);
}

#[test]
fn feature_info_comes_from_the_owner_and_is_absent_for_unclaimed_kinds() {
    let node = FeatureNode::new(FeatureId(Uuid::new_v4()), &Marker);
    let unclaimed = registry(vec![FakeBench::new("a")]);
    assert_eq!(unclaimed.feature_info(&node), None);

    let claimed = registry(vec![FakeBench::new("a").claiming(&["kind.marker"])]);
    let info = claimed.feature_info(&node).expect("the owner answers");
    // The fake bench keeps the trait's default presentation.
    assert_eq!(info.icon, "tree-feature");
    assert_eq!(info.family_label, "kind.marker feature");
    assert!(!info.builds_solid);
}

#[test]
fn the_nearest_feature_within_tolerance_wins_the_pick_and_hidden_ones_never_do() {
    let registry = registry(vec![
        FakeBench::new("a")
            .claiming(&["kind.marker"])
            .picking_at(6.0),
        FakeBench::new("b")
            .claiming(&["kind.marker2"])
            .picking_at(2.0),
    ]);
    let mut doc = Document::new("t");
    let far = doc.add_feature(Marker, "far".into()).unwrap();
    let near = doc.add_feature(Marker2, "near".into()).unwrap();

    assert_eq!(registry.pick_feature(&doc, &pick(), 8.0), Some(near));
    assert_eq!(registry.pick_feature(&doc, &pick(), 4.0), Some(near));
    assert_eq!(registry.pick_feature(&doc, &pick(), 1.0), None);

    doc.set_feature_visible(near, false);
    assert_eq!(registry.pick_feature(&doc, &pick(), 8.0), Some(far));
}

#[test]
fn passive_geometry_comes_from_owners_and_skips_the_hidden_and_the_edited() {
    let registry = registry(vec![
        FakeBench::new("a").claiming(&["kind.marker"]).drawing(),
        FakeBench::new("b").claiming(&["kind.marker2"]),
    ]);
    let mut doc = Document::new("t");
    let drawn = doc.add_feature(Marker, "drawn".into()).unwrap();
    let silent = doc.add_feature(Marker2, "silent".into()).unwrap();
    let edited = doc.add_feature(Marker, "edited".into()).unwrap();
    let hidden = doc.add_feature(Marker, "hidden".into()).unwrap();
    doc.set_feature_visible(hidden, false);

    let ids: Vec<FeatureId> = registry
        .passive_geometries(&doc, Some(edited))
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(ids, vec![drawn]);
    assert!(!ids.contains(&silent) && !ids.contains(&hidden) && !ids.contains(&edited));
}

#[test]
fn a_hook_outcome_keeps_every_request_in_the_hosts_order_and_the_active_object() {
    let mut doc = Document::new("t");
    let feature = doc.add_feature(Marker, "m".into()).unwrap();
    let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
    let orient = CameraOrientRequest {
        plane_origin: [0.0; 3],
        plane_normal: [0.0, 0.0, 1.0],
        plane_up: [0.0, 1.0, 0.0],
    };
    ctx.request(HostRequest::FinishEditing);
    ctx.request(HostRequest::OrientCamera(orient.clone()));
    ctx.request(HostRequest::SwitchWorkbench(WorkbenchId::from("b")));
    ctx.request(HostRequest::JournalLabel("x".into()));
    ctx.request(HostRequest::ActivateTool("t".into()));
    ctx.active_document_object = Some(feature);

    let outcome = HookOutcome::take(&mut ctx);
    assert_eq!(outcome.active_document_object, Some(feature));
    assert_eq!(
        outcome.requests,
        vec![
            HostRequest::ActivateTool("t".into()),
            HostRequest::JournalLabel("x".into()),
            HostRequest::SwitchWorkbench(WorkbenchId::from("b")),
            HostRequest::OrientCamera(orient),
            HostRequest::FinishEditing,
        ]
    );
    assert!(ctx.take_requests().is_empty(), "taken once");
}

#[test]
fn menu_entries_come_from_every_bench_in_registration_order_and_hints_merge() {
    let mut registry = registry(vec![
        FakeBench::new("b")
            .offering(&["b.one"])
            .measuring(&["length", "depth"]),
        FakeBench::new("a")
            .offering(&["a.one", "a.two"])
            .measuring(&["depth", "radius"]),
        FakeBench::new("quiet"),
    ]);
    let mut doc = Document::new("t");
    let entries: Vec<(String, String)> = registry
        .menu_items(&MenuScope::StartPage, &doc)
        .into_iter()
        .map(|(bench, item)| (bench.as_str().to_string(), item.id))
        .collect();
    assert_eq!(
        entries,
        vec![
            ("b".to_string(), "b.one".to_string()),
            ("a".to_string(), "a.one".to_string()),
            ("a".to_string(), "a.two".to_string()),
        ]
    );
    let hints = registry.property_hints();
    assert_eq!(hints.length_keys, vec!["length", "depth", "radius"]);

    let mut ctx = WorkbenchRuntimeContext::new(&mut doc, [0.0; 3], [0.0; 3], (0, 0, 1, 1));
    let a = registry.workbench_mut(&WorkbenchId::from("a")).unwrap();
    assert!(a.on_command("a.two", &MenuScope::StartPage, &mut ctx));
    assert!(!a.on_command("b.one", &MenuScope::StartPage, &mut ctx));
}

#[test]
fn a_bench_taken_out_leaves_its_kinds_free_for_the_next_to_claim() {
    let mut registry = registry(vec![
        FakeBench::new("a").claiming(&["a"]),
        FakeBench::new("pkg").claiming(&["pkg.thing"]),
    ]);
    let kind = WorkbenchId::from("pkg.thing");
    assert!(
        registry
            .unregister_workbench(&WorkbenchId::from("pkg"))
            .is_some()
    );
    assert_eq!(registry.ids(), &[WorkbenchId::from("a")]);
    assert_eq!(registry.owner_id_of(&kind), None, "its kinds are unowned");
    assert!(
        registry
            .unregister_workbench(&WorkbenchId::from("pkg"))
            .is_none()
    );
    // A new version registers in its place.
    registry
        .register_workbench(Box::new(FakeBench::new("pkg").claiming(&["pkg.thing"])))
        .expect("the id and the kinds are free again");
    assert_eq!(registry.owner_id_of(&kind), Some(&WorkbenchId::from("pkg")));
}
