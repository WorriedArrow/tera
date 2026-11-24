//! time, randomness, network, disk and task spawning behind traits, with a production impl and a simulated one.

mod hash;

pub use hash::{FixedState, FnvHasher};

/// `std::collections::HashMap` with a hasher that has no per-process random
/// seed, so iteration order depends only on the operations applied. core crates
/// use this instead of the std default, which seeds from os randomness.
pub type HashMap<K, V> = std::collections::HashMap<K, V, FixedState>;

/// `std::collections::HashSet` with a fixed hasher, see [`HashMap`].
pub type HashSet<T> = std::collections::HashSet<T, FixedState>;
