'use strict';
/* Settings: sectioned like Thunderbird – Appearance, Accounts (full per-account settings),
   Composition, Notifications, Privacy & Security, Spam, Cloud & Backup (S3, Nextcloud),
   Devices (pairing), Calling, About. */

let themeEditor = null; // { theme, isNew }
let setSection = 'appearance';
let editAcct = null;    // account id whose settings are open

const SECTIONS = [
  ['appearance', 'palette', 'Appearance'],
  ['accounts', 'inbox', 'Accounts'],
  ['composition', 'pen', 'Composition'],
  ['notifications', 'bell', 'Notifications'],
  ['security', 'shield', 'Privacy & Security'],
  ['spam', 'junk', 'Spam'],
  ['storage', 'cloud', 'Cloud & Backup'],
  ['devices', 'qr', 'Devices'],
  ['calling', 'phone', 'Calling'],
  ['about', 'info', 'About'],
];

function seg(name, value, options) {
  return `<div class="seg" data-seg="${name}">${options.map(([v, l]) => `<button data-v="${esc(v)}" class="${String(value) === String(v) ? 'active' : ''}">${esc(l)}</button>`).join('')}</div>`;
}
function toggle(name, on, label, hint = '') {
  return `<label class="toggle-row"><span><b>${esc(label)}</b>${hint ? `<small>${esc(hint)}</small>` : ''}</span>
    <input type="checkbox" class="switch" data-toggle="${name}" ${on ? 'checked' : ''}/></label>`;
}
/** Toggle bound to an account preference. */
function aptoggle(key, on, label, hint = '') {
  return `<label class="toggle-row"><span><b>${esc(label)}</b>${hint ? `<small>${esc(hint)}</small>` : ''}</span>
    <input type="checkbox" class="switch" data-ap="${key}" ${on ? 'checked' : ''}/></label>`;
}
const kindName = (k) => ({ m365: 'Microsoft 365', gmail: 'Gmail', imap: 'IMAP' }[k] || k);

function renderSettings() {
  if (editAcct && !acct(editAcct)) editAcct = null;
  const page = $('#settings-page');
  const keep = page.querySelector('.set-body')?.scrollTop;
  page.innerHTML = `<div class="set-layout">
    <nav class="set-nav">
      <h2>Settings</h2>
      ${SECTIONS.map(([id, ic, label]) => `<button data-sec="${id}" class="${setSection === id ? 'active' : ''}">${icon(ic, 17)}<span>${label}</span></button>`).join('')}
    </nav>
    <div class="set-body">${sectionHtml(setSection)}</div>
  </div>`;
  if (keep && page.dataset.view === setSection + (editAcct || '')) page.querySelector('.set-body').scrollTop = keep;
  page.dataset.view = setSection + (editAcct || '');
  if (setSection === 'appearance') renderThemeEditor();
  if (setSection === 'storage') { ncWire(); s3Wire(); }
  if (setSection === 'devices') pairWire();
}

