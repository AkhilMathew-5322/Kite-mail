'use strict';
/* Archive tab: mail imported from PST / MSG / EML / MBOX / Maildir / Kite backups.
   Separate from the accounts; full-text search, reading, reply / forward (with attachments),
   copy back into a mailbox, save as .eml. */

const AV = {
  archives: [],
  cur: null,            // { archive, folder } – folder null = whole archive
  list: [],
  offset: 0,
  more: false,
  query: '',
  view: null,
  allowRemote: false,
  importing: false,
};
const AV_PAGE = 200;

async function loadArchives() {
  $('#av-import-folder').hidden = !!S.mobile;   // phones: pick files (no folder picker)
  try { AV.archives = await invoke('archive_list'); } catch (e) { toast(errText(e), 'error'); AV.archives = []; }
  renderArchiveTree();
}

function setAvStep(step) {
  const el = $('#view-archive');
  const order = { nav: 0, list: 1, read: 2 };
  const prev = el.dataset.step;
  el.dataset.step = step;
  // phones: Android Back walks back nav <- list <- message
  if ($('#app').classList.contains('narrow') && order[step] > order[prev]) pushLayer('av-' + step, () => { el.dataset.step = prev; });
}

function renderArchiveTree() {
  const tree = $('#av-tree');
  if (!AV.archives.length) {
    tree.innerHTML = '<p class="hint" style="margin:14px 8px">Nothing imported yet.</p>';
    return;
  }
  tree.innerHTML = AV.archives.map((a) => {
    const isCur = AV.cur?.archive === a.id;
    const folders = a.folders.map((f) => `<div class="av-folder${isCur && AV.cur.folder === f.name ? ' active' : ''}" data-arch="${a.id}" data-folder="${esc(f.name)}" style="padding-left:${14 + (f.name.split('/').length - 1) * 12}px">
        ${icon('folder', 16)}<span title="${esc(f.name)}">${esc(f.name.split('/').pop())}</span><small>${f.count.toLocaleString()}</small></div>`).join('');
    return `<div class="av-arch" data-id="${a.id}">
      <div class="av-arch-head${isCur && AV.cur.folder === null ? ' active' : ''}" data-arch="${a.id}" title="${esc(a.source)}">
        ${icon('database', 17)}<b>${esc(a.name)}</b><small>${a.count.toLocaleString()}</small>
        <button class="icon" data-arch-menu="${a.id}" title="Rename or delete">${icon('menu', 15)}</button></div>
      ${folders}
    </div>`;
  }).join('');
}

async function openArchiveFolder(archiveId, folder) {
  AV.cur = { archive: archiveId, folder };
  AV.query = '';
  $('#av-search').value = '';
  AV.offset = 0;
  AV.list = [];
  renderArchiveTree();
  await loadArchivePage();
  setAvStep('list');
}

async function loadArchivePage() {
  if (!AV.cur) return;
  try {
    const page = await invoke('archive_list_messages', { archiveId: AV.cur.archive, folder: AV.cur.folder, offset: AV.offset, limit: AV_PAGE });
    AV.list = AV.offset ? AV.list.concat(page) : page;
    AV.more = page.length === AV_PAGE;
    AV.offset += page.length;
  } catch (e) { toast(errText(e), 'error'); }
  renderArchiveList();
}

function archiveName(id) { return AV.archives.find((a) => a.id === id)?.name || 'Archive'; }

function renderArchiveList() {
  const title = AV.query
    ? `<b>Results</b> <span class="muted">${AV.list.length}${AV.list.length >= 500 ? '+' : ''}</span>`
    : AV.cur ? `<b>${esc(AV.cur.folder ? AV.cur.folder.split('/').pop() : archiveName(AV.cur.archive))}</b> <span class="muted">${AV.list.length.toLocaleString()}${AV.more ? '+' : ''}</span>` : '';
  $('#av-title').innerHTML = title;
  const box = $('#av-messages');
  if (!AV.list.length) {
    box.innerHTML = `<div class="empty"><p>${AV.query ? 'No matches' : AV.cur ? 'No messages' : 'Pick an archive on the left, or search all archives above.'}</p></div>`;
    return;
  }
  box.innerHTML = AV.list.map((m) => `<div class="av-row${AV.view?.summary.uid === m.id ? ' selected' : ''}" data-id="${m.id}">
      ${avatar(m.from_name, m.from_addr, 36)}
      <div class="body">
        <div class="top"><b>${esc(m.from_name || m.from_addr || '(unknown sender)')}</b>${m.has_attach ? icon('clip', 13) : ''}<small>${fmtDate(m.date)}</small></div>
        <div class="subj">${esc(m.subject || '(no subject)')}${AV.query ? `<span class="where">${esc(archiveName(m.archive_id))} › ${esc(m.folder.split('/').pop())}</span>` : ''}</div>
        <div class="prev">${esc(m.preview)}</div>
      </div></div>`).join('') + (AV.more && !AV.query ? '<button class="ghost av-more" id="av-more">Load more</button>' : '');
}

