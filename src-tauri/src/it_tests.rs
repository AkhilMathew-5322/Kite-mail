//! End-to-end tests against a real IMAP + SMTP server.
//! Skipped unless LM_IT=1. Expects: IMAPS on localhost:993 and SMTPS on localhost:4650,
//! user tester@localhost / secret123 (see README "Development").

use crate::db::Db;
use crate::mail::AppState;
use crate::models::*;
use crate::secrets;

const SAMPLE: &[u8] = b"From: =?UTF-8?B?SsO8cmdlbg==?= <j@example.com>\r\n\
To: tester@localhost, Carol <carol@example.com>\r\n\
Subject: =?UTF-8?Q?Caf=C3=A9_invoice?=\r\n\
Date: Mon, 16 Nov 2026 10:00:00 +0400\r\n\
Message-ID: <inv-1@example.com>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n\
--b1\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Invoice attached <img src=\"https://tracker.example.com/p.gif\"></p>\r\n\
--b1\r\nContent-Type: application/pdf; name=\"invoice.pdf\"\r\nContent-Disposition: attachment; filename=\"invoice.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQ=\r\n\
--b1--\r\n";

fn plain(subject: &str, date: &str) -> Vec<u8> {
    format!("From: Bob <bob@example.com>\r\nTo: tester@localhost\r\nSubject: {subject}\r\nDate: {date}\r\nMessage-ID: <{subject}@example.com>\r\n\r\nHello {subject}\r\n").into_bytes()
}

type Raw = imap::Session<native_tls::TlsStream<std::net::TcpStream>>;

fn raw_session() -> Raw {
    raw_session_for("tester@localhost")
}

fn raw_session_for(user: &str) -> Raw {
    let tls = native_tls::TlsConnector::new().unwrap();
    let tcp = std::net::TcpStream::connect(("localhost", 993)).unwrap();
    let mut client = imap::Client::new(tls.connect("localhost", tcp).unwrap());
    client.read_greeting().unwrap();
    client.login(user, "secret123").map_err(|(e, _)| e).unwrap()
}

fn wipe(s: &mut Raw) {
    let names: Vec<String> = s.list(Some(""), Some("*")).unwrap().iter().map(|n| n.name().to_string()).collect();
    for n in names {
        if ["INBOX", "Sent", "Trash", "Drafts"].contains(&n.as_str()) {
            let mb = s.select(&n).unwrap();
            if mb.exists > 0 {
                s.store("1:*", "+FLAGS.SILENT (\\Deleted)").unwrap();
                s.expunge().unwrap();
            }
        } else {
            let _ = s.delete(&n);
        }
    }
}

fn account(id: i64) -> Account {
    Account {
        id, name: "Test User".into(), email: "tester@localhost".into(), kind: "imap".into(),
        imap_host: "localhost".into(), imap_port: 993, smtp_host: "localhost".into(), smtp_port: 4650,
        smtp_security: "ssl".into(), username: "tester@localhost".into(), tenant: String::new(),
        client_id: String::new(), client_secret: String::new(), save_sent: true,
    }
}

