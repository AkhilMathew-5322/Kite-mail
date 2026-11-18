'use strict';
/* The Files folder as a gallery: pictures, PDFs, Word and other documents collected from mail
   (or added from this device). It is a real folder on the mail server – every file is a small
   message with the file attached – so it syncs to every device and shows in any mail app. */

const FILE_KINDS = [
  ['all', 'All'], ['images', 'Pictures'], ['pdf', 'PDF'], ['word', 'Documents'], ['excel', 'Sheets'], ['other', 'Other'],
];
const THUMBS = new Map();          // "acct|folder|uid" -> small data URL (session cache)
let filesFilter = 'all';
let thumbObserver = null;

function fileKind(name) {
  const ext = (name.split('.').pop() || '').toLowerCase();
  if (['jpg', 'jpeg', 'png', 'gif', 'webp', 'heic', 'heif', 'bmp', 'tif', 'tiff'].includes(ext)) return 'images';
  if (ext === 'pdf') return 'pdf';
  if (['doc', 'docx', 'odt', 'rtf', 'pages', 'txt'].includes(ext)) return 'word';
  if (['xls', 'xlsx', 'ods', 'csv', 'numbers', 'ppt', 'pptx', 'odp', 'key'].includes(ext)) return 'excel';
  return 'other';
}

function isFilesView() {
  return !!S.cur && !S.searching && folderOf(S.cur.accountId, S.cur.folder)?.special === 'files';
}

function renderFilesGrid() {
  const counts = { all: S.messages.length };
  for (const m of S.messages) { const k = fileKind(m.subject); counts[k] = (counts[k] || 0) + 1; }
  $('#files-filter').innerHTML = FILE_KINDS.filter(([k]) => k === 'all' || counts[k])
    .map(([k, l]) => `<button data-v="${k}" class="${filesFilter === k ? 'active' : ''}">${l}${counts[k] ? ` <small>${counts[k]}</small>` : ''}</button>`).join('');
  const list = S.messages.filter((m) => filesFilter === 'all' || fileKind(m.subject) === filesFilter);
  const grid = $('#files-grid');
  if (!S.messages.length) {
    grid.innerHTML = `<div class="list-empty">${icon('files', 40)}<p><b>Your Files folder</b><br>Pictures, PDFs and Word files from your mail are collected here and sync to all your devices –
      also in Outlook, Thunderbird or your phone's mail app.</p></div>`;
    return;
  }
  grid.innerHTML = list.map((m) => {
    const k = fileKind(m.subject);
    const ext = (m.subject.split('.').pop() || '').slice(0, 4).toUpperCase();
    const id = `${m.account_id}|${m.folder}|${m.uid}`;
    const thumb = THUMBS.get(id);
    return `<div class="file-card kind-${k}" data-uid="${m.uid}" data-id="${esc(id)}" title="${esc(m.subject)}">
      <div class="fc-thumb">${thumb ? `<img src="${thumb}" alt="">` : `<span class="fc-ext">${k === 'images' ? icon('image', 30) : esc(ext || 'FILE')}</span>`}</div>
      <div class="fc-name">${esc(m.subject || 'file')}</div>
      <div class="fc-meta">${fmtSize(m.size)} · ${fmtDate(m.date)}</div>
      <div class="fc-actions">
        <button class="icon" data-f="save" title="Save to this device">${icon('download', 15)}</button>
        <button class="icon" data-f="send" title="Send by email">${icon('send', 15)}</button>
        <button class="icon" data-f="delete" title="Delete">${icon('trash', 15)}</button>
      </div>
    </div>`;
  }).join('') || '<div class="list-empty"><p>No files of this kind yet.</p></div>';
  observeThumbs();
}

