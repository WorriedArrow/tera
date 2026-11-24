use core::cmp::Ordering;
use keyenc_proto::engine::{self, Timestamp};
use keyenc_proto::key::{self, Dir, Nulls};
use keyenc_proto::numeric::{self, Num};
use keyenc_proto::row::{self, Col};
use proptest::prelude::*;

fn pg_cmp_f64(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ => a.partial_cmp(&b).unwrap(),
    }
}

fn pg_cmp_f32(a: f32, b: f32) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ => a.partial_cmp(&b).unwrap(),
    }
}

fn esc(v: &[u8]) -> Vec<u8> {
    let mut o = Vec::new();
    key::bytes_payload(v, &mut o);
    o
}

fn num_enc(n: &Num) -> Vec<u8> {
    let mut o = Vec::new();
    numeric::payload(n, &mut o);
    o
}

fn num_rank(n: &Num) -> u8 {
    match n {
        Num::NegInf => 0,
        Num::Finite { .. } => 1,
        Num::PosInf => 2,
        Num::NaN => 3,
    }
}

// reference order from postgres cmp_numerics.
fn pg_cmp_num(a: &Num, b: &Num) -> Ordering {
    match (a, b) {
        (
            Num::Finite { neg: na, mantissa: ma, scale: sa },
            Num::Finite { neg: nb, mantissa: mb, scale: sb },
        ) => {
            let s = (*sa).max(*sb);
            let va = *ma as i128 * 10i128.pow(s - sa) * if *na { -1 } else { 1 };
            let vb = *mb as i128 * 10i128.pow(s - sb) * if *nb { -1 } else { 1 };
            va.cmp(&vb)
        }
        _ => num_rank(a).cmp(&num_rank(b)),
    }
}

fn arb_num() -> impl Strategy<Value = Num> {
    prop_oneof![
        1 => Just(Num::NegInf),
        1 => Just(Num::PosInf),
        1 => Just(Num::NaN),
        20 => (any::<bool>(), 0u128..1_000_000_000_000_000, 0u32..20)
            .prop_map(|(neg, mantissa, scale)| Num::Finite { neg, mantissa, scale }),
        5 => (any::<bool>(), 0u128..2000, 0u32..6)
            .prop_map(|(neg, m, scale)| Num::Finite { neg, mantissa: m * 10u128.pow(scale % 3), scale }),
    ]
}

fn arb_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(prop_oneof![Just(0u8), Just(1u8), Just(0xffu8), Just(0x61u8), any::<u8>()], 0..8)
}