#[test]
fn end_to_end_imap_smtp() {
    if std::env::var("LM_IT").is_err() {
        eprintln!("skipped (set LM_IT=1)");
        return;
    }
    let out_dir = "/tmp/smtp_out";
    let mine = |p: &std::path::PathBuf| std::fs::read_to_string(p).map(|s| s.starts_with("X-Envelope-From: tester@localhost")).unwrap_or(false);
    for f in std::fs::read_dir(out_dir).unwrap() {
        let p = f.unwrap().path();
        if mine(&p) {
            let _ = std::fs::remove_file(p);
        }
    }

    let mut raw = raw_session();
    wipe(&mut raw);
    raw.append("INBOX", SAMPLE).finish().unwrap();
    raw.append("INBOX", &plain("first", "Sun, 15 Nov 2026 09:00:00 +0000")).finish().unwrap();
    raw.append("INBOX", &plain("second", "Tue, 17 Nov 2026 09:00:00 +0000")).finish().unwrap();
    raw.create("Projects").unwrap();
    raw.create("Projects/TLR").unwrap();

    let st = AppState::new(Db::memory());
    let mut acct = account(0);
    acct.id = st.db.insert_account(&acct).unwrap();
    secrets::set(&st.db, &secrets::password_key(acct.id), "secret123").unwrap();

    // Folders
    let folders = st.refresh_folders(&acct).unwrap();
    let f: Vec<_> = folders.iter().map(|f| (f.display.as_str(), f.special.as_str(), f.depth)).collect();
    assert_eq!(&f[..4], &[("Inbox", "inbox", 0), ("Drafts", "drafts", 0), ("Sent", "sent", 0), ("Trash", "trash", 0)]);
    assert!(f.contains(&("TLR", "", 1)));

    // Server-side search finds mail that was never downloaded, and adds it to the cache
    let found = st.search_server(&acct, "INBOX", "second", 50).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].subject.contains("second"));
    assert_eq!(st.db.list_messages(acct.id, "INBOX", 100).unwrap().len(), 1);
    assert!(st.search_server(&acct, "INBOX", "no-such-words-anywhere", 50).unwrap().is_empty());
    // quotes / backslashes / non-ASCII must not break the IMAP command
    assert!(st.search_server(&acct, "INBOX", r#"he said "hi" \ ok"#, 50).is_ok());
    assert!(st.search_server(&acct, "INBOX", "عربي", 50).is_ok());

    // Sync
    st.sync_folder(&acct, "INBOX", 100).unwrap();
    let list = st.db.list_messages(acct.id, "INBOX", 100).unwrap();
    let subjects: Vec<_> = list.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(subjects, vec!["second", "Café invoice", "first"], "sorted newest first");
    let inv = list.iter().find(|m| m.subject == "Café invoice").unwrap().clone();
    assert!(inv.has_attach && !inv.seen);
    assert_eq!(inv.from_name, "Jürgen");
    assert_eq!(st.db.folders(acct.id).unwrap()[0].unread, 3);

    // Open -> marks read on server, finds attachment and remote image
    let view = st.open_message(&acct, "INBOX", inv.uid).unwrap();
    assert!(view.has_remote);
    assert_eq!(view.attachments.iter().filter(|a| !a.inline).map(|a| a.name.as_str()).collect::<Vec<_>>(), vec!["invoice.pdf"]);
    st.sync_folder(&acct, "INBOX", 100).unwrap();
    assert!(st.db.summary(acct.id, "INBOX", inv.uid).unwrap().unwrap().seen, "\\Seen persisted on server");

    // Flag
    st.set_flag(&acct, "INBOX", &[inv.uid], "flagged", true).unwrap();
    st.db.update_flags(acct.id, "INBOX", &[(inv.uid, true, false, false)]).unwrap(); // clobber locally
    st.sync_folder(&acct, "INBOX", 100).unwrap();
    assert!(st.db.summary(acct.id, "INBOX", inv.uid).unwrap().unwrap().flagged, "flag came back from server");

    // Server-side deletion is detected
    let first_uid = list.iter().find(|m| m.subject == "first").unwrap().uid;
    raw.select("INBOX").unwrap();
    raw.uid_store(first_uid.to_string(), "+FLAGS.SILENT (\\Deleted)").unwrap();
    raw.expunge().unwrap();
    st.sync_folder(&acct, "INBOX", 100).unwrap();
    assert_eq!(st.db.list_messages(acct.id, "INBOX", 100).unwrap().len(), 2);

    // Delete -> moves to Trash
    let second_uid = list.iter().find(|m| m.subject == "second").unwrap().uid;
    st.delete(&acct, "INBOX", &[second_uid]).unwrap();
    st.sync_folder(&acct, "Trash", 100).unwrap();
    assert_eq!(st.db.list_messages(acct.id, "Trash", 100).unwrap()[0].subject, "second");

    // Move invoice to Projects/TLR
    st.move_to(&acct, "INBOX", &[inv.uid], "Projects/TLR").unwrap();
    st.sync_folder(&acct, "INBOX", 100).unwrap();
    assert!(st.db.list_messages(acct.id, "INBOX", 100).unwrap().is_empty());
    st.sync_folder(&acct, "Projects/TLR", 100).unwrap();
    let moved = st.db.list_messages(acct.id, "Projects/TLR", 100).unwrap();
    assert_eq!(moved[0].subject, "Café invoice");

    // Offline cache: bodies prefetched, then readable from cache
    assert_eq!(st.prefetch_bodies(&acct, "Projects/TLR", 25, 2_000_000).unwrap(), 1);
    assert_eq!(st.db.search(Some(acct.id), "Invoice attached", 10).unwrap().len(), 1, "body text searchable");

    // Send a reply with a local attachment, then forward with original attachments
    let tmp = std::env::temp_dir().join("lm-test-note.txt");
    std::fs::write(&tmp, "note body").unwrap();
    st.send(&acct, &Draft {
        account_id: acct.id, to: "Jürgen <j@example.com>".into(), cc: "".into(), bcc: "secret@example.com".into(),
        subject: "Re: Café invoice".into(), body: "Thanks!".into(),
        in_reply_to: Some("<inv-1@example.com>".into()), references: None,
        attachments: vec![tmp.to_string_lossy().into_owned()], forward_from: None,
        reply_to_msg: Some(MsgRef { folder: "Projects/TLR".into(), uid: moved[0].uid }),
        draft: None, local_draft_id: None, message_id: None, link_large: false,
        ..Default::default()
    }).unwrap();
    st.send(&acct, &Draft {
        account_id: acct.id, to: "dave@example.com".into(), cc: "".into(), bcc: "".into(),
        subject: "Fwd: Café invoice".into(), body: "FYI".into(), in_reply_to: None, references: None,
        attachments: vec![], forward_from: Some(MsgRef { folder: "Projects/TLR".into(), uid: moved[0].uid }), reply_to_msg: None,
        draft: None, local_draft_id: None, message_id: None, link_large: false,
        ..Default::default()
    }).unwrap();

    let mut sent_files: Vec<_> = std::fs::read_dir(out_dir).unwrap().map(|e| e.unwrap().path()).filter(|p| mine(p)).collect();
    sent_files.sort_by_key(|p| p.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0));
    assert_eq!(sent_files.len(), 2);
    let reply = std::fs::read_to_string(&sent_files[0]).unwrap();
    assert!(reply.contains("X-Envelope-To: j@example.com,secret@example.com"), "{reply}");
    assert!(!reply.contains("Bcc:"));
    assert!(reply.contains("In-Reply-To: <inv-1@example.com>"));
    assert!(reply.contains("lm-test-note.txt"));
    let fwd = std::fs::read_to_string(&sent_files[1]).unwrap();
    assert!(fwd.contains("invoice.pdf"), "forward includes original attachment");

    // Sent copies saved via APPEND; original marked answered
    st.sync_folder(&acct, "Sent", 100).unwrap();
    assert_eq!(st.db.list_messages(acct.id, "Sent", 100).unwrap().len(), 2);
    st.sync_folder(&acct, "Projects/TLR", 100).unwrap();
    assert!(st.db.summary(acct.id, "Projects/TLR", moved[0].uid).unwrap().unwrap().answered);

    // Reconnect after the server drops the connection
    std::process::Command::new("doveadm").args(["-c", "/etc/dovecot/test/dovecot.conf", "kick", "tester@localhost"]).status().ok();
    st.sync_folder(&acct, "INBOX", 100).expect("auto-reconnect after kick");

    // Wrong password is reported
    let mut bad = account(999);
    bad.username = "tester@localhost".into();
    let err = crate::mail::connect(&bad, &crate::mail::Auth::Password("nope".into())).err().unwrap();
    assert!(err.contains("login failed"), "{err}");
}

