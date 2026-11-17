//! Tauri commands exposed to the web UI. Each one runs its blocking work off the UI thread.

use crate::mail::{connect, Auth, Shared};
use crate::models::*;
use crate::{mime, oauth, secrets};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| format!("Internal error: {e}"))?
}

fn open_in_browser(app: &AppHandle) -> impl Fn(&str) -> Result<(), String> + '_ {
    move |url: &str| app.opener().open_url(url, None::<&str>).map_err(|e| format!("Cannot open browser: {e}"))
}

// ---------------------------------------------------------------- accounts

#[tauri::command]
pub async fn list_accounts(state: State<'_, Shared>) -> Result<Vec<Account>, String> {
    state.db.accounts()
}

fn account_from_input(n: &NewAccount) -> Result<Account, String> {
    let email = n.email.trim().to_string();
    if !email.contains('@') {
        return Err("Enter a valid email address".into());
    }
    let name = if n.name.trim().is_empty() { email.split('@').next().unwrap_or("").to_string() } else { n.name.trim().to_string() };
    let domain = email.split('@').nth(1).unwrap_or("").to_string();
    let mut a = Account {
        id: 0,
        name,
        email: email.clone(),
        kind: n.kind.clone(),
        imap_host: String::new(),
        imap_port: 993,
        smtp_host: String::new(),
        smtp_port: 587,
        smtp_security: "starttls".into(),
        username: email.clone(),
        tenant: String::new(),
        client_id: n.client_id.trim().to_string(),
        client_secret: n.client_secret.trim().to_string(),
        save_sent: false,
    };
    match n.kind.as_str() {
        "m365" => {
            a.imap_host = "outlook.office365.com".into();
            a.smtp_host = "smtp.office365.com".into();
            a.tenant = if n.tenant.trim().is_empty() { domain } else { n.tenant.trim().to_string() };
        }
        "gmail" => {
            a.imap_host = "imap.gmail.com".into();
            a.smtp_host = "smtp.gmail.com".into();
            a.smtp_port = 465;
            a.smtp_security = "ssl".into();
            if a.client_secret.is_empty() {
                return Err("Gmail needs the OAuth Client secret from your Google Cloud desktop client".into());
            }
        }
        "imap" => {
            if n.imap_host.trim().is_empty() || n.smtp_host.trim().is_empty() {
                return Err("IMAP and SMTP server are required".into());
            }
            if n.password.is_empty() {
                return Err("Password is required".into());
            }
            a.imap_host = n.imap_host.trim().into();
            a.imap_port = if n.imap_port == 0 { 993 } else { n.imap_port };
            a.smtp_host = n.smtp_host.trim().into();
            a.smtp_port = if n.smtp_port == 0 { 465 } else { n.smtp_port };
            a.smtp_security = match n.smtp_security.as_str() {
                "ssl" | "starttls" | "none" => n.smtp_security.clone(),
                _ => "ssl".into(),
            };
            if !n.username.trim().is_empty() {
                a.username = n.username.trim().into();
            }
            a.save_sent = true;
        }
        other => return Err(format!("Unknown account type: {other}")),
    }
    Ok(a)
}

#[tauri::command]
pub async fn add_account(app: AppHandle, state: State<'_, Shared>, input: NewAccount) -> Result<Account, String> {
    let st = state.inner().clone();
    blocking(move || {
        let mut acct = account_from_input(&input)?;
        if acct.is_oauth() {
            let p = oauth::provider(&acct.kind, &acct.tenant, &acct.client_id, &acct.client_secret)?;
            let tokens = oauth::authorize(&p, &acct.email, open_in_browser(&app))?;
            if tokens.refresh_token.is_none() {
                return Err("The provider did not return a refresh token (offline access). Check the app's API permissions.".into());
            }
            acct.id = st.db.insert_account(&acct)?;
            st.store_tokens(acct.id, &tokens)?;
        } else {
            // Verify credentials before saving anything.
            connect(&acct, &Auth::Password(input.password.clone()))?.logout();
            acct.id = st.db.insert_account(&acct)?;
            secrets::set(&st.db, &secrets::password_key(acct.id), &input.password)?;
        }
        if let Err(e) = st.refresh_folders(&acct) {
            st.forget_account(acct.id);
            secrets::delete(&st.db, &secrets::password_key(acct.id));
            secrets::delete(&st.db, &secrets::refresh_key(acct.id));
            let _ = st.db.delete_account(acct.id);
            return Err(e);
        }
        Ok(acct)
    })
    .await
}

