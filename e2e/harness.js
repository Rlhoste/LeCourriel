'use strict';

// Launches the real app window, as on a computer where it was just installed, and
// drives it through WebDriver (tauri-driver → WebKitWebDriver). Each launch gets its
// own empty data folder and HOME, so no config, cache, tasks or storage carry over.

const { spawn, execFileSync } = require('node:child_process');
const fs = require('node:fs');
const net = require('node:net');
const os = require('node:os');
const path = require('node:path');

const ROOT = path.resolve(__dirname, '..');
const APP = process.env.E2E_APP || path.join(ROOT, 'src-tauri/target/debug/courriel');

/** A throwaway self-signed certificate for "localhost", trusted by the app via SSL_CERT_FILE. */
function makeCert(dir) {
  const cert = path.join(dir, 'cert.pem');
  const key = path.join(dir, 'key.pem');
  execFileSync('openssl', [
    'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '2',
    '-subj', '/CN=localhost', '-addext', 'subjectAltName=DNS:localhost,IP:127.0.0.1',
    '-keyout', key, '-out', cert,
  ], { stdio: 'ignore' });
  return { certFile: cert, cert: fs.readFileSync(cert, 'utf8'), key: fs.readFileSync(key, 'utf8') };
}

function freePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.unref();
    srv.on('error', reject);
    srv.listen(0, '127.0.0.1', () => {
      const { port } = srv.address();
      srv.close(() => resolve(port));
    });
  });
}

async function waitForPort(port, ms = 20000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const ok = await new Promise(resolve => {
      const s = net.connect(port, '127.0.0.1', () => { s.end(); resolve(true); });
      s.on('error', () => resolve(false));
    });
    if (ok) return;
    await new Promise(r => setTimeout(r, 200));
  }
  throw new Error(`tauri-driver did not start on port ${port}`);
}

/**
 * Start the app on a fresh install. Returns { browser, dataDir, close }.
 * `certFile`: the CA bundle the app trusts (the fake IMAP server's certificate).
 */
async function launchFreshApp({ certFile, extraEnv = {} } = {}) {
  const { remote } = await import('webdriverio');
  if (!fs.existsSync(APP)) throw new Error(`App not built: ${APP} (run scripts/e2e.sh)`);
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'courriel-e2e-'));
  const dirs = { home: 'home', data: 'data', xdgData: 'xdg-data', xdgConfig: 'xdg-config', xdgCache: 'xdg-cache' };
  for (const k of Object.keys(dirs)) {
    dirs[k] = path.join(root, dirs[k]);
    fs.mkdirSync(dirs[k], { recursive: true });
  }
  const env = {
    ...process.env,
    HOME: dirs.home,
    XDG_DATA_HOME: dirs.xdgData,
    XDG_CONFIG_HOME: dirs.xdgConfig,
    XDG_CACHE_HOME: dirs.xdgCache,
    COURRIEL_DATA_DIR: dirs.data,
    RUST_LOG: 'courriel=debug',
    ...(certFile ? { SSL_CERT_FILE: certFile } : {}),
    ...extraEnv,
  };
  const port = await freePort();
  const nativePort = await freePort();
  const logFile = path.join(root, 'app.log');
  const log = fs.openSync(logFile, 'a');
  const driver = spawn('tauri-driver', ['--port', String(port), '--native-port', String(nativePort)], {
    cwd: root, env, stdio: ['ignore', log, log],
  });
  let browser;
  try {
    await waitForPort(port);
    browser = await remote({
      hostname: '127.0.0.1',
      port,
      logLevel: 'error',
      // WebKitWebDriver speaks classic WebDriver only, not BiDi.
      capabilities: { 'wdio:enforceWebDriverClassic': true, 'tauri:options': { application: APP } },
    });
  } catch (e) {
    driver.kill();
    throw e;
  }
  return {
    browser,
    dataDir: dirs.data,
    logFile,
    async close() {
      await browser.deleteSession().catch(() => {});
      driver.kill();
      await new Promise(r => driver.once('exit', r).once('error', r));
      if (!process.env.E2E_KEEP) fs.rmSync(root, { recursive: true, force: true });
    },
  };
}

// ── What a user does ────────────────────────────────────────────────────────

async function waitVisible(browser, selector, timeout = 15000) {
  const el = await browser.$(selector);
  await el.waitForDisplayed({ timeout, timeoutMsg: `${selector} never became visible` });
  return el;
}

async function type(browser, selector, value) {
  const el = await waitVisible(browser, selector);
  await el.setValue(value);
}

/** Fill the account the app shows on first start, as a user would. */
async function fillAccount(browser, { host, port, user, pass }) {
  await waitVisible(browser, '#settings-overlay .account-block');
  await type(browser, '.account-block .a-sender-email', user);
  await type(browser, '.account-block .a-imap-host', host);
  await type(browser, '.account-block .a-imap-port', String(port));
  await type(browser, '.account-block .a-imap-user', user);
  await type(browser, '.account-block .a-imap-pass', pass);
  await type(browser, '.account-block .a-smtp-host', host);
}

async function openSettingsTab(browser, tab) {
  const overlay = await browser.$('#settings-overlay');
  if (!(await overlay.isDisplayed())) await (await browser.$('#btn-settings')).click();
  await (await waitVisible(browser, `#settings-overlay .tab[data-tab="${tab}"]`)).click();
}

/** Settings → AI → Custom provider pointing at `url`. */
async function fillAi(browser, { url, key, model }) {
  await openSettingsTab(browser, 'ai');
  await (await waitVisible(browser, '#s-ai-provider')).selectByAttribute('value', 'custom');
  await type(browser, '#s-ai-url', url);
  await type(browser, '#s-ai-key', key);
  await type(browser, '#s-ai-model', model);
}

async function save(browser) {
  await (await waitVisible(browser, '#settings-save')).click();
  await (await browser.$('#settings-overlay')).waitForDisplayed({ reverse: true, timeout: 10000,
    timeoutMsg: 'Settings did not close after Save' });
}

async function showPanel(browser, panel) {
  await (await browser.$(`.nav-btn[data-panel="${panel}"]`)).click();
}

/** Text of an element, '' if it is missing or hidden. (WebKitWebDriver's getText drops
 *  ellipsised text such as subjects, so read the DOM, but only for displayed elements.) */
async function textOf(browser, selector) {
  const el = await browser.$(selector);
  if (!(await el.isExisting()) || !(await el.isDisplayed())) return '';
  return browser.execute(e => e.textContent.replace(/\s+/g, ' ').trim(), el);
}

/** Wait until the element's visible text contains `needle`; the error shows what was there. */
async function waitText(browser, selector, needle, timeout = 30000) {
  let last = '';
  try {
    await browser.waitUntil(async () => (last = await textOf(browser, selector)).includes(needle),
      { timeout, interval: 300 });
  } catch {
    throw new Error(`"${needle}" never appeared in ${selector}. It showed:\n${last || '(nothing)'}`);
  }
}

module.exports = {
  makeCert, launchFreshApp, fillAccount, fillAi, openSettingsTab, save, showPanel,
  textOf, waitText, waitVisible,
};
