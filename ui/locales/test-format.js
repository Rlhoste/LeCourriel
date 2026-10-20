// Formatting test: node ui/locales/test-format.js
const fs = require('fs'), path = require('path');
global.document = { documentElement: {} };
global.localStorage = { setItem() {}, getItem() { return null; } };
const src = fs.readFileSync(path.join(__dirname, '..', 'i18n.js'), 'utf8') + `
;STRINGS.en = { 'x.one': 'one {n}', 'x.other': 'other {n}', 'x.zero': 'none' };
STRINGS.ru = { 'x.one': 'one {n}', 'x.few': 'few {n}', 'x.many': 'many {n}', 'x.other': 'other {n}' };
STRINGS.ar = { 'x.one': 'one', 'x.two': 'two', 'x.few': 'few {n}', 'x.many': 'many {n}', 'x.other': 'other {n}' };
STRINGS.fr = {}; STRINGS.ja = {};
module.exports = { setLang, tn, fmtShort, fmtFull, fmtDayMonth, fmtWeekday, fmtMonthYear, weekdayNames, fmtTime, languageName, document };`;
const m = {}; new Function('module', src)(m);
const L = m.exports;
let fails = 0;
const eq = (got, want, what) => { if (got !== want) { console.log(`FAIL ${what}: got "${got}", want "${want}"`); fails++; } };
const ts = Date.UTC(2026, 9, 20, 8, 5) / 1000;   // Tue 20 Oct 2026
const now = ts * 1000 + 3600e3;                  // one hour later

L.setLang('ru');
eq([1, 3, 5, 21].map(n => L.tn('x', n)).join('|'), 'one 1|few 3|many 5|one 21', 'ru plurals');
L.setLang('ar');
// Digits may be Latin or Arabic-Indic depending on the ICU version: compare the forms only.
eq([1, 2, 5, 11, 100].map(n => L.tn('x', n).split(' ')[0]).join('|'), 'one|two|few|many|other', 'ar plurals');
eq(L.document.documentElement.dir, 'rtl', 'ar is right-to-left');
L.setLang('en');
eq(L.tn('x', 0), 'none', 'zero form');
eq(L.document.documentElement.dir, 'ltr', 'en is left-to-right');

L.setLang('fr');
eq(L.fmtDayMonth('2026-10-20'), '20 oct.', 'fr day month');
eq(L.fmtMonthYear(2026, 10), 'octobre 2026', 'fr month year');
eq(L.weekdayNames()[0], 'lun.', 'fr Monday first');
eq(L.languageName(), 'français', 'fr language name');
L.setLang('en');
eq(L.fmtDayMonth('2026-10-20'), '20 Oct', 'en (GB) day month');
L.setLang('ru');
eq(L.fmtWeekday('2026-10-20'), 'вт', 'ru weekday');
L.setLang('ja');
eq(L.fmtMonthYear(2026, 10), '2026年10月', 'ja month year');
L.setLang('fr');
eq(L.fmtShort(ts, now).length, 5, 'today shows a time');                // "10:05" in local time
eq(L.fmtShort(ts, now + 3 * 86400e3), 'mar.', 'this week shows the weekday');
eq(L.fmtShort(ts, now + 30 * 86400e3), '20 oct.', 'older shows day and month');
console.log(fails ? `${fails} failure(s)` : 'all formatting checks passed');
process.exit(fails ? 1 : 0);
