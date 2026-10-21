// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Surface 8, members and devices (0.2). Members are principals; every grant
//! and approval traces to one.

use serde_json::{Value, json};

use crate::fake::*;
use crate::ledger::{Entry, Source};
use crate::model::*;

impl Fake {
    pub fn list_members(&mut self, who: &Who, _body: &Value) -> Res {
        if who.member.is_none() {
            return Err(refuse("not_a_member", "only a member, or an app in a member's session, lists the household").into());
        }
        let members: Vec<Value> = self
            .household
            .members
            .iter()
            .map(|m| json!({ "member": m.id, "name": m.name, "role": m.role, "may_approve": m.may_approve, "devices": m.devices }))
            .collect();
        Ok(json!({ "members": members, "guardians": self.household.guardians, "zone": self.household.zone }))
    }

    pub fn pair_device(&mut self, who: &Who, body: &Value) -> Res {
        let member = self.shell_member(who, "Pairing a device")?;
        let device = s(body, "device")?;
        self.household.member_mut(&member).unwrap().devices.insert(device.clone());

        let plain = format!("{} paired {device}; the approval path now trusts it.", self.member_name(&member));
        self.journal(Entry::new("pairing", Source::principal(&member), plain).member(&member).detail(json!({ "device": device })));
        Ok(json!({ "member": member, "device": device }))
    }

    /// Guardian relation (G-14): set, replace or remove a child's guardians. Owner only.
    pub fn set_guardians(&mut self, who: &Who, body: &Value) -> Res {
        let member = self.shell_member(who, "Setting a guardian relation")?;
        if !self.is_owner(&member) {
            return Err(refuse("not_owner", "only the owner sets guardian relations").into());
        }
        let child = s(body, "child")?;
        let guardians = strings(body, "guardians");
        if self.household.member(&child).is_none() {
            return Err(bad(format!("{child} is not a member of the household")));
        }
        if let Some(g) = guardians.iter().find(|g| self.household.member(g).is_none_or(|m| m.role == Role::Child)) {
            return Err(bad(format!("{g} is not an adult member of the household")));
        }

        let plain = if guardians.is_empty() {
            self.household.guardians.remove(&child);
            format!("{} removed {}'s guardian relation.", self.member_name(&member), self.member_name(&child))
        } else {
            self.household.guardians.insert(child.clone(), guardians.clone());
            let names: Vec<String> = guardians.iter().map(|g| self.member_name(g)).collect();
            format!("{} set {}'s guardians: {}.", self.member_name(&member), self.member_name(&child), names.join(", "))
        };
        self.journal(Entry::new("guardians", Source::principal(&member), plain).member(&member).detail(json!({ "child": child, "guardians": guardians })));
        Ok(json!({ "guardians": self.household.guardians }))
    }

    pub fn list_guardians(&mut self, who: &Who, _body: &Value) -> Res {
        if who.member.is_none() {
            return Err(refuse("not_a_member", "only a member, or an app in a member's session, reads guardian relations").into());
        }
        Ok(json!({ "guardians": self.household.guardians }))
    }
}
