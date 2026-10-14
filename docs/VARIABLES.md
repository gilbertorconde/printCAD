# Variables and formulas

Any number in a model can be a formula: a pad's length, a hole's diameter,
a sketch dimension, a sketch text's size, spacing and angle, a joint's
offset, a datum's offset, the components of a
pad's or pocket's custom direction (`Pad.direction_x`, `_y`, `_z`). A formula can read
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

## Sketch dimensions

Adding a dimension opens its editor next to its label, showing what it
measures, with the value selected. To set a number, type it and press
Enter. Escape keeps the measured value. Double clicking a dimension's
label opens the same editor. You can switch this off in Preferences ›
Sketch ("Ask for a dimension's value"); new dimensions then take the
measured value and their row in the panel gets the keyboard.

While you draw, the fields beside the cursor (a line's length, an arc's
sweep, a slot's width) take a formula as well as a number. The formula
is worked out once, as the shape is placed: the shape keeps the value it
came to, not the formula, so it does not follow the variables later. To
have a size follow them, set its dimension by a formula as below.

The editor also:

- lists the variables that fit the dimension (lengths for a length,
  angles for an angle) as chips, each with its value. Click one and the
  dimension reads it.
- takes a formula in the same field, with names completed as you type.
- saves the value as a variable with "+ Save as variable": give it a name
  and pick a set (or "New set"), and Save. A typed number becomes the
  variable's value with its unit (`12 mm`), and a formula becomes the
  variable's formula. The dimension then reads the variable. With no
  variable set yet, one called `Variables` is made.

## Completion

Wherever a formula is typed (a number's `fx`, a variable, a
configuration's value, a sketch dimension), a list of the five names most
like the word you are typing opens under it, each with its value. Up and
Down move through it; Enter or Tab puts the highlighted name in, a click
puts in the one clicked, and Escape puts the list away. A word right
after a number is taken as a unit (`2 in`) and is left alone.

## What a formula can say

- Numbers and arithmetic: `+ - * / % ^` and brackets.
- Units after a number: `mm`, `cm`, `m`, `um` (or `µm`), `in`, `ft`, `deg`
  (or `°`), `rad`.
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

A text in a sketch lists its size, spacing and angle among the sketch's
numbers (`Text "v1" size`), which a formula can set but no formula reads.
Set by a formula, the letters are made again at the new value in the font
the document keeps. Formulas make numbers, so the text's string is not one
of them.

Renaming an object or a variable rewrites every formula that refers to
it. A formula that reads itself, however far round, is reported as a loop;
a name that is missing, or that two objects share, is reported too.

## Configurations

Configurations are versions of a model: a small, a medium and a large
bracket, each giving some variables values of its own. Right-click the
document's row in the tree and pick "New configurations table", then
select the table: its Data tab has the one in effect at the top, the
variables the configurations set ("+ Variable" adds one), and the
configurations themselves ("Add a configuration"). Click a configuration
to give each variable a formula, or leave it empty to keep the variable's
own, and Apply (or Enter); "Put in effect", or the selector at the top,
switches to it, and everything that follows those variables rebuilds.
Under them the Table shows every configuration's values at once, a row per
configuration; "Configurations as columns" turns it to a column per
configuration and a row per variable, which stays narrow when there are
many variables. The choice is the view's, not the document's.

A row's value takes the place of the variable's formula, so it cannot
read that same variable (that would be a loop).

A configuration can also leave bodies out ("Leaves out" in its editor):
while it is in effect they are not drawn, picked, exported or checked. An
assembly with two versions of a part swaps them by leaving one out of each
configuration. `pc.config.leave_out{name = ..., bodies = {...}}` does the
same from a script.

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
