// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The fixture runner. A fixture is a scripted scenario with a fixed
//! expected ledger, written as JSON so Orca Keel can write it in any
//! language. The format is in interfaces/diamond-dog/fake-fixtures.md.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::fake::{CallError, Fake};

pub struct Outcome {
    pub name: String,
    pub failures: Vec<String>,
    pub ledger: Vec<String>,
}

impl Outcome {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Runs one fixture in a fresh fake.
pub fn run(fixture: &Value) -> Outcome {
    let mut fake = Fake::new();
    let mut vars: BTreeMap<String, Value> = BTreeMap::new();
    let mut failures = Vec::new();
    let name = fixture["name"].as_str().unwrap_or("unnamed fixture").to_string();

    if let Some(setup) = fixture.get("setup")
        && let Err(e) = fake.control("setup", setup)
    {
        failures.push(format!("setup: {}", describe(&e)));
    }

    let steps = fixture["steps"].as_array().cloned().unwrap_or_default();
    for (i, step) in steps.iter().enumerate() {
        let step = substitute(step, &vars);
        let label = match (step.get("call"), step.get("control")) {
            (Some(c), _) => format!("step {} ({})", i + 1, c.as_str().unwrap_or_default()),
            (_, Some(c)) => format!("step {} (control {})", i + 1, c.as_str().unwrap_or_default()),
            _ => format!("step {}", i + 1),
        };
        let body = step.get("body").cloned().unwrap_or(json!({}));

        let result = if let Some(path) = step["call"].as_str() {
            match split_call(path) {
                Some((surface, version, call)) => fake.call(surface, version, call, &body),
                None => Err(CallError::Bad(format!("`{path}` is not surface/vX.Y/call"))),
            }
        } else if let Some(path) = step["control"].as_str() {
            fake.control(path, &body)
        } else {
            failures.push(format!("{label}: names neither `call` nor `control`"));
            continue;
        };

        let got = response(&result);
        let expect = step.get("expect").cloned().unwrap_or(json!("ok"));
        if let Err(why) = check_expect(&expect, &got) {
            failures.push(format!("{label}: {why}"));
            // A failed step makes later ones meaningless.
            break;
        }

        if let Some(saves) = step.get("save").and_then(Value::as_object) {
            let from = if got["ok"] == true { &got["result"] } else { &got["refused"] };
            for (var, path) in saves {
                match lookup(from, path.as_str().unwrap_or_default()) {
                    Some(v) => {
                        vars.insert(var.clone(), v.clone());
                    }
                    None => failures.push(format!("{label}: nothing at `{}` to save as {var}", path.as_str().unwrap_or_default())),
                }
            }
        }
    }

    let ledger: Vec<Value> = fake.ledger.iter().map(|e| serde_json::to_value(e).unwrap()).collect();

    // The expected ledger: each entry matches a later actual entry, in order.
    let expected = substitute(&fixture.get("expect_ledger").cloned().unwrap_or(json!([])), &vars);
    let mut from = 0;
    for (i, want) in expected.as_array().cloned().unwrap_or_default().iter().enumerate() {
        match ledger[from..].iter().position(|got| subset(want, got, "").is_ok()) {
            Some(at) => from += at + 1,
            None => {
                let near = ledger[from..]
                    .iter()
                    .filter(|got| got["kind"] == want["kind"])
                    .filter_map(|got| subset(want, got, "").err())
                    .next()
                    .map(|why| format!("; nearest entry of that kind differs at {why}"))
                    .unwrap_or_default();
                failures.push(format!("expected ledger entry {} not found after line {from}: {want}{near}", i + 1));
            }
        }
    }

    let forbidden = substitute(&fixture.get("forbid_ledger").cloned().unwrap_or(json!([])), &vars);
    for want in forbidden.as_array().cloned().unwrap_or_default() {
        if let Some(got) = ledger.iter().find(|got| subset(&want, got, "").is_ok()) {
            failures.push(format!("forbidden ledger entry present at line {}: {}", got["seq"], got["plain"]));
        }
    }

    if let Some(want) = fixture.get("expect_mock") {
        let want = substitute(want, &vars);
        if let Err(why) = subset(&want, &fake.mock.summary(), "mock") {
            failures.push(format!("mock services differ at {why}"));
        }
    }

    Outcome {
        name,
        failures,
        ledger: fake.ledger.iter().map(|e| format!("{:>3}  {}  {:<16} {}", e.seq, e.local, e.kind, e.plain)).collect(),
    }
}

pub fn response(result: &Result<Value, CallError>) -> Value {
    match result {
        Ok(v) => json!({ "ok": true, "result": v }),
        Err(CallError::Refused(r)) => json!({ "ok": false, "refused": { "code": r.code, "reason": r.text, "ledger_seq": r.seq } }),
        Err(CallError::Bad(text)) => json!({ "ok": false, "error": text }),
    }
}

fn describe(e: &CallError) -> String {
    match e {
        CallError::Refused(r) => format!("refused {}: {}", r.code, r.text),
        CallError::Bad(text) => text.clone(),
    }
}

pub fn split_call(path: &str) -> Option<(&str, &str, &str)> {
    let mut parts = path.trim_matches('/').split('/');
    let (surface, version, call) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    Some((surface, version.strip_prefix('v')?, call))
}

fn check_expect(expect: &Value, got: &Value) -> Result<(), String> {
    let summary = || match got.get("refused") {
        Some(r) => format!("refused {}: {}", r["code"].as_str().unwrap_or_default(), r["reason"].as_str().unwrap_or_default()),
        None => match got.get("error") {
            Some(e) => format!("error: {}", e.as_str().unwrap_or_default()),
            None => "ok".into(),
        },
    };

    match expect {
        Value::String(word) if word == "ok" => (got["ok"] == true).then_some(()).ok_or_else(|| format!("expected ok, got {}", summary())),
        Value::String(word) if word == "refused" => got.get("refused").map(|_| ()).ok_or_else(|| format!("expected a refusal, got {}", summary())),
        Value::String(word) if word == "error" => got.get("error").map(|_| ()).ok_or_else(|| format!("expected an error, got {}", summary())),
        Value::Object(o) if o.contains_key("refused") => {
            let code = got["refused"]["code"].as_str();
            if code.is_some() && (o["refused"].is_null() || o["refused"].as_str() == code) {
                Ok(())
            } else {
                Err(format!("expected refusal {}, got {}", o["refused"], summary()))
            }
        }
        Value::Object(o) if o.contains_key("ok") => {
            if got["ok"] != true {
                return Err(format!("expected ok, got {}", summary()));
            }
            subset(&o["ok"], &got["result"], "result")
        }
        other => Err(format!("`expect` is \"ok\", \"refused\", \"error\", {{\"refused\": code}} or {{\"ok\": {{...}}}}, not {other}")),
    }
}

/// Every field in `want` is in `got` with an equal value; arrays match
/// element by element and must be the same length.
pub fn subset(want: &Value, got: &Value, at: &str) -> Result<(), String> {
    match (want, got) {
        (Value::Object(w), Value::Object(g)) => {
            for (k, wv) in w {
                let path = if at.is_empty() { k.clone() } else { format!("{at}.{k}") };
                match g.get(k) {
                    Some(gv) => subset(wv, gv, &path)?,
                    None if wv.is_null() => {}
                    None => return Err(format!("{path}: missing")),
                }
            }
            Ok(())
        }
        (Value::Array(w), Value::Array(g)) => {
            if w.len() != g.len() {
                return Err(format!("{at}: {} items, expected {}", g.len(), w.len()));
            }
            for (i, (wv, gv)) in w.iter().zip(g).enumerate() {
                subset(wv, gv, &format!("{at}[{i}]"))?;
            }
            Ok(())
        }
        _ if want == got => Ok(()),
        _ => Err(format!("{at}: {got}, expected {want}")),
    }
}

fn lookup<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').filter(|p| !p.is_empty()).try_fold(v, |v, key| match key.parse::<usize>() {
        Ok(i) => v.get(i),
        Err(_) => v.get(key),
    })
}