fn draft(account_id: i64, to: &str, subject: &str, body: &str) -> Draft {
    Draft {
        account_id, to: to.into(), cc: String::new(), bcc: "hidden@example.com".into(), subject: subject.into(), body: body.into(),
        in_reply_to: None, references: None, attachments: vec![], forward_from: None, reply_to_msg: None,
        draft: None, local_draft_id: None, message_id: None, link_large: false,
        ..Default::default()
    }
}

/// Drafts, archive, contacts autocomplete, todos, backup + restore, IDLE push.
/// Uses a second mailbox (tester2) so it can run in parallel with the test above.
#[test]
fn end_to_end_phase1_features() {
    if std::env::var("LM_IT").is_err() {
        eprintln!("skipped (set LM_IT=1)");
        return;
    }
    let mut raw = raw_session_for("tester2@localhost");
    wipe(&mut raw);
    raw.append("INBOX", &plain("keep-me", "Sun, 15 Nov 2026 09:00:00 +0000")).finish().unwrap();
    raw.append("INBOX", SAMPLE).finish().unwrap();

    let st = AppState::new(Db::memory());
    let mut acct = account(0);
    acct.email = "tester2@localhost".into();
    acct.username = "tester2@localhost".into();
    acct.id = st.db.insert_account(&acct).unwrap();
    secrets::set(&st.db, &secrets::password_key(acct.id), "secret123").unwrap();
    st.refresh_folders(&acct).unwrap();

    // --- Drafts: save (incomplete recipient), re-save replaces, reopen, send deletes it
    let mut d = draft(acct.id, "sar", "Plan", "first version");
    let s1 = st.save_draft(&acct, &d).unwrap();
    assert!(s1.server_error.is_none(), "{:?}", s1.server_error);
    let r1 = s1.draft.clone().expect("draft uid");
    d.draft = Some(MsgRef { folder: r1.folder.clone(), uid: r1.uid });
    d.local_draft_id = Some(s1.local_draft_id);
    d.message_id = Some(s1.message_id.clone());
    d.body = "second version".into();
    d.to = "Sara <sara@example.com>".into();
    let s2 = st.save_draft(&acct, &d).unwrap();
    let r2 = s2.draft.clone().unwrap();
    assert_ne!(r1.uid, r2.uid);
    st.sync_folder(&acct, "Drafts", 100).unwrap();
    assert_eq!(st.db.list_messages(acct.id, "Drafts", 100).unwrap().len(), 1, "old draft replaced");
    let view = st.open_draft(&acct, "Drafts", r2.uid).unwrap();
    assert_eq!((view.to.as_str(), view.bcc.as_str(), view.body.as_str()), ("Sara <sara@example.com>", "hidden@example.com", "second version"));
    assert!(st.db.pending_drafts().unwrap().is_empty(), "synced drafts are not pending");
    d.draft = Some(MsgRef { folder: r2.folder.clone(), uid: r2.uid });
    st.send(&acct, &d).unwrap();
    st.sync_folder(&acct, "Drafts", 100).unwrap();
    assert!(st.db.list_messages(acct.id, "Drafts", 100).unwrap().is_empty(), "draft removed after sending");

    // --- Archive: no Archive folder yet -> created and used
    st.sync_folder(&acct, "INBOX", 100).unwrap();
    let inbox = st.db.list_messages(acct.id, "INBOX", 100).unwrap();
    let keep = inbox.iter().find(|m| m.subject == "keep-me").unwrap().uid;
    let target = st.archive(&acct, "INBOX", &[keep]).unwrap();
    assert_eq!(target, "Archive");
    st.sync_folder(&acct, "Archive", 100).unwrap();
    assert_eq!(st.db.list_messages(acct.id, "Archive", 100).unwrap()[0].subject, "keep-me");
    assert!(st.db.folders(acct.id).unwrap().iter().any(|f| f.special == "archive"));

    // --- Contacts + autocomplete (contacts first, then senders seen in mail)
    st.db.save_contact(&Contact { name: "Sara Khan".into(), email: "sara@kadenboriss.com".into(), phone: "+971 50 000 0000".into(), ..Default::default() }).unwrap();
    let sug = st.db.suggest("j", 8).unwrap();
    assert!(sug.iter().any(|s| s.email == "j@example.com" && !s.is_contact), "{sug:?}");
    let sug = st.db.suggest("sara", 8).unwrap();
    assert!(sug[0].is_contact && sug[0].email == "sara@kadenboriss.com");

    // --- Todos: due reminder fires exactly once; editing the due date re-arms it
    let past = chrono::Utc::now().timestamp() - 5;
    let id = st.db.save_todo(&Todo { title: "Pay invoice".into(), due: Some(past), ..Default::default() }).unwrap();
    assert_eq!(st.db.take_due_todos().unwrap().len(), 1);
    assert!(st.db.take_due_todos().unwrap().is_empty());
    st.db.save_todo(&Todo { id, title: "Pay invoice".into(), due: Some(past - 60), ..Default::default() }).unwrap();
    assert_eq!(st.db.take_due_todos().unwrap().len(), 1, "re-armed after due change");

    // --- Backup then restore into an emptied mailbox
    st.sync_folder(&acct, "Sent", 100).unwrap();
    let zip = std::env::temp_dir().join("lm-backup-test.zip");
    let count = st.backup(&acct, &zip, &|_| {}).unwrap();
    assert_eq!(count, 3, "inbox(1) + archive(1) + sent(1)");
    wipe(&mut raw);
    raw.delete("Archive").ok();
    let (added, skipped) = st.restore(&acct, &zip, &|_| {}).unwrap();
    assert_eq!((added, skipped), (3, 0));
    let (added, skipped) = st.restore(&acct, &zip, &|_| {}).unwrap();
    assert_eq!((added, skipped), (0, 3), "second restore skips duplicates");
    st.sync_folder(&acct, "INBOX", 100).unwrap();
    let back = st.db.list_messages(acct.id, "INBOX", 100).unwrap();
    assert_eq!(back[0].subject, "Café invoice");
    assert!(!back[0].seen, "flags restored (was unread)");

    // --- IDLE: a message appended by another client wakes the waiting connection
    let inbox_name = "INBOX".to_string();
    let mut conn = None;
    // first call opens the connection and returns quickly if nothing pending
    let _ = st.idle_wait(&mut conn, &acct, &inbox_name, std::time::Duration::from_secs(1));
    let t = std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(700));
        let mut r = raw_session_for("tester2@localhost");
        r.append("INBOX", &plain("pushed", "Tue, 17 Nov 2026 09:00:00 +0000")).finish().unwrap();
    });
    let started = std::time::Instant::now();
    let changed = st.idle_wait(&mut conn, &acct, &inbox_name, std::time::Duration::from_secs(20)).unwrap();
    t.join().unwrap();
    assert!(changed && started.elapsed().as_secs() < 10, "IDLE woke up on new mail");
    let new = st.sync_collect_new(&acct, "INBOX", 100).unwrap();
    assert_eq!(new.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), vec!["pushed"]);
}

