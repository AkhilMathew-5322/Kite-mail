'use strict';
/* Settings page: appearance & themes (with custom theme editor), accounts (signature, backup/restore),
   notifications, writing (LanguageTool), calling (3CX / tel:), privacy, integrations roadmap. */

let themeEditor = null; // { theme, isNew }

function seg(name, value, options) {
  return `<div class="seg" data-seg="${name}">${options.map(([v, l]) => `<button data-v="${esc(v)}" class="${String(value) === String(v) ? 'active' : ''}">${esc(l)}</button>`).join('')}</div>`;
}
function toggle(name, on, label, hint = '') {
  return `<label class="toggle-row"><span><b>${esc(label)}</b>${hint ? `<small>${esc(hint)}</small>` : ''}</span>
    <input type="checkbox" class="switch" data-toggle="${name}" ${on ? 'checked' : ''}/></label>`;
}

function renderSettings() {
  const st = S.settings;
  const lights = allThemes().filter((t) => !t.dark);
  const darks = allThemes().filter((t) => t.dark);
  const callMode = !st.call_template ? 'tel' : st.call_template.includes('/webclient') ? '3cx' : 'custom';
  const page = $('#settings-page');
  page.innerHTML = `
  <header class="page-head"><div><h2>Settings</h2><p class="muted">Make Kite Mail yours</p></div></header>

  <section class="card">
    <h3>${icon('palette', 18)} Appearance</h3>
    <div class="set-row"><span>Mode</span>${seg('mode', st.mode, [['system', 'System'], ['light', 'Light'], ['dark', 'Dark']])}</div>
    <h4>Light theme</h4>
    <div class="gallery" data-gallery="light_theme">${lights.map((t) => themeSwatch(t, t.id === st.light_theme)).join('')}</div>
    <h4>Dark theme</h4>
    <div class="gallery" data-gallery="dark_theme">${darks.map((t) => themeSwatch(t, t.id === st.dark_theme)).join('')}</div>
    <div class="set-actions">
      <button class="ghost" id="new-theme">${icon('plus', 16)} Create custom theme</button>
      ${currentTheme()?.custom ? `<button class="ghost" id="edit-theme">${icon('edit', 16)} Edit "${esc(currentTheme().name)}"</button>` : ''}
    </div>
    <div id="theme-editor"></div>
    <div class="set-row"><span>Density</span>${seg('density', st.density, [['comfortable', 'Comfortable'], ['compact', 'Compact']])}</div>
    <div class="set-row"><span>Text size</span><input type="range" min="12" max="18" step="1" value="${st.font_size}" id="font-size"/><b id="font-size-v">${st.font_size}px</b></div>
    ${toggle('dark_emails', st.dark_emails, 'Dark email bodies in dark mode', 'Inverts bright email backgrounds while keeping photos natural')}
  </section>

  <section class="card">
    <h3>${icon('inbox', 18)} Accounts</h3>
    <div id="acct-cards">${S.accounts.map((a) => `
      <div class="acct-card" data-id="${a.id}">
        <div class="ac-top">${avatar(a.name, a.email, 40)}<div><b>${esc(a.name)}</b><small>${esc(a.email)} · ${{ m365: 'Microsoft 365', gmail: 'Gmail', imap: 'IMAP' }[a.kind]}</small></div>
          <span class="grow"></span>
          ${a.kind !== 'imap' ? `<button class="ghost" data-a="reconnect">${icon('refresh', 15)} Reconnect</button>` : ''}
          <button class="ghost" data-a="backup">${icon('download', 15)} Backup</button>
          <button class="ghost" data-a="restore">${icon('upload', 15)} Restore</button>
          <button class="ghost danger" data-a="remove">${icon('trash', 15)}</button>
        </div>
        <label class="sig">Signature<textarea rows="3" data-sig="${a.id}" placeholder="e.g. Your name · Title · Company · +971 …">${esc((st.signatures || {})[a.id] || '')}</textarea></label>
        <div class="progress" hidden><div class="bar"><i></i></div><small></small></div>
      </div>`).join('') || '<p class="muted">No accounts yet.</p>'}</div>
    <button class="primary" id="set-add-account">${icon('plus', 16)} Add account</button>
  </section>

  <section class="card">
    <h3>${icon('bell', 18)} Notifications</h3>
    ${toggle('notify_mail', st.notify_mail, 'New mail', 'Instant (push) notifications for new messages in your inboxes')}
    ${toggle('notify_todos', st.notify_todos, 'Task reminders', 'Alert when a task is due')}
  </section>

  <section class="card">
    <h3>${icon('type', 18)} Writing</h3>
    <p class="muted small">Spelling is checked as you type using your system dictionary. For grammar & style suggestions, Kite Mail uses
      <b>LanguageTool</b> – the free public server, or your own (Docker / Nextcloud LanguageTool app).</p>
    <div class="set-row"><span>LanguageTool server</span><input id="lt-url" value="${esc(st.lt_url)}" placeholder="https://api.languagetool.org"/></div>
    <div class="set-row"><span>Language</span><select id="lt-lang">${[['auto', 'Auto-detect'], ['en-US', 'English (US)'], ['en-GB', 'English (UK)'], ['ar', 'Arabic'], ['fr', 'French'], ['de-DE', 'German'], ['es', 'Spanish'], ['pt-PT', 'Portuguese'], ['nl', 'Dutch'], ['ru-RU', 'Russian']]
      .map(([v, l]) => `<option value="${v}" ${st.lt_lang === v ? 'selected' : ''}>${l}</option>`).join('')}</select></div>
  </section>

  <section class="card">
    <h3>${icon('phone', 18)} Calling</h3>
    <p class="muted small">Phone numbers in contacts and email links can be dialled straight from Kite Mail.</p>
    <div class="set-row"><span>Call using</span>${seg('callmode', callMode, [['tel', 'Phone app (tel:)'], ['3cx', '3CX Web Client'], ['custom', 'Custom URL']])}</div>
    <div class="set-row" id="call-url-row" ${callMode === 'tel' ? 'hidden' : ''}><span>URL</span>
      <input id="call-url" value="${esc(st.call_template)}" placeholder="https://yourcompany.3cx.ae/webclient/#/call?phone={number}"/></div>
    <p class="hint">${callMode === 'tel' ? 'Uses whichever app handles phone links on this device – the 3CX app, Teams, Phone Link or your phone dialler.' : 'Use {number} where the number should go.'}</p>
    <div class="set-row"><span>Test</span><input id="call-test" placeholder="+971 4 000 0000"/><button class="ghost" id="call-test-btn">${icon('phone', 15)} Call</button></div>
  </section>

  <section class="card">
    <h3>${icon('shield', 18)} Privacy</h3>
    <div class="set-row"><span>Remote images</span>${seg('remote_images', st.remote_images, [['ask', 'Ask (block trackers)'], ['always', 'Always load']])}</div>
  </section>

  <section class="card soon">
    <h3>${icon('cloud', 18)} Integrations <span class="pill">Phase 2</span></h3>
    <div class="soon-grid">
      <div>${icon('cloud', 22)}<b>Nextcloud</b><small>Contacts (CardDAV) & tasks (CalDAV) sync, backups to Nextcloud, large attachments as share links</small></div>
      <div>${icon('qr', 22)}<b>Pair your phone</b><small>Scan a QR code in the Android app to copy accounts, contacts, tasks and themes</small></div>
    </div>
  </section>

  <section class="card">
    <h3>${icon('info', 18)} About</h3>
    <p class="muted small">Kite Mail 0.2.0 · Tauri + Rust · your mail, contacts and tasks are stored only on this device and your mail servers.</p>
  </section>`;
  renderThemeEditor();
}

