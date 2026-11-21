//! How one solid object of the source becomes Design features: primitives,
//! extrusions, booleans, dress-ups, holes, patterns and lofts, with the
//! sketches, axes, planes and picks their settings name.

use super::*;

impl<'a> Importer<'a> {
    /// A solid object as features, or why it cannot be.
    pub(super) fn solid(
        &mut self,
        object: &FcObject,
    ) -> Result<Result<ImportedObject, Lost>, ResolveError> {
        let type_name = object.type_name.as_str();
        let lost = |reason: String| Ok(Err(Lost::Setting(reason)));
        let cut =
            type_name == "PartDesign::Pocket" || type_name.starts_with("PartDesign::Subtractive");
        let is_boolean = matches!(type_name, "Part::Fuse" | "Part::Cut" | "Part::Common");
        let supported = type_name == "PartDesign::Pad"
            || type_name == "PartDesign::Pocket"
            || type_name.starts_with("PartDesign::Additive")
            || type_name.starts_with("PartDesign::Subtractive")
            || is_boolean
            || matches!(
                type_name,
                "PartDesign::Fillet"
                    | "PartDesign::Chamfer"
                    | "PartDesign::Hole"
                    | "PartDesign::Mirrored"
                    | "PartDesign::LinearPattern"
                    | "PartDesign::PolarPattern"
                    | "PartDesign::MultiTransform"
                    | "PartDesign::FeatureBase"
                    | "PartDesign::Boolean"
                    | "Part::Box"
                    | "Part::Cylinder"
                    | "Part::Cone"
                    | "Part::Sphere"
                    | "Part::Torus"
                    | "Part::Extrusion"
            );
        if !supported {
            return lost(format!("unsupported {type_name}"));
        }
        if type_name == "Part::Extrusion" {
            return self.extrusion_body(object);
        }
        if is_boolean && !same_placement(object.placement("Placement")?, Placement::IDENTITY) {
            return lost("placement of the boolean result".into());
        }
        let base = if is_boolean {
            object.link("Base")?
        } else if object.has("BaseFeature") {
            object.link("BaseFeature")?
        } else {
            None
        };
        // Each input is resolved before anything is made.
        if let Some(base) = base {
            self.resolved(base)?;
        }
        if base.is_none()
            && (cut
                || is_boolean
                || matches!(
                    type_name,
                    "PartDesign::Fillet"
                        | "PartDesign::Chamfer"
                        | "PartDesign::Hole"
                        | "PartDesign::Mirrored"
                        | "PartDesign::LinearPattern"
                        | "PartDesign::PolarPattern"
                        | "PartDesign::MultiTransform"
                        | "PartDesign::FeatureBase"
                        | "PartDesign::Boolean"
                ))
        {
            return lost("missing base solid".into());
        }
        let feature = if is_boolean {
            let Some(tool) = object.link("Tool")? else {
                return lost("missing boolean tool".into());
            };
            let tool = match self.solid_of(tool, &object.name)? {
                Ok(body) => body,
                Err(lost) => return Ok(Err(lost)),
            };
            DesignFeature::BodyBoolean {
                tool_body: tool,
                kind: match type_name {
                    "Part::Fuse" => kernel_api::BoolKind::Fuse,
                    "Part::Cut" => kernel_api::BoolKind::Cut,
                    _ => kernel_api::BoolKind::Common,
                },
                more_tools: Vec::new(),
                refine: optional_bool(object, "Refine", false)?,
                boolean_tolerance: None,
            }
        } else if type_name == "PartDesign::FeatureBase" {
            // A body started from another solid takes a clone of it, which
            // `place` makes when the base is not this body's own.
            return self.based_on(object, base.expect("checked above"));
        } else if let Some(made) = self.design_feature(object, cut)? {
            match made {
                Ok(feature) => feature,
                Err(lost) => return Ok(Err(lost)),
            }
        } else {
            match self.primitive(object, cut)? {
                Ok(feature) => feature,
                Err(lost) => return Ok(Err(lost)),
            }
        };
        self.place(object, feature, base)
    }

    /// A source body started from another solid: its body begins with a
    /// clone of that solid's body.
    fn based_on(
        &mut self,
        object: &FcObject,
        base: &str,
    ) -> Result<Result<ImportedObject, Lost>, ResolveError> {
        let source = match self.solid_of(base, &object.name)? {
            Ok(source) => source,
            Err(lost) => return Ok(Err(lost)),
        };
        let Some(owner) = self.source_body(&object.name).cloned() else {
            return Ok(Err(Lost::Setting("a base outside a body".into())));
        };
        let body = self.segment(&owner)?;
        if self.last_solid.contains_key(&body) {
            return Ok(Err(Lost::Setting("a base after the body's start".into())));
        }
        self.add_solid_step(object, DesignFeature::Clone { source }, body)
    }

