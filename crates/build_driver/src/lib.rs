//! Building a document's solids, one way for every caller.
//!
//! The benches plan each body's build (`DocumentService::rebuild_jobs`);
//! this crate does the rest: it starts each job ([`begin`]: the body's
//! errors cleared, a plan's failure recorded, an emptied history's
//! geometry dropped), runs the chain ([`execute`], and
//! [`build_until_failure`] for a failing step, which leaves the body the
//! history before it), and lands the results ([`store_answers`],
//! [`store_solid`], [`record_failure`], [`clear_unbuilt`]).
//!
//! [`build_all`] puts the pieces together on the calling thread, round
//! after round until no job is left: headless scripts and project import
//! build that way. The app runs the chain on its kernel worker threads and
//! lands what comes back through the same pieces, so the three never
//! disagree on what a build leaves in the document.

mod chain;
mod run;
mod store;

pub use chain::{BuildFailure, Built, Chain, build_until_failure, execute};
pub use run::{BodyBuild, Build, KernelStep, ShapeReader, build_all};
pub use store::{
    Begun, Landed, Next, begin, clear_unbuilt, land_shape, record_failure, store_answers,
    store_solid,
};

pub use kernel_ogeom::{ChainCache, OgeomKernel};
