(function exposeCourrielCore(root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  if (root) root.CourrielCore = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function createCourrielCore() {
  'use strict';

  const PRIORITY_ORDER = Object.freeze({
    urgent: 0,
    high_priority: 1,
    task: 2,
    other_priority: 3,
    threat: 4,
    information: 5,
  });
  const SORT_KEYS = new Set(['date', 'sender', 'subject', 'read', 'priority', 'project']);

  function normalizedSort(sort) {
    return {
      key: SORT_KEYS.has(sort?.key) ? sort.key : 'date',
      ascending: typeof sort?.ascending === 'boolean' ? sort.ascending : false,
    };
  }

  function sortValue(email, key) {
    switch (key) {
      case 'sender': return email.from_name || email.from_email || '';
      case 'subject': return email.subject || '';
      case 'read': return email.is_read ? 1 : 0;
      case 'priority': return PRIORITY_ORDER[email.priority] ?? 99;
      case 'project': return email.project || '';
      default: return Number(email.date_ts) || 0;
    }
  }

  function sortEmails(emails, requestedSort, locale = 'en') {
    const sort = normalizedSort(requestedSort);
    return [...(emails || [])].sort((a, b) => {
      const av = sortValue(a, sort.key);
      const bv = sortValue(b, sort.key);
      const primary = typeof av === 'number'
        ? av - bv
        : String(av).localeCompare(String(bv), locale, { sensitivity: 'base' });
      const fallback = (Number(b.date_ts) || 0) - (Number(a.date_ts) || 0)
        || (Number(b.uid) || 0) - (Number(a.uid) || 0);
      return primary ? (sort.ascending ? primary : -primary) : fallback;
    });
  }

  // "Re: Fwd: TR: Subject" -> "subject", so replies can be matched to the email they answer.
  const REPLY_PREFIX = /^\s*((re|fwd?|tr|aw|wg|sv|vs|rv|res|enc|r)(\[\d+\])?\s*:\s*)+/i;
  function threadSubject(subject) {
    return String(subject || '').replace(REPLY_PREFIX, '').replace(/\s+/g, ' ').trim().toLowerCase();
  }

  // Groups an already sorted list into discussions. Emails are linked by Message-ID and
  // In-Reply-To / References; when those are missing, a reply ("Re: …") joins the email
  // with the same subject. Each discussion keeps the place of its first email in the sorted
  // list; that email leads it and the others follow, newest first.
  function groupThreads(emails) {
    const list = emails || [];
    const parent = list.map((_, i) => i);
    const find = i => (parent[i] === i ? i : (parent[i] = find(parent[i])));
    const union = (a, b) => { const ra = find(a), rb = find(b); if (ra !== rb) parent[Math.max(ra, rb)] = Math.min(ra, rb); };

    const byId = new Map();
    list.forEach((e, i) => { if (e.message_id) byId.set(e.message_id.trim(), i); });
    const subjectFirst = new Map();
    list.forEach((e, i) => {
      for (const id of String(e.references || '').match(/<[^>]+>/g) || []) {
        if (byId.has(id)) union(i, byId.get(id));
      }
      const key = threadSubject(e.subject);
      if (!key) return;
      if (!subjectFirst.has(key)) subjectFirst.set(key, []);
      subjectFirst.get(key).push(i);
    });
    for (const members of subjectFirst.values()) {
      if (members.length < 2 || !members.some(i => REPLY_PREFIX.test(list[i].subject || ''))) continue;
      for (const i of members) union(i, members[0]);
    }

    const groups = new Map();
    list.forEach((e, i) => {
      const root = find(i);
      if (!groups.has(root)) groups.set(root, []);
      groups.get(root).push(e);
    });
    return [...groups.values()].map(([lead, ...rest]) => ({
      lead,
      replies: rest.sort((a, b) => (Number(b.date_ts) || 0) - (Number(a.date_ts) || 0)),
    }));
  }

  // Discussion mode: what the discussion list and each opened discussion show.
  // `folders` holds the cached emails per folder ({ inbox, sent, trash, spam, drafts }, every account);
  // `context` is { me: own address of each account, in account order, now: ms, signatures: each account's signature as text }.
  // Returns the listed discussions, newest first. Each one is a list line plus its `items`,
  // oldest first; an item's `kind` says what it is: 'earlier' (older messages not on this computer,
  // at the top), 'day' separator, 'people' (recipients changed), 'message', 'draft' (unsent, at the bottom).
  function buildDiscussions(folders, context = {}) {
    const listed = new Set([...(folders?.inbox || []), ...(folders?.sent || [])]);
    // Each folder numbers its UIDs separately: an item keeps its folder so a search hit finds it.
    const folderOf = new Map();
    for (const name of ['inbox', 'sent', 'trash', 'spam']) for (const e of folders?.[name] || []) folderOf.set(e, name);
    const drafts = new Set(folders?.drafts || []);
    const texts = newTexts([...folderOf.keys()], context.signatures || []);
    return groupThreads(sortEmails([...folderOf.keys(), ...drafts]))
      .map(({ lead, replies }) => [lead, ...replies].reverse())
      .filter(emails => emails.some(e => listed.has(e)))
      .map(emails => discussion(emails.filter(e => !drafts.has(e)), emails.filter(e => drafts.has(e)), context, folderOf, texts))
      .sort((a, b) => b.date_ts - a.date_ts);
  }

  function discussion(emails, drafts, context, folderOf, texts) {
    const own = new Set((context.me || []).map(a => String(a || '').trim().toLowerCase()).filter(Boolean));
    const isMine = email => own.has(String(email.from_email || '').trim().toLowerCase());
    const last = emails[emails.length - 1];
    const unread = emails.filter(e => !e.is_read && !isMine(e));
    const people = new Map();
    const meet = (name, address) => {
      const key = String(address || '').trim().toLowerCase();
      if (key && !own.has(key) && !people.has(key)) people.set(key, person(name, address.trim()));
    };
    for (const e of emails) {
      meet(e.from_name, e.from_email);
      for (const a of [...splitAddresses(e.to), ...splitAddresses(e.cc)]) meet(a.name, a.address);
    }
    const others = [...people.values()];
    return {
      id: String(emails[0].message_id || emails[0].uid),
      people: others,
      title: others.slice(0, 2).map(p => p.short).join(', ') + (others.length > 2 ? ` +${others.length - 2}` : ''),
      subject: emails[0].subject || '',
      preview: { mine: isMine(last), text: String(last.body_text || '').replace(/\s+/g, ' ').trim().slice(0, 200) },
      date_ts: Number(last.date_ts) || 0,
      unread: unread.length,
      priority: unread.reduce((top, e) => (top && PRIORITY_ORDER[top] <= (PRIORITY_ORDER[e.priority] ?? 99) ? top : e.priority), null) ?? null,
      items: [
        ...(missesEarlier(emails) ? [{ kind: 'earlier' }] : []),
        ...discussionItems(emails, isMine, people, context.now ?? Date.now(), folderOf, texts),
        ...drafts.map(email => ({ kind: 'draft', email, text: String(email.body_text || '').trim(), date_ts: Number(email.date_ts) || 0 })),
      ],
      reply: replyAll(last, context.me || []),
    };
  }

  // True when an email refers to one that is not among the discussion's emails (not on this computer).
  function missesEarlier(emails) {
    const ids = new Set(emails.map(e => String(e.message_id || '').trim()).filter(Boolean));
    return emails.some(e => (String(e.references || '').match(/<[^>]+>/g) || []).some(id => !ids.has(id)));
  }

  // The reply bar answers everyone on the latest message (`email`): its sender (or Reply-To)
  // and its To in `to`, its Cc in `cc`, never any of my own addresses. `from` is the index in
  // `me` of the account that received it (or sent it, when the latest message is mine).
  function replyAll(email, me) {
    const mine = me.map(a => String(a || '').trim().toLowerCase());
    const seen = new Set(mine.filter(Boolean));
    const pick = list => list.filter(a => {
      const key = a.address.toLowerCase();
      if (!key || seen.has(key)) return false;
      seen.add(key);
      return true;
    }).map(a => person(a.name, a.address));
    const account = address => (address ? mine.indexOf(address.trim().toLowerCase()) : -1);
    const fromMe = account(email.from_email);
    const sender = email.reply_to ? splitAddresses(email.reply_to) : [{ name: email.from_name || '', address: String(email.from_email || '').trim() }];
    const received = [...splitAddresses(email.to), ...splitAddresses(email.cc)].map(a => account(a.address)).find(i => i >= 0);
    const subject = String(email.subject || '');
    const to = pick(fromMe >= 0 ? splitAddresses(email.to) : [...sender, ...splitAddresses(email.to)]);
    const cc = pick(splitAddresses(email.cc));
    bubbleLabels([...to, ...cc]);
    return {
      email,
      from: fromMe >= 0 ? fromMe : received ?? 0,
      to,
      cc,
      subject: /^re\s*:/i.test(subject) ? subject : `Re: ${subject}`,
    };
  }

  // Sets each person's bubble `label`: first name and last-name initial ("David F."), the last
  // name growing until two labels differ ("Leo Bi.", "Leo Bo."); no name -> the part before @.
  function bubbleLabels(people) {
    const parts = people.map(p => {
      const words = /@/.test(p.name) ? [] : String(p.name || '').split(/\s+/).filter(Boolean);
      return { p, first: words[0], last: words.length > 1 ? words[words.length - 1] : '', n: 1 };
    });
    const label = x => (!x.first ? x.p.address.split('@')[0]
      : !x.last ? x.first
        : x.n >= x.last.length ? `${x.first} ${x.last}` : `${x.first} ${x.last.slice(0, x.n)}.`);
    for (let grew = true; grew;) {
      grew = false;
      const count = new Map();
      for (const x of parts) count.set(label(x), (count.get(label(x)) || 0) + 1);
      for (const x of parts) {
        if (count.get(label(x)) > 1 && x.last && x.n < x.last.length) { x.n++; grew = true; }
      }
    }
    for (const x of parts) x.p.label = label(x);
    return people;
  }

  // Oldest first: a 'day' separator whenever the day changes, then each 'message'.
  // Messages in a row from the same person on the same day stack: `first` / `last` of the run.
  // A 'people' note before a message says whom its sender `added` (`cc` when only in Cc) or `removed`.
  // A message's `text` is only what its sender newly wrote (see newTexts).
  function discussionItems(emails, isMine, people, now, folderOf, texts) {
    const items = [];
    const today = startOfDay(now);
    let prev = null;
    let seen = null;
    for (const email of emails) {
      const ts = Number(email.date_ts) || 0;
      const sender = person(email.from_name, String(email.from_email || '').trim());
      const day = startOfDay(ts * 1000);
      if (!prev || prev.day !== day) {
        const daysAgo = Math.round((today - day) / 86400e3);
        items.push({ kind: 'day', day: daysAgo === 0 ? 'today' : daysAgo > 0 && daysAgo < 7 ? 'weekday' : 'date', date_ts: ts });
      }
      const here = recipients(email, people);
      let note = null;
      if (here && seen) {
        const added = [...here].filter(([key]) => !seen.has(key) && key !== sender.address.toLowerCase()).map(([, p]) => p);
        const removed = [...seen].filter(([key]) => !here.has(key)).map(([, p]) => p);
        if (added.length || removed.length) items.push(note = { kind: 'people', by: sender, mine: isMine(email), added, removed });
      }
      if (here) seen = here;
      const sameRun = prev && !note && prev.day === day && prev.item.sender.address.toLowerCase() === sender.address.toLowerCase();
      if (prev && !sameRun) prev.item.last = true;
      const item = {
        kind: 'message', email, folder: folderOf.get(email), mine: isMine(email), sender, date_ts: ts,
        text: texts.get(email), first: !sameRun, last: false,
      };
      items.push(item);
      prev = { day, item };
    }
    if (prev) prev.item.last = true;
    return items;
  }

  // Only what the person newly wrote, cut by rules (no AI): the quoted history goes.
  // When a cut would leave nothing, the message is kept whole.
  const QUOTE_FROM = /^\s*(?:De|From)\s*:/i;
  const QUOTE_SUBJECT = /^\s*(?:Objet|Subject)\s*:/i;
  const QUOTE_INTRO = /^\s*(?:Le|On)\s.*(?:a\s+écrit|wrote)\s*:\s*$/i;   // may wrap onto a 2nd line
  const RULE_LINE = /^\s*(?:[-_─=]{3,}|-+\s*(?:Original Message|Message d'origine)\s*-+)?\s*$/i;
  function newText(email, signatures = []) {
    // A link shows as "[text][1]" with its address on a "[1]: url" line at the very end
    // (after any quote): put the address back beside it, as "[text](url)".
    const links = new Map();
    const body = String(email.body_text || '').replace(/\r\n?/g, '\n').split('\n')
      .filter(line => { const m = /^\[(\d+)\]:\s*(\S+)\s*$/.exec(line); if (m) links.set(m[1], m[2]); return !m; })
      .join('\n');
    const whole = body.replace(/\[([^\]\n]*)\]\[(\d+)\]/g, (all, text, n) => (links.has(n) ? `[${text}](${links.get(n)})` : all)).trim();
    const lines = whole.split('\n');
    const kept = [];
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      if (QUOTE_FROM.test(line) && lines.slice(i + 1, i + 6).some(l => QUOTE_SUBJECT.test(l))) break;
      if (QUOTE_INTRO.test(line) || QUOTE_INTRO.test(`${line} ${lines[i + 1]}`)) break;
      if (/^--\s?$/.test(line)) break;   // signature delimiter "-- " (the space may be trimmed)
      if (!line.trimStart().startsWith('>')) kept.push(line);
    }
    for (const signature of signatures) cutBlock(kept, signature.split('\n'));
    while (kept.length && RULE_LINE.test(kept[kept.length - 1])) kept.pop();
    return kept.join('\n').trim() || whole;
  }

  // Each email's new text, then the last lines one sender ends 2+ of their emails with
  // (a signature we were not told about) are cut: the longest such block, up to MAX_TAIL lines.
  const MAX_TAIL = 15;
  function newTexts(emails, signatures) {
    const texts = new Map(emails.map(e => [e, newText(e, signatures)]));
    const tails = new Map();   // sender + last k lines → distinct emails ending so
    const tailKeys = email => {
      const lines = texts.get(email).split('\n').map(plainLine).filter(Boolean);
      const sender = String(email.from_email || '').trim().toLowerCase();
      return lines.slice(-MAX_TAIL).map((_, k, last) => `${sender}\n${last.slice(last.length - k - 1).join('\n')}`);
    };
    for (const email of emails) {
      for (const key of tailKeys(email)) {
        if (!tails.has(key)) tails.set(key, new Set());
        tails.get(key).add(String(email.message_id || email.uid));
      }
    }
    for (const email of emails) {
      const k = tailKeys(email).findLastIndex(key => tails.get(key).size >= 2) + 1;
      if (!k) continue;
      const lines = texts.get(email).split('\n');
      let cut = lines.length;
      for (let n = 0; n < k; cut--) if (plainLine(lines[cut - 1])) n++;
      texts.set(email, lines.slice(0, cut).join('\n').trim() || texts.get(email));
    }
    return texts;
  }

  // A line's words only: formatting marks (**bold**, *italic*, [link][1] / [link](url)) and spacing ignored.
  const plainLine = line => String(line)
    .replace(/\]\[\d+\]|\]\([^)]*\)/g, ']').replace(/[*[\]]/g, '').replace(/\s+/g, ' ').trim();

  // Removes from `lines` the first run whose non-empty lines are exactly `block`'s non-empty lines.
  function cutBlock(lines, block) {
    const want = block.map(plainLine).filter(Boolean);
    if (!want.length) return;
    const at = lines.map((l, i) => [plainLine(l), i]).filter(([l]) => l);
    for (let s = 0; s + want.length <= at.length; s++) {
      if (!want.every((w, k) => at[s + k][0] === w)) continue;
      let start = at[s][1];
      let end = at[s + want.length - 1][1] + 1;
      while (end < lines.length && !lines[end].trim() && (start === 0 || !lines[start - 1].trim())) end++;
      lines.splice(start, end - start);
      return;
    }
  }

  // Everyone on an email but me (sender, To, Cc), keyed by address; null when it names no recipient at all.
  // `people` holds the discussion's others as first met, so a name seen once is kept.
  function recipients(email, people) {
    const to = splitAddresses(email.to), cc = splitAddresses(email.cc);
    if (!to.length && !cc.length) return null;
    const here = new Map();
    for (const [address, inCc] of [[email.from_email, false], ...to.map(a => [a.address, false]), ...cc.map(a => [a.address, true])]) {
      const key = String(address || '').trim().toLowerCase();
      if (people.has(key) && !here.has(key)) here.set(key, { ...people.get(key), cc: inCc });
    }
    return here;
  }

  const startOfDay = ms => { const d = new Date(ms); d.setHours(0, 0, 0, 0); return d.getTime(); };

  // A person seen in a discussion: names, initials and colour for the avatar.
  const PERSON_COLOURS = ['#d9534f', '#3c9d5d', '#a05cd6', '#d68a1c', '#2a9fb8', '#c2577f', '#6d7f2a', '#5b6ee1'];
  function person(name, address) {
    const words = String(name || '').trim().split(/\s+/).filter(Boolean);
    const local = address.split('@')[0];
    const initials = words.length > 1 ? words[0][0] + words[1][0] : (words[0] || local).slice(0, 2);
    let hash = 0;
    for (const ch of address.toLowerCase()) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
    return {
      name: words.join(' '), address, short: words[0] || local, initials: initials.toUpperCase(),
      colour: PERSON_COLOURS[hash % PERSON_COLOURS.length],
    };
  }

  // "Ann <ann@x.com>, bob@x.com" -> [{ name: 'Ann', address: 'ann@x.com' }, { name: '', address: 'bob@x.com' }].
  // Commas inside quotes or <…> do not split.
  function splitAddresses(value) {
    const parts = [];
    let cur = '', inQuote = false, inAngle = false;
    for (const ch of String(value || '')) {
      if (ch === '"') inQuote = !inQuote;
      else if (ch === '<' && !inQuote) inAngle = true;
      else if (ch === '>' && !inQuote) inAngle = false;
      if (ch === ',' && !inQuote && !inAngle) { parts.push(cur); cur = ''; } else cur += ch;
    }
    parts.push(cur);
    return parts.map(s => s.trim()).filter(Boolean).map(s => {
      const m = s.match(/^(.*?)\s*<([^>]+)>$/);
      return m ? { name: m[1].trim().replace(/^"|"$/g, ''), address: m[2].trim() } : { name: '', address: s };
    });
  }

  function removeEmailFromState(state, uid, kind = null) {
    if (!state || typeof state !== 'object') return state;
    const drop = key => {
      if (Array.isArray(state[key])) state[key] = state[key].filter(item => item.uid !== uid);
    };
    if (kind === 'meeting') drop('meeting_items');
    else if (kind === 'deadline') drop('deadlines');
    else {
      const skip = new Set(['week_days', 'projects', 'sent_emails', 'drafts_emails', 'trash_emails', 'spam_emails']);
      for (const [key, value] of Object.entries(state)) {
        if (Array.isArray(value) && !skip.has(key)) drop(key);
      }
    }
    if (kind === null || kind === 'deadline') {
      for (const day of state.week_days || []) {
        for (const n of [1, 2, 3]) {
          if (day[`dl${n}_uid`] === uid) day[`dl${n}_subject`] = '';
        }
      }
    }
    return state;
  }

  function autoUpdatesEnabled(storage) {
    try { return storage.getItem('autoUpdate') !== 'false'; }
    catch (_) { return true; }
  }

  function saveAutoUpdates(storage, enabled) {
    try {
      storage.setItem('autoUpdate', String(Boolean(enabled)));
      return true;
    } catch (_) {
      return false;
    }
  }

  function updateProgressPercent(progress) {
    if (progress?.finished) return 100;
    const downloaded = Number(progress?.downloaded);
    const total = Number(progress?.total);
    if (!Number.isFinite(downloaded) || !Number.isFinite(total) || total <= 0) return null;
    return Math.max(0, Math.min(100, downloaded / total * 100));
  }

  function needsAccountSetup(settings) {
    return !Array.isArray(settings?.accounts) || settings.accounts.length === 0;
  }

  function attentionEmails(state, includeRead = false) {
    return [...(state?.urgent_emails || []), ...(state?.high_emails || [])]
      .filter(email => includeRead || !email.is_read);
  }

  function activeTasks(state) {
    return (state?.tasks || []).filter(task => task.status === 'todo' || task.status === 'in_progress');
  }

  return {
    PRIORITY_ORDER,
    normalizedSort,
    sortEmails,
    threadSubject,
    groupThreads,
    buildDiscussions,
    bubbleLabels,
    splitAddresses,
    removeEmailFromState,
    autoUpdatesEnabled,
    saveAutoUpdates,
    updateProgressPercent,
    needsAccountSetup,
    attentionEmails,
    activeTasks,
  };
});