    fn primitive(&mut self, object: &FcObject, cut: bool) -> Result<Made, ResolveError> {
        let lost = |reason: String| Ok(Err(Lost::Setting(reason)));
        for skew in ["FirstAngle", "SecondAngle"] {
            if object.has(skew) && object.float(skew)? != 0.0 {
                return lost(format!("unsupported {skew}"));
            }
        }
        let kind = object
            .type_name
            .rsplit(':')
            .next()
            .expect("type has a name")
            .trim_start_matches("Additive")
            .trim_start_matches("Subtractive");
        let f = |p| object.float(p);
        let primitive = match kind {
            "Box" => kernel_api::PrimitiveKind::Box {
                length: f("Length")?,
                width: f("Width")?,
                height: f("Height")?,
            },
            "Cylinder" => kernel_api::PrimitiveKind::Cylinder {
                radius: f("Radius")?,
                height: f("Height")?,
                angle_deg: f("Angle")?,
            },
            "Cone" => kernel_api::PrimitiveKind::Cone {
                radius1: f("Radius1")?,
                radius2: f("Radius2")?,
                height: f("Height")?,
                angle_deg: f("Angle")?,
            },
            "Sphere" => kernel_api::PrimitiveKind::Sphere {
                radius: f("Radius")?,
                angle1_deg: f("Angle1")?,
                angle2_deg: f("Angle2")?,
                angle3_deg: f("Angle3")?,
            },
            "Torus" => kernel_api::PrimitiveKind::Torus {
                radius1: f("Radius1")?,
                radius2: f("Radius2")?,
                angle1_deg: f("Angle1")?,
                angle2_deg: f("Angle2")?,
                angle3_deg: f("Angle3")?,
            },
            _ => return lost(format!("unsupported primitive {kind}")),
        };
        let pose = self
            .parents
            .placement(self.source, &object.name)?
            .compose(object.placement("Placement")?, &object.name)?;
        let [x_axis, _, z_axis] = frame(&object.name, pose)?;
        Ok(Ok(DesignFeature::Primitive {
            kind: primitive,
            placement: kernel_api::Placement {
                origin: pose.position.to_array(),
                x_axis,
                z_axis,
            },
            subtractive: cut,
            keep_inside: false,
            attached: None,
            refine: optional_bool(object, "Refine", false)?,
            boolean_tolerance: None,
        }))
    }

    /// An extrusion of a sketch or a text: a body of its own holding the
    /// sketch (a text's made here) and a pad of it.
    fn extrusion_body(
        &mut self,
        object: &FcObject,
    ) -> Result<Result<ImportedObject, Lost>, ResolveError> {
        let lost = |reason: &str| Ok(Err(Lost::Setting(reason.into())));
        let Some(profile) = object.link("Base")? else {
            return lost("an extrusion of nothing");
        };
        let Some(shape) = self.source.object(profile) else {
            return Ok(Err(Lost::Dependency(profile.into())));
        };
        let text = is_text(shape);
        if !text && shape.type_name != "Sketcher::SketchObject" {
            return lost("an extrusion of other than a sketch or a text");
        }
        if !same_placement(object.placement("Placement")?, Placement::IDENTITY) {
            return lost("placement of the extrusion's result");
        }
        for taper in ["TaperAngle", "TaperAngleRev"] {
            if object.has(taper) && object.float(taper)? != 0.0 {
                return lost("a tapered extrusion");
            }
        }
        if !text && !optional_bool(object, "Solid", false)? {
            return lost("an extrusion of outlines into sheets");
        }
        // Along the profile's normal, or a direction square to it.
        let placement = self
            .parents
            .placement(self.source, profile)?
            .compose(shape.placement("Placement")?, profile)?;
        let normal = placement.rotation.transform_vector([0.0, 0.0, 1.0]);
        let mode = if object.has("DirMode") {
            object.integer("DirMode")?
        } else {
            0
        };
        let direction = vector(object, "Dir")?;
        let along = glam::DVec3::from_array(direction).dot(glam::DVec3::from_array(normal));
        let flip = match mode {
            2 => false,
            0 if (along.abs() - glam::DVec3::from_array(direction).length()).abs() < 1e-9
                && along != 0.0 =>
            {
                along < 0.0
            }
            _ => return lost("an extrusion along other than its profile's normal"),
        } ^ optional_bool(object, "Reversed", false)?;
        let forward = object.float("LengthFwd")?;
        let back = if object.has("LengthRev") {
            object.float("LengthRev")?
        } else {
            0.0
        };
        let symmetric = optional_bool(object, "Symmetric", false)?;
        let (forward, back) = match (forward, back) {
            (f, b) if f == 0.0 && b == 0.0 && mode == 0 => (along.abs(), 0.0),
            (f, b) if f >= 0.0 && b >= 0.0 && f + b > 0.0 => (f, b),
            _ => return lost("an extrusion with no length"),
        };
        let name = object.name.as_str();
        let mut data = serde_json::json!({ "symmetric": symmetric, "refine": false });
        if forward > 0.0 && (back == 0.0 || symmetric) {
            data["mode"] = "Dimension".into();
            data["length"] = design_length(name, "LengthFwd", forward)?.into();
            data["reversed"] = flip.into();
        } else if forward == 0.0 {
            data["mode"] = "Dimension".into();
            data["length"] = design_length(name, "LengthRev", back)?.into();
            data["reversed"] = (!flip).into();
        } else {
            data["mode"] = "TwoLengths".into();
            data["length"] = design_length(name, "LengthFwd", forward)?.into();
            data["length2"] = design_length(name, "LengthRev", back)?.into();
            data["reversed"] = flip.into();
        }
        let made_text = if text && !self.out.objects.contains_key(profile) {
            match crate::text_import::text_sketch(
                shape,
                placement,
                self.document,
                &mut self.out.findings,
            )? {
                Ok(sketch) => Some(sketch),
                Err(reason) => return Ok(Err(Lost::Setting(reason))),
            }
        } else {
            None
        };
        if made_text.is_none() && !self.out.objects.contains_key(profile) {
            self.resolved(profile)?;
        }
        let body = self.new_body(label(object)?, BodyKind::Features, None);
        let sketch = match made_text {
            Some(sketch) => {
                let id = self.add_sketch(shape, sketch, Some(body))?;
                self.out
                    .objects
                    .insert(profile.into(), ImportedObject::Sketch(id));
                self.visited.insert(profile.into());
                id
            }
            None => match self.out.objects.get(profile) {
                Some(ImportedObject::Sketch(id)) => *id,
                _ => return Ok(Err(Lost::Dependency(profile.into()))),
            },
        };
        data["sketch"] = serde_json::to_value(sketch).expect("an id serializes");
        let feature: DesignFeature = serde_json::from_value(serde_json::json!({ "Pad": data }))
            .map_err(|e| object.format_error(e.to_string()))?;
        self.add_solid_step(object, feature, body)
    }

