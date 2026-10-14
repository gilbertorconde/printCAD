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
| Arrays | `Vec<CopyArray>` | A body or a whole component repeated in a row, a ring, along a path or at a sketch's points, each repeat a linked copy the array owns, with the occurrences it skips |
| Workbench storage | `WorkbenchId -> JSON` | Data a workbench keeps outside features |
| Assets | `AssetReference` | Files kept verbatim: what an import or a replaced shape came from, sketch and texture pictures, kept fonts |
| Geometry | `ImportedGeometry` per body | The mesh, and for solids the kernel shape |
| Base solids | `ImportedGeometry` per body | The shape a body's features start from |
| Imported structure | `ImportedObjectNode` | An import's assemblies, parts, annotations and layers |
| Display unit, measure unit, agent rules | `Unit`, `Option<Unit>`, `String` | How lengths are shown; the unit the measure tool reads in, when chosen; what an AI agent working on it follows |
| Version stamp | `FileStamp` (`written_by`) | The container's format and the app version that wrote the file it was read from; not an op, a save writes the running one |

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
body or by their own eye, and go to STEP as drawn callouts carrying
their text, each naming the face it was put on, so a reader reads it as
a note on that face of that part. Every change is one `SetBodyNotes` op holding all of the
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
- **Some operations cannot be undone.** An import, a new asset (such as a
  picture or a kept font), a shape repair, a mesh conversion or refine, a
  replaced shape and deleting a body that holds anything clear the undo
  history. Deleting a body as it was made, with nothing on it, undoes.

## The file

A `.prtcad` file is a tar archive. It can be compressed with gzip
(`.prtcad.gz`) or zstd (`.prtcad.zst`).

```
thumbnail.png        Preview of the model, first so it reads quickly
meta.json            What wrote the file: {format, app_version}
document.json        Metadata, features, bodies
assets/<name>.<ext>  Kept files: import sources, pictures, fonts
mesh/<id>.bin        Each body's mesh, MessagePack
mesh/<id>.base.bin   A body's base solid's mesh
brep/<id>.bin        Each body's kernel shape, ogeom native text
brep/<id>.colors     Each body's face colours
brep/<id>.base.bin   A body's base solid, and .base.colors its face colours
```

`document.json` holds an empty mesh where each body's mesh goes; the mesh
is its own entry. A file whose JSON still holds its meshes opens too.
Sketch numbers are stored as `f64`, so a solved sketch reads back exactly.

A document with an import can be hundreds of megabytes, so saving and
opening run on a background thread.

New fields on saved types take `#[serde(default)]`, so older files keep
opening.

### Older and newer versions

`meta.json` is a `FileStamp`: the container's format number
(`CONTAINER_FORMAT`) and the app version that saved it. A file without it
was written before 0.9.0. A file from a newer version (a larger format, or
a later version of the same format) opens with a warning and a card
offering Save As, since saving over it may lose what this version does
not know. Saving over a file from another version asks first; for an
older one only while Preferences › General › Files says to.

A file saved by 0.9.0 opens in 0.8.1 without a warning, since 0.8.1 does
not read the stamp. There, feature kinds 0.9.0 adds do not build, options
it adds to existing features are passed over, and notes, arrays and the
measure unit are lost if the file is saved again. Files from 0.8.1 and
earlier open in 0.9.0 as before.

### Not in the file

- Each file's view, the open tabs and the window and panel sizes are kept
  in the application's own folder (`views.json`, `session.json`), never
  in the document: a view change is not an edit.
- A linked copy's shape and mesh are its source's, and a part linked
  from another file is read from that file; both are derived again on
  load.
- Cosmetic threads are worked out from the features that declare them on
  every evaluation.

## The document server

The application does not write the file itself. Each document has a server
process, `printcad-serverd`, reached over a local socket. The application
sends it the saved bytes and every operation. The server stores them without
reading them: the file, and the operation log beside it
(`<file>.oplog.jsonl`).

If the server cannot start, the application writes the file directly and
logs a warning.
