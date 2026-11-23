//! RPC envelope conversion and length-prefixed framing.
//!
//! Implement `RpcCodec` using Minted's existing `ser` and `de` APIs. The
//! RPC runtime deliberately does not assume Serde or a particular disk API.

use std::{collections::HashMap, io};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::{RpcCallError, RpcError, RpcMessage, RpcProtocolError};
use crate::{Key, Value, Values};

pub const PROTOCOL_VERSION: u8 = 1;
pub const DEFAULT_MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

fn object(fields: HashMap<Key, Value>) -> Value { Value::Object(Values::from(fields)) }

#[cfg(feature = "introspect")]
fn take(object: &mut Values, key: &str) -> Option<Value> { object.0.remove(&Key::from(key)) }

#[cfg(not(feature = "introspect"))]
fn take(object: &mut Values, key: &str) -> Option<Value> { object.remove(&Key::from(key)) }

/// Turn a typed RPC envelope into a Minted value, consuming the payload.
pub fn message_into_value(message: RpcMessage) -> Value
{
	let mut fields: HashMap<Key, Value> = HashMap::new();
	match message
	{
		RpcMessage::Request { id, method, params } =>
		{
			fields.insert("type".into(), "request".into());
			fields.insert("id".into(), Value::Uint(id));
			fields.insert("method".into(), Value::String(method));
			fields.insert("params".into(), params);
		}
		RpcMessage::Response { id, result } =>
		{
			fields.insert("type".into(), "response".into());
			fields.insert("id".into(), Value::Uint(id));
			match result
			{
				Ok(value) =>
				{
					fields.insert("result".into(), value);
				}
				Err(error) =>
				{
					fields.insert("error".into(), error.into_value());
				}
			}
		}
		RpcMessage::Notification { method, params } =>
		{
			fields.insert("type".into(), "notification".into());
			fields.insert("method".into(), Value::String(method));
			fields.insert("params".into(), params);
		}
	}
	object(fields)
}

fn required_id(fields: &mut Values) -> Result<u64, RpcProtocolError>
{
	match take(fields, "id")
	{
		Some(Value::Uint(id)) => Ok(id),
		_ => Err(RpcProtocolError::InvalidMessage("expected unsigned request ID")),
	}
}

fn required_method(fields: &mut Values) -> Result<String, RpcProtocolError>
{
	match take(fields, "method")
	{
		Some(Value::String(name)) if !name.is_empty() => Ok(name),
		_ => Err(RpcProtocolError::InvalidMessage("expected non-empty method name")),
	}
}

/// Decode the wire envelope. Unknown keys are ignored for forward compatibility.
pub fn message_from_value(value: Value) -> Result<RpcMessage, RpcProtocolError>
{
	let Value::Object(mut fields) = value
	else
	{
		return Err(RpcProtocolError::InvalidMessage("RPC envelope must be an object"));
	};

	match take(&mut fields, "type")
	{
		Some(Value::String(kind)) if kind == "request" => Ok(RpcMessage::Request {
			id:     required_id(&mut fields)?,
			method: required_method(&mut fields)?,
			params: take(&mut fields, "params").ok_or(RpcProtocolError::InvalidMessage("request missing params"))?,
		}),
		Some(Value::String(kind)) if kind == "notification" => Ok(RpcMessage::Notification {
			method: required_method(&mut fields)?,
			params: take(&mut fields, "params")
				.ok_or(RpcProtocolError::InvalidMessage("notification missing params"))?,
		}),
		Some(Value::String(kind)) if kind == "response" =>
		{
			let id = required_id(&mut fields)?;
			let result = match (take(&mut fields, "result"), take(&mut fields, "error"))
			{
				(Some(value), None) => Ok(value),
				(None, Some(error)) => Err(RpcError::try_from(&error)?),
				_ =>
				{
					return Err(RpcProtocolError::InvalidMessage(
						"response must have exactly one of result or error",
					));
				}
			};
			Ok(RpcMessage::Response { id, result })
		}
		_ => Err(RpcProtocolError::InvalidMessage("unknown RPC message type")),
	}
}

