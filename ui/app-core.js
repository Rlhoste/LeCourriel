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
  // `folders` holds the cached emails per folder ({ inbox, sent, trash, spam }, every account);
  // `context` is { me: own addresses of every account, now: ms }.
  // Returns the listed discussions, newest first. Each one is a list line plus its `items`,
  // oldest first; an item's `kind` says what it is ('day' separator or 'message').
  function buildDiscussions(folders, context = {}) {
    const listed = new Set([...(folders?.inbox || []), ...(folders?.sent || [])]);
    const all = [...listed, ...(folders?.trash || []), ...(folders?.spam || [])];
    return groupThreads(sortEmails(all))
      .map(({ lead, replies }) => discussion([lead, ...replies].reverse(), context))
      .filter(d => d.items.some(i => i.email && listed.has(i.email)))
      .sort((a, b) => b.date_ts - a.date_ts);
  }

  function discussion(emails, context) {
    const own = new Set((context.me || []).map(a => String(a).trim().toLowerCase()));
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
      items: discussionItems(emails, isMine, context.now ?? Date.now()),
    };
  }

  // Oldest first: a 'day' separator whenever the day changes, then each 'message'.
  // Messages in a row from the same person on the same day stack: `first` / `last` of the run.
  function discussionItems(emails, isMine, now) {
    const items = [];
    const today = startOfDay(now);
    let prev = null;
    for (const email of emails) {
      const ts = Number(email.date_ts) || 0;
      const sender = person(email.from_name, String(email.from_email || '').trim());
      const day = startOfDay(ts * 1000);
      const sameRun = prev && prev.day === day && prev.item.sender.address.toLowerCase() === sender.address.toLowerCase();
      if (!prev || prev.day !== day) {
        const daysAgo = Math.round((today - day) / 86400e3);
        items.push({ kind: 'day', day: daysAgo === 0 ? 'today' : daysAgo > 0 && daysAgo < 7 ? 'weekday' : 'date', date_ts: ts });
      }
      if (prev && !sameRun) prev.item.last = true;
      const parts = emailParts(email).map(part => ({ ...part, email, mine: isMine(email), sender, date_ts: ts, first: false, last: false }));
      parts[0].first = !sameRun;
      items.push(...parts);
      prev = { day, item: parts[parts.length - 1] };
    }
    if (prev) prev.item.last = true;
    return items;
  }

  // What one email shows as: its text, split by each image of its body ('image' items,
  // the cid: images the backend put in the HTML as data: URLs), then an 'attachment' item per file.
  // An email laid out as a page (newsletter, invoice) is one 'page' card instead of its text and images.
  const BODY_IMAGE = /<img\b[^>]*?\bsrc\s*=\s*(["'])(data:image\/[^"']*)\1[^>]*>/gi;
  function emailParts(email) {
    const html = String(email.body_html || '');
    let parts = [];
    let from = 0;
    if (html && (email.html_only || mostlyTables(html))) parts.push({ kind: 'page' });
    else for (const m of html.matchAll(BODY_IMAGE)) {
      parts.push({ kind: 'message', text: htmlText(html.slice(from, m.index)) }, { kind: 'image', src: m[2] });
      from = m.index + m[0].length;
    }
    if (parts[0]?.kind !== 'page') parts.push({ kind: 'message', text: parts.length ? htmlText(html.slice(from)) : String(email.body_text || '').trim() });
    for (const a of email.attachments || []) parts.push({ kind: 'attachment', name: a.name, size: a.size, mime: a.mime });
    if (parts.length > 1) parts = parts.filter(p => p.kind !== 'message' || p.text);
    return parts;
  }

  // Most of the text sits in tables: a layout, not a written message (a signature table is not enough).
  function mostlyTables(html) {
    const letters = piece => htmlText(piece).replace(/\s/g, '').length;
    let inside = 0, outside = 0, depth = 0, from = 0;
    for (const m of html.matchAll(/<(\/?)table\b[^>]*>/gi)) {
      if (depth > 0) inside += letters(html.slice(from, m.index)); else outside += letters(html.slice(from, m.index));
      depth = Math.max(0, depth + (m[1] ? -1 : 1));
      from = m.index + m[0].length;
    }
    if (depth > 0) inside += letters(html.slice(from)); else outside += letters(html.slice(from));
    return inside > outside;
  }

  // Plain text of a piece of HTML: one line per block, entities decoded.
  const ENTITIES = { nbsp: ' ', amp: '&', lt: '<', gt: '>', quot: '"', apos: "'" };
  function htmlText(html) {
    return html
      .replace(/\s+/g, ' ')
      .replace(/<(head|style|script|title)\b[\s\S]*?<\/\1\s*>/gi, '')
      .replace(/<!--[\s\S]*?-->/g, '')
      .replace(/<br\b[^>]*>|<\/?(p|div|tr|li|ul|ol|table|blockquote|h[1-6])\b[^>]*>/gi, '\n')
      .replace(/<[^>]*>/g, '')
      .replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (m, e) => {
        if (e[0] !== '#') return ENTITIES[e.toLowerCase()] ?? m;
        const code = /x/i.test(e[1]) ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10);
        return code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : m;
      })
      .split('\n').map(line => line.replace(/[ \t ]+/g, ' ').trim()).join('\n')
      .replace(/\n{3,}/g, '\n\n')
      .trim();
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
    removeEmailFromState,
    autoUpdatesEnabled,
    saveAutoUpdates,
    updateProgressPercent,
    needsAccountSetup,
    attentionEmails,
    activeTasks,
  };
});
