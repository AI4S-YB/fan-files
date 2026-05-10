use std::path::Path;

pub fn run(output: &Path) {
    let content = include_str!("../../../skill/fan-files.md");
    std::fs::write(output, content).expect("Failed to write skill file");
    println!("Skill written to {}", output.display());
}
