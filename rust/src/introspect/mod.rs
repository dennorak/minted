//! This module provides the `Introspect` and `IntrospectValue` traits, which allow for type-safe access to values stored in a [`Values`] collection or a [`Value`] value.
//! These traits are implemented for types that can be represented as a collection of key-value pairs, such as structs and enums.
use std::sync::LazyLock;
#[cfg(test)]
mod tests;
use crate::{Key, Value, Values};
pub use minted_macros::Introspect;
/// An error that can occur when constructing a [`ValueView`] from a `Values` collection
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueViewError {
    /// A required key is missing from the [`Values`] collection
    Missing {
        /// The missing key
        key: Key,
    },
    /// A key exists in the [`Values`] collection, but its value is of the wrong type
    WrongType {
        /// The key with the wrong type
        key: Key,
    },
    /// A tag value tried to convert from the wrong type of key
    InvalidCast,
}

impl std::fmt::Display for ValueViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing { key } => write!(f, "Missing required key: {}", key),
            Self::WrongType { key } => write!(f, "Wrong type for key: {}", key),
            Self::InvalidCast => write!(f, "Wrong type for tag value"),
        }
    }
}

impl std::error::Error for ValueViewError {}
/// A view into a [`Values`] collection
pub trait Introspect<'a>: Sized {
    /// The type of the immutable view
    type View;
    /// The type of the mutable view
    type ViewMut;
    /// Constructs a view from a reference to a [`Values`] collection
    fn view(tags: &'a Values) -> Result<Self::View, ValueViewError>;
    /// Constructs a mutable view from a mutable reference to a [`Values`] collection
    fn view_mut(tags: &'a mut Values) -> Result<Self::ViewMut, ValueViewError>;
    /// Returns a cloned view of the [`Values`] collection
    fn from_tags(tags: &'a Values) -> Result<Self, ValueViewError>;
    /// Converts from an owned view to a [`Values`] collection
    fn into_tags(self) -> Values;
}

/// A view into a [`Value`] value
pub trait IntrospectValue<'a>: Sized {
    /// The type of the immutable view
    type View;
    /// The type of the mutable view
    type ViewMut;
    /// Constructs a view from a reference to a [`Value`] value
    fn view(tag: &'a Value) -> Result<Self::View, ValueViewError>;
    /// Constructs a mutable view from a mutable reference to a [`Value`] value
    fn view_mut(tag: &'a mut Value) -> Result<Self::ViewMut, ValueViewError>;
    /// Returns a cloned value of the [`Value`]
    fn from_tag(tag: &'a Value) -> Result<Self, ValueViewError>;
    /// Converts from an owned value to a [`Value`]
    fn into_tag(self) -> Value;
    /// Converts from a value into a view, for defaults
    fn view_value(value: impl Into<Self::View>) -> Self::View {
        value.into()
    }
    /// Returns a default view of the type, for defaults
    fn default_view() -> Self::View;
    /// Returns a default value of the type, for defaults
    fn default_value() -> Self;
}

#[inline]
fn root_wrong_type() -> ValueViewError {
    ValueViewError::WrongType {
        key: Key::String("root".to_owned()),
    }
}

impl<'a> IntrospectValue<'a> for String {
    type View = &'a str;
    type ViewMut = &'a mut Self;
    fn view(tag: &'a Value) -> Result<Self::View, ValueViewError> {
        if let Value::String(s) = tag {
            Ok(s)
        } else {
            Err(root_wrong_type())
        }
    }
    fn view_mut(tag: &'a mut Value) -> Result<Self::ViewMut, ValueViewError> {
        if let Value::String(s) = tag {
            Ok(s)
        } else {
            Err(root_wrong_type())
        }
    }
    fn from_tag(tag: &'a Value) -> Result<Self, ValueViewError> {
        if let Value::String(s) = tag {
            Ok(s.into())
        } else {
            Err(root_wrong_type())
        }
    }
    fn into_tag(self) -> Value {
        Value::String(self)
    }
    fn default_view() -> Self::View {
        ""
    }
    fn default_value() -> Self {
        String::new()
    }
}

impl<'a, T: Introspect<'a>> IntrospectValue<'a> for T {
    type View = T::View;
    type ViewMut = T::ViewMut;
    fn view(tag: &'a Value) -> Result<Self::View, ValueViewError> {
        let Value::Object(tags) = tag else {
            return Err(root_wrong_type());
        };
        T::view(tags)
    }
    fn view_mut(tag: &'a mut Value) -> Result<Self::ViewMut, ValueViewError> {
        let Value::Object(tags) = tag else {
            return Err(root_wrong_type());
        };
        T::view_mut(tags)
    }
    fn from_tag(tag: &'a Value) -> Result<Self, ValueViewError> {
        let Value::Object(tags) = tag else {
            return Err(root_wrong_type());
        };
        T::from_tags(tags)
    }
    fn into_tag(self) -> Value {
        Value::Object(self.into_tags())
    }
    fn default_view() -> Self::View {
        unreachable!("Introspect types cannot be inherent default");
    }
    fn default_value() -> Self {
        unreachable!("Introspect types cannot be inherent default");
    }
}

impl<'a, T: IntrospectValue<'a>> IntrospectValue<'a> for Vec<T> {
    type View = Vec<T::View>;
    type ViewMut = Vec<T::ViewMut>;
    fn view(tag: &'a Value) -> Result<Self::View, ValueViewError> {
        let Value::List(list) = tag else {
            return Err(ValueViewError::WrongType {
                key: Key::String("root".to_owned()),
            });
        };
        list.iter().map(|value| T::view(value)).collect()
    }
    fn view_mut(tag: &'a mut Value) -> Result<Self::ViewMut, ValueViewError> {
        let Value::List(list) = tag else {
            return Err(ValueViewError::WrongType {
                key: Key::String("root".to_owned()),
            });
        };
        list.iter_mut().map(|value| T::view_mut(value)).collect()
    }
    fn from_tag(tag: &'a Value) -> Result<Self, ValueViewError> {
        let Value::List(list) = tag else {
            return Err(ValueViewError::WrongType {
                key: Key::String("root".to_owned()),
            });
        };
        list.iter().map(|value| T::from_tag(value)).collect()
    }
    fn into_tag(self) -> Value {
        Value::List(self.into_iter().map(|value| value.into_tag()).collect())
    }
    fn default_view() -> Self::View {
        Vec::new()
    }
    fn default_value() -> Self {
        Vec::new()
    }
}

impl<'a> IntrospectValue<'a> for Values {
    type View = &'a Self;
    type ViewMut = &'a mut Self;
    fn view(tag: &'a Value) -> Result<Self::View, ValueViewError> {
        if let Value::Object(tags) = tag {
            Ok(tags)
        } else {
            Err(root_wrong_type())
        }
    }
    fn view_mut(tag: &'a mut Value) -> Result<Self::ViewMut, ValueViewError> {
        if let Value::Object(tags) = tag {
            Ok(tags)
        } else {
            Err(root_wrong_type())
        }
    }
    fn from_tag(tag: &'a Value) -> Result<Self, ValueViewError> {
        if let Value::Object(tags) = tag {
            Ok(tags.clone())
        } else {
            Err(root_wrong_type())
        }
    }
    fn into_tag(self) -> Value {
        Value::Object(self)
    }
    fn default_view() -> Self::View {
        static DEFAULT_TAGS: LazyLock<Values> = LazyLock::new(|| Values::default());
        &DEFAULT_TAGS
    }
    fn default_value() -> Self {
        Values::default()
    }
}

impl IntoIterator for Values {
    type Item = (Key, Value);
    type IntoIter = std::collections::hash_map::IntoIter<Key, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Values {
    type Item = (&'a Key, &'a Value);
    type IntoIter = std::collections::hash_map::Iter<'a, Key, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'a> IntoIterator for &'a mut Values {
    type Item = (&'a Key, &'a mut Value);
    type IntoIter = std::collections::hash_map::IterMut<'a, Key, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}

impl<K: Into<Key>> Extend<(K, Value)> for Values {
    fn extend<T: IntoIterator<Item = (K, Value)>>(&mut self, iter: T) {
        self.0.extend(iter.into_iter().map(|(k, v)| (k.into(), v)));
    }
}

impl std::ops::AddAssign for Values {
    fn add_assign(&mut self, other: Self) {
        self.extend(other.0);
    }
}

impl<S, T> std::ops::AddAssign<(S, T)> for Values
where
    S: Into<Key>,
    T: Into<Value>,
{
    fn add_assign(&mut self, other: (S, T)) {
        self.insert(other.0.into(), other.1.into());
    }
}

impl<S> std::ops::SubAssign<S> for Values
where
    S: Into<Key>,
{
    fn sub_assign(&mut self, key: S) {
        self.remove(key.into());
    }
}

impl<K: Into<Key>> FromIterator<(K, Value)> for Values {
    fn from_iter<T: IntoIterator<Item = (K, Value)>>(iter: T) -> Self {
        let mut map = std::collections::HashMap::new();
        for (key, value) in iter {
            map.insert(key.into(), value);
        }
        Self(map)
    }
}

impl From<std::collections::HashMap<Key, Value>> for Values {
    fn from(map: std::collections::HashMap<Key, Value>) -> Self {
        Self(map)
    }
}

impl From<Values> for std::collections::HashMap<Key, Value> {
    fn from(values: Values) -> Self {
        values.0
    }
}

impl Value {
    /// Returns a view into the `Value` value
    pub fn get_view<'a, V>(&'a self) -> Result<V::View, ValueViewError>
    where
        V: IntrospectValue<'a>,
    {
        V::view(self)
    }

    /// Returns a mutable view into the `Value` value
    pub fn get_view_mut<'a, V>(&'a mut self) -> Result<V::ViewMut, ValueViewError>
    where
        V: IntrospectValue<'a>,
    {
        V::view_mut(self)
    }
}

impl PartialEq for Values {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl std::ops::Index<&Key> for Values {
    type Output = Value;
    fn index(&self, index: &Key) -> &Self::Output {
        &self.0[index]
    }
}

impl std::ops::IndexMut<&Key> for Values {
    fn index_mut(&mut self, index: &Key) -> &mut Self::Output {
        self.0.get_mut(index).expect("Key not found in Values")
    }
}

impl std::ops::Index<&str> for Values {
    type Output = Value;
    fn index(&self, index: &str) -> &Self::Output {
        &self.0[&Key::String(index.to_owned())]
    }
}

impl std::ops::IndexMut<&str> for Values {
    fn index_mut(&mut self, index: &str) -> &mut Self::Output {
        self.0
            .get_mut(&Key::String(index.to_owned()))
            .expect("Key not found in Values")
    }
}

impl std::ops::Index<String> for Values {
    type Output = Value;
    fn index(&self, index: String) -> &Self::Output {
        &self.0[&Key::String(index)]
    }
}

impl std::ops::IndexMut<String> for Values {
    fn index_mut(&mut self, index: String) -> &mut Self::Output {
        self.0
            .get_mut(&Key::String(index))
            .expect("Key not found in Values")
    }
}

impl std::ops::Index<u64> for Values {
    type Output = Value;
    fn index(&self, index: u64) -> &Self::Output {
        &self.0[&Key::Hashed(index)]
    }
}

impl std::ops::IndexMut<u64> for Values {
    fn index_mut(&mut self, index: u64) -> &mut Self::Output {
        self.0
            .get_mut(&Key::Hashed(index))
            .expect("Key not found in Values")
    }
}

impl Values {
    /// Creates a new tags object with a given capacity
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self(std::collections::HashMap::with_capacity(capacity))
    }

    /// Checks if the collection is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// Returns the number of tags in the collection
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }
    /// Checks if the collection contains a tag with the given key
    #[must_use]
    pub fn contains_key(&self, key: impl Into<Key>) -> bool {
        self.0.contains_key(&key.into())
    }
    /// Returns an iterator over the keys of the collection
    #[must_use]
    pub fn keys(&self) -> std::collections::hash_map::Keys<'_, Key, Value> {
        self.0.keys()
    }
    /// Removes the tag with the given key from the collection, returning the removed `Value` if it existed
    pub fn remove(&mut self, key: impl Into<Key>) -> Option<Value> {
        self.0.remove(&key.into())
    }
    /// Returns an iterator over the key-value pairs in the collection
    #[must_use]
    pub fn iter(&self) -> std::collections::hash_map::Iter<'_, Key, Value> {
        self.0.iter()
    }
    /// Returns a mutable iterator over the key-value pairs in the collection
    pub fn iter_mut(&mut self) -> std::collections::hash_map::IterMut<'_, Key, Value> {
        self.0.iter_mut()
    }
    /// Returns the raw `Value` value for the given key, if it exists
    #[must_use]
    pub fn get_raw(&self, key: impl Into<Key>) -> Option<&Value> {
        self.0.get(&key.into())
    }
    /// Returns the mutable raw `Value` value for the given key, if it exists
    pub fn get_raw_mut(&mut self, key: impl Into<Key>) -> Option<&mut Value> {
        self.0.get_mut(&key.into())
    }
    /// Drains the collection, returning an iterator over the key-value pairs
    pub fn drain(&mut self) -> std::collections::hash_map::Drain<'_, Key, Value> {
        self.0.drain()
    }
    /// Returns a value of type `T` for the given key by value, if it exists and can be converted to `T`
    #[must_use]
    pub fn get<T>(&self, key: impl Into<Key>) -> Option<T>
    where
        Value: TryInto<T>,
    {
        self.0
            .get(&key.into())
            .and_then(|v| v.clone().try_into().ok())
    }

    /// Returns a value of type `T` for the given key by reference, if it exists and can be converted to `T`
    #[must_use]
    pub fn get_ref<'a, T>(&'a self, key: impl Into<Key>) -> Option<&'a T>
    where
        &'a T: TryFrom<&'a Value>,
    {
        self.0.get(&key.into()).and_then(|v| v.try_into().ok())
    }

    /// Returns a mutable value of type `T` for the given key, if it exists and can be converted to `T`
    pub fn get_mut<'a, T>(&'a mut self, key: impl Into<Key>) -> Option<&'a mut T>
    where
        &'a mut T: TryFrom<&'a mut Value>,
    {
        self.0.get_mut(&key.into()).and_then(|v| v.try_into().ok())
    }

    /// Returns a list of mutable values of type `T` for the given keys, if they exist and can be converted to `T`
    pub fn get_many_mut<'a, const N: usize>(
        &mut self,
        keys: [&'a Key; N],
    ) -> [Option<&mut Value>; N] {
        self.0.get_disjoint_mut(keys)
    }

    /// Sets the value for the given key, converting it to a `Value` if necessary
    pub fn insert<T>(&mut self, key: impl Into<Key>, value: T)
    where
        Value: From<T>,
    {
        self.0.insert(key.into(), value.into());
    }

    /// Returns a list of values of type `T` for the given key, if it exists and can be converted to `Vec<T>`
    pub fn get_list<T>(&self, key: impl Into<Key>) -> Option<Vec<T>>
    where
        T: TryFrom<Value, Error = ()>,
    {
        let Value::List(list) = self.0.get(&key.into())?.clone() else {
            return None;
        };
        list.into_iter()
            .map(T::try_from)
            .collect::<Result<_, _>>()
            .ok()
    }

    /// Returns a bytes array for the given key, if it exists and can be converted to `Vec<u8>`
    pub fn get_bytes(&self, key: impl Into<Key>) -> Option<Vec<u8>> {
        let Value::Bytes(bytes) = self.0.get(&key.into())?.clone() else {
            return None;
        };
        Some(bytes)
    }

    /// Returns a mutable bytes array for the given key, if it exists and can be converted to `Vec<u8>`
    pub fn get_bytes_mut(&mut self, key: impl Into<Key>) -> Option<&mut Vec<u8>> {
        let Value::Bytes(bytes) = self.0.get_mut(&key.into())? else {
            return None;
        };
        Some(bytes)
    }

    /// Returns a list of references to values of type `T` for the given key, if it exists and can be converted to `Vec<T>`
    pub fn get_list_ref<'a, T>(&'a self, key: impl Into<Key>) -> Option<Vec<&'a T>>
    where
        &'a T: TryFrom<&'a Value, Error = ()>,
    {
        self.0
            .get(&key.into())
            .and_then(|v| Vec::<&T>::try_from(v).ok())
    }

    /// Inserts a value into a list for the given key, creating a new list if necessary
    pub fn insert_into_list<T>(
        &mut self,
        key: impl Into<Key>,
        value: T,
    ) -> Result<(), ValueViewError>
    where
        Value: From<T>,
    {
        let key = key.into();
        match self.0.get_mut(&key) {
            Some(Value::List(list)) => {
                list.push(value.into());
                Ok(())
            }
            Some(_) => Err(ValueViewError::WrongType { key }),
            None => {
                self.0.insert(key, Value::List(vec![value.into()]));
                Ok(())
            }
        }
    }

    /// Removes a value from a list for the given key, returning the removed value if it existed
    pub fn remove_from_list<T>(&mut self, key: impl Into<Key>, value: &T) -> Option<T>
    where
        for<'a> &'a Value: TryInto<&'a T, Error = ()>,
        T: PartialEq + TryFrom<Value>,
    {
        let Some(Value::List(list)) = self.0.get_mut(&key.into()) else {
            return None;
        };
        let pos = list.iter().position(|v| v.try_into().ok() == Some(value))?;
        let removed = list.remove(pos);
        removed.try_into().ok()
    }

    /// Returns a view into the `Values` collection, allowing for type-safe access to its values
    pub fn get_view<'a, V>(&'a self) -> Result<V::View, ValueViewError>
    where
        V: Introspect<'a>,
    {
        V::view(self)
    }

    /// Returns a mutable view into the `Values` collection, allowing for type-safe access to its values
    pub fn get_view_mut<'a, V>(&'a mut self) -> Result<V::ViewMut, ValueViewError>
    where
        V: Introspect<'a>,
    {
        V::view_mut(self)
    }

    /// Returns a cloned view of the `Values` collection
    pub fn clone_view<'a, V>(&'a self) -> Result<V, ValueViewError>
    where
        V: Introspect<'a>,
    {
        V::from_tags(self)
    }
}

