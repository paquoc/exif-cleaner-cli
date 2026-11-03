//! Linked-layer blocks (`lnk2` / `lnk3` / `lnkD`): find embedded smart-object files and replace
//! their payload while keeping every other byte of the record.
//!
//! Layout (Adobe "Photoshop File Formats Specification", Linked Layer): a sequence of items, each
//! a big-endian `u64` length + item data, padded to a multiple of 4. Item: kind (`liFD` embedded,
//! `liFE` external, `liFA` alias), `u32` version (1–8), Pascal uuid, Unicode file name, file type,
//! creator, `u64` data length, open-descriptor flag (+ versioned descriptor), then for `liFD` the
//! file bytes, followed by version-specific fields: child document id (v5+, Unicode string),
//! asset modification time (v6+, 8 bytes), asset lock state (v7+, 1 byte), versioned descriptor
//! holding the `contentID` (v8+, Photoshop 2025 and later). Any bytes after those are kept
//! verbatim as extension bytes.

use std::ops::Range;

use crate::descriptor;
use crate::error::{vec_with_capacity, Error, Result};
use crate::reader::Rd;

/// One item's framing inside a block, as absolute offsets into the block data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawItem {
    /// Offset of the `u64` length prefix.
    pub start: usize,
    /// The item data covered by the length.
    pub body: Range<usize>,
    /// Padding bytes after the item.
    pub pad: Range<usize>,
    /// `liFD`, `liFE` or `liFA`.
    pub kind: [u8; 4],
    /// Raw uuid bytes (Pascal string).
    pub uuid: Vec<u8>,
}

/// An embedded (`liFD`) file record, with the offsets needed to replace its payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Embedded {
    /// Index into the block's items.
    pub index: usize,
    pub version: u32,
    pub uuid: Vec<u8>,
    pub file_name: String,
    pub file_type: [u8; 4],
    /// Absolute offset of the `u64` data length field.
    pub data_len_at: usize,
    /// Absolute range of the file bytes.
    pub payload: Range<usize>,
}

const ITEM: &str = "linked-layer item";

fn required_pad(len: usize) -> usize {
    (4 - len % 4) % 4
}

/// Splits a block into items, verifying framing and that each item's identity (kind, version,
/// uuid) is readable. A final item may omit its padding entirely; any other shortfall, and any
/// tail too short to be an item, is an error.
pub fn split_items(block: &[u8]) -> Result<Vec<RawItem>> {
    let mut items = Vec::new();
    let mut r = Rd::new(block, "linked-layer block");
    while r.remaining() > 0 {
        let start = r.pos();
        if r.remaining() < 8 {
            return Err(r.err(format!("{} trailing bytes do not form an item", r.remaining())));
        }
        let len = r.u64()?;
        let body_start = r.pos();
        r.take_u64(len)?;
        let body = body_start..r.pos();
        let need = required_pad(body.len());
        let pad = if r.remaining() >= need {
            r.skip(need)?;
            body.end..r.pos()
        } else if r.remaining() == 0 {
            body.end..body.end
        } else {
            return Err(r.err("item padding is truncated"));
        };
        let item = block.get(body.clone()).ok_or_else(|| r.err("item out of range"))?;
        let mut ir = Rd::new(item, ITEM);
        let kind = ir.array::<4>()?;
        if !matches!(&kind, b"liFD" | b"liFE" | b"liFA") {
            return Err(ir.err(format!("unknown item kind {:?}", String::from_utf8_lossy(&kind))));
        }
        let _version = ir.u32()?;
        let n = ir.u8()?;
        let uuid = ir.take(usize::from(n))?.to_vec();
        items.push(RawItem { start, body, pad, kind, uuid });
    }
    Ok(items)
}

/// Parses the embedded record of `item` (which must be `liFD`), checking its version-specific
/// fields after the payload.
pub fn parse_embedded(block: &[u8], index: usize, item: &RawItem) -> Result<Embedded> {
    if &item.kind != b"liFD" {
        return Err(Error::Invalid(format!("item {:?} is {}, not an embedded file", String::from_utf8_lossy(&item.uuid), String::from_utf8_lossy(&item.kind))));
    }
    let body = block.get(item.body.clone()).ok_or_else(|| Error::format(ITEM, "out of range"))?;
    let base = item.body.start;
    let mut r = Rd::new(body, ITEM);
    r.skip(4)?;
    let version = r.u32()?;
    if !(1..=8).contains(&version) {
        return Err(Error::Unsupported(format!("embedded file record version {version}")));
    }
    let n = r.u8()?;
    let uuid = r.take(usize::from(n))?.to_vec();
    let file_name = r.unicode()?;
    let file_type = r.array::<4>()?;
    r.skip(4)?; // creator
    let data_len_at = r.pos();
    let len = r.u64()?;
    if r.u8()? != 0 {
        descriptor::skip_versioned(&mut r)?;
    }
    let p0 = r.pos();
    r.take_u64(len)?;
    let p1 = r.pos();
    if version >= 5 {
        r.skip_unicode()?; // child document id
    }
    if version >= 6 {
        r.skip(8)?; // asset modification time
    }
    if version >= 7 {
        r.skip(1)?; // asset locked state
    }
    if version >= 8 {
        descriptor::skip_versioned(&mut r)?; // contentID descriptor
    }
    Ok(Embedded { index, version, uuid, file_name, file_type, data_len_at: base + data_len_at, payload: base + p0..base + p1 })
}

