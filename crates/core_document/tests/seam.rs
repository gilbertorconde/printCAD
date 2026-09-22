//! The registry side of the workbench seam: what the host learns about a
//! bench from its descriptor alone.

use core_document::{
    DocumentError, DocumentResult, DocumentService, FeatureId, FeatureNode, Workbench,
    WorkbenchContext, WorkbenchDescriptor, WorkbenchFeature, WorkbenchId,
};
use uuid::Uuid;

/// A bench that declares itself and nothing else.
struct FakeBench {
    id: &'static str,
    kinds: Vec<&'static str>,
    modal: bool,
}

impl FakeBench {
    fn new(id: &'static str) -> Self {
        Self {
            id,
            kinds: Vec::new(),
            modal: false,
        }
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
