# How faces and edges are named

A feature that picks a face or an edge (a sketch on a face, a pad up to a
face, a fillet's edges, a joint's faces) keeps the face's or edge's
**name** beside the point where it was picked. When the part rebuilds,
the face or edge is found by its name first, and by the point only when
the name finds nothing. This is what keeps a fillet on its edge when a
dimension earlier in the history changes, or when a feature goes in
before it.

This page is the rules those names follow, so a reference that lands on
the wrong face can be reported precisely. Scripts read names through
`doc.faces` and `doc.edges` (as strings: a name is a 64-bit number, more
than a script's numbers hold exactly).

## What a name is

A name is a 64-bit number made from what it names, the same on every
machine and in every version, because documents keep it
(`kernel_api::naming`). Zero is no name. A name is made from a parent
name and a part (`child(parent, part)`): a feature's name and the sketch
element a face was swept from, or a feature's name and the word `end`.

## The rules

1. **A feature names the faces it makes from what made them.** Each wall
   a sketch's segment sweeps is named after that segment (the sketch
   element it was drawn as, `ProfileWire::names`), so the walls keep
   their names while the sketch's dimensions change.
2. **A feature's ends are named by where they stand** along the sketch's
   normal: `start` on the sketch's plane, `end` past it, `back` behind it
   (a feature built both ways).
3. **Any other face a feature makes is named by its kind of surface and
   the way it faces**, ties among alike faces broken by where each lies.
   A box keeps its six names when it grows.
4. **A face an operation leaves as it was keeps its names**, however the
   kernel stores it afterwards: recognised by its kind of surface, its
   edges and vertices and where they lie.
5. **A face an operation splits, trims or merges takes the names of every
   face it came from.** The kernel's own record of the operation says
   which; where it says nothing, a piece is named after the face it lies
   on, and a merged face after every face it covers. One face can so
   carry several names, and a split leaves several faces with one name.
6. **A face made from an edge** (a fillet's round, a chamfer's band) is
   named after that edge's two faces.
7. **An edge is named by the two faces it runs between**, by their least
   names.
8. **Finding a face by name**: the face bearing that name; among several
   (a face a later feature split), the one nearest the point the
   reference was picked at. **Finding an edge**: an edge between faces of
   the two names; among several, the one passing nearest the point.
9. **A pick keeps one name of a face**, its least: enough to find it, and
   the same whichever way the face was reached.

## Where each rule is held

| Rule | Code | Test (`crates/kernel_ogeom/tests/kernel/naming.rs`) |
| --- | --- | --- |
| 1, 2, 3 | `naming::tool_names`, `NameMap::name_the_rest` | `every_face_is_named_and_the_names_hold_across_builds` |
| 4, 5 | `NameMap::carry` | `an_up_to_face_follows_its_face_by_name`, `a_thickness_opens_its_face_by_name` |
| 6 | `NameMap::carry` (an edge's faces) | `a_fillet_s_round_is_named_after_its_edge` |
| 7, 9 | `naming::edge_faces`, `NameMap::primary` | `a_fillet_s_round_is_named_after_its_edge` |
| 8 | `naming::find_face`, `naming::find_edge` | `a_face_probe_finds_its_face_by_name` |

## Reporting a reference that moved

Say which feature's reference moved, what it was picked on, and what
changed before it moved (a dimension, a feature added before it). The
names of the faces involved, from `doc.faces` before and after, show
whether the face kept its name (a lookup problem, rule 8) or lost it
(a naming problem, rules 1 to 7).
