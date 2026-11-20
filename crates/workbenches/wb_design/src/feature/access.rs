//! What a feature holds, read and changed across the variants: its
//! sketches, originals, fuse settings, placing faces and kind.

use super::*;

impl DesignFeature {
    /// The sketch this feature consumes, when it is sketch-based.
    pub fn sketch(&self) -> Option<FeatureId> {
        match self {
            DesignFeature::Pad { sketch, .. } | DesignFeature::Pocket { sketch, .. } => *sketch,
            DesignFeature::Revolution { sketch, .. }
            | DesignFeature::Groove { sketch, .. }
            | DesignFeature::Helix { sketch, .. }
            | DesignFeature::Hole { sketch, .. }
            | DesignFeature::Project { sketch, .. }
            | DesignFeature::Rib { sketch, .. }
            | DesignFeature::Text { sketch, .. } => Some(*sketch),
            DesignFeature::Pipe { profile, .. } => Some(*profile),
            _ => None,
        }
    }

    /// The sketch regions a pad, pocket, revolution or groove takes by
    /// name; empty for every other feature, and for one taking its whole
    /// profile.
    pub fn regions(&self) -> &[String] {
        match self {
            DesignFeature::Pad { regions, .. }
            | DesignFeature::Pocket { regions, .. }
            | DesignFeature::Revolution { regions, .. }
            | DesignFeature::Groove { regions, .. } => regions,
            _ => &[],
        }
    }

    /// Every sketch referenced by this feature.
    pub fn sketches(&self) -> Vec<FeatureId> {
        match self {
            DesignFeature::Loft { sections, .. } => {
                sections.iter().filter_map(LoftSection::feature).collect()
            }
            DesignFeature::Pipe {
                profile,
                spine,
                orientation,
                sections,
                ..
            } => {
                let mut all = vec![*profile, *spine];
                all.extend(orientation.path());
                all.extend(sections.iter().copied());
                all
            }
            _ => self.sketch().into_iter().collect(),
        }
    }

    /// Earlier features this feature re-applies (patterns/mirror).
    pub fn originals(&self) -> Option<&[FeatureId]> {
        match self {
            DesignFeature::Mirrored { originals, .. }
            | DesignFeature::LinearPattern { originals, .. }
            | DesignFeature::PolarPattern { originals, .. }
            | DesignFeature::CircularPattern { originals, .. }
            | DesignFeature::PointPattern { originals, .. }
            | DesignFeature::PathPattern { originals, .. }
            | DesignFeature::FillPattern { originals, .. }
            | DesignFeature::MultiTransform { originals, .. } => Some(originals),
            _ => None,
        }
    }

    /// The occurrences a pattern or mirror leaves out (0 the original).
    pub fn skipped(&self) -> Option<&[u32]> {
        match self {
            DesignFeature::Mirrored { skipped, .. }
            | DesignFeature::LinearPattern { skipped, .. }
            | DesignFeature::PolarPattern { skipped, .. }
            | DesignFeature::CircularPattern { skipped, .. }
            | DesignFeature::PointPattern { skipped, .. }
            | DesignFeature::PathPattern { skipped, .. }
            | DesignFeature::FillPattern { skipped, .. }
            | DesignFeature::MultiTransform { skipped, .. } => Some(skipped),
            _ => None,
        }
    }

    /// Leave occurrence `index` out, or bring it back when it was: whether
    /// the feature is one with occurrences.
    pub fn toggle_skipped(&mut self, index: u32) -> bool {
        let skipped = match self {
            DesignFeature::Mirrored { skipped, .. }
            | DesignFeature::LinearPattern { skipped, .. }
            | DesignFeature::PolarPattern { skipped, .. }
            | DesignFeature::CircularPattern { skipped, .. }
            | DesignFeature::PointPattern { skipped, .. }
            | DesignFeature::PathPattern { skipped, .. }
            | DesignFeature::FillPattern { skipped, .. }
            | DesignFeature::MultiTransform { skipped, .. } => skipped,
            _ => return false,
        };
        match skipped.iter().position(|&i| i == index) {
            Some(at) => {
                skipped.remove(at);
            }
            None => {
                skipped.push(index);
                skipped.sort_unstable();
            }
        }
        true
    }

    /// The refine switch and boolean tolerance of a feature that fuses or
    /// cuts; `None` for one that does neither.
    pub(super) fn fuse_settings(&self) -> Option<(bool, Option<f32>)> {
        fuse_settings!(self, |refine, boolean_tolerance| (
            *refine,
            *boolean_tolerance
        ))
    }

    /// [`Self::fuse_settings`], to change.
    fn fuse_settings_mut(&mut self) -> Option<(&mut bool, &mut Option<f32>)> {
        fuse_settings!(self, |refine, boolean_tolerance| (
            refine,
            boolean_tolerance
        ))
    }

    /// Whether the feature merges the coplanar faces its fuse or cut
    /// leaves behind.
    pub fn refine(&self) -> bool {
        match self {
            DesignFeature::Fillet { refine, .. } | DesignFeature::Chamfer { refine, .. } => *refine,
            _ => self.fuse_settings().is_some_and(|(refine, _)| refine),
        }
    }

