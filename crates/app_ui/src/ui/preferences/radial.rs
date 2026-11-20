//! The Input page's Radial menu tab: each workbench's slices.

use super::*;

/// Each workbench's radial menu: the slices, clockwise from the top, that
/// a right press held still over the view offers while it is active.
pub(super) fn radial_page(ui: &mut Ui, state: &mut PreferencesState, registry: &DocumentService) {
    let benches: Vec<_> = registry
        .ids()
        .iter()
        .filter_map(|id| {
            let descriptor = registry.descriptor(id)?;
            let tools: Vec<_> = registry
                .tools_for(id)
                .ok()?
                .iter()
                .filter(|t| t.planned.is_none())
                .map(|t| (t.id.clone(), t.label.clone()))
                .collect();
            (!tools.is_empty()).then(|| {
                (
                    id,
                    descriptor.label.clone(),
                    descriptor.radial.clone(),
                    tools,
                )
            })
        })
        .collect();
    if benches.is_empty() {
        return;
    }
    state.radial_bench = state.radial_bench.min(benches.len() - 1);
    note_card(
        ui,
        Note::Info,
        None,
        "Hold the right button still over the view to open the menu, move toward a slice \
         and let go to start it. A quick right click still opens the view's menu.",
    );
    ui.add_space(SPACE_2);
    let names: Vec<(usize, &str)> = benches
        .iter()
        .enumerate()
        .map(|(i, (_, label, _, _))| (i, label.as_str()))
        .collect();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Workbench").font(sans(FONT_SM)).color(TEXT2));
        select_field(ui, "radial_bench", &mut state.radial_bench, &names, 200.0);
    });
    let (bench, _, declared, tools) = &benches[state.radial_bench];
    let own = state.draft.radial.get(bench.as_str()).cloned();
    let shown = crate::contract::RadialView::chosen_ids(
        bench,
        declared,
        &registry
            .tools_for(bench)
            .map(<[_]>::to_vec)
            .unwrap_or_default(),
        &state.draft,
    );
    // A slot's choices: nothing, the bench's tools, and a command the
    // settings name that is no tool, as it is.
    let mut choices: Vec<(String, String)> = vec![(String::new(), "(nothing)".into())];
    choices.extend(tools.iter().cloned());
    for id in &shown {
        if !choices.iter().any(|(c, _)| c == id) {
            choices.push((id.clone(), id.clone()));
        }
    }
    let options: Vec<(usize, &str)> = choices
        .iter()
        .enumerate()
        .map(|(i, (_, label))| (i, label.as_str()))
        .collect();
    let mut slots: Vec<String> = shown.clone();
    slots.resize(core_document::RADIAL_SLOTS, String::new());
    let mut changed = false;
    for (i, slot) in slots.iter_mut().enumerate() {
        let mut picked = choices.iter().position(|(id, _)| id == slot).unwrap_or(0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Slice {}", i + 1))
                    .font(sans(FONT_SM))
                    .color(TEXT2),
            );
            if select_field(ui, ("radial_slot", i), &mut picked, &options, 240.0) {
                *slot = choices[picked].0.clone();
                changed = true;
            }
        });
    }
    if changed {
        slots.retain(|id| !id.is_empty());
        state.draft.radial.insert(bench.as_str().to_string(), slots);
    }
    ui.add_space(SPACE_2);
    let label = if declared.is_empty() {
        "Most used tools"
    } else {
        "The workbench's own"
    };
    if own.is_some() && secondary_button(ui, label).clicked() {
        state.draft.radial.remove(bench.as_str());
    }
}
