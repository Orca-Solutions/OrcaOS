// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Scenario A harness and facilitator controls.
//!
//!   harness suite [--reps N] [--race-reps N] [--seed S] [--out DIR] [--only T,T]
//!   harness test T [--reps N] [--seed S] [--out DIR]
//!   harness cards RUN          compare every agent's card list with golem.yaml
//!   harness inspect RUN        the inspector's answers for one run
//!   harness ctl RUN OP ARGS    facilitator controls (see `ctl` below)
//!
//! Run from experiment/, with the Golem server, the deployed component and
//! the mock mail service up. Scorecard: excluded (test harness).

mod golem;
mod inspector;
mod mock;
mod record;
mod scenario;
mod tests;

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

use golem::Golem;
use mock::Mock;
use scenario::Run;
use tests::{Ctx, Rep};

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    format!("{secs:x}")
}

fn validity_ms() -> u64 {
    std::env::var("ORCA_VALIDITY_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(600_000)
}

struct Suite<'a> {
    ctx: Ctx<'a>,
    out: PathBuf,
    prefix: String,
    seed: u64,
    reps: Vec<Rep>,
}

impl Suite<'_> {
    fn run_test(&mut self, test: &str, reps: u32) {
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.out.join("repetitions.jsonl"))
            .expect("results file");

        for rep in 1..=reps {
            let run = format!("{}-{}-{rep}", self.prefix, test.replace('-', ""));
            let seed = self.seed.wrapping_add(rep as u64 * 7919).wrapping_add(test.len() as u64);
            let result = tests::repeat(&self.ctx, test, run, rep, seed);

            let failed: Vec<&str> = result.checks.iter().filter(|c| !c.pass).map(|c| c.name.as_str()).collect();
            println!(
                "test {test} rep {rep}/{reps}: {} ({} ms){}",
                if result.passed() { "pass" } else { "FAIL" },
                result.elapsed_ms,
                if failed.is_empty() { String::new() } else { format!(" failed: {failed:?}") }
            );
            writeln!(log, "{}", serde_json::to_string(&result).unwrap()).unwrap();
            self.reps.push(result);
        }
    }

    /// 7a, 7b and 7d finish only once their validity windows close.
    fn after_windows(&mut self) -> Vec<Value> {
        let waiting: Vec<&Rep> = self.reps.iter().filter(|r| matches!(r.test.as_str(), "7a" | "7b" | "7d")).collect();
        let Some(last) = waiting.iter().filter_map(|r| r.notes["not_after_ms"].as_u64()).max() else {
            return Vec::new();
        };

        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        // The adapter checks one step after each window closes, then backs off.
        let wait = last.saturating_sub(now) + 30_000;
        println!("waiting {} s for the last validity window to close", wait / 1000);
        std::thread::sleep(Duration::from_millis(wait));

        waiting
            .iter()
            .map(|rep| {
                let checks = tests::after_window(&self.ctx, rep);
                let pass = checks.iter().all(|c| c.pass);
                println!("test {} rep {} after the window: {}", rep.test, rep.rep, if pass { "pass" } else { "FAIL" });
                json!({ "test": rep.test, "rep": rep.rep, "run": rep.run, "pass": pass, "checks": checks })
            })
            .collect()
    }

    fn summary(&self, cards_before: &(bool, Value), cards_after: &(bool, Value), windows: &[Value]) -> String {
        let mut tests: Vec<String> = self.reps.iter().map(|r| r.test.clone()).collect();
        tests.dedup();

        let mut lines = vec![
            "# Scenario A on Golem: acceptance results".to_string(),
            String::new(),
            format!("Run prefix `{}`, base seed {}. Each test passes only if every check holds in every repetition.", self.prefix, self.seed),
            String::new(),
            format!("Card lists matched golem.yaml before the suite: {}; after: {}.", cards_before.0, cards_after.0),
            String::new(),
            "| Test | Repetitions | Passed | Result |".into(),
            "| --- | --- | --- | --- |".into(),
        ];
        for test in &tests {
            let reps: Vec<&Rep> = self.reps.iter().filter(|r| &r.test == test).collect();
            let passed = reps.iter().filter(|r| r.passed()).count();
            let window: Vec<&Value> = windows.iter().filter(|w| w["test"] == test.as_str()).collect();
            let window_pass = window.iter().filter(|w| w["pass"] == true).count();
            let all = passed == reps.len() && window_pass == window.len();
            let extra = if window.is_empty() { String::new() } else { format!(", after window {window_pass}/{}", window.len()) };
            lines.push(format!("| {test} | {} | {passed}{extra} | {} |", reps.len(), if all { "passed" } else { "failed" }));
        }
        lines.push(String::new());
        lines.push("Failed checks, if any, are listed below with their repetition; full detail is in repetitions.jsonl.".into());
        for r in self.reps.iter().filter(|r| !r.passed()) {
            for c in r.checks.iter().filter(|c| !c.pass) {
                lines.push(format!("- {} rep {} ({}): {}", r.test, r.rep, r.run, c.name));
            }
        }
        for w in windows.iter().filter(|w| w["pass"] == false) {
            lines.push(format!("- {} rep {} after the window ({})", w["test"].as_str().unwrap_or(""), w["rep"], w["run"].as_str().unwrap_or("")));
        }
        lines.join("\n") + "\n"
    }
}

