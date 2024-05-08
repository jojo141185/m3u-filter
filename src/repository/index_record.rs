use std::fs::File;
use std::io::{Error, Read, Seek, SeekFrom, Write};
use std::path::Path;
use crate::utils::file_utils;

/**
We write the structs with bincode::encode to a file.
To access each entry we need a index file where we can find the
Entries with offset and size of the encoded struct.
This is used for the index file where first entry is the index
of the encoded file, and size is the size of the encoded struct.
 */
pub(crate) struct IndexRecord {
    pub index: u32,
    pub size: u32,
}

impl IndexRecord {
    pub(crate) fn from_file(file: &mut File, offset: u32) -> Result<Self, Error> {
        file.seek(SeekFrom::Start(offset as u64))?;
        let mut index_bytes = [0u8; 4];
        let mut size_bytes = [0u8; 4];
        file.read_exact(&mut index_bytes)?;
        file.read_exact(&mut size_bytes)?;
        let index = u32::from_le_bytes(index_bytes);
        let size = u32::from_le_bytes(size_bytes);
        Ok(IndexRecord { index, size })
    }

    pub(crate) fn to_file(path: &Path, index: u32, size: u32, append: bool) -> Result<(), Error> {
        match file_utils::open_file_append(path, append) {
            Ok(mut file) => {
                let bytes = IndexRecord::to_bytes(index, size);
                file.write_all(&bytes)
            },
            Err(err) => Err(err)
        }
    }

    // pub fn from_bytes(bytes: &[u8], cursor: &mut usize) -> Result<Self, Error> {
    //     if let Ok(index_bytes) = bytes[*cursor..*cursor + 4].try_into() {
    //         *cursor += 4;
    //         if let Ok(size_bytes) = bytes[*cursor..*cursor + 2].try_into() {
    //             *cursor += 2;
    //             let index = u32::from_le_bytes(index_bytes);
    //             let size = u16::from_le_bytes(size_bytes);
    //             return Ok(IndexRecord { index, size });
    //         }
    //     }
    //     Err(Error::new(ErrorKind::Other, "Failed to read index"))
    // }

    // pub fn as_bytes(&self) -> [u8; 6] {
    //     IndexRecord::to_bytes(self.index, self.size)
    // }

    pub(crate) fn to_bytes(index: u32, size: u32) -> [u8; 8] {
        let index_bytes: [u8; 4] = index.to_le_bytes();
        let size_bytes: [u8; 4] = size.to_le_bytes();
        let mut combined_bytes: [u8; 8] = [0; 8];
        combined_bytes[..4].copy_from_slice(&index_bytes);
        combined_bytes[4..].copy_from_slice(&size_bytes);
        combined_bytes
    }

    pub(crate) fn get_record_size() -> u32 { 8 }
    pub(crate) fn get_index_offset(index: u32) -> u32 { index  * 8 }
}