'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const root = path.resolve(__dirname, '../..');
const html = fs.readFileSync(path.join(root, 'ui/index.html'), 'utf8');
const app = fs.readFileSync(path.join(root, 'ui/app.js'), 'utf8');
const backend = fs.readFileSync(path.join(root, 'src-tauri/src/main.rs'), 'utf8');

test('every literal getElementById reference exists in the HTML shell', () => {
  const referenced = [...app.matchAll(/getElementById\('([^']+)'\)/g)].map(match => match[1]);
  const dynamicallyCreated = new Set([...app.matchAll(/\.id\s*=\s*'([^']+)'/g)].map(match => match[1]));
  const missing = [...new Set(referenced)].filter(id =>
    !new RegExp(`\\bid=["']${id}["']`).test(html) && !dynamicallyCreated.has(id)
  );
  assert.deepEqual(missing, []);
});

test('HTML ids are unique', () => {
  const ids = [...html.matchAll(/\bid=["']([^"']+)["']/g)].map(match => match[1]);
  const duplicates = ids.filter((id, index) => ids.indexOf(id) !== index);
  assert.deepEqual([...new Set(duplicates)], []);
});

test('all settings tabs have a matching content panel', () => {
  const tabs = [...html.matchAll(/class="[^"]*\btab\b[^"]*" data-tab="([^"]+)"/g)].map(match => match[1]);
  assert.ok(tabs.length >= 5);
  for (const tab of tabs) assert.match(html, new RegExp(`id="tab-${tab}"`));
});

test('frontend core loads before the application', () => {
  const coreIndex = html.indexOf('<script src="app-core.js"');
  const appIndex = html.indexOf('<script src="app.js"');
  assert.ok(coreIndex >= 0);
  assert.ok(coreIndex < appIndex);
});

test('every literal frontend command is registered by the Rust backend', () => {
  const calls = [
    ...app.matchAll(/\bcmd\('([a-z_]+)'/g),
    ...app.matchAll(/\binvoke\('([a-z_]+)'/g),
  ].map(match => match[1]);
  const missing = [...new Set(calls)].filter(command =>
    !new RegExp(`(?:async\\s+)?fn\\s+${command}\\b`).test(backend)
  );
  assert.deepEqual(missing, []);
});

test('update controls and translated labels are present', () => {
  for (const id of ['s-update-current', 's-update-auto', 's-update-check', 's-update-install', 's-update-status', 's-update-progress']) {
    assert.match(html, new RegExp(`id="${id}"`));
  }
  for (const key of ['set.updates', 'set.updateCurrent', 'set.updateAuto', 'set.updateCheck', 'set.updateInstall']) {
    assert.match(html, new RegExp(`data-i18n="${key}"`));
  }
});

test('all supported locale scripts load before app startup', () => {
  const expected = ['en', 'fr', 'zh', 'hi', 'es', 'ar', 'bn', 'ru', 'pt', 'ur', 'id', 'de', 'ja'];
  const appIndex = html.indexOf('<script src="app.js"');
  for (const locale of expected) {
    const localeIndex = html.indexOf(`locales/${locale}.js`);
    assert.ok(localeIndex >= 0, `${locale} locale script missing`);
    assert.ok(localeIndex < appIndex, `${locale} locale loads after app.js`);
  }
});
