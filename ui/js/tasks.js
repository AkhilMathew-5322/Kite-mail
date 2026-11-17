'use strict';
/* Tasks: quick add, due dates with reminders (notifications come from the Rust side), link to emails. */

let taskFilter = 'open';

async function loadTodos() {
  try { S.todos = await invoke('list_todos'); } catch (e) { toast(errText(e), 'error'); }
  updateTaskBadge();
}

function updateTaskBadge() {
  const now = nowSec();
  const due = S.todos.filter((t) => !t.done && t.due && t.due <= now).length;
  const b = $('#badge-tasks');
  b.textContent = due || '';
  b.hidden = !due;
  b.classList.toggle('alert', due > 0);
}

function quickDue(kind) {
  const d = new Date();
  if (kind === '1h') d.setHours(d.getHours() + 1);
  if (kind === 'tonight') { d.setHours(18, 0, 0, 0); if (d < new Date()) d.setDate(d.getDate() + 1); }
  if (kind === 'tomorrow') { d.setDate(d.getDate() + 1); d.setHours(9, 0, 0, 0); }
  if (kind === 'nextweek') { d.setDate(d.getDate() + ((8 - d.getDay()) % 7 || 7)); d.setHours(9, 0, 0, 0); }
  return Math.floor(d.getTime() / 1000);
}

function dueLabel(t) {
  if (!t.due) return '';
  const now = nowSec();
  const d = new Date(t.due * 1000);
  const today = new Date(); today.setHours(0, 0, 0, 0);
  const dayDiff = Math.round((new Date(d).setHours(0, 0, 0, 0) - today) / 86400000);
  const time = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const day = dayDiff === 0 ? 'Today' : dayDiff === 1 ? 'Tomorrow' : dayDiff === -1 ? 'Yesterday' : d.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short' });
  const cls = t.done ? '' : t.due <= now ? 'overdue' : dayDiff === 0 ? 'today' : '';
  return `<span class="due ${cls}">${icon('clock', 13)} ${day} ${time}</span>`;
}

function renderTasks() {
  const now = nowSec();
  const endOfDay = new Date(); endOfDay.setHours(23, 59, 59, 999);
  const eod = endOfDay.getTime() / 1000;
  const list = S.todos.filter((t) => {
    if (taskFilter === 'done') return t.done;
    if (t.done) return false;
    if (taskFilter === 'today') return t.due && t.due <= eod;
    if (taskFilter === 'overdue') return t.due && t.due <= now;
    return true;
  });
  const open = S.todos.filter((t) => !t.done).length;
  const overdue = S.todos.filter((t) => !t.done && t.due && t.due <= now).length;
  $('#task-summary').textContent = `${open} open${overdue ? ` · ${overdue} due` : ''}`;
  const box = $('#task-list');
  if (!list.length) {
    box.innerHTML = `<div class="list-empty">${icon('tasks', 44)}<p>${taskFilter === 'done' ? 'Nothing completed yet.' : 'You\'re all caught up!'}</p></div>`;
    return;
  }
  box.innerHTML = list.map((t) => `<div class="task ${t.done ? 'done' : ''} ${t.priority ? 'high' : ''}" data-id="${t.id}">
      <button class="check" data-t="done" title="${t.done ? 'Mark open' : 'Complete'}">${icon('check', 14)}</button>
      <div class="t-main" data-t="edit">
        <div class="t-title">${esc(t.title)}</div>
        ${t.notes ? `<div class="t-notes">${esc(t.notes)}</div>` : ''}
        <div class="t-meta">${dueLabel(t)}${t.uid ? `<button class="chip linkmail" data-t="mail">${icon('mail', 12)} Open email</button>` : ''}</div>
      </div>
      <button class="icon ${t.priority ? 'on' : ''}" data-t="prio" title="Important">${icon('flag', 16)}</button>
      <button class="icon" data-t="del" title="Delete">${icon('trash', 16)}</button>
    </div>`).join('');
}

async function saveTodo(t, quiet = false) {
  try {
    const saved = await invoke('save_todo', { todo: t });
    queueNcSync();
    await loadTodos();
    renderTasks();
    if (!quiet) toast('Task saved', 'ok');
    return saved;
  } catch (e) { toast(errText(e), 'error'); return null; }
}

