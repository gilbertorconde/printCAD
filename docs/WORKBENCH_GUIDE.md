# Creating Workbenches for printCAD

This guide explains how to create custom workbenches for printCAD. Workbenches are modular plugins that extend the application with new tools, commands, and UI panels.

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
    CommandDescriptor, InputResult, ToolDescriptor, ToolKind, Workbench,
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
        // Register tools
        context.register_tool(ToolDescriptor::new(
            "my.tool1",
            "Tool One",
            ToolKind::Utility,
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
    fn ui_left_panel(&mut self, ui: &mut egui::Ui, ctx: &WorkbenchRuntimeContext) {}

    #[cfg(feature = "egui")]
    fn ui_right_panel(&mut self, ui: &mut egui::Ui, ctx: &WorkbenchRuntimeContext) {}

    #[cfg(feature = "egui")]
    fn ui_settings(&mut self, ui: &mut egui::Ui) -> bool { false }
}
```

---

## Registering Tools and Commands

### Tools

Tools are interactive operations that respond to user input. Register them in `configure()`:

```rust
fn configure(&self, context: &mut WorkbenchContext) {
    context.register_tool(ToolDescriptor::new(
        "sketch.line",      // Unique tool ID
        "Line",             // Display label
        ToolKind::Sketch,   // Category
    ));
}
```

**Tool Kinds:**
- `ToolKind::Sketch` - 2D sketching tools
- `ToolKind::PartDesign` - 3D modeling tools
- `ToolKind::Utility` - General utilities

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

    /// Cursor position in viewport coordinates
    pub cursor_viewport_pos: Option<(f32, f32)>,
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

```rust
#[cfg(feature = "egui")]
fn ui_right_panel(&mut self, ui: &mut egui::Ui, ctx: &WorkbenchRuntimeContext) {
    ui.heading("Properties");
    
    if let Some(body_id) = ctx.selected_body_id {
        ui.label(format!("Selected: {:?}", body_id));
    } else {
        ui.label("Nothing selected");
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
    ToolKind, Workbench, WorkbenchContext, WorkbenchDescriptor,
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
            ToolKind::Utility,
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

| Type | Description |
|------|-------------|
| `Workbench` | Main trait for workbench plugins |
| `WorkbenchDescriptor` | Metadata (id, label, description) |
| `WorkbenchContext` | Registration context for tools/commands |
| `WorkbenchRuntimeContext` | Runtime access to app state |
| `ToolDescriptor` | Tool metadata (id, label, kind) |
| `CommandDescriptor` | Command metadata (id, label) |
| `WorkbenchInputEvent` | Input event types |
| `InputResult` | Input handling result |

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