    /// Pads, pockets, dress-ups, holes, transforms, lofts and body booleans;
    /// `None` for any other kind.
    fn design_feature(
        &mut self,
        object: &FcObject,
        cut: bool,
    ) -> Result<Option<Made>, ResolveError> {
        Ok(Some(match object.type_name.as_str() {
            "PartDesign::Pad" | "PartDesign::Pocket" => self.extrusion(object, cut)?,
            "PartDesign::Fillet" | "PartDesign::Chamfer" => self.dress_up(object)?,
            "PartDesign::Hole" => self.hole(object)?,
            "PartDesign::Mirrored"
            | "PartDesign::LinearPattern"
            | "PartDesign::PolarPattern"
            | "PartDesign::MultiTransform" => self.transformed(object)?,
            "PartDesign::AdditiveLoft" | "PartDesign::SubtractiveLoft" => self.loft(object, cut)?,
            "PartDesign::Boolean" => self.body_boolean(object)?,
            _ => return Ok(None),
        }))
    }

    /// A boolean of the body's solid with whole other bodies.
    fn body_boolean(&mut self, object: &FcObject) -> Result<Made, ResolveError> {
        let mut tools = Vec::new();
        for tool in object.link_list("Group")? {
            match self.solid_of(tool, &object.name)? {
                Ok(body) => tools.push(body),
                Err(lost) => return Ok(Err(lost)),
            }
        }
        let Some((&tool_body, more_tools)) = tools.split_first() else {
            return Ok(Err(Lost::Setting("a boolean with no tool body".into())));
        };
        let kind = match object.integer("Type")? {
            0 => kernel_api::BoolKind::Fuse,
            1 => kernel_api::BoolKind::Cut,
            2 => kernel_api::BoolKind::Common,
            other => return Ok(Err(Lost::Setting(format!("unsupported Type={other}")))),
        };
        Ok(Ok(DesignFeature::BodyBoolean {
            tool_body,
            kind,
            more_tools: more_tools.to_vec(),
            refine: optional_bool(object, "Refine", false)?,
            boolean_tolerance: None,
        }))
    }

    pub(super) fn extrusion(&mut self, object: &FcObject, cut: bool) -> Result<Made, ResolveError> {
        let lost = |reason: String| Ok(Err(Lost::Setting(reason)));
        let sides = if object.has("SideType") {
            object.integer("SideType")?
        } else {
            0
        };
        let first = object.integer("Type")?;
        // Files that set the sides apart end each side on its own and have
        // no two-lengths end.
        let two_lengths = !object.has("SideType") && first == 4;
        let second = if sides == 1 {
            Some(object.integer("Type2")?)
        } else {
            None
        };
        let symmetric = match sides {
            0 => optional_bool(object, "Midplane", false)?,
            1 => false,
            2 => true,
            other => return lost(format!("unsupported SideType={other}")),
        };
        let two_sided = two_lengths || second.is_some();
        for (setting, unused) in [
            ("TaperAngle", true),
            ("TaperAngle2", two_sided),
            ("Offset", first != 3),
            ("Offset2", second.is_some_and(|end| end != 3)),
        ] {
            if unused && object.has(setting) && object.float(setting)? != 0.0 {
                return lost(format!("unsupported {setting}"));
            }
        }
        if object.has("AlongSketchNormal") && !object.bool("AlongSketchNormal")? {
            return lost("custom direction".into());
        }
        if optional_bool(object, "UseCustomVector", false)? {
            return lost("custom direction".into());
        }
        if object.has("StartType") && object.integer("StartType")? != 0 {
            return lost("start offset".into());
        }
        let (mode, up_to_face) = if two_lengths {
            ("TwoLengths", None)
        } else {
            match self.extrusion_end(object, cut, first, "UpToFace")? {
                Ok(end) => end,
                Err(lost) => return Ok(Err(lost)),
            }
        };
        let second_end = match second {
            Some(end) => match self.extrusion_end(object, cut, end, "UpToFace2")? {
                Ok(end) => Some(end),
                Err(lost) => return Ok(Err(lost)),
            },
            None => None,
        };
        if symmetric && !matches!(mode, "Dimension" | "ThroughAll") {
            return lost("symmetric extrusion to a face".into());
        }
        let Some((profile, subs)) = object.link_sub("Profile")? else {
            return lost("missing profile".into());
        };
        if !subs.is_empty() {
            return lost("profile sub-elements".into());
        }
        if !same_placement(
            self.parents.placement(self.source, &object.name)?,
            self.parents.placement(self.source, profile)?,
        ) {
            return lost("profile in another coordinate system".into());
        }
        let sketch = match self.sketch(profile)? {
            Ok(sketch) => sketch,
            Err(lost) => return Ok(Err(lost)),
        };
        let measured = matches!(mode, "Dimension" | "TwoLengths");
        // Only a measured end reads its length; the others keep one for show.
        let length = if measured {
            object.float("Length")?
        } else {
            1.0
        };
        if mode == "Dimension" && length == 0.0 {
            return lost("zero extrusion length".into());
        }
        let reversed = object.bool("Reversed")? ^ (measured && length < 0.0);
        let mut data = serde_json::json!({ "sketch": sketch, "reversed": reversed, "symmetric": symmetric, "mode": mode, "refine": optional_bool(object, "Refine", false)? });
        let (first_length, second_length) = if cut {
            ("depth", "depth2")
        } else {
            ("length", "length2")
        };
        let length_of = |name| design_length(&object.name, name, object.float(name)?.abs());
        data[first_length] = design_length(&object.name, "Length", length.abs())?.into();
        if two_lengths || second_end.is_some_and(|(end, _)| end == "Dimension") {
            data[second_length] = length_of("Length2")?.into();
        }
        // A pocket's offset moves its end along the target face's outward
        // normal, as here; a pad's moves it the other way.
        let sense = if cut { 1.0 } else { -1.0 };
        let offset_of = |name| design_length(&object.name, name, sense * object.float(name)?);
        if let Some(pick) = up_to_face {
            data["up_to_face"] = serde_json::to_value(pick).expect("a pick serializes");
            data["up_to_offset"] = offset_of("Offset")?.into();
        }
        if let Some((end, pick)) = second_end {
            data["mode2"] = end.into();
            if let Some(pick) = pick {
                data["up_to_face2"] = serde_json::to_value(pick).expect("a pick serializes");
                data["up_to_offset2"] = offset_of("Offset2")?.into();
            }
        }
        if cut {
            data["through_all"] = (mode == "ThroughAll").into();
        }
        Ok(Ok(serde_json::from_value(
            serde_json::json!({ if cut { "Pocket" } else { "Pad" }: data }),
        )
        .map_err(|e| object.format_error(e.to_string()))?))
    }