#[tauri::command]
pub async fn reconnect_account(app: AppHandle, state: State<'_, Shared>, account_id: i64) -> Result<(), String> {
    let st = state.inner().clone();
    blocking(move || {
        let acct = st.db.account(account_id)?;
        if !acct.is_oauth() {
            return Err("Reconnect is only needed for Microsoft 365 / Gmail accounts".into());
        }
        let p = oauth::provider(&acct.kind, &acct.tenant, &acct.client_id, &acct.client_secret)?;
        let tokens = oauth::authorize(&p, &acct.email, open_in_browser(&app))?;
        st.forget_account(account_id);
        st.store_tokens(account_id, &tokens)?;
        st.refresh_folders(&acct).map(|_| ())
    })
    .await
}

#[tauri::command]
pub async fn remove_account(state: State<'_, Shared>, account_id: i64) -> Result<(), String> {
    let st = state.inner().clone();
    blocking(move || {
        st.forget_account(account_id);
        secrets::delete(&st.db, &secrets::password_key(account_id));
        secrets::delete(&st.db, &secrets::refresh_key(account_id));
        st.db.delete_account(account_id)
    })
    .await
}

// ---------------------------------------------------------------- folders & messages

#[tauri::command]
pub async fn list_folders(state: State<'_, Shared>, account_id: i64) -> Result<Vec<Folder>, String> {
    state.db.folders(account_id)
}

#[tauri::command]
pub async fn refresh_folders(state: State<'_, Shared>, account_id: i64) -> Result<Vec<Folder>, String> {
    let st = state.inner().clone();
    blocking(move || {
        let acct = st.db.account(account_id)?;
        st.refresh_folders(&acct)
    })
    .await
}

#[tauri::command]
pub async fn list_messages(state: State<'_, Shared>, account_id: i64, folder: String, limit: u32) -> Result<Vec<MsgSummary>, String> {
    state.db.list_messages(account_id, &folder, limit)
}

#[tauri::command]
pub async fn sync_folder(state: State<'_, Shared>, account_id: i64, folder: String, limit: u32) -> Result<Vec<MsgSummary>, String> {
    let st = state.inner().clone();
    blocking(move || {
        let acct = st.db.account(account_id)?;
        st.sync_folder(&acct, &folder, limit.max(50))?;
        st.db.list_messages(account_id, &folder, limit.max(50))
    })
    .await
}

#[tauri::command]
pub async fn prefetch_bodies(state: State<'_, Shared>, account_id: i64, folder: String) -> Result<u32, String> {
    let st = state.inner().clone();
    blocking(move || {
        let acct = st.db.account(account_id)?;
        st.prefetch_bodies(&acct, &folder, 25)
    })
    .await
}

#[tauri::command]
pub async fn open_message(state: State<'_, Shared>, account_id: i64, folder: String, uid: u32) -> Result<MessageView, String> {
    let st = state.inner().clone();
    blocking(move || {
        let acct = st.db.account(account_id)?;
        st.open_message(&acct, &folder, uid)
    })
    .await
}

#[tauri::command]
pub async fn set_flag(state: State<'_, Shared>, account_id: i64, folder: String, uids: Vec<u32>, flag: String, value: bool) -> Result<(), String> {
    let st = state.inner().clone();
    blocking(move || {
        let acct = st.db.account(account_id)?;
        st.set_flag(&acct, &folder, &uids, &flag, value)
    })
    .await
}

#[tauri::command]
pub async fn delete_messages(state: State<'_, Shared>, account_id: i64, folder: String, uids: Vec<u32>) -> Result<(), String> {
    let st = state.inner().clone();
    blocking(move || {
        let acct = st.db.account(account_id)?;
        st.delete(&acct, &folder, &uids)
    })
    .await
}

#[tauri::command]
pub async fn move_messages(state: State<'_, Shared>, account_id: i64, folder: String, uids: Vec<u32>, target: String) -> Result<(), String> {
    let st = state.inner().clone();
    blocking(move || {
        let acct = st.db.account(account_id)?;
        st.move_to(&acct, &folder, &uids, &target)
    })
    .await
}

#[tauri::command]
pub async fn search(state: State<'_, Shared>, account_id: Option<i64>, query: String) -> Result<Vec<MsgSummary>, String> {
    if query.trim().len() < 2 {
        return Ok(vec![]);
    }
    state.db.search(account_id, query.trim(), 300)
}

#[tauri::command]
pub async fn send_message(state: State<'_, Shared>, draft: Draft) -> Result<(), String> {
    let st = state.inner().clone();
    blocking(move || {
        let acct = st.db.account(draft.account_id)?;
        st.send(&acct, &draft)
    })
    .await
}

// ---------------------------------------------------------------- files & links

