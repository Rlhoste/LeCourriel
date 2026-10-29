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
    [markedBefore, markedNow] = [markedNow, new Map()];   // HTML already turned to text by the last build is reused
    const threads = groupThreads(sortEmails([...folderOf.keys(), ...drafts])).map(({ lead, replies }) => [lead, ...replies].reverse());
    const texts = newTexts([...folderOf.keys()], context.signatures || [], threads);
    // A task taken from an email names it by its inbox id (other folders number theirs apart).
    const tasks = new Map((context.tasks || []).filter(t => t.source_uid != null).map(t => [t.source_uid, t]));
    const taskOf = email => (folderOf.get(email) === 'inbox' ? tasks.get(email.uid) : undefined);
    return threads
      .filter(emails => emails.some(e => listed.has(e)))
      .map(emails => discussion(emails.filter(e => !drafts.has(e)), emails.filter(e => drafts.has(e)), context, folderOf, texts, taskOf))
      .sort((a, b) => b.date_ts - a.date_ts);
  }

  function discussion(emails, drafts, context, folderOf, texts, taskOf) {
    const own = new Set((context.me || []).map(a => String(a || '').trim().toLowerCase()).filter(Boolean));
    const isMine = email => own.has(String(email.from_email || '').trim().toLowerCase());
    // What lists the discussion (Inbox and Sent) leads its line; Trash and Spam only show inside.
    const listed = emails.filter(e => ['inbox', 'sent'].includes(folderOf.get(e)));
    const last = listed[listed.length - 1];
    const unread = listed.filter(e => !e.is_read && !isMine(e));
    const people = new Map();
    const everyone = new Map();   // the details sheet: me too
    const meet = (name, address) => {
      const key = String(address || '').trim().toLowerCase();
      if (!key || everyone.has(key)) return;
      const p = person(name, address.trim());
      everyone.set(key, own.has(key) ? { ...p, mine: true } : p);
      if (!own.has(key)) people.set(key, p);
    };
    for (const e of emails) {
      meet(e.from_name, e.from_email);
      for (const a of [...splitAddresses(e.to), ...splitAddresses(e.cc)]) meet(a.name, a.address);
    }
    const others = [...people.values()];
    const messages = discussionItems(emails, isMine, people, context.now ?? Date.now(), folderOf, texts, taskOf);
    return {
      // A uid is only unique within its folder.
      id: String(emails[0].message_id || `${folderOf.get(emails[0])}:${emails[0].uid}`),
      people: others,
      title: others.slice(0, 2).map(p => p.short).join(', ') + (others.length > 2 ? ` +${others.length - 2}` : ''),
      subject: emails[0].subject || '',
      preview: { mine: isMine(last), text: String(last.body_text || '').replace(/\s+/g, ' ').trim().slice(0, 200) },
      date_ts: Number(last.date_ts) || 0,
      unread: unread.length,
      priority: unread.reduce((top, e) => (top && PRIORITY_ORDER[top] <= (PRIORITY_ORDER[e.priority] ?? 99) ? top : e.priority), null) ?? null,
      items: [
        ...(missesEarlier(emails) ? [{ kind: 'earlier' }] : []),
        ...messages,
        ...drafts.map(email => ({ kind: 'draft', email, text: String(email.body_text || '').trim(), date_ts: Number(email.date_ts) || 0 })),
      ],
      reply: replyAll(last, context.me || []),
      info: {
        people: [...everyone.values()],
        subjects: [...new Set(emails.map(e => String(e.subject || '').trim()).filter(Boolean))],
        where: whereKept([...emails.map(e => [e, folderOf.get(e)]), ...drafts.map(e => [e, 'drafts'])]),
        summary: String(emails.findLast(e => String(e.summary || '').trim())?.summary || '').trim(),
        files: messages.filter(i => i.kind === 'image' || i.kind === 'attachment'),
      },
    };
  }

  // The search bar over the discussion list: the discussions where every word of `query` is found
  // in a person's name or address (not mine), a subject, a message text or an attachment name;
  // case and accents ignored.
  const fold = s => String(s).normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase();
  const searchable = new WeakMap();   // discussion → its folded text, made once per built list
  function filterDiscussions(list, query) {
    const words = fold(query).split(/\s+/).filter(Boolean);
    if (!words.length) return list;
    const textOf = d => {
      if (!searchable.has(d)) {
        searchable.set(d, fold([
          ...d.people.flatMap(p => [p.name, p.address]), ...d.info.subjects,
          ...d.items.map(i => i.text ?? ''),
          ...d.items.filter(i => i.kind === 'attachment').map(i => i.name),
        ].join('\n')));
      }
      return searchable.get(d);
    };
    return list.filter(d => words.every(w => textOf(d).includes(w)));
  }

  // The details sheet's "where": per account (its position, from the email id's bits above 32),
  // how many of the discussion's emails each folder holds. `kept` is [email, folder] pairs.
  const FOLDER_ORDER = ['inbox', 'sent', 'trash', 'spam', 'drafts'];
  function whereKept(kept) {
    const accounts = new Map();
    for (const [email, folder] of kept) {
      const account = Math.floor((Number(email.uid) || 0) / 2 ** 32);
      if (!accounts.has(account)) accounts.set(account, new Map());
      const counts = accounts.get(account);
      counts.set(folder, (counts.get(folder) || 0) + 1);
    }
    return [...accounts].sort(([a], [b]) => a - b).map(([account, counts]) => ({
      account,
      folders: FOLDER_ORDER.filter(f => counts.has(f)).map(folder => ({ folder, count: counts.get(folder) })),
    }));
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
    // The account that received it is in its uid (mailing lists, Bcc and aliases hide it from To/Cc).
    const received = Math.floor(Number(email.uid) / 2 ** 32);
    const subject = String(email.subject || '');
    const to = pick(fromMe >= 0 ? splitAddresses(email.to) : [...sender, ...splitAddresses(email.to)]);
    const cc = pick(splitAddresses(email.cc));
    bubbleLabels([...to, ...cc]);
    return {
      email,
      from: fromMe >= 0 ? fromMe : received < me.length ? received : 0,
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
  function discussionItems(emails, isMine, people, now, folderOf, texts, taskOf = () => undefined) {
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
      const parts = emailParts(email, texts.get(email)).map(part => ({ ...part, email, folder: folderOf.get(email), mine: isMine(email), sender, date_ts: ts, first: false, last: false }));
      parts[0].first = !sameRun;
      const task = taskOf(email);
      if (task) parts[parts.length - 1].task = task;
      items.push(...parts);
      prev = { day, item: parts[parts.length - 1] };
    }
    if (prev) prev.item.last = true;
    return items;
  }

  // What one email shows as: its text, split by each image of its body ('image' items,
  // the cid: images the backend put in the HTML as data: URLs), then an 'attachment' item per file.
  function emailParts(email, text) {
    const parts = [];
    let lines = [];
    const flush = () => {
      const piece = lines.join('\n').trim();
      if (piece) parts.push({ kind: 'message', text: piece });
      lines = [];
    };
    for (const line of text.split('\n')) {
      if (line.startsWith(IMAGE)) { flush(); parts.push({ kind: 'image', src: line.slice(1) }); } else lines.push(line);
    }
    flush();
    for (const a of email.attachments || []) parts.push({ kind: 'attachment', name: a.name, size: a.size, mime: a.mime });
    return parts.length ? parts : [{ kind: 'message', text: '' }];
  }

  // An HTML body as bubble text: a line per block or table cell, **bold**, *italic*, [link](url),
  // [[button]](url) for a link drawn as a button, and an IMAGE line (IMAGE + data: URL) per picture of the body.
  // Head, styles, scripts, hidden text (a newsletter's preheader) and tracking pixels go.
  const IMAGE = '\u0001';
  const ENTITIES = {
    nbsp: ' ', amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", eacute: 'é', egrave: 'è', ecirc: 'ê', agrave: 'à',
    acirc: 'â', ccedil: 'ç', ocirc: 'ô', ucirc: 'û', ugrave: 'ù', icirc: 'î', iuml: 'ï', euml: 'ë', Eacute: 'É', Agrave: 'À',
    rsquo: '’', lsquo: '‘', rdquo: '”', ldquo: '“', laquo: '«', raquo: '»', hellip: '…', mdash: '—', ndash: '–', bull: '•',
    middot: '·', copy: '©', reg: '®', trade: '™', euro: '€', deg: '°', times: '×', zwnj: '', zwj: '', shy: '',
  };
  const decode = s => String(s)
    .replace(/&(#x[0-9a-f]+|#\d+|[a-z]+\d*);/gi, (m, e) => {
      if (e[0] !== '#') return ENTITIES[e] ?? ENTITIES[e.toLowerCase()] ?? m;
      const code = /x/i.test(e[1]) ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10);
      return code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : m;
    })
    .replace(/[­͏​-‏⁠﻿]/g, '');
  // Each attribute of a tag, by lower-case name ('' when it has no value), values decoded.
  function attrsOf(text) {
    const map = new Map();
    for (const m of String(text || '').matchAll(/([^\s=/"'>]+)(?:\s*=\s*("[^"]*"|'[^']*'|[^\s"'>]+))?/g)) {
      const value = m[2] ?? '';
      map.set(m[1].toLowerCase(), decode(/^["']/.test(value) ? value.slice(1, -1) : value));
    }
    return map;
  }
  const styleOf = attrs => (attrs.get('style') || '').toLowerCase().replace(/\s+/g, '');
  const VOID = new Set(['img', 'br', 'hr', 'input', 'meta', 'link', 'area', 'base', 'col', 'wbr', 'source']);
  const OPEN_TAG = /<([a-z][\w:-]*)((?:"[^"]*"|'[^']*'|[^'">])*)>/gi;
  // Where the element opening at `at` ends (after its closing tag; the end of the HTML when never closed).
  function elementEnd(html, name, at) {
    const tag = new RegExp(OPEN_TAG.source, 'iy');
    tag.lastIndex = at;
    const open = tag.exec(html)?.[0] || '';
    if (VOID.has(name) || open.endsWith('/>')) return at + open.length;
    const tags = new RegExp(`<(/?)${name}(?=[\\s/>])[^>]*>`, 'gi');
    tags.lastIndex = at;
    let depth = 0;
    for (let m; (m = tags.exec(html));) {
      depth += m[1] ? -1 : m[0].endsWith('/>') ? 0 : 1;
      if (depth <= 0) return m.index + m[0].length;
    }
    return html.length;
  }
  // `html` without the elements `test(name, attrs)` picks (with all they hold).
  function removeElements(html, test) {
    let out = '';
    let from = 0;
    for (const m of html.matchAll(OPEN_TAG)) {
      if (m.index < from || !test(m[1].toLowerCase(), attrsOf(m[2]))) continue;
      out += html.slice(from, m.index);
      from = elementEnd(html, m[1].toLowerCase(), m.index);
    }
    return out + html.slice(from);
  }
  const isHidden = (name, attrs) => {
    const style = styleOf(attrs);
    return attrs.has('hidden') || /(?:^|;)(?:display:none|visibility:hidden|opacity:0(?:\.0*)?(?:;|$))/.test(style)
      || (/(?:^|;)max-height:0(?:px)?(?:;|$)/.test(style) && /overflow:hidden/.test(style));
  };
  const COLOURLESS = /^(?:none|transparent|inherit|initial|white|#fff(?:fff)?|rgba?\(255,255,255(?:,1)?\))?(?:!important)?$/;
  const coloured = attrs => {
    const background = /(?:^|;)background(?:-color)?:([^;]+)/.exec(styleOf(attrs))?.[1] ?? '';
    return !COLOURLESS.test(background.replace(/url\([^)]*\)/g, '').trim()) || !COLOURLESS.test((attrs.get('bgcolor') || '').toLowerCase().trim());
  };
  const looksButton = attrs => (attrs.get('role') || '').toLowerCase() === 'button' || coloured(attrs)
    || /(?:^|;)border(?:-[a-z]+)?:[^;]*(?:solid|dashed|dotted|double)/.test(styleOf(attrs))
    || /(?:^|;)padding(?:-[a-z]+)?:[^;]*[1-9]/.test(styleOf(attrs));
  // A cell holding one link and nothing else.
  const onlyALink = inner => (inner.match(/<a\b/gi) || []).length === 1
    && !decode(removeElements(inner, name => name === 'a').replace(/<[^>]*>/g, '')).trim();
  const BREAK = new Set(['div', 'tr', 'td', 'th', 'li', 'dt', 'dd', 'center', 'section', 'article', 'header', 'footer', 'hr', 'address', 'pre', 'form', 'caption']);
  const PARAGRAPH = new Set(['p', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'table', 'ul', 'ol', 'blockquote', 'dl']);
  const TOKEN = /<(\/?)([a-z][\w:-]*)((?:"[^"]*"|'[^']*'|[^'">])*)>|<[!?/][^>]*>|([^<]+|<)/gi;
  // The signature a mail app marks in its HTML: Gmail, Outlook (desktop, web, mobile), Apple Mail,
  // Thunderbird, Proton Mail.
  const SIGNATURE_CLASS = /^(?:gmail_signature(?:_prefix)?|moz-signature|protonmail_signature_block)$/i;
  const SIGNATURE_ID = /^(?:x_)?(?:Signature|AppleMailSignature|ms-outlook-mobile-signature)$/i;
  const isSignatureMark = (name, attrs) => (attrs.get('class') || '').split(/\s+/).some(c => SIGNATURE_CLASS.test(c))
    || attrs.get('data-smartmail') === 'gmail_signature' || SIGNATURE_ID.test(attrs.get('id') || '');
  // The quoted history a mail app marks in its HTML: the element goes (Gmail, Apple Mail, Thunderbird,
  // Yahoo, Outlook for Mac), or all from it to the end (Outlook: the quote follows the header it marks).
  const QUOTE_CLASS = /^(?:gmail_quote(?:_container)?|yahoo_quoted|moz-cite-prefix)$/i;
  const QUOTE_ID = /^(?:yahoo_quoted_\d+|mail-editor-reference-message-container)$/i;
  const isQuoteMark = (name, attrs) => (attrs.get('class') || '').split(/\s+/).some(c => QUOTE_CLASS.test(c))
    || (name === 'blockquote' && (attrs.get('type') || '').toLowerCase() === 'cite') || QUOTE_ID.test(attrs.get('id') || '');
  const QUOTE_TO_END = /<[a-z][^>]*?\sid\s*=\s*["']?(?:x_)?(?:divRplyFwdMsg|appendonsend|stopSpelling)\b/i;
  const removeQuotes = html => removeElements(html.slice(0, QUOTE_TO_END.exec(html)?.index ?? html.length), isQuoteMark);
  let markedNow = new Map();   // HTML → its texts, for this build (and the one before)
  let markedBefore = new Map();
  // { text, signed, full() }: `text` is without the marked quote and signature, `signed` when a marked
  // signature was cut (when cutting leaves nothing, the text keeps what was cut); `full()` keeps the quote.
  function htmlMarked(source) {
    const known = markedNow.get(source) ?? markedBefore.get(source);
    if (known !== undefined) { markedNow.set(source, known); return known; }
    const html = removeElements(String(source)
      .replace(/<!--[\s\S]*?-->/g, '')
      .replace(/<(head|style|script|title|template)\b[\s\S]*?<\/\1\s*>/gi, ''), isHidden);
    const unquoted = removeQuotes(html);
    const unsigned = removeElements(unquoted, isSignatureMark);
    let text = unsigned !== unquoted ? marked(unsigned) : '';
    const signed = Boolean(text);
    if (!text) text = marked(unquoted) || marked(html);
    let full;
    const result = { text, signed, full: () => (full ??= marked(removeElements(html, isSignatureMark)) || marked(html)) };
    markedNow.set(source, result);
    return result;
  }
  function marked(html) {
    let out = '';
    let pending = 0;      // line breaks owed before the next text
    let space = false;    // a space owed before the next text
    let bold = 0, italic = 0;
    const links = [];     // the <a> we are in: { url, button } or null (not a web or mail link)
    const lists = [];     // the <ul>/<ol> we are in: a count for <ol>, null for <ul>
    let open = { link: null, bold: false, italic: false };
    let buttonUntil = -1; // a coloured cell around one link: that link is a button
    const close = (keep = {}) => {
      if (open.italic && !keep.italic) { out += '*'; open.italic = false; }
      if (open.bold && !keep.bold) { out += '**'; open.bold = false; }
      if (open.link && !keep.link) { out += `${open.link.button ? ']]' : ']'}(${open.link.url})`; open.link = null; }
    };
    const breakLine = n => { close(); pending = Math.max(pending, n); space = false; };
    const startLine = () => {
      if (out && pending) out += '\n'.repeat(pending);
      pending = 0;
    };
    const write = text => {
      const link = links.length ? links[links.length - 1] : null;
      const want = { link, bold: !link && bold > 0, italic: !link && italic > 0 };
      // What changes closes, inner marks first (and what was inside it opens again).
      if (open.link !== want.link) close();
      else if (open.bold !== want.bold) close({ link: true });
      else if (open.italic !== want.italic) close({ link: true, bold: true });
      if (pending) startLine();
      else if (space && out && !out.endsWith('\n')) out += ' ';
      space = false;
      if (want.link && !open.link) { out += want.link.button ? '[[' : '['; open.link = want.link; }
      if (want.bold && !open.bold) { out += '**'; open.bold = true; }
      if (want.italic && !open.italic) { out += '*'; open.italic = true; }
      out += link ? text.replace(/[[\]]/g, '') : text;
    };
    for (const m of html.matchAll(TOKEN)) {
      if (m[4] !== undefined) {
        const text = decode(m[4]).replace(/\s+/g, ' ');
        if (!text.trim()) { space = true; continue; }
        if (text.startsWith(' ')) space = true;
        write(text.trim());
        if (text.endsWith(' ')) space = true;
        continue;
      }
      if (!m[2]) continue;
      const name = m[2].toLowerCase();
      const closing = m[1] === '/';
      if (name === 'br') { close(); pending = Math.min(2, pending + 1); space = false; continue; }
      if (PARAGRAPH.has(name)) breakLine(2);
      else if (BREAK.has(name)) breakLine(1);
      if (name === 'b' || name === 'strong') bold = Math.max(0, bold + (closing ? -1 : 1));
      else if (name === 'i' || name === 'em') italic = Math.max(0, italic + (closing ? -1 : 1));
      else if (name === 'ul' || name === 'ol') { if (closing) lists.pop(); else lists.push(name === 'ol' ? 0 : null); }
      else if (name === 'li' && !closing) {
        startLine();
        const n = lists.length && lists[lists.length - 1] !== null ? ++lists[lists.length - 1] : null;
        out += n === null ? '* ' : `${n}. `;
      } else if ((name === 'td' || name === 'th') && !closing) {
        const attrs = attrsOf(m[3]);
        const end = elementEnd(html, name, m.index);
        if (coloured(attrs) && onlyALink(html.slice(m.index + m[0].length, end))) buttonUntil = end;
      } else if (name === 'a') {
        if (closing) { links.pop(); continue; }
        const attrs = attrsOf(m[3]);
        const href = (attrs.get('href') || '').trim();
        links.push(/^(?:https?:|mailto:)/i.test(href)
          ? { url: href.replace(/\s+/g, '').replace(/\(/g, '%28').replace(/\)/g, '%29'), button: m.index < buttonUntil || looksButton(attrs) }
          : null);
      } else if (name === 'img') {
        const attrs = attrsOf(m[3]);
        const src = (attrs.get('src') || '').trim();
        const size = key => parseInt(attrs.get(key) ?? new RegExp(`(?:^|;)${key}:(\\d+)`).exec(styleOf(attrs))?.[1], 10);
        if (/^data:image\//i.test(src) && !(size('width') <= 2 || size('height') <= 2)) {
          breakLine(1);
          startLine();
          out += IMAGE + src.replace(/\s+/g, '');
          pending = 1;
        } else if (links.length && (attrs.get('alt') || '').trim()) {
          write(attrs.get('alt').replace(/\s+/g, ' ').trim());   // a picture used as a link: its words
        }
      }
    }
    close();
    return out.split('\n').map(line => line.trim()).join('\n').replace(/\n{3,}/g, '\n\n').trim();
  }

  // Only what the person newly wrote, cut by rules (no AI): the quoted history goes.
  // When a cut would leave nothing, the message is kept whole.
  const QUOTE_FROM = /^\s*(?:De|From)\s*:/i;
  const QUOTE_SUBJECT = /^\s*(?:Objet|Subject)\s*:/i;
  // May wrap onto a 2nd line. It always carries the date ("Le lun. 26 oct. 2026 à 09:12, … a écrit :"),
  // which tells it from a sentence like "Le client a écrit :".
  const QUOTE_INTRO = /^\s*(?:Le|On)\s.*\d.*(?:a\s+écrit|wrote)\s*:\s*$/i;
  const RULE_LINE = /^\s*(?:[-_─=]{3,}|-+\s*(?:Original Message|Message d'origine)\s*-+)?\s*$/i;
  // `signed`: its HTML marked the signature, already cut; no closing is guessed then.
  // A forward: its note is cut, the forwarded message is kept whole after it.
  function newText(email, signatures = [], body = emailText(email)) {
    const cut = cutText(body.text, signatures, body.signed);
    if (body.forwarded !== undefined) return [cut, body.forwarded].filter(Boolean).join('\n\n');
    return cut || body.text;
  }

  // An email's whole text ({ text, signed, forwarded }): its HTML as bubble text, else its plain text.
  // `forwarded`, for a forward, is the forwarded message, out of `text` (the note before it).
  const FORWARD_SUBJECT = /^\s*(?:fwd?|tr|wg|rv|enc)\s*:/i;
  const FORWARD_LINE = /^[\s-]*(?:forwarded message|message transféré|begin forwarded message|début du message (?:réexpédié|transféré))\s*:?[\s-]*$/i;
  const FORWARD_WORDS = /forwarded message|message transféré|début du message (?:réexpédié|transféré)|moz-forward-container/i;
  function emailText(email) {
    const html = String(email.body_html || '');
    const fromHtml = html.trim() ? htmlMarked(html) : { text: '' };
    if (fromHtml.text) {
      const forward = (FORWARD_SUBJECT.test(email.subject || '') || FORWARD_WORDS.test(html)) && forwardOf(fromHtml.full(), email);
      return forward || { text: fromHtml.text, signed: fromHtml.signed };
    }
    // A link shows as "[text][1]" with its address on a "[1]: url" line at the very end
    // (after any quote): put the address back beside it, as "[text](url)".
    const links = new Map();
    const body = String(email.body_text || '').replace(/\r\n?/g, '\n').split('\n')
      .filter(line => { const m = /^\[(\d+)\]:\s*(\S+)\s*$/.exec(line); if (m) links.set(m[1], m[2]); return !m; })
      .join('\n');
    const text = body.replace(/\[([^\]\n]*)\]\[(\d+)\]/g, (all, t, n) => (links.has(n) ? `[${t}](${links.get(n)})` : all)).trim();
    return forwardOf(text, email) || { text, signed: false };
  }

  // A forward: a "Forwarded message" line comes before any quote, or the subject says Fwd:/TR: and a quote starts.
  // Never a reply (Re:, AW:…), even to a forward: what it quotes is history.
  const REPLY_SUBJECT = /^\s*(?:re|aw|sv|antw|r|réf)\s*:/i;
  function forwardOf(text, email) {
    if (REPLY_SUBJECT.test(email.subject || '')) return null;
    const lines = text.split('\n');
    const split = i => ({ text: lines.slice(0, i).join('\n').trim(), signed: false, forwarded: lines.slice(i).join('\n').trim() });
    for (let i = 0; i < lines.length; i++) {
      if (FORWARD_LINE.test(plainLine(lines[i]))) return split(i);
      if (quoteStarts(lines, i)) return FORWARD_SUBJECT.test(email.subject || '') ? split(i) : null;
    }
    return null;
  }

  // A whole line only: "Envoyé de mon iPhone", "Sent from Outlook for Android", "Get Outlook for iOS".
  const MOBILE = new RegExp('^\\s*(?:(?:envoyée?|sent|expédiée?)\\s+(?:de|depuis|à partir de|from|with|via)\\s+(?:mon|ma|my)?\\s*(?:\\S+\\s+)?'
    + '(?:iphone|ipad|android|blackberry|samsung|galaxy|mobile|smartphone|téléphone|phone|tablet|tablette|appareil|outlook|mail|yahoo|gmail|protonmail|courrier)[^\\s,;:]*'
    + '|(?:get|obtenir|télécharger)\\s+outlook\\s+(?:for|pour)\\s+[^\\s,;:]+)(?:\\s+[^\\s,;:]+){0,3}'
    // "…, please excuse brevity" / "…, excusez la brièveté"; any other comma means a sentence ("Sent from my phone, call me").
    + '(?:\\s*[,.]\\s*(?:please\\s+|veuillez\\s+|merci\\s+d\'|désolée?\\s+pour\\s+)?(?:excuse|excusez|pardon|sorry|la\\s+brièveté).*)?\\s*$', 'i');
  // A paragraph of 100+ characters, starting like a legal notice and saying it is confidential and for its recipient only.
  const DISCLAIMER_START = /^\W*(?:(?:ce|cet|le présent)\s+(?:message|courriel|e-?mail|mail)|les informations|this\s+(?:e-?mail|message|communication|transmission)|the information|confidential|confidentialit|avertissement|disclaimer|avis de confidentialit|privileged)/i;
  const DISCLAIMER_WORDS = [/confiden|privil/i, /destinataire|destiné|intended|addressee|recipient|erreur|error/i];
  // "Please consider the environment before printing", "Pensez à la planète avant d'imprimer".
  const ENVIRONMENT = [/environ|planet|planète|tree|arbre|paper|papier/i,
    /before printing|avant d'imprimer|avant impression|n'imprimez|ne pas imprimer|n'imprimer|need to print|print only if|imprimer (?:que|uniquement) si/i];
  // A closing alone on its line: "Cordialement,", "Bien à vous", "Best regards".
  const CLOSING = /^\s*(?:(?:bien|très|tres)\s+cordialement|cordialement|cdlt|bien\s+à\s+(?:vous|toi)|(?:bien\s+)?amicalement|(?:sincères|meilleures|bonnes)\s+salutations(?:\s+distinguées)?|salutations(?:\s+distinguées)?|(?:thanks\s+(?:&|and)\s+|best|kind|warm|warmest|many)?\s*regards|best(?:\s+wishes)?|(?:yours\s+)?(?:sincerely|faithfully|truly)|cheers)\s*[,.!]?\s*$/i;
  // A line that reads as written text, not a signature line: a P.S., or a sentence of 5+ words.
  const isWritten = line => /^p\.?\s?s\b/i.test(line) || (/[.!?]$/.test(line) && line.split(/\s+/).length >= 5);

  // Line i starts the quoted history: an Outlook header block (From: then Subject: within 5 lines,
  // or 3+ header lines in a row), "Le … a écrit :", "-----Original Message-----".
  const HEADER_LINE = /^(?:from|de|sent|envoyé|date|to|à|a|cc|subject|objet)\s*:/i;
  const ORIGINAL_LINE = /^\s*-{2,}\s*(?:original message|message d'origine)\s*-{2,}\s*$/i;
  function quoteStarts(lines, i) {
    const line = lines[i];
    if (line.startsWith(IMAGE)) return false;
    if (QUOTE_FROM.test(plainLine(line))) {
      const next = lines.slice(i + 1, i + 6).map(plainLine);
      if (next.some(l => QUOTE_SUBJECT.test(l)) || (next.length >= 2 && next.slice(0, 2).every(l => HEADER_LINE.test(l)))) return true;
    }
    return QUOTE_INTRO.test(line) || QUOTE_INTRO.test(`${line} ${lines[i + 1]}`) || ORIGINAL_LINE.test(line);
  }

  // Line i starts the quoted history, or a signature block ("-- ", a "sent from my phone" line,
  // a legal notice, a "think before printing" line): it and all after go.
  function historyStarts(lines, i) {
    const line = lines[i];
    if (line.startsWith(IMAGE)) return false;
    if (quoteStarts(lines, i)
      || /^--\s?$/.test(line)   // signature delimiter "-- " (the space may be trimmed)
      || MOBILE.test(line)
      || ENVIRONMENT.every(re => re.test(line))) return true;
    if (!DISCLAIMER_START.test(line)) return false;
    const end = lines.findIndex((l, j) => j > i && !l.trim());
    const paragraph = lines.slice(i, end < 0 ? lines.length : end).join(' ');
    return paragraph.length >= 100 && DISCLAIMER_WORDS.every(re => re.test(paragraph));
  }

  // Line i is a closing ("Cordialement,") followed only by signature lines (name, title, phone…)
  // up to the end or the quoted history: the closing stays, all after it goes. Real text after it keeps it.
  function closingStarts(lines, i) {
    if (!CLOSING.test(lines[i])) return false;
    const rest = [];
    for (let j = i + 1; j < lines.length && !historyStarts(lines, j); j++) {
      const line = plainLine(lines[j]);
      if (line && !line.startsWith('>')) rest.push(line);
    }
    return rest.length <= MAX_TAIL && !rest.some(isWritten);
  }

  // What is left of a text once the quoted history and the signatures go ('' when nothing).
  function cutText(whole, signatures = [], signed = false) {
    const lines = whole.split('\n');
    const kept = [];
    for (let i = 0; i < lines.length; i++) {
      if (historyStarts(lines, i)) break;
      if (!lines[i].trimStart().startsWith('>')) kept.push(lines[i]);
      if (!signed && closingStarts(lines, i)) break;
    }
    for (const signature of signatures) cutBlock(kept, signature.split('\n'));
    while (kept.length && RULE_LINE.test(kept[kept.length - 1])) kept.pop();
    return kept.join('\n').trim();
  }

  // Each email's new text. Then, in each discussion (`threads`, oldest email first), the end of an email
  // that repeats an earlier message (20+ letters) is cut: a quote no rule knew. Then the last lines one
  // sender ends 2+ of their emails with (a signature we were not told about) are cut: the longest such
  // block, up to MAX_TAIL lines. One line alone counts only when long and not a sentence: "Thanks" or
  // "Merci" is no signature. Emails whose HTML marked their signature are left out (it is already cut),
  // and forwards too (what they end with is the forwarded message).
  const MAX_TAIL = 15;
  const MIN_TAIL_LINE = 15;
  const MIN_REPEAT = 20;
  function newTexts(emails, signatures, threads = []) {
    const unsigned = [];
    const forwards = new Set();
    const texts = new Map(emails.map(e => {
      const whole = emailText(e);
      if (whole.forwarded !== undefined) forwards.add(e);
      else if (!whole.signed) unsigned.push(e);
      return [e, newText(e, signatures, whole)];
    }));
    for (const thread of threads) {
      const earlier = [];
      for (const email of thread.filter(e => texts.has(e)).sort((a, b) => (a.date_ts || 0) - (b.date_ts || 0))) {
        if (!forwards.has(email)) texts.set(email, cutRepeat(texts.get(email), earlier));
        const words = plainWords(texts.get(email));
        if (words.length >= MIN_REPEAT) earlier.push(words);
      }
    }
    const tails = new Map();   // sender + last k lines → distinct emails ending so
    const tailKeys = email => {
      const lines = texts.get(email).split('\n').map(plainLine).filter(Boolean);
      const sender = String(email.from_email || '').trim().toLowerCase();
      return lines.slice(-MAX_TAIL).map((_, k, last) => `${sender}\n${last.slice(last.length - k - 1).join('\n')}`);
    };
    const isTail = (key, k) => tails.get(key).size >= 2
      && (k > 0 || (key.length - key.indexOf('\n') - 1 >= MIN_TAIL_LINE && !isWritten(key.slice(key.indexOf('\n') + 1))));
    for (const email of unsigned) {
      for (const key of tailKeys(email)) {
        if (!tails.has(key)) tails.set(key, new Set());
        tails.get(key).add(String(email.message_id || email.uid));
      }
    }
    for (const email of unsigned) {
      const k = tailKeys(email).findLastIndex(isTail) + 1;
      if (!k) continue;
      const lines = texts.get(email).split('\n');
      let cut = lines.length;
      for (let n = 0; n < k; cut--) if (plainLine(lines[cut - 1])) n++;
      texts.set(email, lines.slice(0, cut).join('\n').trim() || texts.get(email));
    }
    return texts;
  }

  // A text's words, for spotting it again quoted: no ">" quote marks, formatting, spacing or case.
  const plainWords = text => text.split('\n').map(l => plainLine(l.replace(/^(?:\s*>)+/, ''))).join('').replace(/\s+/g, '').toLowerCase();

  // `text` up to where its lines (from the 2nd) repeat one of the `earlier` texts (as plainWords),
  // with the line just before when it introduces the quote ("…, 26.10.2026:"). Whole when that leaves nothing.
  function cutRepeat(text, earlier) {
    if (!earlier.length) return text;
    const lines = text.split('\n');
    let at = -1;
    let rest = '';
    for (let s = lines.length - 1; s >= 1; s--) {
      const words = plainWords(lines[s]);
      rest = words + rest;
      if (words && earlier.some(w => rest.startsWith(w))) at = s;
    }
    if (at < 0) return text;
    while (at > 0 && !lines[at - 1].trim()) at--;
    if (at > 0 && /:\s*$/.test(lines[at - 1]) && plainLine(lines[at - 1]).length <= 100) at--;
    return lines.slice(0, at).join('\n').trim() || text;
  }

  // A line's words only: formatting marks (**bold**, *italic*, [link][1] / [link](url)) and spacing ignored;
  // an image line has none.
  const plainLine = line => (String(line).startsWith(IMAGE) ? '' : String(line)
    .replace(/\]\[\d+\]|\]\([^)]*\)/g, ']').replace(/[*[\]]/g, '').replace(/\s+/g, ' ').trim());

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
      colour: PERSON_COLOURS[hash % PERSON_COLOURS.length], org: orgOf(address),
    };
  }

  // The organisation in an address: "ann@mail.studio-k.fr" -> "studio-k", "bob@ox.ac.uk" -> "ox".
  const SECOND_LEVEL = new Set(['co', 'com', 'org', 'net', 'ac', 'gov', 'edu', 'ne', 'or', 'go']);
  function orgOf(address) {
    const at = address.lastIndexOf('@');
    if (at < 0) return '';
    const labels = address.slice(at + 1).toLowerCase().split('.').filter(Boolean);
    if (labels.length > 1) labels.pop();
    if (labels.length > 1 && SECOND_LEVEL.has(labels[labels.length - 1])) labels.pop();
    return labels.pop() || '';
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
    filterDiscussions,
    fold,
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
