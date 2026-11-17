'use strict';
/* Mail view: accounts & folders sidebar, virtualized message list, reader, message actions. */

const PAGE = 100;
const FOLDER_ICON = { inbox: 'inbox', drafts: 'draft', sent: 'send', trash: 'trash', junk: 'junk', archive: 'archive', all: 'all', '': 'folder' };
const NO_COUNT = new Set(['sent', 'drafts', 'trash', 'junk', 'all']);
const key = (m) => `${m.account_id}|${m.folder}|${m.uid}`;
const curKey = () => (S.cur ? `${S.cur.accountId}|${S.cur.folder}` : '');
const limit = () => S.limits[curKey()] || PAGE;
const folderOf = (accountId, name) => (S.folders[accountId] || []).find((f) => f.name === name);
const folderName = (accountId, name) => folderOf(accountId, name)?.display || name;

// ------------------------------------------------------------------ sidebar
function renderSidebar() {
  const nav = $('#accounts');
  nav.innerHTML = '';
  for (const a of S.accounts) {
    const box = document.createElement('div');
    box.className = 'account';
    const err = S.errors[a.id];
    const collapsed = S.collapsed[a.id];
    box.innerHTML = `<div class="acct-head" title="${esc(err || a.email)}">
        ${avatar(a.name, a.email, 26)}
        <div class="acct-names"><b>${esc(a.name || a.email)}</b><small>${esc(a.email)}</small></div>
        ${err ? `<span class="warn" title="${esc(err)}">${icon('junk', 16)}</span>` : ''}
        <span class="chev ${collapsed ? '' : 'open'}">${icon('back', 14)}</span>
      </div>
      ${err && a.kind !== 'imap' ? `<button class="reconnect" data-id="${a.id}">${icon('refresh', 14)} Reconnect account</button>` : ''}`;
    $('.acct-head', box).onclick = () => { S.collapsed[a.id] = !collapsed; renderSidebar(); };
    const rc = $('.reconnect', box);
    if (rc) rc.onclick = () => reconnect(a.id);
    if (!collapsed) {
      const list = document.createElement('div');
      list.className = 'folders';
      for (const f of S.folders[a.id] || []) {
        const row = document.createElement('div');
        const active = S.cur && S.cur.accountId === a.id && S.cur.folder === f.name && !S.searching;
        row.className = 'folder' + (f.noselect ? ' noselect' : '') + (active ? ' active' : '');
        row.style.paddingLeft = 10 + f.depth * 14 + 'px';
        const showCount = f.unread > 0 && !NO_COUNT.has(f.special);
        row.innerHTML = `${icon(FOLDER_ICON[f.special] ?? 'folder', 17)}<span class="fname">${esc(f.display)}</span>${showCount ? `<span class="count">${f.unread}</span>` : ''}`;
        row.title = f.display;
        if (!f.noselect) row.onclick = () => { selectFolder(a.id, f.name); $('#app').classList.remove('drawer'); };
        list.appendChild(row);
      }
      box.appendChild(list);
    }
    nav.appendChild(box);
  }
  const unread = S.accounts.reduce((n, a) => n + ((S.folders[a.id] || []).find((f) => f.special === 'inbox')?.unread || 0), 0);
  document.title = unread ? `(${unread}) Kite Mail` : 'Kite Mail';
  const b = $('#badge-mail');
  b.textContent = unread > 99 ? '99+' : unread || '';
  b.hidden = !unread;
}

async function reloadFolders(accountId) {
  try { S.folders[accountId] = await invoke('list_folders', { accountId }); } catch (_) { /* ignore */ }
  renderSidebar();
}

async function refreshAccount(accountId, loud = false) {
  try {
    S.folders[accountId] = await invoke('refresh_folders', { accountId });
    delete S.errors[accountId];
    renderSidebar();
    if (loud) toast('Folders refreshed', 'ok');
  } catch (e) {
    S.errors[accountId] = errText(e);
    renderSidebar();
    if (loud) toast(errText(e), 'error');
  }
}

async function reconnect(accountId) {
  toast('A browser window opened – sign in there to reconnect.');
  try {
    await withBusy('Waiting for sign-in…', () => invoke('reconnect_account', { accountId }));
    delete S.errors[accountId];
    toast('Reconnected', 'ok');
    await reloadFolders(accountId);
    if (S.cur?.accountId === accountId) syncCurrent();
  } catch (e) { toast(errText(e), 'error'); }
}

