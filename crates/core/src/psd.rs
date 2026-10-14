//! Section-level PSD/PSB layout: just enough structure to replace image resources and global
//! tagged blocks while copying every other byte (layer records, channel data, merged image)
//! verbatim. Nothing is decoded or re-encoded.
//!
//! Layout from Adobe's "Photoshop File Formats Specification": header (26 bytes), color mode data
//! (`u32` length), image resources (`u32` length), layer and mask information (`u32`, or `u64` in
//! PSB), image data (rest of the file). The global tagged-block reading (8-byte lengths in PSB for
//! certain keys, padding detected by the next signature) follows photocraft-psd's `tagged.rs`.

use std::ops::Range;

use crate::error::{vec_with_capacity, Error, Result};
use crate::reader::{put_len, Rd};

/// Keys whose length field is 8 bytes in PSB files (the specification's list plus keys observed
/// with 8-byte lengths in real files; same list as photocraft-psd `PSB_LONG_KEYS`).
const PSB_LONG_KEYS: [&[u8; 4]; 21] = [
    b"LMsk", b"Lr16", b"Lr32", b"Layr", b"Mt16", b"Mt32", b"Mtrn", b"Alph", b"FMsk", b"lnk2", b"FEid", b"FXid", b"PxSD", b"lnk3", b"lnkE", b"pths", b"extd", b"extn",
    b"FELS", b"cinf", b"artd",
];

/// One image resource.
#[derive(Clone, Debug)]
pub struct Resource {
    pub id: u16,
    /// From the signature to the end of the padding.
    pub span: Range<usize>,
    /// Offset of the `u32` size field.
    pub size_at: usize,
    pub data: Range<usize>,
}

/// One global tagged block.
#[derive(Clone, Debug)]
pub struct Block {
    pub key: [u8; 4],
    pub long: bool,
    /// From the signature to the end of the padding.
    pub span: Range<usize>,
    pub data: Range<usize>,
    pub pad: Range<usize>,
}

/// The parsed layout of a PSD/PSB file.
#[derive(Clone, Debug)]
pub struct Layout {
    pub psb: bool,
    /// Offset of the image resources section's `u32` length.
    pub resources_at: usize,
    pub resources: Vec<Resource>,
    /// Offset of the layer and mask section's length field.
    pub layer_mask_at: usize,
    /// The section's content (after the length field).
    pub layer_mask: Range<usize>,
    /// Global tagged blocks (empty when the section has none).
    pub blocks: Vec<Block>,
    /// Bytes from the start of the section content to the first global block.
    pub blocks_start: usize,
    /// End of the last block (start of any trailing bytes in the section).
    pub blocks_end: usize,
}

fn is_sig(b: &[u8]) -> bool {
    b.starts_with(b"8BIM") || b.starts_with(b"8B64")
}

/// `true` if `bytes` start like a PSD (`version` 1) or PSB (`version` 2) file.
pub fn is_psd(bytes: &[u8]) -> bool {
    matches!(bytes.get(..6), Some([b'8', b'B', b'P', b'S', 0, 1 | 2]))
}

impl Layout {
    pub fn parse(bytes: &[u8]) -> Result<Layout> {
        let mut r = Rd::new(bytes, "PSD header");
        if r.array::<4>()? != *b"8BPS" {
            return Err(r.err("not a PSD/PSB file"));
        }
        let psb = match r.u16()? {
            1 => false,
            2 => true,
            v => return Err(r.err(format!("version {v}"))),
        };
        r.skip(20)?; // reserved, channels, height, width, depth, mode
        let cm = r.u32()?;
        r.skip(cm as usize)?;

        let resources_at = r.pos();
        let res_len = r.u32()? as usize;
        let res_end = r.pos().checked_add(res_len).filter(|&e| e <= bytes.len()).ok_or_else(|| r.err("image resources overrun the file"))?;
        let mut resources = Vec::new();
        let section = bytes.get(..res_end).ok_or_else(|| r.err("image resources"))?;
        let mut rr = Rd::at(section, r.pos(), "image resource");
        while rr.remaining() > 0 {
            let start = rr.pos();
            let sig = rr.array::<4>()?;
            if !matches!(&sig, b"8BIM" | b"MeSa" | b"AgHg" | b"PHUT" | b"DCSR") {
                return Err(rr.err(format!("resource signature {:?}", String::from_utf8_lossy(&sig))));
            }
            let id = rr.u16()?;
            let n = usize::from(rr.u8()?);
            rr.skip(n + (n + 1) % 2)?; // name padded to even, length byte included
            let size_at = rr.pos();
            let size = rr.u32()? as usize;
            let d0 = rr.pos();
            rr.skip(size)?;
            let data = d0..rr.pos();
            if size % 2 == 1 && rr.remaining() > 0 {
                rr.skip(1)?;
            }
            resources.push(Resource { id, span: start..rr.pos(), size_at, data });
        }
        let mut r = Rd::at(bytes, res_end, "layer and mask section");

        let layer_mask_at = r.pos();
        let lm_len = r.len_field(psb)?;
        let lm0 = r.pos();
        r.take_u64(lm_len)?;
        let layer_mask = lm0..r.pos();

        let mut blocks = Vec::new();
        let (mut blocks_start, mut blocks_end) = (layer_mask.end, layer_mask.end);
        if !layer_mask.is_empty() {
            let section = bytes.get(..layer_mask.end).ok_or_else(|| r.err("section"))?;
            let mut s = Rd::at(section, layer_mask.start, "layer and mask section");
            let li = s.len_field(psb)?;
            s.take_u64(li)?;
            if s.remaining() >= 4 {
                let gm = s.u32()?;
                s.skip(gm as usize)?;
                blocks_start = s.pos();
                blocks = read_blocks(&mut s, psb)?;
                blocks_end = s.pos();
            }
        }
        Ok(Layout { psb, resources_at, resources, layer_mask_at, layer_mask, blocks, blocks_start, blocks_end })
    }
}

