# Sketch interaction runtime

`sketch_runtime` resolves geometric picks, snaps, gestures, previews and
construction plans. It depends on geometry and units. The application owns its
document, constraint solver, draft sessions and undo history.

Populate `scene::PickScene` with authoritative geometry. Retain each
`geometry::SourceId` when an element moves or its storage order changes.
Synchronize after applying or solving edits, before refreshing a continuing
gesture. The scene caches exact carriers; rational poles stay private and
intersection picks retain both source addresses.

`pick::Pick` carries a position and addressed roles. `binding::for_snap` resolves
membership requirements. `placement::Placement` combines explicit positions,
point snaps and grid precedence. Numeric fields and formulas belong to the
application.

`point`, `circle`, `line`, `rect` and `conic` own their gesture state. Preview and
application use the same resolved geometry. Validate every source, apply a plan
atomically, then acknowledge the actual created or reused ids through `Applied`.
A rejection restores pending input. Rectangle placement and handle drags use one
state machine; a solved document draft can accompany it independently.

`LineHistoryId` is an opaque receipt for a closed gesture. Preserve it through
redo and never reuse it for another edit. Backspace checks the newest receipt
before undo; `undone_in_history` restores the input phase against refreshed
geometry. A held script can acknowledge without a receipt per link.

Replay addressed picks without picking again. Coordinate recordings retain their
thresholds and input conventions, including focus versus branch-vertex hyperbola
input. Runtime addresses are independent of the document's file format.

Run `cargo test -p sketch_runtime`. Application tests cover solving, task drafts,
undo/redo and replay through viewport input.
