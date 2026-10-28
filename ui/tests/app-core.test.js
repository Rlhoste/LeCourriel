'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const {
  normalizedSort,
  buildDiscussions,
  sortEmails,
  removeEmailFromState,
  autoUpdatesEnabled,
  saveAutoUpdates,
  updateProgressPercent,
  needsAccountSetup,
  attentionEmails,
  activeTasks,
  threadSubject,
  groupThreads,
} = require('../app-core.js');

const mail = (uid, overrides = {}) => ({
  uid,
  date_ts: uid * 10,
  from_name: '',
  from_email: `person${uid}@example.com`,
  subject: `Subject ${uid}`,
  is_read: false,
  priority: 'information',
  project: null,
  ...overrides,
});

test('missing sort defaults to newest date first', () => {
  assert.deepEqual(normalizedSort(), { key: 'date', ascending: false });
});

test('unknown sort key safely defaults to date', () => {
  assert.equal(normalizedSort({ key: 'broken', ascending: true }).key, 'date');
});

test('sort does not mutate the application email array', () => {
  const source = [mail(1), mail(3), mail(2)];
  const before = source.map(item => item.uid);
  sortEmails(source, { key: 'date', ascending: false });
  assert.deepEqual(source.map(item => item.uid), before);
});

test('date sort supports newest first', () => {
  assert.deepEqual(sortEmails([mail(1), mail(3), mail(2)], { key: 'date', ascending: false }).map(e => e.uid), [3, 2, 1]);
});

test('date sort supports oldest first', () => {
  assert.deepEqual(sortEmails([mail(3), mail(1), mail(2)], { key: 'date', ascending: true }).map(e => e.uid), [1, 2, 3]);
});

test('sender sort prefers display name and ignores case', () => {
  const result = sortEmails([
    mail(1, { from_name: 'zoe' }),
    mail(2, { from_name: 'Alice' }),
    mail(3, { from_name: 'bob' }),
  ], { key: 'sender', ascending: true }, 'en-GB');
  assert.deepEqual(result.map(e => e.from_name), ['Alice', 'bob', 'zoe']);
});

test('sender sort falls back to email address', () => {
  const result = sortEmails([
    mail(1, { from_email: 'z@example.com' }),
    mail(2, { from_email: 'a@example.com' }),
  ], { key: 'sender', ascending: true });
  assert.deepEqual(result.map(e => e.uid), [2, 1]);
});

test('subject sort supports descending order', () => {
  const result = sortEmails([
    mail(1, { subject: 'Alpha' }), mail(2, { subject: 'Zulu' }), mail(3, { subject: 'Mike' }),
  ], { key: 'subject', ascending: false });
  assert.deepEqual(result.map(e => e.subject), ['Zulu', 'Mike', 'Alpha']);
});

test('read status can put unread messages first', () => {
  const result = sortEmails([
    mail(1, { is_read: true }), mail(2, { is_read: false }), mail(3, { is_read: true }),
  ], { key: 'read', ascending: true });
  assert.equal(result[0].is_read, false);
});

test('priority follows the visible business order', () => {
  const priorities = ['information', 'task', 'threat', 'urgent', 'other_priority', 'high_priority'];
  const result = sortEmails(priorities.map((priority, i) => mail(i + 1, { priority })), { key: 'priority', ascending: true });
  assert.deepEqual(result.map(e => e.priority), ['urgent', 'high_priority', 'task', 'other_priority', 'threat', 'information']);
});

test('unknown priority sorts after known priorities', () => {
  const result = sortEmails([mail(1, { priority: 'unknown' }), mail(2, { priority: 'urgent' })], { key: 'priority', ascending: true });
  assert.deepEqual(result.map(e => e.priority), ['urgent', 'unknown']);
});

test('project sort handles messages without a project', () => {
  const result = sortEmails([
    mail(1, { project: 'Zulu' }), mail(2, { project: null }), mail(3, { project: 'Alpha' }),
  ], { key: 'project', ascending: true });
  assert.deepEqual(result.map(e => e.project), [null, 'Alpha', 'Zulu']);
});

test('equal primary values use date and uid as deterministic fallback', () => {
  const result = sortEmails([
    mail(1, { subject: 'Same', date_ts: 10 }),
    mail(3, { subject: 'Same', date_ts: 20 }),
    mail(2, { subject: 'Same', date_ts: 20 }),
  ], { key: 'subject', ascending: true });
  assert.deepEqual(result.map(e => e.uid), [3, 2, 1]);
});

