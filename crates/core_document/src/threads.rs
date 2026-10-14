//! Cosmetic threads: threads a feature declares on a bore without cutting
//! them, each with its designation and where the bore opens. The benches
//! answer them (`Workbench::cosmetic_threads`) and the registry keeps them
//! on the document as derived state each time it evaluates, so the parts
//! list, the annotations layer and export read them without knowing which
//! bench made them.

use crate::{BodyId, FeatureId};

/// One threaded bore, in its body's own frame.
#[derive(Debug, Clone, PartialEq)]
pub struct ThreadCallout {
    /// The feature that threads it.
    pub feature: FeatureId,
    pub body: BodyId,
    /// What the thread is called ("M6", "1/4-20 2B LH").
    pub designation: String,
    /// The centre of the bore's mouth.
    pub mouth: [f64; 3],
    /// Down the bore, into the material, of unit length.
    pub axis: [f64; 3],
    /// The bore's radius, mm.
    pub radius: f64,
}

impl ThreadCallout {
    /// What a callout of it reads.
    pub fn label(&self) -> String {
        format!("{} thread", self.designation)
    }

    /// Its callout's leader, in the body's frame: from a point of the
    /// bore's rim out of the material and away from the bore, to where the
    /// label stands.
    pub fn leader(&self) -> ([f32; 3], [f32; 3]) {
        let axis = glam::DVec3::from_array(self.axis).normalize_or(glam::DVec3::Z);
        let side = axis.any_orthonormal_vector();
        let rim = glam::DVec3::from_array(self.mouth) + side * self.radius;
        let reach = self.radius * 2.0 + 4.0;
        let label = rim + (side - axis) * reach;
        (rim.as_vec3().to_array(), label.as_vec3().to_array())
    }

    /// A point on the bore's wall a little inside its mouth, below where
    /// the leader starts: the face the thread is cut in.
    pub fn on_wall(&self) -> [f64; 3] {
        let axis = glam::DVec3::from_array(self.axis).normalize_or(glam::DVec3::Z);
        let side = axis.any_orthonormal_vector();
        let mouth = glam::DVec3::from_array(self.mouth);
        (mouth + side * self.radius + axis * (self.radius * 0.25)).to_array()
    }
}

/// `threads` counted by designation, in the order each first appears:
/// what a part's line of the parts list reads ("2 × M6 thread").
pub fn count_by_designation<'a>(
    threads: impl IntoIterator<Item = &'a ThreadCallout>,
) -> Vec<(String, u32)> {
    let mut counted: Vec<(String, u32)> = Vec::new();
    for thread in threads {
        match counted.iter_mut().find(|(d, _)| *d == thread.designation) {
            Some((_, n)) => *n += 1,
            None => counted.push((thread.designation.clone(), 1)),
        }
    }
    counted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(designation: &str) -> ThreadCallout {
        ThreadCallout {
            feature: FeatureId::new(),
            body: BodyId::new(),
            designation: designation.into(),
            mouth: [10.0, 0.0, 5.0],
            axis: [0.0, 0.0, -1.0],
            radius: 2.5,
        }
    }

    #[test]
    fn threads_count_by_designation_in_the_order_they_come() {
        let threads = [thread("M6"), thread("M4"), thread("M6")];
        assert_eq!(
            count_by_designation(&threads),
            vec![("M6".to_string(), 2), ("M4".to_string(), 1)]
        );
    }

    /// The leader starts on the rim at the mouth and leaves the material.
    #[test]
    fn a_leader_starts_on_the_rim_and_runs_out_of_the_bore() {
        let t = thread("M6");
        let (at, label) = t.leader();
        let at = glam::DVec3::from_array(at.map(f64::from));
        let label = glam::DVec3::from_array(label.map(f64::from));
        let mouth = glam::DVec3::from_array(t.mouth);
        assert!(((at - mouth).length() - t.radius).abs() < 1e-5);
        assert!((at.z - mouth.z).abs() < 1e-5, "on the mouth's plane");
        assert!(label.z > at.z, "out of the material");
        let off_axis = |p: glam::DVec3| (p - mouth).truncate().length();
        assert!(off_axis(label) > off_axis(at), "away from the bore");
        let wall = glam::DVec3::from_array(t.on_wall());
        assert!((off_axis(wall) - t.radius).abs() < 1e-9, "on the wall");
        assert!(wall.z < mouth.z, "inside the bore");
    }
}
