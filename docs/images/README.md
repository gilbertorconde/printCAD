# Pictures in the guides

Pictures of models (`bracket-fillet`, `holes-section`, `nut-trap`,
`generators`, `surface-shade`, `hinged-arm`) are drawn headless by
`scripts/doc-pictures.sh`, which runs each scene's Lua and ends it in
`pc.doc.picture`: run it again after a change that shows in them.

The others are crops of the running app, the website's `site/assets/app.png`
whole: the app's own picture of its window (`PRINTCAD_BENCH_WINDOW=<path>`,
so nothing else on the screen is in them) at 1600 x 940, with a separate
configuration folder (`XDG_CONFIG_HOME`, tool panels and bench tabs on),
the scene set up by the bench hooks (`PRINTCAD_BENCH_SKETCH=1` or `=pad`,
`PRINTCAD_OPEN_DOC` on a design from the corpus, `PRINTCAD_BENCH_TASK`,
`PRINTCAD_BENCH_CLICK` with `PRINTCAD_BENCH_TOOL`), then cropped to the
panel or the part of the view a guide talks about. The website's shot is the
owner's toolhead carriage, opened from a copy with the body selected and
annotations off.
