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

Right-click the document's row in the model tree and pick "New variable
set". Sets are rows of the tree; select one and the property panel's Data
tab lists its variables with what each comes to. Click a variable to edit
its name, formula (it shows what the formula comes to as you type) and
comment, or remove it; "Add a variable" at the end adds one. The set's
Label row renames it.

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
bracket, each giving some variables values of its own. Right-click the
document's row in the tree and pick "New configurations table", then
select the table: its Data tab has the one in effect at the top, the
variables the configurations set ("+ Variable" adds one), and the
configurations themselves ("Add a configuration"). Click a configuration
to give each variable a formula, or leave it empty to keep the variable's
own; "Put in effect", or the selector at the top, switches to it, and
everything that follows those variables rebuilds.

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
