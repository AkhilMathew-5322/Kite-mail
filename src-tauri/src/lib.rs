//! Kite Mail - lightweight desktop & mobile email client.

mod archive;
mod commands;
mod dav;
mod db;
mod files;
mod grammar;
#[cfg(test)]
mod it_tests;
mod mail;
mod mime;
mod net;
mod models;
mod nextcloud;
mod oauth;
mod pair;
mod s3;
mod push;
mod qr;
mod secrets;
mod smtp;

use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init());
    builder
        .setup(|app| {
            std::thread::spawn(commands::clean_temp_attachments);
            let dir = if cfg!(mobile) {
                app.path().app_data_dir().unwrap_or_else(|_| db::data_dir())
            } else {
                db::data_dir()
            };
            let _ = std::fs::create_dir_all(&dir);
            let db = db::Db::open(dir.join("kitemail.db")).map_err(|e| e.to_string())?;
            let state: mail::Shared = Arc::new(mail::AppState::new(db));
            let archive = archive::Archive::open(&dir.join("archive.db")).map_err(|e| e.to_string())?;
            app.manage(Arc::new(archive));
            app.manage(state.clone());
            push::start(app.handle().clone(), state);
            #[cfg(mobile)]
            {
                use tauri_plugin_notification::NotificationExt;
                let _ = app.notification().request_permission();
            }
            #[cfg(target_os = "linux")]
            enable_linux_webview_features(app);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_accounts,
            commands::add_account,
            commands::reconnect_account,
            commands::update_account,
            commands::cancel_sign_in,
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
            commands::search_server,
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
            commands::app_info,
            commands::nc_get_config,
            commands::nc_connect,
            commands::nc_save_options,
            commands::nc_disconnect,
            commands::nc_sync_now,
            commands::nc_task_lists,
            commands::nc_backup_now,
            commands::s3_get_config,
            commands::s3_connect,
            commands::s3_save_options,
            commands::s3_disconnect,
            commands::s3_backup_now,
            commands::mark_spam,
            commands::conversation,
            commands::files_save_attachments,
            commands::files_add_local,
            commands::files_thumbnail,
            commands::files_to_temp,
            commands::files_collect_recent,
            commands::archive_list,
            commands::archive_import,
            commands::archive_cancel,
            commands::archive_list_messages,
            commands::archive_search,
            commands::archive_open,
            commands::archive_attachment,
            commands::archive_save_eml,
            commands::archive_copy_to_mailbox,
            commands::archive_rename,
            commands::archive_delete,
            commands::archive_delete_messages,
            commands::read_receipt,
            commands::receipt_done,
            commands::read_signature_file,
            commands::pick_one_file,
            commands::file_data_url,
            commands::pair_start,
            commands::pair_import,
            commands::pair_stop,
            commands::qr_decode,
            commands::file_sizes,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kite Mail");
}

/// WebKitGTK ships with spell checking and the camera (getUserMedia) off: turn on spell
/// checking for the system language, and the camera for "Scan QR code". Only Kite Mail's own UI
/// runs scripts (email bodies are script-less sandboxed frames), so camera requests are allowed.
/// (WebView2 on Windows and Android's WebView do both natively, asking the user for the camera.)
#[cfg(target_os = "linux")]
fn enable_linux_webview_features(app: &tauri::App) {
    use webkit2gtk::{glib::prelude::Cast, PermissionRequestExt, SettingsExt, UserMediaPermissionRequest, WebContextExt, WebViewExt};
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.with_webview(|wv| {
            let view = wv.inner();
            if let Some(ctx) = view.context() {
                ctx.set_spell_checking_enabled(true);
                let lang = std::env::var("LANG").ok().and_then(|l| l.split('.').next().map(String::from)).filter(|l| l.len() >= 2 && l != "C");
                let langs = [lang.unwrap_or_else(|| "en_US".into())];
                ctx.set_spell_checking_languages(&langs.iter().map(String::as_str).collect::<Vec<_>>());
            }
            if let Some(settings) = WebViewExt::settings(&view) {
                settings.set_enable_media_stream(true);
            }
            view.connect_permission_request(|_, req| {
                if let Some(media) = req.downcast_ref::<UserMediaPermissionRequest>() {
                    media.allow();
                    return true;
                }
                false
            });
        });
    }
}
