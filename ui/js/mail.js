'use strict';
/* Mail view: accounts & folders sidebar, virtualized message list, reader, message actions. */

const PAGE = 100;
const FOLDER_ICON = { files: 'files', inbox: 'inbox', drafts: 'draft', sent: 'send', trash: 'trash', junk: 'junk', archive: 'archive', all: 'all', '': 'folder' };
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
  // On phones the drawer shows only the current account; the avatar switcher changes account.
  const phone = $('#app').classList.contains('narrow') && S.cur && S.accounts.length > 1;
  for (const a of phone ? S.accounts.filter((x) => x.id === S.cur.accountId) : S.accounts) {
    const box = document.createElement('div');
    box.className = 'account';
    const err = S.errors[a.id];
    const collapsed = !phone && S.collapsed[a.id];
    box.style.setProperty('--c', acctColor(a));
    box.innerHTML = `<div class="acct-head" title="${esc(err || a.email)}">
        ${avatar(a.name, a.email, 26)}
        <div class="acct-names"><b>${esc(aprefs(a.id).label || a.name || a.email)}</b><small>${esc(a.email)}</small></div>
        ${err ? `<span class="warn" title="${esc(err)}">${icon('junk', 16)}</span>` : ''}
        ${phone ? `<span class="switch-hint">${icon('chevrons', 16)}</span>` : `<span class="chev ${collapsed ? '' : 'open'}">${icon('back', 14)}</span>`}
      </div>
      ${err && a.kind !== 'imap' ? `<button class="reconnect" data-id="${a.id}">${icon('refresh', 14)} Reconnect account</button>` : ''}`;
    $('.acct-head', box).onclick = phone ? () => openAccountSheet() : () => { S.collapsed[a.id] = !collapsed; renderSidebar(); };
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
        if (!f.noselect) row.onclick = () => { selectFolder(a.id, f.name); if ($('#app').classList.contains('drawer')) { $('#app').classList.remove('drawer'); dropLayer('drawer'); } };
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
  renderAccountChip();
}

// ------------------------------------------------------------------ account switcher (phones)
const inboxUnread = (id) => (S.folders[id] || []).find((f) => f.special === 'inbox')?.unread || 0;

function renderAccountChip() {
  const chip = $('#btn-acct');
  const a = S.cur && acct(S.cur.accountId);
  chip.hidden = !a;
  if (!a) return;
  document.documentElement.style.setProperty('--acct-color', acctColor(a));
  const others = S.accounts.some((x) => x.id !== a.id && inboxUnread(x.id) > 0);
  const html = `${avatar(a.name, a.email, 32)}${others ? '<span class="dot" title="New mail in another account"></span>' : ''}`;
  if (chip.innerHTML !== html) chip.innerHTML = html;
  chip.title = S.accounts.length > 1 ? `${a.email} – tap to switch, swipe to flip accounts` : a.email;
}

