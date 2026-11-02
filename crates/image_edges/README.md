# image_edges

Subpixel edge chains in a grey image, with nothing but the standard
library: smoothing, Sobel gradients, ridges kept where the gradient peaks
across the edge, thresholds read from the image's own noise, weak edges
kept where they join strong ones, each point refined by a parabola
across the gradient, and the points linked into chains that know which
side is brighter.

```rust
let gray = image_edges::Gray { width: 4, height: 1, pixels: &[0.0, 0.0, 1.0, 1.0] };
let chains = image_edges::detect(&gray, &image_edges::Settings::default());
assert!(chains.is_empty(), "a single row has no ridge to follow");
```

Coordinates are in pixels, the centre of pixel (i, j) at (i, j), x to the
right and y down. Like `metrology` and `sketch_solver`, the directory can
be copied to another consumer as it is.
