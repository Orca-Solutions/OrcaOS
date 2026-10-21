// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Canonical payloads and the hashes that bind an authorization to one.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The message the mail adapter sends, in the form the approval path shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub from: String,
    pub to: String,
    pub subject: String,
    pub body: String,
    pub attachment: Attachment,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub content: String,
}

impl Message {
    /// One line per field, in a fixed order, with lengths so that no field can
    /// absorb its neighbour. Two messages hash alike only if every field does.
    pub fn canonical(&self) -> String {
        let fields = [
            ("from", &self.from),
            ("to", &self.to),
            ("subject", &self.subject),
            ("body", &self.body),
            ("attachment.name", &self.attachment.name),
            ("attachment.content", &self.attachment.content),
        ];

        fields
            .iter()
            .map(|(name, value)| format!("{name}:{}:{value}\n", value.len()))
            .collect()
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// The hash the approval path shows and the approval records.
pub fn payload_hash(message: &Message) -> String {
    sha256_hex(message.canonical().as_bytes())
}

/// The value an authorization is bound to: the canonical payload's hash together
/// with the operation, the adapter, the activity and the expiry.
pub fn authorization_binding(
    payload_hash: &str,
    operation: &str,
    adapter: &str,
    activity: &str,
    expires_at_ms: u64,
) -> String {
    let canonical = format!(
        "payload:{payload_hash}\noperation:{operation}\nadapter:{adapter}\nactivity:{activity}\nexpires:{expires_at_ms}\n"
    );

    sha256_hex(canonical.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> Message {
        Message {
            from: "principal-1@mock.test".into(),
            to: "accountant@mock.test".into(),
            subject: "Q3 contracts".into(),
            body: "Summary attached.".into(),
            attachment: Attachment {
                name: "summary.txt".into(),
                content: "one page".into(),
            },
        }
    }

    #[test]
    fn any_changed_field_changes_the_hash() {
        let original = payload_hash(&message());

        let mut recipient = message();
        recipient.to = "other@mock.test".into();
        assert_ne!(payload_hash(&recipient), original);

        let mut attachment = message();
        attachment.attachment.content = "swapped".into();
        assert_ne!(payload_hash(&attachment), original);
    }

    #[test]
    fn fields_cannot_absorb_their_neighbours() {
        let mut a = message();
        a.subject = "ab".into();
        a.body = "c".into();

        let mut b = message();
        b.subject = "a".into();
        b.body = "bc".into();

        assert_ne!(payload_hash(&a), payload_hash(&b));
    }

    #[test]
    fn the_binding_covers_operation_adapter_activity_and_expiry() {
        let hash = payload_hash(&message());
        let base = authorization_binding(&hash, "send", "mail-adapter", "A1", 10);

        assert_ne!(base, authorization_binding(&hash, "promote", "mail-adapter", "A1", 10));
        assert_ne!(base, authorization_binding(&hash, "send", "other-adapter", "A1", 10));
        assert_ne!(base, authorization_binding(&hash, "send", "mail-adapter", "A2", 10));
        assert_ne!(base, authorization_binding(&hash, "send", "mail-adapter", "A1", 11));
    }
}
