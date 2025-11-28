# Creating Workbenches for printCAD

This guide explains how to create custom workbenches for printCAD. Workbenches are modular plugins that extend the application with new tools, commands, and UI panels.

## Architecture Note: Why is the Workbench Trait in `core_document`?

The `Workbench` trait is defined in the `core_document` crate, which may seem unusual at first glance. Here's why this organization makes sense:

1. **Tight Coupling to Document Model**: Workbenches are the primary mechanism for creating and managing document features. The `WorkbenchRuntimeContext` provides mutable access to the document, and workbenches directly call document methods like `add_feature()`, `update_feature_data()`, etc.

2. **Document Service Registry**: The `DocumentService` (which manages workbenches) lives in `core_document`. The registry needs to know about the `Workbench` trait to store and invoke workbenches.

3. **Feature Serialization**: The document needs to know about workbenches to properly serialize/deserialize features. Methods like `deserialize_feature()` and `feature_dependencies()` are part of the workbench interface.

4. **Workbench Storage**: The document stores workbench-specific data (`WorkbenchStorage`), creating a bidirectional relationship between documents and workbenches.

5. **Dependency Direction**: Currently, workbenches depend on `core_document` (not the other way around). This keeps the dependency graph clean: workbenches are extensions of the document system, not separate systems.

While the trait includes UI-related methods (`ui_left_panel`, `ui_right_panel`), these are optional and feature-gated. The core relationship is between workbenches and the document model, which justifies placing the trait in `core_document`.

## Overview

A workbench in printCAD is a self-contained module that provides:

- **Tools**: Interactive operations (e.g., Line, Circle, Pad, Fillet)
- **Commands**: Non-interactive actions (e.g., Solve Constraints, Recompute)
- **UI Panels**: Custom content in the left panel, right panel, and settings
- **Input Handling**: Mouse and keyboard event processing
- **Lifecycle Hooks**: Activation/deactivation callbacks

## Quick Start

### 1. Create a New Crate

```bash
cd crates
cargo new wb_my_workbench --lib
```

### 2. Configure `Cargo.toml`

```toml
[package]
name = "wb_my_workbench"
version = "0.1.0"
edition = "2024"

[features]
default = ["egui"]
egui = ["core_document/egui", "dep:egui"]

[dependencies]
core_document = { path = "../core_document" }
egui = { workspace = true, optional = true }
```

### 3. Implement the Workbench

```rust
use core_document::{
    CommandDescriptor, InputResult, ToolDescriptor, Workbench,
    WorkbenchContext, WorkbenchDescriptor, WorkbenchInputEvent,
    WorkbenchRuntimeContext,
};

/// My custom workbench.
#[derive(Default)]
pub struct MyWorkbench {
    // Your workbench state goes here
    counter: u32,
}

impl Workbench for MyWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            "wb.my-workbench",        // Unique ID (use "wb." prefix)
            "My Workbench",           // Display name
            "Description of my workbench.",
        )
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        // Register tools (radio button behavior by default)
        context.register_tool(ToolDescriptor::new(
            "my.tool1",
            "Tool One",
            Some("utility"),  // Optional category
        ));

        // Register commands
        context.register_command(CommandDescriptor::new(
            "my.do_something",
            "Do Something",
        ));
    }
}
```

### 4. Register the Workbench

In `app_shell/src/main.rs`:

```rust
use wb_my_workbench::MyWorkbench;

fn main() -> Result<()> {
    // ...
    let mut registry = DocumentService::default();
    registry.register_workbench(Box::new(MyWorkbench::default()))?;
    // ...
}
```

---

## The Workbench Trait

The `Workbench` trait is the core interface for all workbenches:

