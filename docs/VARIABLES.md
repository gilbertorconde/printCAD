# Variables and formulas

Any number in a model can be a formula: a pad's length, a hole's diameter,
a sketch dimension, a joint's offset, a datum's offset. A formula can read
variables and the numbers of other objects, and everything that depends on
it rebuilds when they change.

## Variable sets

A variable set is a named object holding variables, each defined by a
formula:

| Printer | |
| --- | --- |
| `nozzle` | `0.4 mm` |
| `layer` | `0.2 mm` |
| `wall` | `3 * Printer.nozzle` |

A document can have as many sets as it needs (`Printer`, `Bracket`,
`Fasteners`). Formulas read a variable as `Set.name`.

## What a formula can say

- Numbers and arithmetic: `+ - * / % ^` and brackets.
- Units after a number: `mm`, `cm`, `m`, `um`, `in`, `ft`, `deg` (or `°`),
  `rad`.
- References: `Printer.wall`, `Pad.length`, a named sketch dimension such
  as `Profile.width`. A name with spaces goes in backticks:
  `` `Top plate`.thickness ``.
- Functions: `abs`, `min`, `max`, `round`, `floor`, `ceil`, `sqrt`,
  `hypot`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, and
  `if(condition, then, else)` with `< <= > >= == !=`, `&&`, `||`, `!`.
- Constants: `pi`, `tau`.

## Units are checked

Every value knows what it is: a length, an angle, a plain number, an area.
A length field refuses `30 deg`, and `2 mm * 3 mm` is an area, not a
length. A number written without a unit takes the unit of what is beside
it, or of the field: `Printer.wall + 2` in a length field adds 2 in the
document's unit, and an angle field reads a bare `45` as degrees.
`sin(30)` takes degrees too.

## Referring to other objects

Each feature's numbers have names: a pad's `length`, a pocket's `depth`, a
hole's `diameter`, a fillet's `radius`, a pattern's `occurrences`, a
datum's `offset_z`. A sketch dimension can be read once it has a name.
`pc.doc.parameters{id = ...}` lists them all.

Renaming an object or a variable rewrites every formula that refers to
it. A formula that reads itself, however far round, is reported as a loop,
as is a name that is missing or that two objects share.

## Undo and saving

The document keeps what you typed; what a formula comes to is worked out
again when it opens. Changing a variable is one undo step, together with
everything it moves.
