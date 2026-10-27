// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Surface 6, records (0.3). A record is an object with a source and a trust
//! label, read only inside its audience. Relayed content never creates
//! authority: nothing in a record's content can produce a grant, an approval
//! or an operation. Keeping a record inside the house is not consequential;
//! a record leaving the house is surface 7.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::runtime::*;
use crate::ledger::{Entry, Source};
use crate::model::*;

/// Who is reading or writing, and as which member.
struct Reader {
    app: Option<String>,
    member: String,
}

impl Runtime {
    fn reader(&self, who: &Who) -> Result<Reader, CallError> {
        let member = who.member.clone().ok_or_else(|| bad("records are read and written as a member: add `member` to the caller"))?;
        Ok(Reader { app: who.app.clone(), member })
    }

    fn parse_audience(&self, v: Option<&Value>) -> Result<Audience, CallError> {
        let Some(v) = v else {
            return Err(bad("missing `audience`: \"household\", {\"members\": [...]} or {\"guardians_of\": child}"));
        };
        let audience: Audience = serde_json::from_value(v.clone()).map_err(|e| bad(format!("the audience could not be read: {e}")))?;
        match &audience {
            Audience::Everyone(word) if word != "household" => Err(bad("the only named audience is \"household\"")),
            Audience::Members { members } => match members.iter().find(|m| self.household.member(m).is_none()) {
                Some(m) => Err(bad(format!("{m} is not a member of the household"))),
                None => Ok(audience),
            },
            Audience::Guardians { guardians_of } if !self.household.guardians.contains_key(guardians_of) => {
                Err(bad(format!("{guardians_of} has no guardian relation")))
            }
            _ => Ok(audience),
        }
    }

    /// The writer's own audience (G-5): for a member, the household; for a
    /// routine, the audience of the activity it runs in.
    fn writer_audience(&self, who: &Who) -> BTreeSet<String> {
        match who.kind {
            CallerKind::Routine => self.core.activity(who.activity.as_deref().unwrap_or_default()).unwrap().audience.clone(),
            _ => self.household.members.iter().map(|m| m.id.clone()).collect(),
        }
    }

    fn may_read(&self, reader: &Reader, record: &Record) -> Result<(), &'static str> {
        if let Some(app) = &reader.app
            && *app != record.app
            && !self.apps.get(app).is_some_and(|a| a.manifest.may_read(&record.app, &record.record_type))
        {
            return Err("read_scope_missing");
        }

