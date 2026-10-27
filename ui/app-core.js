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
    removeEmailFromState,
    autoUpdatesEnabled,
    saveAutoUpdates,
    updateProgressPercent,
    needsAccountSetup,
    attentionEmails,
    activeTasks,
  };
});
