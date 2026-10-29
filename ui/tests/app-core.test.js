'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const {
  normalizedSort,
  buildDiscussions,
  filterDiscussions,
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

test('unread emails in Trash or Spam count neither as unread nor for the priority mark', () => {
  const [line] = buildDiscussions({
    inbox: [mail(1, { message_id: '<a@x>', subject: 'Budget', is_read: true })],
    trash: [mail(2, { references: '<a@x>', priority: 'urgent' })],
    spam: [mail(3, { subject: 'Re: Budget', priority: 'threat' })],
  }, ME);
  assert.equal(line.unread, 0);
  assert.equal(line.priority, null);
});

test('a newer email in Trash or Spam is neither the last message of the line nor the one the reply bar answers', () => {
  const [line] = buildDiscussions({
    inbox: [mail(1, { message_id: '<a@x>', subject: 'Budget', from_email: 'leo@d.com', body_text: 'Here it is' })],
    spam: [mail(3, { subject: 'Re: Budget', from_email: 'scam@bad.com', body_text: 'Click here' })],
    trash: [mail(2, { references: '<a@x>', from_email: 'ann@x.com', body_text: 'Old' })],
  }, ME);
  assert.deepEqual(line.preview, { mine: false, text: 'Here it is' });
  assert.equal(line.date_ts, 10);
  assert.equal(line.reply.email.uid, 1);
  assert.deepEqual(addresses(line.reply.to), ['leo@d.com']);
});