```rust
pub trait Workbench: Send {
    /// Returns metadata describing this workbench.
    fn descriptor(&self) -> WorkbenchDescriptor;

    /// Called once at registration to declare tools and commands.
    fn configure(&self, context: &mut WorkbenchContext);

    /// Called when this workbench becomes active.
    fn on_activate(&mut self, ctx: &mut WorkbenchRuntimeContext) {}

    /// Called when this workbench is deactivated.
    fn on_deactivate(&mut self, ctx: &mut WorkbenchRuntimeContext) {}

    /// Called every frame while this workbench is active.
    fn on_frame(&mut self, dt: f32, ctx: &mut WorkbenchRuntimeContext) {}

    /// Called when an input event occurs while this workbench is active.
    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        InputResult::ignored()
    }

    // UI hooks (require "egui" feature)
    #[cfg(feature = "egui")]
    fn ui_left_panel(&mut self, ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {}

    #[cfg(feature = "egui")]
    fn ui_right_panel(&mut self, ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {}

    /// Whether this workbench exposes right-panel UI.
    /// Called by the host to determine if the right panel should be shown.
    #[cfg(feature = "egui")]
    fn wants_right_panel(&self) -> bool {
        false
    }

    /// Check if a tool is enabled given the current runtime context.
    /// Called by the UI to determine if a tool button should be enabled/disabled.
    /// Default implementation returns true for all tools.
    fn is_tool_enabled(&self, _tool_id: &str, _ctx: &WorkbenchRuntimeContext) -> bool {
        true
    }

    #[cfg(feature = "egui")]
    fn ui_settings(&mut self, ui: &mut egui::Ui) -> bool { false }

    /// Finish/close the current editing session (e.g., finish sketch).
    /// Called when the user requests to finish editing (e.g., via UI button).
    fn finish_editing(&mut self, _ctx: &mut WorkbenchRuntimeContext) {}
}
```

---

## Registering Tools and Commands

### Tools

Tools are interactive operations that respond to user input. Register them in `configure()`:

```rust
fn configure(&self, context: &mut WorkbenchContext) {
    // Action tools (fire-and-forget buttons, not toggles)
    context.register_tool(ToolDescriptor::new_action(
        "sketch.create",    // Unique tool ID
        "Create Sketch",    // Display label
        Some("sketch"),     // Optional category for grouping
    ));

    // Regular tools (radio button behavior - only one active at a time)
    context.register_tool(ToolDescriptor::new(
        "sketch.line",      // Unique tool ID
        "Line",             // Display label
        Some("sketch"),     // Optional category for grouping
    ));
}
```

**Tool Behavior:**

- `ToolBehavior::Radio` (default) - Radio button behavior: only one tool can be active at a time. Clicking an active tool deactivates it. This is the default for `ToolDescriptor::new()`.
- `ToolBehavior::Action` - Action button behavior: fire-and-forget. Clicking triggers the action but doesn't keep the tool "active". Use `ToolDescriptor::new_action()` to create action tools.

**Tool Categories:**

The `category` parameter is optional and purely informational. It can be used for grouping/organization (e.g., `"sketch"`, `"modeling"`, `"utility"`). It doesn't affect tool behavior - that's controlled by the `behavior` field.

### Commands

Commands are non-interactive actions (like menu items or shortcuts):

```rust
context.register_command(CommandDescriptor::new(
    "sketch.constraints.solve",
    "Solve Constraints",
));
```

---

## Handling Input Events

Implement `on_input()` to respond to mouse and keyboard events:

```rust
fn on_input(
    &mut self,
    event: &WorkbenchInputEvent,
    active_tool: Option<&str>,
    ctx: &mut WorkbenchRuntimeContext,
) -> InputResult {
    // Only handle events if one of our tools is active
    let tool = match active_tool {
        Some(t) if t.starts_with("my.") => t,
        _ => return InputResult::ignored(),
    };

    match event {
        WorkbenchInputEvent::MousePress { button, viewport_pos } => {
            if *button == MouseButton::Left {
                ctx.log_info(format!("Click at {:?}", viewport_pos));
                return InputResult::consumed();
            }
        }
        WorkbenchInputEvent::KeyPress { key } => {
            if *key == KeyCode::Escape {
                ctx.log_info("Cancelled");
                return InputResult::consumed();
            }
        }
        _ => {}
    }

    InputResult::ignored()
}
```

### Input Events

```rust
pub enum WorkbenchInputEvent {
    MousePress { button: MouseButton, viewport_pos: (f32, f32) },
    MouseRelease { button: MouseButton, viewport_pos: (f32, f32) },
    MouseMove { viewport_pos: (f32, f32) },
    KeyPress { key: KeyCode },
    KeyRelease { key: KeyCode },
}
```

