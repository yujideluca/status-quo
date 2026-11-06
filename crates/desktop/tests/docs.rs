//! The user tutorial must stay true: links resolve, widgets are all explained, the variables it
//! names exist, and its example code is the code a test compiles.
use std::{fs, path::Path};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
}
fn read(relative: &str) -> String {
    fs::read_to_string(root().join(relative)).unwrap_or_else(|e| panic!("{relative}: {e}"))
}
const PAGES: &[&str] = &[
    "README.md",
    "docs/index.html",
    "docs/guide/install.md",
    "docs/guide/widgets.md",
    "docs/guide/accounts.md",
    "docs/guide/make-it-yours.md",
    "docs/guide/build-a-widget.md",
];
/// Targets of `](target)`, `href="target"` and `src="target"`, without the fragment.
fn targets(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (open, close) in [("](", ")"), ("href=\"", "\""), ("src=\"", "\"")] {
        let mut rest = text;
        while let Some(i) = rest.find(open) {
            rest = &rest[i + open.len()..];
            if let Some(j) = rest.find(close) {
                found.push(rest[..j].to_string());
            }
        }
    }
    found
        .into_iter()
        .filter(|t| {
            !t.is_empty()
                && !t.starts_with("http")
                && !t.starts_with('#')
                && !t.starts_with("mailto")
        })
        .map(|t| t.split('#').next().unwrap().to_string())
        .filter(|t| !t.is_empty() && !t.contains(' ') && !t.contains('{') && !t.contains('<'))
        .collect()
}

// REQ SQ-40
#[test]
fn every_relative_link_and_image_of_the_tutorial_exists() {
    for page in PAGES {
        let dir = Path::new(page).parent().unwrap();
        for target in targets(&read(page)) {
            let path = root().join(dir).join(&target);
            assert!(
                path.exists(),
                "{page} links to {target}, which does not exist"
            );
        }
    }
}

// REQ SQ-40
#[test]
fn every_registered_widget_and_every_named_variable_is_documented_and_real() {
    let widgets = read("docs/guide/widgets.md");
    let readme = read("README.md");
    for descriptor in sq_widgets::registry().descriptors() {
        let name = format!("`{}`", descriptor.kind);
        assert!(widgets.contains(&name), "widgets guide misses {name}");
        assert!(readme.contains(&name), "README misses {name}");
    }
    let mut source = String::new();
    for entry in walk(&root().join("crates")) {
        source.push_str(&fs::read_to_string(entry).unwrap_or_default());
    }
    for page in PAGES {
        let text = read(page);
        let mut rest = text.as_str();
        while let Some(i) = rest.find("STATUS_QUO_") {
            let name: String = rest[i..]
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || *c == '_')
                .collect();
            rest = &rest[i + name.len()..];
            assert!(
                source.contains(&format!("\"{name}\"")),
                "{page} names {name}, absent from the code"
            );
        }
    }
}
fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|n| n == "target" || n == "tests")
            {
                continue;
            }
            files.extend(walk(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files
}

// REQ SQ-40
#[test]
fn the_widget_code_in_the_guide_is_the_code_a_test_compiles() {
    let test = read("crates/tray/tests/custom_widget.rs");
    let code = test
        .split("// DOC-START\n")
        .nth(1)
        .and_then(|t| t.split("// DOC-END").next())
        .expect("markers in custom_widget.rs")
        .trim();
    assert!(
        read("docs/guide/build-a-widget.md").contains(code),
        "the guide's example drifted from the tested code"
    );
}

// REQ SQ-40
#[test]
fn the_tour_names_every_chapter_it_promises() {
    let readme = read("README.md");
    for chapter in [
        "The big picture",
        "Put it on your computer",
        "Your first minute",
        "The widgets",
        "Plug in your accounts",
        "Make it yours",
        "Build your own widget",
        "When something looks off",
    ] {
        assert!(readme.contains(chapter), "missing chapter {chapter}");
    }
    let page = read("docs/index.html");
    assert_eq!(
        page.matches("class=\"scene\"").count(),
        8,
        "the styled page has eight scenes"
    );
    for colour in ["#b76e79", "#c98b86", "#e8b9ab", "#c4c8cf", "#e9dcc6"] {
        assert!(
            page.contains(colour) || read("docs/img/hero.svg").contains(colour),
            "palette colour {colour} missing"
        );
    }
}
