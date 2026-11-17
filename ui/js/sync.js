'use strict';
/* Phase 2: Nextcloud settings, phone pairing (QR), mobile gestures & back button. */

S.nc = { enabled: false };
S.mobile = false;

async function loadNc() {
  try { S.nc = await invoke('nc_get_config'); } catch (_) { S.nc = { enabled: false }; }
}

const queueNcSync = debounce(() => {
  if (S.nc.enabled) invoke('nc_sync_now').catch(() => {});
}, 4000);

function ago(ts) {
  if (!ts) return 'never';
  const s = nowSec() - ts;
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return fmtDate(ts, true);
}

// ------------------------------------------------------------------ Nextcloud card
function ncCardHtml() {
  const c = S.nc;
  if (!c.enabled) {
    return `<section class="card nc-card">
      <h3>${icon('cloud', 18)} Nextcloud</h3>
      <p class="muted small">Sync contacts and tasks with Nextcloud (and your phone), back up mail to a Nextcloud folder, and send large attachments as share links.
        Create an <b>app password</b> in Nextcloud → Settings → Security → Devices & sessions.</p>
      <div class="set-row"><span>Server</span><input id="nc-server" value="${esc(c.server || '')}" placeholder="https://cloud.yourcompany.com"/></div>
      <div class="set-row"><span>Username</span><input id="nc-user" value="${esc(c.user || '')}" autocomplete="off"/></div>
      <div class="set-row"><span>App password</span><input id="nc-pass" type="password" autocomplete="off" placeholder="xxxxx-xxxxx-xxxxx-xxxxx-xxxxx"/></div>
      <details class="adv"><summary>Other DAV server (Radicale, Baïkal, SOGo…)</summary>
        <div class="set-row"><span>Contacts URL</span><input id="nc-curl" value="${esc(c.contacts_url || '')}" placeholder="https://dav.example.com/user/contacts/"/></div>
        <div class="set-row"><span>Tasks URL</span><input id="nc-turl" value="${esc(c.tasks_url || '')}" placeholder="https://dav.example.com/user/tasks/"/></div>
      </details>
      <div class="set-actions"><span class="grow"></span><button class="primary" id="nc-connect">${icon('cloud', 16)} Connect</button></div>
      <p class="small" id="nc-msg"></p>
    </section>`;
  }
  return `<section class="card nc-card">
    <h3>${icon('cloud', 18)} Nextcloud <span class="pill ok">Connected</span></h3>
    <p class="muted small">${esc(c.user)} @ ${esc(c.server || c.contacts_url)} · last sync ${ago(c.last_sync)}
      ${c.last_error ? `<br><span class="danger">${icon('junk', 13)} ${esc(c.last_error)}</span>` : ''}</p>
    ${toggle('nc_sync_contacts', c.sync_contacts, 'Sync contacts (CardDAV)', 'Two-way with Nextcloud Contacts – and any phone synced to it')}
    ${toggle('nc_sync_tasks', c.sync_tasks, 'Sync tasks (CalDAV)', 'Two-way with Nextcloud Tasks, including due-date reminders')}
    <div class="set-row"><span>Task list</span><select id="nc-tasklist"><option value="${esc(c.tasks_url || '')}">${c.tasks_url ? 'Selected list' : 'Automatic ("Tasks")'}</option></select>
      <button class="ghost" id="nc-load-lists">Load lists</button></div>
    <h4>Automatic backups</h4>
    <div class="set-row"><span>Schedule</span>${seg('nc_backup_schedule', c.backup_schedule, [['off', 'Off'], ['daily', 'Daily'], ['weekly', 'Weekly']])}</div>
    <div class="set-row"><span>Folder</span><input id="nc-bfolder" value="${esc(c.backup_folder)}"/></div>
    <div class="set-row"><span>Keep last</span><input id="nc-bkeep" type="number" min="1" max="100" value="${c.backup_keep}" class="num"/> backups per account</div>
    <h4>Large attachments</h4>
    <div class="set-row"><span>Offer a link above</span><input id="nc-lmb" type="number" min="1" max="2000" value="${c.link_threshold_mb}" class="num"/> MB</div>
    <div class="set-row"><span>Links expire after</span><input id="nc-lexp" type="number" min="0" max="365" value="${c.link_expire_days}" class="num"/> days (0 = never)</div>
    <div class="set-actions">
      <button class="ghost danger" id="nc-disconnect">Disconnect</button><span class="grow"></span>
      <button class="primary" id="nc-sync">${icon('refresh', 16)} Sync now</button>
    </div>
  </section>`;
}

