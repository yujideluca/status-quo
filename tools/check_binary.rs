//! Checks that a built program asks the computer for nothing a clean system lacks (SQ-47).
//!
//!   check-binary [--max-glibc 2.17] FILE...
//!
//! Windows programs (.exe): every library they import must be part of Windows itself. A
//! program that imports VCRUNTIME140.dll or another Visual C++ library does not start on a
//! clean Windows 10 or 11 (v0.1.0 to v0.1.2 shipped that way).
//! Linux programs: the only libraries linked in are the C library's own, and no function newer
//! than the given glibc version is used, so the program starts on old distributions too. The
//! drawing libraries are loaded later, by name, and installed by the installer.
//! No dependencies: the file formats (PE and ELF) are read here.
use std::{env, fs, process::ExitCode};

/// Libraries every Windows 10 and 11 has. Names are compared in lower case.
const WINDOWS_OWN: &[&str] = &[
    "advapi32.dll",
    "bcrypt.dll",
    "bcryptprimitives.dll",
    "comctl32.dll",
    "comdlg32.dll",
    "crypt32.dll",
    "d3d11.dll",
    "d3d12.dll",
    "d3dcompiler_47.dll",
    "dcomp.dll",
    "dwmapi.dll",
    "dxgi.dll",
    "gdi32.dll",
    "imm32.dll",
    "kernel32.dll",
    "ntdll.dll",
    "ole32.dll",
    "oleaut32.dll",
    "opengl32.dll",
    "powrprof.dll",
    "psapi.dll",
    "secur32.dll",
    "setupapi.dll",
    "shell32.dll",
    "shlwapi.dll",
    "user32.dll",
    "userenv.dll",
    "uxtheme.dll",
    "version.dll",
    "winmm.dll",
    "ws2_32.dll",
    "pdh.dll",
    "iphlpapi.dll",
    "netapi32.dll",
    "propsys.dll",
    "cfgmgr32.dll",
    "shcore.dll",
    "msimg32.dll",
    "winspool.drv",
    "combase.dll",
    "rpcrt4.dll",
];
/// The C library and what belongs to it; everything else must be loaded by name at run time.
const LINUX_OWN: &[&str] = &[
    "libc.so.6",
    "libm.so.6",
    "libgcc_s.so.1",
    "libdl.so.2",
    "libpthread.so.0",
    "librt.so.1",
    "libutil.so.1",
];

fn u16_at(b: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as usize)
}
fn u32_at(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize)
}
fn u64_at(b: &[u8], at: usize) -> Option<usize> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?) as usize)
}
fn text_at(b: &[u8], at: usize) -> Option<String> {
    let rest = b.get(at..)?;
    let end = rest.iter().position(|c| *c == 0)?;
    Some(String::from_utf8_lossy(&rest[..end]).into_owned())
}

