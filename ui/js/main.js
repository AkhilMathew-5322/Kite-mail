'use strict';
/* App shell: tabs, keyboard shortcuts, backend events, startup. */

function switchTab(tab) {
  S.tab = tab;
  $('#app').dataset.tab = tab;
  $('#app').classList.remove('drawer');
  $$('.rail-btn[data-tab]').forEach((b) => b.classList.toggle('active', b.dataset.tab === tab));
  if (tab === 'contacts') { loadContacts().then(() => { S.contactsView = S.contacts; renderContactList(); renderContactDetail(); }); }
  if (tab === 'tasks') { loadTodos().then(renderTasks); }
  if (tab === 'settings') renderSettings();
  if (tab === 'mail') requestAnimationFrame(() => renderVisible(true));
}

$$('.rail-btn[data-tab]').forEach((b) => (b.onclick = () => switchTab(b.dataset.tab)));
$('#btn-theme').onclick = async () => {
  await saveSettings({ mode: isDarkMode() ? 'light' : 'dark' });
  applyTheme();
  if (S.tab === 'settings') renderSettings();
};

// ------------------------------------------------------------------ keyboard
document.addEventListener('keydown', (e) => {
  if (document.querySelector('dialog[open]')) return;
  const typing = ['INPUT', 'TEXTAREA', 'SELECT'].includes(document.activeElement?.tagName);
  if (e.key === 'F5') { e.preventDefault(); refreshAll(); return; }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'n') { e.preventDefault(); openCompose(); return; }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f' && S.tab === 'mail') { e.preventDefault(); $('#search').focus(); return; }
  if (typing || S.tab !== 'mail' || e.ctrlKey || e.metaKey || e.altKey) return;
  const list = selectedMessages();
  switch (e.key) {
    case 'Delete': e.preventDefault(); deleteMessages(list); break;
    case 'e': e.preventDefault(); archiveMessages(list); break;
    case 'u': e.preventDefault(); setSeen(list, false); break;
    case 's': e.preventDefault(); toggleFlag(list, !(list[0]?.flagged)); break;
    case 't': if (S.view) { e.preventDefault(); quickTaskFromMail(S.view); } break;
    case 'r': if (S.view) { e.preventDefault(); openCompose({ mode: 'reply', view: S.view }); } break;
    case 'a': if (S.view) { e.preventDefault(); openCompose({ mode: 'replyall', view: S.view }); } break;
    case 'f': if (S.view) { e.preventDefault(); openCompose({ mode: 'forward', view: S.view }); } break;
    case 'Escape': $('#app').classList.remove('reading'); break;
    case 'ArrowDown': case 'ArrowUp': case 'j': case 'k': {
      e.preventDefault();
      const keys = S.messages.map(key);
      const curIdx = S.view ? keys.indexOf(key(S.view.summary)) : -1;
      const step = e.key === 'ArrowDown' || e.key === 'j' ? 1 : -1;
      const m = S.messages[Math.max(0, Math.min(keys.length - 1, curIdx + step))];
      if (!m) break;
      S.selected = new Set([key(m)]);
      S.anchor = key(m);
      const i = ROWS.findIndex((r) => r.m === m);
      const box = $('#messages');
      if (i >= 0 && (OFFS[i] < box.scrollTop || OFFS[i] + rowH() > box.scrollTop + box.clientHeight)) box.scrollTop = OFFS[i] - box.clientHeight / 3;
      renderVisible(true);
      openMessage(m);
      break;
    }
    default:
  }
});

// ------------------------------------------------------------------ backend events
listen('mail-updated', async ({ payload }) => {
  const { account_id, folder } = payload;
  delete S.errors[account_id];
  await reloadFolders(account_id);
  if (S.cur && S.cur.accountId === account_id && S.cur.folder === folder) reloadCurrentFromCache();
});
listen('account-error', ({ payload }) => {
  S.errors[payload.account_id] = payload.error;
  renderSidebar();
});
listen('todos-updated', async () => {
  await loadTodos();
  if (S.tab === 'tasks') renderTasks();
});
listen('backup-progress', ({ payload }) => onBackupProgress(payload));
listen('nc-synced', async ({ payload }) => {
  await loadNc();
  const changed = payload.contacts_down + payload.tasks_down + payload.deleted;
  if (!changed) return;
  await loadContacts(); await loadTodos();
  if (S.tab === 'contacts') { S.contactsView = S.contacts; renderContactList(); renderContactDetail(); }
  if (S.tab === 'tasks') renderTasks();
});
listen('nc-error', async () => { await loadNc(); if (S.tab === 'settings') renderSettings(); });
listen('pair-event', ({ payload }) => onPairEvent(payload));

// ------------------------------------------------------------------ responsive layout
const narrowQuery = window.matchMedia('(max-width: 760px)');
const applyNarrow = () => { $('#app').classList.toggle('narrow', narrowQuery.matches); document.body.classList.toggle('narrow', narrowQuery.matches); };
narrowQuery.addEventListener('change', applyNarrow);
applyNarrow();

// ------------------------------------------------------------------ startup
async function init() {
  hydrateIcons();
  try { Object.assign(S.settings, DEFAULT_SETTINGS, await invoke('get_settings')); } catch (_) { /* defaults */ }
  applyTheme();
  try { const info = await invoke('app_info'); S.mobile = !!info.mobile; S.version = info.version; } catch (_) { /* desktop */ }
  document.body.classList.toggle('mobile', S.mobile);
  await loadNc();
  try {
    S.accounts = await invoke('list_accounts');
    for (const a of S.accounts) S.folders[a.id] = await invoke('list_folders', { accountId: a.id });
  } catch (e) { toast(errText(e), 'error'); }
  renderSidebar();
  loadContacts();
  loadTodos();
  if (!S.accounts.length) { renderList(); if (S.mobile) mobileOnboarding(); else openAccountDialog(); return; }
  await openFirstInbox();
  for (const a of S.accounts) refreshAccount(a.id);
  recoverDrafts();
  // Inbox changes arrive instantly via push (IMAP IDLE); other open folders refresh every 10 minutes.
  setInterval(() => {
    if (S.cur && !S.searching && folderOf(S.cur.accountId, S.cur.folder)?.special !== 'inbox') syncCurrent(true);
    for (const a of S.accounts) refreshAccount(a.id);
    updateTaskBadge();
  }, 10 * 60 * 1000);
  setInterval(updateTaskBadge, 60 * 1000);
}

init();
