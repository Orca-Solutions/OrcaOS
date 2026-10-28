// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Surface 1, install (0.2): the manifest is the ceiling on everything an app
//! can later ask for, and off means off.

use serde_json::{Value, json};

use crate::runtime::*;
use crate::ledger::{Entry, Source};
use crate::model::*;

impl Runtime {
    pub fn install(&mut self, who: &Who, body: &Value) -> Res {
        let member = self.shell_member(who, "Installing an app")?;
        if !self.is_owner(&member) {
            return Err(refuse("not_owner", "only the owner installs apps").into());
        }

        let manifest: Manifest = serde_json::from_value(body.get("manifest").cloned().unwrap_or_default())
            .map_err(|e| bad(format!("the manifest could not be read: {e}")))?;
        let app = manifest.app.clone();
        if self.apps.get(&app).is_some_and(|a| a.installed) {
            return Err(refuse("already_installed", format!("{} is already installed", manifest.display_name())).app(&app).into());
        }

        for (surface, version) in &manifest.surfaces {
            let answers = SURFACES.iter().any(|(name, current)| name == surface && version_answers(current, version));
            if !answers {
                return Err(refuse("surface_version_unsupported", format!("the manifest pins {surface} {version}, which this Diamond Dog does not answer"))
                    .app(&app)
                    .into());
            }
        }

        for op in &manifest.operations {
            match adapter_of(&op.operation) {
                Some((adapter, _)) if adapter == op.adapter => {}
                _ => {
                    return Err(refuse(
                        "no_adapter_contract",
                        format!("there is no adapter contract for {} through a {} adapter", op.operation, op.adapter),
                    )
                    .app(&app)
                    .operation(&op.operation)
                    .object(&op.object)
                    .into());
                }
            }
        }

        if let Some(p) = manifest.providers.iter().find(|p| p.memory_spans_activities) {
            return Err(refuse(
                "hosted_model_rule",
                format!("provider {} keeps memory across activities, which the hosted-model rule refuses", p.name),
            )
            .app(&app)
            .into());
        }

        let name = manifest.display_name();
        let read_scopes = json!(manifest.read_scopes);
        let operations: Vec<&str> = manifest.operations.iter().map(|o| o.operation.as_str()).collect();
        let detail = json!({ "version": manifest.version, "operations": operations, "read_scopes": read_scopes, "surfaces": manifest.surfaces });

        self.apps.insert(
            app.clone(),
            App {
                manifest,
                installed: true,
                on: false,
                paused: false,
                switched_on_by: None,
                allowance: None,
            },
        );

        let plain = format!("{} installed {name}; it is switched off until a member switches it on.", self.member_name(&member));
        self.journal(Entry::new("install", Source::principal(&member), plain).app(&app).member(&member).detail(detail));

        Ok(json!({ "app": app, "on": false, "read_scopes_granted": read_scopes }))
    }

    pub fn switch_on(&mut self, who: &Who, body: &Value) -> Res {
        let member = self.shell_member(who, "Switching an app on")?;
        let app_id = s(body, "app")?;
        let app = self.installed(&app_id)?.clone();
        self.may_switch(&member, &app).map_err(|e| with_app(e, &app_id))?;

        // Check every standing grant asked for before changing anything.
        let asked = body.get("standing_grants").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut wanted = Vec::new();
        for g in &asked {
            let operation = s(g, "operation")?;
            let object = s(g, "object")?;
            let decl = app.manifest.operation(&operation).filter(|d| d.grant == GrantKind::Standing);
            let Some(decl) = decl else {
                return Err(refuse("operation_not_in_manifest", format!("{} does not declare {operation} as a standing operation", app.manifest.display_name()))
                    .app(&app_id)
                    .operation(operation)
                    .object(object)
                    .into());
            };
            if !object_matches(&decl.object, &object) {
                return Err(refuse("object_outside_manifest", format!("{object} is outside the manifest's {}", decl.object))
                    .app(&app_id)
                    .operation(operation)
                    .object(object)
                    .into());
            }
            wanted.push((operation, object, time(g, "expires_at")?));
        }

        {
            let a = self.apps.get_mut(&app_id).unwrap();
            a.on = true;
            a.paused = false;
            a.switched_on_by = Some(member.clone());
        }
        let plain = format!(
            "{} switched {} on; its scheduled runs act for {}.",
            self.member_name(&member),
            app.manifest.display_name(),
            self.member_name(&member)
        );
        self.journal(Entry::new("switch_on", Source::principal(&member), plain).app(&app_id).member(&member));

        let mut grants = Vec::new();
        for (operation, object, expires_at) in wanted {
            let gid = self.make_grant(Holder::App(app_id.clone()), &app_id, &operation, &object, &member, GrantKind::Standing, None, expires_at, None, false);
            grants.push(gid);
        }

        // Held install grants are shown again; the ones the member confirms go
        // live. Only a grant's own principal can confirm it; anyone else's stays held.
        let confirm = strings(body, "confirm_held");
        let held: Vec<Grant> = self.core.grants().filter(|g| g.app == app_id && g.state() == GrantState::Held).cloned().collect();
        let mut still_held = Vec::new();
        for g in held {
            let named = confirm.contains(&g.id) || confirm.iter().any(|c| c == "all");
            if named && g.principal == member {
                let plain = format!("{} confirmed install grant {} ({} on {}) again.", self.member_name(&member), g.id, g.operation, g.object);
                let entry = Entry::new("grant_confirmed", Source::principal(&member), plain)
                    .app(&app_id)
                    .member(&member)
                    .holder(g.holder.label())
                    .grant(&g.id)
                    .operation(&g.operation)
                    .object(&g.object);
                self.core.change_grant(&g.id, GrantState::Live, entry).expect("a held grant can be confirmed");
                grants.push(g.id);
            } else {
                still_held.push(g.id);
            }
        }

        let standing: Vec<&str> = app
            .manifest
            .operations
            .iter()
            .filter(|o| o.grant == GrantKind::Standing)
            .map(|o| o.operation.as_str())
            .collect();
        Ok(json!({ "app": app_id, "on": true, "grants": grants, "held_for_confirmation": still_held, "standing_operations": standing }))
    }

