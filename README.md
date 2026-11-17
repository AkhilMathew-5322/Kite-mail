# Kite Mail

Kite Mail is a lightweight email client for **Windows and Linux**, with an Android build in progress. It's built with **Tauri 2 (Rust)** and a UI written in plain HTML and JavaScript with no framework. It uses the system web view instead of bundling Chromium, so the app file is about 11 MB, and CPU use while idle is close to zero.

**Supported accounts:** Microsoft 365 (OAuth), Gmail (OAuth), and any provider that supports IMAP and SMTP.

## What's new in 0.2 (Phase 1)

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

### Android (experimental, Phase 2 adds phone-optimised screens and QR pairing)
You need Android Studio (SDK and **NDK**), Java 17, and the Rust Android targets:
```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
export NDK_HOME=$ANDROID_HOME/ndk/<version>
cargo tauri android init
cargo tauri android build --apk --debug --target aarch64     # installable test APK
```
The interface already switches to a phone layout on narrow screens: a bottom tab bar, a slide-out folder drawer and a full-screen reader.

### GitHub Actions
Push this folder to a GitHub repo. `.github/workflows/build.yml` builds the Windows installer, the Linux packages, and a test Android APK, which you can download from the run's **Artifacts**. The Android job is marked experimental, so if it fails the Windows and Linux builds still succeed.

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

## Performance notes
- Push mail uses one sleeping IMAP connection per account. The app wakes only when the server reports a change, so idle CPU is close to zero (measured at about 0.03% over 60 seconds).
- The message list only creates DOM rows for what is on screen, so large folders scroll smoothly.
- Animations run only while something is loading. Nothing redraws in the background.
- The release binary is about 11 MB, with LTO and size optimisation. Most of the memory use is the system web view.

## Phase 2 roadmap
- **Android app**: phone-tuned screens, and **QR pairing** that copies accounts, contacts, tasks, themes and settings to the phone (encrypted). Mail then syncs from the server.
- **Nextcloud**: two-way sync of contacts (CardDAV) and tasks (CalDAV), scheduled backups to a Nextcloud folder, and large attachments sent as share links.

## Project layout
```
ui/index.html, style.css
ui/js/  icons · themes · core · mail · compose · contacts · tasks · settings · main
src-tauri/src/
  lib.rs        app setup, commands, Linux spell-check, mobile entry point
  commands.rs   Tauri commands used by the UI
  mail.rs       IMAP pool, sync, drafts, archive, IDLE, backup/restore (mbox), send pipeline
  push.rs       push-mail threads + task reminders + notifications
  oauth.rs      OAuth 2.0 + PKCE with loopback redirect (Microsoft / Google)
  smtp.rs       message building & sending (lettre)
  mime.rs       parsing, previews, HTML→text, inline images, draft re-open
  grammar.rs    LanguageTool client
  db.rs         SQLite cache, contacts, tasks, settings, local drafts
  secrets.rs    OS keyring with fallback
  it_tests.rs   end-to-end tests against real IMAP/SMTP servers
```

## Tests
```bash
cd src-tauri
cargo test                       # 14 unit tests
LM_IT=1 cargo test it_tests      # 2 end-to-end tests: needs IMAPS on localhost:993 + SMTPS on :4650
                                 # users tester@localhost and tester2@localhost / secret123
```
