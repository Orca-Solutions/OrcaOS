// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scenario A's task logic: the deterministic processor and the drafter.
//!
//! Every platform build runs this same code inside its own participant
//! wrappers, so task code is not a variable between platforms. It holds no
//! authority: the wrappers read and write through the platform's enforced
//! stores and pass text in and out.

use orca_ref::{Attachment, Message};

/// The synthetic inputs, fabricated for the experiment. No real data.
pub mod inputs {
    pub const C1_V1: &str = include_str!("../../synthetic-inputs/c1@v1.txt");
    pub const C1_V2: &str = include_str!("../../synthetic-inputs/c1@v2.txt");
    pub const C2_V1: &str = include_str!("../../synthetic-inputs/c2@v1.txt");
    pub const C3_V1: &str = include_str!("../../synthetic-inputs/c3@v1.txt");
    pub const TAX_RETURN_2025: &str = include_str!("../../synthetic-inputs/tax-return-2025.txt");

    /// Every object in the document store, by the identifier the protocol uses.
    pub const ALL: [(&str, &str); 5] = [
        ("c1@v1", C1_V1),
        ("c1@v2", C1_V2),
        ("c2@v1", C2_V1),
        ("c3@v1", C3_V1),
        ("tax-return-2025", TAX_RETURN_2025),
    ];
}

pub const SENDER: &str = "principal-1@mock.test";
pub const RECIPIENT: &str = "accountant@mock.test";

fn field<'a>(text: &'a str, name: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
        .map(str::trim)
        .unwrap_or("not stated")
}

/// One contract's digest: the fields an accountant needs, in a fixed order.
pub fn digest(snapshot: &str, text: &str) -> String {
    format!(
        "{snapshot}\n  Parties: {}\n  Term: {}\n  Value: {}\n  Renewal: {}\n  Payment: {}\n",
        field(text, "Parties"),
        field(text, "Term"),
        field(text, "Value"),
        field(text, "Renewal"),
        field(text, "Payment"),
    )
}

/// The alternative processor for the replacement test: the same contract
/// (one digest per snapshot, the same fields), a different implementation.
pub fn digest_alt(snapshot: &str, text: &str) -> String {
    let wanted = ["Parties", "Term", "Value", "Renewal", "Payment"];
    let mut out = format!("{snapshot}\n");

    for name in wanted {
        out.push_str(&format!("  {name}: {}\n", field(text, name)));
    }
    out
}

/// The one-page summary the drafter attaches.
pub fn summary(digests: &[String]) -> String {
    let mut sorted = digests.to_vec();
    sorted.sort();

    format!(
        "Q3 contracts summary (synthetic test data)\n\n{}\n{} contracts covered.\n",
        sorted.join("\n"),
        sorted.len()
    )
}

/// The message the drafter proposes. It is sent only after approval.
pub fn compose(summary: &str) -> Message {
    Message {
        from: SENDER.into(),
        to: RECIPIENT.into(),
        subject: "Q3 contracts summary".into(),
        body: "Hello,\n\nThe summary of the Q3 contracts is attached.\n\nprincipal-1\n".into(),
        attachment: Attachment {
            name: "q3-contracts-summary.txt".into(),
            content: summary.into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_processors_produce_the_same_digest() {
        for (id, text) in inputs::ALL {
            assert_eq!(digest(id, text), digest_alt(id, text));
        }
    }

    #[test]
    fn the_summary_does_not_depend_on_digest_order() {
        let a = digest("c1@v1", inputs::C1_V1);
        let b = digest("c3@v1", inputs::C3_V1);

        assert_eq!(summary(&[a.clone(), b.clone()]), summary(&[b, a]));
    }
}
