//! Small constructors for the panel's widgets: sections, grids, actions,
//! checks and the quantities of each unit.

use super::*;

pub(super) fn section(
    id: &str,
    title: &str,
    count: Option<usize>,
    summary: Option<String>,
    open: bool,
    children: Vec<Widget>,
) -> Widget {
    PanelSection {
        id: id.into(),
        title: title.into(),
        count,
        summary,
        open,
        children,
        ..Default::default()
    }
    .into()
}

/// The rows of the item list `id` among `widgets` and their sections,
/// taken out of it.
pub(super) fn take_items(widgets: &mut [Widget], id: &str) -> Vec<Item> {
    for widget in widgets {
        match widget {
            Widget::Items(list) if list.id == id => return std::mem::take(&mut list.items),
            Widget::Section(section) => {
                let items = take_items(&mut section.children, id);
                if !items.is_empty() {
                    return items;
                }
            }
            _ => {}
        }
    }
    Vec::new()
}

pub(super) fn grid(id: &str, roomy: bool, children: Vec<Widget>) -> Widget {
    FieldGrid {
        id: id.into(),
        children,
        roomy,
        ..Default::default()
    }
    .into()
}

pub(super) fn action(id: &str, label: &str, hint: &str, separated: bool) -> PanelAction {
    PanelAction {
        id: id.into(),
        label: label.into(),
        hint: hint.into(),
        separated,
        ..Default::default()
    }
}

pub(super) fn check(
    id: &str,
    label: &str,
    text: &str,
    on: bool,
    enabled: bool,
    hint: &str,
) -> Widget {
    Check {
        id: id.into(),
        label: label.into(),
        text: text.into(),
        on,
        enabled,
        hint: hint.into(),
    }
    .into()
}

/// The fillet and chamfer tools' switch that keeps the corner.
pub(super) fn corner_check(on: bool) -> Widget {
    check(
        "tool.corner_keep",
        "Corner",
        "Keep it",
        on,
        true,
        "The corner stays as a construction point on both curves, with its constraints",
    )
}

/// One of `options`, `width` wide.
pub(super) fn choose<T: PartialEq>(
    id: &str,
    label: &str,
    options: &[(T, &str)],
    now: T,
    width: f32,
) -> Widget {
    Select {
        id: id.into(),
        label: label.into(),
        options: options.iter().map(|(_, o)| o.to_string()).collect(),
        selected: options
            .iter()
            .position(|(v, _)| *v == now)
            .unwrap_or(usize::MAX),
        width: Some(width),
        ..Default::default()
    }
    .into()
}

/// A whole number from `min` to `max`.
pub(super) fn integer(id: &str, label: &str, value: u32, min: f64, max: f64) -> Quantity {
    Quantity {
        id: id.into(),
        label: label.into(),
        value: f64::from(value),
        dim: Dim::Number,
        min: Some(min),
        max: Some(max),
        decimals: 0,
        speed: Some(0.1),
        ..Default::default()
    }
}

pub(super) fn count(id: &str, label: &str, value: u32, min: f64, max: f64) -> Widget {
    integer(id, label, value, min, max).into()
}

/// A length, above zero.
pub(super) fn mm_quantity(id: &str, label: &str, value: f64) -> Quantity {
    Quantity {
        id: id.into(),
        label: label.into(),
        value,
        dim: Dim::Length,
        min: Some(0.001),
        max: Some(1.0e6),
        ..Default::default()
    }
}

pub(super) fn mm(id: &str, label: &str, value: f64) -> Widget {
    mm_quantity(id, label, value).into()
}

/// A position or a shift: either way along its axis.
pub(super) fn offset(id: &str, label: &str, value: f64) -> Widget {
    Quantity {
        min: Some(-1.0e6),
        max: Some(1.0e6),
        ..mm_quantity(id, label, value)
    }
    .into()
}

/// A length in `range`, or any.
pub(super) fn length(id: &str, label: &str, value: f64, range: Option<(f64, f64)>) -> Widget {
    Quantity {
        min: range.map(|r| r.0),
        max: range.map(|r| r.1),
        ..mm_quantity(id, label, value)
    }
    .into()
}

pub(super) fn degrees(id: &str, label: &str, value: f64) -> Widget {
    Quantity {
        id: id.into(),
        label: label.into(),
        value,
        dim: Dim::Angle,
        speed: Some(0.5),
        ..Default::default()
    }
    .into()
}

/// A whole number between a down and an up button, each with its state
/// and its words on hover.
pub(super) fn stepper(
    id: &str,
    label: &str,
    value: usize,
    text: &str,
    down: (bool, &str),
    up: (bool, &str),
) -> Widget {
    Quantity {
        id: id.into(),
        label: label.into(),
        value: value as f64,
        dim: Dim::Number,
        decimals: 0,
        stepper: Some(Stepper {
            text: text.into(),
            down: down.0,
            up: up.0,
            down_hint: down.1.into(),
            up_hint: up.1.into(),
        }),
        ..Default::default()
    }
    .into()
}
