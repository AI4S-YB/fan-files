use fan_core::config::Config;

pub fn run(_config: &Config, path: &str, json: bool) {
    if json {
        println!("[]");
    } else {
        println!("Suggest for: {} (not yet implemented)", path);
    }
}
