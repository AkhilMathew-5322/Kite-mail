'use strict';
/* Compose: reply/forward prefill, address autocomplete, draft autosave, grammar check, signatures. */

const AUTOSAVE_MS = 20000;

function prefixSubject(prefix, subject) {
  const s = subject || '';
  return new RegExp('^' + prefix + '\\s*:', 'i').test(s) ? s : `${prefix}: ${s}`;
}

function signatureFor(accountId) {
  const sig = (S.settings.signatures || {})[accountId];
  return sig ? `\n\n-- \n${sig}` : '';
}

/**
 * Opens the compose sheet.
 * opts: { mode: new|reply|replyall|forward|draft, view, to, cc, bcc, subject, body, accountId, draftRef, localId, messageId, attachments }
 */
function openCompose(opts = {}) {
  if (!S.accounts.length) { toast('Add an account first', 'error'); return; }
  let { mode = 'new', view = null, to = '', cc = '', bcc = '', subject = '', body = '' } = opts;
  const fromSel = $('#c-from');
  fromSel.innerHTML = S.accounts.map((a) => `<option value="${a.id}">${esc(a.name)} &lt;${esc(a.email)}&gt;</option>`).join('');
  const accountId = opts.accountId ?? view?.summary.account_id ?? S.cur?.accountId ?? S.accounts[0].id;
  fromSel.value = String(accountId);

  S.compose = {
    mode, in_reply_to: null, references: null, forward_from: null, reply_to_msg: null, files: [], fwdNames: [],
    draft: opts.draftRef || null, local_draft_id: opts.localId || null, message_id: opts.messageId || null,
    dirty: false, saving: false, timer: null, issues: [],
  };
  if (view) {
    const s = view.summary;
    const ref = { folder: s.folder, uid: s.uid };
    const who = fmtAddr(s.from_name, s.from_addr);
    const quoted = messageText(view).split('\n').map((l) => (l ? '> ' + l : '>')).join('\n');
    if (mode === 'reply' || mode === 'replyall') {
      to = view.reply_to || who;
      if (mode === 'replyall') {
        const me = new Set(S.accounts.map((a) => a.email.toLowerCase()));
        const seen = new Set(splitAddrs(to).map(bareAddr));
        cc = splitAddrs(`${view.to}, ${view.cc}`).filter((x) => {
          const b = bareAddr(x);
          if (!b || me.has(b) || seen.has(b)) return false;
          seen.add(b); return true;
        }).join(', ');
      }
      subject = prefixSubject('Re', s.subject);
      body = `${signatureFor(accountId)}\n\nOn ${fmtDate(s.date, true)}, ${who} wrote:\n${quoted}`;
      S.compose.in_reply_to = view.message_id || null;
      S.compose.references = view.references || null;
      S.compose.reply_to_msg = ref;
    } else if (mode === 'forward') {
      subject = prefixSubject('Fwd', s.subject);
      body = `${signatureFor(accountId)}\n\n---------- Forwarded message ----------\nFrom: ${who}\nDate: ${fmtDate(s.date, true)}\nSubject: ${s.subject}\nTo: ${view.to}\n${view.cc ? 'Cc: ' + view.cc + '\n' : ''}\n${messageText(view)}`;
      S.compose.forward_from = ref;
      S.compose.fwdNames = view.attachments.filter((a) => !a.inline).map((a) => a.name);
    }
  } else if (mode === 'new' && !body) {
    body = signatureFor(accountId);
  }
  if (mode === 'draft') {
    S.compose.in_reply_to = opts.in_reply_to || null;
    S.compose.references = opts.references || null;
    S.compose.fwdNames = opts.attachments || [];
    if (opts.draftRef) S.compose.forward_from = opts.draftRef; // keep the draft's own attachments when sending
  }
  $('#compose-title').textContent = { reply: 'Reply', replyall: 'Reply all', forward: 'Forward', draft: 'Draft' }[mode] || 'New message';
  $('#c-to').value = to;
  $('#c-cc').value = cc;
  $('#c-bcc').value = bcc;
  $$('.field.cc').forEach((f) => (f.hidden = !(cc || bcc)));
  $('#c-subject').value = subject;
  $('#c-body').value = body;
  $('#c-status').textContent = S.compose.draft ? 'Saved draft' : '';
  $('#c-grammar').hidden = true;
  renderComposeFiles();
  $('#compose').showModal();
  if (S.mobile || $('#app').classList.contains('narrow')) pushLayer('compose', () => { if (S.compose) closeCompose(); });
  const ta = $('#c-body');
  if (to) { ta.focus(); ta.setSelectionRange(0, 0); ta.scrollTop = 0; } else $('#c-to').focus();
}