    /// How one side of a pad or pocket ends, by the file's number for it.
    /// The two kinds number their ends alike but for the second: a pad's
    /// runs to the last face, a pocket's through all.
    fn extrusion_end(
        &mut self,
        object: &FcObject,
        cut: bool,
        end: i64,
        face: &str,
    ) -> Result<Result<(&'static str, Option<FacePick>), Lost>, ResolveError> {
        Ok(Ok(match (end, cut) {
            (0, _) => ("Dimension", None),
            (1, true) => ("ThroughAll", None),
            (1, false) => ("ToLast", None),
            (2, _) => ("ToFirst", None),
            (3, _) => match self.face_pick(object, face)? {
                Ok(pick) => ("UpToFace", Some(pick)),
                Err(reason) => return Ok(Err(Lost::Setting(reason))),
            },
            _ => {
                return Ok(Err(Lost::Setting(format!(
                    "unsupported extrusion Type={end}"
                ))));
            }
        }))
    }

    fn dress_up(&mut self, object: &FcObject) -> Result<Made, ResolveError> {
        let edges = if optional_bool(object, "UseAllEdges", false)? {
            EdgeSel::All
        } else {
            let (edges, faces) = match self.picks(object, "Base")? {
                Ok(picks) => picks,
                Err(lost) => return Ok(Err(lost)),
            };
            match (edges.is_empty(), faces.is_empty()) {
                (false, true) => EdgeSel::Edges(edges),
                (true, false) => EdgeSel::Faces(faces),
                (true, true) => return Ok(Err(Lost::Setting("no edges picked".into()))),
                (false, false) => {
                    return Ok(Err(Lost::Setting("edges and faces picked together".into())));
                }
            }
        };
        let size = |name| design_length(&object.name, name, object.float(name)?);
        let refine = optional_bool(object, "Refine", false)?;
        // The source carries a fillet or chamfer on along every edge that
        // continues a picked one tangentially.
        if object.type_name == "PartDesign::Fillet" {
            return Ok(Ok(DesignFeature::Fillet {
                radius: size("Radius")?,
                edges,
                follow_tangent: true,
                corner: Default::default(),
                refine,
                mode: Default::default(),
                radius2: 0.0,
                flip: false,
                profile: Default::default(),
                rho: 0.5,
                points: Vec::new(),
                overflow: Default::default(),
            }));
        }
        let mode = match object.integer("ChamferType")? {
            0 => ChamferMode::EqualDistance,
            1 => ChamferMode::TwoDistances,
            2 => ChamferMode::DistanceAngle,
            other => {
                return Ok(Err(Lost::Setting(format!(
                    "unsupported ChamferType={other}"
                ))));
            }
        };
        Ok(Ok(DesignFeature::Chamfer {
            size: size("Size")?,
            mode,
            size2: if object.has("Size2") {
                size("Size2")?
            } else {
                0.0
            },
            angle_deg: if object.has("Angle") {
                size("Angle")?
            } else {
                0.0
            },
            flip: optional_bool(object, "FlipDirection", false)?,
            edges,
            follow_tangent: true,
            corner: Default::default(),
            refine,
            overflow: Default::default(),
        }))
    }