#[tauri::command]
pub async fn pick_files(app: AppHandle) -> Result<Vec<String>, String> {
    blocking(move || {
        let files = app.dialog().file().set_title("Attach files").blocking_pick_files().unwrap_or_default();
        Ok(files.into_iter().filter_map(|f| f.into_path().ok()).map(|p| p.to_string_lossy().into_owned()).collect())
    })
    .await
}

fn safe_file_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() { "attachment".into() } else { cleaned }
}

#[tauri::command]
pub async fn save_attachment(app: AppHandle, state: State<'_, Shared>, account_id: i64, folder: String, uid: u32, index: usize) -> Result<bool, String> {
    let st = state.inner().clone();
    blocking(move || {
        let acct = st.db.account(account_id)?;
        let raw = st.raw_message(&acct, &folder, uid)?;
        let (name, _, bytes) = mime::attachment(&raw, index)?;
        let Some(path) = app.dialog().file().set_file_name(safe_file_name(&name)).blocking_save_file() else {
            return Ok(false);
        };
        let path = path.into_path().map_err(|e| e.to_string())?;
        std::fs::write(&path, bytes).map_err(|e| format!("Cannot save file: {e}"))?;
        Ok(true)
    })
    .await
}

#[tauri::command]
pub async fn open_attachment(app: AppHandle, state: State<'_, Shared>, account_id: i64, folder: String, uid: u32, index: usize) -> Result<(), String> {
    let st = state.inner().clone();
    blocking(move || {
        let acct = st.db.account(account_id)?;
        let raw = st.raw_message(&acct, &folder, uid)?;
        let (name, _, bytes) = mime::attachment(&raw, index)?;
        let dir = std::env::temp_dir().join("kitemail").join(format!("{account_id}-{uid}"));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(safe_file_name(&name));
        std::fs::write(&path, bytes).map_err(|e| format!("Cannot write temp file: {e}"))?;
        app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| format!("Cannot open file: {e}"))
    })
    .await
}

