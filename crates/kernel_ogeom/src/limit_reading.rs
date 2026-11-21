//! Reading a previewed fillet's or chamfer's size limit: inline in its
//! build, or after it, from what the build left (a [`LimitReading`]), so
//! the preview never waits on the kernel's probe builds.

use std::sync::{Arc, Mutex};

use kernel_api::{LawLimit, SizeLimit, SolidOp};
use ogeom::topo::{Model, Shape};

use crate::naming::{self, NameMap};
use crate::ops;

/// How many fillet and chamfer limits a memo keeps.
const KEPT_LIMITS: usize = 8;

/// How many limits of a variable fillet's radii a memo keeps.
const KEPT_LAW_LIMITS: usize = 32;

/// The limits a body's builds read, by the history before the op and the
/// op's size-free key: a drag of a size finds its limit here on every
/// build after the first. Shared between a [`crate::ChainCache`] and the
/// readings its builds leave, so a reading made after its build is kept
/// where the next build looks.
#[derive(Default)]
pub(crate) struct LimitMemo {
    /// Newest last.
    limits: Vec<(u64, ops::dressup::Limits)>,
    /// Each radius of a variable fillet by its own key, newest last.
    law_limits: Vec<(u64, Option<f64>)>,
}

pub(crate) type SharedMemo = Arc<Mutex<LimitMemo>>;

fn lock(memo: &SharedMemo) -> std::sync::MutexGuard<'_, LimitMemo> {
    memo.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn keep<T>(kept: &mut Vec<(u64, T)>, key: u64, answer: T, most: usize) {
    kept.retain(|(k, _)| *k != key);
    kept.push((key, answer));
    if kept.len() > most {
        kept.remove(0);
    }
}

/// Which limit an op's reading is, and where the memo keeps it: the op's
/// place and size-free key, the memo's key for its first size, and each
/// law radius's place, own key and the memo's key for it.
#[derive(Clone)]
pub(crate) struct LimitKeys {
    pub op_index: usize,
    pub key: u64,
    /// `None` without a memo: read every time.
    pub held: Option<u64>,
    pub law: Vec<(usize, u64, Option<u64>)>,
}

impl LimitKeys {
    /// The limit as `memo` knows it, every part of it read before.
    pub fn known(&self, memo: &SharedMemo) -> Option<SizeLimit> {
        let memo = lock(memo);
        let (largest, sides) = memo
            .limits
            .iter()
            .find(|(k, _)| Some(*k) == self.held)
            .map(|(_, known)| *known)?;
        let law = self
            .law
            .iter()
            .map(|&(_, key, held)| {
                let held = held?;
                let (_, largest) = memo.law_limits.iter().find(|(k, _)| *k == held)?;
                Some(LawLimit {
                    key,
                    largest: *largest,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(SizeLimit {
            op_index: self.op_index,
            key: self.key,
            largest,
            sides,
            law,
        })
    }

    /// Read the limit of `op` on `solid`, the names of the solid set as
    /// the op sees them.
    pub fn read(&self, model: &Model, solid: &Shape, op: &SolidOp) -> SizeLimit {
        let (largest, sides) = ops::dressup::size_limit(model, solid, op);
        let law = self
            .law
            .iter()
            .map(|&(at, key, _)| LawLimit {
                key,
                largest: ops::dressup::law_limit(model, solid, op, at),
            })
            .collect();
        SizeLimit {
            op_index: self.op_index,
            key: self.key,
            largest,
            sides,
            law,
        }
    }

    /// Keep `limit` in `memo` for the next build that asks.
    pub fn remember(&self, memo: &SharedMemo, limit: &SizeLimit) {
        let mut memo = lock(memo);
        if let Some(held) = self.held {
            keep(
                &mut memo.limits,
                held,
                (limit.largest, limit.sides),
                KEPT_LIMITS,
            );
        }
        for (&(_, _, held), law) in self.law.iter().zip(&limit.law) {
            if let Some(held) = held {
                keep(&mut memo.law_limits, held, law.largest, KEPT_LAW_LIMITS);
            }
        }
    }
}

/// A previewed fillet's or chamfer's size limit, left by its build to be
/// read after it: the solid before the op in a model of its own, the op,
/// and the names and tolerance the op was built with. Plain data, so it
/// may be read on any thread; what it reads is kept where the body's next
/// builds look.
pub struct LimitReading {
    pub(crate) keys: LimitKeys,
    pub(crate) model: Model,
    pub(crate) solid: Shape,
    pub(crate) op: SolidOp,
    pub(crate) names: NameMap,
    pub(crate) tolerance: Option<f64>,
    pub(crate) memo: SharedMemo,
}

impl LimitReading {
    /// Which reading this is: what the build's
    /// `SolidBuildResult::size_limit_to_come` names, the same for every
    /// build of the same history before the op and the same size-free op.
    pub fn id(&self) -> u64 {
        self.keys.held.unwrap_or_default()
    }

    /// Read the limit: what the build would have read inline. It is not
    /// kept until [`Self::keep`], so a reading cut short by a cancel is
    /// never remembered.
    pub fn read(&self) -> SizeLimit {
        crate::progress::context(format_args!(
            "{} size limit",
            crate::progress::op_label(&self.op)
        ));
        let _tolerance = ops::ToleranceScope::state(self.tolerance);
        let named = naming::set_current(self.names.clone());
        let limit = self.keys.read(&self.model, &self.solid, &self.op);
        drop(named);
        // What the reading found by position belongs to no op's warnings.
        crate::probe::take_found_by_position();
        limit
    }

    /// Keep `limit`, as read, for the body's next builds.
    pub fn keep(&self, limit: &SizeLimit) {
        self.keys.remember(&self.memo, limit);
    }
}
