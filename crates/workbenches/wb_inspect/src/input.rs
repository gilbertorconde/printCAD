//! Pointer and key input: tools activated, clicks that arm, pick and
//! grow fits, pins and the alignment by points.

use crate::{
    BEST_FIT, CLEARANCE, COMPARE_COMMAND, COMPARE_TOOL, CONSTRUCT_COMMAND, DEFAULT_CUTOFF,
    DIMENSION_COMMAND, ELEMENT_MAP, EXTRACT, FIT_COMMAND, GROW_COMMAND, InspectionWorkbench,
    OVERHANG, PIN, RESET_ALIGNMENT, SAVE_IGES, SAVE_REPORT, SAVE_STEP, SECTION, SHADOW, THICKNESS,
    TOLERANCE, align, align_commands, clearance, compare, compare::Comparison,
    construction_export::write_construction, element, element::Element, fitting::answer_of,
    fitting::fit_marked, fitting::grow_at, landing::record_alignment, making::add_construction,
    making::add_dimension, making::add_tolerance, making::characteristic_for, making::element_at,
    making::element_solid, making::selected_elements, mapping::add_clearance,
    mapping::add_comparison, mapping::add_overhang, mapping::add_thickness, mapping::compare_pair,
    mapping::thickness_body, overhang, picks, registration::dim_kind_of_tool,
    registration::kind_of_tool, registration::made_of_tool, report, report::report_of, thickness,
    tolerance, tolerance_task,
};
use core_document::{
    BodyId, CommandArgs, InputResult, Workbench, WorkbenchInputEvent, WorkbenchRuntimeContext,
};
use serde_json::json;

/// The limit a tolerance made from the toolbar starts at, mm.
const DEFAULT_LIMIT: f64 = 0.1;