/// Rebuilds `block` with the payloads of the given embedded records replaced. Everything else,
/// including the order and bytes of untouched items and each replaced record's prefix and
/// suffix, is copied verbatim; only the payload, its data length, the item length and (when the
/// alignment changes) the item padding are rewritten.
pub fn replace_payloads(block: &[u8], items: &[RawItem], replacements: &[(&Embedded, &[u8])]) -> Result<Vec<u8>> {
    let get = |r: Range<usize>| block.get(r).ok_or_else(|| Error::format(ITEM, "offset out of range"));
    // Output size, checked.
    let mut total = block.len();
    for (e, new) in replacements {
        let item = items.get(e.index).ok_or_else(|| Error::Invalid("replacement for an unknown item".into()))?;
        let new_body = item.body.len().checked_sub(e.payload.len()).and_then(|b| b.checked_add(new.len())).ok_or(Error::TooLarge("item"))?;
        total = total.checked_sub(item.pad.end - item.start).and_then(|t| t.checked_add(8 + new_body + 3)).ok_or(Error::TooLarge("block"))?;
    }
    let mut out = vec_with_capacity(total)?;
    for (i, item) in items.iter().enumerate() {
        let Some((e, new)) = replacements.iter().find(|(e, _)| e.index == i) else {
            out.extend_from_slice(get(item.start..item.pad.end)?);
            continue;
        };
        let rel_len = e.data_len_at.checked_sub(item.body.start).ok_or_else(|| Error::format(ITEM, "bad data length offset"))?;
        let new_body_len = item.body.len() - e.payload.len() + new.len();
        out.extend_from_slice(&(new_body_len as u64).to_be_bytes());
        out.extend_from_slice(get(item.body.start..item.body.start + rel_len)?);
        out.extend_from_slice(&(new.len() as u64).to_be_bytes());
        out.extend_from_slice(get(e.data_len_at + 8..e.payload.start)?);
        out.extend_from_slice(new);
        out.extend_from_slice(get(e.payload.end..item.body.end)?);
        let old_pad = get(item.pad.clone())?;
        let unpadded_last = old_pad.is_empty() && required_pad(item.body.len()) != 0;
        if unpadded_last {
            // Keep the file's style: the last item carried no padding.
        } else if new_body_len % 4 == item.body.len() % 4 {
            out.extend_from_slice(old_pad);
        } else {
            out.resize(out.len() + required_pad(new_body_len), 0);
        }
    }
    Ok(out)
}