#[tauri::command]
pub async fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    let allowed = ["http://", "https://", "tel:", "callto:", "sip:"];
    if !allowed.iter().any(|p| lower.starts_with(p)) {
        return Err("Only web and phone links can be opened".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- drafts

#[tauri::command]
pub async fn save_draft(state: State<'_, Shared>, draft: Draft) -> Result<DraftSaved, String> {
    let st = state.inner().clone();
    blocking(move || {
        let acct = st.db.account(draft.account_id)?;
        st.save_draft(&acct, &draft)
    })
    .await
}

#[tauri::command]
pub async fn discard_draft(state: State<'_, Shared>, account_id: i64, draft: Option<MsgRef>, local_draft_id: Option<i64>) -> Result<(), String> {
    let st = state.inner().clone();
    blocking(move || {
        let acct = st.db.account(account_id)?;
        st.discard_draft(&acct, draft.as_ref(), local_draft_id)
    })
    .await
}

#[tauri::command]
pub async fn open_draft(state: State<'_, Shared>, account_id: i64, folder: String, uid: u32) -> Result<DraftView, String> {
    let st = state.inner().clone();
    blocking(move || {
        let acct = st.db.account(account_id)?;
        st.open_draft(&acct, &folder, uid)
    })
    .await
}

/// Drafts that were saved locally but never reached the server (e.g. app closed while offline).
#[tauri::command]
pub async fn pending_drafts(state: State<'_, Shared>) -> Result<Vec<serde_json::Value>, String> {
    Ok(state
        .db
        .pending_drafts()?
        .into_iter()
        .filter_map(|(id, json)| {
            let mut v: serde_json::Value = serde_json::from_str(&json).ok()?;
            v["local_draft_id"] = id.into();
            Some(v)
        })
        .collect())
}

// ---------------------------------------------------------------- archive

#[tauri::command]
pub async fn archive_messages(state: State<'_, Shared>, account_id: i64, folder: String, uids: Vec<u32>) -> Result<String, String> {
    let st = state.inner().clone();
    blocking(move || {
        let acct = st.db.account(account_id)?;
        st.archive(&acct, &folder, &uids)
    })
    .await
}

// ---------------------------------------------------------------- contacts

#[tauri::command]
pub async fn list_contacts(state: State<'_, Shared>, query: String) -> Result<Vec<Contact>, String> {
    state.db.contacts(&query)
}

#[tauri::command]
pub async fn save_contact(state: State<'_, Shared>, contact: Contact) -> Result<Contact, String> {
    if contact.name.trim().is_empty() && contact.email.trim().is_empty() && contact.phone.trim().is_empty() {
        return Err("Enter a name, email or phone number".into());
    }
    let mut c = contact;
    c.id = state.db.save_contact(&c)?;
    Ok(c)
}

#[tauri::command]
pub async fn delete_contact(state: State<'_, Shared>, id: i64) -> Result<(), String> {
    state.db.delete_contact(id)
}

#[tauri::command]
pub async fn suggest_addresses(state: State<'_, Shared>, query: String) -> Result<Vec<AddressSuggestion>, String> {
    state.db.suggest(&query, 8)
}

// ---------------------------------------------------------------- todos

#[tauri::command]
pub async fn list_todos(state: State<'_, Shared>) -> Result<Vec<Todo>, String> {
    state.db.todos()
}

#[tauri::command]
pub async fn save_todo(state: State<'_, Shared>, todo: Todo) -> Result<Todo, String> {
    if todo.title.trim().is_empty() {
        return Err("Task title is required".into());
    }
    let mut t = todo;
    t.id = state.db.save_todo(&t)?;
    Ok(t)
}

#[tauri::command]
pub async fn delete_todo(state: State<'_, Shared>, id: i64) -> Result<(), String> {
    state.db.delete_todo(id)
}

// ---------------------------------------------------------------- settings, grammar, calls

#[tauri::command]
pub async fn get_settings(state: State<'_, Shared>) -> Result<serde_json::Value, String> {
    Ok(state.db.get_setting("app").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| serde_json::json!({})))
}

#[tauri::command]
pub async fn save_settings(state: State<'_, Shared>, settings: serde_json::Value) -> Result<(), String> {
    state.db.set_setting("app", &settings.to_string())
}

#[tauri::command]
pub async fn check_grammar(state: State<'_, Shared>, text: String) -> Result<Vec<GrammarIssue>, String> {
    let st = state.inner().clone();
    blocking(move || {
        let url = crate::push::setting_str(&st, "lt_url", "https://api.languagetool.org");
        let lang = crate::push::setting_str(&st, "lt_lang", "auto");
        crate::grammar::check(&url, &lang, &text)
    })
    .await
}

/// Dials a number through the configured handler: tel: (3CX / Teams / Phone Link app)
/// or a custom URL template such as https://pbx.example.com/webclient/#/call?phone={number}
#[tauri::command]
pub async fn call_number(app: AppHandle, state: State<'_, Shared>, number: String) -> Result<(), String> {
    let clean: String = number.chars().filter(|c| c.is_ascii_digit() || *c == '+' || *c == '*' || *c == '#').collect();
    if clean.len() < 3 {
        return Err("Not a valid phone number".into());
    }
    let template = crate::push::setting_str(state.inner(), "call_template", "tel:{number}");
    let url = template.replace("{number}", &clean);
    app.opener().open_url(url, None::<&str>).map_err(|e| format!("No app is set up to handle calls: {e}"))
}

// ---------------------------------------------------------------- backup

#[tauri::command]
pub async fn backup_account(app: AppHandle, state: State<'_, Shared>, account_id: i64) -> Result<Option<serde_json::Value>, String> {
    let st = state.inner().clone();
    blocking(move || {
        use tauri::Emitter;
        let acct = st.db.account(account_id)?;
        let name = format!("kitemail-backup-{}-{}.zip", acct.email.replace('@', "_at_"), chrono::Local::now().format("%Y-%m-%d"));
        let Some(path) = app.dialog().file().add_filter("Zip archive", &["zip"]).set_file_name(&name).blocking_save_file() else {
            return Ok(None);
        };
        let path = path.into_path().map_err(|e| e.to_string())?;
        let emitter = app.clone();
        let count = st.backup(&acct, &path, &|p| {
            let _ = emitter.emit("backup-progress", &p);
        })?;
        Ok(Some(serde_json::json!({ "count": count, "path": path.to_string_lossy() })))
    })
    .await
}

#[tauri::command]
pub async fn restore_account(app: AppHandle, state: State<'_, Shared>, account_id: i64) -> Result<Option<serde_json::Value>, String> {
    let st = state.inner().clone();
    blocking(move || {
        use tauri::Emitter;
        let acct = st.db.account(account_id)?;
        let Some(path) = app.dialog().file().add_filter("Kite Mail backup", &["zip"]).blocking_pick_file() else {
            return Ok(None);
        };
        let path = path.into_path().map_err(|e| e.to_string())?;
        let emitter = app.clone();
        let (added, skipped) = st.restore(&acct, &path, &|p| {
            let _ = emitter.emit("backup-progress", &p);
        })?;
        Ok(Some(serde_json::json!({ "added": added, "skipped": skipped })))
    })
    .await
}