test('removing an inbox email removes it from derived inbox lists', () => {
  const state = {
    all_emails: [mail(1), mail(2)], urgent_emails: [mail(1)], high_emails: [mail(2)],
    meeting_items: [mail(1)], deadlines: [{ uid: 1 }], tasks: [{ id: 9, source_uid: 1 }],
    week_days: [], projects: [{ name: 'Keep' }], sent_emails: [mail(1)], drafts_emails: [mail(1)],
    trash_emails: [mail(1)], spam_emails: [mail(1)],
  };
  removeEmailFromState(state, 1);
  assert.deepEqual(state.all_emails.map(e => e.uid), [2]);
  assert.equal(state.urgent_emails.length, 0);
  assert.equal(state.meeting_items.length, 0);
  assert.equal(state.sent_emails.length, 1, 'folder with separate UID namespace is preserved');
  assert.equal(state.projects.length, 1);
});

test('meeting dismissal only removes the meeting entry', () => {
  const state = { all_emails: [mail(1)], meeting_items: [mail(1)], week_days: [] };
  removeEmailFromState(state, 1, 'meeting');
  assert.equal(state.meeting_items.length, 0);
  assert.equal(state.all_emails.length, 1);
});

test('deadline dismissal clears matching calendar labels', () => {
  const state = {
    deadlines: [{ uid: 7 }, { uid: 8 }],
    week_days: [{ dl1_uid: 7, dl1_subject: 'Remove', dl2_uid: 8, dl2_subject: 'Keep' }],
  };
  removeEmailFromState(state, 7, 'deadline');
  assert.deepEqual(state.deadlines.map(d => d.uid), [8]);
  assert.equal(state.week_days[0].dl1_subject, '');
  assert.equal(state.week_days[0].dl2_subject, 'Keep');
});

test('automatic updates default to enabled on an empty installation', () => {
  assert.equal(autoUpdatesEnabled({ getItem: () => null }), true);
});

test('automatic updates respect an explicit opt-out', () => {
  assert.equal(autoUpdatesEnabled({ getItem: () => 'false' }), false);
});

test('automatic updates fail open when browser storage is unavailable', () => {
  assert.equal(autoUpdatesEnabled({ getItem: () => { throw new Error('blocked'); } }), true);
});

test('automatic update preference is persisted as a boolean string', () => {
  const values = new Map();
  const storage = { getItem: key => values.get(key), setItem: (key, value) => values.set(key, value) };
  assert.equal(saveAutoUpdates(storage, false), true);
  assert.equal(values.get('autoUpdate'), 'false');
  assert.equal(autoUpdatesEnabled(storage), false);
});

test('storage write failures are reported without throwing', () => {
  assert.equal(saveAutoUpdates({ setItem: () => { throw new Error('full'); } }, true), false);
});

test('download progress calculates a percentage', () => {
  assert.equal(updateProgressPercent({ downloaded: 25, total: 100 }), 25);
});

test('download progress is clamped to valid bounds', () => {
  assert.equal(updateProgressPercent({ downloaded: 200, total: 100 }), 100);
  assert.equal(updateProgressPercent({ downloaded: -10, total: 100 }), 0);
});

test('finished installation always reports 100 percent', () => {
  assert.equal(updateProgressPercent({ finished: true }), 100);
});

test('unknown download size has no misleading percentage', () => {
  assert.equal(updateProgressPercent({ downloaded: 10, total: 0 }), null);
  assert.equal(updateProgressPercent({}), null);
});

test('empty and missing settings trigger first-run account setup', () => {
  assert.equal(needsAccountSetup(undefined), true);
  assert.equal(needsAccountSetup({}), true);
  assert.equal(needsAccountSetup({ accounts: [] }), true);
});

test('a fully configured app does not reopen account setup', () => {
  assert.equal(needsAccountSetup({ accounts: [{ label: 'Work' }] }), false);
});

test('overview attention combines only unread urgent and high-priority mail', () => {
  const state = {
    urgent_emails: [mail(1), mail(2, { is_read: true })],
    high_emails: [mail(3), mail(4, { is_read: true })],
  };
  assert.deepEqual(attentionEmails(state).map(item => item.uid), [1, 3]);
});

