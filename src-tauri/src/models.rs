use serde::{Deserialize, Serialize};

/// A configured mail account. Secrets (passwords / refresh tokens) are NOT stored here.
#[derive(Serialize, Clone, Debug)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub email: String,
    /// "m365" | "gmail" | "imap"
    pub kind: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    /// "ssl" (implicit TLS) | "starttls" | "none"
    pub smtp_security: String,
    pub username: String,
    pub tenant: String,
    pub client_id: String,
    #[serde(skip_serializing)]
    pub client_secret: String,
    pub save_sent: bool,
}

impl Account {
    pub fn is_oauth(&self) -> bool {
        self.kind == "m365" || self.kind == "gmail"
    }
}

/// Data sent from the "Add account" dialog.
#[derive(Deserialize, Debug, Default)]
#[serde(default)]
pub struct NewAccount {
    pub name: String,
    pub email: String,
    pub kind: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: String,
    pub username: String,
    pub password: String,
    pub tenant: String,
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Folder {
    pub account_id: i64,
    /// Raw IMAP name (modified UTF-7) - used for all server commands.
    pub name: String,
    /// Human readable name.
    pub display: String,
    pub special: String,
    pub noselect: bool,
    pub depth: u32,
    pub unread: u32,
}

#[derive(Serialize, Clone, Debug)]
pub struct MsgSummary {
    pub account_id: i64,
    pub folder: String,
    pub uid: u32,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to: String,
    pub date: i64,
    pub size: u32,
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub has_attach: bool,
    /// First ~160 characters of the body, once the body has been downloaded.
    pub preview: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct AttachmentInfo {
    pub index: usize,
    pub name: String,
    pub mime: String,
    pub size: usize,
    pub inline: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct MessageView {
    pub summary: MsgSummary,
    pub to: String,
    pub cc: String,
    pub reply_to: String,
    pub message_id: String,
    pub references: String,
    pub html: String,
    pub text: String,
    pub has_remote: bool,
    pub attachments: Vec<AttachmentInfo>,
    /// Sender asked for a read receipt (Disposition-Notification-To), if any.
    pub mdn_to: String,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct MsgRef {
    pub folder: String,
    pub uid: u32,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct Draft {
    pub account_id: i64,
    /// Sending identity (an alias of the account); empty = the account's own name/address.
    #[serde(default)]
    pub from_name: String,
    #[serde(default)]
    pub from_email: String,
    #[serde(default)]
    pub reply_to: String,
    #[serde(default)]
    pub organization: String,
    /// Ask the recipient's mail app to send a read receipt.
    #[serde(default)]
    pub request_receipt: bool,
    /// Attach the sender's contact card (.vcf).
    #[serde(default)]
    pub vcard: bool,
    /// HTML signature: `sig_text` is its plain-text form as it appears in `body`; in the HTML
    /// part that text is replaced by `sig_html`.
    #[serde(default)]
    pub sig_text: String,
    #[serde(default)]
    pub sig_html: String,
    pub to: String,
    #[serde(default)]
    pub cc: String,
    #[serde(default)]
    pub bcc: String,
    pub subject: String,
    pub body: String,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Option<String>,
    /// Local file paths to attach.
    #[serde(default)]
    pub attachments: Vec<String>,
    /// When forwarding: include the original message's attachments.
    #[serde(default)]
    pub forward_from: Option<MsgRef>,
    /// When replying: mark the original as answered after sending.
    #[serde(default)]
    pub reply_to_msg: Option<MsgRef>,
    /// Server copy of this draft (in the Drafts folder) - replaced on save, removed on send.
    #[serde(default)]
    pub draft: Option<MsgRef>,
    /// Local recovery copy of this draft.
    #[serde(default)]
    pub local_draft_id: Option<i64>,
    /// Stable Message-ID used while editing a draft.
    #[serde(default)]
    pub message_id: Option<String>,
    /// Upload large local attachments to Nextcloud and send share links instead.
    #[serde(default)]
    pub link_large: bool,
}

#[derive(Serialize, Debug)]
pub struct DraftSaved {
    pub local_draft_id: i64,
    pub draft: Option<MsgRefOut>,
    pub message_id: String,
    pub server_error: Option<String>,
}

#[derive(Serialize, Debug, Clone)]
pub struct MsgRefOut {
    pub folder: String,
    pub uid: u32,
}

/// A draft re-opened for editing.
#[derive(Serialize, Debug)]
pub struct DraftView {
    pub account_id: i64,
    pub from_email: String,
    pub receipt: bool,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    pub in_reply_to: String,
    pub references: String,
    pub message_id: String,
    pub attachments: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct Contact {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub phone: String,
    pub company: String,
    pub notes: String,
    pub favorite: bool,
}

#[derive(Serialize, Debug, Clone)]
pub struct AddressSuggestion {
    pub name: String,
    pub email: String,
    pub is_contact: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct Todo {
    pub id: i64,
    pub title: String,
    pub notes: String,
    /// Unix seconds; None = no due date.
    pub due: Option<i64>,
    pub done: bool,
    /// 0 normal, 1 high
    pub priority: i64,
    /// Optional link to an email.
    pub account_id: Option<i64>,
    pub folder: Option<String>,
    pub uid: Option<u32>,
    pub created: i64,
}

#[derive(Serialize, Debug)]
pub struct GrammarIssue {
    pub offset: usize,
    pub length: usize,
    pub message: String,
    pub replacements: Vec<String>,
    pub category: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Progress {
    pub phase: String,
    pub folder: String,
    pub done: u64,
    pub total: u64,
}

/// Per-account options (Thunderbird's "Account Settings"), stored in the app settings under
/// account_prefs.<id>. Only what the backend needs is read here; the UI keeps the rest.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct AccountPrefs {
    pub bcc_self: bool,
    /// Messages dated within this many days are always synced (on top of the newest `sync_max`).
    pub sync_days: u32,
    pub sync_max: u32,
    /// Download message bodies in the background for offline reading.
    pub offline_bodies: bool,
}

impl Default for AccountPrefs {
    fn default() -> Self {
        Self { bcc_self: false, sync_days: 14, sync_max: 500, offline_bodies: true }
    }
}