async function openDraftMessage(m) {
  try {
    const d = await withBusy('Opening draft…', () => invoke('open_draft', { accountId: m.account_id, folder: m.folder, uid: m.uid }));
    openCompose({
      mode: 'draft', accountId: d.account_id, to: d.to, cc: d.cc, bcc: d.bcc, subject: d.subject, body: d.body,
      draftRef: { folder: m.folder, uid: m.uid }, messageId: d.message_id,
      in_reply_to: d.in_reply_to, references: d.references, attachments: d.attachments,
    });
  } catch (e) { toast(errText(e), 'error'); }
}

function composePayload() {
  const c = S.compose;
  return {
    account_id: Number($('#c-from').value),
    to: $('#c-to').value, cc: $('#c-cc').value, bcc: $('#c-bcc').value,
    subject: $('#c-subject').value, body: $('#c-body').value,
    in_reply_to: c.in_reply_to, references: c.references,
    attachments: c.files, forward_from: c.forward_from, reply_to_msg: c.reply_to_msg,
    draft: c.draft, local_draft_id: c.local_draft_id, message_id: c.message_id,
  };
}

function renderComposeFiles() {
  const box = $('#c-attachments');
  box.innerHTML = '';
  for (const n of S.compose.fwdNames) {
    const c = document.createElement('div');
    c.className = 'att small-att';
    c.innerHTML = `${icon('clip', 14)}<span>${esc(n)}</span><small class="muted">included</small>`;
    box.appendChild(c);
  }
  S.compose.files.forEach((p, i) => {
    const c = document.createElement('div');
    c.className = 'att small-att';
    c.innerHTML = `${icon('clip', 14)}<span title="${esc(p)}">${esc(p.split(/[\\/]/).pop())}</span><button class="icon" title="Remove">${icon('x', 14)}</button>`;
    $('button', c).onclick = () => { S.compose.files.splice(i, 1); markDirty(); renderComposeFiles(); };
    box.appendChild(c);
  });
}

// ------------------------------------------------------------------ drafts
function markDirty() {
  const c = S.compose;
  if (!c) return;
  c.dirty = true;
  $('#c-status').textContent = 'Editing…';
  clearTimeout(c.timer);
  c.timer = setTimeout(() => saveDraft(), AUTOSAVE_MS);
}

async function saveDraft(quiet = true) {
  const c = S.compose;
  if (!c || c.saving || c.sending || !c.dirty) return true;
  c.saving = true;
  clearTimeout(c.timer);
  $('#c-status').textContent = 'Saving draft…';
  let done;
  c.savePromise = new Promise((r) => { done = r; });
  try {
    const r = await invoke('save_draft', { draft: composePayload() });
    if (S.compose !== c) return true;
    c.local_draft_id = r.local_draft_id;
    c.message_id = r.message_id;
    if (r.draft) c.draft = r.draft;
    c.dirty = false;
    const t = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    $('#c-status').textContent = r.server_error ? `Saved on this device ${t} (server: offline)` : `Draft saved ${t}`;
    if (!quiet) toast(r.server_error ? 'Draft saved on this device – will sync when online' : 'Draft saved', 'ok');
    return true;
  } catch (e) {
    $('#c-status').textContent = 'Draft not saved';
    if (!quiet) toast(errText(e), 'error');
    return false;
  } finally { c.saving = false; c.savePromise = null; done(); }
}

