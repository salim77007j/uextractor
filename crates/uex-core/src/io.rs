//! Binary reading/writing helpers shared by all parsers (LE byte order like UE).

use crate::error::{Error, Result};
use std::io::{Read, Seek, SeekFrom, Write};

pub trait UexRead: Read {
    fn u8(&mut self) -> Result<u8> {
        let mut b = [0u8; 1];
        self.read_exact(&mut b)?;
        Ok(b[0])
    }
    fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }
    fn u16(&mut self) -> Result<u16> {
        let mut b = [0u8; 2];
        self.read_exact(&mut b)?;
        Ok(u16::from_le_bytes(b))
    }
    fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }
    fn u32(&mut self) -> Result<u32> {
        let mut b = [0u8; 4];
        self.read_exact(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn u64(&mut self) -> Result<u64> {
        let mut b = [0u8; 8];
        self.read_exact(&mut b)?;
        Ok(u64::from_le_bytes(b))
    }
    fn i64(&mut self) -> Result<i64> {
        Ok(self.u64()? as i64)
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.u64()?))
    }
    fn read_bytes(&mut self, len: usize) -> Result<Vec<u8>> {
        let mut v = vec![0u8; len];
        self.read_exact(&mut v)?;
        Ok(v)
    }
    /// UE FString: i32 length; negative = UTF-16 (incl. NUL), positive = UTF-8/ASCII (incl. NUL).
    fn fstring(&mut self) -> Result<String> {
        let len = self.i32()?;
        if len == 0 {
            return Ok(String::new());
        }
        if len < 0 {
            let count = (-len) as usize;
            if count > 1 << 26 {
                return Err(Error::malformed("fstring utf16 length out of range"));
            }
            let mut chars = Vec::with_capacity(count);
            for _ in 0..count {
                chars.push(self.u16()?);
            }
            let end = chars.iter().position(|&c| c == 0).unwrap_or(chars.len());
            Ok(String::from_utf16_lossy(&chars[..end]))
        } else {
            let len = len as usize;
            if len > 1 << 26 {
                return Err(Error::malformed("fstring utf8 length out of range"));
            }
            let bytes = self.read_bytes(len)?;
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
        }
    }
    /// UE serialized bool (i32 0/1)
    fn bool32(&mut self) -> Result<bool> {
        Ok(self.i32()? != 0)
    }
    fn fguid(&mut self) -> Result<[u8; 16]> {
        let mut g = [0u8; 16];
        self.read_exact(&mut g)?;
        Ok(g)
    }
}

impl<T: Read> UexRead for T {}

pub fn align(mut value: u64, alignment: u64) -> u64 {
    value = (value + alignment - 1) & !(alignment - 1);
    value
}

pub fn write_fstring(w: &mut impl Write, s: &str) -> Result<()> {
    if s.is_ascii() {
        w.write_all(&((s.len() as i32) + 1).to_le_bytes())?;
        w.write_all(s.as_bytes())?;
        w.write_all(&[0])?;
    } else {
        let units: Vec<u16> = s.encode_utf16().collect();
        let neg_len: i32 = -((units.len() as i32) + 1);
        w.write_all(&neg_len.to_le_bytes())?;
        for u in units {
            w.write_all(&u.to_le_bytes())?;
        }
        w.write_all(&0u16.to_le_bytes())?;
    }
    Ok(())
}

pub fn write_u64(w: &mut impl Write, v: u64) -> Result<()> {
    w.write_all(&v.to_le_bytes())?;
    Ok(())
}

pub fn read_to_vec(mut r: impl Read, len: usize) -> Result<Vec<u8>> {
    r.read_bytes(len)
}

pub fn seek_start(f: &mut (impl Read + Seek), off: u64) -> Result<()> {
    f.seek(SeekFrom::Start(off))?;
    Ok(())
}
