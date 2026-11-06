//! Embeds the program to install. CI builds `status-quo.exe` first and names it in
//! `STATUS_QUO_PAYLOAD`; without it the installer is built empty and looks for a
//! `status-quo.exe` next to itself instead.
use std::{env, fs, path::PathBuf};

fn main() {
    let dest = PathBuf::from(env::var("OUT_DIR").unwrap()).join("payload.bin");
    println!("cargo:rerun-if-env-changed=STATUS_QUO_PAYLOAD");
    match env::var("STATUS_QUO_PAYLOAD") {
        Ok(path) if !path.is_empty() => {
            println!("cargo:rerun-if-changed={path}");
            fs::copy(&path, &dest).unwrap_or_else(|e| panic!("STATUS_QUO_PAYLOAD {path}: {e}"));
        }
        _ => fs::write(&dest, []).unwrap(),
    }
}