    fn hole(&mut self, object: &FcObject) -> Result<Made, ResolveError> {
        let sketch = match self.profile_sketch(object, "Profile")? {
            Ok(sketch) => sketch,
            Err(lost) => return Ok(Err(lost)),
        };
        let through_all = match object.integer("DepthType")? {
            0 => false,
            1 => true,
            other => return Ok(Err(Lost::Setting(format!("unsupported DepthType={other}")))),
        };
        if optional_bool(object, "Midplane", false)? {
            return Ok(Err(Lost::Setting("hole drilled both ways".into())));
        }
        let length = |name| design_length(&object.name, name, object.float(name)?);
        let cut = match enum_name(object, "HoleCutType")?.as_deref() {
            None | Some("None") => serde_json::json!("None"),
            Some(name) => {
                let diameter = length("HoleCutDiameter")?;
                let depth = length("HoleCutDepth")?;
                let angle_deg = length("HoleCutCountersinkAngle")?;
                // A standard's seat is a counterbore unless the standard
                // sinks the head.
                let sunk = ["Countersink", "2009", "7046", "10642", "14584"]
                    .iter()
                    .any(|mark| name.contains(mark));
                if name == "Counterdrill" {
                    serde_json::json!({ "Counterdrill": { "diameter": diameter, "depth": depth, "angle_deg": angle_deg } })
                } else if sunk {
                    serde_json::json!({ "Countersink": { "diameter": diameter, "angle_deg": angle_deg } })
                } else {
                    serde_json::json!({ "Counterbore": { "diameter": diameter, "depth": depth } })
                }
            }
        };
        let drill_point = if !through_all && object.integer("DrillPoint")? == 1 {
            serde_json::json!({ "Angled": { "angle_deg": length("DrillPointAngle")? } })
        } else {
            serde_json::json!("Flat")
        };
        let taper_deg = if optional_bool(object, "Tapered", false)? {
            90.0 - length("TaperedAngle")?
        } else {
            0.0
        };
        let data = serde_json::json!({ "Hole": {
            "sketch": sketch,
            "diameter": length("Diameter")?,
            "depth": length("Depth")?,
            "through_all": through_all,
            "cut": cut,
            "drill_point": drill_point,
            "point_in_depth": optional_bool(object, "DrillForDepth", false)?,
            "taper_deg": taper_deg,
            "reversed": optional_bool(object, "Reversed", false)?,
            "refine": optional_bool(object, "Refine", false)?,
        }});
        let mut feature: DesignFeature =
            serde_json::from_value(data).map_err(|e| object.format_error(e.to_string()))?;
        if optional_bool(object, "Threaded", false)? {
            let shown = match optional_bool(object, "ModelThread", false)? {
                false => shown_thread(object, &feature)?,
                true => None,
            };
            match (shown, &mut feature) {
                (
                    Some(spec),
                    DesignFeature::Hole {
                        thread,
                        threaded,
                        cosmetic_thread,
                        ..
                    },
                ) => {
                    *thread = Some(spec);
                    *threaded = true;
                    *cosmetic_thread = true;
                }
                _ => note(
                    &mut self.out.findings,
                    &object.name,
                    "object setting",
                    "thread kept as the bore it is drilled to",
                ),
            }
        }
        Ok(Ok(feature))
    }

    /// A mirror or a pattern of chosen features, which sit earlier in the
    /// same body, or of the whole solid so far.
    fn transformed(&mut self, object: &FcObject) -> Result<Made, ResolveError> {
        let whole = object.has("TransformMode") && object.integer("TransformMode")? == 1;
        let mut originals = Vec::new();
        if !whole {
            let body = self.source_body(&object.name).map(|o| o.name.clone());
            for original in object.link_list("Originals")? {
                let in_body = body
                    .as_ref()
                    .and_then(|owner| self.segments.get(owner))
                    .copied();
                match self.resolved(original)? {
                    Some(ImportedObject::Solid {
                        body,
                        feature: Some(feature),
                    }) if Some(body) == in_body => originals.push(feature),
                    Some(_) => {
                        return Ok(Err(Lost::Setting(format!(
                            "{original} is not a feature of the same body"
                        ))));
                    }
                    None => return Ok(Err(Lost::Dependency(original.into()))),
                }
            }
        }
        let refine = optional_bool(object, "Refine", false)?;
        if object.type_name == "PartDesign::MultiTransform" {
            let mut steps = Vec::new();
            for step in object.link_list("Transformations")? {
                let Some(step) = self.source.object(step) else {
                    return Ok(Err(Lost::Dependency(step.into())));
                };
                match self.step(object, step)? {
                    Ok(step) => steps.push(step),
                    Err(lost) => return Ok(Err(lost)),
                }
            }
            return Ok(Ok(DesignFeature::MultiTransform {
                originals,
                steps,
                refine,
                boolean_tolerance: None,
                skipped: Vec::new(),
            }));
        }
        Ok(match self.step(object, object)? {
            Err(lost) => Err(lost),
            Ok(TransformStep::Mirror { plane }) => Ok(DesignFeature::Mirrored {
                originals,
                plane,
                refine,
                boolean_tolerance: None,
                skipped: Vec::new(),
            }),
            Ok(TransformStep::Linear {
                axis,
                length,
                occurrences,
            }) => Ok(DesignFeature::LinearPattern {
                originals,
                axis,
                length: length.abs(),
                occurrences,
                spacing_mode: false,
                reversed: length < 0.0,
                refine,
                boolean_tolerance: None,
                spacings: Vec::new(),
                skipped: Vec::new(),
            }),
            Ok(TransformStep::Polar {
                axis,
                angle_deg,
                occurrences,
            }) => Ok(DesignFeature::PolarPattern {
                originals,
                axis,
                angle_deg: angle_deg.abs(),
                occurrences,
                reversed: angle_deg < 0.0,
                refine,
                boolean_tolerance: None,
                step_mode: false,
                angles: Vec::new(),
                skipped: Vec::new(),
            }),
            Ok(_) => Err(Lost::Setting(format!("unsupported {}", object.type_name))),
        })
    }

