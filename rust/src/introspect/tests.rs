extern crate self as minted;
use super::*;
use crate::value;
use minted_macros::Introspect;

pub fn test_object() -> Values {
    #[allow(unused)]
    let mut tags = crate::value! {
        "null" => (),
        "bool" => true,
        "int" => 42,
        "float32" => 3.146f32,
        "float64" => 2.718628f64,
        "string" => "Hello, world!",
        "bytes" => Value::Bytes(vec![1, 2, 3, 4]),
        "list" => vec!["hello", "world"],
        "object" => crate::value!{
            "nested" => "value".to_owned(),
        },
    };
    tags
}

#[test]
fn test_store_load() {
    let tags = test_object();
    assert_eq!(tags.get::<()>("null"), Some(()));
    assert_eq!(tags.get::<bool>("bool"), Some(true));
    assert_eq!(tags.get::<i64>("int"), Some(42));
    assert_eq!(tags.get::<f32>("float32"), Some(3.146f32));
    assert_eq!(tags.get::<f64>("float64"), Some(2.718628f64));
    assert_eq!(tags.get::<String>("string"), Some("Hello, world!".into()));
    assert_eq!(tags.get_bytes("bytes"), Some(vec![1, 2, 3, 4]));
    assert_eq!(
        tags.get_list::<String>("list"),
        Some(vec!["hello".to_string(), "world".to_string()])
    );
    assert_eq!(
        tags.get::<Values>("object"),
        Some(value! {"nested" => "value".to_owned()})
    );
}

#[test]
fn test_extend() {
    let mut tags = value! {"key1" => "value1"};
    tags.extend(value! {"key2" => "value2"});
    assert_eq!(tags.get_ref::<String>("key1"), Some(&"value1".into()));
    assert_eq!(tags.get_ref::<String>("key2"), Some(&"value2".into()));
}

#[test]
fn test_add_assign_tag() {
    let mut tags = value! {"key1" => "value1"};
    tags += ("key2", 1.0f32);
    assert_eq!(tags.get_ref::<String>("key1"), Some(&"value1".into()));
    assert_eq!(tags.get_ref::<f32>("key2"), Some(&1.0f32));
}

#[test]
fn test_add_assign_sets() {
    let mut tags1 = value! {"key1" => "value1"};
    let tags2 = value! {"key2" => false};
    tags1 += tags2;
    assert_eq!(tags1.get_ref::<String>("key1"), Some(&"value1".into()));
    assert_eq!(tags1.get_ref::<bool>("key2"), Some(&false));
}

#[test]
fn test_sub_assign() {
    let mut tags = value! {"key1" => "value1", "key2" => 42};
    tags -= "key1";
    assert_eq!(tags.get_ref::<String>("key1"), None);
    assert_eq!(tags.get_ref::<i64>("key2"), Some(&42.into()));
}

#[derive(Introspect, Clone)]
#[allow(dead_code)]
struct Character {
    health: i64,
    char_name: String,
    #[mint(default(expr = "\"Ranger\""))]
    char_class: String,
}

#[test]
fn test_tag_view() {
    let tags = value! {
        "health" => 100,
        "char_name" => "Aragorn",
    };
    let view = tags.get_view::<Character>().unwrap();
    assert_eq!(*view.health, 100);
    assert_eq!(view.char_name, "Aragorn");
    assert_eq!(view.char_class, "Ranger");
}

#[test]
fn test_tag_view_mut() {
    let mut tags = value! {
        "health" => 100,
        "char_name" => "Aragorn",
        "char_class" => "Ranger",
    };
    let view = tags.get_view_mut::<Character>().unwrap();
    assert_eq!(*view.health, 100);
    assert_eq!(view.char_name, "Aragorn");
    assert_eq!(view.char_class, "Ranger");
    *view.health += 50;
    assert_eq!(*view.health, 150);
    assert_eq!(tags.get::<i64>("health"), Some(150));
}

#[derive(Introspect, Clone)]
struct Nester {
    char_name: String,

    #[mint(key = "char")]
    char_class: Character,
}

#[test]
fn test_nested_tag_view() {
    let tags = value! {
        "char_name" => "Aragorn",
        "char" => value!{
            "health" => 100,
            "char_name" => "Aragorn",
        },
    };
    let view = tags.get_view::<Nester>().unwrap();
    assert_eq!(view.char_name, "Aragorn");
    assert_eq!(*view.char_class.health, 100);
    assert_eq!(view.char_class.char_name, "Aragorn");
    assert_eq!(view.char_class.char_class, "Ranger");
}

