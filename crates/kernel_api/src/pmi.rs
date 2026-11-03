//! Semantic annotations exchanged without kernel topology. Probes name
//! the controlled faces; the caller specifies their coordinate frame.

/// Semantic annotations of one body. Workbenches answer in world
/// coordinates; export receives probes in the body's own frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SemanticPmi {
    pub tolerances: Vec<PmiTolerance>,
    pub datums: Vec<PmiDatum>,
    pub dimensions: Vec<PmiDimension>,
}

impl SemanticPmi {
    pub fn is_empty(&self) -> bool {
        self.tolerances.is_empty() && self.datums.is_empty() && self.dimensions.is_empty()
    }

    /// Move every face probe into another frame.
    pub fn map_points(&mut self, point: impl Fn([f64; 3]) -> [f64; 3]) {
        for at in self
            .tolerances
            .iter_mut()
            .map(|t| &mut t.at)
            .chain(self.datums.iter_mut().map(|d| &mut d.at))
            .chain(self.dimensions.iter_mut().flat_map(|d| &mut d.features))
        {
            for p in at {
                *p = point(*p);
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PmiTolerance {
    /// AP242 entity word without `_tolerance`, such as `flatness`.
    pub kind: String,
    pub name: String,
    /// Zone width in millimetres.
    pub magnitude: f64,
    /// Datum labels in precedence order, primary first.
    pub datums: Vec<String>,
    pub at: Vec<[f64; 3]>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PmiDatum {
    pub label: String,
    pub at: Vec<[f64; 3]>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PmiDimension {
    /// The dimensional characteristic, e.g. `diameter` or `linear distance`.
    pub name: String,
    /// True for radians; false for millimetres. Allowances use the same unit.
    pub angular: bool,
    pub nominal: f64,
    pub plus: Option<f64>,
    /// Signed lower allowance, normally negative.
    pub minus: Option<f64>,
    /// One probe group for a size, two ordered groups for a location.
    pub features: Vec<Vec<[f64; 3]>>,
}
