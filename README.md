# Kite Mail

Kite Mail is a lightweight email client for **Windows, Linux and Android**. It's built with **Tauri 2 (Rust)** and a UI written in plain HTML and JavaScript with no framework. It uses the system web view instead of bundling Chromium, so the app file is about 11 MB, and CPU use while idle is close to zero.

**Supported accounts:** Microsoft 365 (OAuth), Gmail (OAuth), and any provider that supports IMAP and SMTP.

## What's new in 0.4 (Thunderbird-style settings, S3, solid themes)
- **S3 object storage** (Settings → Cloud & Backup): Amazon S3, Oracle Cloud Object Storage, Cloudflare R2, Wasabi, MinIO and other S3-compatible storage. Daily/weekly mailbox backups with "keep the latest N", one-click backup per account, and large attachments sent as private download links that expire (1–7 days). Keys are stored in the system keychain; requests are signed with AWS Signature V4.
- **Account settings like Thunderbird** (Settings → Accounts → Settings): account name and colour, default account, your name, reply-to, organization, HTML signatures (or a signature file), attach your vCard, **other identities / aliases** to send from, server settings (test & save), save sent copies, Bcc yourself, reply above/below the quote, read-receipt requests, how many days of mail to keep in sync and offline copies.
- **Solid colour themes:** Black, Blue, Charcoal, Emerald, Maroon, Purple, White, Sky Blue, Silver – flat colours, no gradients. **Animations are off by default** (Appearance → Animations).
- **Notifications:** choose what they show (sender & subject / sender only / just "New mail") and a sound (built-in chime, your own sound file, or silent).
- **Privacy & Security:** scam detection warns about links whose text shows a different site than where they go, raw-IP links, look-alike (punycode) domains and senders whose name pretends to be another address. **Return receipts:** ask / always / never send when a sender requests one.
- **Spam:** Spam button and **J** shortcut – move to Junk (created if missing) or delete, optionally mark read; "Not spam" in the Junk folder moves mail back to the Inbox.
- Settings are now organised in sections with a side menu (chips on phones).
- Security: lettre upgraded to 0.11.24 (six SMTP advisories published 2026-11-17).

