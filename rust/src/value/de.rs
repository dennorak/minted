use std::collections::{HashMap, hash_map};

use std::convert::TryFrom;

use std::fmt;

use serde::de::{
	self, DeserializeOwned, DeserializeSeed, EnumAccess, Error as _, MapAccess, SeqAccess, VariantAccess, Visitor,
};

use serde::{Deserialize, forward_to_deserialize_any};

use super::{Key, Value};

use crate::error::{Error, ErrorKind, PathSegment};

fn wrong_type(expected: &'static str, found: &Value) -> Error
{
	Error::new(ErrorKind::TypeMismatch {
		expected,
		found: found.kind().name(),
	})
}

/// Deserialize a Rust value from an owned MINT `Value`.
///
/// No key-mode argument is required: ordinary map keys retain their original
/// type, and known struct fields are recognized by name or by field hash.
pub fn from_value<T: DeserializeOwned>(value: Value) -> Result<T, Error> { T::deserialize(value) }

fn with_list<'de, V>(items: Vec<Value>, visitor: V) -> Result<V::Value, Error>
where
	V: Visitor<'de>,
{
	let mut access = ListAccess {
		items: items.into_iter(),
		index: 0,
	};

	let result = visitor.visit_seq(&mut access)?;

	if access.items.len() != 0
	{
		return Err(Error::new(ErrorKind::UnconsumedElements {
			container: "sequence",
			remaining: access.items.len(),
		}));
	}

	Ok(result)
}

/// Hash/name resolution applies only to structs, not ordinary maps.
///
/// `fields` is supplied by Serde's derived `Deserialize` implementation and
/// includes the identifiers that it knows how to recognize.
fn with_object<'de, V>(
	entries: HashMap<Key, Value>,
	fields: Option<&'static [&'static str]>,
	visitor: V,
) -> Result<V::Value, Error>
where
	V: Visitor<'de>,
{
	let hashed_fields = if let Some(fields) = fields
	{
		if entries.keys().any(|key| matches!(key, Key::Hashed(_)))
		{
			let mut lookup = HashMap::with_capacity(fields.len());

			for &field in fields
			{
				let hash = crate::utils::hash_field(field);

				if let Some(previous) = lookup.insert(hash, field)
				{
					if previous != field
					{
						return Err(Error::new(ErrorKind::FieldHashCollision {
							hash,
							first: previous,
							second: field,
						}));
					}
				}
			}

			Some(lookup)
		}
		else
		{
			None
		}
	}
	else
	{
		None
	};

	let mut access = ObjectAccess {
		entries: entries.into_iter(),
		pending_value: None,
		pending_path: None,
		hashed_fields,
	};

	let result = visitor.visit_map(&mut access)?;

	if access.pending_value.is_some() || access.entries.len() != 0
	{
		return Err(Error::new(ErrorKind::UnconsumedElements {
			container: "object",
			remaining: access.entries.len() + usize::from(access.pending_value.is_some()),
		}));
	}

	Ok(result)
}

macro_rules! deserialize_integers {
    ($conversion:ident; $($ty:ident),+ $(,)?) => {
        paste::paste! {
            $(
                fn [<deserialize_ $ty>]<V>(
                    self,
                    visitor: V,
                ) -> Result<V::Value, Error>
                where
                    V: Visitor<'de>,
                {
                    let value = $conversion::<$ty>(self)?;
                    visitor.[<visit_ $ty>](value)
                }
            )+
        }
    };
}

macro_rules! deserialize_value {
    (
        $(
            $method:ident =>
                $variant:ident,
                $visit:ident,
                $expected:literal
        );+ $(;)?
    ) => {
        $(
            fn $method<V>(
                self,
                visitor: V,
            ) -> Result<V::Value, Error>
            where
                V: Visitor<'de>,
            {
                match self {
                    Value::$variant(value) => visitor.$visit(value),
                    other => Err(wrong_type($expected, &other)),
                }
            }
        )+
    };
}

// The Value deserializer is owned, so strings/byte buffers can move directly
// into the visitor. Use `DeserializeOwned` in the public entry point.
impl<'de> de::Deserializer<'de> for Value
{
	type Error = Error;

	fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		match self
		{
			Value::Null => visitor.visit_unit(),
			Value::Bool(value) => visitor.visit_bool(value),
			Value::Int(value) => visitor.visit_i64(value),
			Value::Uint(value) => visitor.visit_u64(value),
			Value::Float32(value) => visitor.visit_f32(value),
			Value::Float64(value) => visitor.visit_f64(value),
			Value::String(value) => visitor.visit_string(value),
			Value::Bytes(value) => visitor.visit_byte_buf(value),
			Value::List(items) => with_list(items, visitor),
			Value::Object(entries) => with_object(entries.into(), None, visitor),
		}
	}

	deserialize_value! {
		deserialize_bool => Bool, visit_bool, "boolean";
		deserialize_str => String, visit_string, "string";
		deserialize_string => String, visit_string, "string";
		deserialize_bytes => Bytes, visit_byte_buf, "bytes";
		deserialize_byte_buf => Bytes, visit_byte_buf, "bytes";
	}

	deserialize_integers!(
		integer_as_signed;
		i8, i16, i32, i64, i128
	);

	deserialize_integers!(
		integer_as_unsigned;
		u8, u16, u32, u64, u128
	);

	fn deserialize_f32<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		match self
		{
			Value::Float32(value) => visitor.visit_f32(value),
			Value::Float64(value) => visitor.visit_f32(value as f32),
			other => Err(wrong_type("floating-point number", &other)),
		}
	}

	fn deserialize_f64<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		match self
		{
			Value::Float32(value) => visitor.visit_f64(f64::from(value)),
			Value::Float64(value) => visitor.visit_f64(value),
			other => Err(wrong_type("floating-point number", &other)),
		}
	}

	fn deserialize_char<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		match self
		{
			Value::String(value) =>
			{
				let mut chars = value.chars();

				match (chars.next(), chars.next())
				{
					(Some(value), None) => visitor.visit_char(value),

					_ => Err(Error::new(ErrorKind::InvalidCharacter {
						count: value.chars().count(),
					})),
				}
			}

			other => Err(wrong_type("character", &other)),
		}
	}

	fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		match self
		{
			Value::Null => visitor.visit_none(),
			other => visitor.visit_some(other),
		}
	}

	fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		match self
		{
			Value::Null => visitor.visit_unit(),
			other => Err(wrong_type("null/unit", &other)),
		}
	}

	fn deserialize_unit_struct<V>(self, _name: &'static str, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		self.deserialize_unit(visitor)
	}

	fn deserialize_newtype_struct<V>(self, _name: &'static str, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		visitor.visit_newtype_struct(self)
	}

	fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		match self
		{
			Value::List(items) => with_list(items, visitor),

			// This also lets an ordinary Vec<u8> read a Bytes value.
			Value::Bytes(bytes) => with_list(
				bytes.into_iter().map(|byte| Value::Uint(u64::from(byte))).collect(),
				visitor,
			),

			other => Err(wrong_type("list", &other)),
		}
	}

	fn deserialize_tuple<V>(self, len: usize, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		match &self
		{
			Value::List(items) if items.len() != len =>
			{
				return Err(Error::new(ErrorKind::InvalidTupleLength {
					expected: len,
					actual:   items.len(),
				}));
			}

			_ =>
			{}
		}

		self.deserialize_seq(visitor)
	}

	fn deserialize_tuple_struct<V>(self, _name: &'static str, len: usize, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		self.deserialize_tuple(len, visitor)
	}

	fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		match self
		{
			Value::Object(entries) => with_object(entries.into(), None, visitor),

			other => Err(wrong_type("object", &other)),
		}
	}

	fn deserialize_struct<V>(
		self,
		_name: &'static str,
		fields: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		match self
		{
			Value::Object(entries) => with_object(entries.into(), Some(fields), visitor),

			other => Err(wrong_type("object", &other)),
		}
	}

	fn deserialize_enum<V>(
		self,
		_name: &'static str,
		_variants: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		let (variant, contents) = match self
		{
			Value::String(variant) => (variant, None),

			Value::Object(mut entries) if entries.len() == 1 =>
			{
				let (key, value) = entries.drain().next().expect("length checked");

				match key
				{
					Key::String(variant) => (variant, Some(value)),

					Key::Hashed(_) =>
					{
						return Err(Error::new(ErrorKind::InvalidEnumRepresentation {
							reason: "enum variant name must be a string",
						}));
					}
				}
			}

			other => return Err(wrong_type("enum variant string or tagged object", &other)),
		};

		visitor.visit_enum(TaggedEnumAccess { variant, contents })
	}

	fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		match self
		{
			Value::String(value) => visitor.visit_string(value),

			Value::Uint(value) => visitor.visit_u64(value),

			other => Err(wrong_type("identifier", &other)),
		}
	}

	fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		visitor.visit_unit()
	}
}

