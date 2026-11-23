#![allow(clippy::useless_conversion, reason = "used in features")]

use std::collections::{HashMap, hash_map::Entry};

use std::convert::TryFrom;

use serde::ser::{self, SerializeMap as _};

use serde::Serialize;

use super::{Key, Value};

use crate::error::{Error, ErrorKind};

/// Serialize a Serde value into MINT's intermediate representation, using string field names for object keys.
pub fn to_value_named<T: ?Sized + Serialize>(value: &T) -> Result<Value, Error> { to_value(value, KeyMode::String) }

/// Serialize a Serde value into MINT's intermediate representation, using hashed field names for object keys.
pub fn to_value_hashed<T: ?Sized + Serialize>(value: &T) -> Result<Value, Error> { to_value(value, KeyMode::Id) }

fn to_value<T: ?Sized + Serialize>(value: &T, key_mode: KeyMode) -> Result<Value, Error>
{
	value.serialize(Serializer::new(key_mode))
}

impl Serialize for Key
{
	fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error>
	{
		match self
		{
			Self::String(value) => serializer.serialize_str(value),
			Self::Hashed(value) => serializer.serialize_u64(*value),
		}
	}
}

impl Serialize for Value
{
	fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error>
	{
		match self
		{
			Self::Null => serializer.serialize_unit(),
			Self::Bool(value) => serializer.serialize_bool(*value),
			Self::Uint(value) => serializer.serialize_u64(*value),
			Self::Int(value) => serializer.serialize_i64(*value),
			Self::Float32(value) => serializer.serialize_f32(*value),
			Self::Float64(value) => serializer.serialize_f64(*value),
			Self::String(value) => serializer.serialize_str(value),
			Self::Bytes(value) => serializer.serialize_bytes(value),
			Self::List(value) => value.serialize(serializer),
			Self::Object(value) =>
			{
				let mut map = serializer.serialize_map(Some(value.len()))?;

				for (key, value) in value
				{
					map.serialize_entry(key, value)?;
				}

				map.end()
			}
		}
	}
}

/// Object keys are limited to strings and nonnegative integers.
fn to_key<T: ?Sized + Serialize>(key: &T, key_mode: KeyMode) -> Result<Key, Error>
{
	match to_value(key, key_mode)?
	{
		Value::String(key) => Ok(Key::String(key)),
		Value::Uint(key) => Ok(Key::Hashed(key)),
		Value::Int(key) if key >= 0 => Ok(Key::Hashed(key as u64)),
		_ => Err(Error::new(ErrorKind::InvalidMapKey)),
	}
}

fn at_key(error: Error, key: &Key) -> Error
{
	match key
	{
		Key::String(name) => error.at_key(name.clone()),
		Key::Hashed(id) => error.at_id(*id),
	}
}

fn tagged(variant: &str, contents: Value) -> Value
{
	Value::Object(HashMap::from([(Key::String(variant.to_owned()), contents)]).into())
}

/// Serde serializer whose result is a `Value`.
pub struct Serializer
{
	key_mode: KeyMode,
}

#[derive(Clone, Copy, Debug, Default)]

pub enum KeyMode
{
	#[default]
	String,
	Id,
}

impl Serializer
{
	pub const fn new(key_mode: KeyMode) -> Self { Self { key_mode } }
	fn field_key(&self, name: &str) -> Key
	{
		match self.key_mode
		{
			KeyMode::String => Key::String(name.to_owned()),

			KeyMode::Id => Key::Hashed(crate::utils::hash_field(name)),
		}
	}
}

impl ser::Serializer for Serializer
{
	type Ok = Value;
	type Error = Error;
	type SerializeSeq = VecSerializer;
	type SerializeTuple = VecSerializer;
	type SerializeTupleStruct = VecSerializer;
	type SerializeTupleVariant = TupleVariantSerializer;
	type SerializeMap = MapSerializer;
	type SerializeStruct = MapSerializer;
	type SerializeStructVariant = StructVariantSerializer;

	fn serialize_bool(self, value: bool) -> Result<Value, Error> { Ok(Value::Bool(value)) }