### Input Results

```rust
InputResult::consumed()    // Event handled, stop propagation
InputResult::ignored()     // Event not handled, continue propagation
InputResult::redraw_only() // Request redraw but don't consume event
```

---

## The Runtime Context

`WorkbenchRuntimeContext` provides access to application state:

```rust
pub struct WorkbenchRuntimeContext<'a> {
    /// The active document (mutable access for edits)
    pub document: &'a mut Document,

    /// Current camera position in world space
    pub camera_position: [f32; 3],

    /// Current camera target (orbit center) in world space
    pub camera_target: [f32; 3],

    /// Viewport dimensions (x, y, width, height) in pixels
    pub viewport: (u32, u32, u32, u32),

    /// World position under the cursor (if hovering geometry)
    pub hovered_world_pos: Option<[f32; 3]>,

    /// ID of the body under the cursor
    pub hovered_body_id: Option<Uuid>,

    /// ID of the currently selected body
    pub selected_body_id: Option<Uuid>,

    /// Active document object (selected feature in tree - separate from editing mode)
    pub active_document_object: Option<FeatureId>,

    /// Cursor position in viewport coordinates
    pub cursor_viewport_pos: Option<(f32, f32)>,

    /// Request camera orientation to a plane (set by workbench, read by host)
    pub camera_orient_request: Option<CameraOrientRequest>,

    /// Request to exit sketch mode (set by workbench UI, read by host)
    pub finish_sketch_requested: bool,
}
```

### Logging

Use the context to log messages to the in-app log panel:

```rust
ctx.log_info("Operation completed");
ctx.log_warn("Something might be wrong");
ctx.log_error("Operation failed!");
```

### Document Access

Modify the document through the context:

```rust
ctx.document.mark_dirty();  // Mark document as modified
```

---

## Feature Management

The document provides a **generic, extensible feature tree** that allows workbenches to define their own feature types. Features are stored in a directed acyclic graph (DAG) with dependency tracking.

### Defining Feature Types

To create a feature type, implement the `WorkbenchFeature` trait:

```rust
use core_document::{FeatureId, WorkbenchFeature, WorkbenchId, DocumentResult, FeatureError};
use serde::{Serialize, Deserialize};
use serde_json;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyFeature {
    pub name: String,
    pub value: f32,
    pub depends_on: Option<FeatureId>, // Optional dependency
}

impl WorkbenchFeature for MyFeature {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from("wb.my-workbench")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap()
    }

    fn from_json(value: &serde_json::Value) -> DocumentResult<Self> {
        serde_json::from_value(value.clone())
            .map_err(|e| FeatureError::Deserialization(e.to_string()))
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        self.depends_on.into_iter().collect()
    }

    fn name(&self) -> &str {
        &self.name
    }
}
```

### Adding Features to the Document

Use the runtime context to add features:

```rust
fn on_input(
    &mut self,
    event: &WorkbenchInputEvent,
    active_tool: Option<&str>,
    ctx: &mut WorkbenchRuntimeContext,
) -> InputResult {
    match event {
        WorkbenchInputEvent::MousePress { .. } => {
            let feature = MyFeature {
                name: "My Feature".to_string(),
                value: 42.0,
                depends_on: None,
            };

            match ctx.document.add_feature(feature, "My Feature".to_string()) {
                Ok(feature_id) => {
                    ctx.log_info(format!("Created feature: {:?}", feature_id));
                    ctx.document.mark_dirty();
                    InputResult::consumed()
                }
                Err(e) => {
                    ctx.log_error(format!("Failed to create feature: {}", e));
                    InputResult::consumed()
                }
            }
        }
        _ => InputResult::ignored(),
    }
}
```

### Reading Features

Retrieve feature data from the document:

```rust
// Get feature data as JSON
if let Some(data) = ctx.document.get_feature_data(feature_id) {
    // Deserialize to your feature type
    if let Ok(feature) = MyFeature::from_json(data) {
        ctx.log_info(format!("Feature value: {}", feature.value));
    }
}

// Get feature metadata (name, dirty flag, etc.)
if let Some(meta) = ctx.document.get_feature_meta(feature_id) {
    ctx.log_info(format!("Feature name: {}, dirty: {}", meta.name, meta.dirty));
}
```

