//! Standalone build helper: rustc tools/build-web.rs -o /tmp/status-quo-build-web
use std::{
    env, fs,
    path::PathBuf,
    process::{Command, ExitCode},
};
fn run(mut command: Command) -> Result<(), String> {
    let status = command.status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Build command failed: {status}"))
    }
}
fn build() -> Result<(), String> {
    let root = env::current_dir().map_err(|e| e.to_string())?;
    if !root.join("crates/web/Cargo.toml").exists() {
        return Err("Run this helper from the Status Quo repository root".into());
    }
    let mut cargo = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo.env("CARGO_PROFILE_RELEASE_STRIP", "none");
    cargo.args([
        "build",
        "--locked",
        "-p",
        "sq-web",
        "--target",
        "wasm32-unknown-unknown",
        "--release",
    ]);
    run(cargo)?;
    let output = root.join("dist/web");
    fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    let target = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let mut bindgen =
        Command::new(env::var_os("WASM_BINDGEN").unwrap_or_else(|| "wasm-bindgen".into()));
    bindgen
        .arg(target.join("wasm32-unknown-unknown/release/sq_web.wasm"))
        .args(["--target", "web", "--no-typescript", "--out-dir"])
        .arg(output.join("pkg"));
    run(bindgen)?;
    fs::copy(
        root.join("crates/web/site/index.html"),
        output.join("index.html"),
    )
    .map_err(|e| e.to_string())?;
    println!("Browser sample built at {}", output.display());
    Ok(())
}
fn serve() -> Result<(), String> {
    use std::io::{Read, Write};
    let root = env::current_dir()
        .map_err(|e| e.to_string())?
        .join("dist/web");
    let listener = std::net::TcpListener::bind("127.0.0.1:8765").map_err(|e| e.to_string())?;
    println!("Synthetic browser sample: http://127.0.0.1:8765/");
    for connection in listener.incoming() {
        let Ok(mut stream) = connection else { continue };
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(5)));
        let mut request = [0u8; 4096];
        let Ok(n) = stream.read(&mut request) else {
            continue;
        };
        let request = String::from_utf8_lossy(&request[..n]);
        let mut first = request.lines().next().unwrap_or("").split_whitespace();
        let method = first.next().unwrap_or("");
        let path = first.next().unwrap_or("").split('?').next().unwrap_or("");
        let relative = if path == "/" {
            "index.html"
        } else {
            path.strip_prefix('/').unwrap_or("")
        };
        let valid = method == "GET"
            && !relative.is_empty()
            && relative.split('/').all(|p| {
                !p.is_empty() && p != ".." && p != "." && !p.contains('%') && !p.contains('\\')
            });
        let bytes = if valid {
            fs::read(root.join(relative)).ok()
        } else {
            None
        };
        let (status, body) = match bytes {
            Some(bytes) => ("200 OK", bytes),
            None => ("404 Not Found", b"Not found".to_vec()),
        };
        let mime = match relative.rsplit('.').next() {
            Some("wasm") => "application/wasm",
            Some("js") => "text/javascript",
            Some("html") => "text/html; charset=utf-8",
            _ => "application/octet-stream",
        };
        let header=format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",body.len());
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&body);
    }
    Ok(())
}
fn main() -> ExitCode {
    let result = if env::args().nth(1).as_deref() == Some("--serve") {
        serve()
    } else {
        build()
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