fn arb_ts() -> impl Strategy<Value = Timestamp> {
    (0u64..4, 0u32..3).prop_map(|(w, l)| Timestamp { wall: w * (1u64 << 40), logical: l })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20_000))]

    #[test]
    fn ints_preserve_order(a: i64, b: i64, c: i32, d: i32, e: i16, f: i16) {
        prop_assert_eq!(key::i64_payload(a).cmp(&key::i64_payload(b)), a.cmp(&b));
        prop_assert_eq!(key::i32_payload(c).cmp(&key::i32_payload(d)), c.cmp(&d));
        prop_assert_eq!(key::i16_payload(e).cmp(&key::i16_payload(f)), e.cmp(&f));
    }

    #[test]
    fn floats_follow_postgres_order(a in any::<u64>(), b in any::<u64>(), pick in 0u8..4) {
        let specials = [0.0f64, -0.0, f64::INFINITY, f64::NEG_INFINITY, f64::NAN, -f64::NAN];
        let a = if pick == 0 { specials[(a % 6) as usize] } else { f64::from_bits(a) };
        let b = if pick == 1 { specials[(b % 6) as usize] } else { f64::from_bits(b) };
        prop_assert_eq!(key::f64_payload(a).cmp(&key::f64_payload(b)), pg_cmp_f64(a, b));
        let (a32, b32) = (a as f32, b as f32);
        prop_assert_eq!(key::f32_payload(a32).cmp(&key::f32_payload(b32)), pg_cmp_f32(a32, b32));
    }

    #[test]
    fn bytes_preserve_order_and_are_prefix_free(a in arb_bytes(), b in arb_bytes()) {
        let (ea, eb) = (esc(&a), esc(&b));
        prop_assert_eq!(ea.cmp(&eb), a.cmp(&b));
        if a != b {
            prop_assert!(!ea.starts_with(&eb) && !eb.starts_with(&ea));
        }
        let (dec, used) = key::decode_bytes(&ea).unwrap();
        prop_assert_eq!(dec, a);
        prop_assert_eq!(used, ea.len());
    }

    #[test]
    fn numeric_follows_postgres_order(a in arb_num(), b in arb_num()) {
        let (ea, eb) = (num_enc(&a), num_enc(&b));
        let want = pg_cmp_num(&a, &b);
        prop_assert_eq!(ea.cmp(&eb), want);
        if want != Ordering::Equal {
            prop_assert!(!ea.starts_with(&eb) && !eb.starts_with(&ea));
        }
    }

    #[test]
    fn uvarint_orders_and_round_trips(a: u64, b: u64, small in 0u64..600) {
        for v in [a, b, small] {
            let mut e = Vec::new();
            key::put_uvarint(v, &mut e);
            prop_assert_eq!(key::decode_uvarint(&e), Some((v, e.len())));
        }
        let (mut ea, mut eb) = (Vec::new(), Vec::new());
        key::put_uvarint(a, &mut ea);
        key::put_uvarint(small, &mut eb);
        prop_assert_eq!(ea.cmp(&eb), a.cmp(&small));
    }

    #[test]
    fn composite_keys_respect_direction_and_nulls(
        d1 in any::<bool>(), n1 in any::<bool>(), d2 in any::<bool>(), n2 in any::<bool>(),
        a1 in prop::option::of(-3i64..3), a2 in prop::option::of(arb_bytes()),
        b1 in prop::option::of(-3i64..3), b2 in prop::option::of(arb_bytes()),
    ) {
        let dir = |d: bool| if d { Dir::Desc } else { Dir::Asc };
        let nulls = |n: bool| if n { Nulls::First } else { Nulls::Last };
        let (d1, n1, d2, n2) = (dir(d1), nulls(n1), dir(d2), nulls(n2));
        let enc = |x: Option<i64>, y: &Option<Vec<u8>>| {
            let mut k = Vec::new();
            let p = x.map(key::i64_payload);
            key::put_column(&mut k, d1, n1, p.as_ref().map(|p| &p[..]));
            let q = y.as_ref().map(|v| esc(v));
            key::put_column(&mut k, d2, n2, q.as_deref());
            k
        };
        let want = col_cmp(&a1, &b1, d1, n1).then_with(|| col_cmp(&a2, &b2, d2, n2));
        prop_assert_eq!(enc(a1, &a2).cmp(&enc(b1, &b2)), want);
    }

    #[test]
    fn engine_keys_sort_by_user_key_then_newest_first(
        ka in arb_bytes(), kb in arb_bytes(),
        ta in prop::option::of(arb_ts()), tb in prop::option::of(arb_ts()),
    ) {
        let (ea, eb) = (engine::encode(&ka, ta), engine::encode(&kb, tb));
        let want = ka.cmp(&kb).then_with(|| match (ta, tb) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (Some(x), Some(y)) => y.cmp(&x),
        });
        prop_assert_eq!(engine::compare(&ea, &eb), want);
        prop_assert_eq!(engine::decode(&ea), Some((&ka[..], ta)));
        prop_assert_eq!(engine::split(&ea), Some(ka.len()));
    }

    #[test]
    fn rows_round_trip(
        cols in prop::collection::btree_map(0u32..400, prop::option::of(arb_bytes()), 0..12),
        probe in 0u32..400,
    ) {
        let list: Vec<(u32, Option<&[u8]>)> = cols.iter().map(|(&id, v)| (id, v.as_deref())).collect();
        let enc = row::encode(&list);
        for (&id, v) in &cols {
            let want = match v { Some(v) => Col::Value(v), None => Col::Null };
            prop_assert_eq!(row::get(&enc, id), Some(want));
        }
        if !cols.contains_key(&probe) {
            prop_assert_eq!(row::get(&enc, probe), Some(Col::Missing));
        }
    }
}

fn col_cmp<T: Ord>(a: &Option<T>, b: &Option<T>, d: Dir, n: Nulls) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) if n == Nulls::First => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) if n == Nulls::First => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) if d == Dir::Desc => y.cmp(x),
        (Some(x), Some(y)) => x.cmp(y),
    }
}