// Helpers for signed/unsigned targets. Positive values are interchangeable

// between Value::Int and Value::Uint, but overflow always returns an error.

fn integer_as_signed<T>(value: Value) -> Result<T, Error>
where
	T: TryFrom<i64> + TryFrom<u64>,
{
	match value
	{
		Value::Int(number) => T::try_from(number).map_err(|_| {
			Error::new(ErrorKind::IntegerOutOfRange {
				target: std::any::type_name::<T>(),
			})
		}),

		Value::Uint(number) => T::try_from(number).map_err(|_| {
			Error::new(ErrorKind::IntegerOutOfRange {
				target: std::any::type_name::<T>(),
			})
		}),

		other => Err(wrong_type("integer", &other)),
	}
}

fn integer_as_unsigned<T>(value: Value) -> Result<T, Error>
where
	T: TryFrom<i64> + TryFrom<u64>,
{
	match value
	{
		Value::Int(number) => T::try_from(number).map_err(|_| {
			Error::new(ErrorKind::IntegerOutOfRange {
				target: std::any::type_name::<T>(),
			})
		}),

		Value::Uint(number) => T::try_from(number).map_err(|_| {
			Error::new(ErrorKind::IntegerOutOfRange {
				target: std::any::type_name::<T>(),
			})
		}),

		other => Err(wrong_type("integer", &other)),
	}
}

struct ListAccess
{
	items: std::vec::IntoIter<Value>,
	index: usize,
}

impl<'de> SeqAccess<'de> for &mut ListAccess
{
	type Error = Error;

	fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Error>
	where
		T: DeserializeSeed<'de>,
	{
		match self.items.next()
		{
			None => Ok(None),
			Some(item) =>
			{
				let index = self.index;
				self.index += 1;
				seed.deserialize(item).map(Some).map_err(|e| e.at_index(index))
			}
		}
	}

	fn size_hint(&self) -> Option<usize> { Some(self.items.len()) }
}

struct ObjectAccess
{
	entries: hash_map::IntoIter<Key, Value>,

	pending_value: Option<Value>,
	pending_path:  Option<PathSegment>,

	hashed_fields: Option<HashMap<u64, &'static str>>,
}

impl<'de> MapAccess<'de> for &mut ObjectAccess
{
	type Error = Error;

	fn next_key_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Error>
	where
		T: DeserializeSeed<'de>,
	{
		if self.pending_value.is_some()
		{
			return Err(Error::new(ErrorKind::InvalidDeserializerState {
				operation: "next_key_seed called before next_value_seed",
			}));
		}

		let (key, value) = match self.entries.next()
		{
			Some(entry) => entry,

			None => return Ok(None),
		};

		self.pending_value = Some(value);

		let path = match (&key, &self.hashed_fields)
		{
			(Key::Hashed(id), Some(fields)) => fields
				.get(id)
				.map(|name| PathSegment::Field((*name).to_owned()))
				.unwrap_or(PathSegment::Id(*id)),
			(Key::String(name), Some(_)) => PathSegment::Field(name.clone()),
			(Key::String(name), None) => PathSegment::Key(name.clone()),
			(Key::Hashed(id), None) => PathSegment::Id(*id),
		};
		self.pending_path = Some(path.clone());

		let key = match (key, &self.hashed_fields)
		{
			(Key::Hashed(id), Some(fields)) => match fields.get(&id)
			{
				Some(name) => Key::String((*name).to_owned()),

				// Preserve Serde's unknown-field / deny_unknown_fields behavior.

				// The marker cannot match a normal Rust field identifier.
				None => Key::String(format!("\0mint_unknown_hash_{id:016x}")),
			},

			(key, _) => key,
		};

		seed.deserialize(KeyDeserializer(key)).map(Some).map_err(|e| e.at(path))
	}

	fn next_value_seed<T>(&mut self, seed: T) -> Result<T::Value, Error>
	where
		T: DeserializeSeed<'de>,
	{
		let value = self.pending_value.take().ok_or_else(|| {
			Error::new(ErrorKind::InvalidDeserializerState {
				operation: "next_value_seed called without next_key_seed",
			})
		})?;

		let path = self.pending_path.take().expect("pending map value must have a path");
		seed.deserialize(value).map_err(|e| e.at(path))
	}

	fn size_hint(&self) -> Option<usize> { Some(self.entries.len()) }
}

/// Deserializer for map keys; IDs remain integers in ordinary maps.

struct KeyDeserializer(Key);

