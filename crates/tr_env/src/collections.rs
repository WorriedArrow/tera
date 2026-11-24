//! deterministic hash maps for core crates. iteration order depends only on
//! the seed and the sequence of operations, never on the os or addresses.

use std::hash::BuildHasher;

use xxhash_rust::xxh3::Xxh3;

/// fixed-seed xxh3. use DetState::seeded with a value drawn from Env::rng for
/// maps keyed by client-controlled data, so production gets a per-process
/// seed (hash flooding) while the sim stays reproducible from its seed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetState {
    seed: u64,
}

impl DetState {
    pub const fn seeded(seed: u64) -> DetState {
        DetState { seed }
    }
}

impl BuildHasher for DetState {
    type Hasher = Xxh3;
    fn build_hasher(&self) -> Xxh3 {
        Xxh3::with_seed(self.seed)
    }
}

#[allow(clippy::disallowed_types)]
pub type HashMap<K, V> = std::collections::HashMap<K, V, DetState>;
#[allow(clippy::disallowed_types)]
pub type HashSet<T> = std::collections::HashSet<T, DetState>;