test('discussions without Message-ID get distinct ids even when an Inbox and a Sent email share a uid', () => {
  const list = buildDiscussions({
    inbox: [mail(1, { subject: 'Lunch' })],
    sent: [mail(1, { subject: 'Invoice', from_email: 'me@work.com', to: 'leo@d.com' })],
  }, ME);
  assert.equal(list.length, 2);
  assert.notEqual(list[0].id, list[1].id);
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

test('a sender carries the organisation of its address for the postcard band', () => {
  const org = address => buildDiscussions({ inbox: [mail(1, { from_email: address })] }, ME)[0].items.find(i => i.kind === 'message').sender.org;
  assert.equal(org('leo@acme.com'), 'acme');
  assert.equal(org('ann@mail.studio-k.fr'), 'studio-k');
  assert.equal(org('bob@ox.ac.uk'), 'ox');
  assert.equal(org('kim@shop.co.jp'), 'shop');
  assert.equal(org('dev@localhost'), 'localhost');
  assert.equal(org('nobody'), '');
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

test('a short reply quoting an email laid out in tables is still a message', () => {
  const quoted = '<table><tr><td><h1>October invoice</h1><p>Total due before the end of the month: 1 200 euros.</p>'
    + '<p>Pay online or by bank transfer to the account below.</p></td></tr></table>';
  const [d] = buildDiscussions({
    inbox: [mail(1, {
      body_text: 'Paid, thanks.\n\nLe lun. 26 oct. 2026 à 09:12, Billing <billing@x.com> a écrit :\n> October invoice',
      body_html: `<p>Paid, thanks.</p><div>Le lun. 26 oct. 2026 à 09:12, Billing &lt;billing@x.com&gt; a écrit :</div><blockquote>${quoted}</blockquote>`,
    })],
  }, ME);
  assert.deepEqual(d.items.slice(1).map(shown), [['message', 'Paid, thanks.', 'FL']]);
});

test('in an email split by images, the quoted history and its images are cut too', () => {
  const [d] = buildDiscussions({
    inbox: [mail(1, {
      body_html: '<p>Here is the plan:</p><img src="data:image/png;base64,AAA"><p>Thanks!</p>'
        + '<div>Le lun. 26 oct. 2026, Me a écrit :</div><blockquote><p>Send it</p><img src="data:image/png;base64,BBB"></blockquote>',
    })],
  }, ME);
  assert.deepEqual(d.items.slice(1).map(shown), [
    ['message', 'Here is the plan:', 'F'], ['image', 'data:image/png;base64,AAA', ''], ['message', 'Thanks!', 'L'],
  ]);
});

const addresses = list => list.map(p => p.address);

test('the reply bar answers everyone on the latest message, never myself', () => {
  const [d] = buildDiscussions({
    inbox: [
      mail(1, { message_id: '<a@x>', subject: 'Budget', from_email: 'old@x.com' }),
      mail(2, {
        references: '<a@x>', subject: 'Re: Budget', from_name: 'Leo Bianchini', from_email: 'leo@d.com',
        to: 'Me <ME@work.com>, "Damien Delhay" <damien@o.com>, leo@d.com', cc: 'me@home.org, ann@x.com, Damien <damien@o.com>',
      }),
    ],
  }, ME);
  assert.equal(d.reply.email.uid, 2);
  assert.deepEqual(addresses(d.reply.to), ['leo@d.com', 'damien@o.com']);
  assert.deepEqual(addresses(d.reply.cc), ['ann@x.com']);
  assert.equal(d.reply.subject, 'Re: Budget');
  assert.equal(d.reply.from, 0, 'the account that received it');
});

test('the reply goes to Reply-To when set, and From follows the account that received it', () => {
  const [d] = buildDiscussions({
    inbox: [mail(2 ** 32 + 1, { subject: 'Offer', from_email: 'news@shop.com', reply_to: 'Sales <sales@shop.com>', to: 'me@home.org' })],
  }, ME);
  assert.deepEqual(addresses(d.reply.to), ['sales@shop.com']);
  assert.equal(d.reply.from, 1);
});

test('From follows the account that received the email even when none of its addresses is in To or Cc', () => {
  const second = 2 ** 32;   // emails of the second account
  const [d] = buildDiscussions({
    inbox: [mail(second + 5, { subject: 'Weekly news', from_email: 'leo@d.com', to: 'team@lists.org', cc: 'me@work.com' })],
  }, ME);
  assert.equal(d.reply.from, 1);
});

test('replying after my own message goes to its recipients, from the account that sent it', () => {
  const [d] = buildDiscussions({
    inbox: [mail(1, { message_id: '<a@x>', subject: 'Plan', from_email: 'leo@d.com', to: 'me@work.com' })],
    sent: [mail(2, { references: '<a@x>', subject: 'RE: Plan', from_email: 'me@home.org', to: 'leo@d.com', cc: 'ann@x.com' })],
  }, ME);
  assert.deepEqual(addresses(d.reply.to), ['leo@d.com']);
  assert.deepEqual(addresses(d.reply.cc), ['ann@x.com']);
  assert.equal(d.reply.subject, 'RE: Plan');
  assert.equal(d.reply.from, 1);
});

test('bubble names use the first name and a last-name initial, growing until two differ', () => {
  const [d] = buildDiscussions({
    inbox: [mail(1, {
      from_name: 'David Fort', from_email: 'david@x.com',
      to: 'Leo Bianchini <lbi@x.com>, Leo Bonnet <lbo@x.com>, Ann Lee <ann@x.com>, Ann Lee <ann2@y.com>, "nobody@z.com" <nobody@z.com>, solo@z.com, Cher <cher@z.com>',
    })],
  }, ME);
  assert.deepEqual(d.reply.to.map(p => p.label), ['David F.', 'Leo Bi.', 'Leo Bo.', 'Ann Lee', 'Ann Lee', 'nobody', 'solo', 'Cher']);
});

// Only the new text: each bubble's `text` is cut by rules (quoted history, signatures).
const bubbles = (inbox, context = ME) => buildDiscussions({ inbox }, context)
  .flatMap(d => d.items.filter(i => i.kind === 'message').map(i => i.text));

test('an Outlook quoted block (De : / Envoyé : / À : / Objet :, or From: / Sent:) and all after it is cut', () => {
  assert.deepEqual(bubbles([
    mail(1, { subject: 'A', body_text: 'Merci, c\'est noté.\n\n________________________________\nDe : Leo Bianchini <leo@d.com>\nEnvoyé : lundi 26 octobre 2026 09:12\nÀ : Me <me@work.com>\nObjet : RE: Budget\n\nVoici le budget.' }),
    mail(2, { subject: 'B', body_text: 'Thanks, noted.\n\n-----Original Message-----\nFrom: Leo Bianchini <leo@d.com>\nSent: Monday, October 26, 2026 9:12 AM\nTo: Me <me@work.com>\nSubject: RE: Budget\n\nHere is the budget.' }),
  ]), ['Thanks, noted.', 'Merci, c\'est noté.']);
});

test('"Le … a écrit :" and "On … wrote:" (even split over two lines) cut all after them; ">" lines go', () => {
  assert.deepEqual(bubbles([
    mail(1, { subject: 'A', body_text: 'Parfait.\n\nLe lun. 26 oct. 2026 à 09:12, Leo Bianchini <leo@d.com> a écrit :\n> Voici le budget.\n\nLeo' }),
    mail(2, { subject: 'B', body_text: 'Great.\n\nOn Mon, Oct 26, 2026 at 9:12 AM Leo Bianchini <\nleo@d.com> wrote:\n\nHere is the budget.' }),
    mail(3, { subject: 'C', body_text: '> Lundi ?\nOui.\n> Et mardi ?\n>> Plutôt lundi\nNon.' }),
  ]), ['Oui.\nNon.', 'Great.', 'Parfait.']);
});

test('a sentence of mine that says someone wrote something, without a date, is not a quote intro', () => {
  assert.deepEqual(bubbles([
    mail(1, { subject: 'A', body_text: 'Bonjour,\nLe client a écrit :\n« Je veux être remboursé. »\nJe propose de refuser.' }),
    mail(2, { subject: 'B', body_text: 'Hi,\nOn Friday the client\nwrote:\nI want a refund.\nI suggest we say no.' }),
  ]), ['Hi,\nOn Friday the client\nwrote:\nI want a refund.\nI suggest we say no.', 'Bonjour,\nLe client a écrit :\n« Je veux être remboursé. »\nJe propose de refuser.']);
});

test('a "-- " line and all after it is cut', () => {
  assert.deepEqual(bubbles([
    mail(1, { subject: 'A', body_text: 'À demain.\n-- \nLeo Bianchini\nDirecteur' }),
    mail(2, { subject: 'B', body_text: 'See you.\n--\nLeo Bianchini\nDirector' }),
    mail(3, { subject: 'C', body_text: 'Score: 3 -- 2\n--- not a signature' }),
  ]), ['Score: 3 -- 2\n--- not a signature', 'See you.', 'À demain.']);
});

test('my accounts\' signatures are cut wherever they appear, matched exactly', () => {
  // A signature is given as its text; in the email its formatting may show as marks.
  const context = { ...ME, signatures: ['David Fort\n\nO2GAME |  o2game.com', '', 'Best regards,\nDavid'] };
  assert.deepEqual(bubbles([
    mail(1, { subject: 'A', from_email: 'me@work.com', body_text: 'Bonjour Leo,\n\nC\'est validé.\n\nDavid Fort\n**O2GAME** | [o2game.com][1]' }),
    mail(2, { subject: 'B', from_email: 'me@work.com', body_text: 'Hi Leo,\n\nApproved.\n\nBest regards,\nDavid\n\nP.S. Lunch?' }),
    mail(3, { subject: 'C', from_email: 'me@work.com', body_text: 'Best regards,\nDavid Fort' }),
  ], context), ['Best regards,\nDavid Fort', 'Hi Leo,\n\nApproved.\n\nP.S. Lunch?', 'Bonjour Leo,\n\nC\'est validé.']);
});

test('the same last lines at the end of 2+ emails from one sender are cut; a cut leaving nothing keeps all', () => {
  const leoSignature = '\n\nLeo Bianchini\nDirecteur financier\nTél. 01 23 45 67 89';
  const bobSignature = '\n\nBob\nSent from my phone';
  assert.deepEqual(bubbles([
    mail(1, { subject: 'A', from_email: 'leo@d.com', body_text: `Bonjour,\n\nVoici le budget.${leoSignature}` }),
    mail(2, { subject: 'B', from_email: 'leo@d.com', body_text: `Merci !\n\nÀ lundi.${leoSignature}` }),
    mail(3, { subject: 'C', from_email: 'ana@d.com', body_text: 'OK.\n\nTél. 01 23 45 67 89' }),
    mail(4, { subject: 'D', from_email: 'bob@e.com', body_text: `Sure.${bobSignature}` }),
    mail(5, { subject: 'E', from_email: 'bob@e.com', body_text: `Done.${bobSignature}` }),
    mail(6, { subject: 'F', from_email: 'bob@e.com', body_text: 'Bob\nSent from my phone' }),
  ]), ['Bob\nSent from my phone', 'Done.', 'Sure.', 'OK.\n\nTél. 01 23 45 67 89', 'Merci !\n\nÀ lundi.', 'Bonjour,\n\nVoici le budget.']);
});

test('bold, italic, lists and links stay; a link\'s address, noted at the very end, comes back beside it', () => {
  assert.deepEqual(bubbles([
    mail(1, { subject: 'A', body_text: 'Voir **le budget** et *la note* : [le lien][1]\n\n* point un\n* point deux\n\nLe lun. 26 oct. 2026 à 09:12, Leo <leo@d.com> a écrit :\n> ancien [x][2]\n\n[1]: https://a.com/b\n[2]: https://old.com' }),
    mail(2, { subject: 'B', body_text: 'See **the budget**: [here][1]\n\n-- \nLeo | [site][2]\n\n[1]: https://a.com/en\n[2]: https://leo.com' }),
  ]), ['See **the budget**: [here](https://a.com/en)', 'Voir **le budget** et *la note* : [le lien](https://a.com/b)\n\n* point un\n* point deux']);
});

test('each message says which folder it is in, so a search result lands on it (UIDs repeat across folders)', () => {
  const list = buildDiscussions({
    inbox: [mail(5, { message_id: '<a@x>', subject: 'Budget' }), mail(6, { subject: 'Lunch' })],
    sent: [mail(5, { from_email: 'me@work.com', references: '<a@x>', date_ts: 70 })],
    trash: [mail(8, { references: '<a@x>' })],
  }, ME);
  const hit = { folder: 'sent', uid: 5 };
  const landing = list.filter(d => d.items.some(i => i.kind === 'message' && i.folder === hit.folder && i.email.uid === hit.uid));
  assert.deepEqual(landing.map(d => d.subject), ['Budget']);
  assert.deepEqual(landing[0].items.filter(i => i.kind === 'message').map(i => i.folder), ['inbox', 'sent', 'trash']);
});

test('a note says who joined or left when the recipients change, before that message', () => {
  const [d] = buildDiscussions({
    inbox: [
      mail(1, { message_id: '<a@x>', from_name: 'Leo B', from_email: 'leo@d.com', to: 'me@work.com, Paul <paul@d.com>', date_ts: at(26, 9) }),
      mail(3, { references: '<a@x>', from_name: 'Leo B', from_email: 'leo@d.com', to: 'me@work.com', cc: 'Damien <damien@o.com>', date_ts: at(26, 11) }),
      mail(4, { references: '<a@x>', from_name: 'Damien', from_email: 'damien@o.com', to: 'LEO@d.com', cc: 'me@work.com', date_ts: at(26, 12) }),
      mail(5, { references: '<a@x>', from_name: 'Zoe', from_email: 'zoe@z.com', to: 'leo@d.com, damien@o.com', cc: 'me@work.com', date_ts: at(26, 13) }),
    ],
    sent: [mail(2, { references: '<a@x>', from_email: 'me@home.org', to: 'leo@d.com', cc: 'paul@d.com, ann@x.com', date_ts: at(26, 10) })],
  }, ME);
  const shape = d.items.map(i => (i.kind === 'people'
    ? `${i.mine ? 'me' : i.by.short} +${i.added.map(p => p.short + (p.cc ? '(cc)' : '')).join(',')} -${i.removed.map(p => p.short).join(',')}`
    : i.kind === 'message' ? `${i.email.uid}${i.first ? 'F' : ''}` : i.kind));
  // Zoe was a hidden copy: she is not "added" by her own reply.
  assert.deepEqual(shape, ['day', '1F', 'me +ann(cc) -', '2F', 'Leo +Damien(cc) -Paul,ann', '3F', '4F', '5F']);
});

test('a note at the top says when earlier messages of the discussion are not on this computer', () => {
  const list = buildDiscussions({
    inbox: [
      mail(1, { message_id: '<a@x>', subject: 'Complete' }),
      mail(2, { references: '<a@x>', subject: 'Re: Complete' }),
      mail(3, { message_id: '<c@x>', references: '<gone@x> <c0@x>', subject: 'Re: Partial' }),
      mail(4, { references: '<gone@x> <c@x>', subject: 'Re: Partial' }),
    ],
  }, ME);
  assert.deepEqual(list.map(d => [d.subject, d.items[0].kind]), [['Re: Partial', 'earlier'], ['Complete', 'day']]);
  assert.equal(list[0].items.filter(i => i.kind === 'earlier').length, 1);
});

test('a draft of the discussion shows as an unsent message at the bottom', () => {
  const list = buildDiscussions({
    inbox: [mail(1, { message_id: '<a@x>', subject: 'Budget', from_name: 'Leo', from_email: 'leo@d.com', body_text: 'Numbers?', date_ts: at(26, 9) })],
    sent: [mail(2, { references: '<a@x>', from_email: 'me@work.com', body_text: 'Soon.', date_ts: at(27, 9) })],
    drafts: [
      mail(3, { references: '<a@x>', from_email: 'me@work.com', body_text: ' Here they are \n', date_ts: at(28, 10), is_read: false }),
      mail(4, { subject: 'Draft alone', from_email: 'me@work.com', date_ts: at(28, 11) }),
    ],
  }, ME);
  assert.deepEqual(list.map(d => d.subject), ['Budget'], 'a lone draft is no discussion');
  const [d] = list;
  assert.deepEqual(d.items.map(i => i.kind), ['day', 'message', 'day', 'message', 'draft']);
  assert.deepEqual(d.items.at(-1), { kind: 'draft', email: d.items.at(-1).email, text: 'Here they are', date_ts: at(28, 10) });
  assert.equal(d.items.at(-1).email.uid, 3);
  assert.equal(d.items.at(-2).last, true);
  assert.deepEqual([d.date_ts, d.preview.text], [at(27, 9), 'Soon.'], 'the list line ignores the draft');
});

test('the details sheet lists everyone in the discussion with their full address, me included', () => {
  const [d] = buildDiscussions({
    inbox: [mail(1, { message_id: '<a@x>', from_name: 'Leo B', from_email: 'leo@d.com', to: 'Me <ME@work.com>', cc: 'paul@d.com' })],
    sent: [mail(2, { references: '<a@x>', from_name: 'Me', from_email: 'me@work.com', to: 'Leo B <leo@d.com>' })],
  }, ME);
  assert.deepEqual(d.info.people.map(p => [p.name, p.address, !!p.mine]), [
    ['Leo B', 'leo@d.com', false],
    ['Me', 'ME@work.com', true],
    ['', 'paul@d.com', false],
  ]);
});

test('the details sheet lists the subjects used, and how many emails each account keeps in each folder', () => {
  const second = 2 ** 32;   // an email id carries its account above bit 32
  const [d] = buildDiscussions({
    inbox: [
      mail(1, { message_id: '<a@x>', subject: 'Caméra LDV - REX' }),
      mail(3, { references: '<a@x>', subject: 'RE: Caméra LDV - REX' }),
      mail(second + 4, { references: '<a@x>', subject: 'RE: Caméra LDV - REX', date_ts: 40 }),
    ],
    sent: [mail(2, { references: '<a@x>', from_email: 'me@work.com', subject: 'RE: Caméra LDV - REX' })],
    trash: [mail(5, { references: '<a@x>', subject: 'Re: Caméra LDV - REX' })],
    drafts: [mail(6, { references: '<a@x>', from_email: 'me@work.com', subject: 'RE: Caméra LDV - REX' })],
  }, ME);
  assert.deepEqual(d.info.subjects, ['Caméra LDV - REX', 'RE: Caméra LDV - REX', 'Re: Caméra LDV - REX']);
  assert.deepEqual(d.info.where, [
    { account: 0, folders: [{ folder: 'inbox', count: 2 }, { folder: 'sent', count: 1 }, { folder: 'trash', count: 1 }, { folder: 'drafts', count: 1 }] },
    { account: 1, folders: [{ folder: 'inbox', count: 1 }] },
  ]);
});

test('the details sheet gives the AI summary of the latest summarised email, and every image and file shared', () => {
  const [d] = buildDiscussions({
    inbox: [
      mail(1, { message_id: '<a@x>', summary: 'Leo asks for the camera report.', body_html: '<p>Look</p><img src="data:image/png;base64,AAA"><p>ok</p>' }),
      mail(2, { references: '<a@x>', summary: 'Paul sends the report.', attachments: [{ name: 'rex.pdf', size: 51000, mime: 'application/pdf' }] }),
      mail(3, { references: '<a@x>', summary: '' }),
    ],
  }, ME);
  assert.equal(d.info.summary, 'Paul sends the report.');
  assert.deepEqual(d.info.files.map(f => [f.kind, f.src || f.name, f.email.uid]), [
    ['image', 'data:image/png;base64,AAA', 1],
    ['attachment', 'rex.pdf', 2],
  ]);
});

test('a received email the board took a task from carries the task, once, after its last piece', () => {
  const task = { id: 7, title: 'Send the REX', status: 'todo', source_uid: 1 };
  const [d] = buildDiscussions({
    inbox: [mail(1, { message_id: '<a@x>', body_text: 'Can you?', attachments: [{ name: 'a.pdf', size: 1, mime: 'application/pdf' }] })],
    sent: [mail(1, { references: '<a@x>', from_email: 'me@work.com', body_text: 'Sure.', date_ts: 20 })],   // same UID, other folder
  }, { ...ME, tasks: [task, { id: 8, title: 'Hand-made', status: 'todo', source_uid: null }] });
  assert.deepEqual(d.items.filter(i => i.email).map(i => [i.folder, i.kind, i.task?.id ?? null]), [
    ['inbox', 'message', null],
    ['inbox', 'attachment', 7],
    ['sent', 'message', null],
  ]);
});

test('the list search keeps discussions whose people, subjects or message text hold every word', () => {
  const list = buildDiscussions({
    inbox: [
      mail(1, { message_id: '<a@x>', from_name: 'Léo Bianchini', from_email: 'leo@studio.fr', subject: 'Budget', body_text: 'Le devis est prêt.' }),
      mail(2, { from_name: 'Ann', from_email: 'ann@x.com', subject: 'Lunch', body_text: 'Pizza?' }),
      mail(4, { subject: 'News', html_only: true, body_html: '<table><tr><td>Big <b>promo</b></td></tr></table>' }),
    ],
    sent: [mail(3, { references: '<a@x>', from_email: 'me@work.com', subject: 'Re: Budget 2027', body_text: 'Merci' })],
  }, ME);
  const found = q => filterDiscussions(list, q).map(d => d.subject);
  assert.deepEqual(found('LEO'), ['Budget'], 'a name, accents and case ignored');
  assert.deepEqual(found('studio.fr'), ['Budget'], 'an address');
  assert.deepEqual(found('2027'), ['Budget'], 'any subject of the discussion');
  assert.deepEqual(found('pizza'), ['Lunch'], 'message text');
  assert.deepEqual(found('promo'), ['News'], 'the text of an email shown as a page');
  assert.deepEqual(found('pret merci'), ['Budget'], 'words may sit in different messages');
  assert.deepEqual(found('pizza merci'), [], 'every word must match');
  assert.deepEqual(found('me@work.com'), [], 'not my own address: it is on every discussion');
});

test('an empty list search shows the whole list', () => {
  const list = buildDiscussions({ inbox: [mail(1), mail(2)] }, ME);
  assert.equal(filterDiscussions(list, '   '), list);
  assert.equal(filterDiscussions(list, ''), list);
});