test('overview attention can include already-opened urgent and high-priority mail', () => {
  const state = {
    urgent_emails: [mail(1), mail(2, { is_read: true })],
    high_emails: [mail(3), mail(4, { is_read: true })],
  };
  assert.deepEqual(attentionEmails(state, true).map(item => item.uid), [1, 2, 3, 4]);
});

test('empty application state has no attention mail', () => {
  assert.deepEqual(attentionEmails({}), []);
});

test('overview tasks include todo and in-progress work only', () => {
  const state = { tasks: [
    { id: 1, status: 'todo' }, { id: 2, status: 'in_progress' },
    { id: 3, status: 'done' }, { id: 4, status: 'given_up' },
  ] };
  assert.deepEqual(activeTasks(state).map(item => item.id), [1, 2]);
});

test('empty application state has no active tasks', () => {
  assert.deepEqual(activeTasks({}), []);
});

test('reply prefixes are ignored when matching subjects', () => {
  assert.equal(threadSubject('Re: TR: Fwd:  Budget  2026'), 'budget 2026');
  assert.equal(threadSubject('RE[2]: budget 2026'), 'budget 2026');
});

test('replies are grouped with the email they answer, by headers', () => {
  const sorted = sortEmails([
    mail(1, { message_id: '<a@x>', subject: 'Budget' }),
    mail(2, { message_id: '<b@x>', subject: 'Other' }),
    mail(3, { message_id: '<c@x>', references: '<a@x>', subject: 'Changed subject' }),
    mail(4, { message_id: '<d@x>', references: '<a@x> <c@x>', subject: 'Re: Budget' }),
  ]);
  const groups = groupThreads(sorted);
  assert.deepEqual(groups.map(g => g.lead.uid), [4, 2]);
  assert.deepEqual(groups[0].replies.map(e => e.uid), [3, 1]);
  assert.deepEqual(groups[1].replies, []);
});

test('a reply without headers joins the email with the same subject', () => {
  const groups = groupThreads(sortEmails([
    mail(1, { subject: 'Defence date' }),
    mail(2, { subject: 'RE: Defence date' }),
    mail(3, { subject: 'Weekly report' }),
    mail(4, { subject: 'Weekly report' }),
  ]));
  assert.deepEqual(groups.map(g => [g.lead.uid, g.replies.map(e => e.uid)]), [[4, []], [3, []], [2, [1]]]);
});

// Discussion mode. `at(day, hour)` is a local time in October 2026; "now" is Wed 28 Oct, noon.
const at = (day, hour = 9, minute = 0) => new Date(2026, 9, day, hour, minute).getTime() / 1000;
const ME = { me: ['me@work.com', 'Me@Home.org'], now: new Date(2026, 9, 28, 12).getTime() };
const messageUids = d => d.items.filter(i => i.kind === 'message').map(i => i.email.uid);

test('a discussion gathers linked emails of every folder, oldest first', () => {
  const list = buildDiscussions({
    inbox: [mail(1, { message_id: '<a@x>', subject: 'Budget' }), mail(5, { references: '<a@x>' })],
    sent: [mail(3, { from_email: 'me@work.com', subject: 'Re: Budget' })],
    trash: [mail(4, { references: '<a@x>' })],
    spam: [mail(2, { subject: 'Win a prize' })],
  }, ME);
  assert.equal(list.length, 1);
  assert.deepEqual(messageUids(list[0]), [1, 3, 4, 5]);
  assert.equal(list[0].id, '<a@x>', 'kept while new emails arrive');
});

test('a discussion only in Trash or Spam is not listed; the list is newest first', () => {
  const list = buildDiscussions({
    inbox: [mail(1, { subject: 'Old' }), mail(3, { subject: 'New' })],
    sent: [mail(2, { from_email: 'me@work.com', subject: 'Middle' })],
    trash: [mail(4, { subject: 'Deleted' })],
    spam: [mail(5, { subject: 'Junk' })],
  }, ME);
  assert.deepEqual(list.map(d => d.subject), ['New', 'Middle', 'Old']);
});