async function closeCompose() {
  const c = S.compose;
  const empty = !$('#c-to').value.trim() && !$('#c-subject').value.trim() && $('#c-body').value.trim() === signatureFor(Number($('#c-from').value)).trim();
  if (c && c.dirty && !empty) {
    const choice = await choiceBox('Keep this message as a draft?', [
      { value: 'discard', label: 'Discard', cls: 'danger' }, { value: 'cancel', label: 'Keep editing' }, { value: 'save', label: 'Save draft', cls: 'primary' },
    ]);
    if (choice === 'cancel' || choice === null) return;
    if (choice === 'save') { await saveDraft(false); }
    if (choice === 'discard') await discardDraft(false);
  }
  clearTimeout(c?.timer);
  S.compose = null;
  $('#compose').close();
  dropLayer('compose');
  if (S.cur && folderOf(S.cur.accountId, S.cur.folder)?.special === 'drafts') syncCurrent(true);
}

async function discardDraft(close = true) {
  const c = S.compose;
  if (!c) return;
  if (close && !(await confirmBox('Discard this message?', 'Discard', true))) return;
  try {
    await invoke('discard_draft', { accountId: Number($('#c-from').value), draft: c.draft, localDraftId: c.local_draft_id });
  } catch (e) { toast(errText(e), 'error'); }
  if (close) {
    clearTimeout(c.timer);
    S.compose = null;
    $('#compose').close();
    toast('Draft discarded');
    if (S.cur && folderOf(S.cur.accountId, S.cur.folder)?.special === 'drafts') syncCurrent(true);
  }
}

async function sendCompose() {
  const btn = $('#c-send');
  const c = S.compose;
  if (!c || c.sending) return;
  let draft = composePayload();
  if (!draft.to.trim() && !draft.cc.trim() && !draft.bcc.trim()) { toast('Add at least one recipient', 'error'); $('#c-to').focus(); return; }
  if (!draft.subject.trim() && !(await confirmBox('Send without a subject?', 'Send'))) return;
  if (S.nc.enabled && draft.attachments.length) {
    const sizes = await invoke('file_sizes', { paths: draft.attachments }).catch(() => []);
    const limit = (S.nc.link_threshold_mb || 10) * 1048576;
    const big = sizes.filter((n) => n >= limit);
    if (big.length) {
      const mb = (big.reduce((a, b) => a + b, 0) / 1048576).toFixed(1);
      const choice = await choiceBox(`${big.length} attachment${big.length > 1 ? 's are' : ' is'} large (${mb} MB). Many mail servers reject big emails.\nUpload ${big.length > 1 ? 'them' : 'it'} to Nextcloud and send a download link instead?`,
        [{ value: 'cancel', label: 'Cancel' }, { value: 'attach', label: 'Attach anyway' }, { value: 'link', label: 'Send as link', cls: 'primary' }], 'Large attachments');
      if (choice === 'cancel' || choice === null) return;
      draft.link_large = choice === 'link';
    }
  }
  btn.disabled = true;
  c.sending = true;            // blocks autosave from re-creating the draft after it is sent
  clearTimeout(c.timer);
  try {
    if (c.savePromise) {       // an autosave is mid-flight: let it finish so we delete the right server draft
      await c.savePromise;
      draft = { ...composePayload(), link_large: draft.link_large };
    }
    await withBusy(draft.link_large ? 'Uploading to Nextcloud & sending…' : 'Sending…', () => invoke('send_message', { draft }));
    $('#compose').close();
    S.compose = null;
    dropLayer('compose');
    toast('Message sent', 'ok');
    if (draft.reply_to_msg) {
      const m = S.messages.find((x) => x.account_id === draft.account_id && x.folder === draft.reply_to_msg.folder && x.uid === draft.reply_to_msg.uid);
      if (m) { m.answered = true; renderVisible(true); }
    }
    if (S.cur && ['drafts', 'sent'].includes(folderOf(S.cur.accountId, S.cur.folder)?.special)) syncCurrent(true);
  } catch (e) { c.sending = false; toast(errText(e), 'error'); }
  finally { btn.disabled = false; }
}

