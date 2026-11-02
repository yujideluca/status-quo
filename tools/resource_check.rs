//! Observe explicit production PIDs; never guess application processes by name.
use std::{
    env, fs,
    process::Command,
    thread,
    time::{Duration, Instant},
};
fn sample(pid: u32) -> Result<(u64, u64, u64), String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|e| format!("PID {pid} stat: {e}"))?;
    let status = fs::read_to_string(format!("/proc/{pid}/status"))
        .map_err(|e| format!("PID {pid} status: {e}"))?;
    parse(&stat, &status)
}
fn parse(stat: &str, status: &str) -> Result<(u64, u64, u64), String> {
    let (_, tail) = stat
        .rsplit_once(") ")
        .ok_or("Malformed /proc stat command")?;
    let fields: Vec<_> = tail.split_whitespace().collect();
    let number = |index: usize| {
        fields
            .get(index)
            .ok_or("Missing /proc stat field")?
            .parse::<u64>()
            .map_err(|_| "Invalid /proc stat number")
    };
    let ticks = number(11)?
        .checked_add(number(12)?)
        .ok_or("CPU counter overflow")?;
    let start = number(19)?;
    let rss = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .ok_or("Missing VmRSS")?;
    let mut parts = rss.split_whitespace();
    let kb = parts
        .next()
        .ok_or("Missing RSS value")?
        .parse::<u64>()
        .map_err(|_| "Invalid RSS")?;
    if parts.next() != Some("kB") {
        return Err("Unexpected RSS unit".into());
    }
    Ok((ticks, kb.checked_mul(1024).ok_or("RSS overflow")?, start))
}
fn run() -> Result<(), String> {
    let args: Vec<_> = env::args().skip(1).collect();
    // REQ SQ-23: reader self-test only; the five-minute production measurement is recorded separately.
    if args.first().is_some_and(|s| s == "--self-test") {
        let mut fields = vec!["0"; 20];
        fields[0] = "S";
        fields[11] = "12";
        fields[12] = "8";
        fields[19] = "500";
        let stat = format!("123 (name with ) and spaces) {}", fields.join(" "));
        assert_eq!(
            parse(&stat, "VmRSS:\t1024 kB\n").unwrap(),
            (20, 1024 * 1024, 500)
        );
        assert!(parse(&stat, "VmRSS: 1024 bytes").is_err());
        assert!(parse("malformed", "VmRSS: 1 kB").is_err());
        assert!(parse(&stat, "Name: test").is_err());
        println!("Resource reader: spaced process names, CPU ticks, KiB units and malformed data verified");
        return Ok(());
    }
    let seconds = args
        .first()
        .ok_or("Usage: resource-check SECONDS COLLECTOR_PID PANEL_PID")?
        .parse::<u64>()
        .map_err(|_| "Invalid duration")?;
    if seconds == 0 || seconds > 3600 {
        return Err("Duration must be 1..3600 seconds".into());
    }
    let pids = args[1..]
        .iter()
        .map(|s| s.parse::<u32>().map_err(|_| "Invalid PID"))
        .collect::<Result<Vec<_>, _>>()?;
    if pids.len() != 2 || pids[0] == pids[1] || pids.contains(&0) {
        return Err("Supply exactly two distinct collector/panel PIDs".into());
    }
    let clock = Command::new("getconf")
        .arg("CLK_TCK")
        .output()
        .map_err(|e| e.to_string())?;
    if !clock.status.success() {
        return Err("getconf CLK_TCK failed".into());
    }
    let rate = String::from_utf8(clock.stdout)
        .map_err(|_| "Invalid clock output")?
        .trim()
        .parse::<f64>()
        .map_err(|_| "Invalid clock rate")?;
    if !rate.is_finite() || rate <= 0.0 {
        return Err("Invalid clock rate".into());
    }
    let initial = pids
        .iter()
        .map(|pid| sample(*pid))
        .collect::<Result<Vec<_>, _>>()?;
    let mut peak = initial.iter().map(|(_, rss, _)| *rss).sum::<u64>();
    let started = Instant::now();
    let duration = Duration::from_secs(seconds);
    loop {
        thread::sleep(Duration::from_secs(1).min(duration.saturating_sub(started.elapsed())));
        let latest = pids
            .iter()
            .map(|pid| sample(*pid))
            .collect::<Result<Vec<_>, _>>()?;
        for (before, after) in initial.iter().zip(&latest) {
            if before.2 != after.2 {
                return Err("A PID was reused during observation".into());
            }
        }
        peak = peak.max(latest.iter().map(|(_, rss, _)| *rss).sum());
        if started.elapsed() >= duration {
            let ticks = initial
                .iter()
                .zip(&latest)
                .map(|(a, b)| b.0.checked_sub(a.0).ok_or("CPU counter regressed"))
                .collect::<Result<Vec<_>, _>>()?
                .iter()
                .sum::<u64>();
            let elapsed = started.elapsed().as_secs_f64();
            let cpu = ticks as f64 / rate / elapsed * 100.0;
            let complete = seconds >= 300;
            let passed = complete && cpu < 1.0 && peak < 200 * 1024 * 1024;
            println!("{{\"duration_seconds\":{elapsed:.3},\"one_core_cpu_percent\":{cpu:.4},\"peak_combined_rss_bytes\":{peak},\"five_minute_observation\":{complete},\"passed\":{passed}}}");
            if !complete {
                return Err("Diagnostic interval shorter than the required five minutes; acceptance not established".into());
            }
            return if passed {
                Ok(())
            } else {
                Err("Resource acceptance target missed".into())
            };
        }
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Resource check: {error}");
        std::process::exit(1);
    }
}
