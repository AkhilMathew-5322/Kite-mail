# Building & developing Kite Mail

For people who build the app themselves or work on the code. Back to the [README](../README.md).

## Build

### Windows 10/11
1. Install **Rust** from https://rustup.rs (choose the default MSVC toolchain).
2. Install **Visual Studio 2022 Build Tools** with the **"Desktop development with C++"** workload. WebView2 is already included in Windows.
3. Install the Tauri CLI and build:
   ```powershell
   cargo install tauri-cli --version "^2" --locked
   cd kitemail
   cargo tauri build
   ```
   The installer is written to `src-tauri\target\release\bundle\nsis\Kite Mail_<version>_x64-setup.exe` (an `.msi` is created as well).

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
**Installing on a phone:** copy the `.apk` to the phone and open it. You'll need to allow "Install unknown apps" for your file manager or browser. Then open Kite Mail → **Scan code from other device**.

### GitHub Actions & releases
Every push to `main` builds the Windows installer, the Linux packages and the Android APK (run **Artifacts**, kept 90 days, sign-in needed).

**Releases are automatic.** When `main` gets a new version number (`version` in `src-tauri/tauri.conf.json`), CI publishes a **Release** named after it (e.g. `v0.5.2`). It has every installer attached and the newest section of `docs/CHANGELOG.md` as its notes. Pushes that keep the same version only produce Artifacts. To rebuild an existing release, push its tag again (`git tag -f v0.5.2 && git push -f origin v0.5.2`) or publish it from **Releases** on github.com.

### Android signing (needed for in-place updates)
Android only installs an update over the existing app if both are signed with the **same key**. Without a saved key, CI signs each APK with a new throwaway key (file name ends in `-testkey`). You then have to uninstall the old app first, which removes its accounts. Do this once:

1. Create a key (keep the file and password safe – losing them means users must reinstall). `keytool` comes with Java / Android Studio:
   ```powershell
   keytool -genkeypair -keystore kitemail.jks -alias kitemail -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=Kite Mail"
   ```
2. Base64-encode it: PowerShell `[Convert]::ToBase64String([IO.File]::ReadAllBytes("kitemail.jks")) | Set-Clipboard`
   (Linux: `base64 -w0 kitemail.jks`).
3. On GitHub: **Settings → Secrets and variables → Actions → New repository secret**, three times:
   `ANDROID_KEYSTORE_B64` (the base64 text), `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` (`kitemail`).

From then on, every APK is signed with your key and installs over the previous one, keeping your accounts. (The first switch from a `-testkey` build still needs one uninstall.)

## How pairing and QR scanning work
Pairing copies accounts (with sign-ins), contacts, tasks, themes, signatures and the Nextcloud/S3 connection. The data is encrypted with AES-256-GCM using a one-time key that exists only inside the QR code, served for 10 minutes on the local network. The scanner runs inside Kite Mail on every platform: camera preview through the system WebView (`getUserMedia`), QR decoding in Rust (`rqrr`). The camera is on only while the scanner is open.

## Performance notes
- Push mail uses one sleeping IMAP connection per account. The app wakes only when the server reports a change, so idle CPU is close to zero (measured at about 0.03% over 60 seconds).
- The message list only creates DOM rows for what is on screen, so large folders scroll smoothly.
- Animations run only while something is loading. Nothing redraws in the background.
- The Windows installer is about 4 MB (the app about 11 MB), with LTO and size optimisation. Most of the memory use is the system web view.

## Project layout
```
ui/index.html, style.css
ui/js/  icons · themes · core · mail · compose · contacts · tasks · files · archive · settings · qr · sync · main
src-tauri/src/
  lib.rs        app setup, commands, Linux spell-check + camera, mobile entry point
  commands.rs   Tauri commands used by the UI
  mail.rs       IMAP pool, sync, drafts, archive, IDLE, backup/restore (mbox), send pipeline
  push.rs       push-mail threads + task reminders + notifications
  oauth.rs      OAuth 2.0 + PKCE with loopback redirect (Microsoft / Google)
  smtp.rs       message building & sending (lettre)
  mime.rs       parsing, previews, HTML→text, inline images, draft re-open
  grammar.rs    LanguageTool client
  dav.rs        WebDAV/CardDAV/CalDAV client, vCard + VTODO mapping that keeps unknown fields
  nextcloud.rs  contacts/tasks two-way sync, backups with retention, share links
  pair.rs       encrypted one-time device pairing (AES-256-GCM, QR, LAN transfer)
  qr.rs         QR code reading for the scanner
  files.rs      the Files folder (one message per file, server-side dedupe)
  s3.rs         S3 client (SigV4) for backups and attachment links
  net.rs        HTTPS client on the platform TLS stack
  db.rs         SQLite cache, contacts, tasks, settings, local drafts
  secrets.rs    OS keyring with fallback
  it_tests.rs   end-to-end tests against real IMAP/SMTP servers, Nextcloud/DAV, S3, Files folder, conversations
  archive.rs    import tests; the Outlook ones use public samples if present: /tmp/msgsamples (msg-extractor's
                example-msg-files) and /tmp/pstsamples (pst-extractor's enron.pst) – skipped otherwise
                (S3: `moto_server -p 9010` with INITIAL_NO_AUTH_ACTION_COUNT set, so signatures are checked)
```

## Tests
```bash
cd src-tauri
cargo test                       # unit tests (pairing over the network, AWS SigV4 vectors, QR decoding, PST/MSG/MBOX/Maildir import…)
cargo clippy --all-targets -- -D warnings
cargo audit                      # dependency security check (policy: .cargo/audit.toml)
LM_IT=1 cargo test -- --test-threads=1   # adds end-to-end tests against real servers:
                                 #   IMAPS localhost:993 + SMTPS :4650 (Dovecot + aiosmtpd), users tester…tester5@localhost / secret123
                                 #   a second Dovecot on :1993 with the cPanel-style "INBOX." namespace
                                 #   DAV on :8090 – Radicale (CardDAV/CalDAV) + WsgiDAV (files) + share-API mock
                                 #   S3: moto_server on :9010 (INITIAL_NO_AUTH_ACTION_COUNT set, so signatures are checked)
```
