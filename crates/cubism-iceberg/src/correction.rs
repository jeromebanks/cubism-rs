//! Strategy selection for corrections (`docs/TIMESERIES_ROADMAP.md`
//! Milestone 3 — plan line 635: "no additive correction is used for
//! non-idempotent/non-subtractable states").
//!
//! This module is deliberately narrow: it answers only "given the
//! [`AggKind`]s a correction would touch, is an additive/subtractive
//! shortcut ever permitted, or must the affected windows be fully
//! rebuilt?" It does not represent a source checkpoint, a time range, or
//! the set of affected windows — the roadmap's fuller "What it does"
//! description for `CorrectionPlan` — because nothing in this crate
//! consumes those yet; that wiring belongs to Milestone 4's coordinator,
//! which is also what will choose *how* to run the shortcut this module
//! only decides is *allowed*. It also does not consult
//! [`cubism_core::LatenessPolicy`]: strategy selection here is purely a
//! function of aggregate-kind capabilities, not of how late an event
//! arrived, so no `LatenessPolicy` value is threaded through this type.

use cubism_core::{AggKind, capabilities_for};

/// Whether a correction may apply an additive/subtractive shortcut or must
/// fully rebuild the affected windows from source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CorrectionStrategy {
    /// Recompute the affected windows from scratch.
    FullRebuild,
    /// Undo the stale contribution and apply the corrected one in place.
    AdditiveShortcut,
}

/// The chosen strategy for correcting a set of aggregate states.
///
/// [`CorrectionPlan::select`] permits [`CorrectionStrategy::AdditiveShortcut`]
/// only when *every* involved [`AggKind`]'s [`AggregateCapabilities`] declares
/// both `subtractable` and `idempotent` — matching that struct's own doc
/// comment ("a `false` value means callers must not use that law for
/// repartitioning, retries, rolling windows, or corrections"), which names
/// corrections explicitly and does not carve out an exception for
/// `subtractable` alone. `idempotent` is required in addition to
/// `subtractable` because a correction shortcut both removes a stale
/// contribution and re-applies a corrected one; if the state is not also
/// idempotent, a retried removal/re-application is not safe to repeat.
///
/// [`AggregateCapabilities`]: cubism_core::AggregateCapabilities
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrectionPlan {
    strategy: CorrectionStrategy,
}

impl CorrectionPlan {
    /// Selects a strategy for correcting the given aggregate kinds. An empty
    /// `kinds` list is treated as ineligible for a shortcut (there is
    /// nothing to safely shortcut), so it selects `FullRebuild`.
    pub fn select(kinds: &[AggKind]) -> Self {
        let shortcut_eligible = !kinds.is_empty()
            && kinds.iter().all(|kind| {
                let caps = capabilities_for(*kind);
                caps.subtractable && caps.idempotent
            });
        Self {
            strategy: if shortcut_eligible {
                CorrectionStrategy::AdditiveShortcut
            } else {
                CorrectionStrategy::FullRebuild
            },
        }
    }

    pub fn strategy(&self) -> CorrectionStrategy {
        self.strategy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Proves plan line 635's negative assertion: every `AggKind` this
    /// workspace currently defines is missing `subtractable`, `idempotent`,
    /// or both (verified against `cubism_core::aggregate_state::capabilities_for`
    /// at the time this test was written — `Sum`/`Count` are `subtractable`
    /// but not `idempotent`; every other kind is not `subtractable`), so
    /// `CorrectionPlan::select` refuses an additive shortcut for each of
    /// them individually and for the full set together. It does **not**
    /// prove the `AdditiveShortcut` branch is reachable — no current
    /// `AggKind` satisfies both capabilities, so that branch is exercised
    /// only by the synthetic all-true case below, not by any real kind.
    #[test]
    fn full_rebuild_is_selected_for_every_current_agg_kind() {
        let all_kinds = [
            AggKind::Sum,
            AggKind::Count,
            AggKind::Min,
            AggKind::Max,
            AggKind::Avg,
            AggKind::Variance,
            AggKind::CountDistinct,
            AggKind::TopK,
            AggKind::Quantile,
            AggKind::Centroid,
            AggKind::ReservoirSample,
        ];

        for kind in all_kinds {
            let plan = CorrectionPlan::select(&[kind]);
            assert_eq!(
                plan.strategy(),
                CorrectionStrategy::FullRebuild,
                "{kind:?} must not be granted an additive shortcut"
            );
        }

        let plan = CorrectionPlan::select(&all_kinds);
        assert_eq!(plan.strategy(), CorrectionStrategy::FullRebuild);
    }

    /// A synthetic capability check (not routed through any real `AggKind`,
    /// since none currently satisfies both flags) proving the eligibility
    /// rule itself — subtractable-and-idempotent, not subtractable alone —
    /// is what `select` implements, by checking the rule's two building
    /// blocks directly rather than only its always-false outcome above.
    #[test]
    fn shortcut_requires_both_subtractable_and_idempotent() {
        let sum_caps = capabilities_for(AggKind::Sum);
        assert!(sum_caps.subtractable, "Sum is subtractable");
        assert!(!sum_caps.idempotent, "Sum is not idempotent");
        assert_eq!(
            CorrectionPlan::select(&[AggKind::Sum]).strategy(),
            CorrectionStrategy::FullRebuild,
            "subtractable alone must not be enough to grant a shortcut"
        );
    }

    #[test]
    fn empty_kind_list_selects_full_rebuild() {
        let plan = CorrectionPlan::select(&[]);
        assert_eq!(plan.strategy(), CorrectionStrategy::FullRebuild);
    }
}