    /// One transform (`step`, `owner` itself or one of its steps) as a
    /// step: a mirror, or a linear or polar pattern its length or angle
    /// spanning the occurrences, a reversed one running the other way.
    pub(super) fn step(
        &mut self,
        owner: &FcObject,
        step: &FcObject,
    ) -> Result<Result<TransformStep, Lost>, ResolveError> {
        let lost = |reason: String| Ok(Err(Lost::Setting(reason)));
        let occurrences = |o: &FcObject| -> Result<Result<u32, String>, FcstdError> {
            let n = o.integer("Occurrences")?;
            Ok(u32::try_from(n)
                .ok()
                .filter(|&n| n >= 1)
                .ok_or(format!("{n} occurrences")))
        };
        // A newer file's spacing between occurrences, in place of the span.
        let by_offset = step.has("Mode") && step.integer("Mode")? == 1;
        let reversed = optional_bool(step, "Reversed", false)?;
        Ok(Ok(match step.type_name.as_str() {
            "PartDesign::Mirrored" => match self.mirror_plane(owner, step)? {
                Ok(plane) => TransformStep::Mirror { plane },
                Err(lost) => return Ok(Err(lost)),
            },
            "PartDesign::LinearPattern" => {
                let axis = match self.pattern_axis(owner, step, "Direction")? {
                    Ok(axis) => axis,
                    Err(lost) => return Ok(Err(lost)),
                };
                let occurrences = match occurrences(step)? {
                    Ok(n) => n,
                    Err(why) => return lost(why),
                };
                let span = if by_offset {
                    step.float("Offset")? * f64::from(occurrences.saturating_sub(1))
                } else {
                    step.float("Length")?
                };
                let sense = if reversed { -1.0 } else { 1.0 };
                TransformStep::Linear {
                    axis,
                    length: design_length(&step.name, "Length", sense * span)?,
                    occurrences,
                }
            }
            "PartDesign::PolarPattern" => {
                let axis = match self.pattern_axis(owner, step, "Axis")? {
                    Ok(axis) => axis,
                    Err(lost) => return Ok(Err(lost)),
                };
                let occurrences = match occurrences(step)? {
                    Ok(n) => n,
                    Err(why) => return lost(why),
                };
                let span = if by_offset {
                    step.float("Offset")? * f64::from(occurrences.saturating_sub(1))
                } else {
                    step.float("Angle")?
                };
                let sense = if reversed { -1.0 } else { 1.0 };
                TransformStep::Polar {
                    axis,
                    angle_deg: design_length(&step.name, "Angle", sense * span)?,
                    occurrences,
                }
            }
            other => return lost(format!("unsupported transform {other}")),
        }))
    }

    /// A mirror's plane: one of the body's own planes, a sketch's plane or
    /// one of its axes, or a flat face of the saved shape.
    fn mirror_plane(
        &mut self,
        owner: &FcObject,
        object: &FcObject,
    ) -> Result<Result<MirrorPlane, Lost>, ResolveError> {
        let Some((target, subs)) = object.link_sub("MirrorPlane")? else {
            return Ok(Err(Lost::Setting("missing MirrorPlane".into())));
        };
        let Some(plane) = self.source.object(target) else {
            return Ok(Err(Lost::Dependency(target.into())));
        };
        let plane = match (plane.type_name.as_str(), subs.as_slice()) {
            ("App::Plane", []) => match self.base_axis(owner, plane, target)? {
                Some(0) => Some(MirrorPlane::YZ),
                Some(1) => Some(MirrorPlane::XZ),
                Some(2) => Some(MirrorPlane::XY),
                _ => None,
            },
            ("Sketcher::SketchObject", [] | ["N_Axis"]) => match self.sketch(target)? {
                Ok(sketch) => Some(MirrorPlane::Sketch { sketch, axis: None }),
                Err(lost) => return Ok(Err(lost)),
            },
            ("Sketcher::SketchObject", [axis @ ("H_Axis" | "V_Axis")]) => {
                match self.sketch(target)? {
                    Ok(sketch) => Some(MirrorPlane::Sketch {
                        sketch,
                        axis: Some(if *axis == "H_Axis" {
                            SketchAxis::Horizontal
                        } else {
                            SketchAxis::Vertical
                        }),
                    }),
                    Err(lost) => return Ok(Err(lost)),
                }
            }
            _ => match self.face_pick(object, "MirrorPlane")? {
                Ok(pick) => Some(MirrorPlane::Face(pick)),
                Err(reason) => return Ok(Err(Lost::Setting(reason))),
            },
        };
        Ok(plane.ok_or_else(|| Lost::Setting(format!("mirror plane {target}"))))
    }

