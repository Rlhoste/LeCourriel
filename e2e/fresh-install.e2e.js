'use strict';

// A new computer: the app was just installed, the user opens it and sets it up.
// Every check reads what is on screen, like the user would.
//
// Run with scripts/e2e.sh (it builds the app and starts a virtual display).

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const { FakeImap } = require('./fakes/imap-server');
const { FakeAi } = require('./fakes/ai-server');
const fx = require('./fixtures');
const h = require('./harness');

const USER = 'user@example.test';
const PASS = 'app-password';
const AI_KEY = 'good-key';
const PLACEHOLDERS = ['Could not classify email.', 'Unable to summarize email.'];

let cert, imap, imapPort, ai, aiUrl;

test.before(async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'courriel-e2e-cert-'));
  cert = h.makeCert(dir);
  ai = new FakeAi({ key: AI_KEY, rules: fx.AI_RULES });
  aiUrl = `http://127.0.0.1:${await ai.listen()}/v1`;
});

test.after(async () => {
  await ai.close();
});

// A new mailbox for each journey, so they don't depend on each other.
test.beforeEach(async () => {
  imap = new FakeImap({ cert: cert.cert, key: cert.key, user: USER, pass: PASS });
  fx.seedInbox(imap);
  imapPort = await imap.listen();
  ai.requests = [];
  ai.key = AI_KEY;
});

test.afterEach(async () => {
  await imap.close();
});

const account = () => ({ host: 'localhost', port: imapPort, user: USER, pass: PASS });

/** Run `fn(browser)` in a freshly installed app; the app log is printed if it fails. */
async function withFreshApp(fn) {
  const app = await h.launchFreshApp({ certFile: cert.certFile });
  try {
    await fn(app.browser, app);
  } catch (e) {
    const log = fs.existsSync(app.logFile) ? fs.readFileSync(app.logFile, 'utf8') : '';
    e.message += `\n── app log (last 60 lines) ──\n${log.split('\n').slice(-60).join('\n')}`;
    throw e;
  } finally {
    await app.close();
  }
}

/** The Overview a user should see once the AI has read the mailbox. */
async function assertTriaged(browser) {
  await h.showPanel(browser, 'overview');
  await h.waitText(browser, '#ov-urgent-list', fx.SUBJECTS.urgent);
  await h.waitText(browser, '#ov-urgent-list', fx.SUBJECTS.meeting);
  await h.waitText(browser, '#ov-tasks-list', fx.TASK_TEXT);
  await h.waitText(browser, '#ov-deadlines-list', fx.SUBJECTS.task);
  await h.waitText(browser, '#ov-week-strip', fx.SUBJECTS.task);
  await h.waitText(browser, '#ov-meetings-list', fx.SUBJECTS.meeting);

  // Each task shows when it appeared: the date of its email (received 2 hours ago).
  const appeared = await browser.execute(ts => t('kanban.appeared', { date: fmtDay(ts) }),
    Math.floor(Date.now() / 1000) - 2 * 3600);
  await h.waitText(browser, '#ov-tasks-list', appeared);

  await h.showPanel(browser, 'tasks');
  await h.waitText(browser, '.kanban-col[data-status="todo"] .kanban-cards', fx.TASK_TEXT);
  await h.waitText(browser, '.kanban-col[data-status="todo"] .kanban-cards', appeared);

  await h.showPanel(browser, 'inbox');
  const inbox = await h.textOf(browser, '#panel-inbox .email-list');
  for (const p of PLACEHOLDERS) assert.ok(!inbox.includes(p), `The inbox shows "${p}":\n${inbox}`);
  await h.showPanel(browser, 'overview');
}

test('only the email account entered: emails arrive, the app asks for the AI, then sorts them', async () => {
  await withFreshApp(async browser => {
    // First start opens Settings on Accounts by itself.
    await h.fillAccount(browser, account());
    await h.save(browser);

    // The emails come in without touching anything.
    await h.showPanel(browser, 'inbox');
    await h.waitText(browser, '#panel-inbox .email-list', fx.SUBJECTS.urgent);
    await h.waitText(browser, '#panel-inbox .email-list', fx.SUBJECTS.newsletter);

    // The user is told the AI isn't set up, and gets a way to do it.
    await h.showPanel(browser, 'overview');
    const setup = await h.waitVisible(browser, '#ov-setup-ai', 30000);
    await setup.click();
    await h.waitVisible(browser, '#s-ai-provider');

    // Setting up the AI is enough: the emails already there get sorted.
    await h.fillAi(browser, { url: aiUrl, key: AI_KEY, model: 'fake-model' });
    await h.save(browser);
    await assertTriaged(browser);
    assert.equal(await (await browser.$('#ov-setup-ai')).isDisplayed().catch(() => false), false);
  });
});

