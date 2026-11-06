use crate::{claude_usage, codex_usage};
use serde_json::{json, Value};
use sq_core::Usage;
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

pub fn fetch_claude(root: &Path, now: i64) -> Result<Usage, String> {
    let token = match std::env::var("CLAUDE_CODE_OAUTH_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
    {
        Some(token) => token,
        None => {
            let file = std::fs::File::open(root.join(".credentials.json"))
                .map_err(|_| "Claude OAuth credentials unavailable; sign in with Claude Code")?;
            let credentials: Value = serde_json::from_reader(file.take(1024 * 1024))
                .map_err(|_| "Claude credential file is malformed; original retained")?;
            credentials
                .pointer("/claudeAiOauth/accessToken")
                .and_then(Value::as_str)
                .ok_or("Claude OAuth access token unavailable")?
                .to_string()
        }
    };
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Cannot initialize TLS client")?;
    let response = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("User-Agent", "StatusQuo/0.1.0")
        .send()
        .map_err(|_| "Claude usage request failed or timed out; last value retained")?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Claude usage HTTP {}; sign-in/rate-limit status is independent of token totals",
            status.as_u16()
        ));
    }
    let value: Value = serde_json::from_reader(response.take(1024 * 1024))
        .map_err(|_| "Malformed or oversized Claude usage response")?;
    claude_usage(&value, now)
}
struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
/// Codex usage for the default account, or for the one whose home folder is `home`.
pub fn fetch_codex(home: Option<&Path>, now: i64) -> Result<Usage, String> {
    let mut command = Command::new(crate::program("codex"));
    if let Some(home) = home {
        command.env("CODEX_HOME", home);
    }
    let child = command
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Codex CLI unavailable")?;
    let mut child = ChildGuard(child);
    let mut input = child
        .0
        .stdin
        .take()
        .ok_or("Codex app-server input unavailable")?;
    let output = child
        .0
        .stdout
        .take()
        .ok_or("Codex app-server output unavailable")?;
    let (send, receive) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(output);
        loop {
            let mut line = Vec::new();
            match reader
                .by_ref()
                .take(1024 * 1024)
                .read_until(b'\n', &mut line)
            {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if line.last() != Some(&b'\n') {
                        break;
                    }
                    if send.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut request = |value: Value| -> Result<(), String> {
        writeln!(input, "{value}").map_err(|_| "Codex app-server stopped".into())
    };
    request(
        json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"status_quo","version":"0.1.0"},"capabilities":null}}),
    )?;
    let mut initialized = false;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let line = receive
            .recv_timeout(remaining)
            .map_err(|_| "Codex usage request timed out or server exited; last value retained")?;
        let value: Value =
            serde_json::from_slice(&line).map_err(|_| "Malformed Codex app-server response")?;
        match value.get("id").and_then(Value::as_u64) {
            Some(1) if !initialized => {
                if value.get("error").is_some() {
                    return Err("Codex app-server initialization rejected".into());
                }
                request(json!({"method":"initialized"}))?;
                request(json!({"id":2,"method":"account/rateLimits/read","params":null}))?;
                initialized = true;
            }
            Some(2) => {
                if value.get("error").is_some() {
                    return Err(
                        "Codex account usage unavailable; verify CLI sign-in and account access"
                            .into(),
                    );
                }
                return codex_usage(
                    value.get("result").ok_or("Missing Codex usage result")?,
                    now,
                );
            }
            _ => {}
        }
    }
}
/// Bounded read-only command invocation. Arguments never pass through a shell.
pub fn command_json(program: &str, args: &[&str], timeout: Duration) -> Result<Value, String> {
    command_json_env(program, args, &[], timeout)
}
/// Like `command_json`, with extra environment variables (the account folders of `gh` and `codex`).
pub fn command_json_env(
    program: &str,
    args: &[&str],
    env: &[(&str, &std::ffi::OsStr)],
    timeout: Duration,
) -> Result<Value, String> {
    let deadline = Instant::now() + timeout;
    let child = Command::new(crate::program(program))
        .args(args)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| format!("{program} is unavailable"))?;
    let mut child = ChildGuard(child);
    let output = child.0.stdout.take().ok_or("Command output unavailable")?;
    let (send, receive) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = output
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = send.send(result);
    });
    let bytes = receive
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| format!("{program} read timed out"))?
        .map_err(|_| "Command output could not be read")?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("Command response exceeds size limit".into());
    }
    let status = loop {
        if let Some(status) = child.0.try_wait().map_err(|_| "Cannot reap command")? {
            break status;
        }
        if Instant::now() >= deadline {
            return Err(format!("{program} exit timed out"));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if !status.success() {
        return Err(format!(
            "{program} rejected the read; verify access and sign-in"
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| "Malformed command JSON response".into())
}