function sectionHtml(sec) {
  const st = S.settings;
  switch (sec) {
    case 'appearance': {
      const lights = allThemes().filter((t) => !t.dark);
      const darks = allThemes().filter((t) => t.dark);
      return `<header class="sec-head"><h3>Appearance</h3><p class="muted">Themes, solid colours and motion</p></header>
      <section class="card">
        <div class="set-row"><span>Mode</span>${seg('mode', st.mode, [['system', 'System'], ['light', 'Light'], ['dark', 'Dark']])}</div>
        <h4>Light themes</h4>
        <div class="gallery" data-gallery="light_theme">${lights.map((t) => themeSwatch(t, t.id === st.light_theme)).join('')}</div>
        <h4>Dark themes</h4>
        <div class="gallery" data-gallery="dark_theme">${darks.map((t) => themeSwatch(t, t.id === st.dark_theme)).join('')}</div>
        <p class="hint">Themes marked <em>solid</em> use one flat colour – no gradients or glow (Black, Blue, Charcoal, White…).</p>
        <div class="set-actions">
          <button class="ghost" id="new-theme">${icon('plus', 16)} Create custom theme</button>
          ${currentTheme()?.custom ? `<button class="ghost" id="edit-theme">${icon('edit', 16)} Edit "${esc(currentTheme().name)}"</button>` : ''}
        </div>
        <div id="theme-editor"></div>
      </section>
      <section class="card">
        ${toggle('threads', st.threads, 'Group replies into conversations', 'Replies (including yours from Sent) are shown together, in order')}
        ${toggle('motion', st.motion, 'Animations', 'Slides, fades and transitions. Off = calm, instant interface (also saves battery)')}
        <div class="set-row"><span>Density</span>${seg('density', st.density, [['comfortable', 'Comfortable'], ['compact', 'Compact']])}</div>
        <div class="set-row"><span>Text size</span><input type="range" min="12" max="18" step="1" value="${st.font_size}" id="font-size"/><b id="font-size-v">${st.font_size}px</b></div>
        ${toggle('dark_emails', st.dark_emails, 'Dark email bodies in dark mode', 'Inverts bright email backgrounds while keeping photos natural')}
      </section>`;
    }
    case 'accounts':
      return editAcct ? accountEditorHtml(acct(editAcct)) : accountsListHtml();
    case 'composition':
      return `<header class="sec-head"><h3>Composition</h3><p class="muted">Spelling, grammar and writing style</p></header>
      <section class="card">
        <p class="muted small">Signatures, sending identities, reply position and read-receipt requests are set for each account in
          <a href="#" data-goto="accounts">Accounts</a>.</p>
      </section>
      <section class="card">
        <h4>${icon('type', 16)} Spelling & grammar</h4>
        <p class="muted small">Spelling is checked as you type using your system dictionary. For grammar & style suggestions, Kite Mail uses
          <b>LanguageTool</b> – the free public server, or your own (Docker / Nextcloud LanguageTool app).</p>
        <div class="set-row"><span>LanguageTool server</span><input id="lt-url" value="${esc(st.lt_url)}" placeholder="https://api.languagetool.org"/></div>
        <div class="set-row"><span>Language</span><select id="lt-lang">${[['auto', 'Auto-detect'], ['en-US', 'English (US)'], ['en-GB', 'English (UK)'], ['ar', 'Arabic'], ['fr', 'French'], ['de-DE', 'German'], ['es', 'Spanish'], ['pt-PT', 'Portuguese'], ['nl', 'Dutch'], ['ru-RU', 'Russian']]
          .map(([v, l]) => `<option value="${v}" ${st.lt_lang === v ? 'selected' : ''}>${l}</option>`).join('')}</select></div>
      </section>`;
    case 'notifications':
      return `<header class="sec-head"><h3>Notifications</h3><p class="muted">When new mail arrives or a task is due</p></header>
      <section class="card">
        ${toggle('notify_mail', st.notify_mail, 'New mail', 'Instant (push) notifications for new messages in your inboxes')}
        <div class="set-row"><span>Show</span>${seg('notify_preview', st.notify_preview, [['full', "Sender & subject"], ['sender', 'Sender only'], ['none', 'Just "New mail"']])}</div>
        <div class="set-row"><span>Sound</span>${seg('notify_sound', st.notify_sound, [['default', 'Kite chime'], ['custom', 'Sound file'], ['none', 'Silent']])}
          <button class="ghost" id="sound-play">${icon('volume', 15)} Play</button></div>
        ${st.notify_sound === 'custom' ? `<div class="set-row"><span>Sound file</span><input id="sound-file" value="${esc(st.notify_sound_file)}" readonly placeholder="Choose an .mp3, .ogg or .wav file"/>
          <button class="ghost" id="sound-choose">Browse…</button></div>` : ''}
      </section>
      <section class="card">
        ${toggle('notify_todos', st.notify_todos, 'Task reminders', 'Alert when a task is due')}
      </section>`;
    case 'security':
      return `<header class="sec-head"><h3>Privacy & Security</h3><p class="muted">Tracking, scams and read receipts</p></header>
      <section class="card">
        <h4>Remote content</h4>
        <div class="set-row"><span>Remote images</span>${seg('remote_images', st.remote_images, [['ask', 'Ask (block trackers)'], ['always', 'Always load']])}</div>
        <p class="hint">Remote images can tell the sender when and where you opened a message.</p>
      </section>
      <section class="card">
        <h4>Scam detection</h4>
        ${toggle('scam_warn', st.scam_warn, 'Warn when reading an email that looks like a scam', 'Flags links whose text shows a different site than where they really go, links to raw IP addresses and look-alike (punycode) domains')}
      </section>
      <section class="card">
        <h4>Return receipts</h4>
        <div class="set-row"><span>When a sender asks for a receipt</span>${seg('receipts_reply', st.receipts_reply, [['ask', 'Ask me'], ['always', 'Always send'], ['never', 'Never send']])}</div>
        <p class="hint">To request receipts for mail you send, turn it on per account (Accounts → Composition) or per message in the compose window.</p>
      </section>`;
    case 'spam':
      return `<header class="sec-head"><h3>Spam</h3><p class="muted">Applies to all accounts</p></header>
      <section class="card">
        <div class="set-row"><span>When marked as spam</span>${seg('spam_action', st.spam_action, [['move', 'Move to the Junk folder'], ['delete', 'Delete immediately']])}</div>
        ${toggle('spam_mark_read', st.spam_mark_read, 'Mark as read when marked as spam')}
        <p class="hint">Use <b>Spam</b> in the message toolbar or press <kbd>J</kbd>. In the Junk folder the same button says <b>Not spam</b> and moves the message back to the Inbox. Your mail server's own spam filter keeps running as usual.</p>
      </section>`;
    case 'storage':
      return `<header class="sec-head"><h3>Cloud & Backup</h3><p class="muted">S3 object storage and Nextcloud</p></header>
      ${s3CardHtml()}
      ${ncCardHtml()}
      <section class="card"><p class="muted small">Backups are zip files with one mbox file per folder – they open in Kite Mail (Accounts → an account → Restore) and in Thunderbird (ImportExportTools NG). Large attachments go to S3 when it is set up for links, otherwise to Nextcloud.</p></section>`;
    case 'devices':
      return `<header class="sec-head"><h3>Devices</h3><p class="muted">Copy your setup between computers and phones</p></header>${pairCardHtml()}`;
    case 'calling': {
      const callMode = !st.call_template ? 'tel' : st.call_template.includes('/webclient') ? '3cx' : 'custom';
      return `<header class="sec-head"><h3>Calling</h3><p class="muted">Dial numbers from contacts and emails</p></header>
      <section class="card">
        <div class="set-row"><span>Call using</span>${seg('callmode', callMode, [['tel', 'Phone app (tel:)'], ['3cx', '3CX Web Client'], ['custom', 'Custom URL']])}</div>
        <div class="set-row" id="call-url-row" ${callMode === 'tel' ? 'hidden' : ''}><span>URL</span>
          <input id="call-url" value="${esc(st.call_template)}" placeholder="https://yourcompany.3cx.ae/webclient/#/call?phone={number}"/></div>
        <p class="hint">${callMode === 'tel' ? 'Uses whichever app handles phone links on this device – the 3CX app, Teams, Phone Link or your phone dialler.' : 'Use {number} where the number should go.'}</p>
        <div class="set-row"><span>Test</span><input id="call-test" placeholder="+971 4 000 0000"/><button class="ghost" id="call-test-btn">${icon('phone', 15)} Call</button></div>
      </section>`;
    }
    default:
      return `<header class="sec-head"><h3>About</h3></header>
      <section class="card">
        <p><b>Kite Mail ${esc(S.version || '')}</b></p>
        <p class="muted small">Tauri + Rust. Your mail, contacts and tasks are stored only on this device, your mail servers and the cloud storage you connect.
          Dependencies are checked against the RustSec security database on every build.</p>
      </section>`;
  }
}

