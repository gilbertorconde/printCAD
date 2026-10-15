# project_archive

Standalone `.FCStd` ZIP/XML reader and source sketch decoder. It has no dependency
on an application document, UI, solver, geometry kernel or system CAD library.
The manifest declares its own package settings and dependency versions; the
directory can be copied and built outside its host workspace.

```rust
use project_archive::archive::FcstdArchive;
use project_archive::document_xml::FcDocument;
use project_archive::sketch_decode::FcSketch;

let archive = FcstdArchive::from_bytes(bytes)?;
let source = FcDocument::parse(&archive.document_xml)?;
let sketch = FcSketch::decode(source.object("Sketch").unwrap())?;
```

`coordinates` exposes source points in millimetres and quaternions as `f64`
values and arrays. The consumer chooses its math library, storage precision and
conversion policy. The math dependency is internal to placement composition.
`traversal` provides a bounded pending path and a parent-frame map; the consumer
owns dependency selection, resolution caching and object conversion.

Typed numeric access rejects NaN and infinity with an object and field name.
Placements require a nonzero, finite quaternion norm within 1e-9 of unit length;
only decimal roundoff is normalized. `Placement::compose` returns a named error
on invalid rotations or a nonfinite composed translation. Sketch decoding also
checks computed arc angles/endpoints. Optional local Z and normal X/Y components,
when present, must be finite and within 1e-9 of zero. Application frame validation
and storage-range restrictions belong to adapters.
An absent arc `AngleXU` selects zero rotation of the circle's reference frame;
an explicit value is validated as a finite number. The reference X direction is
positive X for a positive Z normal and negative X for a negative Z normal.
Sketch endpoint addressing swaps the parameter endpoints for a negative normal.

Budgets are inclusive: 256 MiB container, 16 MiB XML, 128 MiB per entry,
512 MiB declared total and 10,000 entries. Ignored entries count towards the
declared budget. XML and shapes also charge actual decompressed bytes.
`ArchiveLimits` can be supplied to `open_with_limits` / `from_bytes_with_limits`.
Direct XML parsing applies the default byte budget, one million parser nodes,
two million raw `<`/`=` allocation hints, and 128 element levels; DTDs are
refused. Object walks allow 128 pending names, including the starting object.
Cycles, missing objects and depth refusals contain the source-object chain.

`roxmltree` is the only XML parser. Each limit runs before the part of it that
needs bounding: the byte cap and the allocation-hint count first, since
`roxmltree` reserves its node and attribute arrays from the raw `<` and `=`
counts; then a single linear scan of the markup counts element nesting (stepping
over comments, CDATA, processing instructions and quoted attribute values, and
refusing a document type declaration), since `roxmltree` parses nested elements
by recursion and has no depth option of its own; then `roxmltree` parses with
its `nodes_limit`. The scan builds nothing and cannot recurse.

The format model preserves object/property trees and unsupported sketch type
names. It does not repair, solve or build them. Loss reports and choices about
unsupported application features belong to adapters.

MIT OR Apache-2.0; permission notices and license texts are bundled in this
directory. The host README's Inspiration section records the source inventory.
