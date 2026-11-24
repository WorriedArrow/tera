//! numeric key payload. ordering follows postgres cmp_numerics
//! (src/backend/utils/adt/numeric.c): -inf < finite < +inf < nan, nan = nan.
//! the finite part follows the sqlite4 key encoding (base-100 digits,
//! 2x+1 for every digit but the last, 2x for the last) with a fixed-width
//! exponent instead of sqlite4's folded exponent classes.

#[derive(Clone, Debug)]
pub enum Num {
    NegInf,
    /// value = mantissa * 10^-scale, negated if `neg`.
    Finite { neg: bool, mantissa: u128, scale: u32 },
    PosInf,
    NaN,
}

pub const CLASS_NEG_INF: u8 = 0x01;
pub const CLASS_NEG: u8 = 0x02;
pub const CLASS_ZERO: u8 = 0x03;
pub const CLASS_POS: u8 = 0x04;
pub const CLASS_POS_INF: u8 = 0x05;
pub const CLASS_NAN: u8 = 0x06;

pub fn payload(n: &Num, out: &mut Vec<u8>) {
    let (neg, mantissa, scale) = match *n {
        Num::NegInf => return out.push(CLASS_NEG_INF),
        Num::PosInf => return out.push(CLASS_POS_INF),
        Num::NaN => return out.push(CLASS_NAN),
        Num::Finite { neg, mantissa, scale } => (neg, mantissa, scale),
    };
    if mantissa == 0 {
        // 0, -0 and 0.000 are all equal in postgres; the exact scale lives in the value.
        return out.push(CLASS_ZERO);
    }
    let digits: Vec<u8> = mantissa.to_string().bytes().map(|b| b - b'0').collect();
    // value = 0.d1 d2 ... dn * 10^e10 with d1 != 0
    let e10 = digits.len() as i64 - scale as i64;
    // move to base 100: if e10 is odd, prepend a zero digit so pairs align.
    let (mut dec, e100) = if e10 % 2 == 0 {
        (digits, e10 / 2)
    } else {
        let mut d = vec![0];
        d.extend(digits);
        (d, (e10 + 1) / 2)
    };
    if dec.len() % 2 == 1 {
        dec.push(0);
    }
    let mut cents: Vec<u8> = dec.chunks(2).map(|c| c[0] * 10 + c[1]).collect();
    while cents.last() == Some(&0) {
        cents.pop();
    }
    out.push(if neg { CLASS_NEG } else { CLASS_POS });
    let start = out.len();
    out.extend_from_slice(&((e100 as i32 as u32) ^ 0x8000_0000).to_be_bytes());
    let last = cents.len() - 1;
    for (i, c) in cents.iter().enumerate() {
        out.push(if i == last { 2 * c } else { 2 * c + 1 });
    }
    if neg {
        for b in &mut out[start..] {
            *b = !*b;
        }
    }
}
