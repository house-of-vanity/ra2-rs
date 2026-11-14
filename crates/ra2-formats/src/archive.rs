//! Read-only CFL3 archives used for components and legacy UI resources.
use crate::components::normalize;
use flate2::read::DeflateDecoder;
use std::{collections::BTreeMap, io::Read};

fn u32_at(bytes: &[u8], offset: usize) -> Result<usize, String> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or("Truncated CFZ integer")?
            .try_into()
            .unwrap(),
    ) as usize)
}

fn inflate(bytes: &[u8], offset: usize) -> Result<Vec<u8>, String> {
    let size = u32_at(bytes, offset)?;
    let expected = u32_at(bytes, offset + 4)?;
    if expected > 16 * 1024 * 1024 || size < 6 {
        return Err("Invalid CFZ block size".into());
    }
    // CFL3 counts the uncompressed-size field as part of the block. Its stream
    // has a two-byte zlib header but no Adler checksum, followed immediately by
    // the next block. A conventional zlib decoder therefore reports a bad checksum.
    let end = offset
        .checked_add(4)
        .and_then(|v| v.checked_add(size))
        .ok_or("CFZ size overflow")?;
    let data = bytes.get(offset + 8..end).ok_or("Truncated CFZ block")?;
    if data.len() < 2
        || data[0] & 15 != 8
        || !((data[0] as u16) * 256 + data[1] as u16).is_multiple_of(31)
    {
        return Err("Invalid CFZ compression header".into());
    }
    let mut decoder = DeflateDecoder::new(&data[2..]);
    let mut out = vec![];
    (&mut decoder)
        .take(expected as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("CFZ deflate: {e}"))?;
    if out.len() != expected || decoder.total_in() as usize != data.len() - 2 {
        return Err("CFZ decompressed size/boundary mismatch".into());
    }
    Ok(out)
}

#[derive(Clone, Debug, Default)]
pub struct CfzArchive {
    pub files: BTreeMap<String, Vec<u8>>,
    /// Source spelling for writing references back to the native engine.
    pub names: BTreeMap<String, String>,
}
impl CfzArchive {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.get(..4) != Some(b"CFL3") {
            return Err("Expected CFL3 archive".into());
        }
        let directory = u32_at(bytes, 4)?;
        if u32_at(bytes, directory)? != 1 {
            return Err("Unsupported CFZ directory compression".into());
        }
        let data = inflate(bytes, directory + 4)?;
        if data.len() != u32_at(bytes, 8)? {
            return Err("CFZ directory size mismatch".into());
        }
        let mut files = BTreeMap::new();
        let mut names = BTreeMap::new();
        let mut pos = 0;
        let mut total = 0;
        while pos < data.len() {
            let size = u32_at(&data, pos)?;
            let offset = u32_at(&data, pos + 4)?;
            let flags = u32_at(&data, pos + 8)?;
            let length = u16::from_le_bytes(
                data.get(pos + 12..pos + 14)
                    .ok_or("Truncated CFZ filename")?
                    .try_into()
                    .unwrap(),
            ) as usize;
            pos += 14;
            let name = std::str::from_utf8(
                data.get(pos..pos + length)
                    .ok_or("Truncated CFZ filename")?,
            )
            .map_err(|_| "Invalid CFZ filename")?;
            pos += length;
            if flags != 1 {
                return Err(format!("Unsupported CFZ entry flags {flags}"));
            }
            let payload = inflate(bytes, offset)?;
            if payload.len() != size {
                return Err(format!("CFZ entry size mismatch: {name}"));
            }
            total += payload.len();
            if total > 128 * 1024 * 1024 {
                return Err("CFZ expanded archive exceeds 128 MiB".into());
            }
            let key = normalize(name);
            if key.split('/').any(|part| part == "..") || key.contains(':') {
                return Err("Invalid CFZ relative path".into());
            }
            names.insert(key.clone(), name.into());
            if files.insert(key, payload).is_some() {
                return Err("Duplicate CFZ entry".into());
            }
        }
        Ok(Self { files, names })
    }
    pub fn get(&self, name: &str) -> Result<&[u8], String> {
        self.files
            .get(&normalize(name))
            .map(Vec::as_slice)
            .ok_or_else(|| format!("Missing CFZ resource: {name}"))
    }
}