// ------------------------------------------------------------------ accounts
function accountsListHtml() {
  return `<header class="sec-head"><h3>Accounts</h3><p class="muted">Identities, servers, signatures and sync – per account</p></header>
  <div class="acct-list-set">${S.accounts.map((a) => {
    const p = aprefs(a.id);
    const isDef = S.settings.default_account === a.id;
    return `<div class="acct-card" data-id="${a.id}" style="--c:${acctColor(a)}">
      <div class="ac-top"><span class="ring">${avatar(a.name, a.email, 40)}</span>
        <div><b>${esc(p.label || a.name || a.email)}</b> ${isDef ? '<span class="pill">Default</span>' : ''}<small>${esc(a.email)} · ${kindName(a.kind)}${p.identities.length ? ` · ${p.identities.length} other identit${p.identities.length > 1 ? 'ies' : 'y'}` : ''}</small></div>
        <span class="grow"></span>
        <button class="primary" data-a="edit">${icon('settings', 15)} Settings</button>
      </div>
    </div>`;
  }).join('') || '<p class="muted">No accounts yet.</p>'}</div>
  <div class="set-actions"><button class="primary" id="set-add-account">${icon('plus', 16)} Add account</button></div>`;
}

function accountEditorHtml(a) {
  const p = aprefs(a.id);
  const st = S.settings;
  const isDef = st.default_account === a.id;
  const sig = (st.signatures || {})[a.id] || '';
  const oauth = a.kind !== 'imap';
  const ids = p.identities.map((x, i) => `<div class="ident-row" data-i="${i}">
      <input data-id-k="name" value="${esc(x.name)}" placeholder="Name"/>
      <input data-id-k="email" value="${esc(x.email)}" placeholder="alias@company.com"/>
      <input data-id-k="reply_to" value="${esc(x.reply_to || '')}" placeholder="Reply-to (optional)"/>
      <button class="icon danger" data-a="id-del" title="Remove">${icon('trash', 15)}</button></div>`).join('');
  return `<div class="acct-editor" data-id="${a.id}" style="--c:${acctColor(a)}">
  <button class="ghost back-link" data-a="back">${icon('back', 15)} All accounts</button>
  <header class="sec-head ae-head"><span class="ring">${avatar(a.name, a.email, 46)}</span>
    <div><h3>${esc(p.label || a.email)}</h3><p class="muted">${esc(a.email)} · ${kindName(a.kind)}</p></div>
    <span class="grow"></span>
    <button class="ghost" data-a="default" ${isDef ? 'disabled' : ''}>${isDef ? `${icon('check', 15)} Default account` : 'Set as default'}</button>
    <button class="ghost danger" data-a="remove">${icon('trash', 15)} Delete</button>
  </header>

  <section class="card">
    <h4>${icon('inbox', 16)} Account</h4>
    <div class="set-row"><span>Account name</span><input data-ap="label" value="${esc(p.label)}" placeholder="${esc(a.email)}"/></div>
    <div class="set-row"><span>Colour</span><input type="color" data-ap="color" value="${p.color || hslToHex(acctColor(a))}"/>
      <button class="ghost" data-a="color-reset" title="Automatic colour">${icon('refresh', 15)} Automatic</button></div>
  </section>

  <section class="card">
    <h4>${icon('id', 16)} Default identity</h4>
    <p class="hint">The information other people see when they read your messages.</p>
    <div class="set-row"><span>Your name</span><input id="ae-name" value="${esc(a.name)}"/></div>
    <div class="set-row"><span>Email address</span><input value="${esc(a.email)}" readonly/></div>
    <div class="set-row"><span>Reply-to address</span><input data-ap="reply_to" value="${esc(p.reply_to)}" placeholder="Recipients will reply to this other address"/></div>
    <div class="set-row"><span>Organization</span><input data-ap="organization" value="${esc(p.organization)}"/></div>
    <label class="sig">Signature text
      <textarea rows="4" data-sig="${a.id}" placeholder="${p.sig_html ? 'e.g. <b>Your Name</b><br>Title · Company' : 'e.g. Your name · Title · Company · +971 …'}" ${p.sig_file_on ? 'disabled' : ''}>${esc(sig)}</textarea></label>
    <label class="check"><input type="checkbox" data-ap="sig_html" ${p.sig_html ? 'checked' : ''}/> Use HTML (e.g. &lt;b&gt;bold&lt;/b&gt;)</label>
    ${p.sig_html && sig ? `<div class="sig-preview"><small class="muted">Preview</small><iframe sandbox="" srcdoc="${esc(`<style>body{font:14px system-ui;margin:6px;color:#222}</style>${sig}`)}"></iframe></div>` : ''}
    <label class="check"><input type="checkbox" data-ap="sig_file_on" ${p.sig_file_on ? 'checked' : ''}/> Attach the signature from a file instead (text or HTML)</label>
    <div class="set-row" ${p.sig_file_on ? '' : 'hidden'}><span></span><input value="${esc(p.sig_file)}" readonly placeholder="No file chosen"/><button class="ghost" data-a="sig-file">Choose…</button></div>
    <label class="check"><input type="checkbox" data-ap="attach_vcard" ${p.attach_vcard ? 'checked' : ''}/> Attach my vCard to messages</label>
    <div class="set-actions"><span class="grow"></span><button class="primary" data-a="save-identity">${icon('check', 15)} Save</button></div>
  </section>

  <section class="card">
    <h4>${icon('users', 16)} Other identities</h4>
    <p class="hint">Aliases or shared mailboxes you can send from (choose them in the From field). Your mail server must allow sending as these addresses.</p>
    <div class="ident-list">${ids || '<p class="muted small">None yet.</p>'}</div>
    <div class="set-actions"><button class="ghost" data-a="id-add">${icon('plus', 15)} Add identity</button></div>
  </section>

  <section class="card">
    <h4>${icon('server', 16)} Server settings</h4>
    ${oauth ? `<p class="muted small">Signed in with ${kindName(a.kind)} (OAuth). Server details are managed by ${a.kind === 'm365' ? 'Microsoft' : 'Google'}.</p>
      <div class="set-actions"><button class="ghost" data-a="reconnect">${icon('refresh', 15)} Sign in again</button></div>` : `
      <div class="set-row"><span>IMAP server</span><input id="ae-ih" value="${esc(a.imap_host)}"/><input id="ae-ip" type="number" class="port" value="${a.imap_port}"/></div>
      <div class="set-row"><span>SMTP server</span><input id="ae-sh" value="${esc(a.smtp_host)}"/><input id="ae-sp" type="number" class="port" value="${a.smtp_port}"/></div>
      <div class="set-row"><span>SMTP security</span><select id="ae-ss">${[['ssl', 'SSL/TLS'], ['starttls', 'STARTTLS'], ['none', 'None (not recommended)']].map(([v, l]) => `<option value="${v}" ${a.smtp_security === v ? 'selected' : ''}>${l}</option>`).join('')}</select></div>
      <div class="set-row"><span>User name</span><input id="ae-user" value="${esc(a.username)}" autocomplete="off"/></div>
      <div class="set-row"><span>Password</span><input id="ae-pass" type="password" autocomplete="new-password" placeholder="Leave empty to keep the current password"/></div>
      <div class="set-actions"><span class="grow"></span><button class="primary" data-a="save-server">${icon('check', 15)} Test & save</button></div>`}
  </section>

  <section class="card">
    <h4>${icon('send', 16)} Copies & folders</h4>
    <label class="toggle-row"><span><b>Save a copy of sent messages</b><small>In the Sent folder (turn off for Gmail/Microsoft 365, which do it themselves, to avoid duplicates)</small></span>
      <input type="checkbox" class="switch" data-a="save-sent" ${a.save_sent ? 'checked' : ''}/></label>
    ${aptoggle('bcc_self', p.bcc_self, 'Bcc myself', 'Every message you send also arrives in your own inbox')}
  </section>

  <section class="card">
    <h4>${icon('pen', 16)} Composition & addressing</h4>
    ${aptoggle('quote', p.quote, 'Quote the original message when replying')}
    <div class="set-row"><span>Then</span>${seg('ap_reply_position', p.reply_position, [['above', 'Start my reply above the quote'], ['below', 'Start my reply below the quote']])}</div>
    ${aptoggle('request_receipt', p.request_receipt, 'Request a read receipt for every message', 'You can still change it per message')}
  </section>

  <section class="card">
    <h4>${icon('files', 16)} Files folder</h4>
    <p class="hint">A <b>Files</b> folder on your mail server, next to Inbox and Sent, that collects pictures and documents from your mail. Every file is a small message with the file attached, so it syncs to all your devices and also shows in Outlook, Thunderbird or a phone's mail app. Duplicates are skipped.</p>
    ${aptoggle('collect_files', p.collect_files, 'Collect attachments from new mail automatically', 'Logos and tiny pictures inside emails are ignored; files over 25 MB are left in the email')}
    <div class="set-row"><span>Collect</span><div class="kinds">${[['images', 'Pictures'], ['pdf', 'PDF'], ['word', 'Word'], ['excel', 'Excel'], ['powerpoint', 'PowerPoint']]
      .map(([k, l]) => `<label class="check"><input type="checkbox" data-kind="${k}" ${p.collect_kinds.includes(k) ? 'checked' : ''}/> ${l}</label>`).join('')}</div></div>
    <div class="set-actions"><button class="ghost" data-a="collect-recent">${icon('download', 15)} Collect from the last 30 days</button></div>
  </section>

  <section class="card">
    <h4>${icon('refresh', 16)} Synchronization & storage</h4>
    <div class="set-row"><span>Keep in sync</span><input type="number" min="1" max="3650" data-ap="sync_days" value="${p.sync_days}" class="port"/><span class="muted">days of mail, plus the newest</span>
      <input type="number" min="50" max="20000" step="50" data-ap="sync_max" value="${p.sync_max}" class="port"/><span class="muted">messages per folder</span></div>
    ${aptoggle('offline_bodies', p.offline_bodies, 'Keep messages for offline reading', 'Downloads recent message bodies in the background')}
    <h4>Backup</h4>
    <div class="set-actions wrap">
      <button class="ghost" data-a="backup">${icon('download', 15)} Back up to a file</button>
      <button class="ghost" data-a="restore">${icon('upload', 15)} Restore from a file</button>
      ${S.s3?.enabled ? `<button class="ghost" data-a="s3backup">${icon('database', 15)} Back up to S3</button>` : ''}
      ${S.nc?.enabled ? `<button class="ghost" data-a="ncbackup">${icon('cloud', 15)} Back up to Nextcloud</button>` : ''}
    </div>
    <div class="progress" hidden><div class="bar"><i></i></div><small></small></div>
  </section>
  </div>`;
}

