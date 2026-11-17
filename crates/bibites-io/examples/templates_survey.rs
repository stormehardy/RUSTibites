//! Surveys a template folder with the strict loader (task-44 diagnostics).
//!
//! Usage: `cargo run -p bibites-io --example templates_survey -- <dir> [--tolerant]`

use std::collections::BTreeMap;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(
        args.get(1)
            .cloned()
            .unwrap_or_else(|| "testdata/real/templates".to_string()),
    );
    let tolerant = args.iter().any(|arg| arg == "--tolerant");
    let mut total = 0;
    let mut ok = 0;
    let mut errors: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("bb8template") {
            continue;
        }
        total += 1;
        let result = if tolerant {
            bibites_io::load_template_tolerant(&path)
        } else {
            bibites_io::load_template(&path)
        };
        match result {
            Ok(_) => ok += 1,
            Err(error) => {
                let text = error.to_string();
                let key = if text.contains("must equal array position") {
                    "node index mismatch".to_string()
                } else if text.contains("duplicate") {
                    "duplicate innovation".to_string()
                } else if text.contains("invalid template brain") {
                    "invalid template brain".to_string()
                } else if text.contains("unsupported") {
                    "unsupported".to_string()
                } else {
                    text.chars().take(80).collect::<String>()
                };
                let entry = errors.entry(key).or_default();
                entry.0 += 1;
                if entry.1.len() < 3 {
                    entry
                        .1
                        .push(path.file_name().unwrap().to_string_lossy().to_string());
                }
            }
        }
    }
    println!(
        "{}: total={total} loadable={ok} rejected={}",
        dir.display(),
        total - ok
    );
    for (kind, (count, examples)) in errors {
        println!("  {count:4} x {kind}  e.g. {examples:?}");
    }
    Ok(())
}
