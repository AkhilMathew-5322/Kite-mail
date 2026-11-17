//! Kite Mail - lightweight desktop & mobile email client.

mod commands;
mod db;
mod grammar;
#[cfg(test)]
mod it_tests;
mod mail;
mod mime;
mod models;
mod oauth;
mod push;
mod secrets;
mod smtp;

use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let dir = if cfg!(mobile) {
                app.path().app_data_dir().unwrap_or_else(|_| db::data_dir())
            } else {
                db::data_dir()
            };
            let _ = std::fs::create_dir_all(&dir);
            let db = db::Db::open(dir.join("kitemail.db")).map_err(|e| e.to_string())?;
            let state: mail::Shared = Arc::new(mail::AppState::new(db));
            app.manage(state.clone());
            push::start(app.handle().clone(), state);
            #[cfg(target_os = "linux")]
            enable_linux_spellcheck(app);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_accounts,
            commands::add_account,
            commands::reconnect_account,
            commands::remove_account,
            commands::list_folders,
            commands::refresh_folders,
            commands::list_messages,
            commands::sync_folder,
            commands::prefetch_bodies,
            commands::open_message,
            commands::set_flag,
            commands::delete_messages,
            commands::move_messages,
            commands::archive_messages,
            commands::search,
            commands::send_message,
            commands::save_draft,
            commands::discard_draft,
            commands::open_draft,
            commands::pending_drafts,
            commands::pick_files,
            commands::save_attachment,
            commands::open_attachment,
            commands::open_url,
            commands::list_contacts,
            commands::save_contact,
            commands::delete_contact,
            commands::suggest_addresses,
            commands::list_todos,
            commands::save_todo,
            commands::delete_todo,
            commands::get_settings,
            commands::save_settings,
            commands::check_grammar,
            commands::call_number,
            commands::backup_account,
            commands::restore_account,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kite Mail");
}

/// WebKitGTK ships with spell checking off; turn it on for the system language.
/// (WebView2 on Windows and Android's WebView spell-check text fields natively.)
#[cfg(target_os = "linux")]
fn enable_linux_spellcheck(app: &tauri::App) {
    use webkit2gtk::{WebContextExt, WebViewExt};
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.with_webview(|wv| {
            if let Some(ctx) = wv.inner().context() {
                ctx.set_spell_checking_enabled(true);
                let lang = std::env::var("LANG").ok().and_then(|l| l.split('.').next().map(String::from)).filter(|l| l.len() >= 2 && l != "C");
                let langs = [lang.unwrap_or_else(|| "en_US".into())];
                ctx.set_spell_checking_languages(&langs.iter().map(String::as_str).collect::<Vec<_>>());
            }
        });
    }
}
