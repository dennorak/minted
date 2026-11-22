use std::collections::HashMap;

pub mod de;
pub mod ser;

/// A key for a value in a map
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A string key
    String(String),
    /// A hashed key
    Hashed(u64),
}

/// A mint value
#[derive(Clone, Default, Debug, PartialEq, minted_macros::MintValue)]
pub enum Value {
    /// A blank value
    ///
    /// This is also the default value
    #[mint(id = 0x00)]
    #[default]
    Null,
    /// A boolean value
    #[mint(id = 0x01)]
    Bool(bool),
    /// An unsigned integer value
    #[mint(id = 0x02, extra_from(u8, u16, u32, usize))]
    Uint(u64),
    /// A signed integer value
    #[mint(id = 0x03, extra_from(i8, i16, i32, isize))]
    Int(i64),
    /// A single-precision floating point value
    #[mint(id = 0x04)]
    Float32(f32),
    /// A double-precision floating point value
    #[mint(id = 0x05)]
    Float64(f64),
    /// A string value
    #[mint(id = 0x06)]
    String(String),
    /// A binary blob value
    #[mint(id = 0x07)]
    Bytes(Vec<u8>),
    /// A heterogeneous list of values
    #[mint(id = 0x08)]
    List(Vec<Value>),
    /// A map of values
    #[mint(id = 0x09)]
    Object(HashMap<Key, Value>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{
        de::from_value,
        ser::{to_value_hashed, to_value_named},
    };
    use crate::tests::{TestEnum, TestStruct, test_struct};
    use serde::{Deserialize, Serialize, de::DeserializeOwned};
    use std::{collections::HashMap, fmt::Debug};

    // Helpers for building expected Value trees.

    pub fn field(name: &str, hashed: bool) -> Key {
        if hashed {
            Key::Hashed(crate::utils::hash_field(name))
        } else {
            Key::String(name.into())
        }
    }

    pub fn string_key(name: &str) -> Key {
        Key::String(name.into())
    }

    pub fn object(entries: impl IntoIterator<Item = (Key, Value)>) -> Value {
        Value::Object(entries.into_iter().collect())
    }

    pub fn tagged(variant: &str, value: Value) -> Value {
        object([(string_key(variant), value)])
    }

    pub fn expected_value(hashed: bool) -> Value {
        object([
            (field("a", hashed), Value::Uint(42)),
            (field("b", hashed), Value::String("Hello, world!".into())),
            (
                field("c", hashed),
                Value::List([1, 2, 3, 4, 5].into_iter().map(Value::Uint).collect()),
            ),
            (field("d", hashed), Value::Bool(true)),
            (
                field("e", hashed),
                Value::List(vec![
                    object([
                        (field("a", hashed), Value::Uint(1)),
                        (field("b", hashed), Value::String("Nested".into())),
                        (field("c", hashed), Value::List(vec![])),
                        (field("d", hashed), Value::Null),
                        (field("e", hashed), Value::List(vec![])),
                        (field("f", hashed), object([])),
                    ]),
                    object([
                        (field("a", hashed), Value::Uint(2)),
                        (field("b", hashed), Value::String("Another".into())),
                        (
                            field("c", hashed),
                            Value::List(vec![Value::Uint(6), Value::Uint(7), Value::Uint(8)]),
                        ),
                        (field("d", hashed), Value::Bool(false)),
                        (field("e", hashed), Value::List(vec![])),
                        (field("f", hashed), object([])),
                    ]),
                ]),
            ),
            (
                field("f", hashed),
                object([
                    (string_key("key1"), Value::String("Unit".into())),
                    (string_key("key2"), tagged("NewType", Value::Uint(123))),
                    (
                        string_key("key3"),
                        tagged(
                            "Tuple",
                            Value::List(vec![Value::Uint(456), Value::String("Tuple".into())]),
                        ),
                    ),
                    (
                        string_key("key4"),
                        tagged(
                            "Struct",
                            object([
                                (field("a", hashed), Value::Uint(789)),
                                (field("b", hashed), Value::String("Struct".into())),
                            ]),
                        ),
                    ),
                ]),
            ),
        ])
    }

    #[test]
    fn bytes_roundtrip() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct Test {
            #[serde(with = "serde_bytes")]
            bytes: Vec<u8>,
        }

        for bytes in [vec![], vec![1, 2, 3], vec![0, 255, 128, 42]] {
            let original = Test { bytes };

            let value = to_value_named(&original).unwrap();

            let Value::Object(ref fields) = value else {
                panic!("expected object");
            };

            assert_eq!(
                fields.get(&Key::String("bytes".into())),
                Some(&Value::Bytes(original.bytes.clone())),
            );

            let decoded: Test = from_value(value).unwrap();
            assert_eq!(decoded, original);
        }
    }