function openAccountSheet() {
  const d = $('#acct-sheet');
  if (d.open) return;
  const items = S.accounts.map((a) => {
    const n = inboxUnread(a.id), cur = S.cur?.accountId === a.id, err = S.errors[a.id];
    return `<button class="acct-item${cur ? ' current' : ''}" data-id="${a.id}" style="--c:${acctColor(a)}">
      ${avatar(a.name, a.email, 42)}
      <span class="acct-names"><b>${esc(aprefs(a.id).label || a.name || a.email)}</b><small>${esc(err ? 'Needs attention – ' + err : a.email)}</small></span>
      ${n ? `<span class="count">${n > 99 ? '99+' : n}</span>` : ''}
      ${cur ? `<span class="tick">${icon('check', 18)}</span>` : ''}
    </button>`;
  }).join('');
  const total = S.accounts.reduce((n, a) => n + inboxUnread(a.id), 0);
  d.innerHTML = `<div class="sheet-handle"></div>
    <header><strong>Accounts</strong><span class="muted small">${total ? `${total} unread` : 'All caught up'}</span></header>
    <div class="acct-list">${items}</div>
    <footer>
      <button class="ghost" data-act="add">${icon('plus', 16)} Add account</button>
      <button class="ghost" data-act="manage">${icon('settings', 16)} Manage</button>
    </footer>`;
  d.onclick = (e) => {
    if (e.target === d) return closeAccountSheet();
    const it = e.target.closest('.acct-item');
    if (it) {
      closeAccountSheet();
      switchAccount(Number(it.dataset.id));
      // opened from the drawer: close that too once the sheet's Back entry is gone
      if ($('#app').classList.contains('drawer')) setTimeout(() => { if (!dropLayer('drawer')) $('#app').classList.remove('drawer'); }, 150);
      return;
    }
    const act = e.target.closest('[data-act]')?.dataset.act;
    if (act === 'add') { closeAccountSheet(); openAccountDialog(); }
    if (act === 'manage') { closeAccountSheet(); switchTab('settings'); }
  };
  // drag the sheet down to dismiss
  let y0 = null, dy = 0;
  d.ontouchstart = (e) => { if ($('.acct-list', d).scrollTop <= 0) { y0 = e.touches[0].clientY; dy = 0; d.style.transition = 'none'; } };
  d.ontouchmove = (e) => { if (y0 === null) return; dy = Math.max(0, e.touches[0].clientY - y0); d.style.transform = `translateY(${dy}px)`; };
  d.ontouchend = () => {
    if (y0 === null) return;
    y0 = null;
    d.style.transition = '';
    if (dy > 90) closeAccountSheet(); else d.style.transform = '';
  };
  d.style.transform = '';
  d.showModal();
  pushLayer('accounts', () => { if (d.open) d.close(); });
}

function closeAccountSheet() {
  const d = $('#acct-sheet');
  if (!d.open) return;
  if (!dropLayer('accounts')) d.close();
}

/** Opens an account at the folder last used there (else its inbox), with a slide in the swipe direction. */
function switchAccount(id, dir = 0) {
  if (!S.accounts.length || S.cur?.accountId === id) return;
  const folders = S.folders[id] || [];
  const last = S.lastFolder[id];
  const target = (last && folders.some((f) => f.name === last)) ? last : folders.find((f) => f.special === 'inbox')?.name;
  if (!target) { refreshAccount(id, true); return; }
  const pane = $('#list-pane');
  pane.style.setProperty('--dx', `${dir >= 0 ? 36 : -36}px`);
  pane.classList.remove('acct-in');
  void pane.offsetWidth;            // restart the animation
  pane.classList.add('acct-in');
  selectFolder(id, target);
  if (navigator.vibrate) try { navigator.vibrate(8); } catch (_) { /* no haptics */ }
}

/** Next/previous account in order (wraps around). */
function cycleAccount(step) {
  if (S.accounts.length < 2 || !S.cur) return;
  const i = S.accounts.findIndex((a) => a.id === S.cur.accountId);
  const next = S.accounts[(i + step + S.accounts.length) % S.accounts.length];
  switchAccount(next.id, step);
  toast(`${next.name || next.email}`, 'info');
}

{
  const chip = $('#btn-acct');
  let sx = 0, sy = 0, swiped = false;
  chip.addEventListener('touchstart', (e) => { sx = e.touches[0].clientX; sy = e.touches[0].clientY; swiped = false; }, { passive: true });
  chip.addEventListener('touchend', (e) => {
    const dx = e.changedTouches[0].clientX - sx, dy = e.changedTouches[0].clientY - sy;
    if (Math.max(Math.abs(dx), Math.abs(dy)) < 24) return;
    swiped = true;
    // swipe down / left = next account, up / right = previous
    cycleAccount((Math.abs(dy) > Math.abs(dx) ? dy > 0 : dx < 0) ? 1 : -1);
  });
  chip.onclick = () => { if (swiped) { swiped = false; return; } openAccountSheet(); };
  $('#list-pane').addEventListener('animationend', () => $('#list-pane').classList.remove('acct-in'));
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
  const waiting = toast('A browser window opened – sign in there to reconnect.', 'info', { label: 'Cancel', ttl: 300000, run: () => invoke('cancel_sign_in') });
  try {
    await withBusy('Waiting for sign-in…', () => invoke('reconnect_account', { accountId }));
    delete S.errors[accountId];
    toast('Reconnected', 'ok');
    await reloadFolders(accountId);
    if (S.cur?.accountId === accountId) syncCurrent();
  } catch (e) { toast(errText(e), 'error'); }
  finally { waiting.remove(); }
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
  const def = S.accounts.find((a) => a.id === S.settings.default_account);
  for (const a of def ? [def, ...S.accounts.filter((x) => x !== def)] : S.accounts) {
    const inbox = (S.folders[a.id] || []).find((f) => f.special === 'inbox');
    if (inbox) return selectFolder(a.id, inbox.name);
  }
}

