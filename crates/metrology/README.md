# metrology

Measurement arithmetic with no geometry kernel behind it: what is said of
a set of residuals or measured points once the geometry has been fitted.

- `stats`: count, mean, RMS, sigma, extremes, percentiles, histograms and
  the share of values within a tolerance, skipping values that were not
  measured (`NaN`).
- `form`: peak-to-peak form error and the smallest and largest diameter a
  round surface reaches.
- `orientation`: the zones of parallelism, perpendicularity, angularity,
  coaxiality and concentricity, from the points or axis ends a feature is
  evaluated at and a datum direction or axis.
- `ramp`: colour ramps for maps (jet and the perceptually uniform viridis,
  magma, inferno and plasma), with end caps distinct from the ramp, and
  how a value maps onto one.
- `design`: the round value a measured length, radius or angle was most
  likely designed at.

The default build uses only the Rust standard library, and the crate takes
nothing from the workspace: to use it elsewhere, copy this directory and
add a path dependency on the copy.
