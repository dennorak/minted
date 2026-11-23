use std::collections::HashMap;

use minted::prelude::*;

#[derive(Introspect, Clone)]
struct CharacterSheet
{
	name:          String,
	#[mint(key = "type")]
	kind:          String,
	default_value: i64,
	#[mint(default(from = "default_value"))]
	value:         i64,
	#[mint(default(expr = true))]
	active:        bool,
}

#[derive(Introspect, Clone)]
enum VariantValue
{
	VariantA
	{
		value: i64
	},
	VariantB
	{
		value: String, enabled: bool
	},
}

#[derive(Introspect, Clone, Default)]
enum UnitVariant
{
	#[default]
	Variant1,
	Variant2,
}

#[derive(Introspect, Clone)]
struct SaveData
{
	player:      CharacterSheet,
	value_enums: HashMap<String, VariantValue>,
	unit_enums:  Vec<UnitVariant>,
}

fn main()
{
	let mut values = value! {
		"player" => value! {
			"type" => "player",
			"default_value" => 100,
			"name" => "Ari",
		},
		"value_enums" => value! {
			"foo" => value! {
				"$type" => "VariantA",
				"value" => 42,
			},
			"bar" => value! {
				"$type" => "VariantB",
				"value" => "hello",
				"enabled" => true,
			},
		},
		"unit_enums" => vec!["Variant1", "Variant2"],
	};

	let view = values.get_view::<SaveData>().unwrap();
	assert!(view.player.active);
	assert_eq!(view.player.value, view.player.default_value);

	// Update the nested data through a mutable view
	{
		let data = values.get_view_mut::<SaveData>().unwrap();
		*data.player.value = 150;
	}

	// Work with typed dictionary entries.
	{
		let data = values.get_view::<SaveData>().unwrap();
		match data.value_enums.get("foo").unwrap()
		{
			VariantValueView::VariantA { value } => assert_eq!(**value, 42),
			_ => panic!("expected VariantA"),
		}
	}

	{
		let mut data = values.get_view_mut::<SaveData>().unwrap();
		match data.value_enums.get_mut("bar").unwrap()
		{
			VariantValueViewMut::VariantB { value, enabled } =>
			{
				assert_eq!(**value, "hello");
				assert!(**enabled);
				**enabled = false;
			}
			_ => panic!("expected VariantB"),
		}
	}

	// Serialize to the binary mint format.
	let encoded = values.clone().to_bytes().unwrap();
	let decoded = Values::from_bytes(&encoded).unwrap();
	assert_eq!(values, decoded);

	println!("{values:#?}");
	println!("serialized bytes: {}", encoded.len());
}