    fn check_serialization<T: Serialize + Debug>(original: &T, expected: &Value, hashed: bool) {
        let actual = if hashed {
            to_value_hashed(original).unwrap()
        } else {
            to_value_named(original).unwrap()
        };

        assert_eq!(
            &actual, expected,
            "serialization mismatch (hashed={hashed})"
        );
    }

    fn check_deserialization<T>(expected: Value, original: &T)
    where
        T: DeserializeOwned + PartialEq + Debug,
    {
        let actual: T = from_value(expected).unwrap();
        assert_eq!(&actual, original);
    }

    fn check_roundtrip<T>(original: &T)
    where
        T: Serialize + DeserializeOwned + PartialEq + Debug,
    {
        for hashed in [false, true] {
            let value = if hashed {
                to_value_hashed(original).unwrap()
            } else {
                to_value_named(original).unwrap()
            };

            let decoded: T = from_value(value).unwrap();

            assert_eq!(&decoded, original, "round-trip mismatch (hashed={hashed})");
        }
    }

    #[test]
    fn struct_serialization() {
        let original = test_struct();

        check_serialization(&original, &expected_value(false), false);

        check_serialization(&original, &expected_value(true), true);
    }

    #[test]
    fn struct_deserialization() {
        let original = test_struct();

        check_deserialization(expected_value(false), &original);
        check_deserialization(expected_value(true), &original);
    }

    #[test]
    fn struct_roundtrip() {
        check_roundtrip(&test_struct());
    }

    #[test]
    fn enum_variants_roundtrip() {
        let variants = [
            TestEnum::Unit,
            TestEnum::NewType(123),
            TestEnum::Tuple(456, "Tuple".into()),
            TestEnum::Struct {
                a: 789,
                b: "Struct".into(),
            },
        ];

        for variant in variants {
            check_roundtrip(&variant);
        }
    }

    #[test]
    fn primitive_values() {
        check_roundtrip(&true);
        check_roundtrip(&false);
        check_roundtrip(&0_u64);
        check_roundtrip(&u64::MAX);
        check_roundtrip(&i64::MIN);
        check_roundtrip(&i64::MAX);
        check_roundtrip(&-123_i32);
        check_roundtrip(&42_u8);
        check_roundtrip(&3.5_f32);
        check_roundtrip(&-123.456_f64);
        check_roundtrip(&'🦀');
        check_roundtrip(&"Hello, 世界!".to_string());
        check_roundtrip(&());
    }

    #[test]
    fn collections_and_options() {
        check_roundtrip(&Vec::<u32>::new());
        check_roundtrip(&vec![1_u32, 2, 3]);
        check_roundtrip(&(42_u32, true, "hello".to_string()));
        check_roundtrip(&[1_u32, 2, 3, 4]);
        check_roundtrip(&Some(42_u32));
        check_roundtrip(&None::<u32>);

        check_roundtrip(&HashMap::from([
            ("one".to_string(), 1_u64),
            ("two".to_string(), 2_u64),
        ]));

        check_roundtrip(&HashMap::from([
            (1_u64, "one".to_string()),
            (u64::MAX, "max".to_string()),
        ]));
    }

    #[test]
    fn raw_value_roundtrip() {
        let value = object([
            (string_key("null"), Value::Null),
            (string_key("bytes"), Value::Bytes(vec![0, 1, 255])),
            (Key::Hashed(123), Value::Float32(1.5)),
            (
                string_key("nested"),
                Value::List(vec![Value::Int(-123), Value::Uint(u64::MAX)]),
            ),
        ]);

        check_roundtrip(&value);
    }

    #[test]
    fn invalid_integer_conversion() {
        let error = from_value::<u8>(Value::Uint(256)).unwrap_err();

        assert!(matches!(
            error.kind,
            crate::error::ErrorKind::IntegerOutOfRange { .. }
        ));

        assert!(from_value::<u64>(Value::Int(-1)).is_err());
        assert!(to_value_named(&u128::MAX).is_err());
        assert!(to_value_named(&i128::MAX).is_err());
    }

    #[test]
    fn type_mismatch_has_context() {
        let mut value = expected_value(true);

        let Value::Object(ref mut fields) = value else {
            unreachable!()
        };

        fields.insert(field("a", true), Value::String("not an integer".into()));

        let error = from_value::<TestStruct>(value).unwrap_err();

        assert!(matches!(
            error.kind,
            crate::error::ErrorKind::TypeMismatch { .. }
        ));

        assert_eq!(
            error.context.path,
            vec![crate::error::PathSegment::Field("a".into())]
        );
    }

    #[test]
    fn invalid_map_key() {
        let map = HashMap::from([(true, 123_u32)]);

        assert!(to_value_named(&map).is_err());
        assert!(to_value_hashed(&map).is_err());
    }
}
