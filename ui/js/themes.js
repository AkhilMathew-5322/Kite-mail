'use strict';
/* Built-in themes + custom themes. Each theme defines a handful of base colours; everything else
   (hover, selection, shadows) is derived in CSS with color-mix(). */

const BUILTIN_THEMES = [
  // light
  { id: 'aurora', name: 'Aurora', dark: false, c: { bg: '#f6f7fb', surface: '#ffffff', side: '#eef0f8', fg: '#1c2033', muted: '#6b7088', line: '#e3e6f0', accent: '#5b5bf6', accent2: '#a855f7' } },
  { id: 'ocean', name: 'Ocean', dark: false, c: { bg: '#f3f8fb', surface: '#ffffff', side: '#e5f0f7', fg: '#13293d', muted: '#5f7487', line: '#dae6ee', accent: '#0284c7', accent2: '#06b6d4' } },
  { id: 'sand', name: 'Sand', dark: false, c: { bg: '#faf7f2', surface: '#fffdf9', side: '#f2ece2', fg: '#2d261f', muted: '#7d7266', line: '#e9e0d3', accent: '#d97706', accent2: '#ef4444' } },
  { id: 'mint', name: 'Mint', dark: false, c: { bg: '#f4faf7', surface: '#ffffff', side: '#e4f3ea', fg: '#142a20', muted: '#5c7468', line: '#d6eadf', accent: '#059669', accent2: '#22c55e' } },
  { id: 'paper', name: 'Paper', dark: false, c: { bg: '#f5f5f4', surface: '#ffffff', side: '#ececea', fg: '#1c1917', muted: '#78716c', line: '#e2e0dd', accent: '#1f2937', accent2: '#6b7280' } },
  // dark
  { id: 'midnight', name: 'Midnight', dark: true, c: { bg: '#0f1220', surface: '#161a2b', side: '#0b0e19', fg: '#e6e8f2', muted: '#8d93ad', line: '#252a40', accent: '#7c7cff', accent2: '#c084fc' } },
  { id: 'graphite', name: 'Graphite', dark: true, c: { bg: '#18191b', surface: '#202124', side: '#131416', fg: '#e8e8ea', muted: '#9a9ca3', line: '#2e3034', accent: '#4f8cff', accent2: '#22d3ee' } },
  { id: 'nord', name: 'Nord', dark: true, c: { bg: '#2e3440', surface: '#3b4252', side: '#272c36', fg: '#eceff4', muted: '#a3acbd', line: '#4c566a', accent: '#88c0d0', accent2: '#81a1c1' } },
  { id: 'forest', name: 'Forest', dark: true, c: { bg: '#0f1a15', surface: '#15241d', side: '#0b140f', fg: '#e3efe8', muted: '#8aa898', line: '#23382d', accent: '#34d399', accent2: '#a3e635' } },
  { id: 'neon', name: 'Neon', dark: true, c: { bg: '#191622', surface: '#211d2e', side: '#13111b', fg: '#f1eefc', muted: '#9d95b8', line: '#302a45', accent: '#ff79c6', accent2: '#8be9fd' } },
];

const THEME_KEYS = [
  ['accent', 'Accent'], ['accent2', 'Accent 2 (gradient)'], ['bg', 'Background'], ['surface', 'Panels'],
  ['side', 'Sidebar'], ['fg', 'Text'], ['muted', 'Secondary text'], ['line', 'Borders'],
];

const DEFAULT_SETTINGS = {
  mode: 'system', light_theme: 'aurora', dark_theme: 'midnight', custom_themes: [],
  density: 'comfortable', font_size: 14, dark_emails: true,
  notify_mail: true, notify_todos: true,
  lt_url: '', lt_lang: 'auto', call_template: '', remote_images: 'ask', signatures: {},
};

const darkQuery = window.matchMedia('(prefers-color-scheme: dark)');

function allThemes() {
  return [...BUILTIN_THEMES, ...(S.settings.custom_themes || [])];
}

function isDarkMode() {
  const m = S.settings.mode;
  return m === 'dark' || (m === 'system' && darkQuery.matches);
}

function currentTheme() {
  const id = isDarkMode() ? S.settings.dark_theme : S.settings.light_theme;
  return allThemes().find((t) => t.id === id) || BUILTIN_THEMES.find((t) => t.dark === isDarkMode());
}

function contrastText(hex) {
  const h = hex.replace('#', '');
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(h.substr(i, 2), 16) / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.45 ? '#111111' : '#ffffff';
}

function applyTheme(theme = currentTheme(), preview = false) {
  const root = document.documentElement;
  for (const [k] of THEME_KEYS) root.style.setProperty('--' + k, theme.c[k]);
  root.style.setProperty('--accent-fg', contrastText(theme.c.accent));
  root.style.colorScheme = theme.dark ? 'dark' : 'light';
  root.dataset.dark = theme.dark ? '1' : '0';
  root.dataset.density = S.settings.density;
  root.style.fontSize = (S.settings.font_size || 14) + 'px';
  if (!preview && typeof refreshThemeButton === 'function') refreshThemeButton();
  if (!preview && S.view && typeof renderBody === 'function') renderBody();
}

darkQuery.addEventListener('change', () => applyTheme());

/** Mini preview card used in the theme gallery. */
function themeSwatch(t, selected) {
  const c = t.c;
  return `<button class="swatch ${selected ? 'selected' : ''}" data-theme="${esc(t.id)}" title="${esc(t.name)}">
    <span class="sw-win" style="background:${c.bg};border-color:${c.line}">
      <span class="sw-side" style="background:${c.side}"></span>
      <span class="sw-body">
        <span class="sw-bar" style="background:linear-gradient(90deg,${c.accent},${c.accent2})"></span>
        <span class="sw-line" style="background:${c.fg}"></span>
        <span class="sw-line short" style="background:${c.muted}"></span>
        <span class="sw-card" style="background:${c.surface};border-color:${c.line}"></span>
      </span>
    </span>
    <span class="sw-name">${esc(t.name)}${t.custom ? ' <em>custom</em>' : ''}</span>
  </button>`;
}