/// Nextcloud features against real DAV servers (Radicale for CardDAV/CalDAV, WsgiDAV for files,
/// share API mocked) – see README "Development". Skipped unless LM_IT=1.
#[test]
fn end_to_end_nextcloud() {
    if std::env::var("LM_IT").is_err() {
        eprintln!("skipped (set LM_IT=1)");
        return;
    }
    use crate::dav::Dav;
    use crate::nextcloud::{self, NcConfig};
    let base = "http://127.0.0.1:8090";
    let cards = format!("{base}/radicale/tester/contacts/");
    let tasks = format!("{base}/radicale/tester/tasks/");
    let d = Dav::new("tester", "secret");
    // clean server collections
    for r in d.list(&cards).unwrap() { d.delete(&crate::dav::resolve(&cards, &r.href), None).unwrap(); }
    for r in d.list(&tasks).unwrap() { d.delete(&crate::dav::resolve(&tasks, &r.href), None).unwrap(); }

    // things created on "the phone" directly in Nextcloud
    let phone_card = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:phone-1\r\nFN:Ahmed R.\r\nN:R.;Ahmed;;;\r\nEMAIL;TYPE=WORK:ahmed@example.ae\r\nTEL;TYPE=CELL:+971 50 111\r\nADR;TYPE=WORK:;;DIFC Gate 3;Dubai;;;UAE\r\nBDAY:1988-05-04\r\nEND:VCARD\r\n";
    d.put(&format!("{cards}phone-1.vcf"), "text/vcard", phone_card, None).unwrap();
    let phone_task = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Phone//EN\r\nBEGIN:VTODO\r\nUID:ptask-1\r\nSUMMARY:Renew trade licence\r\nDUE:20261201T050000Z\r\nCATEGORIES:Admin\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
    d.put(&format!("{tasks}ptask-1.ics"), "text/calendar", phone_task, None).unwrap();
    let event = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//x//EN\r\nBEGIN:VEVENT\r\nUID:ev-1\r\nSUMMARY:Not a task\r\nDTSTART:20261201T050000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    d.put(&format!("{tasks}ev-1.ics"), "text/calendar", event, None).unwrap();

    let st = AppState::new(Db::memory());
    let db = &st.db;
    // created in Kite Mail before Nextcloud was connected
    let sara = db.save_contact(&Contact { name: "Sara Khan".into(), email: "sara@kb.com".into(), ..Default::default() }).unwrap();
    let todo = db.save_todo(&Todo { title: "Send Q4 invoices".into(), due: Some(2_000_000_000), ..Default::default() }).unwrap();

    let cfg = NcConfig { enabled: true, server: base.into(), user: "tester".into(), contacts_url: cards.clone(), tasks_url: tasks.clone(), backup_keep: 2, link_threshold_mb: 1, ..Default::default() };
    nextcloud::test_login(&cfg, "secret").unwrap();
    assert!(nextcloud::test_login(&cfg, "wrong").unwrap_err().contains("rejected"), "bad password reported clearly");
    nextcloud::save(db, &cfg).unwrap();
    crate::secrets::set(db, nextcloud::PASSWORD_KEY, "secret").unwrap();

    // ---- first sync: both directions, events ignored
    let r = nextcloud::sync(db).unwrap();
    assert_eq!((r.contacts_down, r.contacts_up, r.tasks_down, r.tasks_up), (1, 1, 1, 1), "{r:?}");
    let ahmed = db.contacts("Ahmed").unwrap().pop().unwrap();
    assert_eq!((ahmed.email.as_str(), ahmed.phone.as_str()), ("ahmed@example.ae", "+971 50 111"));
    assert_eq!(db.todos().unwrap().iter().filter(|t| t.title == "Renew trade licence").count(), 1);
    assert_eq!(d.list(&cards).unwrap().len(), 2);
    // nothing changed -> nothing transferred
    let r = nextcloud::sync(db).unwrap();
    assert_eq!((r.contacts_down, r.contacts_up, r.tasks_down, r.tasks_up, r.deleted), (0, 0, 0, 0, 0), "idempotent: {r:?}");

    // ---- edit in Kite Mail -> server keeps the fields we don't manage
    db.save_contact(&Contact { phone: "+971 55 999".into(), company: "Gulf Ventures".into(), ..ahmed.clone() }).unwrap();
    let t = db.todos().unwrap().into_iter().find(|t| t.id == todo).unwrap();
    db.save_todo(&Todo { done: true, ..t }).unwrap();
    let r = nextcloud::sync(db).unwrap();
    assert_eq!((r.contacts_up, r.tasks_up), (1, 1));
    let (srv, _) = d.get(&format!("{cards}phone-1.vcf")).unwrap();
    assert!(srv.contains("+971 55 999") && srv.contains("Gulf Ventures"));
    assert!(srv.contains("ADR;TYPE=WORK") && srv.contains("BDAY:1988-05-04"), "phone-only fields preserved:\n{srv}");
    let srv_tasks: Vec<String> = d.multiget(&tasks, &d.list_todos(&tasks).unwrap().into_iter().map(|r| r.href).collect::<Vec<_>>(), true).unwrap().into_iter().map(|x| x.2).collect();
    assert!(srv_tasks.iter().any(|x| x.contains("Send Q4 invoices") && x.contains("STATUS:COMPLETED")));

    // ---- edit on the server (phone) -> pulled
    let (cur, etag) = d.get(&format!("{cards}phone-1.vcf")).unwrap();
    let changed = cur.replace("FN:Ahmed R.", "FN:Ahmed Rahman");
    d.put(&format!("{cards}phone-1.vcf"), "text/vcard", &changed, Some(&etag)).unwrap();
    assert_eq!(nextcloud::sync(db).unwrap().contacts_down, 1);
    assert_eq!(db.contact_by_id(ahmed.id).unwrap().unwrap().name, "Ahmed Rahman");

    // ---- conflict: both sides edit -> server wins, no duplicate
    let (cur, etag) = d.get(&format!("{cards}phone-1.vcf")).unwrap();
    d.put(&format!("{cards}phone-1.vcf"), "text/vcard", &cur.replace("NOTE:", "X-NOTE:").replace("END:VCARD", "NOTE:server edit\r\nEND:VCARD"), Some(&etag)).unwrap();
    db.save_contact(&Contact { notes: "local edit".into(), ..db.contact_by_id(ahmed.id).unwrap().unwrap() }).unwrap();
    nextcloud::sync(db).unwrap();
    assert_eq!(db.contact_by_id(ahmed.id).unwrap().unwrap().notes, "server edit");
    assert_eq!(d.list(&cards).unwrap().len(), 2);

    // ---- deletions both ways
    db.delete_contact(sara).unwrap();
    assert!(db.contacts("Sara").unwrap().is_empty(), "hidden immediately");
    d.delete(&format!("{tasks}ptask-1.ics"), None).unwrap();
    let r = nextcloud::sync(db).unwrap();
    assert_eq!(r.deleted, 2);
    assert_eq!(d.list(&cards).unwrap().len(), 1, "Sara removed from server");
    assert!(db.todos().unwrap().iter().all(|t| t.title != "Renew trade licence"), "server deletion pulled");

    // ---- backup to Nextcloud with retention (keep 2)
    let mut raw = raw_session_for("tester3@localhost");
    wipe(&mut raw);
    raw.append("INBOX", &plain("backup-me", "Sun, 15 Nov 2026 09:00:00 +0000")).finish().unwrap();
    let mut acct = account(0);
    acct.email = "tester3@localhost".into();
    acct.username = "tester3@localhost".into();
    acct.id = db.insert_account(&acct).unwrap();
    crate::secrets::set(db, &crate::secrets::password_key(acct.id), "secret123").unwrap();
    let dir = "/tmp/davroot/files/Kite Mail Backups/tester3@localhost";
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    for old in ["2026-01-01", "2026-02-01"] {
        std::fs::write(format!("{dir}/kitemail-backup-tester3_at_localhost-{old}.zip"), b"old").unwrap();
    }
    let shared = std::sync::Arc::new(st);
    let dest = crate::push::backup_to_nextcloud(&shared, &acct, &|_| {}).unwrap();
    assert!(dest.starts_with("Kite Mail Backups/tester3@localhost/kitemail-backup-"));
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names.len(), 2, "retention keeps 2: {names:?}");
    assert!(names[0].contains("2026-02-01"));
    let zip = std::fs::metadata(format!("{dir}/{}", names[1])).unwrap();
    assert!(zip.len() > 200, "real backup uploaded");
    assert!(!nextcloud::backup_due(&nextcloud::load(&shared.db), "tester3@localhost"));

    // ---- large attachment becomes a share link
    let big = std::env::temp_dir().join("Q4 report big.pdf");
    std::fs::write(&big, vec![b'x'; 1_200_000]).unwrap();
    let small = std::env::temp_dir().join("note.txt");
    std::fs::write(&small, b"small").unwrap();
    let mut body = "Please see attached.\n\nOn Mon, Sara wrote:\n> hi".to_string();
    let mut files = vec![big.to_string_lossy().into_owned(), small.to_string_lossy().into_owned()];
    assert_eq!(nextcloud::linkify_attachments(&shared.db, &mut body, &mut files).unwrap(), 1);
    assert_eq!(files, vec![small.to_string_lossy().into_owned()], "small file still attached");
    assert!(body.contains("📎 Q4 report big.pdf (1.1 MB): http://localhost:8090/s/"), "{body}");
    assert!(body.find("http://localhost").unwrap() < body.find("On Mon").unwrap(), "links above the quote");
    let day = chrono::Local::now().format("%Y-%m-%d").to_string();
    assert!(std::path::Path::new(&format!("/tmp/davroot/files/Kite Mail Attachments/{day}/Q4 report big.pdf")).exists());
}

