use std::io::{Read, Write};

use crate::{Error, error::ErrorKind};

/// current on-disk format version
const CURRENT_VERSION: u32 = 1;
/// magic bytes
const MAGIC: [u8; 4] = *b"PDTG";
/// size of the fixed disk header in bytes
const HEADER_SIZE: usize = 12;

/// The fixed header at the beginning of a disk-format payload.
///
/// The header is encoded as:
///
/// ```text
/// [4 bytes] magic
/// [4 bytes] version, little-endian
/// [4 bytes] payload length, little-endian
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Header {
    /// Magic bytes identifying the format
    magic: [u8; 4],

    /// Disk format version
    version: u32,

    /// Number of bytes in the payload following this header
    length: u32,
}

impl Default for Header {
    fn default() -> Self {
        Self {
            magic: MAGIC,
            version: CURRENT_VERSION,
            length: 0,
        }
    }
}

impl Header {
    pub(crate) const fn size() -> usize {
        HEADER_SIZE
    }

    pub(crate) fn new(length: u32) -> Self {
        Self {
            magic: MAGIC,
            version: CURRENT_VERSION,
            length,
        }
    }

    /// convert to the header from bytes
    pub(crate) fn from_bytes(data: &[u8]) -> Result<Self, Error> {
        if data.len() < HEADER_SIZE {
            return Err(Error::new(ErrorKind::UnexpectedEof).at_span(0..data.len() as u64));
        }

        let magic = data[0..4]
            .try_into()
            .map_err(|e| Error::new(ErrorKind::Slice(e)).at_span(0..4))?;
        let version = u32::from_le_bytes(
            data[4..8]
                .try_into()
                .map_err(|e| Error::new(ErrorKind::Slice(e)).at_span(4..8))?,
        );
        let length = u32::from_le_bytes(
            data[8..12]
                .try_into()
                .map_err(|e| Error::new(ErrorKind::Slice(e)).at_span(8..12))?,
        );

        if magic != MAGIC {
            return Err(Error::new(ErrorKind::InvalidMagic).at_span(0..4));
        }

        if version != CURRENT_VERSION {
            return Err(Error::new(ErrorKind::UnsupportedVersion(version)).at_span(4..8));
        }

        Ok(Self {
            magic,
            version,
            length,
        })
    }

    /// Writes this header to the front of a buffer,
    /// shifting the existing contents to the right.
    pub(crate) fn write_buf(&self, buf: &mut Vec<u8>) {
        let old_len = buf.len();

        buf.resize(old_len + HEADER_SIZE, 0);
        buf.copy_within(0..old_len, HEADER_SIZE);

        buf[0..4].copy_from_slice(&self.magic);
        buf[4..8].copy_from_slice(&self.version.to_le_bytes());
        buf[8..12].copy_from_slice(&self.length.to_le_bytes());
    }

    /// reads a header from the reader
    pub(crate) fn from_reader<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let mut bytes = [0u8; HEADER_SIZE];
        reader.read_exact(&mut bytes)?;
        let header = Self::from_bytes(&bytes)?;
        Ok(header)
    }

    /// writes a header to the reader
    pub(crate) fn to_writer<W: Write>(self, writer: &mut W) -> Result<(), Error> {
        let mut bytes = Vec::with_capacity(HEADER_SIZE);
        self.write_buf(&mut bytes);
        writer.write_all(&bytes)?;
        Ok(())
    }
}