impl<'de> de::Deserializer<'de> for KeyDeserializer
{
	type Error = Error;

	fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		match self.0
		{
			Key::String(value) => visitor.visit_string(value),

			Key::Hashed(value) => visitor.visit_u64(value),
		}
	}

	deserialize_integers!(
		key_number;
		i8, i16, i32, i64, i128,
		u8, u16, u32, u64, u128
	);

	fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		match self.0
		{
			Key::String(value) => visitor.visit_string(value),

			Key::Hashed(_) => Err(Error::new(ErrorKind::TypeMismatch {
				expected: "string map key",
				found:    "numeric key",
			})),
		}
	}

	fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		self.deserialize_str(visitor)
	}

	fn deserialize_char<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		match self.0
		{
			Key::String(value) => de::Deserializer::deserialize_char(Value::String(value), visitor),

			Key::Hashed(_) => Err(Error::new(ErrorKind::TypeMismatch {
				expected: "character map key",
				found:    "numeric key",
			})),
		}
	}

	fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		visitor.visit_some(self)
	}

	fn deserialize_newtype_struct<V>(self, _name: &'static str, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		visitor.visit_newtype_struct(self)
	}

	fn deserialize_enum<V>(
		self,

		_name: &'static str,

		_variants: &'static [&'static str],

		visitor: V,
	) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		match self.0
		{
			Key::String(variant) => visitor.visit_enum(TaggedEnumAccess {
				variant,

				contents: None,
			}),

			Key::Hashed(_) => Err(Error::new(ErrorKind::InvalidEnumRepresentation {
				reason: "enum map key must be a string",
			})),
		}
	}

	fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		self.deserialize_any(visitor)
	}

	fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		visitor.visit_unit()
	}

	forward_to_deserialize_any! {

		bool f32 f64 bytes byte_buf unit unit_struct seq tuple tuple_struct

		map struct

	}
}

fn key_number<T>(key: KeyDeserializer) -> Result<T, Error>
where
	T: TryFrom<u64>,
{
	match key.0
	{
		Key::Hashed(value) => T::try_from(value).map_err(|_| {
			Error::new(ErrorKind::IntegerOutOfRange {
				target: std::any::type_name::<T>(),
			})
		}),
		Key::String(_) => Err(Error::new(ErrorKind::TypeMismatch {
			expected: "integer map key",
			found:    "string key",
		})),
	}
}

struct TaggedEnumAccess
{
	variant: String,

	contents: Option<Value>,
}

impl<'de> EnumAccess<'de> for TaggedEnumAccess
{
	type Error = Error;

	type Variant = TaggedVariantAccess;

	fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), Error>
	where
		V: DeserializeSeed<'de>,
	{
		let name = self.variant;
		let variant = seed
			.deserialize(de::value::StringDeserializer::<Error>::new(name.clone()))
			.map_err(|e| e.at_variant(name.clone()))?;

		Ok((
			variant,
			TaggedVariantAccess {
				variant_name: name,
				contents:     self.contents,
			},
		))
	}
}

struct TaggedVariantAccess
{
	variant_name: String,
	contents:     Option<Value>,
}

impl<'de> VariantAccess<'de> for TaggedVariantAccess
{
	type Error = Error;

	fn unit_variant(self) -> Result<(), Error>
	{
		match self.contents
		{
			None => Ok(()),

			Some(_) => Err(Error::new(ErrorKind::InvalidEnumRepresentation {
				reason: "unit enum variant must be a string",
			})),
		}
	}

	fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value, Error>
	where
		T: DeserializeSeed<'de>,
	{
		let value = self.contents.ok_or_else(|| {
			Error::new(ErrorKind::MissingEnumPayload {
				variant_kind: "newtype",
			})
		})?;

		seed.deserialize(value).map_err(|e| e.at_variant(self.variant_name))
	}

	fn tuple_variant<V>(self, len: usize, visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		let value = self
			.contents
			.ok_or_else(|| Error::new(ErrorKind::MissingEnumPayload { variant_kind: "tuple" }))?;

		de::Deserializer::deserialize_tuple(value, len, visitor).map_err(|e: Error| e.at_variant(self.variant_name))
	}

	fn struct_variant<V>(self, fields: &'static [&'static str], visitor: V) -> Result<V::Value, Error>
	where
		V: Visitor<'de>,
	{
		let value = self
			.contents
			.ok_or_else(|| Error::new(ErrorKind::MissingEnumPayload { variant_kind: "struct" }))?;

		de::Deserializer::deserialize_struct(value, "", fields, visitor)
			.map_err(|e: Error| e.at_variant(self.variant_name))
	}
}