/// S3: real SigV4 against a local S3 server that checks signatures (moto with auth on; keys in
/// /tmp/s3keys as "ACCESS SECRET", bucket "kite-backups" on :9010). Skipped unless LM_IT=1.
#[test]
fn end_to_end_s3() {
    if std::env::var("LM_IT").is_err() {
        eprintln!("skipped (set LM_IT=1)");
        return;
    }
    use crate::s3;
    let keys = std::fs::read_to_string("/tmp/s3keys").expect("run the S3 test server first");
    let (ak, sk) = keys.trim().split_once(' ').unwrap();
    let st = AppState::new(Db::memory());
    let db = &st.db;
    let mut cfg = s3::S3Config {
        endpoint: "http://127.0.0.1:9010".into(),
        bucket: "kite-backups".into(),
        access_key: ak.into(),
        prefix: "it-test".into(),
        path_style: true,
        enabled: true,
        backup_keep: 2,
        link_threshold_mb: 1,
        link_expire_days: 3,
        ..Default::default()
    };
    // wrong secret / bucket give readable errors
    let e = s3::Client::new(&cfg, "not-the-secret").unwrap().test().unwrap_err();
    assert!(e.contains("secret key is wrong"), "{e}");
    let bad = s3::S3Config { bucket: "no-such-bucket-here".into(), ..cfg.clone() };
    let e = s3::Client::new(&bad, sk).unwrap().test().unwrap_err();
    assert!(e.contains("bucket does not exist"), "{e}");
    s3::Client::new(&cfg, sk).unwrap().test().unwrap();
    crate::secrets::set(db, s3::SECRET_KEY, sk).unwrap();
    s3::save(db, &cfg).unwrap();
    let (_, c) = s3::client(db).unwrap();
    for k in c.list("it-test/").unwrap() {
        c.delete(&k).unwrap();
    }

    // backups with retention (keep 2)
    let dir = std::env::temp_dir().join(format!("kite-s3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for day in 1..=3 {
        let f = dir.join(format!("kitemail-backup-akhil_at_x-2026-01-0{day}.zip"));
        std::fs::write(&f, format!("zip {day}")).unwrap();
        let dest = s3::upload_backup(db, &f, "akhil@x.ae").unwrap();
        assert!(dest.starts_with("kite-backups/it-test/backups/akhil@x.ae/"), "{dest}");
    }
    let mut left = c.list("it-test/backups/").unwrap();
    left.sort();
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(left[0].ends_with("2026-01-02.zip") && left[1].ends_with("2026-01-03.zip"));
    assert!(s3::backup_due(&s3::load(db), "akhil@x.ae"));
    s3::mark_backup(db, "akhil@x.ae", None).unwrap();
    assert!(!s3::backup_due(&s3::load(db), "akhil@x.ae"));

    // large attachment -> pre-signed link that downloads the same bytes, original file name.
    // (moto's signature-checking mode can't serve pre-signed GETs, so links are exercised on a
    //  second, non-checking instance on :9011; the link signature itself is verified against
    //  AWS's published example in s3::tests.)
    cfg.endpoint = "http://127.0.0.1:9011".into();
    s3::save(db, &cfg).unwrap();
    let big = dir.join("Case file – Ü 2026.pdf");
    let bytes: Vec<u8> = (0..1_500_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&big, &bytes).unwrap();
    let small = dir.join("note.txt");
    std::fs::write(&small, "hi").unwrap();
    let mut body = "Please see below.\n\nOn Mon, X wrote:\n> old".to_string();
    let mut paths = vec![big.to_string_lossy().into_owned(), small.to_string_lossy().into_owned()];
    assert_eq!(s3::linkify_attachments(db, &mut body, &mut paths).unwrap(), 1);
    assert_eq!(paths, vec![small.to_string_lossy().into_owned()]);
    assert!(body.contains("expire in 3 days") && body.find("📎").unwrap() < body.find("On Mon").unwrap(), "{body}");
    let url = body.split("): ").nth(1).unwrap().split_whitespace().next().unwrap();
    assert!(url.contains("X-Amz-Expires=259200"), "{url}");
    let resp = ureq::get(url).call().unwrap();
    assert!(resp.header("Content-Disposition").unwrap_or("").contains("Case"), "{:?}", resp.header("Content-Disposition"));
    let mut got = Vec::new();
    std::io::Read::read_to_end(&mut resp.into_reader(), &mut got).unwrap();
    assert_eq!(got, bytes);

    cfg.enabled = false;
    s3::save(db, &cfg).unwrap();
    assert!(!s3::backup_due(&s3::load(db), "other@x.ae"));
    let _ = std::fs::remove_dir_all(&dir);
}