/// Read a frame: [u32 BE length][u8 protocol version][encoded Minted Value].
/// A clean EOF before a new frame returns `Ok(None)`; truncated frames error.
/// `max_frame_bytes` includes the version byte but not the 4-byte prefix.
pub async fn read_message<R: AsyncRead + Unpin>(
	reader: &mut R,
	max_frame_bytes: usize,
) -> Result<Option<RpcMessage>, RpcCallError>
{
	let mut length_bytes = [0u8; 4];
	match reader.read_exact(&mut length_bytes[..1]).await
	{
		Ok(_) =>
		{}
		Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
		Err(error) => return Err(error.into()),
	}
	reader.read_exact(&mut length_bytes[1..]).await?;

	let length = u32::from_be_bytes(length_bytes) as usize;
	if length > max_frame_bytes
	{
		return Err(RpcProtocolError::FrameTooLarge {
			size: length,
			max:  max_frame_bytes,
		}
		.into());
	}
	if length < 1
	{
		return Err(RpcProtocolError::InvalidMessage("empty RPC frame").into());
	}

	let mut data = vec![0u8; length];
	reader.read_exact(&mut data).await?;
	let version = data[0];
	if version != PROTOCOL_VERSION
	{
		return Err(RpcProtocolError::UnsupportedVersion {
			received: version,
			expected: PROTOCOL_VERSION,
		}
		.into());
	}

	let value = Value::from_bytes(&data[1..])
		.map_err(|e| RpcProtocolError::Decode(format!("failed to decode RPC frame: {e}")))?;
	Ok(Some(message_from_value(value)?))
}

/// Write one complete frame to a stream. The writer must be owned by one task.
pub async fn write_message<W: AsyncWrite + Unpin>(
	writer: &mut W,
	message: RpcMessage,
	max_frame_bytes: usize,
) -> Result<(), RpcCallError>
{
	let value = message_into_value(message);
	let payload = value
		.to_bytes()
		.map_err(|e| RpcProtocolError::Encode(format!("failed to encode RPC frame: {e}")))?;
	let length = payload.len().checked_add(1).ok_or(RpcProtocolError::FrameTooLarge {
		size: usize::MAX,
		max:  max_frame_bytes,
	})?;

	if length > max_frame_bytes || length > u32::MAX as usize
	{
		return Err(RpcProtocolError::FrameTooLarge {
			size: length,
			max:  max_frame_bytes.min(u32::MAX as usize),
		}
		.into());
	}

	writer.write_all(&(length as u32).to_be_bytes()).await?;
	writer.write_all(&[PROTOCOL_VERSION]).await?;
	writer.write_all(&payload).await?;
	writer.flush().await?;

	Ok(())
}

#[cfg(test)]
mod tests
{
	use super::{
		super::{RpcError, RpcErrorCode},
		*,
	};

	#[test]
	fn envelopes_round_trip()
	{
		let cases = [
			RpcMessage::Request {
				id:     7,
				method: "math.add".into(),
				params: Value::Null,
			},
			RpcMessage::Response {
				id:     7,
				result: Ok(Value::Uint(42)),
			},
			RpcMessage::Response {
				id:     7,
				result: Err(RpcError::new(RpcErrorCode::NOT_FOUND, "not found")),
			},
			RpcMessage::Notification {
				method: "ping".into(),
				params: Value::Bool(true),
			},
		];

		for message in cases
		{
			assert_eq!(
				message_from_value(message_into_value(message.clone())).unwrap(),
				message
			);
		}
	}

	#[test]
	fn rejects_ambiguous_response()
	{
		let mut fields = HashMap::new();
		fields.insert(Key::from("type"), Value::String("response".into()));
		fields.insert(Key::from("id"), Value::Uint(1));
		fields.insert(Key::from("result"), Value::Null);
		fields.insert(Key::from("error"), RpcError::internal().into_value());
		assert!(message_from_value(object(fields)).is_err());
	}
}