        let audience = self.audience_members(&record.audience);
        let allowed_by_member = reader
            .app
            .as_ref()
            .is_some_and(|app| audience.iter().any(|m| self.app_read_allows.contains(&(app.clone(), m.clone()))));
        if audience.contains(&reader.member) || allowed_by_member {
            Ok(())
        } else {
            Err("outside_audience")
        }
    }

    pub fn write_record(&mut self, who: &Who, body: &Value) -> Res {
        let reader = self.reader(who)?;
        let record_type = s(body, "type")?;
        let content = body.get("content").cloned().unwrap_or(Value::Null);
        let provenance = body.get("provenance").cloned().unwrap_or(json!({}));
        let trust = opt_s(&provenance, "trust").unwrap_or_else(|| "untrusted".into());
        let audience = self.parse_audience(body.get("audience"))?;

        let app_id = match &reader.app {
            Some(a) => a.clone(),
            None => s(body, "app")?,
        };
        let app = self.running_app(&app_id)?.clone();
        let r = |code, text: String| refuse(code, text).app(&app_id).object(format!("record-type:{record_type}"));

        if app.manifest.record_type(&record_type).is_none() {
            return Err(r("record_type_undeclared", format!("{} declares no record type {record_type}", app.manifest.display_name())).into());
        }

        // Trust labels: relayed content is untrusted, only a member's own
        // words are labelled member, and only Diamond Dog writes service.
        let relayed = provenance.get("relayed_by").is_some_and(|v| !v.is_null());
        let trust_ok = match trust.as_str() {
            "untrusted" => true,
            "member" => !relayed && (who.is_shell() || opt_s(&provenance, "source").as_deref() == Some(reader.member.as_str())),
            _ => false,
        };
        if !trust_ok {
            return Err(r("trust_label_not_allowed", format!("a record {} cannot be labelled {trust}", if relayed { "relayed from outside" } else { "written by an app" })).into());
        }

        let own = self.writer_audience(who);
        let wanted = self.audience_members(&audience);
        if !wanted.is_subset(&own) {
            let extra: Vec<&String> = wanted.difference(&own).collect();
            return Err(r("audience_too_wide", format!("the audience reaches beyond the writer's own: {extra:?}")).into());
        }

        // A repeat with the same client key writes nothing (G-11).
        let client_key = opt_s(body, "client_key");
        if let Some(key) = &client_key
            && let Some(existing) = self.records.values().find(|x| x.app == app_id && x.client_key.as_ref() == Some(key))
        {
            return Ok(json!({ "object": existing.id, "version": existing.version, "repeat": true }));
        }

        let (id, version) = match opt_s(body, "object") {
            Some(id) => match self.records.get(&id) {
                Some(x) if x.app == app_id && !x.deleted => (id, x.version + 1),
                _ => return Err(bad(format!("{app_id} has no record {id} to update"))),
            },
            None => (self.next_id("R"), 1),
        };
        self.records.insert(
            id.clone(),
            Record {
                id: id.clone(),
                app: app_id.clone(),
                record_type: record_type.clone(),
                version,
                content,
                provenance: provenance.clone(),
                trust: trust.clone(),
                audience: audience.clone(),
                written_by: reader.member.clone(),
                at: self.now(),
                client_key,
                deleted: false,
            },
        );

        let from = if relayed {
            format!(", relayed by {}, labelled {trust}", provenance["relayed_by"].as_str().unwrap_or("an adapter"))
        } else {
            format!(", labelled {trust}")
        };
        let plain = format!("{} filed {record_type} {id} (version {version}){from}.", self.app_name(&app_id));
        let mut entry = Entry::new("record", who.source(), plain)
            .app(&app_id)
            .member(&reader.member)
            .object(format!("record:{id}"))
            .detail(json!({ "type": record_type, "version": version, "trust": trust, "audience": audience, "provenance": provenance }));
        if let Some(a) = &who.activity {
            entry = entry.activity(a);
        }
        self.journal(entry);
        if let Some(a) = &who.activity {
            self.touch(a);
        }

        Ok(json!({ "object": id, "version": version }))
    }

    pub fn read_records(&mut self, who: &Who, body: &Value) -> Res {
        let reader = self.reader(who)?;

        if let Some(id) = opt_s(body, "object") {
            let Some(record) = self.records.get(&id).cloned() else {
                return Err(bad(format!("there is no record {id}")));
            };
            if let Err(code) = self.may_read(&reader, &record) {
                let text = match code {
                    "read_scope_missing" => format!("{} has no read scope over {}'s {}", reader.app.clone().unwrap_or_default(), record.app, record.record_type),
                    _ => format!("{} is outside the record's audience", self.member_name(&reader.member)),
                };
                return Err(refuse(code, text).object(format!("record:{id}")).operation("records.read").into());
            }
            self.journal_read(who, &reader, &record);
            return Ok(json!({ "records": [record_view(&record)] }));
        }

        let record_type = opt_s(body, "type");
        let of_app = opt_s(body, "app");
        let found: Vec<Record> = self
            .records
            .values()
            .filter(|x| record_type.as_ref().is_none_or(|t| *t == x.record_type))
            .filter(|x| of_app.as_ref().is_none_or(|a| *a == x.app))
            .filter(|x| self.may_read(&reader, x).is_ok())
            .cloned()
            .collect();
        for record in &found {
            self.journal_read(who, &reader, record);
        }
        Ok(json!({ "records": found.iter().map(record_view).collect::<Vec<_>>() }))
    }

    /// Cross-app reads are journaled (G-3), and so is every read of a record
    /// whose audience is a guardian relation, where the guardians can see it (G-10).
    fn journal_read(&mut self, who: &Who, reader: &Reader, record: &Record) {
        let cross_app = reader.app.as_ref().is_some_and(|a| *a != record.app);
        let guardians = match &record.audience {
            Audience::Guardians { .. } => Some(self.audience_members(&record.audience).into_iter().collect::<Vec<_>>()),
            _ => None,
        };
        if !cross_app && guardians.is_none() {
            return;
        }

        let by = match &reader.app {
            Some(app) => format!("{} for {}", self.app_name(app), self.member_name(&reader.member)),
            None => self.member_name(&reader.member),
        };
        let plain = format!("{by} read {} {} ({}'s record).", record.record_type, record.id, self.app_name(&record.app));
        let mut entry = Entry::new("record_read", who.source(), plain)
            .member(&reader.member)
            .object(format!("record:{}", record.id))
            .detail(json!({ "record_app": record.app, "type": record.record_type, "cross_app": cross_app }));
        if let Some(app) = &reader.app {
            entry = entry.app(app);
        }
        if let Some(a) = &who.activity {
            entry = entry.activity(a);
        }
        entry.visible_to = guardians;
        self.journal(entry);
    }

    /// Promote (G-5): widens a record's audience inside the household. Not
    /// consequential, so no grant; journaled.
    pub fn promote(&mut self, who: &Who, body: &Value) -> Res {
        let reader = self.reader(who)?;
        let id = s(body, "object")?;
        let audience = self.parse_audience(body.get("audience"))?;
        let Some(record) = self.records.get(&id).cloned() else {
            return Err(bad(format!("there is no record {id}")));
        };
        let r = |code, text: String| refuse(code, text).object(format!("record:{id}")).operation("records.promote");

        if let Err(code) = self.may_read(&reader, &record) {
            return Err(r(code, "the record is outside the caller's reach".into()).into());
        }
        let own = self.writer_audience(who);
        if !self.audience_members(&audience).is_subset(&own) {
            return Err(r("audience_too_wide", "the new audience reaches beyond the writer's own".into()).into());
        }

        self.records.get_mut(&id).unwrap().audience = audience.clone();
        let plain = format!("{} widened who may read {} {id} inside the household.", self.describe(who), record.record_type);
        self.journal(
            Entry::new("promotion", who.source(), plain)
                .app(&record.app)
                .member(&reader.member)
                .object(format!("record:{id}"))
                .detail(json!({ "from": record.audience, "to": audience })),
        );
        Ok(json!({ "object": id, "audience": audience }))
    }

    /// Delete (G-9): the content is gone; the id and type stay, and receipts keep their reference.
    pub fn delete_record(&mut self, who: &Who, body: &Value) -> Res {
        let reader = self.reader(who)?;
        let id = s(body, "object")?;
        let Some(record) = self.records.get(&id).cloned() else {
            return Err(bad(format!("there is no record {id}")));
        };
        let r = |code, text: String| refuse(code, text).object(format!("record:{id}")).operation("records.delete");

        let writer = reader.app.as_deref() == Some(record.app.as_str());
        let owner = who.is_shell() && self.is_owner(&reader.member);
        if !writer && !owner {
            return Err(r("not_writer_or_owner", "only the writing app or the owner deletes a record".into()).into());
        }
        if !self.audience_members(&record.audience).contains(&reader.member) {
            return Err(r("outside_audience", format!("{} is outside the record's audience", self.member_name(&reader.member))).into());
        }

        let x = self.records.get_mut(&id).unwrap();
        x.content = Value::Null;
        x.provenance = Value::Null;
        x.deleted = true;

        let plain = format!("{} deleted {} {id}; its content is gone.", self.describe(who), record.record_type);
        self.journal(
            Entry::new("deletion", who.source(), plain)
                .app(&record.app)
                .member(&reader.member)
                .object(format!("record:{id}"))
                .detail(json!({ "type": record.record_type })),
        );
        Ok(json!({ "object": id, "deleted": true }))
    }

    /// A member lets an app's runs read records in their audience (G-2: asked
    /// once, at install or when the record type first appears).
    pub fn allow_app_read(&mut self, who: &Who, body: &Value) -> Res {
        let member = self.shell_member(who, "Letting an app read your records")?;
        let app = s(body, "app")?;
        self.installed(&app)?;
        self.app_read_allows.insert((app.clone(), member.clone()));

        let plain = format!("{} let {}'s runs read records in their audience.", self.member_name(&member), self.app_name(&app));
        self.journal(Entry::new("read_allowed", Source::principal(&member), plain).app(&app).member(&member));
        Ok(json!({ "app": app, "member": member }))
    }
}

pub fn record_view(r: &Record) -> Value {
    json!({
        "object": r.id,
        "app": r.app,
        "type": r.record_type,
        "version": r.version,
        "content": r.content,
        "provenance": r.provenance,
        "trust": r.trust,
        "audience": r.audience,
        "written_by": r.written_by,
        "at": stamp(r.at),
        "deleted": r.deleted,
    })
}