test('a list line shows the people, the last message, unread count and top unread priority', () => {
  const [line] = buildDiscussions({
    inbox: [
      mail(1, { message_id: '<a@x>', from_name: 'Leo Bianchini', from_email: 'leo@d.com', to: 'Me <me@work.com>, "Damien Delhay" <damien@o.com>', cc: 'ann@x.com, bob@x.com', priority: 'urgent', is_read: true }),
      mail(2, { references: '<a@x>', from_name: 'Damien Delhay', from_email: 'damien@o.com', priority: 'task', body_text: 'Gain fixed.' }),
      mail(3, { references: '<a@x>', from_name: 'Leo Bianchini', from_email: 'leo@d.com', priority: 'high_priority', body_text: 'Thanks' }),
    ],
    sent: [mail(4, { references: '<a@x>', from_email: 'ME@home.org', body_text: '  Sure,\n\nsee you  Friday.\n' })],
  }, ME);
  assert.deepEqual(line.people.map(p => [p.name, p.address, p.initials]), [
    ['Leo Bianchini', 'leo@d.com', 'LB'], ['Damien Delhay', 'damien@o.com', 'DD'], ['', 'ann@x.com', 'AN'], ['', 'bob@x.com', 'BO'],
  ]);
  assert.equal(line.title, 'Leo, Damien +2');
  assert.deepEqual(line.preview, { mine: true, text: 'Sure, see you Friday.' });
  assert.equal(line.date_ts, 40);
  assert.equal(line.unread, 2);
  assert.equal(line.priority, 'high_priority');
});

test('a line without unread emails has no priority mark', () => {
  const [line] = buildDiscussions({ inbox: [mail(1, { priority: 'urgent', is_read: true, from_name: 'Ann' })] }, ME);
  assert.equal(line.unread, 0);
  assert.equal(line.priority, null);
  assert.equal(line.title, 'Ann');
  assert.deepEqual(line.preview, { mine: false, text: '' });
});