impl InspectionWorkbench {
    pub(super) fn handle_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        if let (WorkbenchInputEvent::ToolActivated, Some(id @ (SECTION | SHADOW))) =
            (event, active_tool)
        {
            let mut args = CommandArgs::new();
            match (selected_elements(ctx).first(), ctx.selected_body_id) {
                (Some(element), _) => {
                    args.insert("element".into(), json!(element.0.to_string()));
                }
                (None, Some(scan)) => {
                    args.insert("scan".into(), json!(scan.to_string()));
                }
                (None, None) => return InputResult::consumed(),
            }
            match self.run_command(id, &args, ctx) {
                Ok(answer) => ctx.record(id, args, answer),
                Err(why) => ctx.log_warn(why.to_string()),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(ELEMENT_MAP)) = (event, active_tool) {
            let Some(&element) = selected_elements(ctx).first() else {
                return InputResult::consumed();
            };
            let mut args = CommandArgs::new();
            args.insert("element".into(), json!(element.0.to_string()));
            match self.run_command(ELEMENT_MAP, &args, ctx) {
                Ok(answer) => ctx.record(ELEMENT_MAP, args, answer),
                Err(why) => ctx.log_warn(why.to_string()),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(id @ (SAVE_STEP | SAVE_IGES))) =
            (event, active_tool)
        {
            let iges = id == SAVE_IGES;
            let (kind, extension) = if iges {
                ("IGES", "igs")
            } else {
                ("STEP", "step")
            };
            let items = self.construction_items(ctx.document, None);
            let written = ctx
                .kernel
                .ok_or_else(|| "no kernel to write with".to_string())
                .and_then(|k| write_construction(k, &items, iges).map_err(|e| e.to_string()));
            match written {
                Ok((contents, warnings)) => {
                    for warning in warnings {
                        ctx.log_warn(format!("Save elements as {kind}: {warning}"));
                    }
                    ctx.request(core_document::HostRequest::SaveFile {
                        name: "inspection".to_string(),
                        kind: kind.to_string(),
                        extension: extension.to_string(),
                        contents,
                    });
                }
                Err(why) => ctx.log_warn(format!("Save elements as {kind}: {why}")),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(SAVE_REPORT)) = (event, active_tool) {
            let report = report_of(
                ctx.document,
                None,
                &self.tab.tolerance_readings,
                ctx.kernel,
                true,
            );
            ctx.request(core_document::HostRequest::SaveFile {
                name: "inspection report".to_string(),
                kind: "CSV".to_string(),
                extension: "csv".to_string(),
                contents: report::csv(&report).into_bytes(),
            });
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(TOLERANCE)) = (event, active_tool) {
            let ids = selected_elements(ctx);
            let picked: Vec<Element> = ids
                .iter()
                .filter_map(|id| element_at(ctx.document, *id))
                .collect();
            let Some((characteristic, angle_deg)) = characteristic_for(&picked) else {
                ctx.log_warn("select an element, or an element and its datum");
                return InputResult::consumed();
            };
            let t = tolerance::Tolerance {
                name: String::new(),
                scan: picked[0].scan,
                characteristic,
                element: ids[0],
                datum: ids.get(1).copied(),
                limit: DEFAULT_LIMIT,
                angle_deg,
                ..Default::default()
            };
            match add_tolerance(ctx, t) {
                Ok(answer) => {
                    let id = core_document::FeatureId(
                        answer["tolerance"].as_str().unwrap().parse().unwrap(),
                    );
                    self.tab.tolerance_task = Some(tolerance_task::Task { id, opening: None });
                }
                Err(why) => ctx.log_warn(why),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(THICKNESS)) = (event, active_tool) {
            let Some(body) = thickness_body(ctx) else {
                ctx.log_warn("select the body to read");
                return InputResult::consumed();
            };
            let t = thickness::WallThickness {
                name: String::new(),
                body,
                sphere: false,
                min_wall: None,
                max: None,
            };
            match add_thickness(ctx, &self.tab.maps, t) {
                Ok(answer) => {
                    let mut args = CommandArgs::new();
                    args.insert("body".into(), json!(body.0.to_string()));
                    ctx.log_info(format!(
                        "{}: {:.3} to {:.3} mm",
                        answer["name"].as_str().unwrap_or_default(),
                        answer["min"].as_f64().unwrap_or(f64::NAN),
                        answer["max"].as_f64().unwrap_or(f64::NAN)
                    ));
                    ctx.record(THICKNESS, args, answer);
                }
                Err(why) => ctx.log_warn(why),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(EXTRACT)) = (event, active_tool) {
            let picked = selected_elements(ctx);
            let made = match picked.as_slice() {
                [id] => element_solid(ctx.document, *id, 2.0),
                _ => Err("select one fitted surface".to_string()),
            };
            match made {
                Ok(solid) => {
                    let name = solid["name"].as_str().unwrap_or("Element").to_string();
                    let body = ctx.document.create_body(None);
                    ctx.document.rename_body(body, name.clone());
                    let mut args = CommandArgs::new();
                    args.insert("name".into(), json!(name));
                    ctx.record("doc.new_body", args, json!(body.0.to_string()));
                    let mut args = CommandArgs::new();
                    args.insert("body".into(), json!(body.0.to_string()));
                    args.insert("kind".into(), solid["kind"].clone());
                    args.insert("placement".into(), solid["placement"].clone());
                    ctx.request(core_document::HostRequest::RunCommand {
                        id: "design.primitive".into(),
                        args,
                    });
                    ctx.request(core_document::HostRequest::SelectBody(body));
                    ctx.request(core_document::HostRequest::JournalLabel(format!(
                        "{name} as body"
                    )));
                }
                Err(why) => ctx.log_warn(why),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(CLEARANCE)) = (event, active_tool) {
            let Some((part, housing)) = compare_pair(ctx) else {
                ctx.log_warn("select the scanned part; the solid it fits into is found");
                return InputResult::consumed();
            };
            let c = clearance::Clearance {
                name: String::new(),
                part,
                housing,
                min_gap: None,
                reach: clearance::DEFAULT_REACH,
            };
            match add_clearance(ctx, &self.tab.maps, c) {
                Ok(answer) => {
                    let mut args = CommandArgs::new();
                    args.insert("part".into(), json!(part.0.to_string()));
                    args.insert("housing".into(), json!(housing.0.to_string()));
                    ctx.log_info(format!(
                        "{}: {:.1} % interfering, narrowest gap {}",
                        answer["name"].as_str().unwrap_or_default(),
                        answer["interfering"].as_f64().unwrap_or(0.0) * 100.0,
                        answer["narrowest"]
                            .as_f64()
                            .map_or("none".to_string(), |g| format!("{g:.3} mm"))
                    ));
                    ctx.record(CLEARANCE, args, answer);
                }
                Err(why) => ctx.log_warn(why),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(OVERHANG)) = (event, active_tool) {
            let Some(body) = thickness_body(ctx) else {
                ctx.log_warn("select the body to read");
                return InputResult::consumed();
            };
            let o = overhang::Overhang {
                name: String::new(),
                body,
                limit_deg: 45.0,
            };
            match add_overhang(ctx, &self.tab.maps, o) {
                Ok(answer) => {
                    let mut args = CommandArgs::new();
                    args.insert("body".into(), json!(body.0.to_string()));
                    ctx.log_info(format!(
                        "{}: {:.1} % of the surface past 45°, steepest {:.0}°",
                        answer["name"].as_str().unwrap_or_default(),
                        answer["share"].as_f64().unwrap_or(0.0) * 100.0,
                        answer["steepest"].as_f64().unwrap_or(0.0)
                    ));
                    ctx.record(OVERHANG, args, answer);
                }
                Err(why) => ctx.log_warn(why),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(align_commands::POINTS)) =
            (event, active_tool)
        {
            let aligning = !self.tab.picks.aligning;
            self.tab.picks.clear();
            self.tab.picks.aligning = aligning;
            self.tab.armed = None;
            self.tab.pinning = false;
            return InputResult::consumed();
        }
        if self.tab.picks.aligning
            && let Some(done) = self.align_input(event, ctx)
        {
            return done;
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(id)) = (event, active_tool)
            && align_commands::owns(id)
        {
            if let Some(args) = align_commands::selected_args(id, ctx) {
                match align_commands::run(id, &args, ctx) {
                    Ok(answer) => ctx.record(id, args, answer),
                    Err(why) => ctx.log_warn(why.to_string()),
                }
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(BEST_FIT)) = (event, active_tool) {
            if self.tab.fitting.cancel() {
                ctx.log_info("Best fit stopped");
                return InputResult::consumed();
            }
            let (Some((scan, reference)), Some(kernel)) = (compare_pair(ctx), ctx.kernel) else {
                ctx.log_warn("select the reference body, with one scan in the document");
                return InputResult::consumed();
            };
            let inputs = match compare::inputs_for(ctx.document, scan, reference) {
                Ok(inputs) => inputs,
                Err(why) => {
                    ctx.log_warn(why);
                    return InputResult::consumed();
                }
            };
            let placed = ctx.document.body_placement(reference);
            let reply = self.tab.fitting.begin(ctx.document, scan, reference, true);
            std::thread::spawn(move || {
                reply.send(align::best_fit(kernel, &inputs, placed, true));
            });
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(RESET_ALIGNMENT)) = (event, active_tool) {
            let scan = compare_pair(ctx)
                .map(|(scan, _)| scan)
                .or_else(|| ctx.selected_body_id.map(BodyId));
            if let Some(scan) = scan {
                match record_alignment(
                    ctx,
                    scan,
                    align::How::Reset,
                    core_document::BodyPlacement::IDENTITY,
                ) {
                    Ok(answer) => {
                        let mut args = CommandArgs::new();
                        args.insert("scan".into(), json!(scan.0.to_string()));
                        ctx.record(RESET_ALIGNMENT, args, answer);
                    }
                    Err(why) => ctx.log_warn(why),
                }
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(COMPARE_TOOL)) = (event, active_tool) {
            let Some((scan, reference)) = compare_pair(ctx) else {
                ctx.log_warn("select the reference body, with one scan in the document");
                return InputResult::consumed();
            };
            let c = Comparison {
                name: String::new(),
                scan,
                reference,
                range: None,
                bands: 0,
                ramp: metrology::ramp::Ramp::Jet.name().to_string(),
                tolerance: None,
                max: None,
                facing_limit_deg: Some(90.0),
                volume_cell: None,
            };
            match add_comparison(ctx, &self.tab.maps, c) {
                Ok((_, answer)) => {
                    let mut args = CommandArgs::new();
                    args.insert("scan".into(), json!(scan.0.to_string()));
                    args.insert("reference".into(), json!(reference.0.to_string()));
                    ctx.log_info(format!(
                        "{}: RMS {:.4} mm over {} points",
                        answer["name"].as_str().unwrap_or_default(),
                        answer["rms"].as_f64().unwrap_or(f64::NAN),
                        answer["measured"]
                    ));
                    ctx.record(COMPARE_COMMAND, args, answer);
                }
                Err(why) => ctx.log_warn(why),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(made)) =
            (event, active_tool.and_then(made_of_tool))
        {
            let from = selected_elements(ctx);
            match add_construction(ctx, made, from.clone(), 0.0) {
                Ok((_, answer)) => {
                    let mut args = CommandArgs::new();
                    args.insert("made".into(), json!(made.word().to_lowercase()));
                    args.insert(
                        "from".into(),
                        json!(from.iter().map(|e| e.0.to_string()).collect::<Vec<_>>()),
                    );
                    ctx.log_info(format!(
                        "{} made",
                        answer["name"].as_str().unwrap_or_default()
                    ));
                    ctx.record(CONSTRUCT_COMMAND, args, answer);
                }
                Err(why) => ctx.log_warn(why),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(kind)) =
            (event, active_tool.and_then(dim_kind_of_tool))
        {
            let elements = selected_elements(ctx);
            match add_dimension(ctx, kind, elements.clone(), None) {
                Ok((_, answer)) => {
                    let mut args = CommandArgs::new();
                    args.insert("kind".into(), json!(kind.word().to_lowercase()));
                    args.insert(
                        "elements".into(),
                        json!(elements.iter().map(|e| e.0.to_string()).collect::<Vec<_>>()),
                    );
                    ctx.log_info(format!(
                        "{}: {}",
                        answer["name"].as_str().unwrap_or_default(),
                        answer["value"]
                            .as_f64()
                            .map_or(String::new(), |v| format!("{v:.3} {}", kind.unit()))
                    ));
                    ctx.record(DIMENSION_COMMAND, args, answer);
                }
                Err(why) => ctx.log_warn(why),
            }
            return InputResult::consumed();
        }
        if let (WorkbenchInputEvent::ToolActivated, Some(PIN)) = (event, active_tool) {
            self.tab.pinning = !self.tab.pinning;
            self.tab.armed = None;
            return InputResult::consumed();
        }
        if self.tab.pinning
            && let WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                ..
            } = event
        {
            if let Some((scan, _, at)) = self.hovered_reading(ctx) {
                let mut args = CommandArgs::new();
                args.insert("scan".into(), json!(scan.0.to_string()));
                args.insert("at".into(), json!(at));
                match self.run_command(PIN, &args, ctx) {
                    Ok(answer) => ctx.record(PIN, args, answer),
                    Err(why) => ctx.log_warn(why.to_string()),
                }
                return InputResult::consumed();
            }
            return InputResult::ignored();
        }
        match event {
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Escape,
            } if self.tab.fitting.running() => {
                self.tab.fitting.cancel();
                ctx.log_info("Best fit stopped");
                return InputResult::consumed();
            }
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Escape,
            } if self.tab.armed.is_some()
                || self.tab.pinning
                || self.tab.picks.aligning
                || self.tab.picks.any() =>
            {
                self.tab.armed = None;
                self.tab.pinning = false;
                self.tab.picks.clear();
                return InputResult::consumed();
            }
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                ..
            } => {
                let Some(kind) = self.tab.armed else {
                    return InputResult::ignored();
                };
                let Some(scan) = ctx
                    .hovered_body_id
                    .map(BodyId)
                    .filter(|b| ctx.document.body_scan(*b).is_some())
                else {
                    return InputResult::ignored();
                };
                let Some(face) = ctx.hovered_face else {
                    return InputResult::ignored();
                };
                let at = ctx
                    .document
                    .body_placement(scan)
                    .inverse()
                    .point(face.point)
                    .map(f64::from);
                // Shift pins the click as one more seed; a plain click
                // grows the fit from every pin and itself.
                if ctx.shift_down {
                    self.tab.picks.pin_seed(scan, at);
                    return InputResult::consumed();
                }
                let more = self.tab.picks.take_seeds(scan);
                match grow_at(
                    ctx,
                    kind,
                    scan,
                    at,
                    &more,
                    Some(DEFAULT_CUTOFF),
                    false,
                    kernel_api::FitMethod::LeastSquares,
                ) {
                    Ok((id, element)) => {
                        let mut args = CommandArgs::new();
                        args.insert(
                            "kind".into(),
                            json!(element::kind_word(kind).to_lowercase()),
                        );
                        args.insert("scan".into(), json!(scan.0.to_string()));
                        args.insert("at".into(), json!(at));
                        if !more.is_empty() {
                            args.insert("more".into(), json!(more));
                        }
                        ctx.record(GROW_COMMAND, args, answer_of(id, &element));
                        ctx.log_info(format!("{}: {}", element.name, element.result.summary()));
                    }
                    Err(why) => ctx.log_warn(why),
                }
                return InputResult::consumed();
            }
            _ => {}
        }
        let (WorkbenchInputEvent::ToolActivated, Some(kind)) =
            (event, active_tool.and_then(kind_of_tool))
        else {
            return InputResult::ignored();
        };
        if ctx.current_marking().is_none() {
            self.tab.armed = (self.tab.armed != Some(kind)).then_some(kind);
            self.tab.picks.clear();
            return InputResult::consumed();
        }
        match fit_marked(
            ctx,
            kind,
            Some(DEFAULT_CUTOFF),
            kernel_api::FitMethod::LeastSquares,
        ) {
            Ok((id, element)) => {
                let mut args = CommandArgs::new();
                args.insert(
                    "kind".into(),
                    json!(element::kind_word(kind).to_lowercase()),
                );
                ctx.record(FIT_COMMAND, args, answer_of(id, &element));
                ctx.log_info(format!("{}: {}", element.name, element.result.summary()));
            }
            Err(why) => ctx.log_warn(why),
        }
        InputResult::consumed()
    }

    /// A click or a key while the alignment by points is picking; `None`
    /// leaves the event to the rest.
    fn align_input(
        &mut self,
        event: &WorkbenchInputEvent,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> Option<InputResult> {
        match event {
            WorkbenchInputEvent::MousePress {
                button: core_document::MouseButton::Left,
                ..
            } => {
                let body = BodyId(ctx.hovered_body_id?);
                let at = ctx
                    .document
                    .body_placement(body)
                    .inverse()
                    .point(ctx.hovered_face.as_ref()?.point)
                    .map(f64::from);
                let scan = ctx.document.body_scan(body).is_some();
                self.tab.picks.pick_pair_point(body, scan, at);
                Some(InputResult::consumed())
            }
            WorkbenchInputEvent::KeyPress {
                key: core_document::KeyCode::Enter,
            } => {
                let Some(args) = self.tab.picks.align_args(ctx.document) else {
                    ctx.log_warn(format!(
                        "align by points takes {} pairs on one scan; {} so far",
                        picks::PAIRS_NEEDED,
                        self.tab.picks.pairs()
                    ));
                    return Some(InputResult::consumed());
                };
                match align_commands::run(align_commands::POINTS, &args, ctx) {
                    Ok(answer) => {
                        ctx.record(align_commands::POINTS, args, answer);
                        self.tab.picks.clear();
                    }
                    Err(why) => ctx.log_warn(why.to_string()),
                }
                Some(InputResult::consumed())
            }
            _ => None,
        }
    }
}
