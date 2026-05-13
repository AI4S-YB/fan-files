use fan_core::review::ReviewStore;

pub fn run(clear: bool) {
    let store = ReviewStore::new();

    if clear {
        match store.clear() {
            Ok(()) => println!("Pending review items cleared."),
            Err(e) => eprintln!("Error: {}", e),
        }
        return;
    }

    match store.load() {
        Ok(items) => {
            if items.is_empty() {
                println!("[]");
                return;
            }
            println!("{}", serde_json::to_string_pretty(&items).unwrap());
        }
        Err(e) => eprintln!("Error loading pending items: {}", e),
    }
}
