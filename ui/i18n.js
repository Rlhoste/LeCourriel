/* UI translations and locale-aware formatting.
   Each language lives in ui/locales/<code>.js as STRINGS.<code> = {…}; English is the reference.
   t('key', {vars}) returns the text in the current language, falling back to English, then to
   the key. Static HTML uses data-i18n (text), data-i18n-placeholder and data-i18n-title.
   Dates and plurals use the browser's Intl APIs, so they follow each language's own rules. */

const STRINGS = {};

// Native names, in the order shown in Settings. `locale` is used for Intl formatting.
const LANGUAGES = [
  { id: 'en', name: 'English', locale: 'en-GB' },
  { id: 'fr', name: 'Français', locale: 'fr-FR' },
  { id: 'zh', name: '中文（简体）', locale: 'zh-CN' },
  { id: 'hi', name: 'हिन्दी', locale: 'hi-IN' },
  { id: 'es', name: 'Español', locale: 'es-ES' },
  { id: 'ar', name: 'العربية', locale: 'ar', rtl: true },
  { id: 'bn', name: 'বাংলা', locale: 'bn-BD' },
  { id: 'ru', name: 'Русский', locale: 'ru-RU' },
  { id: 'pt', name: 'Português', locale: 'pt-BR' },
  { id: 'ur', name: 'اردو', locale: 'ur-PK', rtl: true },
  { id: 'id', name: 'Bahasa Indonesia', locale: 'id-ID' },
  { id: 'de', name: 'Deutsch', locale: 'de-DE' },
  { id: 'ja', name: '日本語', locale: 'ja-JP' },
];

let LANG = 'en';
const langInfo = () => LANGUAGES.find(l => l.id === LANG) || LANGUAGES[0];

function setLang(code) {
  LANG = LANGUAGES.some(l => l.id === code) && STRINGS[code] ? code : 'en';
  document.documentElement.lang = LANG;   // also drives the composer's spellchecker and :lang() fonts
  document.documentElement.dir = langInfo().rtl ? 'rtl' : 'ltr';
  try { localStorage.setItem('lang', LANG); } catch (_) {}
}

function t(key, vars = {}) {
  const s = STRINGS[LANG]?.[key] ?? STRINGS.en?.[key] ?? key;
  return s.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? vars[k] : m));
}

// Plural: key.zero when n is 0 and the language has one, else the CLDR category
// (one / two / few / many / other) of the language, falling back to key.other.
function tn(key, n, vars = {}) {
  const all = { n: fmtNumber(n), ...vars };
  const exists = k => STRINGS[LANG]?.[k] !== undefined || (STRINGS[LANG] === undefined && STRINGS.en?.[k] !== undefined);
  if (n === 0 && exists(`${key}.zero`)) return t(`${key}.zero`, all);
  const cat = new Intl.PluralRules(langInfo().locale).select(n);
  return exists(`${key}.${cat}`) ? t(`${key}.${cat}`, all) : t(`${key}.other`, all);
}

function applyStaticI18n(root = document) {
  root.querySelectorAll('[data-i18n]').forEach(el => { el.textContent = t(el.dataset.i18n); });
  root.querySelectorAll('[data-i18n-placeholder]').forEach(el => { el.placeholder = t(el.dataset.i18nPlaceholder); });
  root.querySelectorAll('[data-i18n-title]').forEach(el => { el.title = t(el.dataset.i18nTitle); });
}

// ── Formatting ──
const fmt = (opts) => new Intl.DateTimeFormat(langInfo().locale, opts);
const fromIso = iso => { const [y, m, d] = String(iso).split('-').map(Number); return new Date(y, m - 1, d); };

function fmtNumber(n) {
  return new Intl.NumberFormat(langInfo().locale).format(n);
}

// Email list date: the time today (< 24 h), the weekday this week (< 7 days), else day + month.
function fmtShort(ts, now = Date.now()) {
  const d = new Date(ts * 1000);
  const age = now - d.getTime();
  if (age < 24 * 3600e3) return fmt({ hour: '2-digit', minute: '2-digit' }).format(d);
  if (age < 7 * 24 * 3600e3) return fmt({ weekday: 'short' }).format(d);
  return fmt({ day: 'numeric', month: 'short' }).format(d);
}

function fmtFull(ts) {
  return fmt({ weekday: 'long', day: 'numeric', month: 'long', year: 'numeric', hour: '2-digit', minute: '2-digit' })
    .format(new Date(ts * 1000));
}

function fmtTime(hour, minute) {
  return fmt({ hour: '2-digit', minute: '2-digit' }).format(new Date(2000, 0, 1, hour, minute));
}

function fmtDayMonth(iso) { return fmt({ day: 'numeric', month: 'short' }).format(fromIso(iso)); }
function fmtWeekday(iso)  { return fmt({ weekday: 'short' }).format(fromIso(iso)); }
function fmtMonthYear(year, month) { return fmt({ month: 'long', year: 'numeric' }).format(new Date(year, month - 1, 1)); }
// Monday-first short weekday names for the calendar header.
function weekdayNames() { return [0, 1, 2, 3, 4, 5, 6].map(i => fmt({ weekday: 'short' }).format(new Date(2024, 0, 1 + i))); }
// The current language's own name, in that language ("français", "русский"…).
function languageName(code = LANG) {
  try { return new Intl.DisplayNames([langInfo().locale], { type: 'language' }).of(code); } catch (_) { return code; }
}

// Emails carry a timestamp; older cached ones only have the backend's text.
const emailDateShort = e => (e && e.date_ts ? fmtShort(e.date_ts) : (e?.date_str || ''));
const emailDateFull  = e => (e && e.date_ts ? fmtFull(e.date_ts) : (e?.date_full || e?.date_str || ''));

function greetingText(state) {
  const part = state.hour < 12 ? 'morning' : state.hour < 17 ? 'afternoon' : 'evening';
  const name = (state.user_name || '').trim();
  return name ? t(`greet.${part}`, { name }) : t(`greet.${part}.noname`);
}

function briefingText(b) {
  if (!b) return '';
  const attention = tn('brief.attention', b.count);
  switch (b.kind) {
    case 'live': return t('brief.live', { time: fmtTime(b.hour, b.minute), attention });
    case 'classifying': return tn('brief.classifying', b.count);
    case 'up_to_date': return t('brief.upToDate');
    case 'ai_not_paid': return t('brief.aiNotPaid');
    case 'ai_stopped': return t('brief.aiStopped', { reason: b.detail });
    default: return t('brief.cached', { attention });
  }
}

// Apply the last language before app.js renders anything (called once all locales are loaded).
function initLang() {
  try { setLang(localStorage.getItem('lang') || 'en'); } catch (_) { setLang('en'); }
}