Not included on purpose: chat (Thunderbird's IRC/XMPP), an add-on store, and DNS-over-HTTPS – they add size and attack surface without helping email work. OpenPGP / S/MIME encryption is a planned next step.

## What's new in 0.3.1 (QA + security release)
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

## What's new in 0.3 (Phase 2)

| Area | What you get |
|---|---|
| **Android app** | Phone layout with a bottom tab bar, folder drawer and full-screen reader and compose. Swipe left on a message to **archive** it and swipe right to mark it **read/unread**. Pull down to refresh, and the phone's Back button works throughout. |
| **QR pairing** | On the computer, go to Settings → **Pair a phone**. The phone scans the code and gets all accounts (already signed in), contacts, tasks, themes, signatures and the Nextcloud connection. The data is encrypted with AES-256-GCM using a one-time key that exists only in the QR code. It can be downloaded once, within 10 minutes, on your local network. Mail then syncs from your mail servers. |
| **Nextcloud contacts** | Two-way CardDAV sync. Fields that Kite Mail doesn't manage (extra phone numbers, addresses, birthdays, photos) are preserved. |
| **Nextcloud tasks** | Two-way CalDAV sync with Nextcloud Tasks. Due dates include an alarm, so your phone reminds you too. Categories and subtasks are preserved. |
| **Backups to Nextcloud** | Automatic daily or weekly backup of every account to a Nextcloud folder, keeping the last N backups. There's also a "To Nextcloud" button for an immediate backup. |
| **Large attachments** | Files over a size you choose are uploaded to Nextcloud and sent as a download link (with an optional expiry) instead of bloating the email. |
| **Other DAV servers** | Radicale, Baïkal, SOGo and others work too. Enter their contacts and tasks URLs under "Other DAV server". |
| **Fixes** | The message list now scrolls on Windows. Recent mail always appears at the top, even in inboxes with imported or restored mail. |

## What was added in 0.2 (Phase 1)

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

### Why 3CX calling uses `tel:` links rather than a full integration
The 3CX desktop and mobile apps already handle `tel:` links, so clicking a number dials through your own extension. A direct 3CX Call Control API integration would need extra licensing, PBX configuration and a stored API key, and you wouldn't gain anything for click-to-call. If you use the 3CX Web Client instead of the app, choose **Settings → Calling → 3CX Web Client** and enter `https://YOURPBX/webclient/#/call?phone={number}`.

### Grammarly
Grammarly closed its SDK for third-party apps in 2024, so it can't be built in. LanguageTool is the open-source alternative. You can use the free public server, which is limited to roughly 20 checks per minute, or run your own:
```bash
docker run -d -p 8010:8010 erikvl87/languagetool     # then set http://SERVER:8010 in Settings → Writing
```
(A Nextcloud LanguageTool app also exists.)

---

## 1. Build

### Windows 10/11
1. Install **Rust** from https://rustup.rs (choose the default MSVC toolchain).
2. Install **Visual Studio 2022 Build Tools** with the **"Desktop development with C++"** workload. WebView2 is already included in Windows.
3. Install the Tauri CLI and build:
   ```powershell
   cargo install tauri-cli --version "^2" --locked
   cd kitemail
   cargo tauri build
   ```
   The installer is written to `src-tauri\target\release\bundle\nsis\Kite Mail_0.2.0_x64-setup.exe` (an `.msi` is created as well).

To run it during development, use `cargo tauri dev`.

### Linux (Ubuntu/Debian)
```bash
sudo apt install build-essential curl wget file pkg-config libssl-dev libdbus-1-dev \
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev \
  hunspell-en-us          # dictionary for spell-checking
curl https://sh.rustup.rs -sSf | sh
cargo install tauri-cli --version "^2" --locked
cargo tauri build         # .deb and .AppImage in src-tauri/target/release/bundle/
```

### Android
You need Android Studio (SDK and **NDK**), Java 17, and the Rust Android targets:
```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
export NDK_HOME=$ANDROID_HOME/ndk/<version>
cargo tauri android init
cargo tauri android build --apk --debug --target aarch64     # installable test APK
```
**Installing on a phone:** copy the `.apk` to the phone and open it. You'll need to allow "Install unknown apps" for your file manager or browser. Then open Kite Mail → **Scan code from computer**.

**Signed release APK (smaller and faster):** create a signing key once, then add it to GitHub under **Settings → Secrets and variables → Actions**:
```bash
keytool -genkey -v -keystore kitemail.jks -alias kitemail -keyalg RSA -keysize 2048 -validity 10000
base64 -w0 kitemail.jks      # copy the output
```
| Secret | Value |
|---|---|
| `ANDROID_KEYSTORE_B64` | the base64 output |
| `ANDROID_KEYSTORE_PASSWORD` | the keystore password |
| `ANDROID_KEY_ALIAS` | `kitemail` |

Keep `kitemail.jks` safe: future updates must be signed with the same key. Without these secrets, the build produces a debug-signed test APK instead.

### GitHub Actions & releases
Every push to `main` builds the Windows installer, the Linux packages and the Android APK. You can download them from the run's **Artifacts** for 90 days, and only signed-in users can do so.

**Permanent download links:** tag a version and GitHub creates a **Release** with every installer attached:
```bash
git tag v0.4.0 && git push origin v0.4.0
```
You can also use **Releases → Draft a new release → Choose a tag → `v0.4.0`**.

---

## 2. Microsoft 365 setup (one time, done by a tenant admin)

1. **Entra admin center** → **App registrations → New registration**.
   - Name: `Kite Mail`
   - Supported account types: **this organizational directory only**
   - Redirect URI: **Public client/native (mobile & desktop)** → `http://localhost`
2. **API permissions → Add → APIs my organization uses → Office 365 Exchange Online → Delegated**, then tick `IMAP.AccessAsUser.All` and `SMTP.Send`. Also add **Microsoft Graph → Delegated** `offline_access`, `openid` and `email`. Then click **Grant admin consent**.
3. Copy the **Application (client) ID** and the **Directory (tenant) ID**.
4. Make sure each mailbox allows **IMAP** and **Authenticated SMTP**: in the Admin center go to **Users → Mail → Manage email apps**, or use PowerShell:
   ```powershell
   Set-CASMailbox -Identity user@yourdomain.com -ImapEnabled $true -SmtpClientAuthenticationDisabled $false
   ```
5. In Kite Mail, go to **Add account → Microsoft 365** and enter your email, the Client ID, and the Tenant (the tenant ID or your domain). Sign in when the browser opens.

If a sign-in expires, an ⚠ and a **Reconnect** button appear next to the account.

## 3. Gmail setup (one time)
1. In https://console.cloud.google.com, create a project and enable the **Gmail API**.
2. On the **OAuth consent screen**, choose **Internal** (Workspace) or **External**, add the scope `https://mail.google.com/`, and add yourself under **Test users**.
3. Go to **Credentials → OAuth client ID → Desktop app** and copy the Client ID and Client secret.
4. In Kite Mail, go to **Add account → Gmail**.

> If the consent screen is **External** and still in **Testing**, Google ends the sign-in after 7 days. Use **Internal**, or click **Publish app**, to avoid that.
> Alternatively, use **Other (IMAP)** with `imap.gmail.com:993` / `smtp.gmail.com:465` and an App password.

## 4. Other providers
**Add account → Other (IMAP)**. For example, Hostinger: `imap.hostinger.com:993` and `smtp.hostinger.com:465` with SSL/TLS.

---

## Data & privacy
| What | Windows | Linux |
|---|---|---|
| Mail cache, contacts, tasks, settings | `%APPDATA%\KiteMail\kitemail.db` | `~/.local/share/KiteMail/kitemail.db` |
| Passwords / OAuth tokens | Windows Credential Manager | Secret Service (GNOME Keyring / KWallet) |

- Emails are shown in a sandbox where scripts can't run, and remote images (which are often used as trackers) are blocked until you allow them.
- Nothing is sent anywhere except your mail servers, plus LanguageTool when you click **Check**.
- Dependencies are checked against the RustSec advisory database on every build (`cargo audit --deny warnings`).

## Performance notes
- Push mail uses one sleeping IMAP connection per account. The app wakes only when the server reports a change, so idle CPU is close to zero (measured at about 0.03% over 60 seconds).
- The message list only creates DOM rows for what is on screen, so large folders scroll smoothly.
- Animations run only while something is loading. Nothing redraws in the background.
- The release binary is about 11 MB, with LTO and size optimisation. Most of the memory use is the system web view.

## 5. Nextcloud setup
1. In Nextcloud, go to **Settings → Security → Devices & sessions → Create new app password**, name it "Kite Mail", and copy the password.
2. In Kite Mail, go to **Settings → Nextcloud** and enter your server address (for example `https://cloud.yourcompany.com`), username and app password, then click **Connect**. The first sync starts immediately.
3. Optional: choose a task list, set the backup schedule and folder, and set the size limit for large attachments.

Contacts and tasks sync automatically every 15 minutes and a few seconds after each change. Use **Sync now** for an immediate sync.
If you edit the same contact on two devices at once, the server's version wins. Deletions sync in both directions.

## 6. Pairing a phone
1. Install the APK on the phone and open Kite Mail.
2. On the computer, go to **Settings → Pair a phone → Show pairing QR code**.
3. On the phone, tap **Scan code from computer**. Both devices must be on the **same Wi-Fi**.
4. Done. Your accounts work without signing in again, and mail downloads from the servers.

Can't scan? Choose "Can't scan? Show code" on the computer and paste the code into the phone (Settings → Copy setup from your computer).

## Project layout
```
ui/index.html, style.css
ui/js/  icons · themes · core · mail · compose · contacts · tasks · settings · sync · main
src-tauri/src/
  lib.rs        app setup, commands, Linux spell-check, mobile entry point
  commands.rs   Tauri commands used by the UI
  mail.rs       IMAP pool, sync, drafts, archive, IDLE, backup/restore (mbox), send pipeline
  push.rs       push-mail threads + task reminders + notifications
  oauth.rs      OAuth 2.0 + PKCE with loopback redirect (Microsoft / Google)
  smtp.rs       message building & sending (lettre)
  mime.rs       parsing, previews, HTML→text, inline images, draft re-open
  grammar.rs    LanguageTool client
  dav.rs        WebDAV/CardDAV/CalDAV client, vCard + VTODO mapping that keeps unknown fields
  nextcloud.rs  contacts/tasks two-way sync, backups with retention, share links
  pair.rs       encrypted one-time phone pairing (AES-256-GCM, QR, LAN transfer)
  db.rs         SQLite cache, contacts, tasks, settings, local drafts
  secrets.rs    OS keyring with fallback
  it_tests.rs   end-to-end tests against real IMAP/SMTP servers, Nextcloud/DAV and S3
                (S3: `moto_server -p 9010` with INITIAL_NO_AUTH_ACTION_COUNT set, so signatures are checked)
```

## Tests
```bash
cd src-tauri
cargo test                       # 28 unit tests (incl. pairing over the network and AWS SigV4 test vectors)
cargo audit                      # dependency security check (policy: .cargo/audit.toml)
LM_IT=1 cargo test it_tests      # 3 end-to-end tests against real servers:
                                 #   IMAPS localhost:993 + SMTPS :4650 (Dovecot + aiosmtpd),
                                 #   users tester@, tester2@, tester3@localhost / secret123
                                 #   DAV on :8090 – Radicale (CardDAV/CalDAV) + WsgiDAV (files) + share-API mock
```