/// Replaces the payload of the embedded file `uuid` in one linked-layer block's data (without
/// the tagged block's signature, key, length or padding; big-endian PSD layout only).
///
/// * `Ok(Some(data))`: the record was found and its payload replaced.
/// * `Ok(None)`: the record was found and already holds `new_payload`; keep the old bytes.
/// * `Err`: empty, missing, duplicate or non-UTF-8 uuid, a target that is not `liFD`, an
///   unsupported record version, malformed input or an allocation failure.
pub fn replace_embedded_payload(block_data: &[u8], uuid: &str, new_payload: &[u8]) -> Result<Option<Vec<u8>>> {
    if uuid.is_empty() {
        return Err(Error::Invalid("empty uuid".into()));
    }
    let items = split_items(block_data)?;
    let mut found = None;
    for (i, item) in items.iter().enumerate() {
        let id = std::str::from_utf8(&item.uuid).map_err(|_| Error::format(ITEM, format!("uuid of item {i} is not UTF-8")))?;
        if id == uuid {
            if found.is_some() {
                return Err(Error::Invalid(format!("uuid {uuid} occurs more than once")));
            }
            found = Some(i);
        }
    }
    let i = found.ok_or_else(|| Error::Invalid(format!("uuid {uuid} not found")))?;
    let item = items.get(i).ok_or_else(|| Error::Invalid("item index".into()))?;
    let e = parse_embedded(block_data, i, item)?;
    if block_data.get(e.payload.clone()) == Some(new_payload) {
        return Ok(None);
    }
    replace_payloads(block_data, &items, &[(&e, new_payload)]).map(Some)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
pub(crate) mod tests {
    use super::*;

    /// Fields of a synthetic `liFD` record.
    #[derive(Clone)]
    pub struct Rec {
        pub kind: [u8; 4],
        pub version: u32,
        pub uuid: Vec<u8>,
        pub name: String,
        pub descriptor: Option<Vec<u8>>,
        pub payload: Vec<u8>,
        pub child_id: String,
        pub mtime: f64,
        pub locked: u8,
        pub content: Vec<u8>,
        pub extension: Vec<u8>,
        pub pad: Option<Vec<u8>>,
    }

    impl Rec {
        pub fn new(uuid: &str, payload: Vec<u8>) -> Self {
            Rec {
                kind: *b"liFD",
                version: 7,
                uuid: uuid.as_bytes().to_vec(),
                name: "Ảnh ✓.psb".into(),
                descriptor: None,
                payload,
                child_id: "child-doc".into(),
                mtime: 12345.5,
                locked: 1,
                content: crate::descriptor::tests::sample(),
                extension: vec![],
                pad: None,
            }
        }

        pub fn encode(&self) -> Vec<u8> {
            let mut item = self.kind.to_vec();
            item.extend(self.version.to_be_bytes());
            item.push(self.uuid.len() as u8);
            item.extend(&self.uuid);
            let units: Vec<u16> = self.name.encode_utf16().chain([0]).collect();
            item.extend((units.len() as u32).to_be_bytes());
            units.iter().for_each(|u| item.extend(u.to_be_bytes()));
            item.extend_from_slice(b"8BPB8BIM");
            item.extend((self.payload.len() as u64).to_be_bytes());
            match &self.descriptor {
                Some(d) => {
                    item.push(1);
                    item.extend(d);
                }
                None => item.push(0),
            }
            item.extend(&self.payload);
            if self.version >= 5 {
                let units: Vec<u16> = self.child_id.encode_utf16().collect();
                item.extend((units.len() as u32).to_be_bytes());
                units.iter().for_each(|u| item.extend(u.to_be_bytes()));
            }
            if self.version >= 6 {
                item.extend(self.mtime.to_be_bytes());
            }
            if self.version >= 7 {
                item.push(self.locked);
            }
            if self.version >= 8 {
                item.extend(&self.content);
            }
            item.extend(&self.extension);
            let mut out = (item.len() as u64).to_be_bytes().to_vec();
            out.extend(&item);
            match &self.pad {
                Some(p) => out.extend(p),
                None => out.resize(out.len() + required_pad(item.len()), 0),
            }
            out
        }
    }

    fn item_len(enc: &[u8]) -> usize {
        u64::from_be_bytes(enc[..8].try_into().unwrap()) as usize
    }

    fn payload_of(block: &[u8], uuid: &str) -> Vec<u8> {
        let items = split_items(block).unwrap();
        let (i, item) = items.iter().enumerate().find(|(_, it)| it.uuid == uuid.as_bytes()).unwrap();
        let e = parse_embedded(block, i, item).unwrap();
        block[e.payload].to_vec()
    }

    #[test]
    fn replaces_only_the_payload_across_versions_and_sizes() {
        for version in 1..=8 {
            for desc in [None, Some(crate::descriptor::tests::sample())] {
                for old_len in 0..5usize {
                    for new_len in [0usize, 1, 2, 3, 4, 5, 9, 100] {
                        let mut a = Rec::new("aaa", vec![1; 7]);
                        a.version = version;
                        let mut t = Rec::new("target", vec![0xAB; old_len]);
                        t.version = version;
                        t.descriptor = desc.clone();
                        t.extension = vec![9, 8, 7];
                        let c = Rec::new("ccc", vec![3; 2]);
                        let block = [a.encode(), t.encode(), c.encode()].concat();
                        let new = vec![0xCD; new_len];
                        let out = replace_embedded_payload(&block, "target", &new).unwrap();
                        if new_len == old_len && old_len == 0 {
                            assert!(out.is_none());
                            continue;
                        }
                        let out = out.unwrap();
                        let mut expect = t.clone();
                        expect.payload = new.clone();
                        assert_eq!(out, [a.encode(), expect.encode(), c.encode()].concat(), "v{version} {old_len}->{new_len}");
                        assert_eq!(payload_of(&out, "target"), new);
                    }
                }
            }
        }
    }

    #[test]
    fn same_payload_is_none() {
        let block = [Rec::new("a", vec![1, 2, 3]).encode(), Rec::new("b", vec![4]).encode()].concat();
        assert_eq!(replace_embedded_payload(&block, "a", &[1, 2, 3]).unwrap(), None);
    }

    #[test]
    fn nonzero_padding_kept_when_alignment_unchanged() {
        let mut t = Rec::new("t", vec![1; 5]);
        let natural = required_pad(item_len(&t.encode()));
        assert!(natural > 0);
        t.pad = Some(vec![0xEE; natural]);
        let block = [t.encode(), Rec::new("z", vec![]).encode()].concat();
        let out = replace_embedded_payload(&block, "t", &[2; 9]).unwrap().unwrap();
        let mut e = t.clone();
        e.payload = vec![2; 9];
        assert_eq!(out, [e.encode(), Rec::new("z", vec![]).encode()].concat());
        // Alignment changes: fresh zero padding.
        let out = replace_embedded_payload(&block, "t", &[2; 6]).unwrap().unwrap();
        e.payload = vec![2; 6];
        e.pad = None;
        assert_eq!(out, [e.encode(), Rec::new("z", vec![]).encode()].concat());
    }

    #[test]
    fn unpadded_last_item_stays_unpadded() {
        let mut t = Rec::new("t", vec![1; 5]);
        t.pad = Some(vec![]);
        let block = t.encode();
        assert!(item_len(&block) % 4 != 0);
        let out = replace_embedded_payload(&block, "t", &[3; 2]).unwrap().unwrap();
        t.payload = vec![3; 2];
        assert_eq!(out, t.encode());
    }

    #[test]
    fn identity_errors() {
        let block = [Rec::new("a", vec![1]).encode(), Rec::new("a", vec![2]).encode()].concat();
        assert!(replace_embedded_payload(&block, "a", &[]).is_err(), "duplicate");
        assert!(replace_embedded_payload(&block, "", &[]).is_err(), "empty");
        assert!(replace_embedded_payload(&block, "nope", &[]).is_err(), "missing");
        let mut bad = Rec::new("x", vec![]);
        bad.uuid = vec![0xFF, 0xFE];
        assert!(replace_embedded_payload(&[bad.encode(), Rec::new("b", vec![]).encode()].concat(), "b", &[1]).is_err(), "non-UTF-8 uuid");
        for kind in [*b"liFE", *b"liFA"] {
            let mut r = Rec::new("e", vec![1]);
            r.kind = kind;
            assert!(replace_embedded_payload(&r.encode(), "e", &[2]).is_err());
        }
        for version in [0, 9, 99] {
            let mut r = Rec::new("v", vec![1]);
            r.version = version;
            assert!(replace_embedded_payload(&r.encode(), "v", &[2]).is_err());
        }
    }

    #[test]
    fn truncation_sweep_never_panics_or_returns_partial_data() {
        let mut t = Rec::new("t", vec![5; 11]);
        t.descriptor = Some(crate::descriptor::tests::sample());
        let block = [Rec::new("a", vec![1; 3]).encode(), t.encode()].concat();
        let unpadded = block.len() - required_pad(item_len(&t.encode()));
        for n in 0..block.len() {
            let cut = &block[..n];
            if n == unpadded && n != block.len() {
                // A last item without its padding is accepted (Photoshop's own style).
                assert!(replace_embedded_payload(cut, "t", &[9]).unwrap().is_some());
                continue;
            }
            // Must never panic; a truncated block either errors or (when cut at an item boundary
            // that leaves the target out) reports the target missing.
            assert!(replace_embedded_payload(cut, "t", &[9]).is_err(), "prefix {n}");
        }
        // Tail garbage after full items.
        let mut tail = block.clone();
        tail.extend([0, 0, 0]);
        assert!(replace_embedded_payload(&tail, "t", &[9]).is_err());
        // Length overflow.
        let mut huge = block.clone();
        huge[..8].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(replace_embedded_payload(&huge, "t", &[9]).is_err());
        // Missing suffix: v7 record cut before the lock byte, length adjusted.
        let mut short = Rec::new("s", vec![1]);
        short.locked = 0;
        let mut enc = short.encode();
        let len = u64::from_be_bytes(enc[..8].try_into().unwrap()) as usize;
        enc.truncate(8 + len - 1);
        enc[..8].copy_from_slice(&((len - 1) as u64).to_be_bytes());
        enc.resize(enc.len().next_multiple_of(4), 0);
        assert!(replace_embedded_payload(&enc, "s", &[2]).is_err());
    }
}
