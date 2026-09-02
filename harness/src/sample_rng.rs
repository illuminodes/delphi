/// Deterministic `xorshift64*` generator, so a convergence run can be repeated
/// exactly from its seed.
#[derive(Debug, Clone)]
pub struct SampleRng {
    state: u64,
}

impl SampleRng {
    #[must_use]
    pub const fn seeded(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0x9e37_79b9_7f4a_7c15 } else { seed },
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut state = self.state;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        self.state = state;
        state.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        usize::try_from(self.next_u64() % bound as u64).unwrap_or(0)
    }

    /// Draw `count` distinct indices below `bound` via a partial Fisher-Yates
    /// shuffle, so no index repeats within one draw.
    pub fn choose(&mut self, bound: usize, count: usize) -> Vec<usize> {
        let take = count.min(bound);
        let mut pool: Vec<usize> = (0..bound).collect();
        for index in 0..take {
            let swap = index + self.below(bound - index);
            pool.swap(index, swap);
        }
        pool.truncate(take);
        pool
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::SampleRng;

    #[test]
    fn the_same_seed_replays_the_same_stream() {
        let first: Vec<u64> = (0..8).map(|_| SampleRng::seeded(7).next_u64()).collect();
        let mut rng = SampleRng::seeded(7);
        assert_eq!(first[0], rng.next_u64());
    }

    #[test]
    fn different_seeds_diverge() {
        assert_ne!(
            SampleRng::seeded(1).next_u64(),
            SampleRng::seeded(2).next_u64()
        );
    }

    #[test]
    fn a_zero_seed_still_produces_a_stream() {
        assert_ne!(SampleRng::seeded(0).next_u64(), 0);
    }

    #[test]
    fn choosing_yields_distinct_indices_in_range() {
        let picks = SampleRng::seeded(42).choose(100, 30);
        assert_eq!(picks.len(), 30);
        assert!(picks.iter().all(|index| *index < 100));
        assert_eq!(picks.iter().collect::<HashSet<_>>().len(), 30);
    }

    #[test]
    fn choosing_more_than_available_returns_everything_once() {
        let picks = SampleRng::seeded(9).choose(10, 50);
        assert_eq!(picks.len(), 10);
        assert_eq!(picks.iter().collect::<HashSet<_>>().len(), 10);
    }

    #[test]
    fn choosing_from_nothing_is_empty() {
        assert!(SampleRng::seeded(3).choose(0, 5).is_empty());
    }
}
