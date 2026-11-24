use std::hash::{BuildHasherDefault, Hasher};

/// builds [`FnvHasher`]s. every instance hashes identically on every run and
/// every machine.
pub type FixedState = BuildHasherDefault<FnvHasher>;

// 64-bit offset basis and prime from the fnv spec (draft-eastlake-fnv).
const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// 64-bit fnv-1a (draft-eastlake-fnv). integers are fed as
/// little-endian bytes and `usize` as 64 bits, so a key hashes the same on any
/// target, unlike std's defaults which use native byte order and width.
#[derive(Clone, Copy, Debug)]
pub struct FnvHasher(u64);

impl Default for FnvHasher {
    fn default() -> Self {
        Self(OFFSET_BASIS)
    }
}

impl Hasher for FnvHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut h = self.0;
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(PRIME);
        }
        self.0 = h;
    }

    fn write_u16(&mut self, n: u16) {
        self.write(&n.to_le_bytes());
    }

    fn write_u32(&mut self, n: u32) {
        self.write(&n.to_le_bytes());
    }

    fn write_u64(&mut self, n: u64) {
        self.write(&n.to_le_bytes());
    }

    fn write_u128(&mut self, n: u128) {
        self.write(&n.to_le_bytes());
    }

    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::{BuildHasher, Hash};

    fn fnv(bytes: &[u8]) -> u64 {
        let mut h = FnvHasher::default();
        h.write(bytes);
        h.finish()
    }

    #[test]
    fn matches_spec_test_vectors() {
        // fnv-1a 64-bit test vectors published with the fnv spec.
        assert_eq!(fnv(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn integers_hash_as_little_endian() {
        let mut a = FnvHasher::default();
        a.write_u32(0x0102_0304);
        assert_eq!(a.finish(), fnv(&[4, 3, 2, 1]));

        let mut b = FnvHasher::default();
        b.write_usize(7);
        let mut c = FnvHasher::default();
        c.write_u64(7);
        assert_eq!(b.finish(), c.finish());
    }

    #[test]
    fn fixed_state_has_no_random_seed() {
        let s1 = FixedState::default();
        let s2 = FixedState::default();
        assert_eq!(s1.hash_one("key"), s2.hash_one("key"));

        let mut h = FnvHasher::default();
        "key".hash(&mut h);
        assert_eq!(s1.hash_one("key"), h.finish());
    }

    #[test]
    fn map_iteration_order_is_reproducible() {
        let build = || {
            let mut m = crate::HashMap::default();
            for i in 0..1000u32 {
                m.insert(i, i);
            }
            m.into_keys().collect::<Vec<_>>()
        };
        assert_eq!(build(), build());
    }
}
