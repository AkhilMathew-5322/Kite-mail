# Changelog

Newest first. Each version is also on the [Releases page](https://github.com/AkhilMathew-5322/Kite-mail/releases) with its installers.

## 0.5.4 – permanently signed Android app

- The Android APK is now signed with the project's permanent key. From this version on, updates install over the previous one and keep your accounts. Moving from a `-testkey` build needs one last uninstall.

## 0.5.3 – QR scanning everywhere

- **The Android app no longer closes when scanning a QR code.** The native scanner plugin is replaced by Kite Mail's own scanner. That also drops CameraX and ML Kit from the app.
- **QR scanning on Windows, Linux and Android.** Use **Add account → Scan QR code**, or Settings → Devices, with the camera (webcam on a laptop). You can also choose a screenshot/photo of the code or paste one (Ctrl+V).
- **Pairing works in every direction**: computer → phone, phone → computer, computer → computer. Every device can show a pairing code and scan one.
- Swipes can no longer trigger the WebView's own "go back a page" navigation (touchscreen laptops).

## 0.5.2 – Android back gesture, automatic releases

- **Android Back now works everywhere.** Swiping back (or the back button) closes the open message and returns to the list. It also closes the folder drawer, compose and the account sheet, goes from another folder to Inbox and from Contacts/Tasks/Settings to Mail. At the Inbox it shows "Press back again to exit". This uses Android's native back event, not WebView history.
- **Slide an open message to the right** to go back to the list. Sideways-scrollable content such as wide tables keeps its own swipe.
- The message reader no longer creates a history entry per email opened.
- **GitHub Releases are published automatically** for every new version, with all installers attached.
- Tauri updated to 2.12.7.

## 0.5.1 – Files folder on cPanel-style servers

- The **Files** folder now appears as soon as an account connects, and is filled once from attachments in the last 30 days of Inbox mail.
- It now works on servers that keep every folder under `INBOX.` (cPanel, Courier, many shared hosts). There it's created as `INBOX.Files`; other mail apps show it as *Files* under Inbox.
- If the server still refuses to create the folder, the error now includes the server's own reason.

## 0.5 – Files folder, Archive, conversations
- **Files folder:** a *Files* folder on your mail server, next to Inbox and Sent, that collects pictures, PDFs and Word files from your mail (Excel and PowerPoint optional, per account). Each file is a small message with the file attached, so the folder syncs to every device and also shows in Outlook, Thunderbird or a phone's mail app. Kite Mail shows it as a gallery with picture thumbnails and filters; files can be opened, saved, sent by email or deleted, added from the computer, or saved from any email with the new *Save to Files* button. Duplicates are skipped (checked on the server), logos inside emails and tiny pictures are ignored. Settings → Accounts → an account → *Files folder*, including "Collect from the last 30 days".
- **Archive** (new tab): import **PST, MSG, EML, MBOX, Maildir** and Kite Mail backups – single files or whole folders – into a searchable archive kept apart from your accounts. Folder trees are kept, attachments included, duplicates skipped, progress shown (and stoppable) for big PST files. Full-text search across everything; reply, reply all or **forward with the original attachments** from any account; copy messages back into a mailbox; save as .eml. (OST files – Outlook's offline cache – can't be read; export a PST from Outlook instead.)
- **Conversations:** replies are grouped into one row in the list ("Sara, Omar · 3"), and the reader shows the whole conversation in order – including your own replies from Sent – so you can jump between messages. Turn it off under Appearance.
- **Android app icon:** proper adaptive launcher icon (round, squircle, Android 13 themed icons).

## 0.4.1 – smaller Android app, safer attachments
- **Android APK ~20 MB instead of 224 MB:** CI now always builds the optimised release version, and the app uses one TLS stack (the platform's, same as for IMAP/SMTP) instead of also bundling rustls + ring.
- **Dangerous attachments can't be opened directly:** programs, scripts, shortcuts, disk images and macro documents (.exe, .js, .lnk, .iso, .docm…) are marked in red and can only be saved – the same protection Thunderbird has.
- Copies of opened attachments are removed from the temp folder after a day.
- Signature and sound-file pickers only accept the expected file types.

## 0.4 – Thunderbird-style settings, S3, solid themes
- **S3 object storage** (Settings → Cloud & Backup): Amazon S3, Oracle Cloud Object Storage, Cloudflare R2, Wasabi, MinIO and other S3-compatible storage. Daily/weekly mailbox backups with "keep the latest N", one-click backup per account, and large attachments sent as private download links that expire (1–7 days). Keys are stored in the system keychain; requests are signed with AWS Signature V4.
- **Account settings like Thunderbird** (Settings → Accounts → Settings): account name and colour, default account, your name, reply-to, organization, HTML signatures (or a signature file), attach your vCard, **other identities / aliases** to send from, server settings (test & save), save sent copies, Bcc yourself, reply above/below the quote, read-receipt requests, how many days of mail to keep in sync and offline copies.
- **Solid colour themes:** Black, Blue, Charcoal, Emerald, Maroon, Purple, White, Sky Blue, Silver – flat colours, no gradients. **Animations are off by default** (Appearance → Animations).
- **Notifications:** choose what they show (sender & subject / sender only / just "New mail") and a sound (built-in chime, your own sound file, or silent).
- **Privacy & Security:** scam detection warns about links whose text shows a different site than where they go, raw-IP links, look-alike (punycode) domains and senders whose name pretends to be another address. **Return receipts:** ask / always / never send when a sender requests one.
- **Spam:** Spam button and **J** shortcut – move to Junk (created if missing) or delete, optionally mark read; "Not spam" in the Junk folder moves mail back to the Inbox.
- Settings are now organised in sections with a side menu (chips on phones).
- Security: lettre upgraded to 0.11.24 (six SMTP advisories published 2026-11-17).

Not included on purpose: chat (Thunderbird's IRC/XMPP), an add-on store, and DNS-over-HTTPS – they add size and attack surface without helping email work. OpenPGP / S/MIME encryption is a planned next step.

## 0.3.1 – QA + security release
- **Security:** `cargo audit` reports **0 vulnerabilities**. SQLite, zip and the IMAP library were upgraded (the old IMAP parser used an unsound number-parsing crate). The few remaining "unmaintained" notices come from Tauri's own Linux stack (GTK3/D-Bus) and are listed with reasons in `src-tauri/.cargo/audit.toml`. CI now fails on any new advisory.
- **Account switcher on Android:** tap your avatar (top right) for a bottom sheet with every account, its unread count and colour. **Swipe the avatar** up/down or left/right to flip straight to the next account. A red dot means new mail in another account, each account reopens at the folder you last used, and the drawer shows only the current account's folders.
- **Search on the server:** after a local search, click *Search all of "Folder" on the server* to find mail that was never downloaded.
- **More reliable push:** a dead connection is noticed within 5 minutes (for example after sleep or a Wi-Fi change), retries are capped at 2 minutes, and a crashed push thread restarts by itself.
- **Safer deleting:** permanently deleting a message never removes other messages that another mail app has marked for deletion.
- **Sign-in can be cancelled** while the browser window is open.
- **Refresh (F5)** also re-reads the folder list, so new or renamed folders appear.
- **Pairing QR** lists every network address of the computer, and the phone uses the first one that answers.
- Drafts: sending while an autosave is running no longer leaves a stray draft behind, and drafts of removed accounts are cleaned up.
- Messages and errors now appear on top of open dialogs, and error texts are written in plain language.
- Android downloads fewer and smaller messages in the background to save data.
- Hardening: emails can no longer redirect the reader (`<meta refresh>`, `<base>`, embedded frames are stripped), Windows reserved file names (CON, NUL…) are made safe when saving attachments, and a malformed email can no longer crash the whole app.

## 0.3 – Phase 2

| Area | What you get |
|---|---|
| **Android app** | Phone layout with a bottom tab bar, folder drawer and full-screen reader and compose. Swipe left on a message to **archive** it and swipe right to mark it **read/unread**. Pull down to refresh, and the phone's Back button works throughout. |
| **QR pairing** | On any device, go to Settings → **Devices → Show pairing QR code**. Another computer or phone scans it (camera or a screenshot) and gets all accounts (already signed in), contacts, tasks, themes, signatures and the Nextcloud connection. The data is encrypted with AES-256-GCM using a one-time key that exists only in the QR code. It can be downloaded once, within 10 minutes, on your local network. Mail then syncs from your mail servers. |
| **Nextcloud contacts** | Two-way CardDAV sync. Fields that Kite Mail doesn't manage (extra phone numbers, addresses, birthdays, photos) are preserved. |
| **Nextcloud tasks** | Two-way CalDAV sync with Nextcloud Tasks. Due dates include an alarm, so your phone reminds you too. Categories and subtasks are preserved. |
| **Backups to Nextcloud** | Automatic daily or weekly backup of every account to a Nextcloud folder, keeping the last N backups. There's also a "To Nextcloud" button for an immediate backup. |
| **Large attachments** | Files over a size you choose are uploaded to Nextcloud and sent as a download link (with an optional expiry) instead of bloating the email. |
| **Other DAV servers** | Radicale, Baïkal, SOGo and others work too. Enter their contacts and tasks URLs under "Other DAV server". |
| **Fixes** | The message list now scrolls on Windows. Recent mail always appears at the top, even in inboxes with imported or restored mail. |

## 0.2 – Phase 1

| Area | What you get |
|---|---|
| **Design** | New navigation rail (Mail · Contacts · Tasks · Settings), sender avatars, message previews, list grouped by day (Today / Yesterday / This week), quick actions when you hover over a message, and a fast list that only draws the rows on screen |
| **Dark mode & themes** | System, Light or Dark mode, plus a one-click toggle. Includes 10 built-in themes (Aurora, Ocean, Sand, Mint, Paper, Midnight, Graphite, Nord, Forest, Neon). You can also create **custom themes** with a live colour editor. Emails can optionally be darkened in dark mode. Includes density and text-size options. |
| **Push mail** | New mail arrives instantly through IMAP IDLE (no more 3-minute polling), with desktop notifications |
| **Drafts** | Saved automatically every 20 seconds to the server's Drafts folder, with a local backup copy. Drafts saved while offline are recovered at the next start. Click a draft to keep editing it. A saved draft is removed after you send it. |
| **Archive** | Archive button and the `E` shortcut. Uses the server's Archive folder, or "All Mail" on Gmail, or creates an Archive folder if there isn't one. |
| **Backup & restore** | Settings → Accounts → **Backup** downloads every message in every folder into one `.zip` (a standard mbox file per folder, with read/star flags kept). **Restore** uploads a backup into any account and skips messages that are already there. |
| **Contacts** | Contacts book with favourites, an "add sender to contacts" button, and address autocomplete when composing (from your contacts and people you've received mail from) |
| **Tasks** | To-do list with due dates, quick presets (+1 h, 6 pm, Tomorrow 9 am), an Important flag, and **reminder notifications**. You can create a task from an email with the `T` shortcut and jump back to that email from the task. |
| **Spelling & grammar** | Spell check as you type (Windows: built into WebView2; Linux: system dictionaries). The **Check** button uses LanguageTool for grammar and style suggestions, applied with one click. |
| **Calling (3CX)** | 📞 buttons on contacts and on phone links in emails. Calls go through the `tel:` handler (the 3CX app, Teams or Phone Link) or the 3CX Web Client URL. |
| **Signatures** | One signature per account, added automatically when you compose |

Keyboard shortcuts: `↑`/`↓` or `j`/`k` to move between messages · `Del` delete · `E` archive · `U` mark unread · `S` star · `T` create task · `R` reply · `A` reply all · `F` forward · `Ctrl+N` new message · `Ctrl+F` search · `F5` refresh · `Ctrl+Enter` send · `Ctrl+S` save draft