/// The libraries a Windows program (PE file, 32 or 64 bit) imports.
fn windows_imports(b: &[u8]) -> Option<Vec<String>> {
    let pe = u32_at(b, 0x3c)?;
    if b.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let sections = u16_at(b, pe + 6)?;
    let optional = pe + 24;
    let optional_size = u16_at(b, pe + 20)?;
    // Data directories start after 96 bytes (PE32) or 112 bytes (PE32+); entry 1 is imports.
    let directories = optional
        + if u16_at(b, optional)? == 0x20b {
            112
        } else {
            96
        };
    let table = optional + optional_size;
    let offset = |rva: usize| -> Option<usize> {
        (0..sections).find_map(|i| {
            let s = table + i * 40;
            let (size, address, raw) = (u32_at(b, s + 8)?, u32_at(b, s + 12)?, u32_at(b, s + 20)?);
            (rva >= address && rva < address + size.max(1)).then(|| rva - address + raw)
        })
    };
    let mut names = Vec::new();
    // Entry 1: ordinary imports (20-byte records, name at +12). Entry 13: delayed imports
    // (32-byte records, name at +4).
    for (entry, record, name_at) in [(1, 20, 12), (13, 32, 4)] {
        let rva = u32_at(b, directories + entry * 8)?;
        if rva == 0 {
            continue;
        }
        let mut at = offset(rva)?;
        loop {
            let name = u32_at(b, at + name_at)?;
            if name == 0 {
                break;
            }
            names.push(text_at(b, offset(name)?)?.to_lowercase());
            at += record;
        }
    }
    Some(names)
}
/// The libraries a 64-bit little-endian Linux program (ELF) is linked against.
fn linux_needed(b: &[u8]) -> Option<Vec<String>> {
    if b.get(..6)? != b"\x7fELF\x02\x01" {
        return None;
    }
    let (table, size, count) = (u64_at(b, 0x28)?, u16_at(b, 0x3a)?, u16_at(b, 0x3c)?);
    let section = |i: usize| table + i * size;
    let mut names = Vec::new();
    for i in 0..count {
        let s = section(i);
        // SHT_DYNAMIC = 6; its sh_link names the string table.
        if u32_at(b, s + 4)? != 6 {
            continue;
        }
        let (at, length) = (u64_at(b, s + 0x18)?, u64_at(b, s + 0x20)?);
        let strings = u64_at(b, section(u32_at(b, s + 0x28)?) + 0x18)?;
        for entry in (at..at + length).step_by(16) {
            // DT_NEEDED = 1
            if u64_at(b, entry)? == 1 {
                names.push(text_at(b, strings + u64_at(b, entry + 8)?)?);
            }
        }
    }
    Some(names)
}
/// The newest glibc version named anywhere in the file ("GLIBC_2.34" -> (2, 34)).
fn newest_glibc(b: &[u8]) -> Option<(u32, u32)> {
    let mark = b"GLIBC_";
    let mut newest = None;
    for at in (0..b.len().saturating_sub(mark.len())).filter(|at| b[*at..].starts_with(mark)) {
        let text: String = b[at + mark.len()..]
            .iter()
            .take(12)
            .map(|c| *c as char)
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let mut parts = text.split('.').map(str::parse::<u32>);
        if let (Some(Ok(major)), Some(Ok(minor))) = (parts.next(), parts.next()) {
            newest = newest.max(Some((major, minor)));
        }
    }
    newest
}
fn windows_own(name: &str) -> bool {
    WINDOWS_OWN.contains(&name)
        // Windows' own API sets, except the C runtime ones, which come with Visual C++.
        || (name.starts_with("api-ms-win-") && !name.starts_with("api-ms-win-crt-"))
}
fn problems(file: &str, b: &[u8], max_glibc: (u32, u32)) -> Vec<String> {
    if let Some(imports) = windows_imports(b) {
        return imports
            .iter()
            .filter(|name| !windows_own(name))
            .map(|name| format!("{file}: needs {name}, which a clean Windows does not have"))
            .collect();
    }
    let Some(needed) = linux_needed(b) else {
        return vec![format!(
            "{file}: neither a Windows nor a 64-bit Linux program"
        )];
    };
    let mut found: Vec<String> = needed
        .iter()
        .filter(|name| !LINUX_OWN.contains(&name.as_str()) && !name.starts_with("ld-linux"))
        .map(|name| {
            format!(
                "{file}: is linked against {name}; it must be loaded by name at run time instead"
            )
        })
        .collect();
    if let Some(newest) = newest_glibc(b).filter(|newest| *newest > max_glibc) {
        found.push(format!(
            "{file}: needs glibc {}.{}, newer than the allowed {}.{}",
            newest.0, newest.1, max_glibc.0, max_glibc.1
        ));
    }
    found
}

fn main() -> ExitCode {
    let mut max_glibc = (2, 17);
    let mut files = Vec::new();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--max-glibc" {
            let value = args.next().unwrap_or_default();
            let mut parts = value.split('.').map(str::parse::<u32>);
            match (parts.next(), parts.next()) {
                (Some(Ok(major)), Some(Ok(minor))) => max_glibc = (major, minor),
                _ => {
                    eprintln!("--max-glibc needs a version such as 2.17");
                    return ExitCode::from(2);
                }
            }
        } else {
            files.push(arg);
        }
    }
    if files.is_empty() {
        eprintln!("usage: check-binary [--max-glibc 2.17] FILE...");
        return ExitCode::from(2);
    }
    let mut failed = false;
    for file in &files {
        let found = match fs::read(file) {
            Ok(bytes) => problems(file, &bytes, max_glibc),
            Err(error) => vec![format!("{file}: {error}")],
        };
        if found.is_empty() {
            println!("ok    {file}: asks only for what a clean system has");
        }
        for problem in &found {
            println!("FAIL  {problem}");
        }
        failed |= !found.is_empty();
    }
    ExitCode::from(u8::from(failed))
}