/** "hsl(h s% l%)" -> #rrggbb for <input type=color>. */
function hslToHex(c) {
  const m = /hsl\((\d+)\s+(\d+)%\s+(\d+)%\)/.exec(c);
  if (!m) return c.startsWith('#') ? c : '#5b5bf6';
  const [h, s, l] = [Number(m[1]), Number(m[2]) / 100, Number(m[3]) / 100];
  const k = (n) => (n + h / 30) % 12;
  const f = (n) => l - s * Math.min(l, 1 - l) * Math.max(-1, Math.min(k(n) - 3, 9 - k(n), 1));
  return '#' + [f(0), f(8), f(4)].map((x) => Math.round(x * 255).toString(16).padStart(2, '0')).join('');
}

async function saveIdentities(id, list) {
  await saveAccountPrefs(id, { identities: list.filter((x) => x.email || x.name) });
}

async function accountAction(a, act, btn) {
  const id = a.id;
  const card = btn.closest('.acct-card, .acct-editor');
  switch (act) {
    case 'edit': editAcct = id; renderSettings(); $('#settings-page .set-body').scrollTop = 0; return;
    case 'back': editAcct = null; renderSettings(); return;
    case 'reconnect': return reconnect(id);
    case 'remove': if (await removeAccount(id)) { editAcct = null; renderSettings(); } return;
    case 'default': await saveSettings({ default_account: id }); renderSettings(); toast(`${a.email} is now the default account`, 'ok'); return;
    case 'color-reset': await saveAccountPrefs(id, { color: '' }); renderSidebar(); renderSettings(); return;
    case 'backup': case 'restore': case 'ncbackup': case 's3backup': return runBackup(id, act, card);
    case 'id-add': {
      const list = [...aprefs(id).identities, { name: a.name, email: '', reply_to: '', organization: '' }];
      await saveAccountPrefs(id, { identities: list });
      renderSettings();
      $$('.ident-row input[data-id-k="email"]').pop()?.focus();
      return;
    }
    case 'id-del': {
      const i = Number(btn.closest('.ident-row').dataset.i);
      await saveIdentities(id, aprefs(id).identities.filter((_, j) => j !== i));
      renderSettings();
      return;
    }
    case 'collect-recent': {
      try {
        const n = await withBusy('Collecting files from the last 30 days…', () => invoke('files_collect_recent', { accountId: id, days: 30 }));
        toast(n ? `${n} file${n > 1 ? 's' : ''} added to Files` : 'No new files to collect', 'ok');
        reloadFolders(id);
      } catch (e) { toast(errText(e), 'error'); }
      return;
    }
    case 'sig-file': {
      const path = await invoke('pick_one_file', { title: 'Signature file', extensions: ['txt', 'html', 'htm'] });
      if (path) { await saveAccountPrefs(id, { sig_file: path }); renderSettings(); }
      return;
    }
    case 'save-identity': case 'save-server': {
      const args = {
        accountId: id, name: $('#ae-name').value.trim() || a.name,
        imapHost: $('#ae-ih')?.value ?? a.imap_host, imapPort: Number($('#ae-ip')?.value ?? a.imap_port),
        smtpHost: $('#ae-sh')?.value ?? a.smtp_host, smtpPort: Number($('#ae-sp')?.value ?? a.smtp_port),
        smtpSecurity: $('#ae-ss')?.value ?? a.smtp_security, username: $('#ae-user')?.value ?? a.username,
        password: act === 'save-server' ? ($('#ae-pass')?.value || '') : '', saveSent: a.save_sent,
      };
      try {
        const upd = await withBusy(act === 'save-server' ? 'Testing the connection…' : 'Saving…', () => invoke('update_account', args));
        Object.assign(a, upd);
        renderSidebar();
        renderSettings();
        toast(act === 'save-server' ? 'Connected – server settings saved' : 'Saved', 'ok');
        if (act === 'save-server') refreshAccount(id);
      } catch (e) { toast(errText(e), 'error'); }
      return;
    }
  }
}