	fn serialize_i8(self, value: i8) -> Result<Value, Error> { Ok(Value::Int(value.into())) }
	fn serialize_i16(self, value: i16) -> Result<Value, Error> { Ok(Value::Int(value.into())) }
	fn serialize_i32(self, value: i32) -> Result<Value, Error> { Ok(Value::Int(value.into())) }
	fn serialize_i64(self, value: i64) -> Result<Value, Error> { Ok(Value::Int(value)) }
	fn serialize_i128(self, value: i128) -> Result<Value, Error>
	{
		i64::try_from(value)
			.map(Value::Int)
			.map_err(|_| Error::new(ErrorKind::IntegerOutOfRange { target: "i64" }))
	}

	fn serialize_u8(self, value: u8) -> Result<Value, Error> { Ok(Value::Uint(value.into())) }
	fn serialize_u16(self, value: u16) -> Result<Value, Error> { Ok(Value::Uint(value.into())) }
	fn serialize_u32(self, value: u32) -> Result<Value, Error> { Ok(Value::Uint(value.into())) }
	fn serialize_u64(self, value: u64) -> Result<Value, Error> { Ok(Value::Uint(value)) }
	fn serialize_u128(self, value: u128) -> Result<Value, Error>
	{
		u64::try_from(value)
			.map(Value::Uint)
			.map_err(|_| Error::new(ErrorKind::IntegerOutOfRange { target: "u64" }))
	}

	fn serialize_f32(self, value: f32) -> Result<Value, Error> { Ok(Value::Float32(value)) }
	fn serialize_f64(self, value: f64) -> Result<Value, Error> { Ok(Value::Float64(value)) }

	fn serialize_char(self, value: char) -> Result<Value, Error> { Ok(Value::String(value.to_string())) }
	fn serialize_str(self, value: &str) -> Result<Value, Error> { Ok(Value::String(value.to_owned())) }

	fn serialize_bytes(self, value: &[u8]) -> Result<Value, Error> { Ok(Value::Bytes(value.to_vec())) }

	fn serialize_none(self) -> Result<Value, Error> { Ok(Value::Null) }
	fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Value, Error> { to_value(value, self.key_mode) }

