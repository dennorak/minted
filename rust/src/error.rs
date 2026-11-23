//! Contextual errors for MINT conversion and binary I/O.
//!
//! `ErrorKind` describes the failure; `ErrorContext` records where it happened.
//! Serde conversion errors need not have a byte offset.

use std::{error, fmt, io, str};

use thiserror::Error;

/// A specialized `Result` type for MINT operations.
pub type Result<T> = std::result::Result<T, Error>;

/// A MINT error with context about where it occurred.
#[derive(Debug)]
pub struct Error
{
	/// The kind of error that occurred.
	pub kind:    ErrorKind,
	/// Contextual information about where the error occurred.
	pub context: ErrorContext,
}

/// A span of bytes in a MINT input, using an exclusive end.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ErrorContext
{
	/// Byte position where the operation that failed began.
	pub offset: Option<u64>,
	/// Known byte span of the value, using an exclusive end.
	pub span:   Option<ByteSpan>,
	/// Logical path from the root toward the failing value.
	pub path:   Vec<PathSegment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteSpan
{
	pub start: u64,
	pub end:   u64,
}

impl ByteSpan
{
	pub fn new(start: u64, end: u64) -> Self
	{
		assert!(start <= end, "byte span start must not exceed end");
		Self { start, end }
	}
}

impl From<std::ops::Range<u64>> for ByteSpan
{
	fn from(span: std::ops::Range<u64>) -> Self
	{
		Self {
			start: span.start,
			end:   span.end,
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment
{
	Field(String),
	/// Arbitrary object/map key, represented without losing numeric IDs.
	Key(String),
	Id(u64),
	Index(usize),
	Variant(String),
}

#[derive(Debug, Error)]
pub enum ErrorKind
{
	// Binary format / transport
	#[error("I/O error: {0}")]
	Io(io::Error),
	#[error("invalid UTF-8: {0}")]
	Utf8(str::Utf8Error),
	#[error("slice conversion error: {0}")]
	Slice(std::array::TryFromSliceError),
	#[error("unexpected end of input")]
	UnexpectedEof,
	#[error("invalid file header")]
	InvalidMagic,
	#[error("unsupported version {0}")]
	UnsupportedVersion(u32),
	#[error("unknown key type 0x{0:02X}")]
	UnknownKeyType(u8),
	#[error("unknown value type 0x{0:02X}")]
	UnknownType(u8),
	#[error("invalid boolean byte 0x{0:02X}")]
	InvalidBool(u8),
	#[error("invalid varint")]
	InvalidVarint,
	#[error("length does not fit in usize")]
	LengthOverflow,
	#[error("{kind} exceeds limit {limit}")]
	LimitExceeded
	{
		kind: &'static str, limit: usize
	},
	#[error("nesting depth exceeds limit {limit}")]
	DepthLimitExceeded
	{
		limit: usize
	},
	#[error("{remaining} trailing byte(s) after value")]
	TrailingBytes
	{
		remaining: usize
	},

	// Value / Serde conversion
	#[error("expected {expected}, found {found}")]
	TypeMismatch
	{
		expected: &'static str,
		found:    &'static str,
	},
	#[error("keys must be strings or nonnegative integers")]
	InvalidMapKey,
	#[error("duplicate object key")]
	DuplicateKey,
	#[error("field hash collision (0x{hash:016X}): {first:?} and {second:?}")]
	FieldHashCollision
	{
		hash:   u64,
		first:  &'static str,
		second: &'static str,
	},
	#[error("integer out of range for {target}")]
	IntegerOutOfRange
	{
		target: &'static str
	},
	#[error("invalid enum representation")]
	InvalidEnum,
	#[error("invalid enum representation: {reason}")]
	InvalidEnumRepresentation
	{
		reason: &'static str
	},
	#[error("missing {variant_kind} enum variant payload")]
	MissingEnumPayload
	{
		variant_kind: &'static str
	},
	#[error("expected exactly one Unicode character, found {count}")]
	InvalidCharacter
	{
		count: usize
	},
	#[error("expected tuple of length {expected}, found {actual}")]
	InvalidTupleLength
	{
		expected: usize, actual: usize
	},
	#[error("{container} has {remaining} unconsumed element(s)")]
	UnconsumedElements
	{
		container: &'static str, remaining: usize
	},
	#[error("invalid deserializer state: {operation}")]
	InvalidDeserializerState
	{
		operation: &'static str
	},
	#[error("{0}")]
	Message(String),
}

impl Error
{
	/// Create a new error with the given kind and no context.
	pub fn new(kind: ErrorKind) -> Self
	{
		Self {
			kind,
			context: ErrorContext::default(),
		}
	}

	/// Also available without importing the Serde error traits.
	pub fn custom(message: impl fmt::Display) -> Self { Self::new(ErrorKind::Message(message.to_string())) }

	/// Attach the position at which an operation started (or was detected).
	pub const fn at_offset(mut self, offset: u64) -> Self
	{
		if self.context.offset.is_none()
		{
			self.context.offset = Some(offset);
		}
		self
	}

	/// Attach an exact known span; end is exclusive.
	pub fn at_span(mut self, span: impl Into<ByteSpan>) -> Self
	{
		if self.context.span.is_none()
		{
			self.context.span = Some(span.into());
		}
		self
	}

	/// Prefix a path segment while an inner error propagates outward.
	pub fn at(mut self, segment: PathSegment) -> Self
	{
		self.context.path.insert(0, segment);
		self
	}

	/// Attach a path segment for a field in a struct or map.
	pub fn at_field(self, name: impl Into<String>) -> Self { self.at(PathSegment::Field(name.into())) }

	/// Attach a path segment for an arbitrary map key, represented as a string.
	pub fn at_key(self, key: impl Into<String>) -> Self { self.at(PathSegment::Key(key.into())) }

	/// Attach a path segment for an object ID.
	pub fn at_id(self, id: u64) -> Self { self.at(PathSegment::Id(id)) }

	/// Attach a path segment for an index in a sequence.
	pub fn at_index(self, index: usize) -> Self { self.at(PathSegment::Index(index)) }

	/// Attach a path segment for a variant in an enum.
	pub fn at_variant(self, variant: impl Into<String>) -> Self { self.at(PathSegment::Variant(variant.into())) }
}

impl fmt::Display for Error
{
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result
	{
		write!(f, "{}", self.kind)?;
		if let Some(span) = self.context.span
		{
			write!(f, " at bytes 0x{:X}..0x{:X}", span.start, span.end)?;
		}
		else if let Some(offset) = self.context.offset
		{
			write!(f, " at byte 0x{offset:X}")?;
		}
		if !self.context.path.is_empty()
		{
			write!(f, " (path: $")?;
			for segment in &self.context.path
			{
				match segment
				{
					PathSegment::Field(name) if is_identifier(name) => write!(f, ".{name}")?,
					PathSegment::Field(name) | PathSegment::Key(name) => write!(f, "[{name:?}]")?,
					PathSegment::Id(id) => write!(f, "[#{id}]")?,
					PathSegment::Index(index) => write!(f, "[{index}]")?,
					PathSegment::Variant(name) => write!(f, "::{name}")?,
				}
			}
			write!(f, ")")?;
		}
		Ok(())
	}
}

fn is_identifier(name: &str) -> bool
{
	let mut chars = name.chars();
	matches!(chars.next(), Some('_') | Some('a'..='z') | Some('A'..='Z')) &&
		chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

impl error::Error for Error
{
	fn source(&self) -> Option<&(dyn error::Error + 'static)>
	{
		match &self.kind
		{
			ErrorKind::Io(e) => Some(e),
			ErrorKind::Utf8(e) => Some(e),
			_ => None,
		}
	}
}

impl From<io::Error> for Error
{
	fn from(e: io::Error) -> Self { Self::new(ErrorKind::Io(e)) }
}

impl From<str::Utf8Error> for Error
{
	fn from(e: str::Utf8Error) -> Self { Self::new(ErrorKind::Utf8(e)) }
}

impl serde::ser::Error for Error
{
	fn custom<T: fmt::Display>(message: T) -> Self { Self::custom(message) }
}

impl serde::de::Error for Error
{
	fn custom<T: fmt::Display>(message: T) -> Self { Self::custom(message) }
}

#[cfg(test)]
mod tests
{
	use super::*;

	fn assert_serde_error<T: serde::ser::Error + serde::de::Error + std::error::Error>() {}

	#[test]
	fn implements_serde_error()
	{
		assert_serde_error::<Error>();
		assert_eq!(Error::custom("oops").to_string(), "oops");
	}

	#[test]
	fn renders_nested_path_and_offset()
	{
		let error = Error::new(ErrorKind::UnknownType(0xFE))
			.at_offset(0x1A4)
			.at_index(3)
			.at_field("inventory")
			.at_field("player");

		assert_eq!(
			error.to_string(),
			"unknown value type 0xFE at byte 0x1A4 (path: $.player.inventory[3])"
		);
	}

	#[test]
	fn preserves_inner_offset()
	{
		let error = Error::custom("bad").at_offset(42).at_offset(1);
		assert_eq!(error.context.offset, Some(42));
	}
}