    /// Set whether the feature refines its result. A feature that neither
    /// fuses, cuts, rounds nor bevels has nothing to refine and is left as
    /// it is.
    pub fn set_refine(&mut self, on: bool) {
        if let DesignFeature::Fillet { refine, .. } | DesignFeature::Chamfer { refine, .. } = self {
            *refine = on;
        } else if let Some((refine, _)) = self.fuse_settings_mut() {
            *refine = on;
        }
    }

    /// The tolerance the feature's fuse or cut runs at, when the user
    /// stated one: a positive length, zero and `None` meaning none.
    pub fn boolean_tolerance(&self) -> Option<f32> {
        self.fuse_settings()?
            .1
            .filter(|t| t.is_finite() && *t > 0.0)
    }

    /// State the tolerance the feature's fuse or cut runs at, or leave it
    /// to the kernel with `None`. A feature with no fuse or cut is left as
    /// it is.
    pub fn set_boolean_tolerance(&mut self, tolerance: Option<f32>) {
        if let Some((_, boolean_tolerance)) = self.fuse_settings_mut() {
            *boolean_tolerance = tolerance;
        }
    }

    /// Whether the feature has a refine switch at all.
    pub fn can_refine(&self) -> bool {
        self.has_boolean()
            || matches!(
                self,
                DesignFeature::Fillet { .. } | DesignFeature::Chamfer { .. }
            )
    }

    /// Whether the feature fuses or cuts, and so has a boolean tolerance.
    pub fn has_boolean(&self) -> bool {
        self.fuse_settings().is_some()
    }

    /// A cutting feature that keeps what it shares with the body (the
    /// common) rather than cutting it away.
    pub fn keeps_inside(&self) -> bool {
        match self {
            DesignFeature::Pocket { keep_inside, .. }
            | DesignFeature::Groove { keep_inside, .. }
            | DesignFeature::Loft { keep_inside, .. }
            | DesignFeature::Pipe { keep_inside, .. }
            | DesignFeature::Helix { keep_inside, .. }
            | DesignFeature::Primitive { keep_inside, .. } => *keep_inside && self.is_subtractive(),
            _ => false,
        }
    }

    /// The keep-inside switch of a cutting feature that offers it in
    /// the task with the rest: Pocket, Groove, and a cutting Loft, Pipe or
    /// Primitive.
    pub fn keep_inside_mut(&mut self) -> Option<&mut bool> {
        let cutting = self.is_subtractive();
        match self {
            DesignFeature::Pocket { keep_inside, .. }
            | DesignFeature::Groove { keep_inside, .. } => Some(keep_inside),
            DesignFeature::Loft { keep_inside, .. }
            | DesignFeature::Pipe { keep_inside, .. }
            | DesignFeature::Primitive { keep_inside, .. }
                if cutting =>
            {
                Some(keep_inside)
            }
            _ => None,
        }
    }

    /// A cosmetic thread's designation ("M6", "1/4-20 2B LH"), for a
    /// threaded hole or a thread that shows its thread rather than
    /// cutting it.
    pub fn cosmetic_thread(&self) -> Option<String> {
        if let DesignFeature::Thread { cosmetic: true, .. } = self {
            return crate::build::thread_designation(self);
        }
        let DesignFeature::Hole {
            thread: Some(spec),
            threaded: true,
            modeled_thread: false,
            cosmetic_thread: true,
            ..
        } = self
        else {
            return None;
        };
        let mut name = spec.size.clone();
        if !spec.class.is_empty() {
            name = format!("{name} {}", spec.class);
        }
        if spec.left_handed {
            name.push_str(" LH");
        }
        Some(name)
    }

    /// The face of the solid the feature starts on, when it starts on one.
    pub fn start_face(&self) -> Option<&FacePick> {
        match self {
            DesignFeature::Pad { extras, .. } | DesignFeature::Pocket { extras, .. } => {
                extras.start_face.as_ref()
            }
            DesignFeature::Hole { start, .. } => start.face.as_ref(),
            _ => None,
        }
    }

    /// The faces of the solid before the feature that place it, which a
    /// build finds again by name (else by where they were picked) and the
    /// feature follows: a start face, a mirror's or split's picked plane, a
    /// draft's neutral face, the faces a projected sketch or text goes on.
    pub fn placing_faces(&self) -> Vec<&FacePick> {
        fn mirror(plane: &MirrorPlane) -> Option<&FacePick> {
            match plane {
                MirrorPlane::Face(face) => Some(face),
                _ => None,
            }
        }
        match self {
            DesignFeature::Mirrored { plane, .. }
            | DesignFeature::Split {
                plane,
                sketch: None,
                ..
            } => mirror(plane).into_iter().collect(),
            DesignFeature::MultiTransform { steps, .. } => steps
                .iter()
                .filter_map(|step| match step {
                    TransformStep::Mirror { plane } => mirror(plane),
                    _ => None,
                })
                .collect(),
            DesignFeature::Draft {
                neutral,
                neutral_plane: None,
                ..
            } => vec![neutral],
            DesignFeature::Project { faces, .. } | DesignFeature::Text { faces, .. } => {
                faces.iter().collect()
            }
            DesignFeature::Thread { face, .. }
            | DesignFeature::FillPattern {
                region: FillArea::Face(face),
                ..
            } => vec![face],
            other => other.start_face().into_iter().collect(),
        }
    }