async function ncWire() {
  const page = $('#settings-page');
  const connect = $('#nc-connect');
  if (connect) connect.onclick = async () => {
    const msg = $('#nc-msg');
    msg.className = 'small';
    msg.textContent = 'Connecting…';
    connect.disabled = true;
    try {
      const rep = await invoke('nc_connect', {
        server: $('#nc-server').value, user: $('#nc-user').value, password: $('#nc-pass').value,
        contactsUrl: $('#nc-curl').value, tasksUrl: $('#nc-turl').value,
      });
      toast(`Nextcloud connected · ${rep.contacts_down} contacts and ${rep.tasks_down} tasks downloaded, ${rep.contacts_up + rep.tasks_up} uploaded`, 'ok');
      await loadNc(); await loadContacts(); await loadTodos();
      renderSettings();
    } catch (e) { msg.className = 'small error'; msg.textContent = errText(e); }
    finally { connect.disabled = false; }
  };
  const sync = $('#nc-sync');
  if (sync) sync.onclick = async () => {
    sync.disabled = true;
    try {
      const r = await withBusy('Syncing with Nextcloud…', () => invoke('nc_sync_now'));
      toast(`Synced · ↓ ${r.contacts_down + r.tasks_down}  ↑ ${r.contacts_up + r.tasks_up}${r.deleted ? `  ✕ ${r.deleted}` : ''}`, 'ok');
    } catch (e) { toast(errText(e), 'error'); }
    await loadNc(); renderSettings();
  };
  const dis = $('#nc-disconnect');
  if (dis) dis.onclick = async () => {
    if (!(await confirmBox('Disconnect Nextcloud? Contacts and tasks stay on this device and on the server.', 'Disconnect', true))) return;
    await invoke('nc_disconnect'); await loadNc(); renderSettings();
  };
  const ll = $('#nc-load-lists');
  if (ll) ll.onclick = async () => {
    try {
      const lists = await invoke('nc_task_lists');
      const sel = $('#nc-tasklist');
      sel.innerHTML = `<option value="">Automatic ("Tasks")</option>` + lists.map(([url, name]) => `<option value="${esc(url)}" ${url === S.nc.tasks_url ? 'selected' : ''}>${esc(name)}</option>`).join('');
    } catch (e) { toast(errText(e), 'error'); }
  };
  void page;
}

async function ncSaveOption(patch) {
  try { await invoke('nc_save_options', { options: patch }); await loadNc(); } catch (e) { toast(errText(e), 'error'); }
}

// ------------------------------------------------------------------ pairing card
function pairCardHtml() {
  if (S.mobile) {
    return `<section class="card pair-card">
      <h3>${icon('qr', 18)} Copy setup from your computer</h3>
      <p class="muted small">On the computer open Kite Mail → Settings → <b>Pair a phone</b>, then scan the code. Both devices must be on the same Wi-Fi.</p>
      <div class="set-actions">
        ${window.__TAURI__.barcodeScanner ? `<button class="primary" id="pair-scan">${icon('qr', 16)} Scan QR code</button>` : ''}
      </div>
      <div class="set-row"><span>Or paste code</span><input id="pair-code" placeholder="kitemail://pair?…"/><button class="ghost" id="pair-go">Import</button></div>
    </section>`;
  }
  return `<section class="card pair-card">
    <h3>${icon('qr', 18)} Pair a phone</h3>
    <p class="muted small">Copies your accounts (with sign-ins), contacts, tasks, themes, signatures and Nextcloud connection to Kite Mail on Android.
      Everything is encrypted with a key that exists only inside the QR code. The code works for 10 minutes on your local network – for one or several phones,
      or to re-pair the same phone – until you press Stop. Mail itself syncs from your mail servers.</p>
    <div id="pair-area"><button class="primary" id="pair-start">${icon('qr', 16)} Show pairing QR code</button></div>
  </section>`;
}