/// Replaces `${name}` or `${name.path}` with a saved value. A string that is
/// only one reference takes the value itself, whatever its type.
fn substitute(v: &Value, vars: &BTreeMap<String, Value>) -> Value {
    let resolve = |reference: &str| -> Option<Value> {
        let (name, path) = reference.split_once('.').unwrap_or((reference, ""));
        lookup(vars.get(name)?, path).cloned()
    };

    match v {
        Value::String(text) => {
            if let Some(reference) = text.strip_prefix("${").and_then(|t| t.strip_suffix('}'))
                && !reference.contains("${")
                && let Some(value) = resolve(reference)
            {
                return value;
            }

            let mut out = String::new();
            let mut rest = text.as_str();
            while let Some(start) = rest.find("${") {
                out.push_str(&rest[..start]);
                let Some(len) = rest[start..].find('}') else { break };
                let reference = &rest[start + 2..start + len];
                match resolve(reference) {
                    Some(Value::String(s)) => out.push_str(&s),
                    Some(other) => out.push_str(&other.to_string()),
                    None => out.push_str(&rest[start..start + len + 1]),
                }
                rest = &rest[start + len + 1..];
            }
            out.push_str(rest);
            Value::String(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(|x| substitute(x, vars)).collect()),
        Value::Object(map) => Value::Object(map.iter().map(|(k, x)| (k.clone(), substitute(x, vars))).collect()),
        other => other.clone(),
    }
}