    pub fn switch_off(&mut self, who: &Who, body: &Value) -> Res {
        let member = self.shell_member(who, "Switching an app off")?;
        let app_id = s(body, "app")?;
        let app = self.installed(&app_id)?.clone();
        self.may_switch(&member, &app).map_err(|e| with_app(e, &app_id))?;

        let plain = format!("{} switched {} off: no process, no live grants, no egress.", self.member_name(&member), app.manifest.display_name());
        self.journal(Entry::new("switch_off", Source::principal(&member), plain).app(&app_id).member(&member));
        self.turn_off(&app_id, false, &member, false);

        Ok(json!({ "app": app_id, "on": false }))
    }

    pub fn uninstall(&mut self, who: &Who, body: &Value) -> Res {
        let member = self.shell_member(who, "Uninstalling an app")?;
        let app_id = s(body, "app")?;
        let name = self.installed(&app_id)?.manifest.display_name();
        if !self.is_owner(&member) {
            return Err(refuse("not_owner", "only the owner uninstalls apps").app(&app_id).into());
        }

        self.turn_off(&app_id, false, &member, true);
        self.apps.get_mut(&app_id).unwrap().installed = false;
        self.schedules.retain(|_, sch| sch.app != app_id);

        let plain = format!("{} uninstalled {name}; its records stay under their audience and its ledger stays.", self.member_name(&member));
        self.journal(Entry::new("uninstall", Source::principal(&member), plain).app(&app_id).member(&member));

        Ok(json!({ "app": app_id, "installed": false }))
    }

    pub fn read_app(&mut self, who: &Who, body: &Value) -> Res {
        let app_id = opt_s(body, "app").or_else(|| who.app.clone()).ok_or_else(|| bad("missing `app`"))?;
        let app = self.installed(&app_id)?;
        Ok(json!({
            "app": app_id,
            "on": app.on,
            "paused": app.paused,
            "switched_on_by": app.switched_on_by,
            "manifest": app.manifest,
            "allowance": app.allowance,
        }))
    }

    /// Creates a live grant and journals it. Callers have done the checks.
    #[allow(clippy::too_many_arguments)]
    pub fn make_grant(
        &mut self,
        holder: Holder,
        app: &str,
        operation: &str,
        object: &str,
        principal: &str,
        kind: GrantKind,
        bound_hash: Option<String>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
        derived_from: Option<String>,
        drawn: bool,
    ) -> String {
        let gid = self.next_id("G");
        let grant = Grant::new(gid.clone(), holder.clone(), app, operation, object, principal, kind, bound_hash.clone(), expires_at, derived_from.clone());

        let (recipient, kind_word) = match (&holder, kind) {
            (Holder::App(app), _) => (self.app_name(app), "an install".to_string()),
            (Holder::Activity(a), GrantKind::Standing) => (format!("activity {a}"), "a standing".to_string()),
            (Holder::Activity(a), GrantKind::OneTime) => (format!("activity {a}"), "a one-time".to_string()),
            (Holder::Support(s), _) => (format!("support person {s}"), "a time-boxed".to_string()),
        };
        let until = expires_at.map(|t| format!(" until {}", crate::schedule::local(&self.household.zone, t))).unwrap_or_default();
        let (source, plain) = match (&derived_from, drawn) {
            (Some(from), true) => (
                Source::service("supervisor"),
                format!("{} drew grant {gid} for {operation} on {object} from install grant {from}, given by {}; it expires with the activity.", capitalized(&recipient), self.member_name(principal)),
            ),
            _ => (
                Source::principal(principal),
                format!("{} gave {recipient} {kind_word} grant ({gid}) for {operation} on {object}{until}.", self.member_name(principal)),
            ),
        };
        let mut entry = Entry::new("grant", source, plain)
            .app(app)
            .member(principal)
            .holder(holder.label())
            .grant(&gid)
            .operation(operation)
            .object(object)
            .detail(json!({
                "kind": kind,
                "bound_hash": bound_hash,
                "expires_at": expires_at.map(stamp),
                "derived_from": derived_from,
            }));
        if let Holder::Activity(aid) = &holder {
            entry = entry.activity(aid);
        }
        self.core.issue_grant(grant, entry);
        gid
    }
}

/// Fills in the app on a refusal raised by a shared check.
pub fn with_app(e: CallError, app: &str) -> CallError {
    match e {
        CallError::Refused(r) => (*r).app(app).into(),
        other => other,
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
