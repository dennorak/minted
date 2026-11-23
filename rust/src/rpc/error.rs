//! Error types for the Minted RPC protocol.
//!
//! Only [`RpcError`] is serialized and sent to a remote peer.
//! [`RpcProtocolError`] and [`RpcCallError`] are local errors.

use std::{collections::HashMap, io};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Key, Value, Values};

/// Stable numeric error code carried on the wire.
///
/// Unknown nonzero codes are preserved when decoding remote errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RpcErrorCode(u32);

impl RpcErrorCode
{
	// Protocol errors: 1000–1999.
	/// Unable to parse an incoming request.
	pub const PARSE_ERROR: Self = Self(1000);
	/// The RPC request envelope was invalid.
	pub const INVALID_REQUEST: Self = Self(1001);
	/// The requested RPC method does not exist.
	pub const METHOD_NOT_FOUND: Self = Self(1002);
	/// The provided method parameters were invalid.
	pub const INVALID_PARAMS: Self = Self(1003);
	/// An unexpected server-side failure occurred.
	pub const INTERNAL_ERROR: Self = Self(1004);

	// Generic application errors: 2000–2999.
	/// A requested resource does not exist.
	pub const NOT_FOUND: Self = Self(2000);
	/// A resource with the same identity already exists.
	pub const ALREADY_EXISTS: Self = Self(2001);
	/// The caller does not have permission to perform an operation.
	pub const PERMISSION_DENIED: Self = Self(2002);
	/// The caller is not authenticated.
	pub const UNAUTHENTICATED: Self = Self(2003);
	/// The operation conflicts with the current state.
	pub const CONFLICT: Self = Self(2004);
	/// A resource or quota limit was reached.
	pub const RESOURCE_EXHAUSTED: Self = Self(2005);
	/// The requested service is currently unavailable.
	pub const UNAVAILABLE: Self = Self(2006);

	/// First code available for application-defined errors.
	pub const APPLICATION_BASE: u32 = 10_000;

	/// Construct an application-specific code, if it is in the reserved range.
	pub fn application(code: u32) -> Self
	{
		assert!(
			code >= Self::APPLICATION_BASE,
			"application error code must be >= {}",
			Self::APPLICATION_BASE
		);
		Self(code)
	}

	/// Parse a wire error code, retaining unknown nonzero values.
	pub const fn from_wire(code: u32) -> Option<Self> { if code == 0 { None } else { Some(Self(code)) } }

	/// Return the raw protocol code.
	pub const fn as_u32(self) -> u32 { self.0 }
}

impl std::fmt::Display for RpcErrorCode
{
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.0) }
}

/// An RPC error returned by a remote method.
///
/// This is the only error type serialized into an RPC response. Its numeric
/// `code` is stable; `message` is human-readable and should not be parsed.
#[derive(Debug, Clone, PartialEq, Error, Serialize, Deserialize)]
#[error("RPC error {code}: {message}")]
pub struct RpcError
{
	#[serde(flatten)]
	/// Machine-readable error code.
	pub code:    RpcErrorCode,
	/// Human-readable error message.
	pub message: String,
	/// Optional structured error details.
	pub data:    Option<Value>,
}

impl RpcError
{
	/// Construct a remote error with no additional data.
	pub fn new(code: RpcErrorCode, message: impl Into<String>) -> Self
	{
		Self {
			code,
			message: message.into(),
			data: None,
		}
	}

	/// Attach structured information for the caller.
	pub fn with_data(mut self, data: impl Into<Value>) -> Self
	{
		self.data = Some(data.into());
		self
	}

	/// Build a parse error.
	pub fn parse_error(message: impl Into<String>) -> Self { Self::new(RpcErrorCode::PARSE_ERROR, message) }

	/// Build an invalid-request error.
	pub fn invalid_request(message: impl Into<String>) -> Self { Self::new(RpcErrorCode::INVALID_REQUEST, message) }

	/// Build an unknown-method error.
	pub fn method_not_found(method: &str) -> Self
	{
		Self::new(RpcErrorCode::METHOD_NOT_FOUND, format!("Method '{method}' not found"))
	}

	/// Build an invalid-parameters error.
	pub fn invalid_params(message: impl Into<String>) -> Self { Self::new(RpcErrorCode::INVALID_PARAMS, message) }

	/// Build an internal-server error without leaking implementation details.
	pub fn internal() -> Self { Self::new(RpcErrorCode::INTERNAL_ERROR, "Internal server error") }

	/// Build a resource-not-found error.
	pub fn not_found(message: impl Into<String>) -> Self { Self::new(RpcErrorCode::NOT_FOUND, message) }

	/// Build an already-exists error.
	pub fn already_exists(message: impl Into<String>) -> Self { Self::new(RpcErrorCode::ALREADY_EXISTS, message) }

	/// Build a permission-denied error.
	pub fn permission_denied() -> Self { Self::new(RpcErrorCode::PERMISSION_DENIED, "Permission denied") }

	/// Build an unauthenticated error.
	pub fn unauthenticated() -> Self { Self::new(RpcErrorCode::UNAUTHENTICATED, "Authentication required") }

	/// Build a conflict error.
	pub fn conflict(message: impl Into<String>) -> Self { Self::new(RpcErrorCode::CONFLICT, message) }

	/// Build a resource-exhausted error.
	pub fn resource_exhausted() -> Self { Self::new(RpcErrorCode::RESOURCE_EXHAUSTED, "Resource limit exceeded") }

	/// Build an unavailable-service error.
	pub fn unavailable() -> Self { Self::new(RpcErrorCode::UNAVAILABLE, "Service unavailable") }

