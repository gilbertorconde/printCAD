//! The text fields of a selected text element, and how their edits
//! change its spec.

use super::*;

/// The font chooser's entries past the bundled fonts: an installed font,
/// by family, or a font file.
const INSTALLED: usize = usize::MAX - 1;

const FILE: usize = usize::MAX;

/// Which font entry a text shows: a bundled font (unless a mode was
/// chosen before one is named), an installed family, or a file.
fn font_mode(spec: &crate::text::TextSpec, mode: Option<usize>) -> usize {
    match crate::text::FONTS
        .iter()
        .position(|(name, _)| *name == spec.font)
    {
        Some(i) => mode.filter(|m| *m >= INSTALLED).unwrap_or(i),
        None if crate::text::is_file(&spec.font) => FILE,
        None => INSTALLED,
    }
}

/// A text's string, font, size, spacing and turn as grid fields, ids
/// under `salt`.
pub(super) fn text_fields(
    salt: &str,
    spec: &crate::text::TextSpec,
    mode: Option<usize>,
) -> Vec<Widget> {
    let fonts = crate::text::FONTS;
    let which = font_mode(spec, mode);
    let mut options: Vec<String> = fonts.iter().map(|(name, _)| name.to_string()).collect();
    options.push("An installed font".into());
    options.push("A font file".into());
    let selected = match which {
        INSTALLED => fonts.len(),
        FILE => fonts.len() + 1,
        i => i,
    };
    let mut fields = vec![
        bench_api::Entry {
            id: format!("{salt}.text"),
            label: "Text".into(),
            value: spec.text.clone(),
            multiline: true,
            width: Some(160.0),
            // A text waiting in the view follows each key.
            live: salt == "text_draft",
            ..Default::default()
        }
        .into(),
        Select {
            id: format!("{salt}.font"),
            label: "Font".into(),
            options,
            selected,
            width: Some(160.0),
            ..Default::default()
        }
        .into(),
    ];
    let named = !crate::text::is_bundled(&spec.font);
    if which == INSTALLED {
        let families = crate::text::installed_families();
        fields.push(
            Select {
                id: format!("{salt}.family"),
                label: "Family".into(),
                options: families.to_vec(),
                search: true,
                selected: families
                    .iter()
                    .position(|f| named && f.eq_ignore_ascii_case(&spec.font))
                    .unwrap_or(usize::MAX),
                width: Some(160.0),
                ..Default::default()
            }
            .into(),
        );
    }
    if which == FILE {
        fields.push(
            bench_api::Entry {
                id: format!("{salt}.file"),
                label: "File".into(),
                value: if named {
                    spec.font.clone()
                } else {
                    String::new()
                },
                placeholder: "/path/to/font.ttf".into(),
                width: Some(160.0),
                ..Default::default()
            }
            .into(),
        );
    }
    use crate::text::{Align, PathSide, TextLayout, VAlign};
    fields.push(mm(&format!("{salt}.size"), "Size", spec.size));
    fields.push(mm(&format!("{salt}.spacing"), "Spacing", spec.spacing));
    // The tool chooses where new text goes; a block keeps what it was made
    // as.
    if salt == "text_draft" {
        fields.push(toggles(
            salt,
            "layout",
            "Layout",
            &[
                ("point", "Text from a point"),
                ("rectangle", "Text in a frame"),
                ("arc", "Text along a curve"),
            ],
            Some(layout_index(&spec.layout)),
        ));
    }
    let along = matches!(spec.layout, TextLayout::Path { .. });
    if spec.layout.is_point() {
        fields.push(degrees(&format!("{salt}.angle"), "Angle", spec.angle));
    }
    let align = [Align::Start, Align::Middle, Align::End];
    fields.push(toggles(
        salt,
        "align",
        "Align",
        &[
            ("text-align-start", "Align to the start"),
            ("text-align-middle", "Centre"),
            ("text-align-end", "Align to the end"),
        ],
        align.iter().position(|a| *a == spec.align),
    ));
    if !along {
        let valign = [
            VAlign::Baseline,
            VAlign::Top,
            VAlign::Middle,
            VAlign::Bottom,
        ];
        fields.push(toggles(
            salt,
            "valign",
            "Up and down",
            &[
                (
                    "text-valign-baseline",
                    "The first line's baseline on the point (in a frame, at its top)",
                ),
                (
                    "text-valign-top",
                    "The top on the point, or at the frame's top",
                ),
                (
                    "text-valign-middle",
                    "The middle on the point, or in the frame's middle",
                ),
                (
                    "text-valign-bottom",
                    "The bottom on the point, or at the frame's bottom",
                ),
            ],
            valign.iter().position(|v| *v == spec.valign),
        ));
        fields.push(
            Quantity {
                id: format!("{salt}.line_spacing"),
                label: "Line spacing".into(),
                value: spec.line_spacing,
                dim: Dim::Number,
                min: Some(0.1),
                max: Some(10.0),
                speed: Some(0.01),
                ..Default::default()
            }
            .into(),
        );
    }
    match &spec.layout {
        TextLayout::Point => {}
        TextLayout::Frame { wrap, fit, .. } => {
            fields.push(switches(
                salt,
                "Frame",
                &[
                    (
                        "wrap",
                        "text-wrap",
                        "Break lines between words to keep inside the frame",
                        *wrap,
                    ),
                    (
                        "fit",
                        "text-shrink-fit",
                        "Shrink text larger than the frame to fit it",
                        *fit,
                    ),
                ],
            ));
        }
        TextLayout::Path {
            side,
            offset,
            fit,
            reversed,
            ..
        } => {
            let sides = [PathSide::Above, PathSide::Below, PathSide::Centred];
            fields.push(toggles(
                salt,
                "side",
                "Side",
                &[
                    ("text-path-above", "Standing on the curve"),
                    ("text-path-below", "Hanging from the curve"),
                    ("text-path-centred", "Centred on the curve"),
                ],
                sides.iter().position(|v| v == side),
            ));
            fields.push(mm(&format!("{salt}.offset"), "Offset", *offset));
            fields.push(switches(
                salt,
                "Curve",
                &[
                    (
                        "fit",
                        "text-path-fill",
                        "Spread the letters over the whole curve",
                        *fit,
                    ),
                    (
                        "reversed",
                        "text-path-reverse",
                        "Walk the curve the other way: on a closed curve, inside it",
                        *reversed,
                    ),
                ],
            ));
        }
    }
    fields
}

