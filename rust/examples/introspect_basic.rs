use minted::prelude::*;

#[derive(Introspect, Clone)]
struct PlayerProfile {
    name: String,
    level: i64,
    #[mint(default(expr = "true"))]
    online: bool,
    #[mint(default(from = "level"))]
    current_level: i64,
}

fn main() {
    let mut data = value! {
        "name" => "Ari",
        "level" => 12,
    };

    // Read values through a typed view.
    let view = data.get_view::<PlayerProfile>().unwrap();
    assert_eq!(view.name, "Ari");
    assert_eq!(*view.level, 12);
    assert!(view.online);
    assert_eq!(*view.current_level, 12);

    // Mutate values through a mutable view.
    let player = data.get_view_mut::<PlayerProfile>().unwrap();
    *player.level += 1;
    *player.name = "Arianna".to_string();

    assert_eq!(data.get::<String>("name"), Some("Arianna".to_string()));
    assert_eq!(data.get::<i64>("level"), Some(13));

    // The `tags!` macro is a fast way to build object maps.
    let config = value! {
        "name" => "Mira",
        "level" => 3,
        "online" => false,
    };

    let view = config.get_view::<PlayerProfile>().unwrap();
    assert_eq!(view.name, "Mira");
    assert_eq!(*view.current_level, 3);
    assert!(!view.online);

    println!("data = {data:#?}");
}