test('account and AI set up in the same Save: everything is sorted', async () => {
  await withFreshApp(async browser => {
    await h.fillAccount(browser, account());
    await h.fillAi(browser, { url: aiUrl, key: AI_KEY, model: 'fake-model' });
    await h.save(browser);
    await assertTriaged(browser);

    // Already read on another computer, still urgent: one click away.
    assert.ok(!(await h.textOf(browser, '#ov-urgent-list')).includes(fx.SUBJECTS.readUrgent));
    await (await browser.$('#ov-toggle-opened')).click();
    await h.waitText(browser, '#ov-urgent-list', fx.SUBJECTS.readUrgent);
    await (await browser.$('#ov-toggle-opened')).click();

    // A new project from one short sentence: the AI gets the related emails and the
    // description comes back as "<sentence>. People: …. Keywords: …."
    ai.project = {
      name: 'Production outage', summary: 'Handling the production server outage reported by Alice',
      people: ['Alice Ops (operations)'], keywords: ['production server', 'outage', 'downtime'], importance: 'high',
    };
    ai.projectMatch = /Subject: Production server is down/;
    await h.showPanel(browser, 'projects');
    await (await h.waitVisible(browser, '#projects-new')).click();
    await (await h.waitVisible(browser, '#projects-new-sentence')).setValue('the server outage alice told me about');
    await (await browser.$('#projects-new-create')).click();
    const desc = await h.waitVisible(browser, '#projects-desc');
    await browser.waitUntil(async () => (await desc.getValue()).includes('Keywords:'),
      { timeout: 15000, timeoutMsg: 'The project description was not prepared by the AI' });
    assert.equal(await desc.getValue(),
      'Handling the production server outage reported by Alice. People: Alice Ops (operations). ' +
      'Keywords: production server, outage, downtime.');
    const asked = ai.requests.find(r => r.kind === 'project');
    assert.ok(asked.user.includes('REQUEST: the server outage alice told me about'), asked.user);
    assert.ok(asked.user.includes(`Subject: ${fx.SUBJECTS.urgent}`), `the related email was not sent:\n${asked.user}`);
    // The emails already in the inbox that belong to it are filed straight away.
    await h.waitText(browser, '#projects-emails-list', fx.SUBJECTS.urgent);
    assert.ok(!(await h.textOf(browser, '#projects-emails-list')).includes(fx.SUBJECTS.newsletter));
    await h.waitText(browser, '#projects-list', 'Production outage');
    await h.showPanel(browser, 'overview');

    // A new email arrives while the app is open.
    imap.append('INBOX', fx.lateEmail());
    await (await browser.$('#btn-refresh')).click();
    await h.waitText(browser, '#ov-urgent-list', fx.SUBJECTS.late);
  });
});

test('wrong API key: the error is shown, and fixing the key sorts the emails', async () => {
  await withFreshApp(async browser => {
    await h.fillAccount(browser, account());
    await h.fillAi(browser, { url: aiUrl, key: 'wrong-key', model: 'fake-model' });
    await h.save(browser);

    await h.waitText(browser, '#ov-briefing', 'Incorrect API key provided');

    await h.fillAi(browser, { url: aiUrl, key: AI_KEY, model: 'fake-model' });
    await h.save(browser);
    await assertTriaged(browser);
  });
});

// Opt-in, against a real mailbox and a real AI:
//   E2E_LIVE=1 E2E_IMAP_HOST=… E2E_IMAP_USER=… E2E_IMAP_PASS=… [E2E_IMAP_PORT=993]
//   E2E_AI_URL=https://api.openai.com/v1 E2E_AI_KEY=… E2E_AI_MODEL=… scripts/e2e.sh
test('live: a real mailbox and a real AI', { skip: !process.env.E2E_LIVE && 'set E2E_LIVE=1 to run' }, async () => {
  const env = process.env;
  const app = await h.launchFreshApp({ extraEnv: { SSL_CERT_FILE: '' } });
  try {
    const { browser } = app;
    await h.fillAccount(browser, {
      host: env.E2E_IMAP_HOST, port: env.E2E_IMAP_PORT || 993, user: env.E2E_IMAP_USER, pass: env.E2E_IMAP_PASS,
    });
    await h.fillAi(browser, { url: env.E2E_AI_URL, key: env.E2E_AI_KEY, model: env.E2E_AI_MODEL });
    await h.save(browser);
    await h.showPanel(browser, 'inbox');
    // Wait for the live briefing ("Updated at …"), then judge the result.
    await browser.waitUntil(async () => {
      const state = await browser.execute(() => typeof appState !== "undefined" && appState?.briefing?.kind);
      return state === 'live';
    }, { timeout: 600000, interval: 2000, timeoutMsg: 'Classification never finished' });
    const counts = await browser.execute(() => ({
      all: appState.all_emails.length,
      sorted: appState.all_emails.filter(e => e.priority !== 'information').length,
      placeholders: appState.all_emails.filter(e => !e.summary || e.summary.startsWith('Could not') || e.summary.startsWith('Unable to')).length,
    }));
    assert.ok(counts.all > 0, 'no email fetched');
    assert.ok(counts.sorted > 0, `every email is "information": ${JSON.stringify(counts)}`);
    assert.equal(counts.placeholders, 0, `some emails were not classified: ${JSON.stringify(counts)}`);
  } finally {
    await app.close();
  }
});