async function openArchiveMessage(id) {
  try {
    const v = await withBusy('Opening…', () => invoke('archive_open', { id }));
    v.archive_id = AV.list.find((m) => m.id === id)?.archive_id ?? AV.cur?.archive;
    AV.view = v;
    AV.allowRemote = S.settings.remote_images === 'always';
    showArchiveMessage();
    renderArchiveList();
    setAvStep('read');
  } catch (e) { toast(errText(e), 'error'); }
}

function showArchiveMessage() {
  const v = AV.view;
  $('#av-empty').hidden = !!v;
  $('#av-content').hidden = !v;
  if (!v) return;
  const s = v.summary;
  $('#av-subject').textContent = s.subject || '(no subject)';
  $('#av-from').textContent = s.from_name || s.from_addr;
  $('#av-from-addr').textContent = s.from_name ? s.from_addr : '';
  $('#av-avatar').innerHTML = avatar(s.from_name, s.from_addr, 44);
  $('#av-to').textContent = v.to ? 'To ' + v.to : '';
  $('#av-cc').textContent = v.cc ? 'Cc ' + v.cc : '';
  $('#av-date').textContent = fmtDate(s.date, true);
  $('#av-where').textContent = `${archiveName(v.archive_id)} › ${s.folder}`;
  const atts = $('#av-attachments');
  atts.innerHTML = '';
  for (const a of v.attachments.filter((x) => !x.inline)) {
    const risky = RISKY_EXT.test(a.name.trim().replace(/[. ]+$/, ''));
    const chip = document.createElement('div');
    chip.className = 'att' + (risky ? ' risky' : '');
    chip.innerHTML = `<span class="att-ext">${risky ? icon('shield', 14) : esc((a.name.split('.').pop() || '').slice(0, 4).toUpperCase())}</span>
      <span class="att-name" title="${risky ? 'Program or macro file – it can only be saved' : 'Open ' + esc(a.name)}">${esc(a.name)}<small>${risky ? 'Can run code · ' : ''}${fmtSize(a.size)}</small></span>
      <button class="icon" title="Save as…">${icon('download', 16)}</button>`;
    const args = { id: s.uid, index: a.index };
    $('.att-name', chip).onclick = () => invoke('archive_attachment', { ...args, save: false }).catch((e) => toast(errText(e), 'error'));
    $('button', chip).onclick = () => invoke('archive_attachment', { ...args, save: true }).then((ok) => ok && toast('Saved', 'ok')).catch((e) => toast(errText(e), 'error'));
    atts.appendChild(chip);
  }
  $('#av-remote').hidden = !(v.has_remote && !AV.allowRemote);
  renderMailFrame($('#av-body'), v, AV.allowRemote, (list) => showScamWarning(list, $('#av-scam')));
  $('#av-content .reader-scroll').scrollTop = 0;
}

// ------------------------------------------------------------------ import
async function runArchiveImport(kind) {
  if (AV.importing) return;
  AV.importing = true;
  $('#av-import-files').disabled = $('#av-import-folder').disabled = true;
  try {
    const rep = await invoke('archive_import', { kind });
    if (rep) {
      const parts = [`${rep.added.toLocaleString()} message${rep.added === 1 ? '' : 's'} imported`];
      if (rep.skipped) parts.push(`${rep.skipped.toLocaleString()} duplicates skipped`);
      if (rep.failed) parts.push(`${rep.failed.toLocaleString()} could not be read`);
      toast(`${rep.cancelled ? 'Import stopped – ' : ''}${parts.join(' · ')}`, rep.failed ? 'info' : 'ok');
      if (rep.errors.length) console.warn('Import problems:', rep.errors);
      await loadArchives();
      openArchiveFolder(rep.archive_id, null);
    }
  } catch (e) { toast(errText(e), 'error'); }
  finally {
    AV.importing = false;
    $('#av-import-files').disabled = $('#av-import-folder').disabled = false;
    $('#av-progress').hidden = true;
  }
}

function onArchiveProgress(p) {
  const box = $('#av-progress');
  box.hidden = p.finished;
  if (p.finished) return;
  box.innerHTML = `<b>Importing ${esc(p.file)}</b>
    <div class="bar"><i></i></div>
    <div>${p.added.toLocaleString()} messages${p.skipped ? ` · ${p.skipped.toLocaleString()} duplicates` : ''}${p.failed ? ` · ${p.failed} unreadable` : ''}</div>
    ${p.folder ? `<div class="muted small">${esc(p.folder)}</div>` : ''}
    <button class="ghost small" id="av-cancel">Stop</button>`;
  $('#av-cancel').onclick = () => invoke('archive_cancel');
}

