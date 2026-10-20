// Locale checker: node ui/locales/check.js
// - every key used by app.js / index.html exists in English;
// - every locale has exactly the English keys (plural keys may use the language's own
//   CLDR categories: zero/one/two/few/many/other, and must include .other);
// - every translation keeps the same {placeholders} as English ({n} may be dropped in a
//   singular form such as "1 requires you").
const fs = require('fs');
const path = require('path');
const ui = path.join(__dirname, '..');
const STRINGS = {};
for (const f of fs.readdirSync(__dirname).filter(f => /^[a-z]{2}\.js$/.test(f))) {
  new Function('STRINGS', fs.readFileSync(path.join(__dirname, f), 'utf8'))(STRINGS);
}
const en = STRINGS.en;
const PLURAL = /\.(zero|one|two|few|many|other)$/;
const pluralBases = new Set(Object.keys(en).filter(k => PLURAL.test(k)).map(k => k.replace(PLURAL, '')));
const holders = s => new Set([...String(s).matchAll(/\{(\w+)\}/g)].map(m => m[1]));
let problems = 0;
const bad = msg => { console.log(msg); problems++; };

// Keys used by the code exist in English.
const app = fs.readFileSync(path.join(ui, 'app.js'), 'utf8') + fs.readFileSync(path.join(ui, 'i18n.js'), 'utf8');
const html = fs.readFileSync(path.join(ui, 'index.html'), 'utf8');
for (const m of app.matchAll(/\bt\('([\w.]+)'/g)) if (!m[1].endsWith('.') && m[1] !== 'key' && !(m[1] in en)) bad(`en lacks ${m[1]} (used in code)`);
for (const m of app.matchAll(/\btn\('([\w.]+)'/g)) if (!pluralBases.has(m[1])) bad(`en lacks plural ${m[1]}.*`);
for (const m of html.matchAll(/data-i18n(?:-placeholder|-title)?="([\w.]+)"/g)) if (!(m[1] in en)) bad(`en lacks ${m[1]} (used in index.html)`);

for (const [code, dict] of Object.entries(STRINGS)) {
  if (code === 'en') continue;
  for (const k of Object.keys(en)) {
    if (PLURAL.test(k)) continue;
    if (!(k in dict)) bad(`${code}: missing ${k}`);
  }
  for (const base of pluralBases) {
    if (!(`${base}.other` in dict)) bad(`${code}: missing ${base}.other`);
  }
  for (const [k, v] of Object.entries(dict)) {
    const base = k.replace(PLURAL, '');
    const isPlural = PLURAL.test(k) && pluralBases.has(base);
    if (!isPlural && !(k in en)) { bad(`${code}: extra key ${k}`); continue; }
    if (typeof v !== 'string' || !v.trim()) { bad(`${code}: empty ${k}`); continue; }
    const want = holders(isPlural ? en[`${base}.other`] : en[k]);
    const got = holders(v);
    for (const h of want) if (!got.has(h) && !(isPlural && h === 'n')) bad(`${code}: ${k} lost {${h}}`);
    for (const h of got) if (!want.has(h)) bad(`${code}: ${k} has unknown {${h}}`);
  }
}
const langs = Object.keys(STRINGS).sort();
console.log(`${langs.length} locales (${langs.join(', ')}), ${Object.keys(en).length} English keys, ${problems} problems`);
process.exit(problems ? 1 : 0);