// ------------------------------------------------------------------ S3
async function loadS3() {
  try { S.s3 = await invoke('s3_get_config'); } catch (_) { S.s3 = { enabled: false }; }
}

const S3_PRESETS = {
  aws: { label: 'Amazon S3', endpoint: '', region: 'me-central-1', path: false, hint: 'Leave the endpoint empty for AWS. Region e.g. me-central-1 (UAE), eu-west-1.' },
  oci: { label: 'Oracle Cloud', endpoint: 'https://NAMESPACE.compat.objectstorage.me-dubai-1.oraclecloud.com', region: 'me-dubai-1', path: true, hint: 'Use a Customer Secret Key (Profile → Customer secret keys). NAMESPACE is shown in Tenancy details.' },
  r2: { label: 'Cloudflare R2', endpoint: 'https://ACCOUNT_ID.r2.cloudflarestorage.com', region: 'auto', path: true, hint: 'Create an R2 API token with Object Read & Write.' },
  wasabi: { label: 'Wasabi', endpoint: 'https://s3.eu-central-1.wasabisys.com', region: 'eu-central-1', path: false, hint: '' },
  minio: { label: 'MinIO / other', endpoint: 'http://nas.local:9000', region: 'us-east-1', path: true, hint: 'Any S3-compatible server (MinIO, Ceph, Synology C2, Backblaze B2…).' },
};
let s3Preset = 'aws';