// ------------------------------------------------------------------ actions
async function archiveAction(act, btn) {
  const v = AV.view;
  if (!v) return;
  const s = v.summary;
  if (act === 'reply' || act === 'replyall' || act === 'forward') {
    if (!S.accounts.length) return toast('Add a mail account to send from', 'error');
    return openCompose({ mode: act, view: v, archiveId: s.uid });
  }
  if (act === 'eml') return invoke('archive_save_eml', { id: s.uid }).then((ok) => ok && toast('Saved', 'ok')).catch((e) => toast(errText(e), 'error'));
  if (act === 'delete') {
    if (!(await confirmBox('Delete this message from the archive?', 'Delete', true))) return;
    await invoke('archive_delete_messages', { ids: [s.uid] }).catch((e) => toast(errText(e), 'error'));
    AV.list = AV.list.filter((m) => m.id !== s.uid);
    AV.view = null;
    showArchiveMessage();
    renderArchiveList();
    loadArchives();
    return;
  }
  if (act === 'copy') {
    if (!S.accounts.length) return toast('Add a mail account first', 'error');
    let accountId = S.accounts[0].id;
    if (S.accounts.length > 1) {
      const pick = await popMenu(btn, S.accounts.map((a) => ({ label: a.email, value: a.id, icon: 'inbox' })));
      if (pick == null) return;
      accountId = pick;
    }
    const folder = await chooseMoveTarget(btn, accountId, null);
    if (!folder) return;
    try {
      const [added, skipped] = await withBusy('Copying to the mailbox…', () => invoke('archive_copy_to_mailbox', { ids: [s.uid], accountId, folder }));
      toast(added ? `Copied to ${folderName(accountId, folder)}` : skipped ? `Already in ${folderName(accountId, folder)}` : 'Nothing copied', 'ok');
      reloadFolders(accountId);
    } catch (e) { toast(errText(e), 'error'); }
  }
}

async function archiveMenu(id, anchor) {
  const a = AV.archives.find((x) => x.id === id);
  const choice = await popMenu(anchor, [{ label: 'Rename', value: 'rename', icon: 'edit' }, { label: 'Delete archive', value: 'delete', icon: 'trash' }]);
  if (choice === 'rename') {
    const name = prompt('Archive name', a.name);
    if (name && name.trim()) { await invoke('archive_rename', { archiveId: id, name }).catch((e) => toast(errText(e), 'error')); loadArchives(); }
  }
  if (choice === 'delete') {
    if (!(await confirmBox(`Delete the archive "${a.name}" (${a.count.toLocaleString()} messages)?\nThe original files you imported are not touched.`, 'Delete', true))) return;
    await withBusy('Deleting…', () => invoke('archive_delete', { archiveId: id })).catch((e) => toast(errText(e), 'error'));
    if (AV.cur?.archive === id) { AV.cur = null; AV.list = []; AV.view = null; showArchiveMessage(); renderArchiveList(); }
    loadArchives();
  }
}

const runArchiveSearch = debounce(async (q) => {
  AV.query = q;
  if (q.length < 2) {
    if (AV.cur) { AV.offset = 0; await loadArchivePage(); } else { AV.list = []; renderArchiveList(); }
    return;
  }
  try {
    const res = await invoke('archive_search', { archiveId: AV.cur?.archive ?? null, query: q });
    if (AV.query !== q) return;
    AV.list = res;
    AV.more = false;
    renderArchiveList();
    setAvStep('list');
  } catch (e) { toast(errText(e), 'error'); }
}, 250);

// ------------------------------------------------------------------ wiring
$('#av-import-files').onclick = () => runArchiveImport('files');
$('#av-import-folder').onclick = () => runArchiveImport('folder');
$('#av-tree').addEventListener('click', (e) => {
  const menu = e.target.closest('[data-arch-menu]');
  if (menu) { e.stopPropagation(); return archiveMenu(Number(menu.dataset.archMenu), menu); }
  const f = e.target.closest('.av-folder');
  if (f) return openArchiveFolder(Number(f.dataset.arch), f.dataset.folder);
  const h = e.target.closest('.av-arch-head');
  if (h) openArchiveFolder(Number(h.dataset.arch), null);
});
$('#av-messages').addEventListener('click', (e) => {
  if (e.target.closest('#av-more')) return loadArchivePage();
  const row = e.target.closest('.av-row');
  if (row) openArchiveMessage(Number(row.dataset.id));
});
$('#av-search').addEventListener('input', (e) => runArchiveSearch(e.target.value.trim()));
$('.av-bar').addEventListener('click', (e) => {
  const b = e.target.closest('[data-av]');
  if (b) archiveAction(b.dataset.av, b);
});
$('#av-load-images').onclick = () => { AV.allowRemote = true; showArchiveMessage(); };
$('#av-back-nav').onclick = () => setAvStep('nav');
$('#av-back-list').onclick = () => setAvStep('list');
