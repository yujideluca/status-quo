use crate::LogParser;
use serde::{Deserialize, Serialize};
use sq_core::TokenLedger;
use std::os::unix::fs::MetadataExt;
use std::{
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::Path,
};
const MAX_LINE: usize = 8 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cursor {
    pub offset: u64,
    pub inode: u64,
    pub parser: LogParser,
    pub malformed: u64,
    pub deferred: u64,
    pub modified: i64,
    pub discarding: bool,
    #[serde(default)]
    pub rejection_reasons: std::collections::BTreeMap<String, u64>,
}
#[derive(Default, Debug)]
pub struct Poll {
    pub malformed: u64,
    pub deferred: u64,
    pub rotated: bool,
    pub bytes: u64,
    pub changed: bool,
}
impl Cursor {
    pub fn new(provider: &str, id: &str) -> Self {
        Self {
            offset: 0,
            inode: 0,
            parser: LogParser::new(provider, id),
            malformed: 0,
            deferred: 0,
            modified: 0,
            discarding: false,
            rejection_reasons: std::collections::BTreeMap::new(),
        }
    }
    fn reject(&mut self, reason: &str) {
        let key: String = reason.chars().take(200).collect();
        if self.rejection_reasons.len() < 16 || self.rejection_reasons.contains_key(&key) {
            let count = self.rejection_reasons.entry(key).or_default();
            *count = count.saturating_add(1);
        }
    }
    pub fn poll(
        &mut self,
        path: &Path,
        ledger: &mut TokenLedger,
        budget: u64,
    ) -> Result<Poll, String> {
        let file = File::open(path).map_err(|e| format!("Open transcript: {e}"))?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        let mut result = Poll::default();
        if metadata.ino() != self.inode || metadata.len() < self.offset {
            result.rotated = self.inode != 0;
            self.offset = 0;
            self.discarding = false;
            if self.parser.provider == "claude" {
                let parent = self.parser.parent.clone();
                self.parser = LogParser::new(&self.parser.provider, &self.parser.session);
                self.parser.parent = parent;
            }
            self.inode = metadata.ino();
        }
        if metadata.len() == self.offset {
            self.deferred = 0;
            return Ok(result);
        }
        let mut reader = BufReader::new(file);
        reader
            .seek(SeekFrom::Start(self.offset))
            .map_err(|e| e.to_string())?;
        while result.bytes < budget {
            let mut line = Vec::new();
            // Bound even a malicious single line. Overlong records are skipped visibly,
            // in chunks, without keeping conversation content in the checkpoint.
            let count = std::io::Read::take(&mut reader, (MAX_LINE + 1) as u64)
                .read_until(b'\n', &mut line)
                .map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            let complete = line.last() == Some(&b'\n');
            if !complete && count <= MAX_LINE && !self.discarding {
                result.deferred = 1;
                break;
            }
            self.offset += count as u64;
            result.bytes += count as u64;
            if count > MAX_LINE || self.discarding {
                if !self.discarding {
                    result.malformed += 1;
                    self.malformed += 1;
                }
                self.discarding = !complete;
                continue;
            }
            let parsed = serde_json::from_slice(&line)
                .map_err(|_| "Malformed JSON record".to_string())
                .and_then(|value| self.parser.ingest(&value));
            match parsed {
                Ok(Some(record)) => match ledger.insert(record) {
                    Ok(changed) => result.changed |= changed,
                    Err(_) => {
                        self.reject("Regressive/colliding token observation");
                        result.malformed += 1;
                        self.malformed += 1;
                    }
                },
                Ok(None) => {}
                Err(error) => {
                    self.reject(&error);
                    result.malformed += 1;
                    self.malformed += 1;
                }
            }
        }
        if self.offset < metadata.len() {
            result.deferred = result.deferred.max(1);
        }
        self.deferred = result.deferred;
        self.modified = metadata.mtime();
        Ok(result)
    }
}