    /// A pattern's direction or axis: one of the body's own axes, a
    /// sketch's axis, or an edge of the saved shape.
    fn pattern_axis(
        &mut self,
        owner: &FcObject,
        object: &FcObject,
        property: &str,
    ) -> Result<Result<PatternAxis, Lost>, ResolveError> {
        let Some((target, subs)) = object.link_sub(property)? else {
            return Ok(Err(Lost::Setting(format!("missing {property}"))));
        };
        let Some(reference) = self.source.object(target) else {
            return Ok(Err(Lost::Dependency(target.into())));
        };
        let axis = match (reference.type_name.as_str(), subs.as_slice()) {
            ("App::Line", []) => match self.base_axis(owner, reference, target)? {
                Some(0) => Some(PatternAxis::X),
                Some(1) => Some(PatternAxis::Y),
                Some(2) => Some(PatternAxis::Z),
                _ => None,
            },
            ("Sketcher::SketchObject", [axis @ ("H_Axis" | "V_Axis" | "N_Axis")]) => {
                match self.sketch(target)? {
                    Ok(sketch) => Some(PatternAxis::Sketch {
                        sketch,
                        axis: match *axis {
                            "H_Axis" => SketchAxis::Horizontal,
                            "V_Axis" => SketchAxis::Vertical,
                            _ => SketchAxis::Normal,
                        },
                    }),
                    Err(lost) => return Ok(Err(lost)),
                }
            }
            _ => match self.picks(object, property)? {
                Ok((edges, faces)) if edges.len() == 1 && faces.is_empty() => {
                    Some(PatternAxis::Edge(edges[0]))
                }
                Ok(_) => None,
                Err(lost) => return Ok(Err(lost)),
            },
        };
        Ok(axis.ok_or_else(|| Lost::Setting(format!("{property} {target}"))))
    }

    /// Which of the world's axes (0 x, 1 y, 2 z) one of a body's own planes
    /// or axes stands for: a plane by its normal, an axis by its direction.
    /// `None` when the body has been moved, so they no longer stand where
    /// the world's do.
    fn base_axis(
        &self,
        owner: &FcObject,
        reference: &FcObject,
        target: &str,
    ) -> Result<Option<usize>, FcstdError> {
        if !same_placement(
            self.parents.placement(self.source, &owner.name)?,
            Placement::IDENTITY,
        ) {
            return Ok(None);
        }
        let role = if reference.has("Role") {
            reference.string("Role")?
        } else {
            target
        };
        Ok(match role {
            role if role.starts_with("X_") || role.starts_with("YZ") => Some(0),
            role if role.starts_with("Y_") || role.starts_with("XZ") => Some(1),
            role if role.starts_with("Z_") || role.starts_with("XY") => Some(2),
            _ => None,
        })
    }

    fn loft(&mut self, object: &FcObject, cut: bool) -> Result<Made, ResolveError> {
        let mut sections = Vec::new();
        match self.profile_sketch(object, "Profile")? {
            Ok(sketch) => sections.push(LoftSection::Feature(sketch)),
            Err(lost) => return Ok(Err(lost)),
        }
        for (name, sub) in object.link_sub_list("Sections")? {
            if !sub.is_empty() {
                return Ok(Err(Lost::Setting("section sub-elements".into())));
            }
            match self.sketch(name)? {
                Ok(sketch) => sections.push(LoftSection::Feature(sketch)),
                Err(lost) => return Ok(Err(lost)),
            }
        }
        Ok(Ok(DesignFeature::Loft {
            keep_inside: false,
            sections,
            ruled: optional_bool(object, "Ruled", false)?,
            closed: optional_bool(object, "Closed", false)?,
            subtractive: cut,
            refine: optional_bool(object, "Refine", false)?,
            boolean_tolerance: None,
        }))
    }

    /// The sketch a link names as a whole.
    fn profile_sketch(
        &mut self,
        object: &FcObject,
        property: &str,
    ) -> Result<Result<FeatureId, Lost>, ResolveError> {
        match object.link_sub(property)? {
            None => Ok(Err(Lost::Setting(format!("missing {property}")))),
            Some((_, subs)) if !subs.is_empty() => {
                Ok(Err(Lost::Setting(format!("{property} sub-elements"))))
            }
            Some((name, _)) => self.sketch(name),
        }
    }

    pub(super) fn sketch(&mut self, name: &str) -> Result<Result<FeatureId, Lost>, ResolveError> {
        Ok(match self.resolved(name)? {
            Some(ImportedObject::Sketch(id)) => Ok(id),
            _ => Err(Lost::Dependency(name.into())),
        })
    }

    /// The one face a link names, as `face_pick` for an extrusion's end.
    fn face_pick(
        &mut self,
        object: &FcObject,
        property: &str,
    ) -> Result<Result<FacePick, String>, ResolveError> {
        Ok(match self.picks(object, property)? {
            Ok((edges, faces)) if edges.is_empty() && faces.len() == 1 => Ok(faces[0]),
            Ok(_) => Err(format!("{property} names other than one face")),
            Err(Lost::Setting(reason) | Lost::Dependency(reason)) => Err(reason),
        })
    }