/// A row of icon buttons choosing one of `choices` (icon, words on hover)
/// for `field`, the one at `chosen` lit; a click is `<salt>.<field>:<n>`.
fn toggles(
    salt: &str,
    field: &str,
    label: &str,
    choices: &[(&str, &str)],
    chosen: Option<usize>,
) -> Widget {
    bench_api::Toggles {
        id: format!("{salt}.{field}"),
        label: label.into(),
        buttons: choices
            .iter()
            .enumerate()
            .map(|(n, (icon, hint))| bench_api::ToggleButton {
                id: format!("{salt}.{field}:{n}"),
                icon: icon.to_string(),
                hint: hint.to_string(),
                on: chosen == Some(n),
                disabled: false,
            })
            .collect(),
    }
    .into()
}

/// A row of icon buttons each switching one setting (its field, icon,
/// words on hover and whether it is on); a click is `<salt>.<field>`.
fn switches(salt: &str, label: &str, items: &[(&str, &str, &str, bool)]) -> Widget {
    bench_api::Toggles {
        id: format!("{salt}.{label}"),
        label: label.into(),
        buttons: items
            .iter()
            .map(|(field, icon, hint, on)| bench_api::ToggleButton {
                id: format!("{salt}.{field}"),
                icon: icon.to_string(),
                hint: hint.to_string(),
                on: *on,
                disabled: false,
            })
            .collect(),
    }
    .into()
}

/// The Text tool's layouts, in the order its Layout buttons show them.
fn layout_index(layout: &crate::text::TextLayout) -> usize {
    match layout {
        crate::text::TextLayout::Point => 0,
        crate::text::TextLayout::Frame { .. } => 1,
        crate::text::TextLayout::Path { .. } => 2,
    }
}

/// A text's icon button clicked: a choice (`align:1`) or a switch
/// (`wrap`), which turns over. Whether the text changed.
pub(super) fn text_button(spec: &mut crate::text::TextSpec, button: &str) -> bool {
    match button.split_once(':') {
        Some((field, n)) => match n.parse::<usize>() {
            Ok(index) => text_choice(spec, &mut None, field, index),
            Err(_) => false,
        },
        None => {
            let on = match (&spec.layout, button) {
                (crate::text::TextLayout::Frame { wrap, .. }, "wrap") => !*wrap,
                (
                    crate::text::TextLayout::Frame { fit, .. }
                    | crate::text::TextLayout::Path { fit, .. },
                    "fit",
                ) => !*fit,
                (crate::text::TextLayout::Path { reversed, .. }, "reversed") => !*reversed,
                _ => return false,
            };
            text_toggle(spec, button, on)
        }
    }
}