function s3CardHtml() {
  const c = S.s3 || {};
  if (!c.enabled) {
    const pr = S3_PRESETS[s3Preset];
    return `<section class="card s3-card">
      <h3>${icon('database', 18)} S3 object storage</h3>
      <p class="muted small">Back up mailboxes to a bucket on a schedule and send large attachments as secure download links that expire. Works with Amazon S3, Oracle Cloud Object Storage, Cloudflare R2, Wasabi, MinIO and other S3-compatible storage.</p>
      <div class="set-row"><span>Provider</span><select id="s3-preset">${Object.entries(S3_PRESETS).map(([k, v]) => `<option value="${k}" ${k === s3Preset ? 'selected' : ''}>${v.label}</option>`).join('')}</select></div>
      <div class="set-row"><span>Endpoint</span><input id="s3-endpoint" value="${esc(c.endpoint || '')}" placeholder="${esc(pr.endpoint || 'Empty = Amazon S3')}"/></div>
      <div class="set-row"><span>Region</span><input id="s3-region" value="${esc(c.region && c.region !== 'us-east-1' ? c.region : pr.region)}"/></div>
      <div class="set-row"><span>Bucket</span><input id="s3-bucket" value="${esc(c.bucket || '')}" placeholder="kite-mail-backups"/></div>
      <div class="set-row"><span>Access key</span><input id="s3-ak" value="${esc(c.access_key || '')}" autocomplete="off"/></div>
      <div class="set-row"><span>Secret key</span><input id="s3-sk" type="password" autocomplete="new-password" placeholder="${c.has_secret ? 'Leave empty to keep the saved key' : ''}"/></div>
      <details class="adv"><summary>Advanced</summary>
        <div class="set-row"><span>Folder in bucket</span><input id="s3-prefix" value="${esc(c.prefix ?? 'kitemail')}"/></div>
        <label class="check"><input type="checkbox" id="s3-path" ${(c.bucket ? c.path_style : pr.path) ? 'checked' : ''}/> Path-style URLs (endpoint/bucket/…)</label>
      </details>
      ${pr.hint ? `<p class="hint">${esc(pr.hint)}</p>` : ''}
      <div class="set-actions"><span class="grow"></span><button class="primary" id="s3-connect">${icon('database', 16)} Test & connect</button></div>
    </section>`;
  }
  const last = Object.values(c.last_backup || {}).reduce((m, t) => Math.max(m, t), 0);
  return `<section class="card s3-card">
    <h3>${icon('database', 18)} S3 object storage <span class="pill ok">Connected</span></h3>
    <p class="muted small">Bucket <b>${esc(c.bucket)}</b> · ${esc(c.endpoint || 'Amazon S3 ' + c.region)}${c.prefix ? ` · folder ${esc(c.prefix)}/` : ''}<br>
      Last backup: ${last ? ago(last) : 'never'}${c.last_error ? ` · <span class="danger">${esc(c.last_error)}</span>` : ''}</p>
    <h4>Mailbox backups</h4>
    <div class="set-row"><span>Automatic backup</span>${seg('s3_backup_schedule', c.backup_schedule, [['off', 'Off'], ['daily', 'Daily'], ['weekly', 'Weekly']])}</div>
    <div class="set-row"><span>Keep</span><input type="number" min="1" max="365" id="s3-keep" value="${c.backup_keep}" class="port"/><span class="muted">latest backups per account</span></div>
    <h4>Large attachments</h4>
    ${toggle('s3_links', c.links, 'Send large attachments as download links', 'Uploaded to the bucket; recipients get a private link that expires')}
    <div class="set-row"><span>Files from</span><input type="number" min="1" max="2000" id="s3-mb" value="${c.link_threshold_mb}" class="port"/><span class="muted">MB · links expire after</span>
      <input type="number" min="1" max="7" id="s3-days" value="${c.link_expire_days}" class="port"/><span class="muted">days (max 7)</span></div>
    <div class="set-actions"><button class="ghost" id="s3-backup-all">${icon('upload', 15)} Back up all accounts now</button><span class="grow"></span>
      <button class="ghost" id="s3-change">Change bucket</button><button class="ghost danger" id="s3-disconnect">Disconnect</button></div>
    <div class="progress" hidden><div class="bar"><i></i></div><small></small></div>
  </section>`;
}

function s3Wire() {
  const card = $('.s3-card');
  if (!card) return;
  const preset = $('#s3-preset');
  if (preset) preset.onchange = () => { s3Preset = preset.value; renderSettings(); };
  const conn = $('#s3-connect');
  if (conn) conn.onclick = async () => {
    try {
      await withBusy('Testing the bucket…', () => invoke('s3_connect', {
        endpoint: $('#s3-endpoint').value, region: $('#s3-region').value, bucket: $('#s3-bucket').value,
        accessKey: $('#s3-ak').value, secretKey: $('#s3-sk').value, prefix: $('#s3-prefix').value, pathStyle: $('#s3-path').checked,
      }));
      await loadS3();
      renderSettings();
      toast('S3 connected', 'ok');
    } catch (e) { toast(errText(e), 'error'); }
  };
  const num = (id, key, lo, hi) => { const el = $(id); if (el) el.onchange = () => s3Save({ [key]: Math.min(hi, Math.max(lo, Number(el.value) || lo)) }); };
  num('#s3-keep', 'backup_keep', 1, 365);
  num('#s3-mb', 'link_threshold_mb', 1, 2000);
  num('#s3-days', 'link_expire_days', 1, 7);
  const ch = $('#s3-change');
  if (ch) ch.onclick = () => { S.s3.enabled = false; renderSettings(); };
  const dis = $('#s3-disconnect');
  if (dis) dis.onclick = async () => {
    if (!(await confirmBox('Disconnect S3? Backups already in the bucket stay there.', 'Disconnect', true))) return;
    await invoke('s3_disconnect'); await loadS3(); renderSettings();
  };
  const all = $('#s3-backup-all');
  if (all) all.onclick = async () => {
    for (const a of S.accounts) await runBackup(a.id, 's3backup', card);
    await loadS3();
    renderSettings();
  };
}

async function s3Save(patch) {
  try { await invoke('s3_save_options', { options: patch }); await loadS3(); } catch (e) { toast(errText(e), 'error'); }
}

// ------------------------------------------------------------------ notification sound
let soundCache = { path: '', url: '' };
async function playNotifySound(force = false) {
  const st = S.settings;
  if (st.notify_sound === 'none' && !force) return;
  try {
    if (st.notify_sound === 'custom' && st.notify_sound_file) {
      if (soundCache.path !== st.notify_sound_file) soundCache = { path: st.notify_sound_file, url: await invoke('file_data_url', { path: st.notify_sound_file }) };
      await new Audio(soundCache.url).play();
      return;
    }
    // built-in two-note chime (no sound file shipped)
    const ctx = new (window.AudioContext || window.webkitAudioContext)();
    [[880, 0], [1320, 0.12]].forEach(([f, t]) => {
      const o = ctx.createOscillator(), g = ctx.createGain();
      o.type = 'sine'; o.frequency.value = f;
      g.gain.setValueAtTime(0.0001, ctx.currentTime + t);
      g.gain.exponentialRampToValueAtTime(0.18, ctx.currentTime + t + 0.02);
      g.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + t + 0.35);
      o.connect(g).connect(ctx.destination);
      o.start(ctx.currentTime + t); o.stop(ctx.currentTime + t + 0.4);
    });
    setTimeout(() => ctx.close(), 900);
  } catch (e) { if (force) toast(errText(e), 'error'); }
}

