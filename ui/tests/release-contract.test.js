'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const root = path.resolve(__dirname, '../..');
const tauri = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
const capability = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/capabilities/default.json'), 'utf8'));
const cargo = fs.readFileSync(path.join(root, 'src-tauri/Cargo.toml'), 'utf8');
const workflow = fs.readFileSync(path.join(root, '.github/workflows/build.yml'), 'utf8');

test('Cargo and Tauri package versions stay synchronized', () => {
  const cargoVersion = cargo.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  assert.equal(cargoVersion, tauri.version);
});

test('updater artifacts are enabled for release builds', () => {
  assert.equal(tauri.bundle.createUpdaterArtifacts, true);
});

test('updater endpoint is HTTPS and points to latest.json', () => {
  assert.ok(Array.isArray(tauri.plugins.updater.endpoints));
  assert.ok(tauri.plugins.updater.endpoints.length > 0);
  for (const endpoint of tauri.plugins.updater.endpoints) {
    const url = new URL(endpoint);
    assert.equal(url.protocol, 'https:');
    assert.equal(url.pathname.endsWith('/latest.json'), true);
  }
});

test('a non-empty updater public key is embedded', () => {
  const key = tauri.plugins.updater.pubkey;
  assert.equal(typeof key, 'string');
  assert.ok(key.length > 80);
  assert.doesNotThrow(() => Buffer.from(key, 'base64'));
});

test('desktop capability permits updater operations', () => {
  assert.ok(capability.permissions.includes('updater:default'));
});

test('release builds require the private signing secret', () => {
  assert.match(workflow, /TAURI_SIGNING_PRIVATE_KEY:\s*\$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY \}\}/);
});

test('release workflow publishes installers, signatures, and manifest', () => {
  for (const artifact of ['*.exe', '*.exe.sig', '*.AppImage', '*.AppImage.sig', 'latest.json']) {
    assert.ok(workflow.includes(artifact), `${artifact} is not published`);
  }
});

test('release workflow uses the tested manifest generator', () => {
  assert.match(workflow, /python3 scripts\/create_updater_manifest\.py/);
});