function observeThumbs() {
  thumbObserver?.disconnect();
  thumbObserver = new IntersectionObserver((entries) => {
    for (const en of entries) {
      if (!en.isIntersecting) continue;
      const card = en.target;
      thumbObserver.unobserve(card);
      loadThumb(card);
    }
  }, { root: $('#files-grid'), rootMargin: '200px' });
  $$('#files-grid .file-card.kind-images').forEach((c) => { if (!THUMBS.has(c.dataset.id)) thumbObserver.observe(c); });
}

async function loadThumb(card) {
  const [accountId, folder, uid] = card.dataset.id.split('|');
  try {
    const url = await invoke('files_thumbnail', { accountId: Number(accountId), folder, uid: Number(uid) });
    if (!url) return;
    const small = await shrinkImage(url, 360);
    THUMBS.set(card.dataset.id, small);
    const box = $('.fc-thumb', card);
    if (box) box.innerHTML = `<img src="${small}" alt="">`;
  } catch (_) { /* keep the icon */ }
}

/** Downscales a picture to a small JPEG so the gallery stays light on memory. */
function shrinkImage(url, max) {
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => {
      const k = Math.min(1, max / Math.max(img.width, img.height));
      const c = document.createElement('canvas');
      c.width = Math.max(1, Math.round(img.width * k)); c.height = Math.max(1, Math.round(img.height * k));
      c.getContext('2d').drawImage(img, 0, 0, c.width, c.height);
      resolve(c.toDataURL('image/jpeg', 0.8));
    };
    img.onerror = () => resolve(url);
    img.src = url;
  });
}

async function fileAction(act, card) {
  const m = S.messages.find((x) => String(x.uid) === card.dataset.uid);
  if (!m) return;
  const args = { accountId: m.account_id, folder: m.folder, uid: m.uid };
  if (act === 'open') return invoke('open_attachment', { ...args, index: 0 }).catch((e) => toast(errText(e), 'error'));
  if (act === 'save') return invoke('save_attachment', { ...args, index: 0 }).then((ok) => ok && toast('Saved', 'ok')).catch((e) => toast(errText(e), 'error'));
  if (act === 'send') {
    try {
      const path = await invoke('files_to_temp', args);
      await openCompose({ accountId: m.account_id, subject: m.subject });
      S.compose.files.push(path);
      renderComposeFiles();
    } catch (e) { toast(errText(e), 'error'); }
    return;
  }
  if (act === 'delete') {
    if (!(await confirmBox(`Delete "${m.subject}" from the Files folder?\nIt is removed on all your devices.`, 'Delete', true))) return;
    return deleteMessages([m]);
  }
}

/** Reader: file one or all attachments of the open message into the Files folder. */
async function saveToFiles(indexes) {
  const v = S.view;
  if (!v) return;
  const s = v.summary;
  try {
    const [added, skipped] = await withBusy('Saving to Files…', () => invoke('files_save_attachments', { accountId: s.account_id, folder: s.folder, uid: s.uid, indexes }));
    toast(added ? `Saved to Files${skipped ? ` (${skipped} already there)` : ''}` : 'Already in Files', 'ok');
    reloadFolders(s.account_id);
  } catch (e) { toast(errText(e), 'error'); }
}

$('#files-filter').addEventListener('click', (e) => {
  const b = e.target.closest('button[data-v]');
  if (!b) return;
  filesFilter = b.dataset.v;
  renderFilesGrid();
});
$('#files-grid').addEventListener('click', (e) => {
  const card = e.target.closest('.file-card');
  if (!card) return;
  const b = e.target.closest('[data-f]');
  fileAction(b ? b.dataset.f : 'open', card);
});
$('#files-add').onclick = async () => {
  if (!S.cur) return;
  try {
    const r = await invoke('files_add_local', { accountId: S.cur.accountId });
    if (!r) return;
    toast(r[0] ? `${r[0]} file${r[0] > 1 ? 's' : ''} added${r[1] ? `, ${r[1]} already there` : ''}` : 'Already in Files', 'ok');
    syncCurrent();
  } catch (e) { toast(errText(e), 'error'); }
};