function renderThemeEditor() {
  const box = $('#theme-editor');
  if (!box) return;
  if (!themeEditor) { box.innerHTML = ''; return; }
  const t = themeEditor.theme;
  box.innerHTML = `<div class="theme-editor">
    <div class="te-row"><label>Name<input id="te-name" value="${esc(t.name)}"/></label>
      <label class="row"><input type="checkbox" id="te-dark" ${t.dark ? 'checked' : ''}/> Dark theme</label>
      <label class="row"><input type="checkbox" id="te-solid" ${t.solid ? 'checked' : ''}/> Solid (no gradient)</label></div>
    <div class="te-colors">${THEME_KEYS.map(([k, l]) => `<label><input type="color" data-k="${k}" value="${t.c[k]}"/><span>${esc(l)}</span></label>`).join('')}</div>
    <div class="set-actions">
      ${themeEditor.isNew ? '' : `<button class="ghost danger" id="te-del">${icon('trash', 15)} Delete</button>`}
      <span class="grow"></span><button class="ghost" id="te-cancel">Cancel</button><button class="primary" id="te-save">${icon('check', 15)} Save theme</button>
    </div></div>`;
  box.querySelectorAll('input[type=color]').forEach((inp) => inp.addEventListener('input', () => { t.c[inp.dataset.k] = inp.value; applyTheme(t, true); }));
  $('#te-dark').onchange = (e) => { t.dark = e.target.checked; applyTheme(t, true); };
  $('#te-solid').onchange = (e) => { t.solid = e.target.checked; if (t.solid) t.c.accent2 = t.c.accent; applyTheme(t, true); };
  $('#te-cancel').onclick = () => { themeEditor = null; applyTheme(); renderSettings(); };
  $('#te-save').onclick = async () => {
    t.name = $('#te-name').value.trim() || 'My theme';
    const list = (S.settings.custom_themes || []).filter((x) => x.id !== t.id);
    list.push(t);
    const patch = { custom_themes: list };
    if (t.dark) { patch.dark_theme = t.id; } else { patch.light_theme = t.id; }
    patch.mode = t.dark ? 'dark' : 'light';
    await saveSettings(patch);
    themeEditor = null;
    applyTheme();
    renderSettings();
    toast('Theme saved', 'ok');
  };
  const del = $('#te-del');
  if (del) del.onclick = async () => {
    const list = (S.settings.custom_themes || []).filter((x) => x.id !== t.id);
    const patch = { custom_themes: list };
    if (S.settings.light_theme === t.id) patch.light_theme = 'aurora';
    if (S.settings.dark_theme === t.id) patch.dark_theme = 'midnight';
    await saveSettings(patch);
    themeEditor = null;
    applyTheme();
    renderSettings();
  };
}

function refreshThemeButton() {
  const dark = document.documentElement.dataset.dark === '1';
  $('#btn-theme').innerHTML = `${icon(dark ? 'sun' : 'moon', 22)}<span>${dark ? 'Light' : 'Dark'}</span>`;
}

// ------------------------------------------------------------------ events (delegated)
$('#settings-page').addEventListener('click', async (e) => {
  const nav = e.target.closest('.set-nav [data-sec]');
  if (nav) { setSection = nav.dataset.sec; editAcct = null; themeEditor = null; renderSettings(); $('#settings-page .set-body').scrollTop = 0; return; }
  const go = e.target.closest('[data-goto]');
  if (go) { e.preventDefault(); setSection = go.dataset.goto; renderSettings(); return; }
  const sw = e.target.closest('.swatch');
  if (sw) {
    const gallery = sw.closest('[data-gallery]').dataset.gallery;
    const t = allThemes().find((x) => x.id === sw.dataset.theme);
    const patch = { [gallery]: t.id };
    if (S.settings.mode !== 'system') patch.mode = t.dark ? 'dark' : 'light';
    await saveSettings(patch);
    applyTheme();
    renderSettings();
    return;
  }
  const segBtn = e.target.closest('.seg button');
  if (segBtn) {
    const name = segBtn.closest('[data-seg]').dataset.seg;
    const v = segBtn.dataset.v;
    if (name.startsWith('nc_')) { await ncSaveOption({ [name.slice(3)]: v }); renderSettings(); return; }
    if (name.startsWith('s3_')) { await s3Save({ [name.slice(3)]: v }); renderSettings(); return; }
    if (name.startsWith('ap_')) { await saveAccountPrefs(editAcct, { [name.slice(3)]: v }); renderSettings(); return; }
    if (name === 'callmode') {
      const tpl = v === 'tel' ? '' : v === '3cx' ? (S.settings.call_template.includes('/webclient') ? S.settings.call_template : 'https://YOURPBX.3cx.ae/webclient/#/call?phone={number}') : (S.settings.call_template || 'https://');
      await saveSettings({ call_template: tpl });
    } else {
      await saveSettings({ [name]: v });
      if (name === 'mode' || name === 'density') applyTheme();
      if (name === 'density') renderList();
    }
    renderSettings();
    return;
  }
  if (e.target.closest('#new-theme')) {
    const base = currentTheme();
    themeEditor = { isNew: true, theme: { id: 'custom-' + Date.now(), name: 'My theme', dark: base.dark, solid: !!base.solid, custom: true, c: { ...base.c } } };
    renderThemeEditor();
    $('#theme-editor').scrollIntoView({ block: 'center' });
    return;
  }
  if (e.target.closest('#edit-theme')) {
    themeEditor = { isNew: false, theme: JSON.parse(JSON.stringify(currentTheme())) };
    renderThemeEditor();
    return;
  }
  if (e.target.closest('#set-add-account')) return openAccountDialog();
  if (e.target.closest('#call-test-btn')) {
    return invoke('call_number', { number: $('#call-test').value }).catch((err) => toast(errText(err), 'error'));
  }
  if (e.target.closest('#sound-play')) return playNotifySound(true);
  if (e.target.closest('#sound-choose')) {
    const path = await invoke('pick_one_file', { title: 'Notification sound', extensions: ['mp3', 'ogg', 'wav', 'm4a', 'flac'] });
    if (path) { await saveSettings({ notify_sound_file: path }); renderSettings(); playNotifySound(true); }
    return;
  }
  const ab = e.target.closest('[data-a]');
  if (ab && ab.type !== 'checkbox') {
    const host = ab.closest('[data-id]');
    const a = host && acct(Number(host.dataset.id));
    if (a) accountAction(a, ab.dataset.a, ab);
  }
});