fn suite(args: &[String], golem: &Golem, mock: &Mock, single: Option<&str>) {
    let reps: u32 = flag(args, "--reps").and_then(|v| v.parse().ok()).unwrap_or(20);
    let race_reps: u32 = flag(args, "--race-reps").and_then(|v| v.parse().ok()).unwrap_or(100);
    let seed: u64 = flag(args, "--seed").and_then(|v| v.parse().ok()).unwrap_or(20261020);
    let prefix = flag(args, "--prefix").unwrap_or_else(stamp);
    let out = PathBuf::from(flag(args, "--out").unwrap_or_else(|| format!("results/{prefix}")));
    std::fs::create_dir_all(&out).expect("results directory");

    let plan: Vec<(&str, u32)> = match single {
        Some(test) => vec![(test, reps)],
        None => {
            let all = [
                ("1", 1), ("2", 1), ("4", 1), ("4-offline", 1), ("5", 1), ("6", race_reps),
                ("3", reps), ("3-p1", reps), ("3-p2", reps),
                ("7a", reps), ("7b", reps), ("7c", reps), ("7d", reps), ("7e", reps),
            ];
            let only = flag(args, "--only").map(|o| o.split(',').map(str::to_string).collect::<Vec<_>>());
            all.into_iter().filter(|(t, _)| only.as_ref().is_none_or(|o| o.iter().any(|x| x == t))).collect()
        }
    };

    let record = record::reproducibility(golem, validity_ms(), seed);
    std::fs::write(out.join("record.json"), serde_json::to_string_pretty(&record).unwrap()).unwrap();

    // Before the run: every agent's card list against the declarations.
    let card_run = format!("{prefix}-cards");
    let probe = Run::new(card_run.clone(), golem, mock);
    let _ = probe.setup();
    let cards_before = record::check_cards(golem, &card_run);
    std::fs::write(out.join("cards-before.json"), serde_json::to_string_pretty(&cards_before.1).unwrap()).unwrap();
    if !cards_before.0 {
        println!("a card list does not match golem.yaml; stopping (see cards-before.json)");
        return;
    }

    let mut s = Suite { ctx: Ctx { golem, mock }, out: out.clone(), prefix: prefix.clone(), seed, reps: Vec::new() };
    for (test, n) in plan {
        s.run_test(test, n);
    }
    let windows = s.after_windows();
    std::fs::write(out.join("after-window.json"), serde_json::to_string_pretty(&windows).unwrap()).unwrap();

    let cards_after = record::check_cards(golem, &card_run);
    std::fs::write(out.join("cards-after.json"), serde_json::to_string_pretty(&cards_after.1).unwrap()).unwrap();
    std::fs::write(out.join("mock-logs.json"), serde_json::to_string(&mock.logs()).unwrap()).unwrap();

    let journals: Vec<Value> = s.reps.iter().map(|r| json!({ "run": r.run, "export": Run::new(r.run.clone(), golem, mock).journal() })).collect();
    std::fs::write(out.join("journals.json"), serde_json::to_string(&journals).unwrap()).unwrap();

    let summary = s.summary(&cards_before, &cards_after, &windows);
    std::fs::write(out.join("summary.md"), &summary).unwrap();
    print!("{summary}");
}