    /// The cylinder face text spread round a whole turn goes round, the
    /// first of its placing faces, whose radius the build's answer gives.
    pub fn turn_face(&self) -> Option<&FacePick> {
        match self {
            DesignFeature::Text {
                faces,
                wrap: true,
                along: false,
                full_turn: true,
                ..
            } => faces.first(),
            _ => None,
        }
    }

    /// [`Self::placing_faces`], to move.
    pub fn placing_faces_mut(&mut self) -> Vec<&mut FacePick> {
        fn mirror(plane: &mut MirrorPlane) -> Option<&mut FacePick> {
            match plane {
                MirrorPlane::Face(face) => Some(face),
                _ => None,
            }
        }
        match self {
            DesignFeature::Mirrored { plane, .. }
            | DesignFeature::Split {
                plane,
                sketch: None,
                ..
            } => mirror(plane).into_iter().collect(),
            DesignFeature::MultiTransform { steps, .. } => steps
                .iter_mut()
                .filter_map(|step| match step {
                    TransformStep::Mirror { plane } => mirror(plane),
                    _ => None,
                })
                .collect(),
            DesignFeature::Draft {
                neutral,
                neutral_plane: None,
                ..
            } => vec![neutral],
            DesignFeature::Project { faces, .. } | DesignFeature::Text { faces, .. } => {
                faces.iter_mut().collect()
            }
            DesignFeature::Pad { extras, .. } | DesignFeature::Pocket { extras, .. } => {
                extras.start_face.as_mut().into_iter().collect()
            }
            DesignFeature::Hole { start, .. } => start.face.as_mut().into_iter().collect(),
            DesignFeature::Thread { face, .. }
            | DesignFeature::FillPattern {
                region: FillArea::Face(face),
                ..
            } => vec![face],
            _ => Vec::new(),
        }
    }

    /// True when this feature removes material (must not be a body's first).
    pub fn is_subtractive(&self) -> bool {
        match self {
            DesignFeature::Pocket { .. }
            | DesignFeature::Groove { .. }
            | DesignFeature::Hole { .. }
            | DesignFeature::Thread { .. } => true,
            DesignFeature::Loft { subtractive, .. }
            | DesignFeature::Pipe { subtractive, .. }
            | DesignFeature::Helix { subtractive, .. }
            | DesignFeature::Primitive { subtractive, .. } => *subtractive,
            _ => false,
        }
    }

    /// True for features that modify the running solid instead of sweeping a
    /// new tool (dress-ups, patterns, booleans). These need existing material.
    pub fn is_modifier(&self) -> bool {
        matches!(
            self,
            DesignFeature::Fillet { .. }
                | DesignFeature::Chamfer { .. }
                | DesignFeature::Draft { .. }
                | DesignFeature::Thickness { .. }
                | DesignFeature::DeleteFaces { .. }
                | DesignFeature::OffsetFaces { .. }
                | DesignFeature::OffsetSolid { .. }
                | DesignFeature::Scale { .. }
                | DesignFeature::MoveFaces { .. }
                | DesignFeature::Project { .. }
                | DesignFeature::Text { .. }
                | DesignFeature::Mirrored { .. }
                | DesignFeature::LinearPattern { .. }
                | DesignFeature::PolarPattern { .. }
                | DesignFeature::CircularPattern { .. }
                | DesignFeature::PointPattern { .. }
                | DesignFeature::PathPattern { .. }
                | DesignFeature::FillPattern { .. }
                | DesignFeature::MultiTransform { .. }
                | DesignFeature::BodyBoolean { .. }
                | DesignFeature::Split { .. }
                | DesignFeature::Piece { .. }
        )
    }

    /// The numbers the feature holds that change nothing as it stands,
    /// which its parameters leave out: a honeycomb's row spacing, the
    /// pitch and depth of a thread cut to a standard size, the diameter of
    /// a hole sized from its thread.
    pub fn idle_numbers(&self) -> &'static [&'static str] {
        match self {
            DesignFeature::FillPattern {
                lattice: FillKind::Honeycomb,
                ..
            } => &["row_spacing"],
            DesignFeature::Thread {
                thread: Some(_), ..
            } => &["pitch", "depth"],
            DesignFeature::Hole {
                thread: Some(_), ..
            } => &["diameter"],
            _ => &[],
        }
    }

    /// The split this feature is a piece of, when it is one.
    pub fn split_group(&self) -> Option<u64> {
        match self {
            DesignFeature::Split { split, .. } => Some(*split),
            _ => None,
        }
    }
}