/** Task editor dialog (title, notes, due, priority). */
function editTask(t) {
  return new Promise((resolve) => {
    const d = document.createElement('dialog');
    d.className = 'small-dialog task-dialog';
    d.innerHTML = `<form method="dialog">
      <h3>${t.id ? 'Edit task' : 'New task'}</h3>
      <label>Title<input id="td-title" value="${esc(t.title)}" required /></label>
      <label>Notes<textarea id="td-notes" rows="3">${esc(t.notes || '')}</textarea></label>
      <label>Remind me<input id="td-due" type="datetime-local" value="${toLocalInput(t.due)}" /></label>
      <div class="chips">
        <button type="button" class="chip" data-q="1h">In 1 hour</button><button type="button" class="chip" data-q="tonight">6 pm</button>
        <button type="button" class="chip" data-q="tomorrow">Tomorrow 9 am</button><button type="button" class="chip" data-q="nextweek">Next Monday</button>
        <button type="button" class="chip" data-q="none">No reminder</button>
      </div>
      <label class="row"><input type="checkbox" id="td-prio" ${t.priority ? 'checked' : ''}/> Important</label>
      <footer><button value="cancel">Cancel</button><button value="ok" class="primary">Save</button></footer></form>`;
    document.body.appendChild(d);
    d.querySelector('.chips').onclick = (e) => {
      const q = e.target.dataset.q;
      if (!q) return;
      $('#td-due', d).value = q === 'none' ? '' : toLocalInput(quickDue(q));
    };
    d.addEventListener('close', () => {
      if (d.returnValue === 'ok' && $('#td-title', d).value.trim()) {
        resolve({ ...t, title: $('#td-title', d).value.trim(), notes: $('#td-notes', d).value, due: fromLocalInput($('#td-due', d).value), priority: $('#td-prio', d).checked ? 1 : 0 });
      } else resolve(null);
      d.remove();
    });
    d.showModal();
    $('#td-title', d).select();
  });
}

/** "Add to tasks" from an open email. */
async function quickTaskFromMail(view) {
  const s = view.summary;
  const t = await editTask({ id: 0, title: `Follow up: ${s.subject || '(no subject)'}`, notes: `From ${s.from_name || s.from_addr}`, due: quickDue('tomorrow'), done: false, priority: 0, account_id: s.account_id, folder: s.folder, uid: s.uid });
  if (t) await saveTodo(t);
}

$('#task-add').addEventListener('submit', async (e) => {
  e.preventDefault();
  const title = $('#task-title').value.trim();
  if (!title) return;
  await saveTodo({ id: 0, title, notes: '', due: fromLocalInput($('#task-due').value), done: false, priority: 0 }, true);
  $('#task-title').value = '';
  $('#task-due').value = '';
});
$('#task-title').addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); $('#task-add').requestSubmit(); } });
$('#task-add').addEventListener('click', (e) => {
  const q = e.target.dataset.quick;
  if (q) $('#task-due').value = toLocalInput(quickDue(q));
});
$('#task-filter').addEventListener('click', (e) => {
  const f = e.target.dataset.f;
  if (!f) return;
  taskFilter = f;
  $$('#task-filter button').forEach((b) => b.classList.toggle('active', b.dataset.f === f));
  renderTasks();
});
$('#task-list').addEventListener('click', async (e) => {
  const row = e.target.closest('.task');
  const act = e.target.closest('[data-t]')?.dataset.t;
  if (!row || !act) return;
  const t = S.todos.find((x) => x.id === Number(row.dataset.id));
  if (!t) return;
  if (act === 'done') {
    row.classList.add('completing');
    setTimeout(() => saveTodo({ ...t, done: !t.done }, true), 250);
  }
  if (act === 'prio') saveTodo({ ...t, priority: t.priority ? 0 : 1 }, true);
  if (act === 'del') { await invoke('delete_todo', { id: t.id }); queueNcSync(); await loadTodos(); renderTasks(); }
  if (act === 'edit') { const nt = await editTask(t); if (nt) saveTodo(nt); }
  if (act === 'mail') {
    switchTab('mail');
    await selectFolder(t.account_id, t.folder);
    const m = S.messages.find((x) => x.uid === t.uid);
    if (m) { S.selected = new Set([key(m)]); renderVisible(true); openMessage(m); }
    else toast('That email is no longer in this folder', 'error');
  }
});