impl<'de> Deserialize<'de> for Key
{
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: de::Deserializer<'de>,
	{
		struct KeyVisitor;

		impl<'de> Visitor<'de> for KeyVisitor
		{
			type Value = Key;

			fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result
			{
				formatter.write_str("a string or nonnegative integer map key")
			}

			fn visit_str<E: de::Error>(self, value: &str) -> Result<Key, E> { Ok(Key::String(value.to_owned())) }

			fn visit_string<E: de::Error>(self, value: String) -> Result<Key, E> { Ok(Key::String(value)) }

			fn visit_u64<E: de::Error>(self, value: u64) -> Result<Key, E> { Ok(Key::Hashed(value)) }

			fn visit_i64<E: de::Error>(self, value: i64) -> Result<Key, E>
			{
				let id = u64::try_from(value).map_err(|_| E::custom("negative map key"))?;

				Ok(Key::Hashed(id))
			}
		}

		deserializer.deserialize_any(KeyVisitor)
	}
}

macro_rules! value_visitor_methods {
    (
        $(
            $method:ident($ty:ty) => $expr:expr
        );+ $(;)?
    ) => {
        $(
            fn $method<E: de::Error>(
                self,
                value: $ty,
            ) -> Result<Value, E> {
                Ok(($expr)(value))
            }
        )+
    };
}

impl<'de> Deserialize<'de> for Value
{
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: de::Deserializer<'de>,
	{
		struct ValueVisitor;

		impl<'de> Visitor<'de> for ValueVisitor
		{
			type Value = Value;

			fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result { formatter.write_str("any MINT value") }

			value_visitor_methods! {
				visit_bool(bool) => Value::Bool;
				visit_i64(i64) => Value::Int;
				visit_u64(u64) => Value::Uint;
				visit_f32(f32) => Value::Float32;
				visit_f64(f64) => Value::Float64;
				visit_string(String) => Value::String;
				visit_byte_buf(Vec<u8>) => Value::Bytes;
				visit_char(char) => |c: char| Value::String(c.to_string());
				visit_str(&str) => |s: &str| Value::String(s.to_owned());
				visit_bytes(&[u8]) => |b: &[u8]| Value::Bytes(b.to_vec());
			}

			fn visit_unit<E: de::Error>(self) -> Result<Value, E> { Ok(Value::Null) }

			fn visit_none<E: de::Error>(self) -> Result<Value, E> { Ok(Value::Null) }

			fn visit_some<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error>
			{
				Value::deserialize(deserializer)
			}

			fn visit_i128<E: de::Error>(self, value: i128) -> Result<Value, E>
			{
				let value = i64::try_from(value).map_err(|_| E::custom("i128 out of MINT range"))?;

				Ok(Value::Int(value))
			}

			fn visit_u128<E: de::Error>(self, value: u128) -> Result<Value, E>
			{
				let value = u64::try_from(value).map_err(|_| E::custom("u128 out of MINT range"))?;

				Ok(Value::Uint(value))
			}

			fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error>
			{
				let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0));

				while let Some(value) = seq.next_element()?
				{
					items.push(value);
				}

				Ok(Value::List(items))
			}

			fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error>
			{
				let mut entries = HashMap::with_capacity(map.size_hint().unwrap_or(0));

				while let Some((key, value)) = map.next_entry::<Key, Value>()?
				{
					if entries.insert(key, value).is_some()
					{
						// A::Error is generic here; Serde's custom is the only portable constructor.
						return Err(A::Error::custom("duplicate MINT map key"));
					}
				}

				Ok(Value::Object(entries.into()))
			}
		}

		deserializer.deserialize_any(ValueVisitor)
	}
}

#[cfg(test)]

mod tests
{

	use super::*;

	use serde::{Deserialize, Serialize};

	#[derive(Debug, PartialEq, Serialize, Deserialize)]

	struct Nested
	{
		name: String,

		health: u32,
	}

	#[derive(Debug, PartialEq, Serialize, Deserialize)]

	enum Example
	{
		Empty,

		Number(u32),

		Pair(u32, bool),

		Named
		{
			x: i32,
			y: Nested,
		},
	}

	#[test]

	fn named_round_trip()
	{
		let source = Nested {
			name:   "alpha".into(),
			health: 12,
		};

		let value = super::super::ser::to_value_named(&source).unwrap();

		let decoded: Nested = from_value(value).unwrap();

		assert_eq!(decoded, source);
	}