	/// Build an application-specific error with a custom code.
	pub fn application(code: u32, message: impl Into<String>) -> Self
	{
		Self::new(RpcErrorCode::application(code), message)
	}

	/// Encode this remote error using Minted's dynamic value format.
	pub fn into_value(self) -> Value
	{
		let mut fields: HashMap<Key, Value> = HashMap::new();
		fields.insert("code".into(), Value::Uint(self.code.as_u32() as u64));
		fields.insert("message".into(), Value::String(self.message));

		if let Some(data) = self.data
		{
			fields.insert("data".into(), data);
		}

		Value::Object(Values::from(fields))
	}
}

/// Access an object field with or without the optional `introspect` feature.
#[cfg(feature = "introspect")]
fn field<'a>(object: &'a Values, name: &str) -> Option<&'a Value> { object.0.get(&Key::from(name)) }

#[cfg(not(feature = "introspect"))]
fn field<'a>(object: &'a Values, name: &str) -> Option<&'a Value> { object.get(&Key::from(name)) }

impl TryFrom<&Value> for RpcError
{
	type Error = RpcProtocolError;

	fn try_from(value: &Value) -> Result<Self, Self::Error>
	{
		let Value::Object(fields) = value
		else
		{
			return Err(RpcProtocolError::InvalidMessage("RPC error must be an object"));
		};

		let code = match field(fields, "code")
		{
			Some(Value::Uint(raw)) =>
			{
				let raw =
					u32::try_from(*raw).map_err(|_| RpcProtocolError::InvalidMessage("RPC error code out of range"))?;

				RpcErrorCode::from_wire(raw).ok_or(RpcProtocolError::InvalidMessage("RPC error code cannot be zero"))?
			}
			_ =>
			{
				return Err(RpcProtocolError::InvalidMessage("RPC error is missing a valid code"));
			}
		};

		let message = match field(fields, "message")
		{
			Some(Value::String(message)) => message.clone(),
			_ =>
			{
				return Err(RpcProtocolError::InvalidMessage("RPC error is missing a valid message"));
			}
		};

		Ok(Self {
			code,
			message,
			data: field(fields, "data").cloned(),
		})
	}
}

/// A local error caused by malformed or unexpected RPC protocol data.
///
/// This error is not directly serialized onto the wire.
#[derive(Debug, Error)]
pub enum RpcProtocolError
{
	/// An RPC message has invalid fields or structure.
	#[error("invalid RPC message: {0}")]
	InvalidMessage(&'static str),

	/// A payload could not be encoded.
	#[error("failed to encode RPC payload: {0}")]
	Encode(String),

	/// A payload could not be decoded.
	#[error("failed to decode RPC payload: {0}")]
	Decode(String),

	/// A frame uses an unsupported protocol version.
	#[error("unsupported RPC version {received}, expected {expected}")]
	UnsupportedVersion
	{
		/// Version contained in the received frame.
		received: u8,
		/// Version supported by this peer.
		expected: u8,
	},

	/// A frame exceeds the configured size limit.
	#[error("RPC frame too large: {size} bytes (maximum {max})")]
	FrameTooLarge
	{
		/// Frame size in bytes.
		size: usize,
		/// Maximum allowed size in bytes.
		max:  usize,
	},

	/// A valid but unexpected kind of RPC message was received.
	#[error("unexpected RPC message type")]
	UnexpectedMessage,

	/// A response arrived for an unknown request ID.
	#[error("unexpected response ID {0}")]
	UnexpectedResponse(u64),
}

/// A local failure to complete an RPC call.
///
/// The remote peer may have executed a request even if this reports a timeout
/// or connection failure. Only retry operations known to be idempotent.
#[derive(Debug, Error)]
pub enum RpcCallError
{
	/// The remote handler returned an application or RPC error.
	#[error(transparent)]
	Remote(#[from] RpcError),

	/// The connection reported an I/O error.
	#[error("RPC transport error: {0}")]
	Io(#[from] io::Error),

	/// The peer sent an invalid RPC message or frame.
	#[error(transparent)]
	Protocol(#[from] RpcProtocolError),

	/// The caller stopped waiting for a response.
	#[error("RPC request timed out")]
	Timeout,

	/// The caller cancelled the request.
	#[error("RPC request cancelled")]
	Cancelled,

	/// The connection closed before the call completed.
	#[error("RPC connection closed")]
	ConnectionClosed,
}

/// Result returned by a server-side RPC method.
pub type RpcResult<T = Value> = Result<T, RpcError>;

/// Result returned by a client-side RPC call.
pub type RpcCallResult<T = Value> = Result<T, RpcCallError>;

#[cfg(test)]
mod tests
{
	use super::*;

	#[test]
	fn remote_error_round_trip()
	{
		let error = RpcError::invalid_params("missing player_id").with_data(Value::String("player_id".into()));

		let encoded = error.clone().into_value();
		let decoded = RpcError::try_from(&encoded).unwrap();

		assert_eq!(decoded, error);
	}

	#[test]
	fn unknown_error_code_is_preserved()
	{
		let code = RpcErrorCode::from_wire(65_535).unwrap();
		let error = RpcError::new(code, "custom error");
		let decoded = RpcError::try_from(&error.into_value()).unwrap();

		assert_eq!(decoded.code.as_u32(), 65_535);
	}

	#[test]
	fn rejects_missing_code()
	{
		let mut fields = HashMap::<Key, Value>::new();
		fields.insert("message".into(), "error".into());
		let value = Value::Object(Values::from(fields));

		assert!(matches!(
			RpcError::try_from(&value),
			Err(RpcProtocolError::InvalidMessage(_))
		));
	}
}