// ------------------------------------------------------------------ list
/** Rows of the list: one per conversation (newest message first) when conversations are on. */
function listGroups() {
  if (!S.settings.threads || S.searching) return S.messages.map((m) => ({ m, members: [m] }));
  const byThread = new Map();
  const out = [];
  for (const m of S.messages) {             // S.messages is newest first
    const t = m.thread || key(m);
    const g = byThread.get(t);
    if (g) { g.members.push(m); continue; }
    const row = { m, members: [m] };
    byThread.set(t, row);
    out.push(row);
  }
  return out;
}

/** The messages a list row stands for (a whole conversation in this folder). */
const rowMembers = (m) => S.threadMembers?.get(key(m)) || [m];
async function selectFolder(accountId, folder) {
  S.cur = { accountId, folder };
  S.lastFolder[accountId] = folder;
  S.searching = false; S.searchServer = false; S.searchQuery = '';
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
  const filesView = isFilesView();
  $('#app').classList.toggle('files-mode', filesView);
  $('#messages').hidden = filesView;
  $('#files-view').hidden = !filesView;
  if (filesView) {
    $('#list-title').innerHTML = `<b>Files</b> <span class="muted">${esc(a?.email || '')}</span>`;
    $('#list-footer').innerHTML = '';
    updateBulk();
    return renderFilesGrid();
  }
  $('#list-title').innerHTML = S.searching
    ? `<b>${S.searchServer ? 'Server results' : 'Search results'}</b> <span class="muted">${S.messages.length}</span>`
    : S.cur ? `<b>${esc(folderName(S.cur.accountId, S.cur.folder))}</b> <span class="muted">${esc(a?.email || '')}</span>` : '';
  ROWS = []; OFFS = [];
  let y = 0, last = null;
  S.threadMembers = new Map();
  for (const { m, members } of listGroups()) {
    const g = dayGroup(m.date);
    if (g !== last) { ROWS.push({ h: g }); OFFS.push(y); y += HEAD_H; last = g; }
    if (members.length > 1) S.threadMembers.set(key(m), members);
    ROWS.push({ m, members }); OFFS.push(y); y += rowH();
  }
  $('#msg-spacer').style.height = y + 'px';
  renderVisible(true);
  updateBulk();

  const footer = $('#list-footer');
  footer.innerHTML = '';
  if (!S.messages.length) {
    $('#msg-spacer').innerHTML = `<div class="list-empty">${icon(S.searching ? 'search' : 'inbox', 40)}<p>${S.searching ? (S.searchServer ? 'No matches on the server' : 'No matches in downloaded mail') : S.cur ? 'All clear – no messages here' : ''}</p></div>`;
  }
  if (S.searching && !S.searchServer && S.cur && S.searchQuery) {
    const b = document.createElement('button');
    b.className = 'ghost';
    b.textContent = `Search all of “${folderName(S.cur.accountId, S.cur.folder)}” on the server`;
    b.onclick = () => searchOnServer();
    footer.appendChild(b);
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
    const mem = r.members || [m];
    const sp = folderOf(m.account_id, m.folder)?.special;
    const sentLike = sp === 'sent' || sp === 'drafts';
    const names = [...new Set(mem.slice().reverse().map((x) => (x.from_name || x.from_addr || '').split(' ')[0]).filter(Boolean))];
    const who = sentLike ? (m.to ? 'To: ' + m.to : '(no recipient)') : mem.length > 1 ? names.slice(-3).join(', ') : (m.from_name || m.from_addr || '(unknown)');
    const unread = mem.some((x) => !x.seen), flagged = mem.some((x) => x.flagged), attach = mem.some((x) => x.has_attach);
    const firstTo = splitAddrs(m.to || '')[0] || '';
    const toName = firstTo.replace(/<[^>]*>/, '').replace(/"/g, '').trim();
    const k = key(m);
    html += `<div class="msg${unread ? ' unread' : ''}${S.selected.has(k) ? ' selected' : ''}${flagged ? ' flagged' : ''}" data-key="${esc(k)}" style="top:${OFFS[i]}px;height:${rowH()}px">
      ${compact ? '' : avatar(sentLike ? toName : m.from_name, sentLike ? bareAddr(firstTo) : m.from_addr, 36)}
      <div class="m-main">
        <div class="m-top"><span class="from">${esc(who)}</span>${mem.length > 1 ? `<span class="thread-n" title="${mem.length} messages in this conversation">${mem.length}</span>` : ''}<span class="date">${fmtDate(m.date)}</span></div>
        <div class="m-subj">${m.answered ? `<span class="m-ic">${icon('reply', 13)}</span>` : ''}${esc(m.subject || '(no subject)')}${S.searching ? ` <span class="folder-tag">${esc(folderName(m.account_id, m.folder))}</span>` : ''}</div>
        ${compact ? '' : `<div class="m-prev">${esc(m.preview || '')}</div>`}
      </div>
      <div class="m-side">
        ${attach ? `<span class="m-ic">${icon('clip', 14)}</span>` : ''}
        <button class="star-btn ${flagged ? 'on' : ''}" data-q="star" title="Star">${icon('star', 15)}</button>
      </div>
      <div class="m-hover">
        <button data-q="archive" title="Archive">${icon('archive', 16)}</button>
        <button data-q="delete" title="Delete">${icon('trash', 16)}</button>
        <button data-q="read" title="${unread ? 'Mark read' : 'Mark unread'}">${icon(unread ? 'mail' : 'unread', 16)}</button>
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
    const mem = rowMembers(m);
    if (q === 'star') toggleFlag([m], !mem.some((x) => x.flagged));
    if (q === 'archive') archiveMessages(mem);
    if (q === 'delete') deleteMessages(mem);
    if (q === 'read') setSeen(mem, mem.every((x) => x.seen) ? false : true);
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
  const sel = S.messages.filter((m) => S.selected.has(key(m))).flatMap(rowMembers);
  if (sel.length) return [...new Set(sel)];
  if (!S.view) return [];
  const open = S.messages.find((m) => key(m) === key(S.view.summary));
  return open ? rowMembers(open) : [];
}

// ------------------------------------------------------------------ reader
async function openMessage(m, fromConversation = false) {
  try {
    const view = await withBusy('Loading…', () => invoke('open_message', { accountId: m.account_id, folder: m.folder, uid: m.uid }));
    if (!fromConversation && !S.selected.has(key(m))) return;
    // opening a conversation reads all of it
    const others = rowMembers(m).filter((x) => x !== m && !x.seen);
    if (others.length) setSeen(others, true);
    if (!m.seen) {
      m.seen = true;
      const f = folderOf(m.account_id, m.folder);
      if (f && f.unread > 0) { f.unread--; renderSidebar(); }
      renderVisible(true);
    }
    S.allowRemote = S.settings.remote_images === 'always';
    showReader(view);
    loadConversation(view);
    if ($('#app').classList.contains('narrow') && !$('#app').classList.contains('reading')) pushLayer('reader', () => $('#app').classList.remove('reading'));
    $('#app').classList.add('reading');
  } catch (e) { toast(errText(e), 'error'); }
}

/** The whole conversation (Inbox, Sent, Archive…) in order, above the open message. */
async function loadConversation(view) {
  const box = $('#r-thread');
  box.hidden = true;
  box.innerHTML = '';
  const t = view.summary.thread;
  if (!S.settings.threads || !t || !view.summary.account_id) return;
  let list;
  try { list = await invoke('conversation', { accountId: view.summary.account_id, thread: t }); } catch (_) { return; }
  if (S.view !== view || list.length < 2) return;
  const me = new Set(S.accounts.map((a) => a.email.toLowerCase()));
  box.innerHTML = `<div class="th-head">${icon('reply_all', 15)} ${list.length} messages in this conversation</div>` + list.map((m) => {
    const cur = m.folder === view.summary.folder && m.uid === view.summary.uid;
    const mine = me.has((m.from_addr || '').toLowerCase());
    const sp = folderOf(m.account_id, m.folder)?.special;
    return `<div class="th-item${cur ? ' current' : ''}${mine ? ' mine' : ''}" data-folder="${esc(m.folder)}" data-uid="${m.uid}">
      ${avatar(m.from_name, m.from_addr, 28)}
      <div class="th-body"><div class="th-top"><b>${esc(mine ? 'You' : (m.from_name || m.from_addr))}</b>${sp && sp !== 'inbox' ? `<span class="folder-tag">${esc(folderName(m.account_id, m.folder))}</span>` : ''}<small>${fmtDate(m.date, true)}</small></div>
        <div class="th-prev">${cur ? '<i>Open below</i>' : esc(m.preview || m.subject || '')}</div></div>
    </div>`;
  }).join('');
  box.hidden = false;
  box.onclick = (e) => {
    const it = e.target.closest('.th-item');
    if (!it || it.classList.contains('current')) return;
    const m = list.find((x) => x.folder === it.dataset.folder && String(x.uid) === it.dataset.uid);
    if (m) openMessage(m, true);
  };
}

function contactFor(addr) {
  const a = (addr || '').toLowerCase();
  return S.contacts.find((c) => c.email.toLowerCase() === a);
}

function showReader(view) {
  S.view = view;
  $('#reader-empty').hidden = !!view;
  $('#reader-content').hidden = !view;
  if (!view) { const f = $('#r-body'); if (f.hasAttribute('srcdoc')) f.replaceWith(freshFrame(f)); $('#r-thread').hidden = true; $('#app').classList.remove('reading'); return; }
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
  const inJunk = folderOf(s.account_id, s.folder)?.special === 'junk';
  const sb = $('[data-act=spam]');
  sb.title = inJunk ? 'Not spam – move to Inbox (J)' : 'Mark as spam (J)';
  sb.classList.toggle('on', inJunk);
  $('#r-scam').hidden = true;
  showReceiptPrompt(view);

  const atts = $('#r-attachments');
  atts.innerHTML = '';
  for (const a of view.attachments.filter((x) => !x.inline)) {
    const chip = document.createElement('div');
    const risky = RISKY_EXT.test(a.name.trim().replace(/[. ]+$/, ''));
    chip.className = 'att' + (risky ? ' risky' : '');
    const ext = (a.name.split('.').pop() || '').slice(0, 4).toUpperCase();
    chip.innerHTML = `<span class="att-ext">${risky ? icon('shield', 14) : esc(ext)}</span><span class="att-name" title="${risky ? 'Program or macro file – it can only be saved, not opened from here' : 'Open ' + esc(a.name)}">${esc(a.name)}<small>${risky ? 'Can run code · ' : ''}${fmtSize(a.size)}</small></span>
      <button class="icon" data-save title="Save as…">${icon('download', 16)}</button>
      ${folderOf(s.account_id, s.folder)?.special === 'files' ? '' : `<button class="icon" data-tofiles title="Save to the Files folder (syncs to all devices)">${icon('files', 16)}</button>`}`;
    const args = { accountId: s.account_id, folder: s.folder, uid: s.uid, index: a.index };
    const tf = $('[data-tofiles]', chip);
    if (tf) tf.onclick = () => saveToFiles([a.index]);
    $('.att-name', chip).onclick = () => invoke('open_attachment', args).catch((e) => toast(errText(e), 'error'));
    $('[data-save]', chip).onclick = () => invoke('save_attachment', args).then((ok) => ok && toast('Saved', 'ok')).catch((e) => toast(errText(e), 'error'));
    atts.appendChild(chip);
  }
  $('.reader-scroll').scrollTop = 0;
  renderBody();
}

/** Defence in depth on top of the sandbox + CSP: drop tags that can redirect the frame
 *  (meta refresh), re-point links (base) or embed active content. DOMParser is inert:
 *  nothing in the email loads or runs while it is parsed. */
function cleanEmailHtml(html) {
  const d = new DOMParser().parseFromString(html, 'text/html');
  d.querySelectorAll('meta[http-equiv], base, script, object, embed, applet, iframe, frame, frameset, link[rel]:not([rel~="stylesheet"])').forEach((n) => n.remove());
  const styles = [...d.head.querySelectorAll('style')].map((n) => n.outerHTML).join('');
  return styles + d.body.innerHTML;
}

function renderBody() {
  const v = S.view;
  if (!v) return;
  $('#r-remote').hidden = !(v.has_remote && !S.allowRemote);
  renderMailFrame($('#r-body'), v, S.allowRemote, showScamWarning);
}

/** Renders a message body into a sandboxed iframe (CSP, remote-image blocking, scam checks,
 *  safe link handling). Shared by the mail reader and the archive reader. */
function renderMailFrame(frame, v, allowRemote, onScams) {
  const imgSrc = allowRemote ? 'data: https: http:' : 'data:';
  const darkify = S.settings.dark_emails && document.documentElement.dataset.dark === '1';
  const body = v.html ? cleanEmailHtml(v.html) : `<pre style="white-space:pre-wrap;font-family:inherit;margin:0">${esc(v.text)}</pre>`;
  const doc = `<!doctype html><html><head><meta charset="utf-8">
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src ${imgSrc}; style-src 'unsafe-inline'; font-src data:">
    <style>html{background:#fff}body{font-family:"Segoe UI",system-ui,sans-serif;font-size:14px;line-height:1.5;color:#1d2330;margin:0;padding:4px 2px 24px;overflow-wrap:anywhere}
    img{max-width:100%;height:auto}blockquote{border-left:3px solid #ccd;margin:0 0 0 4px;padding-left:12px;color:#555}a{color:#3b5bdb}
    ${darkify ? 'html{filter:invert(.92) hue-rotate(180deg)}img,video,picture,[style*="background-image"]{filter:invert(1) hue-rotate(180deg)}' : ''}</style>
    </head><body>${body}</body></html>`;
  frame.parentElement.classList.toggle('darkified', darkify);
  // A fresh iframe per message: re-assigning srcdoc on the same frame adds a history entry,
  // which made Android's Back step through old email bodies instead of closing the reader.
  const old = frame;
  frame = freshFrame(old);
  frame.onload = () => {
    const d = frame.contentDocument;
    if (!d) return;
    const fit = () => { frame.style.height = Math.max(200, d.documentElement.scrollHeight + 8) + 'px'; };
    fit();
    setTimeout(fit, 300);
    d.querySelectorAll('img').forEach((img) => img.addEventListener('load', fit));
    if (frame.id === 'r-body') swipeBack(d, inScrolledX);
    const scams = S.settings.scam_warn ? scanForScam(d, v) : [];
    onScams(scams);
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
        const bad = scams.find((x) => x.el === a);
        if (bad) {
          confirmBox(`This link may be a scam: ${bad.why}.\n\nIt really goes to:\n${href}`, 'Open anyway', true).then((ok) => {
            if (ok) invoke('open_url', { url: href }).catch((err) => toast(errText(err), 'error'));
          });
          return;
        }
        invoke('open_url', { url: href }).catch((err) => toast(errText(err), 'error'));
      }
    });
  };
  frame.srcdoc = doc;
  old.replaceWith(frame); // srcdoc set before insertion = initial load, no history entry
}

/** An empty copy of a message iframe (same id, sandbox, title) that has never navigated. */
function freshFrame(frame) {
  const f = frame.cloneNode(false);
  f.removeAttribute('srcdoc');
  f.removeAttribute('style');
  return f;
}

/** Mirrors is_dangerous_file() in commands.rs (the backend refuses to open these). */
const RISKY_EXT = /\.(exe|com|scr|pif|bat|cmd|msi|msp|mst|msix|msixbundle|appx|appxbundle|appinstaller|vbs|vbe|js|jse|wsf|wsh|wsc|hta|ps1|psm1|psd1|ps1xml|lnk|url|scf|inf|reg|cpl|msc|jar|jnlp|dll|ocx|sys|chm|hlp|gadget|application|iso|img|vhd|vhdx|docm|dotm|xlsm|xltm|xlam|pptm|potm|ppam|ppsm|sldm|one|xll|library-ms|searchconnector-ms|settingcontent-ms|sh|bash|run|bin|desktop|appimage|deb|rpm|command|app|pkg|dmg|apk|xapk)$/i;

// ------------------------------------------------------------------ scam detection
const hostOf = (u) => { try { return new URL(u).hostname.toLowerCase().replace(/^www\./, ''); } catch (_) { return ''; } };
/** Last two labels (three for co.uk-style) – enough to compare "paypal.com" with "paypal.com.evil.io". */
function siteOf(host) {
  const p = host.split('.');
  return p.slice(p.length > 2 && p[p.length - 2].length <= 3 && p[p.length - 1].length === 2 ? -3 : -2).join('.');
}

/** Returns suspicious links: [{ el, why }]. */
function scanForScam(doc, view) {
  const out = [];
  for (const a of doc.querySelectorAll('a[href]')) {
    const href = a.getAttribute('href').trim();
    if (!/^https?:/i.test(href)) continue;
    const host = hostOf(href);
    if (!host) continue;
    const text = (a.textContent || '').trim();
    const shown = /^(https?:\/\/)?([a-z0-9-]+\.)+[a-z]{2,}(\/|$)/i.exec(text.replace(/\s+/g, ''));
    if (shown) {
      const shownHost = hostOf(/^https?:/i.test(shown[0]) ? shown[0] : 'https://' + shown[0]);
      if (shownHost && siteOf(shownHost) !== siteOf(host)) { out.push({ el: a, why: `the text shows ${shownHost} but it goes to ${host}` }); continue; }
    }
    if (/^\d{1,3}(\.\d{1,3}){3}$/.test(host) || host.startsWith('[')) { out.push({ el: a, why: `it points to a bare IP address (${host}) instead of a website name` }); continue; }
    if (host.split('.').some((l) => l.startsWith('xn--'))) { out.push({ el: a, why: `${host} uses look-alike international letters` }); continue; }
    if (/@/.test(href.replace(/^https?:\/\//i, '').split('/')[0])) { out.push({ el: a, why: 'the address hides its real destination after an "@"' }); }
  }
  // sender name pretending to be an address on another domain ("support@bank.com" <x@evil.io>)
  const s = view.summary;
  const named = /[a-z0-9._%+-]+@([a-z0-9-]+\.)+[a-z]{2,}/i.exec(s.from_name || '');
  if (named && siteOf(named[0].split('@')[1].toLowerCase()) !== siteOf((s.from_addr.split('@')[1] || '').toLowerCase())) {
    out.push({ el: null, why: `the sender's name shows ${named[0]} but the mail really comes from ${s.from_addr}` });
  }
  return out;
}

function showScamWarning(list, bar = $('#r-scam')) {
  bar.hidden = !list.length;
  if (!list.length) return;
  bar.innerHTML = `${icon('shield', 18)}<div><b>This message may be a scam.</b> Don't enter passwords or payment details from its links.
    <ul>${[...new Set(list.map((x) => x.why))].slice(0, 4).map((w) => `<li>${esc(w)}</li>`).join('')}</ul></div>`;
  for (const x of list) if (x.el) { x.el.style.outline = '2px solid #e5484d'; x.el.title = 'Suspicious link: ' + x.why; }
}

// ------------------------------------------------------------------ read receipts
async function showReceiptPrompt(view) {
  const bar = $('#r-receipt');
  bar.hidden = true;
  const s = view.summary;
  if (!view.mdn_to || S.settings.receipts_reply === 'never') return;
  if (S.accounts.some((a) => view.summary.from_addr.toLowerCase() === a.email.toLowerCase())) return; // our own mail
  if (await invoke('receipt_done', { messageId: view.message_id }).catch(() => true)) return;
  if (S.view !== view) return;
  const send = async (yes) => {
    bar.hidden = true;
    try {
      await invoke('read_receipt', { accountId: s.account_id, folder: s.folder, uid: s.uid, messageId: view.message_id, send: yes });
      if (yes) toast('Read receipt sent', 'ok');
    } catch (e) { toast(errText(e), 'error'); }
  };
  if (S.settings.receipts_reply === 'always') return send(true);
  bar.innerHTML = `${icon('check', 16)}<span class="grow"><b>${esc(s.from_name || s.from_addr)}</b> asked to be notified when you read this message.</span>
    <button class="ghost" data-r="no">Ignore</button><button class="primary" data-r="yes">Send receipt</button>`;
  bar.hidden = false;
  bar.onclick = (e) => { const b = e.target.closest('[data-r]'); if (b) send(b.dataset.r === 'yes'); };
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
/** Spam: Junk folder (or delete) – in the Junk folder it's "Not spam" and goes back to the Inbox. */
const spamMessages = (list) => {
  if (!list.length) return;
  const inJunk = folderOf(list[0].account_id, list[0].folder)?.special === 'junk';
  const st = S.settings;
  return removeFromList(list, (g) =>
    invoke('mark_spam', { accountId: g[0].account_id, folder: g[0].folder, uids: g.map((m) => m.uid), spam: !inJunk, action: st.spam_action, markRead: st.spam_mark_read }),
    (n) => inJunk ? `${n > 1 ? n + ' messages' : 'Message'} moved to the Inbox` : st.spam_action === 'delete' ? `${n > 1 ? n + ' messages' : 'Message'} deleted as spam` : `${n > 1 ? n + ' messages' : 'Message'} moved to Junk`);
};

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
  if (act === 'spam') spamMessages(list);
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
const closeReader = () => { if (!dropLayer('reader')) $('#app').classList.remove('reading'); };
$('#btn-back').onclick = closeReader;

/** Phone: slide an open message to the right to go back to the list. */
function swipeBack(el, scrolledX) {
  let x0 = 0, y0 = 0, on = false;
  el.addEventListener('touchstart', (e) => {
    const app = $('#app');
    on = e.touches.length === 1 && app.classList.contains('narrow') && app.classList.contains('reading') && !scrolledX(e.target);
    x0 = e.touches[0].clientX; y0 = e.touches[0].clientY;
  }, { passive: true });
  el.addEventListener('touchend', (e) => {
    if (!on) return;
    on = false;
    const t = e.changedTouches[0], dx = t.clientX - x0, dy = Math.abs(t.clientY - y0);
    if (dx > 90 && dy < dx * 0.4) closeReader();
  }, { passive: true });
}
/** True when the touch starts inside something scrolled sideways (wide tables keep their own swipe). */
const inScrolledX = (node) => {
  for (let n = node; n && n.nodeType === 1; n = n.parentElement) if (n.scrollLeft > 0) return true;
  return false;
};
swipeBack($('#reader'), inScrolledX);
$('#btn-drawer').onclick = () => {
  if ($('#app').classList.toggle('drawer')) pushLayer('drawer', () => $('#app').classList.remove('drawer'));
  else dropLayer('drawer');
};
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
    S.searchQuery = '';
    if (S.searching && S.cur) selectFolder(S.cur.accountId, S.cur.folder);
    return;
  }
  try {
    S.searching = true;
    S.searchServer = false;
    S.searchQuery = q;
    const list = await invoke('search', { accountId: S.cur?.accountId ?? null, query: q });
    if (S.searchQuery !== q) return;   // a newer search started meanwhile
    S.messages = list;
    S.selected.clear();
    $('#messages').scrollTop = 0;
    renderSidebar();
    renderList();
  } catch (err) { toast(errText(err), 'error'); }
}, 250);
$('#search').addEventListener('input', (e) => runSearch(e.target.value.trim()));

/** Searches the whole current folder on the mail server, including mail never downloaded. */
async function searchOnServer() {
  const q = S.searchQuery, cur = S.cur;
  if (!q || !cur) return;
  try {
    const list = await withBusy('Searching the server…', () => invoke('search_server', { accountId: cur.accountId, folder: cur.folder, query: q }));
    if (S.searchQuery !== q || !S.searching) return;
    S.searchServer = true;
    S.messages = list;
    S.selected.clear();
    $('#messages').scrollTop = 0;
    renderList();
  } catch (err) { toast(errText(err), 'error'); }
}
$('#btn-refresh').onclick = () => refreshAll();

/** Re-sync the open folder and re-read every account's folder list (new/renamed folders). */
function refreshAll() {
  syncCurrent();
  for (const a of S.accounts) refreshAccount(a.id);
}

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
  $('#acct-cancel-signin').hidden = acctKind === 'imap';
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
    $('#acct-cancel-signin').hidden = true;
  }
};
$('#acct-cancel-signin').onclick = () => invoke('cancel_sign_in');
