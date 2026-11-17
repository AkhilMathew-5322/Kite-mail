'use strict';
/* Shared state, helpers, toasts and dialogs. */

// Android (edge-to-edge): MainActivity.kt reports status bar / navigation bar / keyboard sizes.
window.__kiteInsets = (i) => {
  const r = document.documentElement.style;
  r.setProperty('--sat', (i.top || 0) + 'px');
  r.setProperty('--sab', (i.bottom || 0) + 'px');
  r.setProperty('--sal', (i.left || 0) + 'px');
  r.setProperty('--sar', (i.right || 0) + 'px');
  r.setProperty('--kb', (i.ime || 0) + 'px');
  document.body?.classList.toggle('kb-open', (i.ime || 0) > 0);
};
try { if (window.KiteInsets) window.__kiteInsets(JSON.parse(window.KiteInsets.get())); } catch (_) { /* not Android */ }

const { invoke } = window.__TAURI__.core;
const listen = window.__TAURI__.event?.listen || (async () => () => {});
const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

const S = {
  settings: { ...DEFAULT_SETTINGS },
  view: null,          // open MessageView
  tab: 'mail',         // mail | contacts | tasks | settings
  accounts: [],
  folders: {},
  errors: {},
  collapsed: {},
  lastFolder: {},      // accountId -> folder last opened (account switcher)
  cur: null,           // { accountId, folder }
  messages: [],
  selected: new Set(),
  anchor: null,
  allowRemote: false,
  limits: {},
  searching: false,
  compose: null,
  contacts: [],
  contactEmails: new Set(),
  todos: [],
};

const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const errText = (e) => (typeof e === 'string' ? e : e?.message || JSON.stringify(e));
const acct = (id) => S.accounts.find((a) => a.id === id);
const nowSec = () => Math.floor(Date.now() / 1000);

async function saveSettings(patch) {
  Object.assign(S.settings, patch);
  try { await invoke('save_settings', { settings: S.settings }); } catch (e) { toast(errText(e), 'error'); }
}

// ------------------------------------------------------------------ toasts & progress
function toast(msg, kind = 'info', action = null) {
  const t = document.createElement('div');
  t.className = 'toast ' + kind;
  const ic = { info: 'info', error: 'junk', ok: 'check' }[kind] || 'info';
  t.innerHTML = `${icon(ic, 18)}<span class="toast-msg"></span>`;
  $('.toast-msg', t).textContent = msg;
  if (action) {
    const b = document.createElement('button');
    b.className = 'link';
    b.textContent = action.label;
    b.onclick = () => { t.remove(); action.run(); };
    t.appendChild(b);
  }
  toastHost().appendChild(t);
  requestAnimationFrame(() => t.classList.add('show'));
  setTimeout(() => { t.classList.remove('show'); setTimeout(() => t.remove(), 300); }, action?.ttl ?? (kind === 'error' ? 9000 : action ? 8000 : 3500));
  return t;
}

/** Modal dialogs sit above the page, so toasts raised while one is open go inside it. */
function toastHost() {
  const dlg = [...document.querySelectorAll('dialog[open]')].filter((d) => d.matches(':modal')).pop();
  if (!dlg) return $('#toasts');
  let host = dlg.querySelector(':scope > .toasts');
  if (!host) { host = document.createElement('div'); host.className = 'toasts'; dlg.appendChild(host); }
  return host;
}

let busy = 0;
async function withBusy(label, fn) {
  busy++;
  $('#busy').classList.add('on');
  $('#busy').title = label;
  try { return await fn(); }
  finally { if (--busy === 0) $('#busy').classList.remove('on'); }
}