let pairTimer = null;
function pairWire() {
  const start = $('#pair-start');
  if (start) start.onclick = async () => {
    try {
      const info = await invoke('pair_start');
      let left = info.expires_in;
      $('#pair-area').innerHTML = `<div class="pair-box">
          <div class="qr">${info.qr_svg}</div>
          <div class="pair-side">
            <b>Scan with Kite Mail on your phone</b>
            <p class="muted small">Phone app → Settings → Copy setup from your computer → Scan QR code.<br>${info.accounts} account(s) will be copied.</p>
            <p class="small" id="pair-status">${icon('clock', 14)} Waiting for a phone…</p>
            <p class="small muted" id="pair-left"></p>
            <div class="set-actions"><button class="ghost danger" id="pair-stop">${icon('x', 15)} Stop</button>
              <button class="ghost" id="pair-new">${icon('refresh', 15)} New code</button></div>
            <details class="adv"><summary>Can't scan? Show code</summary><textarea readonly rows="3" class="code">${esc(info.code)}</textarea></details>
          </div></div>`;
      $('#pair-stop').onclick = () => invoke('pair_stop');
      $('#pair-new').onclick = () => start.onclick();
      clearInterval(pairTimer);
      pairTimer = setInterval(() => {
        left--;
        const el = $('#pair-left');
        if (!el) { clearInterval(pairTimer); return; }
        el.textContent = `Code valid for ${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`;
        if (left <= 0) clearInterval(pairTimer);
      }, 1000);
    } catch (e) { toast(errText(e), 'error'); }
  };
  const go = $('#pair-go');
  if (go) go.onclick = () => importPairing($('#pair-code').value);
  const scan = $('#pair-scan');
  if (scan) scan.onclick = scanPairing;
}

/** Live updates from the pairing server: a phone downloaded the setup, or the code ended. */
function onPairEvent(e) {
  const st = $('#pair-status');
  if (e.kind === 'paired') {
    toast(e.count === 1 ? 'Phone paired successfully' : `Paired ${e.count} devices`, 'ok');
    if (st) st.innerHTML = `${icon('check', 14)} ${e.count} device${e.count > 1 ? 's' : ''} paired – the code still works for more phones until it expires or you press Stop.`;
    return;
  }
  clearInterval(pairTimer);
  if (st) st.innerHTML = `${icon(e.count ? 'check' : 'junk', 14)} Code ${e.count ? `used by ${e.count} device${e.count > 1 ? 's' : ''} and ` : ''}closed. Press <b>New code</b> to pair another phone.`;
  const left = $('#pair-left'); if (left) left.textContent = '';
  const stop = $('#pair-stop'); if (stop) stop.hidden = true;
  const qr = $('.pair-box .qr'); if (qr) qr.classList.add('expired');
}

async function scanPairing() {
  const bs = window.__TAURI__.barcodeScanner;
  if (!bs) { toast('Camera scanning is only available in the phone app', 'error'); return; }
  try {
    const perm = await bs.checkPermissions().catch(() => 'prompt');
    if (perm !== 'granted') await bs.requestPermissions();
    const res = await bs.scan({ windowed: false, formats: ['QR_CODE'] });
    if (res?.content) importPairing(res.content);
  } catch (e) { toast(errText(e), 'error'); }
}

async function importPairing(code) {
  if (!code || !code.trim()) { toast('Scan or paste the pairing code first', 'error'); return; }
  try {
    const r = await withBusy('Copying setup…', () => invoke('pair_import', { code: code.trim() }));
    toast(`Done · ${r.accounts} account(s)${r.skipped_accounts ? ` (${r.skipped_accounts} already here)` : ''}, ${r.contacts} contacts, ${r.todos} tasks${r.nextcloud ? ', Nextcloud connected' : ''}`, 'ok');
    $('#account-dlg').open && $('#account-dlg').close();
    document.querySelector('.onboard')?.remove();
    await reloadEverything();
  } catch (e) { toast(errText(e), 'error'); }
}

async function reloadEverything() {
  try { Object.assign(S.settings, DEFAULT_SETTINGS, await invoke('get_settings')); } catch (_) { /* keep */ }
  applyTheme();
  S.accounts = await invoke('list_accounts');
  for (const a of S.accounts) {
    S.folders[a.id] = await invoke('list_folders', { accountId: a.id });
    if (!S.folders[a.id].length) await refreshAccount(a.id);
  }
  await loadNc();
  if (S.nc.enabled) await invoke('nc_sync_now').catch(() => {});
  await loadContacts(); await loadTodos();
  renderSidebar();
  if (!S.cur) openFirstInbox();
  if (S.tab === 'settings') renderSettings();
}