### Updating Features

Update feature data:

```rust
let updated_feature = MyFeature {
    name: "Updated Feature".to_string(),
    value: 100.0,
    depends_on: None,
};

if let Err(e) = ctx.document.update_feature_data(feature_id, updated_feature.to_json()) {
    ctx.log_error(format!("Failed to update feature: {}", e));
}
```

### Feature Dependencies

Features can depend on other features. The document automatically tracks dependencies:

```rust
// Create a dependent feature
let dependent = MyFeature {
    name: "Dependent".to_string(),
    value: 10.0,
    depends_on: Some(base_feature_id), // Depends on base feature
};

let dependent_id = ctx.document.add_feature(dependent, "Dependent".to_string())?;

// When base feature is marked dirty, dependent is automatically marked dirty too
ctx.document.mark_feature_dirty(base_feature_id);
// dependent_id is now also dirty
```

### Workbench Storage

Store additional workbench-specific data outside the feature tree:

```rust
// Store workbench data
let data = serde_json::json!({
    "settings": {
        "grid_snap": true,
        "snap_distance": 1.0,
    }
});

ctx.document.set_workbench_storage(
    WorkbenchId::from("wb.my-workbench"),
    data,
);

// Retrieve workbench data
if let Some(storage) = ctx.document.get_workbench_storage(&WorkbenchId::from("wb.my-workbench")) {
    // Use storage.data (serde_json::Value)
}
```

### Feature API Summary

| Method                                            | Description                                |
| ------------------------------------------------- | ------------------------------------------ |
| `add_feature<F: WorkbenchFeature>(feature, name)` | Add a feature to the document              |
| `get_feature_data(id)`                            | Get feature data as JSON                   |
| `get_feature_meta(id)`                            | Get feature metadata (name, dirty, etc.)   |
| `update_feature_data(id, data)`                   | Update feature data                        |
| `mark_feature_dirty(id)`                          | Mark feature and dependents as dirty       |
| `dirty_features()`                                | Get all dirty features                     |
| `recompute_order()`                               | Get recomputation order (topological sort) |
| `get_workbench_storage(wb_id)`                    | Get workbench-specific storage             |
| `set_workbench_storage(wb_id, data)`              | Set workbench-specific storage             |

---

## Lifecycle Hooks

### Activation/Deactivation

```rust
fn on_activate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
    ctx.log_info("My workbench activated");
    // Initialize state, load resources, etc.
}

fn on_deactivate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
    ctx.log_info("My workbench deactivated");
    // Clean up state, save temporary data, etc.
}
```

### Per-Frame Updates

```rust
fn on_frame(&mut self, dt: f32, ctx: &mut WorkbenchRuntimeContext) {
    // dt is the time since the last frame in seconds
    // Use for animations, continuous updates, etc.
}
```

---

## Custom UI Panels

Implement UI hooks to add custom content to the application panels:

### Left Panel (Tool Info)

```rust
#[cfg(feature = "egui")]
fn ui_left_panel(&mut self, ui: &mut egui::Ui, ctx: &WorkbenchRuntimeContext) {
    ui.separator();
    ui.heading("My Workbench Info");
    ui.label(format!("Counter: {}", self.counter));

    if ui.button("Increment").clicked() {
        self.counter += 1;
    }
}
```

### Right Panel (Properties)

The right panel is shown only when `wants_right_panel()` returns `true`. This allows workbenches to dynamically show/hide the panel based on their state:

```rust
#[cfg(feature = "egui")]
fn wants_right_panel(&self) -> bool {
    // Only show right panel when editing a sketch
    self.active_sketch_id.is_some()
}

#[cfg(feature = "egui")]
fn ui_right_panel(&mut self, ui: &mut egui::Ui, ctx: &mut WorkbenchRuntimeContext) {
    ui.heading("Properties");

    if let Some(body_id) = ctx.selected_body_id {
        ui.label(format!("Selected: {:?}", body_id));
    } else {
        ui.label("Nothing selected");
    }

    // Request to exit editing mode
    if ui.button("Exit Sketch Mode").clicked() {
        ctx.finish_sketch_requested = true;
    }
}
```