$('#settings-page').addEventListener('change', async (e) => {
  const t = e.target;
  if (t.dataset.toggle?.startsWith('nc_')) { await ncSaveOption({ [t.dataset.toggle.slice(3)]: t.checked }); queueNcSync(); return; }
  if (t.dataset.toggle?.startsWith('s3_')) { await s3Save({ [t.dataset.toggle.slice(3)]: t.checked }); return; }
  if (t.dataset.toggle) {
    await saveSettings({ [t.dataset.toggle]: t.checked });
    if (t.dataset.toggle === 'dark_emails') renderBody();
    if (t.dataset.toggle === 'motion') applyTheme();
    if (t.dataset.toggle === 'threads') renderList();
    return;
  }
  // account preference fields
  if (t.dataset.ap && editAcct) {
    const k = t.dataset.ap;
    const v = t.type === 'checkbox' ? t.checked : t.type === 'number' ? Number(t.value) || ACCOUNT_PREF_DEFAULTS[k] : t.value.trim();
    await saveAccountPrefs(editAcct, { [k]: v });
    if (['label', 'color'].includes(k)) renderSidebar();
    if (['label', 'color', 'sig_html', 'sig_file_on'].includes(k)) renderSettings();
    return;
  }
  if (t.dataset.kind && editAcct) {
    const kinds = new Set(aprefs(editAcct).collect_kinds);
    if (t.checked) kinds.add(t.dataset.kind); else kinds.delete(t.dataset.kind);
    await saveAccountPrefs(editAcct, { collect_kinds: [...kinds] });
    return;
  }
  if (t.dataset.idK && editAcct) {
    const list = aprefs(editAcct).identities.map((x) => ({ ...x }));
    const i = Number(t.closest('.ident-row').dataset.i);
    list[i][t.dataset.idK] = t.value.trim();
    if (t.dataset.idK === 'email' && t.value && !/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(t.value.trim())) toast(`"${t.value}" doesn't look like an email address`, 'error');
    await saveIdentities(editAcct, list);
    return;
  }
  if (t.dataset.a === 'save-sent' && editAcct) {
    const a = acct(editAcct);
    try {
      Object.assign(a, await invoke('update_account', { accountId: a.id, name: a.name, imapHost: a.imap_host, imapPort: a.imap_port, smtpHost: a.smtp_host, smtpPort: a.smtp_port, smtpSecurity: a.smtp_security, username: a.username, password: '', saveSent: t.checked }));
    } catch (err) { toast(errText(err), 'error'); t.checked = !t.checked; }
    return;
  }
  const ncNum = { 'nc-bkeep': 'backup_keep', 'nc-lmb': 'link_threshold_mb', 'nc-lexp': 'link_expire_days' }[t.id];
  if (ncNum) return ncSaveOption({ [ncNum]: Math.max(0, Number(t.value) || 0) });
  if (t.id === 'nc-bfolder') return ncSaveOption({ backup_folder: t.value.trim() || 'Kite Mail Backups' });
  if (t.id === 'nc-tasklist') { await ncSaveOption({ tasks_url: t.value }); queueNcSync(); return; }
  if (t.dataset.sig) {
    await saveSettings({ signatures: { ...(S.settings.signatures || {}), [t.dataset.sig]: t.value.trim() } });
    toast('Signature saved', 'ok');
    if (aprefs(Number(t.dataset.sig)).sig_html) renderSettings();
    return;
  }
  if (t.id === 'lt-url') saveSettings({ lt_url: t.value.trim() });
  if (t.id === 'lt-lang') saveSettings({ lt_lang: t.value });
  if (t.id === 'call-url') saveSettings({ call_template: t.value.trim() });
  if (t.id === 'font-size') { await saveSettings({ font_size: Number(t.value) }); applyTheme(); }
});
$('#settings-page').addEventListener('input', (e) => {
  if (e.target.id === 'font-size') { $('#font-size-v').textContent = e.target.value + 'px'; document.documentElement.style.fontSize = e.target.value + 'px'; }
});

// ------------------------------------------------------------------ backup / restore
let backupCard = null;
async function runBackup(accountId, kind, card) {
  if (kind === 'restore' && !(await confirmBox('Restore uploads every message from a Kite Mail backup into this account. Messages that already exist are skipped. Continue?', 'Choose backup file'))) return;
  backupCard = card;
  const prog = $('.progress', card);
  card.querySelectorAll('button[data-a]').forEach((b) => (b.disabled = true));
  try {
    if (kind === 'ncbackup') {
      const dest = await invoke('nc_backup_now', { accountId });
      toast(`Backed up to Nextcloud: ${dest}`, 'ok');
      return;
    }
    if (kind === 's3backup') {
      const dest = await invoke('s3_backup_now', { accountId });
      toast(`Backed up to S3: ${dest}`, 'ok');
      await loadS3();
      return;
    }
    const r = await invoke(kind === 'backup' ? 'backup_account' : 'restore_account', { accountId });
    if (r) toast(kind === 'backup' ? `Backup complete: ${r.count} messages saved` : `Restore complete: ${r.added} added, ${r.skipped} already present`, 'ok');
  } catch (e) { toast(errText(e), 'error'); }
  finally {
    card.querySelectorAll('button[data-a]').forEach((b) => (b.disabled = false));
    prog.hidden = true;
    backupCard = null;
    if (kind === 'restore') reloadFolders(accountId);
  }
}

function onBackupProgress(p) {
  if (!backupCard) return;
  const prog = $('.progress', backupCard);
  prog.hidden = p.phase === 'done';
  const pct = p.total ? Math.round((p.done / p.total) * 100) : 0;
  $('.bar i', prog).style.width = pct + '%';
  $('small', prog).textContent = `${p.phase === 'restore' ? 'Restoring' : 'Backing up'} ${p.folder}: ${p.done} / ${p.total}`;
}