// ------------------------------------------------------------------ dialogs
/** Small modal with custom buttons. Resolves with the clicked button's value (or null on Esc). */
function choiceBox(message, buttons, title = '') {
  return new Promise((resolve) => {
    const d = document.createElement('dialog');
    d.className = 'small-dialog';
    d.innerHTML = `<form method="dialog">${title ? `<h3>${esc(title)}</h3>` : ''}<p>${esc(message)}</p>
      <footer>${buttons.map((b, i) => `<button value="${esc(b.value)}" class="${b.cls || ''}" ${i === buttons.length - 1 ? 'autofocus' : ''}>${esc(b.label)}</button>`).join('')}</footer></form>`;
    document.body.appendChild(d);
    d.addEventListener('close', () => { resolve(d.returnValue || null); d.remove(); });
    d.showModal();
  });
}
const confirmBox = async (msg, ok = 'OK', danger = false) =>
  (await choiceBox(msg, [{ value: 'cancel', label: 'Cancel' }, { value: 'ok', label: ok, cls: danger ? 'danger-solid' : 'primary' }])) === 'ok';

// ------------------------------------------------------------------ formatting
function fmtDate(ts, long = false) {
  if (!ts) return '';
  const d = new Date(ts * 1000), now = new Date();
  if (long) return d.toLocaleString([], { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit' });
  if (d.toDateString() === now.toDateString()) return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const diff = (now - d) / 86400000;
  if (diff < 6) return d.toLocaleDateString([], { weekday: 'short' });
  if (d.getFullYear() === now.getFullYear()) return d.toLocaleDateString([], { day: 'numeric', month: 'short' });
  return d.toLocaleDateString();
}

function dayGroup(ts) {
  if (!ts) return 'Older';
  const d = new Date(ts * 1000), now = new Date();
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime() / 1000;
  if (ts >= start) return 'Today';
  if (ts >= start - 86400) return 'Yesterday';
  if (ts >= start - 6 * 86400) return 'This week';
  if (d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth()) return 'This month';
  return d.toLocaleDateString([], { month: 'long', year: 'numeric' });
}

function fmtSize(n) {
  if (n < 1024) return n + ' B';
  if (n < 1048576) return (n / 1024).toFixed(0) + ' KB';
  return (n / 1048576).toFixed(1) + ' MB';
}

function splitAddrs(s) {
  const out = []; let cur = '', q = false, a = false;
  for (const ch of s || '') {
    if (ch === '"') q = !q;
    if (ch === '<' && !q) a = true;
    if (ch === '>' && !q) a = false;
    if ((ch === ',' || ch === ';') && !q && !a) { if (cur.trim()) out.push(cur.trim()); cur = ''; continue; }
    cur += ch;
  }
  if (cur.trim()) out.push(cur.trim());
  return out;
}
const bareAddr = (s) => ((s.match(/<([^>]+)>/) || [, s])[1]).trim().toLowerCase();
const fmtAddr = (name, addr) => (name ? `${/[,;"]/.test(name) ? `"${name.replace(/"/g, '')}"` : name} <${addr}>` : addr);

/** Deterministic avatar: initials on a colour derived from the name. */
/** Stable colour (hue 0-359) for an address – used for avatars and each account's accent. */
function hueOf(text) {
  let h = 0;
  for (const ch of String(text || '').toLowerCase()) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return h % 360;
}

function avatar(name, email, size = 36) {
  const label = (name || email || '?').trim();
  const words = label.replace(/[^\p{L}\p{N} ]/gu, ' ').split(/\s+/).filter(Boolean);
  const initials = ((words[0]?.[0] || '?') + (words.length > 1 ? words[words.length - 1][0] : '')).toUpperCase();
  const hue = hueOf(email || label);
  return `<span class="avatar" style="width:${size}px;height:${size}px;font-size:${Math.round(size * 0.38)}px;background:linear-gradient(135deg,hsl(${hue} 70% 58%),hsl(${(hue + 40) % 360} 70% 48%))">${esc(initials)}</span>`;
}

function debounce(fn, ms) {
  let t;
  return (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); };
}

/** datetime-local value <-> unix seconds */
const toLocalInput = (ts) => { if (!ts) return ''; const d = new Date(ts * 1000); d.setMinutes(d.getMinutes() - d.getTimezoneOffset()); return d.toISOString().slice(0, 16); };
const fromLocalInput = (v) => (v ? Math.floor(new Date(v).getTime() / 1000) : null);