/// Reads global blocks until the section ends or the bytes stop looking like a block (the rest
/// is kept as trailing data). Padding is the smallest k in 0..=3 after which the next signature
/// or the section end follows; otherwise default zero padding to 4 if present, else none.
fn read_blocks(s: &mut Rd<'_>, psb: bool) -> Result<Vec<Block>> {
    let section_end = s.len();
    let mut blocks = Vec::new();
    loop {
        let start = s.pos();
        let Some(rest) = s.rest() else { break };
        if rest.len() < 12 || !is_sig(rest) {
            break;
        }
        s.skip(4)?;
        let key = s.array::<4>()?;
        let long = psb && PSB_LONG_KEYS.contains(&&key);
        let len = s.len_field(long)?;
        let d0 = s.pos();
        s.take_u64(len)?;
        let data = d0..s.pos();
        let rest = s.rest().unwrap_or_default();
        let found = (0..=3usize).find(|&k| k == rest.len() || (k < rest.len() && rest.get(k..).is_some_and(is_sig)));
        let default = (4 - data.len() % 4) % 4;
        let pad = found.unwrap_or_else(|| if rest.get(..default).is_some_and(|p| p.iter().all(|&b| b == 0)) { default } else { 0 });
        s.skip(pad)?;
        blocks.push(Block { key, long, span: start..s.pos(), data, pad: s.pos() - pad..s.pos() });
        debug_assert!(s.pos() <= section_end);
    }
    Ok(blocks)
}

/// Replacement data for parts of a file.
#[derive(Default)]
pub struct Edits {
    /// (index into `Layout::resources`, new data)
    pub resources: Vec<(usize, Vec<u8>)>,
    /// (index into `Layout::blocks`, new data)
    pub blocks: Vec<(usize, Vec<u8>)>,
}

impl Edits {
    pub fn is_empty(&self) -> bool {
        self.resources.is_empty() && self.blocks.is_empty()
    }
}

/// Writes `bytes` with the edits applied: replaced resources and blocks get new length fields and
/// padding, the two enclosing section lengths are recomputed, and every other byte is copied.
pub fn rebuild(bytes: &[u8], layout: &Layout, edits: &Edits) -> Result<Vec<u8>> {
    let get = |r: Range<usize>| bytes.get(r).ok_or_else(|| Error::format("PSD layout", "offset out of range"));
    let delta: isize = edits.resources.iter().map(|(i, d)| d.len() as isize - layout.resources.get(*i).map_or(0, |r| r.data.len() as isize)).sum::<isize>()
        + edits.blocks.iter().map(|(i, d)| d.len() as isize - layout.blocks.get(*i).map_or(0, |b| b.data.len() as isize)).sum::<isize>();
    let cap = (bytes.len() as isize).saturating_add(delta).saturating_add(16 * (edits.blocks.len() + edits.resources.len()) as isize).max(0) as usize;
    let mut out = vec_with_capacity(cap)?;

    // Header and color mode data.
    out.extend_from_slice(get(0..layout.resources_at)?);

    // Image resources.
    let res_len_at = out.len();
    out.extend_from_slice(&[0; 4]);
    let res_start = out.len();
    for (i, res) in layout.resources.iter().enumerate() {
        match edits.resources.iter().find(|(j, _)| *j == i) {
            None => out.extend_from_slice(get(res.span.clone())?),
            Some((_, data)) => {
                out.extend_from_slice(get(res.span.start..res.size_at)?);
                put_len(&mut out, data.len(), false)?;
                out.extend_from_slice(data);
                if data.len() % 2 == 1 {
                    out.push(0);
                }
            }
        }
    }
    let res_len = u32::try_from(out.len() - res_start).map_err(|_| Error::TooLarge("image resources"))?;
    out.get_mut(res_len_at..res_len_at + 4).ok_or(Error::TooLarge("resources"))?.copy_from_slice(&res_len.to_be_bytes());

    // Layer and mask section.
    if edits.blocks.is_empty() {
        out.extend_from_slice(get(layout.layer_mask_at..layout.layer_mask.end)?);
    } else {
        let lw = if layout.psb { 8 } else { 4 };
        let lm_len_at = out.len();
        out.resize(out.len() + lw, 0);
        let lm_start = out.len();
        out.extend_from_slice(get(layout.layer_mask.start..layout.blocks_start)?);
        for (i, b) in layout.blocks.iter().enumerate() {
            match edits.blocks.iter().find(|(j, _)| *j == i) {
                None => out.extend_from_slice(get(b.span.clone())?),
                Some((_, data)) => {
                    out.extend_from_slice(get(b.span.start..b.span.start + 8)?);
                    put_len(&mut out, data.len(), b.long)?;
                    out.extend_from_slice(data);
                    if data.len() % 4 == b.data.len() % 4 {
                        out.extend_from_slice(get(b.pad.clone())?);
                    } else {
                        out.resize(out.len() + (4 - data.len() % 4) % 4, 0);
                    }
                }
            }
        }
        out.extend_from_slice(get(layout.blocks_end..layout.layer_mask.end)?);
        let mut len = Vec::with_capacity(8);
        put_len(&mut len, out.len() - lm_start, layout.psb)?;
        out.get_mut(lm_len_at..lm_len_at + lw).ok_or(Error::TooLarge("layer section"))?.copy_from_slice(&len);
    }

    // Image data.
    out.extend_from_slice(get(layout.layer_mask.end..bytes.len())?);
    Ok(out)
}