async function removeAccount(accountId) {
  const a = acct(accountId);
  if (!(await confirmBox(`Remove ${a.email} from Kite Mail?\nMail on the server is not affected.`, 'Remove', true))) return false;
  try {
    await invoke('remove_account', { accountId });
    S.accounts = S.accounts.filter((x) => x.id !== accountId);
    delete S.folders[accountId];
    delete S.errors[accountId];
    if (S.cur?.accountId === accountId) { S.cur = null; S.messages = []; renderList(); showReader(null); openFirstInbox(); }
    renderSidebar();
    return true;
  } catch (e) { toast(errText(e), 'error'); return false; }
}

function openFirstInbox() {
  for (const a of S.accounts) {
    const inbox = (S.folders[a.id] || []).find((f) => f.special === 'inbox');
    if (inbox) return selectFolder(a.id, inbox.name);
  }
}

// ------------------------------------------------------------------ list
async function selectFolder(accountId, folder) {
  S.cur = { accountId, folder };
  S.searching = false;
  $('#search').value = '';
  S.selected.clear();
  showReader(null);
  renderSidebar();
  try { S.messages = await invoke('list_messages', { accountId, folder, limit: limit() }); } catch (e) { S.messages = []; }
  $('#messages').scrollTop = 0;
  renderList();
  syncCurrent();
}

async function syncCurrent(quiet = false) {
  const cur = S.cur;
  if (!cur) return;
  const run = async () => {
    const list = await invoke('sync_folder', { accountId: cur.accountId, folder: cur.folder, limit: limit() });
    delete S.errors[cur.accountId];
    if (S.cur === cur && !S.searching) { S.messages = list; renderList(); }
    await reloadFolders(cur.accountId);
    invoke('prefetch_bodies', { accountId: cur.accountId, folder: cur.folder })
      .then((n) => { if (n > 0) reloadCurrentFromCache(); }).catch(() => {});
  };
  try { await (quiet ? run() : withBusy('Syncing…', run)); }
  catch (e) {
    S.errors[cur.accountId] = errText(e);
    renderSidebar();
    if (!quiet) toast(errText(e), 'error');
  }
}

async function reloadCurrentFromCache() {
  if (!S.cur || S.searching) return;
  const cur = S.cur;
  const list = await invoke('list_messages', { accountId: cur.accountId, folder: cur.folder, limit: limit() });
  if (S.cur === cur && !S.searching) { S.messages = list; renderList(); }
}

// Virtualized list: only the rows in view are in the DOM.
let ROWS = [];      // { h: label } | { m: message }
let OFFS = [];      // top offset of each row
const HEAD_H = 30;
const rowH = () => (S.settings.density === 'compact' ? 50 : 78);

function renderList() {
  const a = S.cur && acct(S.cur.accountId);
  $('#list-title').innerHTML = S.searching
    ? `<b>Search results</b> <span class="muted">${S.messages.length}</span>`
    : S.cur ? `<b>${esc(folderName(S.cur.accountId, S.cur.folder))}</b> <span class="muted">${esc(a?.email || '')}</span>` : '';
  ROWS = []; OFFS = [];
  let y = 0, last = null;
  const groups = !S.searching || true;
  for (const m of S.messages) {
    const g = dayGroup(m.date);
    if (groups && g !== last) { ROWS.push({ h: g }); OFFS.push(y); y += HEAD_H; last = g; }
    ROWS.push({ m }); OFFS.push(y); y += rowH();
  }
  $('#msg-spacer').style.height = y + 'px';
  renderVisible(true);
  updateBulk();

  const footer = $('#list-footer');
  footer.innerHTML = '';
  if (!S.messages.length) {
    $('#msg-spacer').innerHTML = `<div class="list-empty">${icon(S.searching ? 'search' : 'inbox', 40)}<p>${S.searching ? 'No matches in downloaded mail' : S.cur ? 'All clear – no messages here' : ''}</p></div>`;
  } else if (!S.searching && S.cur && S.messages.length >= limit()) {
    const b = document.createElement('button');
    b.className = 'ghost';
    b.textContent = 'Load older messages';
    b.onclick = () => { S.limits[curKey()] = limit() + PAGE; syncCurrent(); };
    footer.appendChild(b);
  }
}