function renderThemeEditor() {
  const box = $('#theme-editor');
  if (!box) return;
  if (!themeEditor) { box.innerHTML = ''; return; }
  const t = themeEditor.theme;
  box.innerHTML = `<div class="theme-editor">
    <div class="te-row"><label>Name<input id="te-name" value="${esc(t.name)}"/></label>
      <label class="row"><input type="checkbox" id="te-dark" ${t.dark ? 'checked' : ''}/> Dark theme</label></div>
    <div class="te-colors">${THEME_KEYS.map(([k, l]) => `<label><input type="color" data-k="${k}" value="${t.c[k]}"/><span>${esc(l)}</span></label>`).join('')}</div>
    <div class="set-actions">
      ${themeEditor.isNew ? '' : `<button class="ghost danger" id="te-del">${icon('trash', 15)} Delete</button>`}
      <span class="grow"></span><button class="ghost" id="te-cancel">Cancel</button><button class="primary" id="te-save">${icon('check', 15)} Save theme</button>
    </div></div>`;
  box.querySelectorAll('input[type=color]').forEach((inp) => inp.addEventListener('input', () => { t.c[inp.dataset.k] = inp.value; applyTheme(t, true); }));
  $('#te-dark').onchange = (e) => { t.dark = e.target.checked; applyTheme(t, true); };
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
    themeEditor = { isNew: true, theme: { id: 'custom-' + Date.now(), name: 'My theme', dark: base.dark, custom: true, c: { ...base.c } } };
    renderThemeEditor();
    $('#theme-editor').scrollIntoView({ behavior: 'smooth', block: 'center' });
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
  const ab = e.target.closest('[data-a]');
  if (ab) {
    const card = ab.closest('.acct-card');
    const id = Number(card.dataset.id);
    const a = ab.dataset.a;
    if (a === 'reconnect') reconnect(id);
    if (a === 'remove' && (await removeAccount(id))) renderSettings();
    if (a === 'backup' || a === 'restore') runBackup(id, a, card);
  }
});

$('#settings-page').addEventListener('change', async (e) => {
  const t = e.target;
  if (t.dataset.toggle) { await saveSettings({ [t.dataset.toggle]: t.checked }); if (t.dataset.toggle === 'dark_emails') renderBody(); }
  if (t.dataset.sig) { await saveSettings({ signatures: { ...(S.settings.signatures || {}), [t.dataset.sig]: t.value.trim() } }); toast('Signature saved', 'ok'); }
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
  card.querySelectorAll('[data-a]').forEach((b) => (b.disabled = true));
  try {
    const r = await invoke(kind === 'backup' ? 'backup_account' : 'restore_account', { accountId });
    if (r) toast(kind === 'backup' ? `Backup complete: ${r.count} messages saved` : `Restore complete: ${r.added} added, ${r.skipped} already present`, 'ok');
  } catch (e) { toast(errText(e), 'error'); }
  finally {
    card.querySelectorAll('[data-a]').forEach((b) => (b.disabled = false));
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
