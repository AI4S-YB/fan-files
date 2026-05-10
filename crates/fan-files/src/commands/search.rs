use fan_core::config::Config;

pub fn run(_config: &Config, query: &str, json: bool) {
    if json {
        println!("[]");
    } else {
        println!("Search for: {} (not yet implemented)", query);
    }
}