/** Drafts saved locally that never reached the server (e.g. closed while offline). */
async function recoverDrafts() {
  try {
    const list = await invoke('pending_drafts');
    if (!list.length) return;
    toast(`${list.length} unsent draft${list.length > 1 ? 's' : ''} recovered`, 'info', {
      label: 'Open', run: () => {
        const d = list[0];
        openCompose({ mode: 'draft', accountId: d.account_id, to: d.to, cc: d.cc, bcc: d.bcc, subject: d.subject, body: d.body,
          localId: d.local_draft_id, messageId: d.message_id, in_reply_to: d.in_reply_to, references: d.references });
        S.compose.files = d.attachments || [];
        S.compose.dirty = true;
        renderComposeFiles();
      },
    });
  } catch (_) { /* ignore */ }
}

// ------------------------------------------------------------------ address autocomplete
let acTarget = null, acItems = [], acIndex = 0;
const acMenu = $('#ac-menu');

const fetchSuggestions = debounce(async (input) => {
  const parts = input.value.split(/[,;]/);
  const q = parts[parts.length - 1].trim();
  if (q.length < 1) { hideAc(); return; }
  try { acItems = await invoke('suggest_addresses', { query: q }); } catch (_) { acItems = []; }
  if (!acItems.length || document.activeElement !== input) { hideAc(); return; }
  acTarget = input; acIndex = 0;
  renderAc();
}, 120);

function renderAc() {
  const r = acTarget.getBoundingClientRect();
  const fr = $('#compose-form').getBoundingClientRect();
  acMenu.style.left = (r.left - fr.left) + 'px';
  acMenu.style.top = (r.bottom - fr.top + 4) + 'px';
  acMenu.style.width = r.width + 'px';
  acMenu.innerHTML = acItems.map((s, i) => `<div class="ac-item ${i === acIndex ? 'active' : ''}" data-i="${i}">
    ${avatar(s.name, s.email, 26)}<div><b>${esc(s.name || s.email)}</b><small>${esc(s.email)}</small></div>${s.is_contact ? `<span class="ac-tag">${icon('users', 13)}</span>` : ''}</div>`).join('');
  acMenu.hidden = false;
}
function hideAc() { acMenu.hidden = true; acTarget = null; }
function pickAc(i) {
  const s = acItems[i];
  if (!s || !acTarget) return;
  const kept = splitAddrs(acTarget.value);
  kept.pop();
  kept.push(fmtAddr(s.name, s.email));
  acTarget.value = kept.join(', ') + ', ';
  acTarget.focus();
  hideAc();
  markDirty();
}
acMenu.addEventListener('mousedown', (e) => { const it = e.target.closest('.ac-item'); if (it) { e.preventDefault(); pickAc(Number(it.dataset.i)); } });
for (const id of ['#c-to', '#c-cc', '#c-bcc']) {
  const el = $(id);
  el.addEventListener('input', () => fetchSuggestions(el));
  el.addEventListener('blur', () => setTimeout(hideAc, 150));
  el.addEventListener('keydown', (e) => {
    if (acMenu.hidden || acTarget !== el) return;
    if (e.key === 'ArrowDown') { e.preventDefault(); acIndex = (acIndex + 1) % acItems.length; renderAc(); }
    if (e.key === 'ArrowUp') { e.preventDefault(); acIndex = (acIndex - 1 + acItems.length) % acItems.length; renderAc(); }
    if (e.key === 'Enter' || e.key === 'Tab') { e.preventDefault(); pickAc(acIndex); }
    if (e.key === 'Escape') { e.stopPropagation(); e.preventDefault(); hideAc(); }
  });
}

