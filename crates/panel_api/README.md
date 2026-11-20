# printcad-panel-api

The declared panels a printCAD workbench package and the app exchange: the
widgets a task panel or a card over the view is made of, the events the
app sends back when the user changes one, and the floating card. The
library is named `panel_api`.

Packages reach these types through
[`printcad-bench-api`](https://crates.io/crates/printcad-bench-api), which
re-exports every item at its own root, or through
[`printcad-bench-sdk`](https://crates.io/crates/printcad-bench-sdk). They
cross the component interface as JSON.

Versions follow `printcad-bench-api`'s: 0.1.x speaks
`printcad:workbench@0.1`, and a patch release only adds. See
[Versions](https://github.com/gilbertorconde/printCAD/blob/master/docs/PLUGINS.md#versions).

Licensed under MIT or Apache-2.0, at your option.
