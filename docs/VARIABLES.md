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

Windows › Variables opens the Variables panel: one tab per set, a row per
variable with its formula, what it comes to, and a comment. Click a cell
to edit it; the row at the end adds a variable, and the tab strip's "+"
makes a new set. Sets also appear in the model tree, where a double click
opens them in the panel.

## Setting a number by a formula

Select a feature: the property panel's Data tab lists its numbers under
Parameters. Type a value, with a unit if you like (`1 in`), or press `fx`
beside it to give it a formula; while you type, the field shows what the
formula comes to and suggests names (Tab takes the first). A number set by
a formula shows `fx` and its value; click it to change the formula, and
empty the formula to keep the number as it stands.

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

## Configurations

Configurations are versions of a model: a small, a medium and a large
bracket, each giving some variables values of its own. In the Variables
panel's Configurations tab, add the configurations, then with "+ variable"
the variables they set: each row then gives each variable a formula, or
leaves it empty to keep the variable's own. The radio button, or the
selector at the top of the panel, puts one in effect; everything that
follows those variables rebuilds.

A row's value takes the place of the variable's formula, so it cannot
read that same variable (that would be a loop).

File › Export offers "Every configuration": one file per configuration,
named after it (`bracket-Large.3mf`), each built in turn; the
configuration in effect before comes back after. From a script:

```lua
for _, row in ipairs(pc.config.list().rows) do
  pc.config.activate{name = row.name}
  pc.doc.rebuild()
  pc.file.export{path = "bracket-" .. row.name .. ".3mf"}
end
```

## Undo and saving

The document keeps what you typed; what a formula comes to is worked out
again when it opens. Changing a variable is one undo step, together with
everything it moves.
