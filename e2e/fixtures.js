'use strict';

// The mailbox a new user brings, and the fixed answers of the fake AI for it.
// Dates are relative to today so the deadline always lands in the Overview's week strip.

function isoDate(d) {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

function daysFromNow(n) {
  const d = new Date();
  d.setDate(d.getDate() + n);
  return d;
}

let counter = 0;
function eml({ from, subject, body, hoursAgo = 1 }) {
  const date = new Date(Date.now() - hoursAgo * 3600 * 1000).toUTCString().replace('GMT', '+0000');
  counter++;
  return [
    `From: ${from}`,
    'To: Test User <user@example.test>',
    `Subject: ${subject}`,
    `Date: ${date}`,
    `Message-ID: <e2e-${counter}-${Date.now()}@example.test>`,
    'MIME-Version: 1.0',
    'Content-Type: text/plain; charset=utf-8',
    '',
    body,
    '',
  ].join('\r\n');
}

const MEETING_LINK = 'https://zoom.us/j/123456789';
const DEADLINE = isoDate(daysFromNow(2));

const SUBJECTS = {
  urgent: 'Production server is down',
  task: 'Quarterly figures needed',
  meeting: 'Project kickoff call',
  newsletter: 'Weekly newsletter',
  phishing: 'Verify your account now',
  readUrgent: 'Contract must be signed today',
  late: 'Client escalation: refund blocked',
};

const TASK_TEXT = 'Send the quarterly figures';

/** Seed the inbox of a FakeImap. */
function seedInbox(imap) {
  imap.append('INBOX', eml({ from: 'Alice Ops <alice@example.test>', subject: SUBJECTS.urgent,
    body: 'The production server is down since 8am. Please call me as soon as you read this.', hoursAgo: 1 }));
  imap.append('INBOX', eml({ from: 'Bob Finance <bob@example.test>', subject: SUBJECTS.task,
    body: `Hi, can you send me the quarterly figures by ${DEADLINE}? Thanks, Bob`, hoursAgo: 2 }));
  imap.append('INBOX', eml({ from: 'Carol PM <carol@example.test>', subject: SUBJECTS.meeting,
    body: `Kickoff tomorrow at 10:00. Join: ${MEETING_LINK}`, hoursAgo: 3 }));
  imap.append('INBOX', eml({ from: 'News <news@example.test>', subject: SUBJECTS.newsletter,
    body: 'This week in tech: nothing urgent.', hoursAgo: 4 }));
  imap.append('INBOX', eml({ from: 'Security <secure@examp1e.test>', subject: SUBJECTS.phishing,
    body: 'Click http://examp1e.test/login to keep your account.', hoursAgo: 5 }));
  // Already read on the first computer, but still urgent.
  imap.append('INBOX', eml({ from: 'Dan Legal <dan@example.test>', subject: SUBJECTS.readUrgent,
    body: 'Please sign the contract today, the offer expires tonight.', hoursAgo: 6 }), ['\\Seen']);
}

/** The email delivered while the app is open. */
function lateEmail() {
  return eml({ from: 'Eve Sales <eve@example.test>', subject: SUBJECTS.late,
    body: 'Our biggest client cannot get their refund. Need your decision within the hour.', hoursAgo: 0 });
}

const answer = (priority, summary, extra = {}) => ({
  priority, summary, project: null, deadline: null, task: null, meeting_link: null, ...extra,
});

const AI_RULES = [
  { match: /Production server/, answer: answer('urgent', 'The production server is down and Alice needs a call.') },
  { match: /Quarterly figures/, answer: answer('task', 'Bob asks for the quarterly figures.', { task: TASK_TEXT, deadline: DEADLINE }) },
  { match: /kickoff call/, answer: answer('high_priority', 'Kickoff call with Carol tomorrow.', { meeting_link: MEETING_LINK }) },
  { match: /newsletter/, answer: answer('information', 'Weekly tech newsletter.') },
  { match: /Verify your account/, answer: answer('threat', 'Phishing attempt asking for a login.') },
  { match: /Contract must be signed/, answer: answer('urgent', 'Dan needs the contract signed today.') },
  { match: /Client escalation/, answer: answer('urgent', 'A client refund is blocked; decision needed now.') },
];

module.exports = { seedInbox, lateEmail, AI_RULES, SUBJECTS, TASK_TEXT, DEADLINE, MEETING_LINK };
