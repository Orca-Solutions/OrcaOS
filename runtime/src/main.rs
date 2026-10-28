// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Diamond Dog, the runtime: an experimental skeleton.
//!
//! It answers the ten surfaces of the provisional contract over HTTP, as the
//! fake does, on an enforcement core of its own (`core/`): a journal written
//! to disk and hash-chained, single-use payload-bound authorizations
//! consumed consume-if-live at the journal writer, grants and revocation,
//! and the six-state activity lifecycle. The language and architecture of
//! the real runtime still wait on the experiment gate; this is where the
//! enforcement rules get built and tested against the same fixtures in the
//! meantime. See README.md for what it does not do yet.

mod control;
mod core;
mod fixture;
mod http;
mod ledger;
mod mock;
mod model;
mod runtime;
mod schedule;
mod surfaces;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::Utc;

use crate::core::journal::Journal;
use crate::runtime::Runtime;

const USAGE: &str = "\
diamond-dog: the Diamond Dog runtime (experimental skeleton)

usage:
  diamond-dog serve [--port 7478] [--setup household.json]
                    [--journal diamond-dog.journal | --in-memory] [--test-controls]
  diamond-dog check [--ledger] <fixture.json | directory>...
  diamond-dog verify <journal>

serve   answers the contract over HTTP on 127.0.0.1, journaling to a file
        (default diamond-dog.journal); with --test-controls it also answers
        the fixtures' test controls under /fake/ and its clock moves only
        when told to, otherwise it follows the wall clock
check   runs fixtures, each in a fresh runtime; --ledger prints each ledger
verify  checks a journal file's hash chain without starting anything";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => serve(&args[1..]),
        Some("check") => check(&args[1..]),
        Some("verify") => verify(&args[1..]),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn serve(args: &[String]) -> ExitCode {
    let mut port = 7478;
    let mut setup = None;
    let mut journal_path = Some(PathBuf::from("diamond-dog.journal"));
    let mut test_controls = false;

    let mut i = 0;
    while i < args.len() {
        match (args[i].as_str(), args.get(i + 1)) {
            ("--port", Some(p)) => match p.parse() {
                Ok(p) => port = p,
                Err(_) => return fail(&format!("{p} is not a port")),
            },
            ("--setup", Some(file)) => match read_json(Path::new(file)) {
                Ok(v) => setup = Some(v),
                Err(e) => return fail(&e),
            },
            ("--journal", Some(file)) => journal_path = Some(PathBuf::from(file)),
            ("--in-memory", _) => {
                journal_path = None;
                i += 1;
                continue;
            }
            ("--test-controls", _) => {
                test_controls = true;
                i += 1;
                continue;
            }
            _ => return fail(USAGE),
        }
        i += 2;
    }

    let mut runtime = match &journal_path {
        None => {
            eprintln!("the journal is kept in memory only: nothing survives a restart");
            Runtime::with_journal(Journal::in_memory(), Utc::now())
        }
        Some(path) => {
            let (journal, opened) = match Journal::open(path) {
                Ok(j) => j,
                Err(e) => return fail(&format!("Diamond Dog will not start on this journal: {e}")),
            };
            let mut runtime = Runtime::with_journal(journal, Utc::now());
            let report = runtime.core.recover(&opened);
            let receipts = runtime.restore_receipts();
            eprintln!("journal {}: {} lines, {receipts} receipts", path.display(), report.lines);
            if report.torn_tail_discarded {
                eprintln!("  a final line cut off mid-write never committed and was discarded");
            }
            for (what, ids) in [
                ("outcomes now unknown", &report.unknown),
                ("approvals voided", &report.approvals_voided),
                ("grants ended", &report.grants_ended),
                ("activities closed", &report.activities_closed),
            ] {
                if !ids.is_empty() {
                    eprintln!("  recovery: {what}: {}", ids.join(", "));
                }
            }
            runtime
        }
    };

    if let Some(setup) = &setup {
        let result = if test_controls { runtime.control("setup", setup) } else { runtime.configure(setup) };
        if let Err(e) = result {
            return fail(&format!("setup failed: {e:?}"));
        }
    }

    match http::serve(&mut runtime, port, test_controls) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

fn check(args: &[String]) -> ExitCode {
    let show_ledger = args.iter().any(|a| a == "--ledger");
    let mut files: Vec<PathBuf> = Vec::new();
    for arg in args.iter().filter(|a| !a.starts_with("--")) {
        let path = PathBuf::from(arg);
        if path.is_dir() {
            let Ok(entries) = std::fs::read_dir(&path) else {
                return fail(&format!("could not read {arg}"));
            };
            let mut found: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
            found.sort();
            files.extend(found);
        } else {
            files.push(path);
        }
    }
    if files.is_empty() {
        return fail("name at least one fixture file or directory");
    }

    let mut failed = 0;
    for file in &files {
        let outcome = match read_json(file) {
            Ok(fixture) => fixture::run(&fixture),
            Err(e) => {
                failed += 1;
                println!("FAIL  {}\n      {e}", file.display());
                continue;
            }
        };

        let word = if outcome.passed() { "pass" } else { "FAIL" };
        println!("{word}  {}  ({})", outcome.name, file.display());
        for f in &outcome.failures {
            println!("      {f}");
        }
        if show_ledger || !outcome.passed() {
            for line in &outcome.ledger {
                println!("        {line}");
            }
        }
        if !outcome.passed() {
            failed += 1;
        }
    }

    println!("\n{} of {} fixtures passed", files.len() - failed, files.len());
    if failed == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn verify(args: &[String]) -> ExitCode {
    let [path] = args else {
        return fail("name one journal file");
    };
    match Journal::verify_file(Path::new(path)) {
        Ok(opened) => {
            println!("{path}: {} lines, chain intact", opened.lines);
            if opened.torn_tail_discarded {
                println!("its final line was cut off mid-write and never committed; serve discards it on start");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{path}: {e}");
            ExitCode::FAILURE
        }
    }
}

fn read_json(path: &Path) -> Result<serde_json::Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{} is not JSON: {e}", path.display()))
}

fn fail(text: &str) -> ExitCode {
    eprintln!("{text}");
    ExitCode::from(2)
}
