//! engine keys: the bytes tr_storage sorts. user key, then an optional mvcc
//! version, then one byte giving the version length (0 or 12). the shape follows
//! cockroachdb's mvcc key suffix (sigmod 2020 paper sec. 2; pebble comparer docs)
//! so tr_storage can split off the user-key prefix for bloom filters.

use core::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    /// nanoseconds since the unix epoch (hlc physical part, kulkarni et al. 2014).
    pub wall: u64,
    /// hlc logical counter.
    pub logical: u32,
}

pub const VERSION_LEN: u8 = 12;

pub fn encode(user_key: &[u8], ts: Option<Timestamp>) -> Vec<u8> {
    let mut k = Vec::with_capacity(user_key.len() + 13);
    k.extend_from_slice(user_key);
    match ts {
        None => k.push(0),
        Some(t) => {
            k.extend_from_slice(&t.wall.to_be_bytes());
            k.extend_from_slice(&t.logical.to_be_bytes());
            k.push(VERSION_LEN);
        }
    }
    k
}

/// length of the user-key prefix. this is what bloom filters hash and what
/// prefix seeks compare.
pub fn split(k: &[u8]) -> Option<usize> {
    let n = *k.last()? as usize;
    if n != 0 && n != VERSION_LEN as usize {
        return None;
    }
    k.len().checked_sub(1 + n)
}

pub fn decode(k: &[u8]) -> Option<(&[u8], Option<Timestamp>)> {
    let p = split(k)?;
    let v = &k[p..k.len() - 1];
    if v.is_empty() {
        return Some((&k[..p], None));
    }
    let wall = u64::from_be_bytes(v[..8].try_into().ok()?);
    let logical = u32::from_be_bytes(v[8..12].try_into().ok()?);
    Some((&k[..p], Some(Timestamp { wall, logical })))
}

/// user keys ascending; for one user key the bare (unversioned) key first,
/// then versions newest first.
pub fn compare(a: &[u8], b: &[u8]) -> Ordering {
    let (pa, pb) = (split(a).expect("engine key"), split(b).expect("engine key"));
    a[..pa].cmp(&b[..pb]).then_with(|| {
        let va = &a[pa..a.len() - 1];
        let vb = &b[pb..b.len() - 1];
        match (va.is_empty(), vb.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            // fixed-width big-endian, so reversed byte order is newest first.
            (false, false) => vb.cmp(va),
        }
    })
}