#[test]
fn test_nested_tag_view_mut() {
    let mut tags = value! {
        "char_name" => "Aragorn",
        "char" => value!{
            "health" => 100,
            "char_name" => "Aragorn",
        },
    };
    let view = tags.get_view_mut::<Nester>().unwrap();
    assert_eq!(view.char_name, "Aragorn");
    assert_eq!(*view.char_class.health, 100);
    assert_eq!(view.char_class.char_name, "Aragorn");
    assert_eq!(view.char_class.char_class, "Ranger");
    *view.char_class.health += 50;
    assert_eq!(*view.char_class.health, 150);
}

#[derive(Introspect, Clone)]
struct SuperNester {
    #[mint(key = "int")]
    char_name: i64,

    #[mint(key = "char")]
    char_class: Nester,

    #[mint(key = "missing")]
    missing: Option<Nester>,
}

#[test]
fn test_super_nested_tag_view() {
    let mut tags = value! {
        "int" => 42,
        "char" => value!{
            "char_name" => "Aragorn",
            "char" => value!{
                "health" => 100,
                "char_name" => "Aragorn",
            },
        },
    };
    let view = tags.get_view_mut::<SuperNester>().unwrap();
    assert_eq!(*view.char_name, 42);
    assert_eq!(view.char_class.char_name, "Aragorn");
    assert_eq!(*view.char_class.char_class.health, 100);
    assert_eq!(view.char_class.char_class.char_name, "Aragorn");
    assert_eq!(view.char_class.char_class.char_class, "Ranger");
    *view.char_class.char_class.health += 50;
    assert_eq!(*view.char_class.char_class.health, 150);
    assert!(view.missing.is_none());
    assert_eq!(
        tags.get::<Values>("char")
            .unwrap()
            .get::<Values>("char")
            .unwrap()
            .get::<i64>("health"),
        Some(150)
    );
}

#[test]
fn can_clone_out_of_view() {
    let tags = value! {
        "health" => 100,
        "char_name" => "Aragorn",
    };
    let view = tags.clone_view::<Character>().unwrap();
    assert_eq!(view.health, 100);
    assert_eq!(view.char_name, "Aragorn");
}

#[test]
fn can_use_into_tags() {
    let view = Character {
        health: 100,
        char_name: "Aragorn".to_string(),
        char_class: "Ranger".to_string(),
    };
    let tags = view.into_tags();
    assert_eq!(tags.get::<i64>("health"), Some(100));
    assert_eq!(tags.get::<String>("char_name"), Some("Aragorn".to_string()));
    assert_eq!(tags.get::<String>("char_class"), Some("Ranger".to_string()));
}

#[test]
fn can_use_into_tags_2() {
    let mut tags = value! {
        "health" => 100,
        "char_name" => "Aragorn",
    };
    let view = Character {
        health: 80,
        char_name: "Arbalest".to_string(),
        char_class: "Ranger".to_string(),
    };
    let tags2 = view.into_tags();
    tags += tags2;
    assert_eq!(tags.get::<i64>("health"), Some(80));
    assert_eq!(
        tags.get::<String>("char_name"),
        Some("Arbalest".to_string())
    );
    assert_eq!(tags.get::<String>("char_class"), Some("Ranger".to_string()));
}

#[derive(Default, Introspect, Clone)]
enum DefaultEnum {
    #[default]
    One,
    Two,
}

#[derive(Introspect, Clone)]
struct DefaultStuff {
    #[mint(default(expr = "\"teststring\""))]
    oval: String,

    #[mint(default(expr = "42"))]
    num: i64,

    #[mint(default(from = "oval"))]
    inherited: String,

    #[mint(default(expr = "DefaultEnum::Two"))]
    enum_val: DefaultEnum,
}

#[test]
fn can_inherit_tag() {
    let mut tags = Values::default();
    let view = tags.get_view::<DefaultStuff>().unwrap();
    assert_eq!(view.oval, "teststring");
    assert_eq!(view.inherited, "teststring");
    assert_eq!(*view.num, 42);
    let view_mut = tags.get_view_mut::<DefaultStuff>().unwrap();
    assert_eq!(view_mut.oval, "teststring");
    *view_mut.oval = "boo".to_string();
    assert_eq!(view_mut.oval, "boo");
    *view_mut.inherited = "anotherstring".to_string();
    assert_eq!(view_mut.inherited, "anotherstring");
}