// ------------------------------------------------------------------ grammar (LanguageTool)
async function checkGrammar() {
  const text = $('#c-body').value;
  const quoteStart = text.search(/\n(On .+wrote:|---------- Forwarded message|-- \n)/);
  const own = quoteStart > 0 ? text.slice(0, quoteStart) : text;
  const box = $('#c-grammar');
  box.hidden = false;
  box.innerHTML = `<div class="g-head">${icon('type', 16)} Checking…</div>`;
  try {
    const issues = await invoke('check_grammar', { text: own });
    S.compose.issues = issues;
    renderGrammar();
  } catch (e) {
    box.innerHTML = `<div class="g-head error">${icon('junk', 16)} ${esc(errText(e))}</div>`;
  }
}

function renderGrammar() {
  const box = $('#c-grammar');
  const issues = S.compose.issues;
  const text = $('#c-body').value;
  if (!issues.length) {
    box.innerHTML = `<div class="g-head ok">${icon('check', 16)} No issues found <button class="icon" id="g-close">${icon('x', 14)}</button></div>`;
  } else {
    box.innerHTML = `<div class="g-head">${icon('type', 16)} ${issues.length} suggestion${issues.length > 1 ? 's' : ''}<span class="grow"></span><button class="icon" id="g-close">${icon('x', 14)}</button></div>` +
      issues.map((it, i) => {
        const before = text.slice(Math.max(0, it.offset - 25), it.offset);
        const bad = text.slice(it.offset, it.offset + it.length);
        const after = text.slice(it.offset + it.length, it.offset + it.length + 25);
        return `<div class="g-item"><div class="g-ctx">…${esc(before)}<mark>${esc(bad)}</mark>${esc(after)}…</div>
          <div class="g-msg">${esc(it.message)}</div>
          <div class="g-fixes">${it.replacements.map((r, j) => `<button class="chip" data-i="${i}" data-j="${j}">${esc(r || '(remove)')}</button>`).join('')}
          <button class="link small" data-skip="${i}">Ignore</button></div></div>`;
      }).join('');
  }
  $('#g-close').onclick = () => { box.hidden = true; };
}

$('#c-grammar').addEventListener('click', (e) => {
  const b = e.target.closest('button');
  if (!b) return;
  const issues = S.compose.issues;
  if (b.dataset.skip !== undefined) { issues.splice(Number(b.dataset.skip), 1); renderGrammar(); return; }
  if (b.dataset.i === undefined) return;
  const it = issues[Number(b.dataset.i)];
  const rep = it.replacements[Number(b.dataset.j)];
  const ta = $('#c-body');
  ta.value = ta.value.slice(0, it.offset) + rep + ta.value.slice(it.offset + it.length);
  const delta = rep.length - it.length;
  issues.splice(Number(b.dataset.i), 1);
  for (const o of issues) if (o.offset > it.offset) o.offset += delta;
  markDirty();
  renderGrammar();
});

// ------------------------------------------------------------------ wiring
$('#btn-compose').onclick = () => openCompose();
$('#compose-close').onclick = () => closeCompose();
$('#compose').addEventListener('cancel', (e) => { e.preventDefault(); if (acMenu.hidden) closeCompose(); });
$('#c-send').onclick = sendCompose;
$('#c-discard').onclick = () => discardDraft(true);
$('#c-check').onclick = checkGrammar;
$('#c-show-cc').onclick = () => $$('.field.cc').forEach((f) => (f.hidden = false));
$('#c-attach').onclick = async () => {
  try {
    const files = await invoke('pick_files');
    if (files.length) { S.compose.files.push(...files); markDirty(); renderComposeFiles(); }
  } catch (e) { toast(errText(e), 'error'); }
};
$('#c-from').addEventListener('change', markDirty);
for (const id of ['#c-to', '#c-cc', '#c-bcc', '#c-subject', '#c-body']) $(id).addEventListener('input', markDirty);
$('#compose').addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); sendCompose(); }
  if (e.key.toLowerCase() === 's' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); if (S.compose) { S.compose.dirty = true; saveDraft(false); } }
});