#[test]
fn fdb_single_terminator_is_not_prefix_free() {
    // why the bytes encoding uses a two-byte terminator: with a single 0x00
    // terminator [a] is a prefix of [a, 0], and inverting for desc breaks order.
    let single = |v: &[u8]| {
        let mut o = Vec::new();
        for &b in v {
            if b == 0 { o.extend_from_slice(&[0, 0xff]) } else { o.push(b) }
        }
        o.push(0);
        o
    };
    let (sa, sb) = ([0x61u8], [0x61u8, 0]);
    let (a, b) = (single(&sa), single(&sb));
    assert!(b.starts_with(&a));
    let inv = |v: Vec<u8>| v.into_iter().map(|x| !x).collect::<Vec<u8>>();
    // desc must put [a, 0] first, but the inverted single-terminator encoding puts [a] first.
    assert_eq!(inv(a).cmp(&inv(b)), Ordering::Less);
    let (a2, b2) = (esc(&sa), esc(&sb));
    assert!(!b2.starts_with(&a2));
    assert_eq!(inv(a2).cmp(&inv(b2)), Ordering::Greater);
}

#[test]
fn golden_vectors() {
    // the byte layouts quoted in the issue #16 comment.
    let mut k = vec![0x10];
    key::put_uvarint(104, &mut k);
    key::put_uvarint(1, &mut k);
    key::put_column(&mut k, Dir::Asc, Nulls::Last, Some(&key::i64_payload(7)));
    k.push(0x00);
    assert_eq!(k, [0x10, 0x68, 0x01, 0x01, 0x80, 0, 0, 0, 0, 0, 0, 0x07, 0x00]);

    let mut t = Vec::new();
    key::put_column(&mut t, Dir::Asc, Nulls::Last, Some(&esc(&[0x61, 0, 0x62])));
    assert_eq!(t, [0x01, 0x61, 0x00, 0xff, 0x62, 0x00, 0x01]);
    let mut d = Vec::new();
    key::put_column(&mut d, Dir::Desc, Nulls::First, None);
    assert_eq!(d, [0xfd]);
    let mut d = Vec::new();
    key::put_column(&mut d, Dir::Desc, Nulls::First, Some(&key::i32_payload(1)));
    assert_eq!(d, [0xfe, 0x7f, 0xff, 0xff, 0xfe]);

    assert_eq!(key::f64_payload(-0.0), key::f64_payload(0.0));
    assert_eq!(key::f64_payload(0.0), [0x80, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(key::f64_payload(f64::NAN), [0xff, 0xf8, 0, 0, 0, 0, 0, 0]);
    assert_eq!(key::f64_payload(-1.0), [0x40, 0x0f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
}

fn fin(neg: bool, mantissa: u128, scale: u32) -> Vec<u8> {
    num_enc(&Num::Finite { neg, mantissa, scale })
}

#[test]
fn golden_vectors_numeric_engine_row() {
    // 12.5 = 0.1250 * 100^1, centimal digits 12 and 50, bytes 2*12+1 and 2*50
    assert_eq!(fin(false, 125, 1), [0x04, 0x80, 0, 0, 0x01, 25, 100]);
    assert_eq!(fin(false, 12500, 3), fin(false, 125, 1));
    assert_eq!(fin(true, 125, 1), [0x02, 0x7f, 0xff, 0xff, 0xfe, 230, 155]);
    assert_eq!(fin(false, 0, 4), [0x03]);

    let mut v = Vec::new();
    key::put_uvarint(300, &mut v);
    assert_eq!(v, [0xf9, 0x01, 0x2c]);

    let ts = Timestamp { wall: 0x0102030405060708, logical: 9 };
    assert_eq!(engine::encode(&[0x6b], Some(ts)), [0x6b, 1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 9, 12]);
    assert_eq!(engine::encode(&[0x6b], None), [0x6b, 0]);

    let c2 = [0x2a, 0, 0, 0];
    let c5 = [0x68, 0x69];
    let r = row::encode(&[(2, Some(&c2[..])), (3, None), (5, Some(&c5[..]))]);
    assert_eq!(r, [0x01, 0x00, 2, 0, 1, 0, 2, 5, 3, 4, 0, 6, 0, 0x2a, 0, 0, 0, 0x68, 0x69]);
    assert_eq!(row::get(&r, 4), Some(Col::Missing));
    assert_eq!(row::get(&r, 3), Some(Col::Null));
}