### Settings Panel

```rust
#[cfg(feature = "egui")]
fn ui_settings(&mut self, ui: &mut egui::Ui) -> bool {
    let mut changed = false;

    ui.heading("My Workbench Settings");

    // Return true if any settings were modified
    changed
}
```

---

## Complete Example

Here's a complete example of a minimal workbench:

```rust
use core_document::{
    CommandDescriptor, InputResult, MouseButton, KeyCode, ToolDescriptor,
    Workbench, WorkbenchContext, WorkbenchDescriptor,
    WorkbenchInputEvent, WorkbenchRuntimeContext,
};

#[derive(Default)]
pub struct CounterWorkbench {
    click_count: u32,
}

impl Workbench for CounterWorkbench {
    fn descriptor(&self) -> WorkbenchDescriptor {
        WorkbenchDescriptor::new(
            "wb.counter",
            "Counter",
            "A simple counter workbench for demonstration.",
        )
    }

    fn configure(&self, context: &mut WorkbenchContext) {
        context.register_tool(ToolDescriptor::new(
            "counter.click",
            "Click Counter",
            Some("utility"),
        ));
        context.register_command(CommandDescriptor::new(
            "counter.reset",
            "Reset Counter",
        ));
    }

    fn on_activate(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        ctx.log_info("Counter workbench activated");
    }

    fn on_input(
        &mut self,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        // Only handle if our tool is active
        if active_tool != Some("counter.click") {
            return InputResult::ignored();
        }

        match event {
            WorkbenchInputEvent::MousePress {
                button: MouseButton::Left,
                viewport_pos,
            } => {
                self.click_count += 1;
                ctx.log_info(format!(
                    "Click #{} at ({:.0}, {:.0})",
                    self.click_count, viewport_pos.0, viewport_pos.1
                ));
                InputResult::consumed()
            }
            WorkbenchInputEvent::KeyPress { key: KeyCode::R } => {
                self.click_count = 0;
                ctx.log_info("Counter reset");
                InputResult::consumed()
            }
            _ => InputResult::ignored(),
        }
    }

    #[cfg(feature = "egui")]
    fn ui_left_panel(&mut self, ui: &mut egui::Ui, _ctx: &WorkbenchRuntimeContext) {
        ui.separator();
        ui.heading("Counter");
        ui.label(format!("Clicks: {}", self.click_count));
    }
}
```

---

## Best Practices

1. **Use unique IDs**: Prefix your workbench ID with `wb.` and tool IDs with your workbench prefix (e.g., `my.tool1`).

2. **Handle only your tools**: In `on_input()`, check if the active tool belongs to your workbench before processing.

3. **Return appropriate InputResults**: Use `consumed()` when you handle an event, `ignored()` otherwise.

4. **Log user actions**: Use `ctx.log_info()` to provide feedback in the log panel.

5. **Keep state in your struct**: Store tool state, temporary geometry, and configuration in your workbench struct.

6. **Clean up on deactivation**: Use `on_deactivate()` to clean up temporary state.

7. **Use the egui feature flag**: Wrap UI methods with `#[cfg(feature = "egui")]` to allow building without UI.

---

## API Reference

### Types

| Type                      | Description                             |
| ------------------------- | --------------------------------------- |
| `Workbench`               | Main trait for workbench plugins        |
| `WorkbenchDescriptor`     | Metadata (id, label, description)       |
| `WorkbenchContext`        | Registration context for tools/commands |
| `WorkbenchRuntimeContext` | Runtime access to app state             |
| `ToolDescriptor`          | Tool metadata (id, label, kind)         |
| `CommandDescriptor`       | Command metadata (id, label)            |
| `WorkbenchInputEvent`     | Input event types                       |
| `InputResult`             | Input handling result                   |

### Key Codes

Common key codes available in `KeyCode`:

- Letters: `A` through `Z`
- Numbers: `Key0` through `Key9`
- Function keys: `F1` through `F12`
- Special: `Escape`, `Enter`, `Space`, `Delete`, `Backspace`, `Tab`
- Modifiers: `Shift`, `Control`, `Alt`

### Mouse Buttons

```rust
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Other(u16),
}
```
