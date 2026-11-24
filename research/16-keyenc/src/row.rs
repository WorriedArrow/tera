//! row value format v1. modeled on tidb's row format v2
//! (pingcap/tidb docs/design/2018-07-19-row-format.md): sorted column ids,
//! explicit null ids so an absent id means "use the catalog's missing value"
//! (postgres attmissingval, src/backend/access/common/heaptuple.c).

pub const FORMAT_V1: u8 = 0x01;
pub const FLAG_WIDE: u8 = 0x01;

#[derive(Debug, PartialEq, Eq)]
pub enum Col<'a> {
    Value(&'a [u8]),
    Null,
    Missing,
}

/// `cols` must be sorted by column id with no duplicates.
pub fn encode(cols: &[(u32, Option<&[u8]>)]) -> Vec<u8> {
    let present: Vec<(u32, &[u8])> = cols.iter().filter_map(|&(id, v)| v.map(|v| (id, v))).collect();
    let nulls: Vec<u32> = cols.iter().filter(|(_, v)| v.is_none()).map(|&(id, _)| id).collect();
    let data_len: usize = present.iter().map(|(_, v)| v.len()).sum();
    let max_id = cols.iter().map(|&(id, _)| id).max().unwrap_or(0);
    let wide = max_id > 0xff || data_len > 0xffff;
    let mut out = vec![FORMAT_V1, if wide { FLAG_WIDE } else { 0 }];
    out.extend_from_slice(&(present.len() as u16).to_le_bytes());
    out.extend_from_slice(&(nulls.len() as u16).to_le_bytes());
    let put_id = |out: &mut Vec<u8>, id: u32| {
        if wide {
            out.extend_from_slice(&id.to_le_bytes())
        } else {
            out.push(id as u8)
        }
    };
    for &(id, _) in &present {
        put_id(&mut out, id);
    }
    for &id in &nulls {
        put_id(&mut out, id);
    }
    let mut end = 0usize;
    for (_, v) in &present {
        end += v.len();
        if wide {
            out.extend_from_slice(&(end as u32).to_le_bytes())
        } else {
            out.extend_from_slice(&(end as u16).to_le_bytes())
        }
    }
    for (_, v) in &present {
        out.extend_from_slice(v);
    }
    out
}

pub fn get(row: &[u8], id: u32) -> Option<Col<'_>> {
    if *row.first()? != FORMAT_V1 {
        return None;
    }
    let wide = row.get(1)? & FLAG_WIDE != 0;
    let np = u16::from_le_bytes(row.get(2..4)?.try_into().ok()?) as usize;
    let nn = u16::from_le_bytes(row.get(4..6)?.try_into().ok()?) as usize;
    let (idw, offw) = if wide { (4, 4) } else { (1, 2) };
    let ids_at = 6;
    let read_id = |i: usize| -> Option<u32> {
        let p = ids_at + i * idw;
        Some(if wide {
            u32::from_le_bytes(row.get(p..p + 4)?.try_into().ok()?)
        } else {
            *row.get(p)? as u32
        })
    };
    let offs_at = ids_at + (np + nn) * idw;
    let data_at = offs_at + np * offw;
    let read_end = |i: usize| -> Option<usize> {
        let p = offs_at + i * offw;
        Some(if wide {
            u32::from_le_bytes(row.get(p..p + 4)?.try_into().ok()?) as usize
        } else {
            u16::from_le_bytes(row.get(p..p + 2)?.try_into().ok()?) as usize
        })
    };
    // binary search the sorted present ids, then the null ids.
    let (mut lo, mut hi) = (0, np);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let m = read_id(mid)?;
        if m == id {
            let start = if mid == 0 { 0 } else { read_end(mid - 1)? };
            let end = read_end(mid)?;
            return Some(Col::Value(row.get(data_at + start..data_at + end)?));
        }
        if m < id { lo = mid + 1 } else { hi = mid }
    }
    let (mut lo, mut hi) = (np, np + nn);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let m = read_id(mid)?;
        if m == id {
            return Some(Col::Null);
        }
        if m < id { lo = mid + 1 } else { hi = mid }
    }
    Some(Col::Missing)
}