	fn serialize_unit(self) -> Result<Value, Error> { Ok(Value::Null) }
	fn serialize_unit_struct(self, _name: &'static str) -> Result<Value, Error> { Ok(Value::Null) }
	fn serialize_unit_variant(
		self,

		_name: &'static str,

		_variant_index: u32,

		variant: &'static str,
	) -> Result<Value, Error>
	{
		Ok(Value::String(variant.to_owned()))
	}

	fn serialize_newtype_struct<T: ?Sized + Serialize>(self, _name: &'static str, value: &T) -> Result<Value, Error>
	{
		to_value(value, self.key_mode)
	}
	fn serialize_newtype_variant<T: ?Sized + Serialize>(
		self,

		_name: &'static str,

		_variant_index: u32,

		variant: &'static str,

		value: &T,
	) -> Result<Value, Error>
	{
		Ok(tagged(
			variant,
			to_value(value, self.key_mode).map_err(|e| e.at_variant(variant))?,
		))
	}

	fn serialize_seq(self, len: Option<usize>) -> Result<VecSerializer, Error>
	{
		Ok(VecSerializer {
			items: Vec::with_capacity(len.unwrap_or(0)),

			key_mode: self.key_mode,
		})
	}
	fn serialize_tuple(self, len: usize) -> Result<VecSerializer, Error>
	{
		Ok(VecSerializer {
			items: Vec::with_capacity(len),

			key_mode: self.key_mode,
		})
	}
	fn serialize_tuple_struct(self, _name: &'static str, len: usize) -> Result<VecSerializer, Error>
	{
		Ok(VecSerializer {
			items: Vec::with_capacity(len),

			key_mode: self.key_mode,
		})
	}
	fn serialize_tuple_variant(
		self,

		_name: &'static str,

		_variant_index: u32,

		variant: &'static str,

		len: usize,
	) -> Result<TupleVariantSerializer, Error>
	{
		Ok(TupleVariantSerializer {
			variant,

			items: Vec::with_capacity(len),

			key_mode: self.key_mode,
		})
	}

	fn serialize_map(self, len: Option<usize>) -> Result<MapSerializer, Error>
	{
		Ok(MapSerializer {
			entries: HashMap::with_capacity(len.unwrap_or(0)),

			pending_key: None,

			key_mode: self.key_mode,
		})
	}

	fn serialize_struct(self, _name: &'static str, len: usize) -> Result<MapSerializer, Error>
	{
		Ok(MapSerializer {
			entries: HashMap::with_capacity(len),

			pending_key: None,

			key_mode: self.key_mode,
		})
	}
	fn serialize_struct_variant(
		self,
		_name: &'static str,
		_variant_index: u32,
		variant: &'static str,
		len: usize,
	) -> Result<StructVariantSerializer, Error>
	{
		Ok(StructVariantSerializer {
			variant,

			entries: HashMap::with_capacity(len),

			key_mode: self.key_mode,
		})
	}
}

pub struct VecSerializer
{
	items:    Vec<Value>,
	key_mode: KeyMode,
}

impl ser::SerializeSeq for VecSerializer
{
	type Ok = Value;

	type Error = Error;

	fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error>
	{
		let index = self.items.len();
		self.items
			.push(to_value(value, self.key_mode).map_err(|e| e.at_index(index))?);

		Ok(())
	}

	fn end(self) -> Result<Value, Error> { Ok(Value::List(self.items)) }
}

impl ser::SerializeTuple for VecSerializer
{
	type Ok = Value;

	type Error = Error;

	fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error>
	{
		let index = self.items.len();
		self.items
			.push(to_value(value, self.key_mode).map_err(|e| e.at_index(index))?);

		Ok(())
	}

	fn end(self) -> Result<Value, Error> { Ok(Value::List(self.items)) }
}

impl ser::SerializeTupleStruct for VecSerializer
{
	type Ok = Value;

	type Error = Error;

	fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error>
	{
		let index = self.items.len();
		self.items
			.push(to_value(value, self.key_mode).map_err(|e| e.at_index(index))?);

		Ok(())
	}

	fn end(self) -> Result<Value, Error> { Ok(Value::List(self.items)) }
}

pub struct TupleVariantSerializer
{
	variant: &'static str,

	items: Vec<Value>,

	key_mode: KeyMode,
}

impl ser::SerializeTupleVariant for TupleVariantSerializer
{
	type Ok = Value;

	type Error = Error;

	fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error>
	{
		let index = self.items.len();
		self.items
			.push(to_value(value, self.key_mode).map_err(|e| e.at_index(index).at_variant(self.variant))?);

		Ok(())
	}

	fn end(self) -> Result<Value, Error> { Ok(tagged(self.variant, Value::List(self.items))) }
}

pub struct MapSerializer
{
	entries: HashMap<Key, Value>,

	pending_key: Option<Key>,

	key_mode: KeyMode,
}

impl MapSerializer
{
	fn insert(&mut self, key: Key, value: Value) -> Result<(), Error>
	{
		match self.entries.entry(key)
		{
			Entry::Vacant(slot) =>
			{
				slot.insert(value);
				Ok(())
			}
			Entry::Occupied(_) => Err(Error::new(ErrorKind::DuplicateKey)),
		}
	}
}

impl ser::SerializeMap for MapSerializer
{
	type Ok = Value;

	type Error = Error;

	fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Error>
	{
		if self.pending_key.is_some()
		{
			return Err(Error::custom("serialize_key called before serialize_value"));
		}

		self.pending_key = Some(to_key(key, self.key_mode)?);
		Ok(())
	}

	fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error>
	{
		let key = self
			.pending_key
			.take()
			.ok_or_else(|| Error::custom("serialize_value called without serialize_key"))?;

		let converted = to_value(value, self.key_mode).map_err(|e| at_key(e, &key))?;
		self.insert(key.clone(), converted).map_err(|e| at_key(e, &key))
	}

	fn serialize_entry<K: ?Sized + Serialize, V: ?Sized + Serialize>(&mut self, key: &K, value: &V)
	-> Result<(), Error>
	{
		if self.pending_key.is_some()
		{
			return Err(Error::custom("serialize_entry called with a pending key"));
		}

		let key = to_key(key, self.key_mode)?;
		let converted = to_value(value, self.key_mode).map_err(|e| at_key(e, &key))?;
		self.insert(key.clone(), converted).map_err(|e| at_key(e, &key))
	}

	fn end(self) -> Result<Value, Error>
	{
		if self.pending_key.is_some()
		{
			return Err(Error::custom("map ended with a missing value"));
		}
		Ok(Value::Object(self.entries.into()))
	}
}

impl ser::SerializeStruct for MapSerializer
{
	type Ok = Value;
	type Error = Error;

	fn serialize_field<T: ?Sized + Serialize>(&mut self, key: &'static str, value: &T) -> Result<(), Error>
	{
		let field_key = Serializer::new(self.key_mode).field_key(key);
		let converted = to_value(value, self.key_mode).map_err(|e| e.at_field(key))?;
		self.insert(field_key, converted).map_err(|e| e.at_field(key))
	}

	fn end(self) -> Result<Value, Error> { Ok(Value::Object(self.entries.into())) }
}

pub struct StructVariantSerializer
{
	variant:  &'static str,
	entries:  HashMap<Key, Value>,
	key_mode: KeyMode,
}

impl ser::SerializeStructVariant for StructVariantSerializer
{
	type Ok = Value;
	type Error = Error;

	fn serialize_field<T: ?Sized + Serialize>(&mut self, key: &'static str, value: &T) -> Result<(), Error>
	{
		let field_name = key;
		let key = match self.key_mode
		{
			KeyMode::String => Key::String(key.to_owned()),
			KeyMode::Id => Key::Hashed(crate::utils::hash_field(key)),
		};

		let value = to_value(value, self.key_mode).map_err(|e| e.at_field(field_name).at_variant(self.variant))?;

		match self.entries.entry(key)
		{
			Entry::Vacant(slot) =>
			{
				slot.insert(value);
				Ok(())
			}

			Entry::Occupied(_) => Err(Error::new(ErrorKind::DuplicateKey)
				.at_field(field_name)
				.at_variant(self.variant)),
		}
	}

	fn end(self) -> Result<Value, Error> { Ok(tagged(self.variant, Value::Object(self.entries.into()))) }
}

#[cfg(test)]
mod context_tests
{
	use super::*;

	#[derive(serde::Serialize)]
	struct Inner
	{
		count: i8,
	}

	#[derive(serde::Serialize)]
	struct Outer
	{
		rows: Vec<Inner>,
	}

	#[test]
	fn reports_nested_field_and_index()
	{
		// A negative numeric key is invalid for MINT.
		#[derive(serde::Serialize)]
		struct WithMap
		{
			rows: Vec<HashMap<i32, bool>>,
		}

		let input = WithMap {
			rows: vec![HashMap::from([(-1, true)])],
		};
		let err = to_value_hashed(&input).unwrap_err();

		// The error comes from key serialization, so the enclosing collection
		// indexes and struct name are added while unwinding.
		assert!(matches!(err.kind, ErrorKind::InvalidMapKey));

		assert_eq!(
			err.context.path,
			vec![
				crate::error::PathSegment::Field("rows".into()),
				crate::error::PathSegment::Index(0),
			]
		);
	}

	#[test]
	fn hashes_struct_field_names()
	{
		let input = Outer {
			rows: vec![Inner { count: 12 }],
		};

		let output = to_value_hashed(&input).unwrap();

		let Value::Object(root) = output
		else
		{
			panic!("expected object")
		};

		let Value::List(rows) = &root[&Key::Hashed(crate::utils::hash_field("rows"))]
		else
		{
			panic!("expected list")
		};

		let Value::Object(inner) = &rows[0]
		else
		{
			panic!("expected object")
		};

		assert_eq!(inner[&Key::Hashed(crate::utils::hash_field("count"))], Value::Int(12));
	}
}
