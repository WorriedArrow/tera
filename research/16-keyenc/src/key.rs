//! order-preserving encodings of single key columns.
//!
//! every ascending payload is prefix-free, so a descending column can be
//! encoded by inverting every byte of the ascending encoding (sqlite4 key
//! encoding, cockroachdb docs/tech-notes/encoding.md).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nulls {
    First,
    Last,
}

pub const NULL_LOW: u8 = 0x00;
pub const PRESENT: u8 = 0x01;
pub const NULL_HIGH: u8 = 0x02;

/// postgres default: asc nulls last, desc nulls first (create index docs).
pub fn default_nulls(dir: Dir) -> Nulls {
    match dir {
        Dir::Asc => Nulls::Last,
        Dir::Desc => Nulls::First,
    }
}

/// appends one key column. `payload` is the ascending payload of a non-null value.
pub fn put_column(buf: &mut Vec<u8>, dir: Dir, nulls: Nulls, payload: Option<&[u8]>) {
    let start = buf.len();
    // the marker is chosen so nulls land where asked after the optional inversion.
    let null_marker = match (dir, nulls) {
        (Dir::Asc, Nulls::Last) | (Dir::Desc, Nulls::First) => NULL_HIGH,
        (Dir::Asc, Nulls::First) | (Dir::Desc, Nulls::Last) => NULL_LOW,
    };
    match payload {
        None => buf.push(null_marker),
        Some(p) => {
            buf.push(PRESENT);
            buf.extend_from_slice(p);
        }
    }
    if dir == Dir::Desc {
        for b in &mut buf[start..] {
            *b = !*b;
        }
    }
}

pub fn i16_payload(v: i16) -> [u8; 2] {
    ((v as u16) ^ 0x8000).to_be_bytes()
}

pub fn i32_payload(v: i32) -> [u8; 4] {
    ((v as u32) ^ 0x8000_0000).to_be_bytes()
}

pub fn i64_payload(v: i64) -> [u8; 8] {
    ((v as u64) ^ (1 << 63)).to_be_bytes()
}

const CANONICAL_NAN64: u64 = 0x7ff8_0000_0000_0000;
const CANONICAL_NAN32: u32 = 0x7fc0_0000;

/// postgres float8 order: all nans equal and above +inf, -0 = +0
/// (src/include/utils/float.h float8_lt/float8_gt).
pub fn f64_payload(v: f64) -> [u8; 8] {
    let bits = if v.is_nan() {
        CANONICAL_NAN64
    } else if v == 0.0 {
        0
    } else {
        v.to_bits()
    };
    let bits = if bits >> 63 == 1 { !bits } else { bits | (1 << 63) };
    bits.to_be_bytes()
}

pub fn f32_payload(v: f32) -> [u8; 4] {
    let bits = if v.is_nan() {
        CANONICAL_NAN32
    } else if v == 0.0 {
        0
    } else {
        v.to_bits()
    };
    let bits = if bits >> 31 == 1 { !bits } else { bits | (1 << 31) };
    bits.to_be_bytes()
}

pub fn bool_payload(v: bool) -> [u8; 1] {
    [v as u8]
}

/// 0x00 -> 0x00 0xff, terminated by 0x00 0x01. the two-byte terminator keeps
/// the encoding prefix-free, which a single 0x00 terminator (fdb tuple layer)
/// does not: "a" -> 61 00 is a prefix of "a\0" -> 61 00 ff 00.
pub fn bytes_payload(v: &[u8], out: &mut Vec<u8>) {
    for &b in v {
        if b == 0 {
            out.extend_from_slice(&[0x00, 0xff]);
        } else {
            out.push(b);
        }
    }
    out.extend_from_slice(&[0x00, 0x01]);
}

/// decodes an ascending escaped byte string, returning the value and bytes consumed.
pub fn decode_bytes(buf: &[u8]) -> Option<(Vec<u8>, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        let b = *buf.get(i)?;
        if b != 0 {
            out.push(b);
            i += 1;
            continue;
        }
        match *buf.get(i + 1)? {
            0xff => out.push(0),
            0x01 => return Some((out, i + 2)),
            _ => return None,
        }
        i += 2;
    }
}

/// order-preserving unsigned varint for table and index ids.
/// v < 0xf8 is one byte; otherwise 0xf7 + n followed by n big-endian bytes.
pub fn put_uvarint(v: u64, out: &mut Vec<u8>) {
    if v < 0xf8 {
        out.push(v as u8);
        return;
    }
    let n = 8 - (v.leading_zeros() / 8) as usize;
    out.push(0xf7 + n as u8);
    out.extend_from_slice(&v.to_be_bytes()[8 - n..]);
}

pub fn decode_uvarint(buf: &[u8]) -> Option<(u64, usize)> {
    let first = *buf.first()?;
    if first < 0xf8 {
        return Some((first as u64, 1));
    }
    let n = (first - 0xf7) as usize;
    let bytes = buf.get(1..1 + n)?;
    let mut v = 0u64;
    for &b in bytes {
        v = (v << 8) | b as u64;
    }
    // reject non-minimal encodings so every id has exactly one key.
    if v < 0xf8 || (n > 1 && bytes[0] == 0) {
        return None;
    }
    Some((v, 1 + n))
}
