use crate::{command_json_env, timestamp};
use serde_json::Value;
use sq_core::{validate_repository, Activity};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

pub fn github_records(
    repository: &str,
    kind: &str,
    value: &Value,
) -> Result<Vec<Activity>, String> {
    validate_repository(repository)?;
    let records = if kind == "checks" {
        value.get("check_runs").and_then(Value::as_array)
    } else {
        value.as_array()
    }
    .ok_or("Invalid GitHub activity collection")?;
    let mut result = Vec::new();
    for record in records {
        if kind == "issues" && record.get("pull_request").is_some() {
            continue;
        }
        let id = record
            .get("id")
            .map(Value::to_string)
            .or_else(|| record.get("sha").and_then(Value::as_str).map(String::from))
            .ok_or("GitHub activity has no identity")?;
        let (title, date) = match kind {
            "commits" => (
                record
                    .pointer("/commit/message")
                    .and_then(Value::as_str)
                    .and_then(|s| s.lines().next()),
                record
                    .pointer("/commit/committer/date")
                    .and_then(Value::as_str),
            ),
            "checks" => (
                record.get("name").and_then(Value::as_str),
                record
                    .get("completed_at")
                    .and_then(Value::as_str)
                    .or_else(|| record.get("started_at").and_then(Value::as_str)),
            ),
            "releases" => (
                record
                    .get("name")
                    .and_then(Value::as_str)
                    .or_else(|| record.get("tag_name").and_then(Value::as_str)),
                record.get("published_at").and_then(Value::as_str),
            ),
            _ => (
                record.get("title").and_then(Value::as_str),
                record.get("updated_at").and_then(Value::as_str),
            ),
        };
        let url = record
            .get("html_url")
            .and_then(Value::as_str)
            .ok_or("GitHub activity has no source URL")?;
        if !url.starts_with("https://github.com/") {
            return Err("Unexpected GitHub activity source URL".into());
        }
        let status = record
            .get("conclusion")
            .and_then(Value::as_str)
            .or_else(|| record.get("state").and_then(Value::as_str))
            .unwrap_or("");
        result.push(Activity {
            account: String::new(),
            id: format!("{repository}:{kind}:{id}"),
            repository: repository.into(),
            kind: kind.into(),
            title: format!(
                "{}{}",
                title.ok_or("GitHub activity has no title")?,
                if status.is_empty() {
                    String::new()
                } else {
                    format!(" · {status}")
                }
            ),
            at: date.map(timestamp).transpose()?.unwrap_or(0),
            url: url.into(),
        });
    }
    Ok(result)
}
pub struct GithubRead {
    pub items: Vec<Activity>,
    pub errors: Vec<String>,
    pub deferred: usize,
}
pub fn fetch_github(repositories: &[String]) -> GithubRead {
    fetch_github_as("github", None, repositories)
}
/// Reads the repositories with one `gh` login: the default one, or the one whose configuration
/// folder is `config_dir`. Every record carries `account`.
pub fn fetch_github_as(
    account: &str,
    config_dir: Option<&Path>,
    repositories: &[String],
) -> GithubRead {
    let env: Vec<(&str, &std::ffi::OsStr)> = config_dir
        .map(|d| vec![("GH_CONFIG_DIR", d.as_os_str())])
        .unwrap_or_default();
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut items = BTreeMap::new();
    let mut errors = Vec::new();
    let mut deferred = 0;
    for (index, repository) in repositories.iter().enumerate() {
        if Instant::now() >= deadline {
            deferred = repositories.len() - index;
            break;
        }
        if let Err(error) = validate_repository(repository) {
            errors.push(error);
            continue;
        }
        for kind in ["commits", "pulls", "issues", "releases", "checks"] {
            for page in 1..=2 {
                if Instant::now() >= deadline {
                    deferred += 1;
                    break;
                }
                let suffix = match kind {
                    "checks" => "commits/HEAD/check-runs",
                    _ => kind,
                };
                let endpoint = format!("repos/{repository}/{suffix}?per_page=20&page={page}");
                match command_json_env("gh", &["api", &endpoint], &env, Duration::from_secs(10))
                    .and_then(|value| github_records(repository, kind, &value))
                {
                    Ok(records) => {
                        let count = records.len();
                        for mut record in records {
                            record.account = account.into();
                            items.insert(format!("{account}|{}", record.id), record);
                        }
                        if count < 20 {
                            break;
                        }
                    }
                    Err(error) => {
                        errors.push(format!("{repository}/{kind}: {error}"));
                        break;
                    }
                }
            }
        }
    }
    let mut items: Vec<_> = items.into_values().collect();
    items.sort_by_key(|a| std::cmp::Reverse(a.at));
    items.truncate(500);
    GithubRead {
        items,
        errors,
        deferred,
    }
}
pub fn repository_remote(url: &str) -> Option<String> {
    let value = url
        .trim()
        .strip_prefix("git@github.com:")
        .or_else(|| url.trim().strip_prefix("https://github.com/"))?;
    let value = value.strip_suffix(".git").unwrap_or(value);
    validate_repository(value).ok()?;
    Some(value.into())
}
