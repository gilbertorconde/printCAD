//! Components: bodies grouped under a name, nested to any depth, the way
//! an assembly is made of sub-assemblies. A rigid component moves as one,
//! everything inside it held where it sits; a flexible one keeps the joints
//! between its bodies live inside the assembly around it.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{BodyId, Document, DocumentError, DocumentResult, op};

/// Unique identifier for a component in the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct ComponentId(pub Uuid);

impl ComponentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ComponentId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub id: ComponentId,
    pub name: String,
    /// The component this one sits in; `None` at the top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ComponentId>,
    /// The joints between its bodies stay live; a rigid component (the
    /// default) moves as one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flexible: bool,
}

impl Document {
    /// Every component, in the order they were made.
    pub fn components(&self) -> &[Component] {
        &self.components
    }

    pub fn component(&self, id: ComponentId) -> Option<&Component> {
        self.components.iter().find(|c| c.id == id)
    }

    /// The component `body` sits in directly, when it still exists.
    pub fn component_of(&self, body: BodyId) -> Option<ComponentId> {
        let id = self.body(body)?.component?;
        self.component(id).map(|c| c.id)
    }

    /// `id` and every component above it, innermost first.
    pub fn component_chain(&self, id: ComponentId) -> Vec<ComponentId> {
        let mut chain = Vec::new();
        let mut at = Some(id);
        while let Some(id) = at {
            if chain.contains(&id) {
                break;
            }
            let Some(component) = self.component(id) else {
                break;
            };
            chain.push(id);
            at = component.parent;
        }
        chain
    }

    /// Every body in `id` or in a component inside it, in body order.
    pub fn component_bodies(&self, id: ComponentId) -> Vec<BodyId> {
        self.bodies
            .iter()
            .filter(|b| {
                b.component
                    .is_some_and(|c| self.component_chain(c).contains(&id))
            })
            .map(|b| b.id)
            .collect()
    }

    /// The bodies that move as one with `body`: every body of the outermost
    /// rigid component it sits in, or `body` alone.
    pub fn rigid_unit_of(&self, body: BodyId) -> Vec<BodyId> {
        match self.rigid_root(body) {
            Some(root) => self.component_bodies(root),
            None => vec![body],
        }
    }

    /// The outermost rigid component `body` sits in.
    pub fn rigid_root(&self, body: BodyId) -> Option<ComponentId> {
        let inner = self.component_of(body)?;
        self.component_chain(inner)
            .into_iter()
            .rev()
            .find(|c| self.component(*c).is_some_and(|c| !c.flexible))
    }

    /// Every set of two bodies or more that moves as one, each in body
    /// order.
    pub fn rigid_units(&self) -> Vec<Vec<BodyId>> {
        let mut roots: Vec<ComponentId> = Vec::new();
        for body in &self.bodies {
            if let Some(root) = self.rigid_root(body.id)
                && !roots.contains(&root)
            {
                roots.push(root);
            }
        }
        roots
            .into_iter()
            .map(|r| self.component_bodies(r))
            .filter(|bodies| bodies.len() >= 2)
            .collect()
    }

    /// Move `body` to `placement`, and every body that moves as one with
    /// it (`rigid_unit_of`) by the same.
    pub fn place_with_unit(&mut self, body: BodyId, placement: crate::BodyPlacement) {
        let step = placement.after(&self.body_placement(body).inverse());
        for member in self.rigid_unit_of(body) {
            let moved = if member == body {
                placement
            } else {
                step.after(&self.body_placement(member))
            };
            self.set_body_placement(member, moved);
        }
    }

    /// A new component named `name` inside `parent` (at the top for
    /// `None`), rigid.
    pub fn create_component(
        &mut self,
        name: String,
        parent: Option<ComponentId>,
    ) -> DocumentResult<ComponentId> {
        if let Some(parent) = parent
            && self.component(parent).is_none()
        {
            return Err(DocumentError::NotFound(format!("component {}", parent.0)));
        }
        let id = ComponentId::new();
        self.record_and_apply(op::DocumentOp::SetComponent {
            id,
            component: Some(Component {
                id,
                name,
                parent,
                flexible: false,
            }),
        });
        Ok(id)
    }

