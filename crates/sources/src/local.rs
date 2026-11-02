use crate::{memory_event, repository_remote, Cursor};
use serde::{Deserialize, Serialize};
use sq_core::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use walkdir::WalkDir;

pub fn expand(path: &str) -> PathBuf {
    if path == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return expand("~").join(rest);
    }
    PathBuf::from(path)
}
#[derive(Default, Serialize, Deserialize)]
pub struct LocalState {
    pub version: u32,
    pub ledger: TokenLedger,
    pub cursors: BTreeMap<String, Cursor>,
}
impl LocalState {
    /// Rebuild only derived Codex observations whose old sequence identities could collide.
    /// Original transcripts and retained Claude observations remain untouched.
    pub fn upgrade(&mut self) -> usize {
        if self.version >= 2 {
            return 0;
        }
        let before = self.ledger.records.len();
        self.ledger.records.retain(|_, r| r.provider != "codex");
        self.cursors.retain(|_, c| c.parser.provider != "codex");
        self.version = 2;
        before - self.ledger.records.len()
    }

    pub fn discover(&mut self, config: &SourceConfig, now: i64) -> Result<usize, String> {
        let roots = [
            expand(&config.claude).join("projects"),
            expand(&config.codex).join("sessions"),
        ];
        self.cursors
            .retain(|path, _| roots.iter().any(|root| Path::new(path).starts_with(root)));
        let mut discovered = 0;
        for (provider, root) in [
            ("claude", expand(&config.claude).join("projects")),
            ("codex", expand(&config.codex).join("sessions")),
        ] {
            if !root.exists() {
                continue;
            }
            for (visited, entry) in WalkDir::new(&root)
                .follow_links(false)
                .into_iter()
                .enumerate()
            {
                if visited > 100_000 {
                    return Err(
                        "Transcript discovery bound reached; remaining files deferred".into(),
                    );
                }
                let entry = entry.map_err(|e| format!("Transcript directory unreadable: {e}"))?;
                if !entry.file_type().is_file()
                    || entry.path().extension().is_none_or(|e| e != "jsonl")
                {
                    continue;
                }
                let metadata = entry.metadata().map_err(|e| e.to_string())?;
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                if modified < now - 30 * 86400 {
                    continue;
                }
                let key = entry.path().to_string_lossy().to_string();
                if let std::collections::btree_map::Entry::Vacant(slot) = self.cursors.entry(key) {
                    let id = entry
                        .path()
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy();
                    let mut cursor = Cursor::new(provider, &id);
                    if provider == "claude" {
                        if let Some(parent) = entry
                            .path()
                            .parent()
                            .filter(|p| p.file_name().is_some_and(|n| n == "subagents"))
                            .and_then(Path::parent)
                            .and_then(Path::file_name)
                        {
                            cursor.parser.parent = Some(parent.to_string_lossy().into());
                        }
                    }
                    slot.insert(cursor);
                    discovered += 1;
                }
            }
        }
        self.version = 2;
        Ok(discovered)
    }
    pub fn poll(&mut self, snapshot: &mut Snapshot, now: i64) -> bool {
        self.poll_selected(snapshot, now, None)
    }
    pub fn poll_selected(
        &mut self,
        snapshot: &mut Snapshot,
        now: i64,
        selected: Option<&BTreeSet<String>>,
    ) -> bool {
        let mut changed = false;
        let mut remaining = 16 * 1024 * 1024u64;
        let mut errors: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        // Small updated files first keeps an old import from starving current sessions.
        let mut keys: Vec<_> = self
            .cursors
            .keys()
            .filter(|key| {
                selected.is_none_or(|set| set.contains(*key)) || self.cursors[*key].deferred > 0
            })
            .cloned()
            .collect();
        keys.sort_by_key(|p| {
            fs::metadata(p)
                .map(|m| m.len().saturating_sub(self.cursors[p].offset))
                .unwrap_or(0)
        });
        for key in keys {
            let cursor = self.cursors.get_mut(&key).expect("known cursor");
            if remaining == 0 {
                cursor.deferred = 1;
                continue;
            }
            match cursor.poll(
                Path::new(&key),
                &mut self.ledger,
                remaining.min(2 * 1024 * 1024),
            ) {
                Ok(result) => {
                    remaining = remaining.saturating_sub(result.bytes);
                    changed |= result.bytes > 0;
                }
                Err(_) => {
                    errors
                        .entry(if cursor.parser.provider == "claude" {
                            "claude"
                        } else {
                            "codex"
                        })
                        .or_default()
                        .push(key);
                }
            }
        }
        let removed = self.ledger.prune(now - 30 * 86400);
        changed |= removed > 0;
        self.cursors
            .retain(|_, c| c.modified == 0 || c.modified >= now - 30 * 86400);
        if changed || snapshot.tokens.is_empty() {
            for provider in ["claude", "codex"] {
                match self
                    .ledger
                    .buckets(provider, now - 30 * 86400, now + 1, 3600)
                {
                    Ok(buckets) => {
                        snapshot.tokens.insert(provider.into(), buckets);
                    }
                    Err(error) => {
                        errors.entry(provider).or_default().push(error);
                    }
                }
            }
        }
        for provider in ["claude", "codex"] {
            let cursors: Vec<_> = self
                .cursors
                .values()
                .filter(|c| c.parser.provider == provider)
                .collect();
            let malformed = cursors.iter().map(|c| c.malformed).sum();
            let deferred = cursors.iter().map(|c| c.deferred).sum();
            let failures = errors.get(provider).map(Vec::len).unwrap_or(0);
            snapshot.health.insert(
                format!("{provider}-logs"),
                Health {
                    state: if failures > 0 {
                        SourceState::Stale
                    } else if cursors.is_empty() {
                        SourceState::Unavailable
                    } else {
                        SourceState::Fresh
                    },
                    last_success: snapshot
                        .health
                        .get(&format!("{provider}-logs"))
                        .and_then(|h| h.last_success)
                        .or(Some(now)),
                    malformed,
                    deferred,
                    message: format!(
                        "{} transcript files; {} unreadable; {} counter resets observed",
                        cursors.len(),
                        failures,
                        cursors.iter().map(|c| c.parser.resets).sum::<u64>()
                    ),
                    retry_at: None,
                },
            );
        }
        let mut sessions: BTreeMap<String, Session> = BTreeMap::new();
        for cursor in self.cursors.values() {
            let p = &cursor.parser;
            if p.updated_at < now - 30 * 86400 {
                continue;
            }
            let project = Path::new(&p.project)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown project".into());
            let key = format!("{}:{}", p.provider, p.session);
            let session = Session {
                id: p.session.clone(),
                assistant: p.provider.clone(),
                project,
                project_path: p.project.clone(),
                task: "Task label unavailable".into(),
                state: p.state.clone(),
                updated_at: p.updated_at,
                parent: p.parent.clone(),
                progress: None,
                children_supported: p.parent.is_some() || p.provider == "codex",
            };
            if sessions
                .get(&key)
                .is_none_or(|s| s.updated_at < session.updated_at)
            {
                sessions.insert(key, session);
            }
        }
        snapshot.sessions = sessions.into_values().collect();
        changed
    }
}
pub fn memory_feed(root: &Path, now: i64) -> (Vec<MemoryItem>, Health) {
    let mut items = Vec::new();
    let mut malformed = 0;
    let mut deferred = 0;
    if !root.join("events").is_dir() {
        return (
            items,
            Health {
                state: SourceState::Unavailable,
                message: "Configured memory events directory is unavailable".into(),
                ..Health::default()
            },
        );
    }
    for (i, entry) in WalkDir::new(root.join("events"))
        .follow_links(false)
        .into_iter()
        .enumerate()
    {
        if i >= 100_000 {
            deferred += 1;
            break;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                malformed += 1;
                continue;
            }
        };
        if !entry.file_type().is_file() || entry.path().extension().is_none_or(|s| s != "md") {
            continue;
        }
        let path = entry.path();
        let id = path.file_stem().unwrap_or_default().to_string_lossy();
        let result = entry
            .metadata()
            .map_err(|_| ())
            .and_then(|m| {
                if m.len() <= 1024 * 1024 {
                    Ok(())
                } else {
                    Err(())
                }
            })
            .and_then(|_| fs::read_to_string(path).map_err(|_| ()))
            .and_then(|text| memory_event(&text, &id, &path.to_string_lossy()).map_err(|_| ()));
        match result {
            Ok(item) => {
                if ["pref", "decision", "task", "done", "reopen", "archive"]
                    .contains(&item.kind.as_str())
                    && item.at >= now - 30 * 86400
                {
                    items.push(item);
                }
            }
            Err(_) => malformed += 1,
        }
    }
    items.sort_by_key(|m| std::cmp::Reverse(m.at));
    let hidden = items.len().saturating_sub(300);
    items.truncate(300);
    let health=Health{state:if malformed>0{SourceState::Error}else if items.is_empty(){SourceState::Empty}else{SourceState::Fresh},last_success:Some(now),malformed,deferred,
        message:format!("Read-only historical events · {} additional older matching events outside the 300-entry feed",hidden),retry_at:None};
    (items, health)
}
pub fn seed_repositories(roots: &[String]) -> Vec<String> {
    let mut repos = BTreeSet::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(expand(root)) else {
            continue;
        };
        for entry in entries.flatten().take(2000) {
            let config = entry.path().join(".git/config");
            let Ok(text) = fs::read_to_string(config) else {
                continue;
            };
            for line in text.lines() {
                if let Some((key, value)) = line.split_once('=') {
                    if key.trim() == "url" {
                        if let Some(repo) = repository_remote(value) {
                            repos.insert(repo);
                        }
                    }
                }
            }
        }
    }
    repos.into_iter().take(100).collect()
}