/// A text's layout switch turned. Whether it changed.
pub(super) fn text_toggle(spec: &mut crate::text::TextSpec, field: &str, on: bool) -> bool {
    use crate::text::TextLayout;
    let flag = match (&mut spec.layout, field) {
        (TextLayout::Frame { wrap, .. }, "wrap") => wrap,
        (TextLayout::Frame { fit, .. } | TextLayout::Path { fit, .. }, "fit") => fit,
        (TextLayout::Path { reversed, .. }, "reversed") => reversed,
        _ => return false,
    };
    let changed = *flag != on;
    *flag = on;
    changed
}

/// A text's string or font file typed in. Whether it changed.
pub(super) fn text_string(spec: &mut crate::text::TextSpec, field: &str, value: &str) -> bool {
    match field {
        "text" => spec.text = value.to_string(),
        "file" => {
            spec.font = value.to_string();
            spec.font_asset = None;
        }
        _ => return false,
    }
    true
}

/// A text's size, spacing or turn set. Whether it changed.
pub(super) fn text_number(spec: &mut crate::text::TextSpec, field: &str, value: f64) -> bool {
    match field {
        "size" => spec.size = value.max(0.1),
        "spacing" => spec.spacing = value,
        "angle" => spec.angle = value,
        "line_spacing" => spec.line_spacing = value.max(0.1),
        "offset" => match &mut spec.layout {
            crate::text::TextLayout::Path { offset, .. } => *offset = value,
            _ => return false,
        },
        _ => return false,
    }
    true
}

/// A text's font or family chosen. A bundled font or a family is set at
/// once; choosing to name an installed font or a file is remembered in
/// `mode` until one is named. Whether the text changed.
pub(super) fn text_choice(
    spec: &mut crate::text::TextSpec,
    mode: &mut Option<usize>,
    field: &str,
    index: usize,
) -> bool {
    use crate::text::{Align, PathSide, TextLayout, VAlign};
    let fonts = crate::text::FONTS;
    match field {
        "align" => {
            spec.align = [Align::Start, Align::Middle, Align::End][index.min(2)];
            true
        }
        "valign" => {
            spec.valign = [
                VAlign::Baseline,
                VAlign::Top,
                VAlign::Middle,
                VAlign::Bottom,
            ][index.min(3)];
            true
        }
        "side" => match &mut spec.layout {
            TextLayout::Path { side, .. } => {
                *side = [PathSide::Above, PathSide::Below, PathSide::Centred][index.min(2)];
                true
            }
            _ => false,
        },
        "layout" => {
            spec.layout = match index {
                1 => TextLayout::Frame {
                    corners: [Uuid::nil(); 4],
                    wrap: true,
                    fit: false,
                },
                2 => TextLayout::Path {
                    curves: Vec::new(),
                    side: PathSide::Above,
                    offset: 0.0,
                    fit: false,
                    reversed: false,
                },
                _ => TextLayout::Point,
            };
            true
        }
        "font" if index < fonts.len() => {
            spec.font = fonts[index].0.to_string();
            spec.font_asset = None;
            *mode = None;
            true
        }
        "font" => {
            *mode = Some(if index == fonts.len() {
                INSTALLED
            } else {
                FILE
            });
            false
        }
        "family" => match crate::text::installed_families().get(index) {
            Some(name) => {
                spec.font = name.clone();
                spec.font_asset = None;
                true
            }
            None => false,
        },
        _ => false,
    }
}

#[cfg(all(test, feature = "egui"))]
mod text_controls {
    use super::*;
    use crate::text::{TextLayout, TextSpec};

    /// Every layout's controls name icons the design set has, and a click
    /// on one sets what it shows lit.
    #[test]
    fn text_controls_show_known_icons_and_set_what_they_light() {
        let mut spec = TextSpec::default();
        for layout in 0..3 {
            assert!(text_button(&mut spec, &format!("layout:{layout}")));
            for widget in text_fields("text_draft", &spec, None) {
                if let Widget::Toggles(toggles) = widget {
                    for button in &toggles.buttons {
                        assert!(ui_kit::icon::exists(&button.icon), "{}", button.icon);
                    }
                }
            }
        }
        assert!(matches!(spec.layout, TextLayout::Path { .. }));
        assert!(text_button(&mut spec, "align:1"));
        assert!(text_button(&mut spec, "reversed"));
        assert!(text_button(&mut spec, "side:2"));
        let lit: Vec<String> = text_fields("text_draft", &spec, None)
            .into_iter()
            .filter_map(|w| match w {
                Widget::Toggles(t) => Some(t.buttons),
                _ => None,
            })
            .flatten()
            .filter(|b| b.on)
            .map(|b| b.id)
            .collect();
        assert_eq!(
            lit,
            [
                "text_draft.layout:2",
                "text_draft.align:1",
                "text_draft.side:2",
                "text_draft.reversed"
            ]
        );
    }
}
