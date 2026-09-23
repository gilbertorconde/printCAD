# Document model

A `Document` (`crates/core_document`) holds everything a `.prtcad` file
stores. It knows no workbench: features are stored as JSON, and each
workbench reads and writes its own.

## What a document holds

| Part | Type | Notes |
| --- | --- | --- |
| Metadata | `DocumentMetadata` | Id, name, revision |
| Features | `FeatureTree` | Every feature, with its dependencies |
| Bodies | `Vec<Body>` | Id, name, tip, display colour, hidden flag |
| Workbench storage | `WorkbenchId -> JSON` | Data a workbench keeps outside features |
| Assets | `AssetReference` | Files an import came from, kept verbatim |
| Geometry | `ImportedGeometry` per body | The mesh, and for solids the kernel shape |

## Features

A feature is a `FeatureNode`:

- `id`, `name`, and the `body` it belongs to
- `workbench_id`: the kind, which says which workbench owns it
- `visible`, `suppressed`, `dirty`, and the last rebuild `error`
- `seq`: its place in the build history. Always order history by `seq`,
  never by `created_at`.
- `data`: the feature itself, as JSON

A workbench defines a feature type by implementing `WorkbenchFeature`:

```rust
pub trait WorkbenchFeature {
    fn workbench_id() -> WorkbenchId;
    fn to_json(&self) -> serde_json::Value;
    fn from_json(value: &serde_json::Value) -> DocumentResult<Self>;
    fn dependencies(&self) -> Vec<FeatureId>;
    fn name(&self) -> &str;
}
```

It adds one with `add_feature_in_body`, and changes it with
`update_feature_data`. The dependencies it declares decide what is marked
dirty when a feature changes.

## Bodies and geometry

A body is a name and a place in the tree. Its solid is not stored in the
feature tree. It is derived:

- A Part Design body is rebuilt from its features by the kernel.
- An imported body keeps the kernel shape it was read with.
- A mesh body (from STL, OBJ or 3MF) has triangles only, until it is
  converted to a solid.

The result lands in `ImportedGeometry`: an `Arc<TriMesh>` for drawing, a
`revision` the renderer uses to know when to upload again, the bounds, and
the shape's health check. The kernel shape itself is kept beside it as
ogeom native text.

## Edits, undo and replay

- **Every edit records exactly one operation.** Each method that changes the
  document builds a `DocumentOp`, applies it and records it. Replaying the
  operations rebuilds the same document.
- **Derived state records nothing.** Dirty flags, rebuild errors, meshes and
  the preview image are consequences of the operations, not history.
- **Undo applies inverse operations.** Each edit computes its inverse before
  it applies. One gesture, such as a drag or one task in the task panel, is
  one undo step.
- **Some operations cannot be undone.** An import or a new asset clears the
  undo history.

## The file

A `.prtcad` file is a tar archive. It can be compressed with gzip
(`.prtcad.gz`) or zstd (`.prtcad.zst`).

```
thumbnail.png        Preview of the model, first so it reads quickly
document.json        Metadata, features, bodies, meshes
assets/<id>.<ext>    The files imports came from
brep/<id>.bin        Each body's kernel shape, ogeom native text
brep/<id>.colors     Each body's face colours
```

A document with an import can be hundreds of megabytes, so saving and
opening run on a background thread.

New fields on saved types take `#[serde(default)]`, so older files keep
opening.

## The document server

The application does not write the file itself. Each document has a server
process, `printcad-serverd`, reached over a Unix socket. The application
sends it the saved bytes and every operation. The server stores them without
reading them: the file, and the operation log beside it
(`<file>.oplog.jsonl`).

If the server cannot start, the application writes the file directly and
says so in the status bar.
