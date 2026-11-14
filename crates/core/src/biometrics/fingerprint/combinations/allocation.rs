//! Shared absolute-priority bands and valid-subset allocation.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PriorityBand {
    High,
    Middle,
    Low,
}

impl PriorityBand {
    pub(super) const ALL: [Self; 3] = [Self::High, Self::Middle, Self::Low];

    pub(super) fn index(self) -> usize {
        self as usize
    }

    /// A subset belongs to the band of its strongest included point.
    pub(super) fn containing(priority: f64) -> Self {
        if priority > 0.9 {
            Self::High
        } else if priority >= 0.5 {
            Self::Middle
        } else {
            Self::Low
        }
    }

    /// Contiguous leading ranks in a descending-priority point collection.
    pub(super) fn leading_ranks(self, priorities: &[f64]) -> Range<usize> {
        let high_end =
            priorities.partition_point(|&priority| Self::containing(priority) == Self::High);
        let middle_end =
            priorities.partition_point(|&priority| Self::containing(priority) != Self::Low);
        match self {
            Self::High => 0..high_end,
            Self::Middle => high_end..middle_end,
            Self::Low => middle_end..priorities.len(),
        }
    }
}

/// Two high, one middle and one low selection per complete cycle. Exhausted
/// bands release their turns to the remaining bands without changing weights.
/// Only an accepted, unique subset advances a live band's turn.
pub(super) struct BandSchedule {
    slot: usize,
    available: [bool; 3],
}

impl BandSchedule {
    const CYCLE: [PriorityBand; 4] = [
        PriorityBand::High,
        PriorityBand::Middle,
        PriorityBand::High,
        PriorityBand::Low,
    ];

    pub(super) fn new() -> Self {
        Self {
            slot: 0,
            available: [true; 3],
        }
    }

    pub(super) fn current(&mut self) -> Option<PriorityBand> {
        for _ in 0..Self::CYCLE.len() {
            let band = Self::CYCLE[self.slot];
            if self.available[band.index()] {
                return Some(band);
            }
            self.accepted();
        }
        None
    }

    pub(super) fn accepted(&mut self) {
        self.slot = (self.slot + 1) % Self::CYCLE.len();
    }

    pub(super) fn exhaust(&mut self, band: PriorityBand) {
        self.available[band.index()] = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_are_absolute_and_middle_includes_both_boundaries() {
        let priorities = [1., 0.95, 0.9, 0.7, 0.5, 0.49, 0.1];
        for band in PriorityBand::ALL {
            let ranks = band.leading_ranks(&priorities);
            for (rank, &priority) in priorities.iter().enumerate() {
                assert_eq!(
                    ranks.contains(&rank),
                    PriorityBand::containing(priority) == band
                );
            }
        }
        assert_eq!(PriorityBand::Middle.leading_ranks(&priorities), 2..5);
    }

    #[test]
    fn only_accepted_selections_advance_and_exhausted_shares_redistribute() {
        use PriorityBand::{High, Low, Middle};
        let mut schedule = BandSchedule::new();
        let mut next = || {
            let band = schedule.current().unwrap();
            assert_eq!(schedule.current(), Some(band));
            schedule.accepted();
            band
        };
        assert_eq!(
            std::array::from_fn::<_, 8, _>(|_| next()),
            [High, Middle, High, Low, High, Middle, High, Low]
        );

        schedule.exhaust(Middle);
        let actual: Vec<_> = (0..6)
            .map(|_| {
                let band = schedule.current().unwrap();
                schedule.accepted();
                band
            })
            .collect();
        assert_eq!(actual, [High, High, Low, High, High, Low]);
        schedule.exhaust(High);
        assert_eq!(schedule.current(), Some(Low));
        schedule.accepted();
        assert_eq!(schedule.current(), Some(Low));
        schedule.exhaust(Low);
        assert_eq!(schedule.current(), None);
    }
}
