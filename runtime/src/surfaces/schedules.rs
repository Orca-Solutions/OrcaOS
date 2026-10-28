// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Surface 9, scheduled runs (0.3). Each tick opens an activity that acts
//! for the member who switched the app on and draws its grants from the
//! app's install grants. The ticks themselves are in `schedule.rs` and the
//! clock in `runtime.rs`.

use chrono::Duration;
use chrono_tz::Tz;
use serde_json::{Value, json};

use crate::runtime::*;
use crate::ledger::Entry;
use crate::model::*;
use crate::schedule::{self, Cadence};

impl Runtime {
    pub fn schedule(&mut self, who: &Who, body: &Value) -> Res {
        let app_id = match &who.app {
            Some(a) => a.clone(),
            None => s(body, "app")?,
        };
        let routine = s(body, "routine")?;
        let app = self.installed(&app_id)?.clone();
        let name = app.manifest.display_name();
        let r = |code, text: String| refuse(code, text).app(&app_id);

        let decl = app.manifest.routine(&routine).filter(|d| d.trigger == Trigger::Schedule).cloned();
        let Some(decl) = decl else {
            return Err(r("routine_not_scheduled", format!("{name} declares no scheduled routine called {routine}")).into());
        };

        let at = s(body, "at")?;
        let cadence_text = s(body, "cadence")?;
        let zone = opt_s(body, "zone").unwrap_or_else(|| self.household.zone.clone());
        let Some(cadence) = Cadence::parse(&cadence_text) else {
            return Err(bad("`cadence` is \"daily\" or \"weekly:<mon|tue|wed|thu|fri|sat|sun>\""));
        };
        if schedule::parse_time(&at).is_none() {
            return Err(bad("`at` is a local wall-clock time, HH:MM"));
        }
        if zone.parse::<Tz>().is_err() {
            return Err(bad(format!("{zone} is not a time zone")));
        }

        // Default half the cadence; the manifest may shorten it, never lengthen it past the cadence.
        let manifest_window = decl.catch_up_hours.unwrap_or_else(|| cadence.default_catch_up_hours());
        if manifest_window > cadence.hours() {
            return Err(r("catch_up_too_long", format!("the manifest's catch-up window of {manifest_window} hours is longer than the cadence")).into());
        }
        let window = body.get("catch_up_hours").and_then(Value::as_i64).unwrap_or(manifest_window);
        if window > manifest_window {
            return Err(r("catch_up_too_long", format!("the catch-up window may not exceed the manifest's {manifest_window} hours")).into());
        }

        let set_by = who.member.clone().or(app.switched_on_by.clone()).unwrap_or_default();
        let sid = self.next_id("S");
        let sched = Schedule {
            id: sid.clone(),
            app: app_id.clone(),
            routine: routine.clone(),
            at: at.clone(),
            zone: zone.clone(),
            cadence: cadence_text.clone(),
            catch_up_hours: window,
            set_by: set_by.clone(),
        };
        let next = schedule::ticks_between(&sched, self.now(), self.now() + Duration::days(8)).first().map(|t| stamp(t.at));
        self.schedules.insert(sid.clone(), sched);

        let plain = format!("{name}'s {routine} runs {cadence_text} at {at} {zone}, with a {window}-hour catch-up window.");
        self.journal(
            Entry::new("schedule", who.source(), plain)
                .app(&app_id)
                .member(&set_by)
                .detail(json!({ "schedule": sid, "routine": routine, "at": at, "zone": zone, "cadence": cadence_text, "catch_up_hours": window, "next": next })),
        );
        Ok(json!({ "schedule": sid, "next": next, "catch_up_hours": window }))
    }

    pub fn list_schedules(&mut self, who: &Who, body: &Value) -> Res {
        let app = opt_s(body, "app").or_else(|| who.app.clone());
        let items: Vec<&Schedule> = self.schedules.values().filter(|x| app.as_ref().is_none_or(|a| *a == x.app)).collect();
        Ok(json!({ "schedules": items }))
    }

    pub fn unschedule(&mut self, who: &Who, body: &Value) -> Res {
        let sid = s(body, "schedule")?;
        let Some(sched) = self.schedules.get(&sid).cloned() else {
            return Err(bad(format!("there is no schedule {sid}")));
        };
        if !who.is_shell() && who.app.as_deref() != Some(sched.app.as_str()) {
            return Err(refuse("outside_audience", format!("schedule {sid} belongs to another app")).into());
        }
        self.schedules.remove(&sid);

        let plain = format!("{}'s {} schedule {sid} was removed.", self.app_name(&sched.app), sched.routine);
        self.journal(Entry::new("schedule", who.source(), plain).app(&sched.app).detail(json!({ "schedule": sid, "removed": true })));
        Ok(json!({ "schedule": sid, "removed": true }))
    }
}
