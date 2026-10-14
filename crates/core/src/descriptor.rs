//! Skips an ActionDescriptor without building it, to find what follows it.
//!
//! Layout from Adobe's public "Photoshop File Formats Specification" (Descriptor structure); the
//! set of OSTypes matches the one photocraft-psd's `descriptor.rs` reads.

use crate::error::Result;
use crate::reader::Rd;

const MAX_DEPTH: usize = 64;

/// Skips a versioned descriptor (`u32` version 16 + descriptor) starting at the cursor.
pub(crate) fn skip_versioned(r: &mut Rd<'_>) -> Result<()> {
    let version = r.u32()?;
    if version != 16 {
        return Err(r.err(format!("descriptor version {version}, expected 16")));
    }
    skip_descriptor(r, 0)
}

fn skip_id(r: &mut Rd<'_>) -> Result<()> {
    let n = r.u32()?;
    r.skip(if n == 0 { 4 } else { n as usize })
}

fn skip_class(r: &mut Rd<'_>) -> Result<()> {
    r.skip_unicode()?;
    skip_id(r)
}

fn skip_descriptor(r: &mut Rd<'_>, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(r.err("descriptor nesting too deep"));
    }
    skip_class(r)?;
    let n = r.u32()?;
    for _ in 0..n {
        skip_id(r)?;
        skip_value(r, depth + 1)?;
    }
    Ok(())
}

fn skip_value(r: &mut Rd<'_>, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(r.err("descriptor nesting too deep"));
    }
    let ty = r.array::<4>()?;
    match &ty {
        b"obj " => {
            let n = r.u32()?;
            for _ in 0..n {
                skip_reference_item(r)?;
            }
        }
        b"Objc" | b"GlbO" => skip_descriptor(r, depth + 1)?,
        b"VlLs" => {
            let n = r.u32()?;
            for _ in 0..n {
                skip_value(r, depth + 1)?;
            }
        }
        b"doub" | b"comp" => r.skip(8)?,
        b"UntF" => r.skip(12)?,
        b"UnFl" => {
            r.skip(4)?;
            let n = r.u32()?;
            r.skip((n as usize).checked_mul(8).ok_or_else(|| r.err("UnFl count overflow"))?)?;
        }
        b"TEXT" => r.skip_unicode()?,
        b"enum" => {
            skip_id(r)?;
            skip_id(r)?;
        }
        b"long" => r.skip(4)?,
        b"bool" => r.skip(1)?,
        b"type" | b"GlbC" => skip_class(r)?,
        b"alis" | b"Pth " | b"tdta" => {
            let n = r.u32()?;
            r.skip(n as usize)?;
        }
        b"ObAr" => {
            r.skip(4)?;
            skip_descriptor(r, depth + 1)?;
        }
        other => return Err(r.err(format!("unknown descriptor OSType {:?}", String::from_utf8_lossy(other)))),
    }
    Ok(())
}

fn skip_reference_item(r: &mut Rd<'_>) -> Result<()> {
    let ty = r.array::<4>()?;
    match &ty {
        b"prop" => {
            skip_class(r)?;
            skip_id(r)?;
        }
        b"Clss" => skip_class(r)?,
        b"Enmr" => {
            skip_class(r)?;
            skip_id(r)?;
            skip_id(r)?;
        }
        b"rele" => {
            skip_class(r)?;
            r.skip(4)?;
        }
        b"Idnt" | b"indx" => r.skip(4)?,
        b"name" => {
            skip_class(r)?;
            r.skip_unicode()?;
        }
        other => return Err(r.err(format!("unknown reference item {:?}", String::from_utf8_lossy(other)))),
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
pub(crate) mod tests {
    use super::*;

    fn unicode(s: &str) -> Vec<u8> {
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut v = (units.len() as u32).to_be_bytes().to_vec();
        for u in units {
            v.extend_from_slice(&u.to_be_bytes());
        }
        v
    }

    fn code(c: &[u8; 4]) -> Vec<u8> {
        let mut v = vec![0, 0, 0, 0];
        v.extend_from_slice(c);
        v
    }

    /// A versioned descriptor exercising most OSTypes.
    pub fn sample() -> Vec<u8> {
        let mut d = 16u32.to_be_bytes().to_vec();
        d.extend(unicode(""));
        d.extend(code(b"null"));
        let items: Vec<(Vec<u8>, Vec<u8>)> = vec![
            (code(b"Nm  "), [b"TEXT".to_vec(), unicode("page 1")].concat()),
            (code(b"Wdth"), [b"UntF".to_vec(), b"#Pxl".to_vec(), 2.0f64.to_be_bytes().to_vec()].concat()),
            (code(b"Lst "), [b"VlLs".to_vec(), 2u32.to_be_bytes().to_vec(), b"long".to_vec(), vec![0, 0, 0, 1], b"bool".to_vec(), vec![1]].concat()),
            (code(b"Enm "), [b"enum".to_vec(), code(b"Ordn"), code(b"Trgt")].concat()),
            (code(b"Raw "), [b"tdta".to_vec(), 3u32.to_be_bytes().to_vec(), vec![1, 2, 3]].concat()),
            (
                code(b"Ref "),
                [b"obj ".to_vec(), 2u32.to_be_bytes().to_vec(), b"Idnt".to_vec(), vec![0, 0, 0, 9], b"Clss".to_vec(), unicode(""), code(b"Lyr ")].concat(),
            ),
            (code(b"Obj "), [b"Objc".to_vec(), unicode(""), code(b"Pnt "), 1u32.to_be_bytes().to_vec(), code(b"Hrzn"), b"doub".to_vec(), vec![0; 8]].concat()),
            (
                code(b"UnFl"),
                [b"UnFl".to_vec(), b"#Pxl".to_vec(), 2u32.to_be_bytes().to_vec(), vec![0; 16]].concat(),
            ),
        ];
        d.extend((items.len() as u32).to_be_bytes());
        for (k, v) in items {
            d.extend(k);
            d.extend(v);
        }
        d
    }

    #[test]
    fn skips_exactly_one_descriptor() {
        let mut d = sample();
        let len = d.len();
        d.extend_from_slice(b"TAIL");
        let mut r = Rd::new(&d, "test");
        skip_versioned(&mut r).unwrap();
        assert_eq!(r.pos(), len);
    }

    #[test]
    fn truncation_never_panics() {
        let d = sample();
        for n in 0..d.len() {
            assert!(skip_versioned(&mut Rd::new(&d[..n], "test")).is_err(), "prefix {n}");
        }
    }

    #[test]
    fn rejects_unknown_type_and_bad_version() {
        let mut d = sample();
        d[0..4].copy_from_slice(&15u32.to_be_bytes());
        assert!(skip_versioned(&mut Rd::new(&d, "test")).is_err());
        let mut bad = 16u32.to_be_bytes().to_vec();
        bad.extend(unicode(""));
        bad.extend(code(b"null"));
        bad.extend(1u32.to_be_bytes());
        bad.extend(code(b"key "));
        bad.extend_from_slice(b"????");
        assert!(skip_versioned(&mut Rd::new(&bad, "test")).is_err());
    }

    #[test]
    fn deep_nesting_is_an_error() {
        let mut d = 16u32.to_be_bytes().to_vec();
        for _ in 0..200 {
            d.extend(unicode(""));
            d.extend(code(b"null"));
            d.extend(1u32.to_be_bytes());
            d.extend(code(b"key "));
            d.extend_from_slice(b"Objc");
        }
        assert!(skip_versioned(&mut Rd::new(&d, "test")).is_err());
    }
}