// ------------------------------------------------------------------ mobile onboarding
function mobileOnboarding() {
  const d = document.createElement('dialog');
  d.className = 'small-dialog onboard';
  d.innerHTML = `<form method="dialog">
    <div class="ob-logo">${icon('kite', 34)}</div>
    <h3>Welcome to Kite Mail</h3>
    <p>Already using Kite Mail on your computer? Copy everything in one scan.</p>
    <button value="scan" class="primary wide">${icon('qr', 16)} Scan code from computer</button>
    <button value="paste" class="ghost wide">Paste pairing code</button>
    <button value="manual" class="ghost wide">Add an account manually</button></form>`;
  document.body.appendChild(d);
  d.addEventListener('close', () => {
    const v = d.returnValue;
    d.remove();
    if (v === 'scan') scanPairing();
    else if (v === 'paste') { switchTab('settings'); setTimeout(() => $('#pair-code')?.focus(), 100); }
    else openAccountDialog();
  });
  d.showModal();
}

// ------------------------------------------------------------------ touch: swipe actions & pull-to-refresh
(function touchGestures() {
  const box = $('#messages');
  const ptr = $('#ptr');
  let sx = 0, sy = 0, row = null, dx = 0, mode = null, pull = 0, startTop = 0;
  const reset = () => {
    if (row) { row.style.transform = ''; row.classList.remove('swipe-left', 'swipe-right'); }
    ptr.style.height = '';
    ptr.classList.remove('ready', 'pulling');
    row = null; mode = null; dx = 0; pull = 0;
  };
  box.addEventListener('touchstart', (e) => {
    reset();
    const t = e.touches[0];
    sx = t.clientX; sy = t.clientY; startTop = box.scrollTop;
    row = e.target.closest('.msg');
  }, { passive: true });
  box.addEventListener('touchmove', (e) => {
    const t = e.touches[0];
    const mx = t.clientX - sx, my = t.clientY - sy;
    if (!mode) {
      if (Math.abs(mx) > Math.abs(my) + 6 && Math.abs(mx) > 12) mode = 'swipe';
      else if (startTop <= 0 && box.scrollTop <= 0 && my > 12) mode = 'pull';
      else if (Math.abs(my) > 12) mode = 'scroll';
    }
    if (mode === 'swipe' && row) {
      dx = Math.max(-140, Math.min(140, mx));
      row.style.transform = `translateX(${dx}px)`;
      row.classList.toggle('swipe-left', dx < -70);
      row.classList.toggle('swipe-right', dx > 70);
    } else if (mode === 'pull') {
      pull = Math.min(90, (my - 12) * 0.6);
      ptr.classList.add('pulling');
      ptr.style.height = Math.max(0, pull) + 'px';
      ptr.classList.toggle('ready', pull > 55);
    }
  }, { passive: true });
  box.addEventListener('touchend', () => {
    const m = row && messageByKey(row.dataset.key);
    const wasSwipe = mode === 'swipe', d = dx, refresh = mode === 'pull' && pull > 55;
    reset();
    if (wasSwipe && m && d < -70) archiveMessages([m]);
    if (wasSwipe && m && d > 70) setSeen([m], !m.seen);
    if (refresh) syncCurrent();
  });
  // Android cancels touches when it takes over scrolling – never leave the spinner or a row stuck
  box.addEventListener('touchcancel', reset);
  box.addEventListener('scroll', () => { if (mode !== 'pull' && ptr.style.height) reset(); }, { passive: true });
})();

// ------------------------------------------------------------------ Android back button (history-driven)
const layers = [];
function pushLayer(name, close) {
  layers.push({ name, close });
  history.pushState({ layer: name }, '');
}
/** Closes a layer that was dismissed by the UI (not by Back): keeps history in step. Returns true if it was open. */
function dropLayer(name) {
  if (!layers.length || layers[layers.length - 1].name !== name) return false;
  history.back(); // triggers popstate -> close() (idempotent)
  return true;
}
window.addEventListener('popstate', () => {
  const top = layers.pop();
  if (top) top.close();
});
