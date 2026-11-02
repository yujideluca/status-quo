//! Project-specific Rust boundaries: no GUI or IO in portable data/model crates.
use std::{env, fs, path::Path, process};

fn manifest_errors(text: &str, allowed: &[&str]) -> Vec<String> {
    let mut dependencies = false;
    let mut errors = Vec::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            dependencies = line == "[dependencies]";
            // Fail closed on target-specific/build dependency sections as well.
            if line.contains("dependencies")
                && line != "[dependencies]"
                && line != "[dev-dependencies]"
            {
                errors.push(format!("unreviewed dependency section {line}"));
            }
        } else if dependencies && !line.is_empty() {
            let name = line.split(['=', '.']).next().unwrap_or("").trim();
            let declaration: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            let canonical = declaration == format!("{name}.workspace=true")
                || (name == "egui" && declaration == "egui=\"0.31.1\"");
            if !allowed.contains(&name) || !canonical {
                errors.push(format!("unreviewed portable dependency {name}"));
            }
        }
    }
    errors
}
fn source_errors(text: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    for module in ["fs", "net", "process", "thread", "time", "env", "os"] {
        if compact.contains(&format!("std::{module}")) {
            errors.push(format!("portable code uses std::{module}"));
        }
    }
    // Also catch grouped imports (use std::{collections::..., fs}). This is intentionally
    // conservative; comments/strings mentioning forbidden APIs belong in architecture docs.
    for statement in compact.split(';').filter(|s| s.contains("usestd::{")) {
        let names: Vec<_> = statement
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .collect();
        for module in ["fs", "net", "process", "thread", "time", "env", "os"] {
            if names.contains(&module) {
                errors.push(format!("portable grouped import includes {module}"));
            }
        }
    }
    if compact.contains("usestd::*")
        || compact.contains("unsafe{")
        || compact.contains("extern\"C\"")
        || compact.contains("usestdas")
        || compact.contains("externcratestdas")
        || compact.contains("include!(")
        || compact.contains("include_str!(")
        || compact.contains("include_bytes!(")
    {
        errors.push("portable code contains wildcard std import, unsafe or FFI".into());
    }
    errors
}
fn walk(path: &Path, errors: &mut Vec<String>) {
    for item in fs::read_dir(path).expect("read source directory") {
        let path = item.expect("read source entry").path();
        if path.is_dir() {
            walk(&path, errors);
        } else if path.extension().is_some_and(|x| x == "rs") {
            for error in source_errors(&fs::read_to_string(&path).expect("read Rust source")) {
                errors.push(format!("{}: {error}", path.display()));
            }
        }
    }
}
fn main() {
    // REQ SQ-14, SQ-19, SQ-24
    if env::args().any(|x| x == "--self-test") {
        assert!(manifest_errors("[dependencies]\nserde.workspace = true", &["serde"]).is_empty());
        assert!(!manifest_errors("[dependencies]\negui = \"1\"", &["serde"]).is_empty());
        assert!(!manifest_errors(
            "[target.'cfg(unix)'.dependencies]\nreqwest = \"1\"",
            &["serde"]
        )
        .is_empty());
        assert!(!manifest_errors(
            "[dependencies]\nserde = { package = \"reqwest\", version = \"1\" }",
            &["serde"]
        )
        .is_empty());
        assert!(source_errors("use std::collections::BTreeMap;").is_empty());
        assert!(!source_errors("use std as hidden; hidden::fs::read(path)").is_empty());
        assert!(!source_errors("include_str!(path)").is_empty());
        assert!(!source_errors("std :: fs :: read(path)").is_empty());
        assert!(!source_errors("use std::{collections::BTreeMap, process::Command};").is_empty());
        assert!(!source_errors("unsafe { perform_io() }").is_empty());
        println!("Rust architecture checker: planted dependency, IO and FFI defects rejected");
        return;
    }
    let mut errors = Vec::new();
    for (name, allowed) in [
        ("core", &["serde", "serde_json"][..]),
        ("contract", &["sq-core", "serde", "serde_json"][..]),
        ("widgets", &["sq-core", "sq-contract", "serde_json"][..]),
        (
            "tray",
            &["sq-core", "sq-contract", "sq-widgets", "serde_json", "egui"][..],
        ),
    ] {
        let root = Path::new("crates").join(name);
        for error in manifest_errors(
            &fs::read_to_string(root.join("Cargo.toml")).expect("read manifest"),
            allowed,
        ) {
            errors.push(format!("{name}: {error}"));
        }
        walk(&root.join("src"), &mut errors);
    }
    if !errors.is_empty() {
        eprintln!("{}", errors.join("\n"));
        process::exit(1);
    }
    println!(
        "Rust architecture: core/contract/widgets are GUI-free; tray contains approved GUI dependencies without IO"
    );
}
