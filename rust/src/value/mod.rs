use std::collections::HashMap;

#[cfg(test)]
mod tests;

pub mod de;
pub mod ser;

#[doc(hidden)]
#[macro_export]
macro_rules! __tags_count {
    () => { 0usize };
    ($head:expr $(, $tail:expr)*) => {
        1usize + $crate::__tags_count!($($tail),*)
    };
}

/// Creates a [`Value`]
#[macro_export]
macro_rules! value {
    () => {
        $crate::Value::default()
    };

    (
        $(
            $key:literal => $value:expr
        ),* $(,)?
    ) => {{
        let mut items = std::collections::HashMap::with_capacity(
            $crate::__tags_count!($($key),*)
        );

        $(
            items.insert($key.into(), $value.into());
        )*

        Into::<Values>::into(items)
    }};

    ($($value:expr),* $(,)?) => {{
        let mut items = Vec::with_capacity(
            $crate::__tags_count!($($value),*)
        );

        $(
            items.push($value.into());
        )*

        Into::<Vec<$crate::Value>>::into(items)
    }};

    ($value:expr) => {
        $value.into()
    };
}

macro_rules! impl_from_tupl {
    (@impl $($n:literal),+) => {
        paste::paste! {
            impl<$([<T $n>]: Into<Value>),+>
                From<($([<T $n>],)+)> for Value
            {
                #[allow(non_snake_case, reason = "generated code")]
                fn from(($([<T $n>],)+): ($([<T $n>],)+)) -> Self {
                    Value::List(vec![$([<T $n>].into(),)+])
                }
            }
        }
    };

    (@expand ($($done:literal,)*) $next:literal $(, $rest:literal)*) => {
        impl_from_tupl!(@impl $($done,)* $next);
        impl_from_tupl!(@expand ($($done,)* $next,) $($rest),*);
    };

    (@expand ($($done:literal,)*)) => {};

    (16) => {
        impl_from_tupl!(@expand ()
            0, 1, 2, 3, 4, 5, 6, 7,
            8, 9, 10, 11, 12, 13, 14, 15
        );
    };
}

impl_from_tupl!(16);

/// A key for a value in a map
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Key
{
	/// A string key
	String(String),
	/// A hashed key
	Hashed(u64),
}

impl std::fmt::Display for Key
{
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
	{
		match self
		{
			Self::String(s) => write!(f, "{}", s),
			Self::Hashed(h) => write!(f, "{:#x}", h),
		}
	}
}

impl From<&str> for Key
{
	fn from(s: &str) -> Self { Self::String(s.to_string()) }
}

impl From<String> for Key
{
	fn from(s: String) -> Self { Self::String(s) }
}

impl From<u64> for Key
{
	fn from(h: u64) -> Self { Self::Hashed(h) }
}

impl TryFrom<Key> for String
{
	type Error = ();

	fn try_from(key: Key) -> Result<Self, Self::Error>
	{
		match key
		{
			Key::String(s) => Ok(s),
			Key::Hashed(_) => Err(()),
		}
	}
}

impl TryFrom<Key> for u64
{
	type Error = ();

	fn try_from(key: Key) -> Result<Self, Self::Error>
	{
		match key
		{
			Key::String(_) => Err(()),
			Key::Hashed(h) => Ok(h),
		}
	}
}

/// A collection of key-value pairs
#[cfg(not(feature = "introspect"))]
pub type Values = HashMap<Key, Value>;

/// A collection of key-value pairs
#[cfg(feature = "introspect")]
#[derive(Clone, Default)]
pub struct Values<S: std::hash::BuildHasher = std::hash::RandomState>(pub(crate) HashMap<Key, Value, S>);

/// A mint value
#[derive(Clone, Default, PartialEq, minted_macros::MintValue)]
pub enum Value
{
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
	#[cfg_attr(not(feature = "introspect"), mint(id = 0x06))]
	#[cfg_attr(feature = "introspect", mint(id = 0x06, exclude_schema))]
	String(String),
	/// A binary blob value
	#[cfg_attr(not(feature = "introspect"), mint(id = 0x07, exclude_from))]
	#[cfg_attr(feature = "introspect", mint(id = 0x07, exclude_from, exclude_schema))]
	Bytes(Vec<u8>),
	/// A heterogeneous list of values
	#[allow(clippy::use_self, reason = "can't use self in enum variant")]
	#[cfg_attr(not(feature = "introspect"), mint(id = 0x08, exclude_from))]
	#[cfg_attr(feature = "introspect", mint(id = 0x08, exclude_from, exclude_schema))]
	List(Vec<Value>),
	/// A map of values
	#[cfg_attr(not(feature = "introspect"), mint(id = 0x09))]
	#[cfg_attr(feature = "introspect", mint(id = 0x09, exclude_schema))]
	Object(Values),
}

impl std::fmt::Debug for Key
{
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
	{
		match self
		{
			Self::String(s) => write!(f, "{:?}", s),
			Self::Hashed(h) => write!(f, "{:#x}", h),
		}
	}
}

impl std::fmt::Debug for Value
{
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
	{
		match self
		{
			Self::Null => write!(f, "null"),
			Self::Bool(v) => v.fmt(f),
			Self::Uint(v) => v.fmt(f),
			Self::Int(v) => v.fmt(f),
			Self::Float32(v) => v.fmt(f),
			Self::Float64(v) => v.fmt(f),
			Self::String(v) => v.fmt(f),
			Self::Bytes(v) => v.fmt(f),
			Self::List(v) => v.fmt(f),
			Self::Object(v) => v.fmt(f),
		}
	}
}

impl From<&str> for Value
{
	fn from(value: &str) -> Self { Self::String(value.to_owned()) }
}

impl<'a> TryFrom<&'a Value> for &'a str
{
	type Error = ();

	fn try_from(value: &'a Value) -> Result<Self, Self::Error>
	{
		if let Value::String(s) = value { Ok(s) } else { Err(()) }
	}
}

impl<T: Into<Self>> From<Vec<T>> for Value
{
	fn from(value: Vec<T>) -> Self { Self::List(value.into_iter().map(|v| v.into()).collect()) }
}

impl<'a, T> TryFrom<&'a Value> for Vec<&'a T>
where
	&'a T: TryFrom<&'a Value, Error = ()>,
{
	type Error = ();

	fn try_from(value: &'a Value) -> Result<Self, Self::Error>
	{
		if let Value::List(list) = value
		{
			list.iter().map(|v| v.try_into()).collect()
		}
		else
		{
			Err(())
		}
	}
}

impl<T: Into<Self>> From<HashMap<String, T>> for Value
{
	fn from(value: HashMap<String, T>) -> Self
	{
		Self::Object(value.into_iter().map(|(k, v)| (Key::String(k), v.into())).collect())
	}
}

impl<T> From<Option<T>> for Value
where
	T: Into<Self>,
{
	fn from(opt: Option<T>) -> Self { opt.map_or(Self::Null, |value| value.into()) }
}