/**********************************************************************************************
 * Unit Enum wrapper
 */

/// A wrapper for a unit enum tag
pub struct UnitEnum<'a, T>(&'a mut Value, std::marker::PhantomData<T>);

impl<'a, T> UnitEnum<'a, T>
where
    T: IntrospectValue<'a, View = T, ViewMut = UnitEnum<'a, T>>,
{
    /// Creates a new `UnitEnum` wrapper for the given tag
    pub fn new(tag: &'a mut Value) -> Self {
        Self(tag, std::marker::PhantomData)
    }

    /// Gets the value of the unit enum from the tag, returning an error if the tag is not of the correct type
    pub fn get(&'a self) -> Result<T, ValueViewError> {
        T::view(self.0)
    }

    /// Sets the value of the unit enum in the tag, converting it to a `Value` if necessary
    pub fn set(&mut self, value: T) {
        *self.0 = <T as IntrospectValue>::into_tag(value);
    }
}

impl<'a, T> IntrospectValue<'a> for UnitEnum<'a, T>
where
    T: IntrospectValue<'a, View = T, ViewMut = UnitEnum<'a, T>>,
{
    type View = T;
    type ViewMut = Self;

    fn view(tag: &'a Value) -> Result<Self::View, ValueViewError> {
        T::view(tag)
    }
    fn view_mut(tag: &'a mut Value) -> Result<Self::ViewMut, ValueViewError> {
        Ok(Self::new(tag))
    }

    fn from_tag(_: &'a Value) -> Result<Self, ValueViewError> {
        unreachable!("UnitEnum cannot be constructed from a Value directly, use view_mut instead");
    }

    fn into_tag(self) -> Value {
        self.0.clone()
    }

    fn default_view() -> Self::View {
        T::default_view()
    }
    fn default_value() -> Self {
        unreachable!("UnitEnum cannot be constructed from a Value directly, use view_mut instead");
    }
}
