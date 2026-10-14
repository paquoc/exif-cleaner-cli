//! A bounds-checked big-endian cursor. Every read is `get()`-based and returns `Err` on truncation.

use crate::error::{Error, Result};

pub(crate) struct Rd<'a> {
    d: &'a [u8],
    p: usize,
    what: &'static str,
}

impl<'a> Rd<'a> {
    pub fn new(d: &'a [u8], what: &'static str) -> Self {
        Rd { d, p: 0, what }
    }

    pub fn at(d: &'a [u8], p: usize, what: &'static str) -> Self {
        Rd { d, p, what }
    }

    pub fn pos(&self) -> usize {
        self.p
    }

    pub fn len(&self) -> usize {
        self.d.len()
    }

    pub fn remaining(&self) -> usize {
        self.d.len().saturating_sub(self.p)
    }

    /// The unread bytes.
    pub fn rest(&self) -> Option<&'a [u8]> {
        self.d.get(self.p..)
    }

    pub fn err(&self, detail: impl std::fmt::Display) -> Error {
        Error::format(self.what, format!("{detail} (at byte {})", self.p))
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.p.checked_add(n).ok_or_else(|| self.err("length overflow"))?;
        let s = self.d.get(self.p..end).ok_or_else(|| self.err(format!("truncated: need {n} bytes, {} left", self.remaining())))?;
        self.p = end;
        Ok(s)
    }

    pub fn take_u64(&mut self, n: u64) -> Result<&'a [u8]> {
        let n = usize::try_from(n).map_err(|_| self.err("length does not fit in memory"))?;
        self.take(n)
    }

    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let s = self.take(N)?;
        let mut a = [0u8; N];
        a.copy_from_slice(s);
        Ok(a)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    /// A 4- or 8-byte length field.
    pub fn len_field(&mut self, long: bool) -> Result<u64> {
        if long {
            self.u64()
        } else {
            self.u32().map(u64::from)
        }
    }

    /// Skips a Unicode string: `u32` count of UTF-16 code units, then the units.
    pub fn skip_unicode(&mut self) -> Result<()> {
        let n = self.u32()?;
        let bytes = u64::from(n).checked_mul(2).ok_or_else(|| self.err("unicode length overflow"))?;
        self.take_u64(bytes).map(|_| ())
    }

    /// Reads a Unicode string (lossy, trailing NULs trimmed).
    pub fn unicode(&mut self) -> Result<String> {
        let n = self.u32()?;
        let bytes = u64::from(n).checked_mul(2).ok_or_else(|| self.err("unicode length overflow"))?;
        let raw = self.take_u64(bytes)?;
        let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_be_bytes(c.try_into().unwrap_or([0; 2]))).collect();
        Ok(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string())
    }
}

/// Appends a big-endian length field of 4 or 8 bytes, refusing values that do not fit.
pub(crate) fn put_len(out: &mut Vec<u8>, n: usize, long: bool) -> Result<()> {
    if long {
        out.extend_from_slice(&(n as u64).to_be_bytes());
    } else {
        let n = u32::try_from(n).map_err(|_| Error::TooLarge("length exceeds 4 GB; the document needs PSB"))?;
        out.extend_from_slice(&n.to_be_bytes());
    }
    Ok(())
}