#[derive(Introspect, Clone)]
struct CompoundTest {
    list: Vec<Character>,
}

#[test]
fn can_do_objects() {
    let tags = value! {
        "list" => vec![
            value!{
                "health" => 100,
                "char_name" => "listchara",
            },
            value!{
                "health" => 100,
                "char_name" => "somename",
            }
        ]
    };

    let test = tags.get_view::<CompoundTest>().unwrap();
    assert_eq!(test.list.first().unwrap().health, &100);
    assert_eq!(test.list.get(1).unwrap().char_name, "somename");
}

#[derive(Introspect, Clone, Default)]
enum TestUnitEnum {
    #[mint(key = "variant1")]
    #[default]
    Variant1,
    #[mint(key = "variant2")]
    Variant2,
}

#[derive(Introspect, Clone)]
struct HoldsUnitEnum {
    one: TestUnitEnum,
    two: TestUnitEnum,
}

#[test]
fn can_do_unit_enum() {
    let mut tags = value! {
        "one" => "variant1",
        "two" => "variant2",
    };
    let test = tags.get_view::<HoldsUnitEnum>().unwrap();
    match test.one {
        TestUnitEnum::Variant1 => {}
        _ => panic!("Expected Variant1"),
    }
    match test.two {
        TestUnitEnum::Variant2 => {}
        _ => panic!("Expected Variant2"),
    }
    let mut test = tags.get_view_mut::<HoldsUnitEnum>().unwrap();
    test.two.set(TestUnitEnum::Variant1);
    assert_eq!(tags.get::<String>("two"), Some("variant1".to_string()));
}

#[derive(Introspect, Clone)]
enum TestTupleEnum {
    #[mint(key = "variant1")]
    Variant1(i64),
    #[mint(key = "variant2")]
    Variant2(String),
}

#[test]
fn can_do_tuple_enum() {
    let tags1 = value! {
        "$type" => "variant1",
        "p0" => 42,
    };
    let mut tags2 = value! {
        "$type" => "variant2",
        "p0" => "hello",
    };
    let test1 = tags1.get_view::<TestTupleEnum>().unwrap();
    let test2 = tags2.get_view_mut::<TestTupleEnum>().unwrap();
    match test1 {
        TestTupleEnumView::Variant1(value) => assert_eq!(value, &42),
        _ => panic!("Expected Variant1"),
    }
    match test2 {
        TestTupleEnumViewMut::Variant2(value) => {
            assert_eq!(value, "hello");
            *value = "world".to_string();
        }
        _ => panic!("Expected Variant2"),
    }
    assert_eq!(tags2.get::<String>("p0"), Some("world".to_string()));
}

#[derive(Introspect, Clone)]
enum TestValueEnum {
    #[mint(key = "variant1")]
    Variant1 { value: i64 },

    #[mint(key = "variant2")]
    Variant2 { value: String },
}

#[test]
fn can_do_value_enum() {
    let tags1 = value! {
        "$type" => "variant1",
        "value" => 42,
    };
    let mut tags2 = value! {
        "$type" => "variant2",
        "value" => "hello",
    };
    let test1 = tags1.get_view::<TestValueEnum>().unwrap();
    let test2 = tags2.get_view_mut::<TestValueEnum>().unwrap();
    match test1 {
        TestValueEnumView::Variant1 { value } => assert_eq!(value, &42),
        _ => panic!("Expected Variant1"),
    }
    match test2 {
        TestValueEnumViewMut::Variant2 { value } => {
            assert_eq!(value, "hello");
            *value = "world".to_string();
        }
        _ => panic!("Expected Variant2"),
    }
    assert_eq!(tags2.get::<String>("value"), Some("world".to_string()));
}

#[derive(Introspect, Clone)]
struct BlankDefault {
    #[mint(default)]
    value: i64,
}

#[test]
fn can_use_blank_default() {
    let tags = Values::default();
    let view = tags.get_view::<BlankDefault>().unwrap();
    assert_eq!(*view.value, 0);
}

#[derive(Introspect, Clone)]
#[mint(default)]
struct BlankDefaultAll {
    value1: i64,
    value2: bool,
    value3: String,
    value4: Vec<i64>,
}

#[test]
fn can_use_blank_default_all() {
    let tags = Values::default();
    let view = tags.get_view::<BlankDefaultAll>().unwrap();
    assert_eq!(*view.value1, 0);
    assert_eq!(*view.value2, false);
    assert_eq!(view.value3, "");
}
