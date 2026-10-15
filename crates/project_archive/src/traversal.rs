//! Bounded source-object walks. Application conversion remains with the consumer.

use std::collections::BTreeMap;

use crate::archive::FcstdError;
use crate::document_xml::{FcDocument, Placement};

/// Maximum pending objects, including the starting object. Cached dependencies
/// do not consume pending depth; consumers keep their own resolution cache.
pub const MAX_TRAVERSAL_DEPTH: usize = 128;

pub struct TraversalPath {
    kind: &'static str,
    names: Vec<String>,
}

impl TraversalPath {
    pub fn new(kind: &'static str) -> Self {
        Self {
            kind,
            names: Vec::new(),
        }
    }

    pub fn current(&self) -> Option<&str> {
        self.names.last().map(String::as_str)
    }

    /// A pending target would close a cycle, rather than consume more depth.
    pub fn is_pending(&self, name: &str) -> bool {
        self.names.iter().any(|active| active == name)
    }

    /// Complete the current object after its required dependencies are resolved.
    pub fn leave(&mut self) {
        self.names.pop().expect("a pending object must exist");
    }

    pub fn enter(&mut self, name: &str) -> Result<(), FcstdError> {
        let problem = if self.is_pending(name) {
            Some(format!("{} cycle", self.kind))
        } else if self.names.len() == MAX_TRAVERSAL_DEPTH {
            Some(format!("{} depth exceeds {MAX_TRAVERSAL_DEPTH}", self.kind))
        } else {
            None
        };
        if let Some(problem) = problem {
            return Err(self.error(name, &problem, true));
        }
        self.names.push(name.to_owned());
        Ok(())
    }

    pub fn missing_target(&self, name: &str) -> FcstdError {
        self.error(name, "linked object missing", false)
    }

    fn error(&self, name: &str, reason: &str, append_target: bool) -> FcstdError {
        let mut chain = self.names.join(" -> ");
        if append_target {
            if !chain.is_empty() {
                chain.push_str(" -> ");
            }
            chain.push_str(name);
        }
        FcstdError::Format {
            object: name.into(),
            reason: format!("{reason}: {chain}"),
        }
    }
}

/// Child-to-group ownership for groups with their own coordinate system.
pub struct ParentMap {
    parents: BTreeMap<String, String>,
}

impl ParentMap {
    pub fn build(source: &FcDocument) -> Result<Self, FcstdError> {
        let mut parents = BTreeMap::new();
        for group in &source.objects {
            if !matches!(group.type_name.as_str(), "PartDesign::Body" | "App::Part")
                || !group.has("Group")
            {
                continue;
            }
            for child in group.link_list("Group")? {
                if let Some(previous) = parents.insert(child.to_owned(), group.name.clone()) {
                    if previous != group.name {
                        return Err(
                            group.format_error(format!("{child} has multiple parent groups"))
                        );
                    }
                }
            }
        }
        Ok(Self { parents })
    }

    /// Parent frame only; the object's own placement is composed by the caller.
    pub fn placement(&self, source: &FcDocument, name: &str) -> Result<Placement, FcstdError> {
        let mut path = TraversalPath::new("placement");
        let mut placements = Vec::new();
        let mut current = name;
        loop {
            path.enter(current)?;
            let Some(parent) = self.parents.get(current) else {
                break;
            };
            let group = source
                .object(parent)
                .ok_or_else(|| path.missing_target(parent))?;
            placements.push(group.placement("Placement")?);
            current = parent;
        }
        placements
            .into_iter()
            .rev()
            .try_fold(Placement::IDENTITY, |parent, local| {
                parent.compose(local, name)
            })
    }
}

pub fn parent_placement(source: &FcDocument, name: &str) -> Result<Placement, FcstdError> {
    ParentMap::build(source)?.placement(source, name)
}
