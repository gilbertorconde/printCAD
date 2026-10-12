# Document model

A `Document` (`crates/core_document`) holds everything a `.prtcad` file
stores. It knows no workbench: features are stored as JSON, and each
workbench reads and writes its own.

## What a document holds

| Part | Type | Notes |
| --- | --- | --- |
| Metadata | `DocumentMetadata` | Id, name, revision |
| Features | `FeatureTree` | Every feature, with its dependencies |
| Bodies | `Vec<Body>` | Id, name, tip, placement, display colour, material, face colours and textures, notes, hidden and frozen flags, linked copy or file link |
| Components | `Vec<Component>` | Groups of bodies, nested, that move as one |
| Arrays | `Vec<CopyArray>` | A body repeated in a row or a ring, each repeat a linked copy the array owns |
| Workbench storage | `WorkbenchId -> JSON` | Data a workbench keeps outside features |
| Assets | `AssetReference` | Files an import came from, kept verbatim |
| Geometry | `ImportedGeometry` per body | The mesh, and for solids the kernel shape |
| Base solids | `ImportedGeometry` per body | The shape a body's features start from |
| Imported structure | `ImportedObjectNode` | An import's assemblies, parts, annotations and layers |
| Display unit, agent rules | `Unit`, `String` | How lengths are shown; what an AI agent working on it follows |

## Features

A feature is a `FeatureNode`:

- `id`, `name`, and the `body` it belongs to
- `workbench_id`: the kind, which says which workbench owns it
- `visible`, `suppressed`, `dirty`, and the last rebuild `error`
- `seq`: its place in the build history. Always order history by `seq`,
  never by `created_at`.
- `data`: the feature itself, as JSON
- `formulas`: formulas setting its numbers, by parameter key
- `made_by` and `package_source`: the workbench package that last wrote
  it, and where that package came from

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

- A Design body is rebuilt from its features by the kernel.
- An imported body keeps the kernel shape it was read with. Given
  features, that shape becomes the body's base solid (op `SetBodyBase`),
  kept apart from what the features build and saved as
  `brep/<body>.base.bin`; its history starts with a Base feature built
  from it.
- A mesh body (from STL, OBJ, 3MF, PLY, glTF or VRML) has triangles only, until it is
  converted to a solid (op `RequestMeshSolid`). The conversion keeps curved
  stretches as facets; a refine (op `RequestBodyRefine`, offered while the
  body has no features) rebuilds them on the surfaces they approximate.

The result lands in `ImportedGeometry`: an `Arc<TriMesh>` for drawing, a
`revision` the renderer uses to know when to upload again, the bounds, and
the shape's health check. The kernel shape itself is kept beside it as
ogeom native text.

## Imported structure, annotations and layers

An import's tree (assemblies, instances, parts) is kept as
`ImportedObjectNode`s, written by the import's one op and saved with the
document. Two things a STEP or IGES file carries ride on those nodes:

- **Annotations.** The file's dimensions, geometric tolerances, datums and
  notes sit in an `Annotations` group under the imported model, one
  `Annotation` node each: its kind, the text a label shows (`Ø 35 ±0.2`,
  `Position 0.75 | A`), the polylines the file draws it with and where
  its label goes, in the frame of the body it describes. They are drawn
  over the scene where that body is placed and hide with it; View ›
  Annotations turns them all off, and each row's eye hides one.
- **Layers.** A part node lists the layers the file puts its body on, by
  name (an IGES level reads as `level 7`). The property panel shows them.

They come from the file, so a reload does not re-derive them.

## Notes

A note is text the user pins to a point on a body (Add note on a body's
menu, or `pc.doc.note_add`): a leader from the point to a label. It is a
`Note` on the `Body`, its point and label in the body's own frame, so it
moves with the body, and it names the face its point is on: when a
rebuild moves that face, the point goes to the nearest point of the face
found by its name, and the label with it. A text starting with `=` is a
formula, and the label shows what it comes to (`=Pad.length`). A body's
notes are listed in an `Annotations` group under its row, drawn with the
imported annotations (View › Annotations turns both off), hide with the
body or by their own eye, and go to STEP as drawn callouts named by
their text. Every change is one `SetBodyNotes` op holding all of the
body's notes.

## Edits, undo and replay

- **Every edit records exactly one operation.** Each method that changes the
  document builds a `DocumentOp`, applies it and records it. Replaying the
  operations rebuilds the same document.
- **Derived state records nothing.** Dirty flags, rebuild errors, meshes and
  the preview image are consequences of the operations, not history.
- **Undo applies inverse operations.** Each edit computes its inverse before
  it applies. One gesture, such as a drag or one task in the task panel, is
  one undo step.
- **Some operations cannot be undone.** An import, a new asset, a shape
  repair, a mesh conversion or refine and a replaced shape clear the undo
  history.

## The file

A `.prtcad` file is a tar archive. It can be compressed with gzip
(`.prtcad.gz`) or zstd (`.prtcad.zst`).

```
thumbnail.png        Preview of the model, first so it reads quickly
document.json        Metadata, features, bodies, meshes
assets/<id>.<ext>    The files imports came from
brep/<id>.bin        Each body's kernel shape, ogeom native text
brep/<id>.colors     Each body's face colours
brep/<id>.base.bin   A body's base solid, and .base.colors its face colours
```

A document with an import can be hundreds of megabytes, so saving and
opening run on a background thread.

New fields on saved types take `#[serde(default)]`, so older files keep
opening.

## The document server

The application does not write the file itself. Each document has a server
process, `printcad-serverd`, reached over a local socket. The application
sends it the saved bytes and every operation. The server stores them without
reading them: the file, and the operation log beside it
(`<file>.oplog.jsonl`).

If the server cannot start, the application writes the file directly and
logs a warning.
