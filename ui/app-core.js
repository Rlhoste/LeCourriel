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
    // A task taken from an email names it by its inbox id (other folders number theirs apart).
    const tasks = new Map((context.tasks || []).filter(t => t.source_uid != null).map(t => [t.source_uid, t]));
    const taskOf = email => (folderOf.get(email) === 'inbox' ? tasks.get(email.uid) : undefined);
    return groupThreads(sortEmails([...folderOf.keys(), ...drafts]))
      .map(({ lead, replies }) => [lead, ...replies].reverse())
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
  // An email laid out as a page (newsletter, invoice) is one 'page' card instead of its text and images.
  const BODY_IMAGE = /<img\b[^>]*?\bsrc\s*=\s*(["'])(data:image\/[^"']*)\1[^>]*>/gi;
  function emailParts(email, text) {
    const html = String(email.body_html || '');
    let parts = [];
    let from = 0;
    if (html && (email.html_only || mostlyTables(html))) parts.push({ kind: 'page' });
    else for (const m of html.matchAll(BODY_IMAGE)) {
      parts.push({ kind: 'message', text: htmlText(html.slice(from, m.index)) }, { kind: 'image', src: m[2] });
      from = m.index + m[0].length;
    }
    if (parts[0]?.kind !== 'page') parts.push({ kind: 'message', text: parts.length ? htmlText(html.slice(from)) : text });
    // Once a piece reaches the quoted history or the signature, the rest (its images too) goes.
    const cut = parts.findIndex(p => p.kind === 'message' && p.text && cutText(p.text) !== p.text);
    if (parts.length > 1 && cut >= 0) parts.splice(cut, parts.length, { kind: 'message', text: cutText(parts[cut].text) });
    // A signature only the plain text shows (mine, or the same last lines across a sender's emails),
    // with a logo in the HTML: the pieces are cut at its first line too.
    const full = newText(email);
    if (parts.length > 1 && text && full !== text && full.startsWith(text)) {
      const first = plainLine(full.slice(text.length).trim().split('\n')[0]);
      const isFirst = line => plainLine(line) === first;
      const at = parts.findLastIndex(p => p.kind === 'message' && p.text && p.text.split('\n').some(isFirst));
      if (at >= 0) {
        const lines = parts[at].text.split('\n');
        parts.splice(at, parts.length, { kind: 'message', text: lines.slice(0, lines.findLastIndex(isFirst)).join('\n').trim() });
      }
    }
    for (const a of email.attachments || []) parts.push({ kind: 'attachment', name: a.name, size: a.size, mime: a.mime });
    if (parts.length > 1) parts = parts.filter(p => p.kind !== 'message' || p.text);
    return parts;
  }

  // Most of the text sits in tables: a layout, not a written message (a signature table is not enough).
  // Only the new text counts: the quoted history (a quoted newsletter, say) stops the count.
  function mostlyTables(html) {
    let inside = 0, outside = 0, depth = 0, from = 0, ended = false;
    const count = piece => {
      if (ended) return;
      const lines = htmlText(piece).split('\n');
      const end = lines.findIndex((_, i) => cutStarts(lines, i));
      ended = end >= 0;
      const letters = (ended ? lines.slice(0, end) : lines).join('').replace(/\s/g, '').length;
      if (depth > 0) inside += letters; else outside += letters;
    };
    for (const m of html.matchAll(/<(\/?)table\b[^>]*>/gi)) {
      count(html.slice(from, m.index));
      depth = Math.max(0, depth + (m[1] ? -1 : 1));
      from = m.index + m[0].length;
    }
    count(html.slice(from));
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
      .split('\n').map(line => line.replace(/\s+/g, ' ').trim()).join('\n')
      .replace(/\n{3,}/g, '\n\n')
      .trim();
  }

  // Only what the person newly wrote, cut by rules (no AI): the quoted history goes.
  // When a cut would leave nothing, the message is kept whole.
  const QUOTE_FROM = /^\s*(?:De|From)\s*:/i;
  const QUOTE_SUBJECT = /^\s*(?:Objet|Subject)\s*:/i;
  // May wrap onto a 2nd line. It always carries the date ("Le lun. 26 oct. 2026 à 09:12, … a écrit :"),
  // which tells it from a sentence like "Le client a écrit :".
  const QUOTE_INTRO = /^\s*(?:Le|On)\s.*\d.*(?:a\s+écrit|wrote)\s*:\s*$/i;
  const RULE_LINE = /^\s*(?:[-_─=]{3,}|-+\s*(?:Original Message|Message d'origine)\s*-+)?\s*$/i;
  function newText(email, signatures = []) {
    // A link shows as "[text][1]" with its address on a "[1]: url" line at the very end
    // (after any quote): put the address back beside it, as "[text](url)".
    const links = new Map();
    const body = String(email.body_text || '').replace(/\r\n?/g, '\n').split('\n')
      .filter(line => { const m = /^\[(\d+)\]:\s*(\S+)\s*$/.exec(line); if (m) links.set(m[1], m[2]); return !m; })
      .join('\n');
    const whole = body.replace(/\[([^\]\n]*)\]\[(\d+)\]/g, (all, text, n) => (links.has(n) ? `[${text}](${links.get(n)})` : all)).trim();
    return cutText(whole, signatures) || whole;
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

  // Line i starts the quoted history (an Outlook header block, "Le … a écrit :") or a signature block
  // ("-- ", a "sent from my phone" line, a legal notice, a "think before printing" line): it and all after go.
  function historyStarts(lines, i) {
    const line = lines[i];
    if ((QUOTE_FROM.test(line) && lines.slice(i + 1, i + 6).some(l => QUOTE_SUBJECT.test(l)))
      || QUOTE_INTRO.test(line) || QUOTE_INTRO.test(`${line} ${lines[i + 1]}`)
      || /^--\s?$/.test(line)   // signature delimiter "-- " (the space may be trimmed)
      || MOBILE.test(line)
      || ENVIRONMENT.every(re => re.test(line))) return true;
    if (!DISCLAIMER_START.test(line)) return false;
    const end = lines.findIndex((l, j) => j > i && !l.trim());
    const paragraph = lines.slice(i, end < 0 ? lines.length : end).join(' ');
    return paragraph.length >= 100 && DISCLAIMER_WORDS.every(re => re.test(paragraph));
  }

  // Line i is a closing ("Cordialement,") followed only by signature lines (name, title, phone…)
  // up to the end or the quoted history: it and all after go. Real text after it keeps it.
  function closingStarts(lines, i) {
    if (!CLOSING.test(lines[i])) return false;
    const rest = [];
    for (let j = i + 1; j < lines.length && !historyStarts(lines, j); j++) {
      const line = plainLine(lines[j]);
      if (line && !line.startsWith('>')) rest.push(line);
    }
    return rest.length <= MAX_TAIL && !rest.some(isWritten);
  }

  const cutStarts = (lines, i) => historyStarts(lines, i) || closingStarts(lines, i);

  // What is left of a text once the quoted history and the signatures go ('' when nothing).
  function cutText(whole, signatures = []) {
    const lines = whole.split('\n');
    const kept = [];
    for (let i = 0; i < lines.length; i++) {
      if (cutStarts(lines, i)) break;
      if (!lines[i].trimStart().startsWith('>')) kept.push(lines[i]);
    }
    for (const signature of signatures) cutBlock(kept, signature.split('\n'));
    while (kept.length && RULE_LINE.test(kept[kept.length - 1])) kept.pop();
    return kept.join('\n').trim();
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
