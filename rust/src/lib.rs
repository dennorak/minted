#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::panic, clippy::disallowed_methods))]
#![warn(
	missing_docs,
	clippy::all,
	clippy::nursery,
	clippy::todo,
	clippy::print_literal,
	clippy::print_stdout,
	clippy::print_stderr,
	clippy::dbg_macro,
	clippy::unimplemented
)]
#![cfg_attr(not(doctest), doc = include_str!("../readme.md"))]
#![cfg_attr(not(debug_assertions), deny(warnings))]
#![cfg_attr(docsrs, feature(doc_cfg))]

/// Transport-independent, bidirectional async RPC for Minted.
pub mod prelude
{
	#[cfg(feature = "introspect")]
	pub use super::introspect::*;
	#[cfg(feature = "rpc")]
	pub use super::rpc::*;
	pub use super::*;
}

const SPECIALCASE_LIST_KEY: u8 = 0xFF;

extern crate self as minted;

mod header;
mod utils;

mod value;
pub use value::{
	Key, Value, Values,
	de::from_value,
	ser::{to_value_hashed, to_value_named},
};

mod error;
pub use error::{Error, Result};

mod de;
pub use de::{from_bytes, from_reader};

mod ser;
pub use ser::{to_bytes_hashed, to_bytes_named, to_writer_hashed, to_writer_named};

#[cfg(feature = "rpc")]
pub mod rpc;

#[cfg(feature = "introspect")]
pub mod introspect;

#[cfg(test)]
mod tests
{
	use super::*;
	use serde::{Deserialize, Serialize, de::DeserializeOwned};
	use std::{collections::HashMap, fmt::Debug};

	#[derive(Serialize, Deserialize, PartialEq, Debug)]
	pub struct TestStruct
	{
		a: u32,
		b: String,
		c: Vec<u8>,
		d: Option<bool>,
		e: Vec<Self>,
		f: HashMap<String, TestEnum>,
	}

	#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]

	pub enum TestEnum
	{
		Unit,
		NewType(u32),
		Tuple(u32, String),
		Struct
		{
			a: u32,
			b: String,
		},
	}

	pub fn test_struct() -> TestStruct
	{
		TestStruct {
			a: 42,
			b: "Hello, world!".into(),
			c: vec![1, 2, 3, 4, 5],
			d: Some(true),
			e: vec![
				TestStruct {
					a: 1,
					b: "Nested".into(),
					c: vec![],
					d: None,
					e: vec![],
					f: HashMap::new(),
				},
				TestStruct {
					a: 2,
					b: "Another".into(),
					c: vec![6, 7, 8],
					d: Some(false),
					e: vec![],
					f: HashMap::new(),
				},
			],
			f: HashMap::from([
				("key1".into(), TestEnum::Unit),
				("key2".into(), TestEnum::NewType(123)),
				("key3".into(), TestEnum::Tuple(456, "Tuple".into())),
				(
					"key4".into(),
					TestEnum::Struct {
						a: 789,
						b: "Struct".into(),
					},
				),
			]),
		}
	}

	fn check_roundtrip<T>(original: &T)
	where
		T: Serialize + DeserializeOwned + PartialEq + Debug,
	{
		for hashed in [false, true]
		{
			let value = if hashed
			{
				to_bytes_hashed(original).unwrap()
			}
			else
			{
				to_bytes_named(original).unwrap()
			};

			let decoded: T = from_bytes(&value).unwrap();

			assert_eq!(&decoded, original, "round-trip mismatch (hashed={hashed})");
		}
	}

	#[test]
	fn struct_roundtrip() { check_roundtrip(&test_struct()); }

	#[test]
	fn varint_roundtrip()
	{
		let values = [
			0u64,
			1,
			127,
			128,
			255,
			256,
			16383,
			16384,
			65535,
			65536,
			2_147_483_647,
			2_147_483_648,
			4_294_967_295,
			4_294_967_296,
			u64::MAX,
		];

		for &value in &values
		{
			let mut buf = Vec::new();
			ser::write_varint(&mut buf, value).unwrap();

			let mut reader = &buf[..];
			let decoded = de::read_varint(&mut reader).unwrap();

			assert_eq!(decoded, value, "varint round-trip mismatch for value {value}");
		}
	}
}
