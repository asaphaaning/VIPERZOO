//! Read one member from the validated KRU offset-table archive format.
//!
//! The archive directory names offsets, while each member ends at the next
//! directory offset (or end of file). [`read_entry`] validates the complete
//! directory before returning any member, preventing a malformed client file
//! from turning an unchecked offset into a plausible collision table.

use std::{fs, path::Path};

use thiserror::Error;

const ROW_SIZE: usize = 17;
const NAME_SIZE: usize = 13;

/// Extracts one case-insensitive member from a KRU data archive.
///
/// # Errors
///
/// Returns [`enum@Error`] for I/O, malformed directories, invalid names, or a
/// missing member.
pub fn read_entry(path: &Path, wanted: &str) -> Result<Vec<u8>, Error> {
    let data = fs::read(path).map_err(|source| Error::Read {
        path: path.to_owned(),
        source,
    })?;
    let count = usize::try_from(read_u32(&data, 0).ok_or(Error::Truncated)?).unwrap_or(usize::MAX);

    if !(2..=4_096).contains(&count) {
        return Err(Error::Count(count));
    }

    let directory_end = 4_usize
        .checked_add(count.checked_mul(ROW_SIZE).ok_or(Error::Truncated)?)
        .ok_or(Error::Truncated)?;

    if directory_end > data.len() {
        return Err(Error::Truncated);
    }

    let mut entries = Vec::with_capacity(count);

    for index in 0..count {
        let position = 4 + index * ROW_SIZE;
        let offset = usize::try_from(read_u32(&data, position).ok_or(Error::Truncated)?)
            .unwrap_or(usize::MAX);
        let raw_name = &data[position + 4..position + 4 + NAME_SIZE];
        let name_end = raw_name
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(NAME_SIZE);
        let name = std::str::from_utf8(&raw_name[..name_end])
            .map_err(|_| Error::Name(index))?
            .to_owned();

        entries.push((offset, name));
    }

    let offsets_valid = entries.windows(2).all(|pair| pair[0].0 <= pair[1].0)
        && entries
            .first()
            .is_some_and(|entry| entry.0 >= directory_end)
        && entries.last().is_some_and(|entry| entry.0 <= data.len());

    if !offsets_valid {
        return Err(Error::Offsets);
    }

    for (index, (offset, name)) in entries.iter().enumerate() {
        if !name.eq_ignore_ascii_case(wanted) {
            continue;
        }

        let end = entries.get(index + 1).map_or(data.len(), |entry| entry.0);

        return data
            .get(*offset..end)
            .map(<[u8]>::to_vec)
            .ok_or(Error::Extent(index));
    }

    Err(Error::Missing(wanted.into()))
}

fn read_u32(data: &[u8], position: usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(position..position + 4)?.try_into().ok()?;

    Some(u32::from_le_bytes(bytes))
}

/// KRU archive decode failure.
#[derive(Debug, Error)]
pub enum Error {
    /// Archive file could not be read.
    #[error("unable to read client archive {path}: {source}")]
    Read {
        /// Archive path.
        path: std::path::PathBuf,
        /// Filesystem failure.
        source: std::io::Error,
    },
    /// Archive ended inside its header or directory.
    #[error("client archive directory is truncated")]
    Truncated,
    /// Directory count is outside the validated format range.
    #[error("client archive has implausible entry count {0}")]
    Count(usize),
    /// A directory member name was not ASCII/UTF-8.
    #[error("client archive entry {0} has an invalid name")]
    Name(usize),
    /// Directory offsets are unsorted or outside the file.
    #[error("client archive has invalid member offsets")]
    Offsets,
    /// A named member has an invalid extent.
    #[error("client archive entry {0} has an invalid extent")]
    Extent(usize),
    /// Requested member was absent.
    #[error("client archive does not contain {0:?}")]
    Missing(Box<str>),
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn next_offset_bounds_member_extent() {
        let first = b"first";
        let wanted = b"sobj-payload";
        let count = 3_u32;
        let directory_end = 4 + usize::try_from(count).expect("small count") * ROW_SIZE;
        let entries = [
            (directory_end, "tile.tbl"),
            (directory_end + first.len(), "SObj.tbl"),
            (directory_end + first.len() + wanted.len(), ""),
        ];
        let mut data = count.to_le_bytes().to_vec();

        for (offset, name) in entries {
            data.extend(u32::try_from(offset).expect("small fixture").to_le_bytes());
            let mut padded = [0_u8; NAME_SIZE];
            padded[..name.len()].copy_from_slice(name.as_bytes());
            data.extend(padded);
        }

        data.extend(first);
        data.extend(wanted);

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time moves forward")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("viperzoo-archive-{nonce}.dat"));
        std::fs::write(&path, data).expect("fixture writes");
        let decoded = read_entry(&path, "sobj.TBL").expect("member decodes");
        std::fs::remove_file(path).expect("fixture removes");

        assert_eq!(decoded, wanted);
    }
}