    /// The edges and faces a link names (`Edge3`, `Face1`), found on the
    /// shape the file saved for the linked object, placed where the
    /// document places that object.
    fn picks(
        &mut self,
        object: &FcObject,
        property: &str,
    ) -> Result<Result<Picks, Lost>, ResolveError> {
        let Some((target, subs)) = object.link_sub(property)? else {
            return Ok(Err(Lost::Setting(format!("{property} names nothing"))));
        };
        let Some(saved) = self.saved_shape(target) else {
            return Ok(Err(Lost::Setting(format!(
                "{property}: {target} has no saved shape to find {} on",
                subs.join(", ")
            ))));
        };
        let frame = self.parents.placement(self.source, target)?;
        let place = |(point, direction): Mark| {
            let point = frame.rotation.transform_vector(point);
            let origin = frame.position.to_array();
            (
                [0, 1, 2].map(|i| (point[i] + origin[i]) as f32),
                frame.rotation.transform_vector(direction).map(|v| v as f32),
            )
        };
        let (mut edges, mut faces) = (Vec::new(), Vec::new());
        for sub in subs {
            let numbered = |prefix: &str, marks: &[Option<Mark>]| {
                let index = sub.strip_prefix(prefix)?.parse::<usize>().ok()?;
                marks.get(index.checked_sub(1)?).copied().flatten()
            };
            if let Some(mark) = numbered("Edge", &saved.edges) {
                let (point, direction) = place(mark);
                edges.push(EdgePick {
                    point,
                    direction,
                    faces: Default::default(),
                });
            } else if let Some(mark) = numbered("Face", &saved.faces) {
                let (point, normal) = place(mark);
                faces.push(FacePick {
                    point,
                    normal,
                    name: Default::default(),
                });
            } else {
                return Ok(Err(Lost::Setting(format!(
                    "{property}: {sub} is not found on {target}'s saved shape"
                ))));
            }
        }
        Ok(Ok((edges, faces)))
    }

    /// The edges and faces of the shape the file saved for `name`, read once.
    pub(super) fn saved_shape(&mut self, name: &str) -> Option<Rc<SavedShape>> {
        let file = self.source.object(name)?.shape_file("Shape").ok()?;
        if let Some(read) = self.saved.get(file) {
            return read.clone();
        }
        let read = self
            .shapes
            .get(file)
            .and_then(|text| SavedShape::parse(text).ok())
            .map(Rc::new);
        self.saved.insert(file.to_owned(), read.clone());
        read
    }
}

/// A vector property, `[0; 3]` when the object has none.
fn vector(object: &FcObject, name: &str) -> Result<[f64; 3], FcstdError> {
    if !object.has(name) {
        return Ok([0.0; 3]);
    }
    let property = object.property(name)?;
    let Some(element) = property.value.iter().find(|e| e.tag == "PropertyVector") else {
        return Err(object.format_error(format!("property {name}: not a vector")));
    };
    let mut out = [0.0; 3];
    for (slot, attr) in out.iter_mut().zip(["valueX", "valueY", "valueZ"]) {
        *slot = object.attr_f64(name, element, attr)?;
    }
    Ok(out)
}

pub(super) fn to_glam(placement: Placement) -> glam::DAffine3 {
    glam::DAffine3::from_rotation_translation(
        glam::DQuat::from_array(placement.rotation.to_array()),
        glam::DVec3::from_array(placement.position.to_array()),
    )
}

/// An enumeration's chosen value by the name its list gives it, where the
/// property carries its list.
/// The thread a hole shows without cutting it, when its size is one of
/// the standards' and drilling it to that size's tap drill makes the
/// bore the file holds. A size the file writes with its pitch ("M3x0.5")
/// is found as the coarse size of that pitch where no standard names it
/// so.
fn shown_thread(object: &FcObject, hole: &DesignFeature) -> Result<Option<ThreadSpec>, FcstdError> {
    if !object.has("ThreadSize") {
        return Ok(None);
    }
    let Some(name) = enum_name(object, "ThreadSize")? else {
        return Ok(None);
    };
    let named = |standard: ThreadStandard, size: &str| {
        standard.size(size).map(|_| ThreadSpec::new(standard, size))
    };
    let found = ThreadStandard::ALL
        .into_iter()
        .find_map(|standard| named(standard, &name))
        .or_else(|| {
            let (size, pitch) = name.split_once('x')?;
            let pitch: f64 = pitch.parse().ok()?;
            let coarse = ThreadStandard::IsoMetricCoarse.size(size)?;
            ((coarse.pitch - pitch).abs() < 1e-9)
                .then(|| ThreadSpec::new(ThreadStandard::IsoMetricCoarse, size))
        });
    let Some(mut spec) = found else {
        return Ok(None);
    };
    spec.left_handed = object.has("ThreadDirection") && object.integer("ThreadDirection")? == 1;
    let mut threaded = hole.clone();
    if let DesignFeature::Hole {
        thread,
        threaded: on,
        ..
    } = &mut threaded
    {
        *thread = Some(spec.clone());
        *on = true;
    }
    let drilled = f64::from(wb_design::hole_diameter(&threaded));
    let bore = object.float("Diameter")?;
    Ok(((drilled - bore).abs() < 1e-3).then_some(spec))
}

fn enum_name(object: &FcObject, name: &str) -> Result<Option<String>, FcstdError> {
    let index = object.integer(name)?;
    Ok(object
        .property(name)?
        .value
        .iter()
        .find(|element| element.tag == "CustomEnumList")
        .and_then(|list| {
            list.children_named("Enum")
                .nth(usize::try_from(index).ok()?)
        })
        .and_then(|value| value.attr("value"))
        .map(str::to_owned))
}

pub(super) fn label(object: &FcObject) -> Result<String, FcstdError> {
    Ok(if object.has("Label") {
        object.string("Label")?.into()
    } else {
        object.name.clone()
    })
}

fn optional_bool(object: &FcObject, name: &str, absent: bool) -> Result<bool, FcstdError> {
    if object.has(name) {
        object.bool(name)
    } else {
        Ok(absent)
    }
}