	#[test]

	fn hashed_fields_read_by_name()
	{
		let source = Value::Object(
			HashMap::from([
				(
					Key::Hashed(crate::utils::hash_field("name")),
					Value::String("alpha".into()),
				),
				(Key::Hashed(crate::utils::hash_field("health")), Value::Uint(12)),
			])
			.into(),
		);

		let decoded: Nested = from_value(source).unwrap();

		assert_eq!(
			decoded,
			Nested {
				name:   "alpha".into(),
				health: 12,
			}
		);
	}

	#[test]

	fn numeric_map_keys_stay_numeric()
	{
		let source = HashMap::from([(123_u32, "hello".to_owned())]);

		let value = super::super::ser::to_value_named(&source).unwrap();

		let decoded: HashMap<u32, String> = from_value(value).unwrap();

		assert_eq!(decoded, source);
	}

	#[test]

	fn enum_round_trip()
	{
		let cases = [
			Example::Empty,
			Example::Number(12),
			Example::Pair(12, true),
			Example::Named {
				x: -3,
				y: Nested {
					name:   "n".into(),
					health: 5,
				},
			},
		];

		for source in cases
		{
			let value = super::super::ser::to_value_named(&source).unwrap();

			let decoded: Example = from_value(value).unwrap();

			assert_eq!(decoded, source);
		}
	}

	#[test]

	fn raw_value_round_trip()
	{
		let source = Value::Object(
			HashMap::from([
				(Key::Hashed(7), Value::Float32(1.5)),
				(Key::String("bytes".into()), Value::Bytes(vec![1, 2, 3])),
			])
			.into(),
		);

		let decoded: Value = from_value(source.clone()).unwrap();

		assert_eq!(decoded, source);
	}

	#[test]

	fn rejects_numeric_overflow()
	{
		let result: Result<i8, _> = from_value(Value::Uint(1000));

		assert!(result.is_err());
	}
}

#[cfg(test)]
mod context_tests
{
	use super::*;

	#[allow(unused)]
	#[derive(serde::Deserialize, Debug)]
	struct Root
	{
		rows: Vec<Item>,
	}
	#[allow(unused)]
	#[derive(serde::Deserialize, Debug)]
	struct Item
	{
		count: u8,
	}

	#[test]
	fn reports_nested_path()
	{
		let value = Value::Object(
			HashMap::from([(
				Key::Hashed(crate::utils::hash_field("rows")),
				Value::List(vec![Value::Object(
					HashMap::from([(
						Key::Hashed(crate::utils::hash_field("count")),
						Value::String("invalid".into()),
					)])
					.into(),
				)]),
			)])
			.into(),
		);
		let err = from_value::<Root>(value).unwrap_err();
		assert!(matches!(err.kind, ErrorKind::TypeMismatch { .. }));
		assert_eq!(
			err.context.path,
			vec![
				PathSegment::Field("rows".into()),
				PathSegment::Index(0),
				PathSegment::Field("count".into()),
			]
		);
	}
}

#[cfg(test)]
mod structured_error_tests
{
	use super::*;

	#[test]
	fn tuple_length_is_structured()
	{
		let err = from_value::<(u8, u8)>(Value::List(vec![Value::Uint(1)])).unwrap_err();
		assert!(matches!(
			err.kind,
			ErrorKind::InvalidTupleLength {
				expected: 2,
				actual:   1,
			}
		));
	}

	#[test]
	fn character_length_is_structured()
	{
		let err = from_value::<char>(Value::String("abc".into())).unwrap_err();
		assert!(matches!(err.kind, ErrorKind::InvalidCharacter { count: 3 }));
	}

	#[test]
	fn key_mismatch_is_structured()
	{
		let err = from_value::<HashMap<u32, u32>>(Value::Object(
			HashMap::from([(Key::String("bad".into()), Value::Uint(1))]).into(),
		))
		.unwrap_err();
		assert!(matches!(err.kind, ErrorKind::TypeMismatch { .. }));
		assert_eq!(err.context.path, vec![PathSegment::Key("bad".into())]);
	}

	#[test]
	fn missing_enum_payload_is_structured()
	{
		#[derive(serde::Deserialize, Debug)]
		#[allow(unused)]
		enum Example
		{
			Value(u32),
		}
		let err = from_value::<Example>(Value::String("Value".into())).unwrap_err();
		assert!(matches!(
			err.kind,
			ErrorKind::MissingEnumPayload {
				variant_kind: "newtype",
			}
		));
	}
}