    /// Change a component's name, place or rigidity. Putting it inside
    /// itself or one of its own is refused.
    pub fn update_component(&mut self, component: Component) -> DocumentResult<()> {
        let Some(now) = self.component(component.id) else {
            return Err(DocumentError::NotFound(format!(
                "component {}",
                component.id.0
            )));
        };
        if let Some(parent) = component.parent
            && (self.component(parent).is_none()
                || self.component_chain(parent).contains(&component.id))
        {
            return Err(DocumentError::Refused(
                "a component cannot sit inside itself".into(),
            ));
        }
        if *now != component {
            self.record_and_apply(op::DocumentOp::SetComponent {
                id: component.id,
                component: Some(component),
            });
        }
        Ok(())
    }

    /// Take a component apart: its bodies and components go to the one it
    /// sat in.
    pub fn remove_component(&mut self, id: ComponentId) -> DocumentResult<()> {
        let Some(component) = self.component(id).cloned() else {
            return Err(DocumentError::NotFound(format!("component {}", id.0)));
        };
        let bodies: Vec<BodyId> = self
            .bodies
            .iter()
            .filter(|b| b.component == Some(id))
            .map(|b| b.id)
            .collect();
        for body in bodies {
            self.set_body_component(body, component.parent)?;
        }
        let inner: Vec<Component> = self
            .components
            .iter()
            .filter(|c| c.parent == Some(id))
            .cloned()
            .collect();
        for mut child in inner {
            child.parent = component.parent;
            self.update_component(child)?;
        }
        self.record_and_apply(op::DocumentOp::SetComponent {
            id,
            component: None,
        });
        Ok(())
    }

    /// Put `body` in `component`, or at the top for `None`.
    pub fn set_body_component(
        &mut self,
        body: BodyId,
        component: Option<ComponentId>,
    ) -> DocumentResult<()> {
        let Some(entry) = self.body(body) else {
            return Err(DocumentError::NotFound(format!("body {}", body.0)));
        };
        if let Some(c) = component
            && self.component(c).is_none()
        {
            return Err(DocumentError::NotFound(format!("component {}", c.0)));
        }
        if entry.component != component {
            self.record_and_apply(op::DocumentOp::SetBodyComponent {
                id: body,
                component,
            });
        }
        Ok(())
    }

    pub(crate) fn apply_set_component(&mut self, id: ComponentId, component: Option<&Component>) {
        match (self.components.iter_mut().find(|c| c.id == id), component) {
            (Some(entry), Some(component)) => *entry = component.clone(),
            (None, Some(component)) => self.components.push(component.clone()),
            (Some(_), None) => self.components.retain(|c| c.id != id),
            (None, None) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_components_move_as_their_outermost_rigid_one() {
        let mut doc = Document::new("t");
        let (a, b, c) = (
            doc.create_body(Some("A".into())),
            doc.create_body(Some("B".into())),
            doc.create_body(Some("C".into())),
        );
        let outer = doc.create_component("Outer".into(), None).unwrap();
        let inner = doc.create_component("Inner".into(), Some(outer)).unwrap();
        doc.set_body_component(a, Some(outer)).unwrap();
        doc.set_body_component(b, Some(inner)).unwrap();
        doc.set_body_component(c, Some(inner)).unwrap();
        assert_eq!(doc.component_bodies(outer), vec![a, b, c]);
        assert_eq!(doc.rigid_unit_of(b), vec![a, b, c]);
        // The outer one flexible: the inner one still moves as one.
        let mut flexible = doc.component(outer).unwrap().clone();
        flexible.flexible = true;
        doc.update_component(flexible).unwrap();
        assert_eq!(doc.rigid_unit_of(a), vec![a]);
        assert_eq!(doc.rigid_units(), vec![vec![b, c]]);
        // Never inside itself.
        let mut looped = doc.component(outer).unwrap().clone();
        looped.parent = Some(inner);
        assert!(doc.update_component(looped).is_err());
        // Taken apart, its bodies and components go up a level.
        doc.remove_component(outer).unwrap();
        assert_eq!(doc.component_of(a), None);
        assert_eq!(doc.component(inner).unwrap().parent, None);
        assert_eq!(doc.rigid_units(), vec![vec![b, c]]);
    }
}