let lastRange = '';
function renderVisible(force = false) {
  const box = $('#messages');
  if (!ROWS.length) return;
  const top = box.scrollTop, bottom = top + box.clientHeight;
  let lo = 0, hi = OFFS.length - 1;
  while (lo < hi) { const mid = (lo + hi + 1) >> 1; if (OFFS[mid] <= top) lo = mid; else hi = mid - 1; }
  const start = Math.max(0, lo - 4);
  let end = lo;
  while (end < ROWS.length && OFFS[end] < bottom) end++;
  end = Math.min(ROWS.length, end + 6);
  const range = `${start}-${end}`;
  if (!force && range === lastRange) return;
  lastRange = range;
  const compact = S.settings.density === 'compact';
  let html = '';
  for (let i = start; i < end; i++) {
    const r = ROWS[i];
    if (r.h) { html += `<div class="day-head" style="top:${OFFS[i]}px">${esc(r.h)}</div>`; continue; }
    const m = r.m;
    const sp = folderOf(m.account_id, m.folder)?.special;
    const sentLike = sp === 'sent' || sp === 'drafts';
    const who = sentLike ? (m.to ? 'To: ' + m.to : '(no recipient)') : (m.from_name || m.from_addr || '(unknown)');
    const firstTo = splitAddrs(m.to || '')[0] || '';
    const toName = firstTo.replace(/<[^>]*>/, '').replace(/"/g, '').trim();
    const k = key(m);
    html += `<div class="msg${m.seen ? '' : ' unread'}${S.selected.has(k) ? ' selected' : ''}${m.flagged ? ' flagged' : ''}" data-key="${esc(k)}" style="top:${OFFS[i]}px;height:${rowH()}px">
      ${compact ? '' : avatar(sentLike ? toName : m.from_name, sentLike ? bareAddr(firstTo) : m.from_addr, 36)}
      <div class="m-main">
        <div class="m-top"><span class="from">${esc(who)}</span><span class="date">${fmtDate(m.date)}</span></div>
        <div class="m-subj">${m.answered ? `<span class="m-ic">${icon('reply', 13)}</span>` : ''}${esc(m.subject || '(no subject)')}${S.searching ? ` <span class="folder-tag">${esc(folderName(m.account_id, m.folder))}</span>` : ''}</div>
        ${compact ? '' : `<div class="m-prev">${esc(m.preview || '')}</div>`}
      </div>
      <div class="m-side">
        ${m.has_attach ? `<span class="m-ic">${icon('clip', 14)}</span>` : ''}
        <button class="star-btn ${m.flagged ? 'on' : ''}" data-q="star" title="Star">${icon('star', 15)}</button>
      </div>
      <div class="m-hover">
        <button data-q="archive" title="Archive">${icon('archive', 16)}</button>
        <button data-q="delete" title="Delete">${icon('trash', 16)}</button>
        <button data-q="read" title="${m.seen ? 'Mark unread' : 'Mark read'}">${icon(m.seen ? 'unread' : 'mail', 16)}</button>
      </div>
    </div>`;
  }
  $('#msg-spacer').innerHTML = html;
}

const messageByKey = (k) => S.messages.find((m) => key(m) === k);

function onListClick(e) {
  const row = e.target.closest('.msg');
  if (!row) return;
  const m = messageByKey(row.dataset.key);
  if (!m) return;
  const q = e.target.closest('[data-q]')?.dataset.q;
  if (q) {
    e.stopPropagation();
    if (q === 'star') toggleFlag([m], !m.flagged);
    if (q === 'archive') archiveMessages([m]);
    if (q === 'delete') deleteMessages([m]);
    if (q === 'read') setSeen([m], !m.seen);
    return;
  }
  const k = key(m);
  if (e.ctrlKey || e.metaKey) {
    S.selected.has(k) ? S.selected.delete(k) : S.selected.add(k);
    S.anchor = k;
  } else if (e.shiftKey && S.anchor) {
    const keys = S.messages.map(key);
    const [i, j] = [keys.indexOf(S.anchor), keys.indexOf(k)].sort((x, y) => x - y);
    S.selected = new Set(keys.slice(i, j + 1));
  } else {
    S.selected = new Set([k]);
    S.anchor = k;
    if (folderOf(m.account_id, m.folder)?.special === 'drafts') openDraftMessage(m);
    else openMessage(m);
  }
  renderVisible(true);
  updateBulk();
}

function updateBulk() {
  const n = S.selected.size;
  $('#bulk').hidden = n < 2;
  $('#bulk-count').textContent = `${n} selected`;
}

function selectedMessages() {
  const sel = S.messages.filter((m) => S.selected.has(key(m)));
  if (sel.length) return sel;
  return S.view ? S.messages.filter((m) => key(m) === key(S.view.summary)) : [];
}

// ------------------------------------------------------------------ reader
async function openMessage(m) {
  try {
    const view = await withBusy('Loading…', () => invoke('open_message', { accountId: m.account_id, folder: m.folder, uid: m.uid }));
    if (!S.selected.has(key(m))) return;
    if (!m.seen) {
      m.seen = true;
      const f = folderOf(m.account_id, m.folder);
      if (f && f.unread > 0) { f.unread--; renderSidebar(); }
      renderVisible(true);
    }
    S.allowRemote = S.settings.remote_images === 'always';
    showReader(view);
    $('#app').classList.add('reading');
  } catch (e) { toast(errText(e), 'error'); }
}

function contactFor(addr) {
  const a = (addr || '').toLowerCase();
  return S.contacts.find((c) => c.email.toLowerCase() === a);
}

function showReader(view) {
  S.view = view;
  $('#reader-empty').hidden = !!view;
  $('#reader-content').hidden = !view;
  if (!view) { $('#r-body').srcdoc = ''; $('#app').classList.remove('reading'); return; }
  const s = view.summary;
  $('#r-subject').textContent = s.subject || '(no subject)';
  $('#r-from').textContent = s.from_name || s.from_addr;
  $('#r-from-addr').textContent = s.from_name ? s.from_addr : '';
  $('#r-avatar').innerHTML = avatar(s.from_name, s.from_addr, 44);
  $('#r-to').textContent = view.to ? 'To ' + view.to : '';
  $('#r-cc').textContent = view.cc ? 'Cc ' + view.cc : '';
  $('#r-date').textContent = fmtDate(s.date, true);
  const c = contactFor(s.from_addr);
  $('#r-add-contact').hidden = !!c;
  $('#r-call').hidden = !(c && c.phone);
  $('[data-act=star]').classList.toggle('on', s.flagged);

  const atts = $('#r-attachments');
  atts.innerHTML = '';
  for (const a of view.attachments.filter((x) => !x.inline)) {
    const chip = document.createElement('div');
    chip.className = 'att';
    const ext = (a.name.split('.').pop() || '').slice(0, 4).toUpperCase();
    chip.innerHTML = `<span class="att-ext">${esc(ext)}</span><span class="att-name" title="Open ${esc(a.name)}">${esc(a.name)}<small>${fmtSize(a.size)}</small></span>
      <button class="icon" title="Save as…">${icon('download', 16)}</button>`;
    const args = { accountId: s.account_id, folder: s.folder, uid: s.uid, index: a.index };
    $('.att-name', chip).onclick = () => invoke('open_attachment', args).catch((e) => toast(errText(e), 'error'));
    $('button', chip).onclick = () => invoke('save_attachment', args).then((ok) => ok && toast('Saved', 'ok')).catch((e) => toast(errText(e), 'error'));
    atts.appendChild(chip);
  }
  $('.reader-scroll').scrollTop = 0;
  renderBody();
}

function renderBody() {
  const v = S.view;
  if (!v) return;
  $('#r-remote').hidden = !(v.has_remote && !S.allowRemote);
  const imgSrc = S.allowRemote ? 'data: https: http:' : 'data:';
  const darkify = S.settings.dark_emails && document.documentElement.dataset.dark === '1';
  const body = v.html || `<pre style="white-space:pre-wrap;font-family:inherit;margin:0">${esc(v.text)}</pre>`;
  const doc = `<!doctype html><html><head><meta charset="utf-8">
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src ${imgSrc}; style-src 'unsafe-inline'; font-src data:">
    <style>html{background:#fff}body{font-family:"Segoe UI",system-ui,sans-serif;font-size:14px;line-height:1.5;color:#1d2330;margin:0;padding:4px 2px 24px;overflow-wrap:anywhere}
    img{max-width:100%;height:auto}blockquote{border-left:3px solid #ccd;margin:0 0 0 4px;padding-left:12px;color:#555}a{color:#3b5bdb}
    ${darkify ? 'html{filter:invert(.92) hue-rotate(180deg)}img,video,picture,[style*="background-image"]{filter:invert(1) hue-rotate(180deg)}' : ''}</style>
    </head><body>${body}</body></html>`;
  const frame = $('#r-body');
  frame.parentElement.classList.toggle('darkified', darkify);
  frame.onload = () => {
    const d = frame.contentDocument;
    if (!d) return;
    const fit = () => { frame.style.height = Math.max(200, d.documentElement.scrollHeight + 8) + 'px'; };
    fit();
    setTimeout(fit, 300);
    d.querySelectorAll('img').forEach((img) => img.addEventListener('load', fit));
    d.addEventListener('click', (e) => {
      const a = e.target.closest && e.target.closest('a[href]');
      if (!a) return;
      e.preventDefault();
      const href = a.getAttribute('href').trim();
      if (/^mailto:/i.test(href)) {
        const [addr, query] = href.slice(7).split('?');
        const p = new URLSearchParams(query || '');
        openCompose({ to: decodeURIComponent(addr), subject: p.get('subject') || '', body: p.get('body') || '' });
      } else if (/^(tel|callto|sip):/i.test(href)) {
        invoke('call_number', { number: decodeURIComponent(href.replace(/^[a-z]+:/i, '')) }).catch((err) => toast(errText(err), 'error'));
      } else if (/^https?:/i.test(href)) {
        invoke('open_url', { url: href }).catch((err) => toast(errText(err), 'error'));
      }
    });
  };
  frame.srcdoc = doc;
}

/** Readable plain text of the open message (from the rendered HTML: keeps line breaks / table rows). */
function messageText(view) {
  let t = '';
  try {
    const d = $('#r-body').contentDocument;
    if (view.html && d?.body && S.view === view) t = d.body.innerText;
  } catch (_) { /* fall back */ }
  if (!t.trim()) t = view.text || '';
  return t.replace(/\r/g, '').replace(/\n{3,}/g, '\n\n').trim();
}

// ------------------------------------------------------------------ actions
function groupByFolder(list) {
  const g = new Map();
  for (const m of list) {
    const k = `${m.account_id}|${m.folder}`;
    if (!g.has(k)) g.set(k, []);
    g.get(k).push(m);
  }
  return [...g.values()];
}

async function toggleFlag(list, value) {
  if (!list.length) return;
  list.forEach((m) => { m.flagged = value; });
  if (S.view && list.some((m) => key(m) === key(S.view.summary))) { S.view.summary.flagged = value; $('[data-act=star]').classList.toggle('on', value); }
  renderVisible(true);
  for (const g of groupByFolder(list)) {
    invoke('set_flag', { accountId: g[0].account_id, folder: g[0].folder, uids: g.map((m) => m.uid), flag: 'flagged', value }).catch((e) => toast(errText(e), 'error'));
  }
}

async function setSeen(list, value) {
  if (!list.length) return;
  list.forEach((m) => { m.seen = value; });
  renderVisible(true);
  for (const g of groupByFolder(list)) {
    try { await invoke('set_flag', { accountId: g[0].account_id, folder: g[0].folder, uids: g.map((m) => m.uid), flag: 'seen', value }); }
    catch (e) { toast(errText(e), 'error'); }
  }
  new Set(list.map((m) => m.account_id)).forEach((id) => reloadFolders(id));
}

async function removeFromList(list, op, doneMsg) {
  if (!list.length) return;
  const keys = new Set(list.map(key));
  const idx = S.messages.findIndex((m) => keys.has(key(m)));
  const wasOpen = S.view && keys.has(key(S.view.summary));
  // optimistic: remove first, restore on failure
  const before = S.messages;
  S.messages = S.messages.filter((m) => !keys.has(key(m)));
  S.selected.clear();
  renderList();
  try {
    for (const group of groupByFolder(list)) await op(group);
    if (doneMsg) toast(doneMsg(list.length), 'ok');
  } catch (e) {
    toast(errText(e), 'error');
    S.messages = before;
    renderList();
    return;
  }
  if (wasOpen) {
    const next = S.messages[Math.min(idx, S.messages.length - 1)];
    if (next && !$('#app').classList.contains('narrow')) { S.selected.add(key(next)); renderVisible(true); openMessage(next); } else showReader(null);
  }
  new Set(list.map((m) => m.account_id)).forEach((id) => reloadFolders(id));
}

const deleteMessages = (list) => removeFromList(list, (g) =>
  invoke('delete_messages', { accountId: g[0].account_id, folder: g[0].folder, uids: g.map((m) => m.uid) }),
  (n) => `${n > 1 ? n + ' messages' : 'Message'} deleted`);
const moveMessages = (list, target) => removeFromList(list, (g) =>
  invoke('move_messages', { accountId: g[0].account_id, folder: g[0].folder, uids: g.map((m) => m.uid), target }),
  (n) => `Moved to ${folderName(list[0].account_id, target)}`);
const archiveMessages = (list) => {
  if (list.length && folderOf(list[0].account_id, list[0].folder)?.special === 'archive') { toast('Already in Archive'); return; }
  return removeFromList(list, (g) =>
    invoke('archive_messages', { accountId: g[0].account_id, folder: g[0].folder, uids: g.map((m) => m.uid) }),
    (n) => `${n > 1 ? n + ' messages' : 'Message'} archived`);
};

/** Small popover menu next to an element. Resolves with the chosen value or null. */
function popMenu(anchor, items) {
  return new Promise((resolve) => {
    document.querySelector('.popmenu')?.remove();
    const m = document.createElement('div');
    m.className = 'popmenu';
    m.innerHTML = items.map((it, i) => `<button data-i="${i}" style="padding-left:${12 + (it.depth || 0) * 12}px">${it.icon ? icon(it.icon, 16) : ''}<span>${esc(it.label)}</span></button>`).join('');
    document.body.appendChild(m);
    const r = anchor.getBoundingClientRect();
    m.style.top = Math.min(r.bottom + 4, window.innerHeight - m.offsetHeight - 8) + 'px';
    m.style.left = Math.max(8, Math.min(r.left, window.innerWidth - m.offsetWidth - 8)) + 'px';
    const close = (v) => { m.remove(); document.removeEventListener('mousedown', outside, true); resolve(v); };
    const outside = (e) => { if (!m.contains(e.target)) close(null); };
    setTimeout(() => document.addEventListener('mousedown', outside, true));
    m.onclick = (e) => { const b = e.target.closest('button'); if (b) close(items[Number(b.dataset.i)].value); };
  });
}

async function chooseMoveTarget(anchor, accountId, exclude) {
  const items = (S.folders[accountId] || []).filter((f) => !f.noselect && f.name !== exclude)
    .map((f) => ({ label: f.display, value: f.name, icon: FOLDER_ICON[f.special] ?? 'folder', depth: f.depth }));
  return popMenu(anchor, items);
}

$('#messages').addEventListener('click', onListClick);
$('#messages').addEventListener('scroll', () => requestAnimationFrame(() => renderVisible()));
new ResizeObserver(() => renderVisible(true)).observe($('#messages'));

$('.reader-bar').addEventListener('click', async (e) => {
  const b = e.target.closest('button[data-act]');
  if (!b || !S.view) return;
  const act = b.dataset.act;
  if (act === 'reply' || act === 'replyall' || act === 'forward') return openCompose({ mode: act, view: S.view });
  const list = selectedMessages();
  if (act === 'unread') setSeen(list, false);
  if (act === 'star') toggleFlag(list, !S.view.summary.flagged);
  if (act === 'delete') deleteMessages(list);
  if (act === 'archive') archiveMessages(list);
  if (act === 'task') quickTaskFromMail(S.view, b);
  if (act === 'move') {
    const t = await chooseMoveTarget(b, S.view.summary.account_id, S.view.summary.folder);
    if (t) moveMessages(list, t);
  }
});

$('#bulk').addEventListener('click', (e) => {
  const q = e.target.closest('[data-bulk]')?.dataset.bulk;
  const list = S.messages.filter((m) => S.selected.has(key(m)));
  if (q === 'read') setSeen(list, true);
  if (q === 'archive') archiveMessages(list);
  if (q === 'delete') deleteMessages(list);
});

$('#btn-load-images').onclick = () => { S.allowRemote = true; renderBody(); };
$('#btn-back').onclick = () => { $('#app').classList.remove('reading'); };
$('#btn-drawer').onclick = () => $('#app').classList.toggle('drawer');
$('#r-add-contact').onclick = () => {
  const s = S.view.summary;
  openContactEditor({ name: s.from_name, email: s.from_addr });
};
$('#r-call').onclick = () => {
  const c = contactFor(S.view.summary.from_addr);
  if (c?.phone) invoke('call_number', { number: c.phone }).catch((e) => toast(errText(e), 'error'));
};

// ------------------------------------------------------------------ search
const runSearch = debounce(async (q) => {
  if (q.length < 2) {
    if (S.searching && S.cur) selectFolder(S.cur.accountId, S.cur.folder);
    return;
  }
  try {
    S.searching = true;
    S.messages = await invoke('search', { accountId: S.cur?.accountId ?? null, query: q });
    S.selected.clear();
    $('#messages').scrollTop = 0;
    renderSidebar();
    renderList();
  } catch (err) { toast(errText(err), 'error'); }
}, 250);
$('#search').addEventListener('input', (e) => runSearch(e.target.value.trim()));
$('#btn-refresh').onclick = () => syncCurrent();

// ------------------------------------------------------------------ add account dialog
let acctKind = 'm365';
function setKind(kind) {
  acctKind = kind;
  $$('#acct-tabs button').forEach((b) => b.classList.toggle('active', b.dataset.kind === kind));
  $$('#account-dlg .kind').forEach((d) => (d.hidden = !d.classList.contains('kind-' + kind)));
}
$('#acct-tabs').addEventListener('click', (e) => { const k = e.target.dataset.kind; if (k) setKind(k); });

function openAccountDialog() {
  const ls = (k) => localStorage.getItem(k) || '';
  $('#a-ms-client').value = ls('lm.ms.client');
  $('#a-ms-tenant').value = ls('lm.ms.tenant');
  $('#a-g-client').value = ls('lm.g.client');
  $('#acct-msg').textContent = '';
  $('#acct-msg').className = 'small';
  $('#acct-save').disabled = false;
  setKind(acctKind);
  $('#account-dlg').showModal();
  $('#a-email').focus();
}
$('#btn-add-account').onclick = openAccountDialog;
$('#acct-close').onclick = () => $('#account-dlg').close();
$('#a-smtp-sec').onchange = (e) => { $('#a-smtp-port').value = { ssl: 465, starttls: 587, none: 25 }[e.target.value]; };

$('#acct-save').onclick = async () => {
  const v = (id) => $(id).value.trim();
  const input = { kind: acctKind, name: v('#a-name'), email: v('#a-email') };
  if (acctKind === 'm365') Object.assign(input, { client_id: v('#a-ms-client'), tenant: v('#a-ms-tenant') });
  if (acctKind === 'gmail') Object.assign(input, { client_id: v('#a-g-client'), client_secret: v('#a-g-secret') });
  if (acctKind === 'imap') Object.assign(input, {
    imap_host: v('#a-imap-host'), imap_port: Number(v('#a-imap-port')) || 993,
    smtp_host: v('#a-smtp-host'), smtp_port: Number(v('#a-smtp-port')) || 465,
    smtp_security: $('#a-smtp-sec').value, username: v('#a-user'), password: $('#a-pass').value,
  });
  const msg = $('#acct-msg');
  msg.className = 'small';
  msg.textContent = acctKind === 'imap' ? 'Connecting…' : 'A browser window opened – sign in there, then come back here…';
  $('#acct-save').disabled = true;
  try {
    const account = await invoke('add_account', { input });
    if (acctKind === 'm365') { localStorage.setItem('lm.ms.client', input.client_id); localStorage.setItem('lm.ms.tenant', input.tenant); }
    if (acctKind === 'gmail') localStorage.setItem('lm.g.client', input.client_id);
    $('#a-pass').value = '';
    $('#a-g-secret').value = '';
    $('#account-dlg').close();
    S.accounts.push(account);
    await reloadFolders(account.id);
    switchTab('mail');
    const inbox = S.folders[account.id].find((f) => f.special === 'inbox');
    if (inbox) selectFolder(account.id, inbox.name);
    toast(`${account.email} added`, 'ok');
    if (S.tab === 'settings') renderSettings();
  } catch (e) {
    msg.className = 'small error';
    msg.textContent = errText(e);
  } finally {
    $('#acct-save').disabled = false;
  }
};
