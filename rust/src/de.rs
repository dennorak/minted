use std::collections::HashMap;

use serde::Deserialize;

use crate::error::{Error, ErrorKind, Result};
use crate::header::Header;
use crate::{Key, SPECIALCASE_LIST_KEY, Value, from_value};

/// Deserialize a value from a byte slice.
///
/// For an existing value, use `Value::from_bytes()` instead.
pub fn from_bytes<T>(bytes: &[u8]) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let v = Value::from_bytes(bytes)?;
    from_value(v)
}

/// Deserialize a value from a reader.
///
/// For an existing value, use `Value::from_reader()` instead.
pub fn from_reader<T, R>(reader: &mut R) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
    R: std::io::Read,
{
    let v = Value::from_reader(reader)?;
    from_value(v)
}

fn read_u8<R: std::io::Read>(data: &mut R) -> Result<u8> {
    let mut buf = [0u8; 1];
    data.read_exact(&mut buf)
        .map_err(|_| Error::new(ErrorKind::UnexpectedEof))?;
    Ok(buf[0])
}

/// reads a length-prefixed byte slice
///
/// the length is encoded as an unsigned base-128 variable-length integer
fn read_len_prefixed<'a, R: std::io::Read>(data: &mut R) -> Result<Vec<u8>> {
    let len = read_varint(data)?;
    let len = usize::try_from(len).map_err(|_| Error::new(ErrorKind::LengthOverflow))?;

    let mut buf = vec![0u8; len];
    data.read_exact(&mut buf)
        .map_err(|_| Error::new(ErrorKind::UnexpectedEof))?;
    Ok(buf)
}

fn take<const N: usize, R: std::io::Read>(data: &mut R) -> Result<[u8; N]> {
    let mut result = [0u8; N];
    data.read_exact(&mut result)
        .map_err(|_| Error::new(ErrorKind::UnexpectedEof))?;
    Ok(result)
}

/// decodes a ZigZag-encoded unsigned integer
const fn zigzag_decode(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}

/// reads an unsigned base-128 variable-length integer
pub(super) fn read_varint<R: std::io::Read>(data: &mut R) -> Result<u64> {
    let mut value = 0u64;

    for shift in (0..70).step_by(7) {
        let byte = read_u8(data)?;

        // At shift 63, only the lowest bit can fit in a u64.
        if shift == 63 && (byte & 0xfe) != 0 {
            return Err(Error::new(ErrorKind::InvalidVarint));
        }

        value |= u64::from(byte & 0x7f) << shift;

        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }

    Err(Error::new(ErrorKind::InvalidVarint))
}

impl Value {
    /// Deserialize a value from a byte slice.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Self::from_reader(&mut &bytes[..])
    }

    /// Deserialize a value from a reader.
    pub fn from_reader<R: std::io::Read>(reader: &mut R) -> Result<Self> {
        let _header = Header::from_reader(reader)?;
        Self::from_bytes_inner(reader, None)
    }

    fn from_bytes_inner<R: std::io::Read>(reader: &mut R, known_id: Option<u8>) -> Result<Self> {
        let id = match known_id {
            Some(id) => id,
            None => read_u8(reader)?,
        };

        match id {
            // null
            0x00 => Ok(Self::Null),
            // bool
            0x01 => {
                let value = read_u8(reader)?;

                match value {
                    0 => Ok(Self::Bool(false)),
                    1 => Ok(Self::Bool(true)),
                    value => Err(Error::new(ErrorKind::InvalidBool(value))),
                }
            }
            // uint
            0x02 => {
                let value = read_varint(reader)?;
                Ok(Self::Uint(value))
            }
            // int
            0x03 => {
                let value = read_varint(reader)?;
                Ok(Self::Int(zigzag_decode(value)))
            }
            // f32
            0x04 => {
                let bytes = take::<4, _>(reader)?;
                Ok(Self::Float32(f32::from_le_bytes(bytes)))
            }
            // f64
            0x05 => {
                let bytes = take::<8, _>(reader)?;
                Ok(Self::Float64(f64::from_le_bytes(bytes)))
            }
            // string
            0x06 => {
                let bytes = read_len_prefixed(reader)?;
                let value = std::str::from_utf8(&bytes)
                    .map_err(|e| Error::new(ErrorKind::Utf8(e)))?
                    .to_owned();
                Ok(Self::String(value))
            }

            // bytes
            0x07 => {
                let bytes = read_len_prefixed(reader)?;

                Ok(Self::Bytes(bytes.to_vec()))
            }
            // list
            0x08 => {
                let element_id = read_u8(reader)?;
                let count = read_varint(reader)?;
                let count =
                    usize::try_from(count).map_err(|_| Error::new(ErrorKind::LengthOverflow))?;

                // Empty lists have no element type.
                if count == 0 {
                    return Ok(Self::List(Vec::new()));
                }

                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    let known_id = if element_id == SPECIALCASE_LIST_KEY {
                        None
                    } else {
                        Some(element_id)
                    };
                    let value = Self::from_bytes_inner(reader, known_id)?;

                    values.push(value);
                }

                Ok(Self::List(values))
            }
            // object
            0x09 => {
                let length = read_varint(reader)?;
                let length =
                    usize::try_from(length).map_err(|_| Error::new(ErrorKind::LengthOverflow))?;
                let mut items = HashMap::with_capacity(length);
                for _ in 0..length {
                    let key_kind = read_u8(reader)?;
                    let key = match key_kind {
                        0x00 => Key::Hashed(read_varint(reader)?),
                        0x01 => {
                            let bytes = read_len_prefixed(reader)?;
                            let key = std::str::from_utf8(&bytes)
                                .map_err(|e| Error::new(ErrorKind::Utf8(e)))?
                                .to_owned();
                            Key::String(key)
                        }
                        _ => return Err(Error::new(ErrorKind::UnknownKeyType(key_kind))),
                    };

                    let value = Self::from_bytes_inner(reader, None)?;
                    items.insert(key, value);
                }
                Ok(Self::Object(items))
            }
            id => Err(Error::new(ErrorKind::UnknownType(id))),
        }
    }
}
