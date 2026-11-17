'use strict';
/* Contacts: list, search, editor, email / call actions. */

let contactSel = null; // id of selected contact, or 'new'

async function loadContacts(query = '') {
  let list = [];
  try { list = await invoke('list_contacts', { query }); } catch (e) { toast(errText(e), 'error'); }
  S.contactsView = list;
  if (!query) S.contacts = list;
}

function renderContactList() {
  const box = $('#contact-list');
  const list = S.contactsView || S.contacts;
  if (!list.length) {
    box.innerHTML = `<div class="list-empty">${icon('users', 40)}<p>No contacts yet.<br>Add people from an email with ${icon('user_plus', 14)} or the + button.</p></div>`;
    return;
  }
  let html = '', letter = '';
  for (const c of list) {
    const l = c.favorite ? '★' : ((c.name || c.email || '#')[0] || '#').toUpperCase();
    if (l !== letter) { html += `<div class="letter">${l === '★' ? 'Favourites' : esc(l)}</div>`; letter = l; }
    html += `<div class="contact-row ${contactSel === c.id ? 'active' : ''}" data-id="${c.id}">
      ${avatar(c.name, c.email, 36)}<div class="cr-main"><b>${esc(c.name || c.email || c.phone)}</b><small>${esc(c.company || c.email)}</small></div>
      ${c.phone ? `<button class="icon call" data-call="${c.id}" title="Call ${esc(c.phone)}">${icon('phone', 16)}</button>` : ''}</div>`;
  }
  box.innerHTML = html;
}

function renderContactDetail() {
  const box = $('#contact-detail');
  const c = contactSel === 'new' ? S.contactDraft : S.contacts.find((x) => x.id === contactSel);
  if (!c) {
    box.innerHTML = `<div class="empty">${icon('users', 56)}<h3>Your contacts</h3><p>Select someone, or create a new contact.</p></div>`;
    return;
  }
  box.innerHTML = `<div class="contact-card">
    <div class="cc-hero">${avatar(c.name, c.email, 72)}<div><h2>${esc(c.name || 'New contact')}</h2><p class="muted">${esc(c.company || '')}</p></div></div>
    <div class="cc-actions">
      <button class="tb" id="cc-mail" ${c.email ? '' : 'disabled'}>${icon('mail', 16)}<span>Email</span></button>
      <button class="tb" id="cc-call" ${c.phone ? '' : 'disabled'}>${icon('phone', 16)}<span>Call</span></button>
      <button class="tb ${c.favorite ? 'on' : ''}" id="cc-fav">${icon('star', 16)}<span>${c.favorite ? 'Favourite' : 'Add to favourites'}</span></button>
    </div>
    <div class="form-grid">
      <label>Name<input id="cc-name" value="${esc(c.name)}" /></label>
      <label>Email<input id="cc-email" type="email" value="${esc(c.email)}" /></label>
      <label>Phone<input id="cc-phone" type="tel" value="${esc(c.phone)}" placeholder="+971 …" /></label>
      <label>Company<input id="cc-company" value="${esc(c.company)}" /></label>
      <label class="full">Notes<textarea id="cc-notes" rows="4">${esc(c.notes)}</textarea></label>
    </div>
    <footer class="cc-foot">
      ${c.id ? `<button class="ghost danger" id="cc-del">${icon('trash', 16)} Delete</button>` : ''}
      <span class="grow"></span><button class="primary" id="cc-save">${icon('check', 16)} Save</button>
    </footer></div>`;
  const read = () => ({ ...c, name: $('#cc-name').value.trim(), email: $('#cc-email').value.trim(), phone: $('#cc-phone').value.trim(), company: $('#cc-company').value.trim(), notes: $('#cc-notes').value });
  $('#cc-save').onclick = async () => {
    try {
      const saved = await invoke('save_contact', { contact: read() });
      contactSel = saved.id;
      await loadContacts();
      renderContactList(); renderContactDetail();
      toast('Contact saved', 'ok');
      queueNcSync();
      if (S.view) showReader(S.view);
    } catch (e) { toast(errText(e), 'error'); }
  };
  $('#cc-fav').onclick = async () => {
    c.favorite = !c.favorite;
    if (c.id) { await invoke('save_contact', { contact: read() }).catch(() => {}); await loadContacts(); renderContactList(); queueNcSync(); }
    renderContactDetail();
  };
  $('#cc-mail').onclick = () => openCompose({ to: fmtAddr($('#cc-name').value.trim(), $('#cc-email').value.trim()) });
  $('#cc-call').onclick = () => invoke('call_number', { number: $('#cc-phone').value }).catch((e) => toast(errText(e), 'error'));
  const del = $('#cc-del');
  if (del) del.onclick = async () => {
    if (!(await confirmBox(`Delete ${c.name || c.email}?`, 'Delete', true))) return;
    await invoke('delete_contact', { id: c.id });
    queueNcSync();
    contactSel = null;
    await loadContacts();
    renderContactList(); renderContactDetail();
  };
}

/** Opens the Contacts tab with an editor prefilled (e.g. from an email sender). */
function openContactEditor(prefill = {}) {
  switchTab('contacts');
  const existing = prefill.email && S.contacts.find((c) => c.email.toLowerCase() === prefill.email.toLowerCase());
  if (existing) contactSel = existing.id;
  else { S.contactDraft = { id: 0, name: '', email: '', phone: '', company: '', notes: '', favorite: false, ...prefill }; contactSel = 'new'; }
  renderContactList();
  renderContactDetail();
  setTimeout(() => $('#cc-phone')?.focus(), 50);
}

$('#contact-list').addEventListener('click', (e) => {
  const call = e.target.closest('[data-call]');
  if (call) {
    const c = S.contacts.find((x) => x.id === Number(call.dataset.call));
    invoke('call_number', { number: c.phone }).catch((err) => toast(errText(err), 'error'));
    return;
  }
  const row = e.target.closest('.contact-row');
  if (!row) return;
  contactSel = Number(row.dataset.id);
  renderContactList();
  renderContactDetail();
});
$('#btn-new-contact').onclick = () => openContactEditor({});
$('#contact-search').addEventListener('input', debounce(async (e) => {
  await loadContacts(e.target.value.trim());
  renderContactList();
  if (!e.target.value.trim()) S.contactsView = S.contacts;
}, 200));
