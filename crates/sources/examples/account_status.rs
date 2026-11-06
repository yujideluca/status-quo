fn main() {
    let now = chrono::Utc::now().timestamp();
    let root = std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join(".claude");
    for (provider, result) in [
        ("claude", sq_sources::fetch_claude(&root, now)),
        ("codex", sq_sources::fetch_codex(None, now)),
    ] {
        match result {
            Ok(usage) => {
                for window in usage.windows {
                    println!(
                        "{provider}: {} minutes / {}% used / {}",
                        window.minutes, window.used, window.label
                    )
                }
            }
            Err(error) => println!("{provider}: {error}"),
        }
    }
}