/// Facilitator controls for driving one run by hand.
fn ctl(run: &str, op: &str, rest: &[String], golem: &Golem, mock: &Mock) -> Result<String, String> {
    let r = Run::new(run.to_string(), golem, mock);
    let arg = |i: usize| rest.get(i).cloned().unwrap_or_default();

    match op {
        "setup" => r.setup().map(|v| v.to_string()),
        "process" => r.process_and_draft(),
        "show" => r.display(&arg(0)).map(|v| serde_json::to_string_pretty(&v).unwrap()),
        "approve" => r.approve(&arg(0), &arg(1)).map(|a| format!("{} expires {}", a.authorization, a.expires_at_ms)),
        "revoke" => r.supervisor("revoke", &[&arg(0)]),
        "grant" => r.supervisor("grant", &[&arg(0), &arg(1), &arg(2), &arg(3), &arg(4)]),
        "cancel" => r.supervisor("cancel", &[&arg(0)]),
        "close" => r.supervisor("close", &[&arg(0)]),
        "replace" => r.supervisor("replace", &[&arg(0), &arg(1), &r.agent2("Processor2", &arg(1)), &arg(2)]),
        "fault" => mock.fault(&arg(0), &arg(1), run, arg(2).parse().ok(), arg(3).parse().unwrap_or(0)).map(|_| "armed".into()),
        "clear" => mock.clear().map(|_| "cleared".into()),
        "hold" => mock.hold_at(&arg(0), run).map(|_| "armed".into()),
        "release" => {
            r.release_all();
            Ok("released".into())
        }
        "crash" => golem.kill_and_restart().map(|d| format!("restarted in {} ms", d.as_millis())),
        "pending" => r.call(&r.agent("MailAdapter"), "pending", &[]),
        "journal" => Ok(serde_json::to_string_pretty(&r.journal()).unwrap()),
        "deliver-changed" => {
            let a = r.journal();
            let auth = &a["journal"]["authorizations"]["auth-0001"];
            let mut message: Value = serde_json::from_str(&r.display(&arg(0))?["message"].to_string()).unwrap_or_default();
            message["to"] = json!(arg(1));
            r.deliver_directly("auth-0001", auth["expires_at_ms"].as_u64().unwrap_or(0), &message)
        }
        _ => Err(format!("unknown control {op}; see the source for the list")),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let golem = Golem::from_env();
    let mock = Mock::from_env();

    match args.first().map(String::as_str) {
        Some("suite") => suite(&args, &golem, &mock, None),
        Some("test") => suite(&args, &golem, &mock, args.get(1).map(String::as_str)),
        Some("cards") => {
            let (ok, rows) = record::check_cards(&golem, &args[1]);
            println!("{}\nmatch: {ok}", serde_json::to_string_pretty(&rows).unwrap());
        }
        Some("inspect") => {
            let export = Run::new(args[1].clone(), &golem, &mock).journal();
            let report = inspector::report(&export);
            println!("{}\n\n{}", inspector::say(&report), serde_json::to_string_pretty(&report).unwrap());
        }
        Some("ctl") if args.len() >= 3 => match ctl(&args[1], &args[2], &args[3..], &golem, &mock) {
            Ok(out) => println!("{out}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        _ => {
            eprintln!("usage: harness suite|test T|cards RUN|inspect RUN|ctl RUN OP ARGS (see src/main.rs)");
            std::process::exit(2);
        }
    }
}
