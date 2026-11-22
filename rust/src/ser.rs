use crate::error::{Error, ErrorKind, Result};
use crate::header::Header;
use crate::{Key, SPECIALCASE_LIST_KEY, Value, to_value_hashed, to_value_named};
use serde::Serialize;

/// Serialize a value to a byte vector using the named format.
///
/// For an existing value, use `Value::to_bytes()` instead.
pub fn to_bytes_named<T>(value: &T) -> Result<Vec<u8>>
where
    T: Serialize,
{
    let v = to_value_named(value)?;
    v.to_bytes()
}

/// Serialize a value to a byte vector using the hashed format.
///
/// For an existing value, use `Value::to_bytes()` instead.
pub fn to_bytes_hashed<T>(value: &T) -> Result<Vec<u8>>
where
    T: Serialize,
{
    let v = to_value_hashed(value)?;
    v.to_bytes()
}

/// Serialize a value to a writer using the named format.
///
/// For an existing value, use `Value::to_writer()` instead.
pub fn to_writer_named<T, W>(value: &T, writer: &mut W) -> Result<()>
where
    T: Serialize,
    W: std::io::Write,
{
    let v = to_value_named(value)?;
    v.to_writer(writer)
}

/// Serialize a value to a writer using the hashed format.
///
/// For an existing value, use `Value::to_writer()` instead.
pub fn to_writer_hashed<T, W>(value: &T, writer: &mut W) -> Result<()>
where
    T: Serialize,
    W: std::io::Write,
{
    let v = to_value_hashed(value)?;
    v.to_writer(writer)
}

/// Encoded as:
///
/// Homogeneous:
///   element type ID: u8
///   element count:   unsigned varint
///   element payloads...
///
/// Heterogeneous:
///   0xff
///   element count:   unsigned varint
///   (element type ID: u8, element payload)...
///
/// Empty:
///   0xff
///   0
fn write_list(values: &[Value], buf: &mut Vec<u8>) -> Result<()> {
    let count = u64::try_from(values.len()).map_err(|_| Error::new(ErrorKind::LengthOverflow))?;

    let element_id = values
        .first()
        .map(Value::id)
        .unwrap_or(SPECIALCASE_LIST_KEY);

    let homogeneous = values.iter().all(|value| value.id() == element_id);

    let list_type_id = if homogeneous {
        element_id
    } else {
        SPECIALCASE_LIST_KEY
    };

    buf.push(list_type_id);
    write_varint(buf, count);

    for value in values {
        value.to_bytes_inner(buf, homogeneous)?;
    }

    Ok(())
}

/// writes an unsigned integer as a base-128 variable-length integer
pub(super) fn write_varint(buf: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        buf.push((value as u8) | 0x80);
        value >>= 7;
    }

    buf.push(value as u8);
}

/// encodes a signed integer using ZigZag encoding
const fn zigzag_encode(value: i64) -> u64 {
    ((value as u64) << 1) ^ ((value >> 63) as u64)
}

/// writes a length-prefixed byte slice
///
/// the length is encoded as an unsigned base-128 variable-length integer
fn write_len_prefixed(buf: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    let len = u64::try_from(value.len()).map_err(|_| Error::new(ErrorKind::LengthOverflow))?;

    write_varint(buf, len);
    buf.extend_from_slice(value);

    Ok(())
}

impl Value {
    /// Serialize a value to a writer.
    pub fn to_writer<W: std::io::Write>(&self, writer: &mut W) -> Result<()> {
        // get content
        let content = self.to_bytes()?;
        // write content to writer
        writer.write_all(&content)?;
        Ok(())
    }

    /// Serialize a value to a byte vector.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        // create buf, and serialize content
        let mut buf = Vec::new();
        self.to_bytes_inner(&mut buf, false)?;
        // check length and write header
        let length = u32::try_from(buf.len()).map_err(|_| Error::new(ErrorKind::LengthOverflow))?;
        let header = Header::new(length);
        header.write_buf(&mut buf);
        Ok(buf)
    }

    fn to_bytes_inner(&self, buf: &mut Vec<u8>, skip_id: bool) -> Result<()> {
        if !skip_id {
            buf.push(self.id());
        }
        match self {
            Self::Null => (),
            Self::Bool(value) => buf.push(u8::from(*value)),
            Self::Uint(value) => write_varint(buf, *value),
            Self::Int(value) => write_varint(buf, zigzag_encode(*value)),
            Self::Float32(value) => buf.extend_from_slice(&value.to_le_bytes()),
            Self::Float64(value) => buf.extend_from_slice(&value.to_le_bytes()),
            Self::String(value) => write_len_prefixed(buf, value.as_bytes())?,
            Self::Bytes(value) => write_len_prefixed(buf, value)?,
            Self::List(values) => write_list(values, buf)?,
            Self::Object(entries) => {
                let mut data = Vec::with_capacity(entries.len() * 2 + 1);
                write_varint(&mut data, entries.len() as u64);
                for (key, value) in entries {
                    match key {
                        Key::String(key_str) => {
                            data.push(0x01); // key type ID for string
                            write_len_prefixed(&mut data, key_str.as_bytes())?;
                        }
                        Key::Hashed(id) => {
                            data.push(0x00); // key type ID for hashed
                            write_varint(&mut data, *id);
                        }
                    }
                    value.to_bytes_inner(&mut data, false)?;
                }
                buf.extend_from_slice(&data);
            }
        }
        Ok(())
    }
}
