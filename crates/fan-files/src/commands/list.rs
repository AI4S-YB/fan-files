use fan_core::config::Config;

pub fn run(_config: &Config, _category: Option<&str>, _tag: Option<&str>, json: bool) {
    if json {
        println!("[]");
    } else {
        println!("(not yet implemented)");
    }
}