test('my messages (any of my accounts) and theirs take opposite sides, with the sender and its colour', () => {
  const [d] = buildDiscussions({
    inbox: [mail(1, { message_id: '<a@x>', from_name: 'Leo Bianchini', from_email: 'leo@d.com', body_text: ' Hello\nDavid ' })],
    sent: [mail(2, { references: '<a@x>', from_email: 'me@home.org' })],
  }, ME);
  const [theirs, mine] = d.items.filter(i => i.kind === 'message');
  assert.equal(theirs.mine, false);
  assert.equal(theirs.sender.name, 'Leo Bianchini');
  assert.equal(theirs.sender.initials, 'LB');
  assert.match(theirs.sender.colour, /^#[0-9a-f]{6}$/);
  assert.equal(theirs.text, 'Hello\nDavid');
  assert.equal(theirs.date_ts, 10);
  assert.equal(mine.mine, true);
});

test('messages in a row from the same person stack; a new day starts with a separator', () => {
  const leo = { from_name: 'Leo', from_email: 'leo@d.com' };
  const me = { from_email: 'me@work.com' };
  const [d] = buildDiscussions({
    inbox: [
      mail(1, { ...leo, message_id: '<a@x>', date_ts: at(12, 9) }),
      mail(2, { ...leo, references: '<a@x>', date_ts: at(26, 9) }),
      mail(3, { ...leo, references: '<a@x>', date_ts: at(26, 10) }),
      mail(4, { ...leo, references: '<a@x>', date_ts: at(26, 11) }),
      mail(6, { ...leo, references: '<a@x>', date_ts: at(28, 8) }),
      mail(7, { ...leo, references: '<a@x>', date_ts: at(28, 9) }),
    ],
    sent: [mail(5, { ...me, references: '<a@x>', date_ts: at(26, 12) })],
  }, ME);
  const shape = d.items.map(i => (i.kind === 'day' ? i.day : `${i.email.uid}${i.first ? 'F' : ''}${i.last ? 'L' : ''}`));
  assert.deepEqual(shape, ['date', '1FL', 'weekday', '2F', '3', '4L', '5FL', 'today', '6F', '7L']);
  assert.equal(d.items[2].date_ts, at(26, 9));
  assert.equal(d.items[1].sender.colour, d.items[3].sender.colour);
});

// One item of a discussion, reduced to what shows: kind, content and its place in the run.
const shown = i => [i.kind, i.text ?? i.src ?? i.name ?? i.email?.uid, `${i.first ? 'F' : ''}${i.last ? 'L' : ''}`];

test('an image in the middle of an email splits it: text before, the images, then the rest', () => {
  const [d] = buildDiscussions({
    inbox: [mail(1, {
      body_text: 'Look at this:\n\nNice, isn\'t it?',
      body_html: '<html><head><style>p{color:red}</style></head><body><p>Look at <b>this</b>:</p>'
        + '<img alt="" src="data:image/png;base64,AAA"><IMG SRC=\'data:image/jpeg;base64,BBB\' width="10">'
        + '<div>Nice,&nbsp;isn&#39;t it?<br>Bye &amp; see you</div></body></html>',
    })],
  }, ME);
  assert.deepEqual(d.items.slice(1).map(shown), [
    ['message', 'Look at this:', 'F'],
    ['image', 'data:image/png;base64,AAA', ''],
    ['image', 'data:image/jpeg;base64,BBB', ''],
    ['message', 'Nice, isn\'t it?\nBye & see you', 'L'],
  ]);
  assert.ok(d.items.slice(1).every(i => i.email.uid === 1 && i.sender.address === 'person1@example.com' && i.date_ts === 10));
});

test('an email without an image in its HTML keeps its plain text whole', () => {
  const [d] = buildDiscussions({
    inbox: [mail(1, { body_text: 'Hello\n\nDavid', body_html: '<p>Hello</p><p>David</p><img src="https://x.com/logo.png">' })],
  }, ME);
  assert.deepEqual(d.items.slice(1).map(shown), [['message', 'Hello\n\nDavid', 'FL']]);
});

test('each attachment shows as its own item after the text, with its name, size and type', () => {
  const leo = { from_email: 'leo@d.com' };
  const [d] = buildDiscussions({
    inbox: [
      mail(1, { ...leo, message_id: '<a@x>', body_text: 'Both files', attachments: [
        { name: 'plan.pdf', size: 52300, mime: 'application/pdf' },
        { name: 'photo.jpg', size: 1200000, mime: 'image/jpeg' },
      ] }),
      mail(2, { ...leo, references: '<a@x>', body_text: '', attachments: [{ name: 'notes.txt', size: 12, mime: 'text/plain' }] }),
    ],
  }, ME);
  assert.deepEqual(d.items.slice(1).map(shown), [
    ['message', 'Both files', 'F'],
    ['attachment', 'plan.pdf', ''],
    ['attachment', 'photo.jpg', ''],
    ['attachment', 'notes.txt', 'L'],
  ]);
  assert.deepEqual([d.items[3].size, d.items[3].mime], [1200000, 'image/jpeg']);
  assert.equal(d.items[4].email.uid, 2);
});

test('an email laid out as a page (mostly tables, or no plain-text part) shows as one card, not cut', () => {
  const newsletter = '<p>View online</p><table><tr><td><img src="data:image/png;base64,LOGO"></td></tr>'
    + '<tr><td><h1>October news</h1><p>Our new shop opens on Monday with a lot of offers.</p></td></tr></table>';
  const [d] = buildDiscussions({
    inbox: [
      mail(1, { from_email: 'shop@x.com', message_id: '<a@x>', body_text: 'View online\nOctober news', body_html: newsletter,
        attachments: [{ name: 'invoice.pdf', size: 900, mime: 'application/pdf' }] }),
      mail(2, { from_email: 'shop@x.com', references: '<a@x>', body_text: 'Hi', body_html: '<div>Hi</div>', html_only: true }),
    ],
  }, ME);
  assert.deepEqual(d.items.slice(1).map(shown), [['page', 1, 'F'], ['attachment', 'invoice.pdf', ''], ['page', 2, 'L']]);
});

test('a message whose signature is a small table is still a message', () => {
  const [d] = buildDiscussions({
    inbox: [mail(1, {
      body_text: 'Sure, the meeting is moved to Friday at ten, in the big room.\nLeo | CEO',
      body_html: '<p>Sure, the meeting is moved to Friday at ten, in the big room.</p><table><tr><td>Leo</td><td>CEO</td></tr></table>',
    })],
  }, ME);
  assert.deepEqual(d.items.slice(1).map(shown), [['message', 'Sure, the meeting is moved to Friday at ten, in the big room.\nLeo | CEO', 'FL']]);
});
