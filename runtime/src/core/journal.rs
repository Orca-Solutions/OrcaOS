// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The journal writer: the single point of order (Decision log, 2026-10-20).
//!
//! Every ledger line is appended here and nowhere else. A line is committed
//! when it is written and flushed to the journal file; only then does the
//! enforcement core change what it holds in memory, so a revocation takes
//! effect when its record commits and a consumption exists only once its
//! record does.
//!
//! On disk the journal is one JSON object per line, each carrying the hash
//! of the line before it. The chain makes an edit, a reorder or a dropped
//! line in the middle of the file show up when the journal is opened. It is
//! tamper-evident, not tamper-proof: whoever can rewrite the whole file can
//! rewrite the chain too.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::hash;
use crate::ledger::Entry;

/// The chain value before the first line.
pub const GENESIS: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// One line of the journal file.
#[derive(Serialize, Deserialize)]
struct Line {
    seq: u64,
    prev: String,
    hash: String,
    entry: Entry,
}

/// What opening an existing journal found.
#[derive(Debug, Default)]
pub struct Opened {
    pub lines: u64,
    /// A final line that was cut off mid-write: it never committed, so it
    /// was discarded.
    pub torn_tail_discarded: bool,
}

pub struct Journal {
    entries: Vec<Entry>,
    head: String,
    file: Option<(PathBuf, File)>,
}

impl Journal {
    /// A journal kept in memory only, for fixtures and tests.
    pub fn in_memory() -> Self {
        Self {
            entries: Vec::new(),
            head: GENESIS.into(),
            file: None,
        }
    }

    /// Opens the journal file, creating it when it does not exist, and
    /// verifies every line's chain before anything is appended. A final line
    /// cut off mid-write never committed, so it is cut from the file.
    pub fn open(path: &Path) -> Result<(Self, Opened), String> {
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(path)
            .map_err(|e| format!("could not open the journal {}: {e}", path.display()))?;

        let (mut journal, opened, committed_bytes) = Self::read(&file)?;
        if opened.torn_tail_discarded {
            file.set_len(committed_bytes).map_err(|e| format!("could not discard the torn final line: {e}"))?;
            file.sync_all().map_err(|e| format!("could not flush the journal: {e}"))?;
        }

        journal.file = Some((path.to_path_buf(), file));
        Ok((journal, opened))
    }

    /// Checks a journal file's chain without changing it.
    pub fn verify_file(path: &Path) -> Result<Opened, String> {
        let file = File::open(path).map_err(|e| format!("could not open the journal {}: {e}", path.display()))?;
        Self::read(&file).map(|(_, opened, _)| opened)
    }

    /// Reads and verifies every committed line. Returns the journal, what was
    /// found, and how many bytes the committed lines take.
    fn read(file: &File) -> Result<(Self, Opened, u64), String> {
        let mut journal = Self::in_memory();
        let mut opened = Opened::default();
        let mut committed_bytes: u64 = 0;

        let mut reader = BufReader::new(file);
        loop {
            let mut raw = String::new();
            let read = reader.read_line(&mut raw).map_err(|e| format!("could not read the journal: {e}"))?;
            if read == 0 {
                break;
            }

            // A line without its newline was cut off before it was flushed:
            // it never committed, and nothing acted on it.
            if !raw.ends_with('\n') {
                opened.torn_tail_discarded = true;
                break;
            }

            let line: Line = serde_json::from_str(raw.trim_end()).map_err(|e| format!("journal line {} cannot be read: {e}", journal.entries.len() + 1))?;
            journal.verify(&line)?;
            journal.head = line.hash;
            journal.entries.push(line.entry);
            committed_bytes += read as u64;
        }

        opened.lines = journal.entries.len() as u64;
        Ok((journal, opened, committed_bytes))
    }

    fn verify(&self, line: &Line) -> Result<(), String> {
        let seq = self.entries.len() as u64 + 1;
        if line.seq != seq || line.entry.seq != seq {
            return Err(format!("journal line {seq} is numbered {}; a line is missing or out of order", line.seq));
        }
        if line.prev != self.head {
            return Err(format!("journal line {seq} does not follow line {}: the chain is broken", seq - 1));
        }
        if line.hash != chain(&line.prev, &line.entry) {
            return Err(format!("journal line {seq} was changed after it was written: its hash does not match"));
        }
        Ok(())
    }

    /// Appends one line and returns its number. The line is flushed to the
    /// file before this returns. If it cannot be, the runtime stops here
    /// rather than act on something the journal does not hold.
    pub fn append(&mut self, mut entry: Entry) -> u64 {
        let seq = self.entries.len() as u64 + 1;
        entry.seq = seq;
        let hash = chain(&self.head, &entry);

        if let Some((path, file)) = &mut self.file {
            let line = Line {
                seq,
                prev: self.head.clone(),
                hash: hash.clone(),
                entry: entry.clone(),
            };
            let mut text = serde_json::to_string(&line).expect("a ledger line is always JSON");
            text.push('\n');
            if let Err(e) = file.write_all(text.as_bytes()).and_then(|_| file.sync_data()) {
                panic!("fail-stop: the journal {} could not be written ({e}); Diamond Dog stops rather than act without a record", path.display());
            }
        }

        self.head = hash;
        self.entries.push(entry);
        seq
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn is_durable(&self) -> bool {
        self.file.is_some()
    }

    /// The factory reset (G-9): the journal is emptied, and the reset itself
    /// leaves no line.
    pub fn wipe(&mut self) {
        if let Some((path, file)) = &mut self.file
            && let Err(e) = file.set_len(0).and_then(|_| file.sync_all())
        {
            panic!("fail-stop: the journal {} could not be wiped ({e})", path.display());
        }
        self.entries.clear();
        self.head = GENESIS.into();
    }
}

/// The chain value of a line: the hash of the line before it and this line's
/// entry, canonical JSON with sorted keys.
fn chain(prev: &str, entry: &Entry) -> String {
    hash(&json!({ "prev": prev, "entry": entry }))
}
