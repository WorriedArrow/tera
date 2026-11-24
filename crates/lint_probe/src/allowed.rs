// none of this may be reported: the deterministic alias, BTreeMap, Duration.

use tr_env::collections::DetState;
use tr_env::collections::HashMap;
use tr_env::collections::HashSet;

pub fn ok() -> usize {
    let mut m: HashMap<u64, u64> = HashMap::default();
    m.insert(1, 2);
    let mut s: HashSet<u64> = HashSet::with_capacity_and_hasher(4, DetState::seeded(9));
    s.insert(3);
    let mut b = std::collections::BTreeMap::new();
    b.insert(1u8, std::time::Duration::from_millis(3));
    m.len() + s.len() + b.len()
}
