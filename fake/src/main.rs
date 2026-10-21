// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The fake Diamond Dog: a local test double for the provisional contract.
//!
//! It answers the ten surfaces over HTTP, enforces the contract's refusals
//! and writes the ledger in the format meant to be kept. It is not a
//! runtime: it keeps everything in memory, runs nothing isolated, and trusts
//! whoever can reach its port.

mod control;
mod fake;
mod fixture;
mod http;
mod ledger;
mod mock;
mod model;
mod schedule;
mod surfaces;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
fake-diamond-dog: a local test double for the Diamond Dog provisional contract

usage:
  fake-diamond-dog serve [--port 7477] [--setup household.json]
  fake-diamond-dog check [--ledger] <fixture.json | directory>...

serve   answers the contract over HTTP on 127.0.0.1
check   runs fixtures, each in a fresh fake; --ledger prints each ledger";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => serve(&args[1..]),
        Some("check") => check(&args[1..]),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn serve(args: &[String]) -> ExitCode {
    let mut port = 7477;
    let mut fake = fake::Fake::new();
    let mut i = 0;
    while i < args.len() {
        match (args[i].as_str(), args.get(i + 1)) {
            ("--port", Some(p)) => match p.parse() {
                Ok(p) => port = p,
                Err(_) => return fail(&format!("{p} is not a port")),
            },
            ("--setup", Some(file)) => {
                let setup = match read_json(Path::new(file)) {
                    Ok(v) => v,
                    Err(e) => return fail(&e),
                };
                if let Err(e) = fake.control("setup", &setup) {
                    return fail(&format!("setup failed: {e:?}"));
                }
            }
            _ => return fail(USAGE),
        }
        i += 2;
    }

    match http::serve(&mut fake, port) {
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

fn read_json(path: &Path) -> Result<serde_json::Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{} is not JSON: {e}", path.display()))
}

fn fail(text: &str) -> ExitCode {
    eprintln!("{text}");
    ExitCode::from(2)
}
