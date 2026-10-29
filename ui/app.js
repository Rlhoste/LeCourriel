/* global __TAURI__ */
const invoke = window.__TAURI__.core.invoke;
const { listen } = window.__TAURI__.event;

const BUG_REPORT_TO = 'remi.lhoste@o2game.com';
const SPONSOR_URL = 'https://ko-fi.com/rlhoste';

// ── State ──────────────────────────────────────────────────────────────────
let appState = null;
let activePanel = 'overview';
let fontScale = 1.0;
let showOpenedAttention = false;

// ── Tauri helpers ──────────────────────────────────────────────────────────
async function cmd(name, args = {}) {
  try { return await invoke(name, args); }
  catch (e) { console.error(name, e); throw e; }
}

// ── Startup ────────────────────────────────────────────────────────────────
document.addEventListener('DOMContentLoaded', async () => {
  // A composer window (see openCompose) shows only the composer.
  if (window.__COMPOSE__) { startComposeWindow(window.__COMPOSE__); return; }

  // Register listeners FIRST before anything else to avoid race condition
  await listen('state-updated', e => {
    appState = e.payload;
    renderAll();
  });

  await listen('loading', e => {
    document.getElementById('loading-bar').classList.toggle('hidden', !e.payload);
  });

  await listen('error', e => {
    console.error('Backend error:', e.payload);
    document.getElementById('ov-briefing').textContent = e.payload;
  });

  await listen('update-progress', e => renderUpdateProgress(e.payload || {}));
  await listen('compose-sent', () => showToast(t('cmp.sent')));

  initLang();          // language from the last session, until settings arrive
  applyStaticI18n();
  updateTranslateButton();
  setupSidebar();
  setupOverview();
  setupSplitPanels();
  setupCompose();
  setupSettings();
  setupCalendarNav();
  setupCalendarTaskDialog();
  setupNewProject();
  setupKanban();
  setupSearch();

  // Apply saved font scale and theme
  try {
    const s = await invoke('get_settings');
    fontScale = s.font_scale || 1.0;
    document.documentElement.style.fontSize = `${fontScale * 14}px`;
    applyTheme(s.theme);
    if (s.language && s.language !== LANG) applyLanguage(s.language);
    await loadComposeAccounts();
    applyDiscussionMode(!!s.discussion_mode);
    // First start (or no account yet): open Settings on Accounts, with the setup dialog on top.
    if (CourrielCore.needsAccountSetup(s)) {
      document.getElementById('ov-briefing').textContent = t('ov.noAccount');
      await openSettings();
      switchSettingsTab('accounts');
      if (!settingsData.accounts.length) openSetupDialog();
    }
  } catch (_) {}

  // Tell backend we're ready — it will now send cache then start live refresh
  await invoke('app_ready');
  checkAutomaticUpdates();
});

// ── Sidebar navigation ─────────────────────────────────────────────────────
function setupSidebar() {
  document.querySelectorAll('.nav-btn[data-panel]').forEach(btn => {
    btn.addEventListener('click', () => switchPanel(btn.dataset.panel));
  });

  document.getElementById('btn-refresh').addEventListener('click', async () => {
    const bar = document.getElementById('loading-bar');
    bar.classList.remove('hidden');
    try { await cmd('refresh_emails'); }
    catch (_) {}   // the backend also reports errors through the 'error' event
    finally { bar.classList.add('hidden'); }
  });

  document.getElementById('btn-settings').addEventListener('click', openSettings);
  document.getElementById('btn-compose').addEventListener('click', () => openCompose());
  document.getElementById('dm-new').addEventListener('click', () => openCompose());
  // Opens in the system browser (Tauri shell plugin), not inside the app window.
  document.getElementById('btn-sponsor').addEventListener('click', () =>
    window.__TAURI__.shell.open(SPONSOR_URL).catch(e => console.error('open sponsor page', e)));
  document.getElementById('btn-report-bug').addEventListener('click', () => openCompose({
    to: BUG_REPORT_TO,
    subject: 'Sorry Remi, a new bug in your list',
  }));
}

function switchPanel(panel) {
  activePanel = panel;
  document.querySelectorAll('.nav-btn[data-panel]').forEach(b =>
    b.classList.toggle('active', b.dataset.panel === panel)
  );
  document.querySelectorAll('.panel').forEach(p =>
    p.classList.toggle('active', p.id === `panel-${panel}`)
  );
  if (appState) renderPanel(panel);
}

function setupOverview() {
  document.getElementById('ov-setup-ai').addEventListener('click', async () => {
    await openSettings();
    switchSettingsTab('ai');
  });
  document.getElementById('ov-toggle-opened').addEventListener('click', () => {
    showOpenedAttention = !showOpenedAttention;
    if (appState) renderOverview();
    else renderOverviewOpenedToggle();
  });
}

function renderOverviewOpenedToggle() {
  const button = document.getElementById('ov-toggle-opened');
  button.textContent = t(showOpenedAttention ? 'ov.hideOpened' : 'ov.showOpened');
  button.classList.toggle('active', showOpenedAttention);
  button.setAttribute('aria-pressed', String(showOpenedAttention));
}

// ── Split panels bootstrap ─────────────────────────────────────────────────
const SPLIT_PANELS = {
  inbox:     { title: 'Inbox',     key: 'all_emails' },
  important: { title: 'Important', key: 'important_emails' },
  meetings:  { title: 'Meetings',  key: 'meeting_items' },
  sent:      { title: 'Sent',      key: 'sent_emails' },
  drafts:    { title: 'Drafts',    key: 'drafts_emails' },
  spam:      { title: 'Spam',      key: 'spam_emails' },
  trash:     { title: 'Trash',     key: 'trash_emails' },
};
const emailSort = Object.fromEntries(Object.keys(SPLIT_PANELS).map(panel => [panel, {
  key: 'date', ascending: false,
}]));
const EMAIL_SORT_OPTIONS = [
  ['date', 'mail.sort.date'], ['sender', 'mail.sort.sender'], ['subject', 'mail.sort.subject'],
  ['read', 'mail.sort.read'], ['priority', 'mail.sort.priority'], ['project', 'mail.sort.project'],
];

function setupSplitPanels() {
  const tpl = document.getElementById('tpl-split-panel');
  for (const [panelId, meta] of Object.entries(SPLIT_PANELS)) {
    const section = document.getElementById(`panel-${panelId}`);
    const clone = tpl.content.cloneNode(true);
    const title = clone.querySelector('.list-title');
    title.dataset.i18n = `nav.${panelId}`;
    title.textContent = t(title.dataset.i18n);
    clone.querySelector('.list-sort-label').textContent = t('mail.sortBy');
    const sortButton = clone.querySelector('.list-sort-button');
    const sortMenu = clone.querySelector('.list-sort-menu');
    sortButton.addEventListener('click', e => {
      e.stopPropagation();
      document.querySelectorAll('.list-sort-menu').forEach(menu => {
        if (menu !== sortMenu) menu.classList.add('hidden');
      });
      sortMenu.classList.toggle('hidden');
    });
    sortMenu.addEventListener('click', e => {
      const option = e.target.closest('.list-sort-option');
      if (!option) return;
      emailSort[panelId].key = option.dataset.sort;
      emailSort[panelId].ascending = option.dataset.sort !== 'date';
      sortMenu.classList.add('hidden');
      renderSplitPanel(panelId, meta);
    });
    clone.querySelector('.list-sort-direction').addEventListener('click', () => {
      emailSort[panelId].ascending = !emailSort[panelId].ascending;
      renderSplitPanel(panelId, meta);
    });
    section.appendChild(clone);
  }
  document.addEventListener('click', () =>
    document.querySelectorAll('.list-sort-menu').forEach(menu => menu.classList.add('hidden')));
}

// ── Render all ─────────────────────────────────────────────────────────────
function renderAll() {
  renderOverview();
  renderPanel(activePanel);
}

function renderPanel(panel) {
  if (panel === 'overview') { renderOverview(); return; }
  if (panel === 'calendar') { renderCalendar(); return; }
  if (panel === 'projects') { renderProjects(); return; }
  if (panel === 'tasks')    { renderKanban(); return; }
  if (panel === 'discussions') { renderDiscussions(); return; }
  const meta = SPLIT_PANELS[panel];
  if (meta && appState) renderSplitPanel(panel, meta);
}

// ── Discussion mode ────────────────────────────────────────────────────────
// The mailbox like a messaging app: the folders give way to one list of discussions.
const DISCUSSION_HIDDEN_PANELS = ['inbox', 'sent', 'drafts', 'trash', 'spam'];
let openDiscussionId = null;
let replyDraft = null;   // the reply bar: { id, email, subject, from, to, cc, bcc, text, more, adding }
const REPLY_PILLS = 4;   // bubbles shown before "+X" (a fixed count, not measured to the bar width)
const dmFind = { open: false, query: '', at: -1 };   // the Ctrl+F bar of the open discussion
// Brutalist dresses Discussion mode as a post office: postcards, stamps, a mailbox (and some wording).
const postOffice = () => document.documentElement.dataset.theme === 'brutalist';

function applyDiscussionMode(on) {
  document.body.classList.toggle('discussion-mode', on);
  if (on && DISCUSSION_HIDDEN_PANELS.includes(activePanel)) switchPanel('discussions');
  else if (!on && activePanel === 'discussions') switchPanel('inbox');
}

// Built again only when what it is made of changes: the lists are replaced on every change,
// except read marks, which setReadAll changes in place (and counts in `discussionReads`).
let discussionsBuilt = { from: [], list: [] };
let discussionReads = 0;
function discussionList() {
  const from = [appState.all_emails, appState.sent_emails, appState.trash_emails, appState.spam_emails,
    appState.drafts_emails, appState.tasks, composeAccounts, discussionReads];
  if (from.some((part, i) => part !== discussionsBuilt.from[i])) {
    const me = composeAccounts.map(a => splitSender(a.from).email);   // index = account
    discussionsBuilt = {
      from,
      list: CourrielCore.buildDiscussions({
        inbox: appState.all_emails, sent: appState.sent_emails,
        trash: appState.trash_emails, spam: appState.spam_emails, drafts: appState.drafts_emails,
      }, { me, now: Date.now(), signatures: composeAccounts.map(a => htmlText(a.signature_html)), tasks: appState.tasks }),
    };
  }
  return discussionsBuilt.list;
}

// The emails of the open discussion ("folder:uid"): its id changes when its oldest email goes
// or an older one arrives, and the discussion holding one of them is still the open one.
let openDiscussionEmails = new Set();
const dmEmailKey = i => `${i.folder}:${i.email.uid}`;
function findOpenDiscussion(list) {
  const d = list.find(d => d.id === openDiscussionId)
    || (openDiscussionId && list.find(d => d.items.some(i => i.email && openDiscussionEmails.has(dmEmailKey(i)))));
  if (d && d.id !== openDiscussionId) {
    if (replyDraft?.id === openDiscussionId) replyDraft.id = d.id;
    if (shownDiscussionId === openDiscussionId) shownDiscussionId = d.id;
    openDiscussionId = d.id;
  }
  openDiscussionEmails = new Set(d ? d.items.filter(i => i.email).map(dmEmailKey) : []);
  return d;
}

// Opening a discussion reads it: every unread email, on the server too.
// `landOn` ({ folder, uid }) scrolls to that message and highlights it instead of the newest one.
let dmLandOn = null;
function openDiscussion(d, landOn = null) {
  openDiscussionId = d.id;
  dmLandOn = landOn;
  dmFind.at = -1;
  setReadAll(new Set(d.items.filter(i => i.email && i.kind !== 'draft').map(i => i.email)), true);
  switchPanel('discussions');
}

// A search result in discussion mode: its discussion, scrolled to it. False if it is in none.
function openHitInDiscussion(h) {
  const isHit = i => i.email && i.kind !== 'draft' && i.folder === h.folder && i.email.uid === h.uid;
  const d = discussionList().find(d => d.items.some(isHit));
  if (d) openDiscussion(d, { folder: h.folder, uid: h.uid });
  return !!d;
}

function renderDiscussions() {
  if (!appState) return;
  const open = findOpenDiscussion(discussionList());
  renderDiscussionLines();
  document.getElementById('dm-new').textContent = t(postOffice() ? 'dm.newCard' : 'dm.new');
  document.getElementById('dm-search').placeholder = t(postOffice() ? 'dm.searchMail' : 'dm.search');
  renderDiscussion(open);
}

// The list lines, only the discussions the search bar above them finds.
function renderDiscussionLines() {
  const list = CourrielCore.filterDiscussions(discussionList(), document.getElementById('dm-search').value);
  const lines = document.getElementById('dm-lines');
  lines.innerHTML = list.map(d => `
    <div class="dm-line${d.id === openDiscussionId ? ' active' : ''}" data-id="${esc(d.id)}">
      ${d.priority ? `<span class="dm-prio dm-prio-${esc(d.priority)}"></span>` : ''}
      <span class="dm-who">${dmUrgent(d)}<span class="dm-who-text">${esc(d.heading)}</span>${dmMarks(d)}</span>
      <span class="dm-when">${esc(fmtShort(d.date_ts))}</span>
      <span class="dm-sub">
        <span class="dm-people" data-people="${d.people.length}">${peopleLabels(d.people)}</span>
        ${d.standing ? `<span class="dm-standing" title="${esc(t('dm.standing'))}">${esc(d.standing)}</span>` : ''}
      </span>
      <span class="dm-last">${esc(d.preview.mine ? t('dm.you', { text: d.preview.text }) : d.preview.text)}</span>
      ${d.unread ? `<span class="dm-badge">${esc(postOffice() ? tn('dm.unread', d.unread) : fmtNumber(d.unread))}</span>` : ''}
    </div>`).join('');
  lines.querySelectorAll('.dm-line').forEach((line, i) => line.addEventListener('click', () => openDiscussion(list[i])));
  fitPeopleLabels(lines);
}
document.getElementById('dm-search').addEventListener('input', () => { if (appState) renderDiscussionLines(); });

let shownDiscussionId = null;   // the discussion #dm-conv shows
function renderDiscussion(d) {
  const conv = document.getElementById('dm-conv');
  // A reply being written survives refreshes of the same discussion.
  const reply = d && replyDraft?.id === d.id && replyDraft.text ? conv.querySelector('.dm-reply') : null;
  // So does the reading position, unless it was at the newest message.
  const old = d && shownDiscussionId === d.id ? conv.querySelector('.dm-scroll') : null;
  const keepTop = old && old.scrollHeight - old.scrollTop - old.clientHeight >= 4 ? old.scrollTop : null;
  shownDiscussionId = d?.id ?? null;
  renderDiscussionInfo(d);
  if (!d) { conv.innerHTML = ''; replyDraft = null; return; }
  const finding = document.activeElement?.classList.contains('dm-find-input');
  const inbox = dmEmails(d, ['inbox']);
  const html = `
    <div class="dm-head">
      ${avatarStack(d.people)}
      <div class="dm-titles">
        <div class="dm-subject">${esc(d.heading)}</div>
        <div class="dm-people" data-people="${d.people.length}">${peopleLabels(d.people)}</div>
      </div>
      <div class="dm-acts">
        ${inbox.length ? dmActButton('read', d.unread ? 'rp.markRead' : 'rp.markUnread', d.unread ? 'read' : 'unread') : ''}
        ${dmActButton('archive', 'dm.archive', 'archive')}
        ${dmActButton('delete', 'rp.delete', 'delete')}
        ${dmActButton('info', 'dm.details', 'info', dmInfoOpen)}
      </div>
    </div>
    <div class="dm-find${dmFind.open ? '' : ' hidden'}">
      <input class="dm-find-input" type="text" placeholder="${esc(t('dm.find'))}" value="${esc(dmFind.query)}" />
      <span class="dm-find-count"></span>
      <button class="dm-find-prev" title="${esc(t('dm.findPrev'))}">↑</button>
      <button class="dm-find-next" title="${esc(t('dm.findNext'))}">↓</button>
      <button class="dm-find-close" title="${esc(t('cmp.close'))}">✕</button>
    </div>
    <div class="dm-scroll">${d.items.map(discussionItem).join('')}</div>`;
  if (reply) {   // kept in place, so the field keeps the focus
    conv.querySelectorAll('.dm-head, .dm-find, .dm-scroll').forEach(el => el.remove());
    reply.insertAdjacentHTML('beforebegin', html);
  } else {
    conv.innerHTML = html;
    conv.appendChild(replyBar(d));
  }
  fitPeopleLabels(conv.querySelector('.dm-head'));
  const scroll = conv.querySelector('.dm-scroll');
  scroll.scrollTop = keepTop ?? scroll.scrollHeight;
  scroll.addEventListener('click', ev => {   // on the scroll: the find bar rebuilds the bubbles
    const go = ev.target.closest('.dm-task-go');
    if (go) { openTask(Number(go.closest('.dm-task').dataset.task)); return; }
    const status = ev.target.closest('.dm-task-status button');
    if (status) {
      if (!status.classList.contains('on')) setTaskStatus(Number(status.closest('.dm-task').dataset.task), status.dataset.status).catch(e => showToast(String(e)));
      return;
    }
    const del = ev.target.closest('.dm-task-del');
    if (del) { deleteTask(Number(del.closest('.dm-task').dataset.task)).catch(e => showToast(String(e))); return; }
    const a = ev.target.closest('a[data-url]');
    if (!a) return;
    ev.preventDefault();
    window.__TAURI__.shell.open(a.dataset.url).catch(err => console.error('open link', err));
  });
  scroll.addEventListener('contextmenu', ev => {
    const item = dmItemOf(d, ev.target.closest('.dm-row[data-uid]'));
    if (!item) return;
    ev.preventDefault();
    messageMenu(d, item, ev.clientX, ev.clientY);
  });
  conv.querySelectorAll('.dm-act').forEach(btn => btn.addEventListener('click', () => discussionAction(d, btn.dataset.dm)));
  setupDiscussionFind(conv);
  if (dmFind.open) runDiscussionFind(0, !dmLandOn);
  if (finding) conv.querySelector('.dm-find-input').focus();
  const landed = dmLandOn && conv.querySelector(`.dm-row[data-folder="${dmLandOn.folder}"][data-uid="${dmLandOn.uid}"]`);
  dmLandOn = null;
  if (landed) {
    landed.classList.add('dm-landed');
    landed.scrollIntoView({ block: 'center' });
  }
}

// A bubble's text as written: **bold**, *italic* and [links](url); list lines stay as they are.
function bubbleHtml(text) {
  return esc(text)
    .replace(/\[\[([^\]\n]+)\]\]\(((?:https?:|mailto:)[^)\s]+)\)/g, '<a href="#" class="dm-btn" data-url="$2">$1</a>')
    .replace(/\[([^\]\n]+)\]\(((?:https?:|mailto:)[^)\s]+)\)/g, '<a href="#" data-url="$2">$1</a>')
    .replace(/\*\*(?=\S)([^*\n]+?)\*\*/g, '<b>$1</b>')
    .replace(/(^|[^*\w])\*(?=\S)([^*\n]+?)\*(?![*\w])/gm, '$1<i>$2</i>');
}

// The text of an HTML snippet (an account's signature), one line per line break or block.
function htmlText(html) {
  const doc = new DOMParser().parseFromString(String(html || '').replace(/<br\s*\/?>|<\/(?:p|div|li|tr|h\d)>/gi, '$&\n'), 'text/html');
  return doc.body.textContent || '';
}

// Ctrl+F: find in the open discussion, jumping between matches (Enter / Shift+Enter, ↓ / ↑).
document.addEventListener('keydown', e => {
  if (!(e.ctrlKey || e.metaKey) || e.key.toLowerCase() !== 'f' || activePanel !== 'discussions') return;
  const input = document.querySelector('#dm-conv .dm-find-input');
  if (!input) return;
  e.preventDefault();
  dmFind.open = true;
  input.parentElement.classList.remove('hidden');
  input.focus();
  input.select();
  runDiscussionFind(0);
});

function setupDiscussionFind(conv) {
  const input = conv.querySelector('.dm-find-input');
  input.addEventListener('input', () => { dmFind.query = input.value; dmFind.at = -1; runDiscussionFind(0); });
  input.addEventListener('keydown', e => {
    if (e.key === 'Enter') { e.preventDefault(); runDiscussionFind(e.shiftKey ? -1 : 1); }
    else if (e.key === 'Escape') { e.preventDefault(); closeDiscussionFind(); }
  });
  conv.querySelector('.dm-find-prev').addEventListener('click', () => runDiscussionFind(-1));
  conv.querySelector('.dm-find-next').addEventListener('click', () => runDiscussionFind(1));
  conv.querySelector('.dm-find-close').addEventListener('click', closeDiscussionFind);
}

function closeDiscussionFind() {
  dmFind.open = false;
  document.querySelector('#dm-conv .dm-find')?.classList.add('hidden');
  runDiscussionFind(0);
}

// Marks every match in the bubbles; the current one starts at the newest and is scrolled to.
// Marks go inside the bubble's text pieces, so its bold, italic and links stay.
const dmBubbleHtml = new WeakMap();   // bubble → its HTML before any mark
function runDiscussionFind(move, scrollTo = true) {
  const conv = document.getElementById('dm-conv');
  const q = dmFind.open ? dmFind.query.trim() : '';
  conv.querySelectorAll('.dm-bubble').forEach(b => {
    if (!dmBubbleHtml.has(b)) dmBubbleHtml.set(b, b.innerHTML);
    b.innerHTML = dmBubbleHtml.get(b);
    if (!q) return;
    const walker = document.createTreeWalker(b, NodeFilter.SHOW_TEXT);
    const pieces = [];
    while (walker.nextNode()) pieces.push(walker.currentNode);
    for (const piece of pieces) {
      const html = highlight(piece.textContent, [q]);
      if (!html.includes('<mark')) continue;
      const marked = document.createElement('template');
      marked.innerHTML = html;
      piece.replaceWith(marked.content);
    }
  });
  const count = conv.querySelector('.dm-find-count');
  const marks = [...conv.querySelectorAll('.dm-bubble mark')];
  if (!marks.length) { if (count) count.textContent = q ? t('dm.findNone') : ''; return; }
  if (dmFind.at < 0 || dmFind.at >= marks.length) dmFind.at = marks.length - 1;
  else dmFind.at = (dmFind.at + move + marks.length) % marks.length;
  marks[dmFind.at].classList.add('current');
  if (scrollTo) marks[dmFind.at].scrollIntoView({ block: 'center' });
  count.textContent = t('dm.findCount', { n: fmtNumber(dmFind.at + 1), total: fmtNumber(marks.length) });
}

// ── Icons, attachments and drop zones (#42, #43, #44) ───────────────────
// Inline SVG icons: they take the colour and the size of the button that holds them, so
// every theme paints them like their neighbours.
const ICON = {
  expand: '<svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="M15 3h6v6"/><path d="M9 21H3v-6"/><path d="M21 3l-7 7"/><path d="M3 21l7-7"/></svg>',
  clip: '<svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48"/></svg>',
};

// An attachment is { name, size, path }: a file on disk, or the zip the backend built from
// a folder. The editor and the reply bar each keep a list of them.
const attachListHtml = items => items.map((a, i) =>
  `<span class="attach-item" title="${esc(a.path)}">${ICON.clip}`
  + `<span class="attach-name">${esc(a.name)}</span>`
  + `<span class="attach-size">${esc(fmtBytes(a.size))}</span>`
  + `<button class="attach-x" data-at="${i}" title="${esc(t('dm.remove'))}">✕</button></span>`).join('');

// One place that can hold attachments: the composer, or a discussion's reply bar.
function attachTarget(list, items, onError, onChange) {
  const show = () => {
    list.classList.toggle('hidden', !items.length);
    list.innerHTML = attachListHtml(items);
  };
  list.addEventListener('click', e => {
    const x = e.target.closest('.attach-x');
    if (!x) return;
    items.splice(Number(x.dataset.at), 1);
    show();
    if (onChange) onChange();
  });
  const add = got => { items.push(...got); show(); if (onChange) onChange(); };
  const fail = err => onError(t('cmp.attachFailed', { err: String(err) }));
  show();
  return {
    items,
    show,
    clear: () => { items.length = 0; show(); },
    // The OS picker takes files or a folder, never both at once: hence the small menu.
    async pick(kind) {
      try {
        const paths = await pickedPaths(kind);
        if (paths.length) add(await cmd('prepare_attachments', { paths }));
      } catch (err) { fail(err); }
    },
    async drop(e, skip) {
      try {
        const dropped = await droppedItems(e, skip);
        if (dropped.length) add(await cmd('save_dropped_attachments', { items: dropped }));
      } catch (err) { fail(err); }
    },
  };
}

async function pickedPaths(kind) {
  const dialog = window.__TAURI__.dialog;
  if (kind === 'folder') {
    const one = await dialog.open({ title: t('cmp.attachFolder'), directory: true });
    return one ? [one] : [];
  }
  const many = await dialog.open({ title: t('cmp.attachFiles'), multiple: true });
  return Array.isArray(many) ? many.filter(Boolean) : (many ? [many] : []);
}

// "Files… / Folder…", just under the paperclip that opened it.
let attachMenu = null;

function closeAttachMenu() {
  if (!attachMenu) return;
  attachMenu.el.remove();
  document.removeEventListener('mousedown', attachMenu.onDown, true);
  document.removeEventListener('keydown', attachMenu.onKey, true);
  attachMenu = null;
}

function openAttachMenu(anchor, target) {
  const reopening = attachMenu && attachMenu.anchor === anchor;
  closeAttachMenu();
  if (reopening) return;
  const el = document.createElement('div');
  el.className = anchor.closest('.dm-reply') ? 'attach-menu dm-menu' : 'attach-menu';
  el.innerHTML = `<button type="button" data-kind="files">${esc(t('cmp.attachFiles'))}</button>
    <button type="button" data-kind="folder">${esc(t('cmp.attachFolder'))}</button>`;
  document.body.appendChild(el);
  const box = anchor.getBoundingClientRect();
  const under = box.bottom + el.offsetHeight + 10 < window.innerHeight;
  el.style.left = Math.max(8, Math.min(box.left, window.innerWidth - el.offsetWidth - 8)) + 'px';
  el.style.top = `${(under ? box.bottom + 4 : box.top - el.offsetHeight - 4)}px`;
  el.addEventListener('click', e => {
    const btn = e.target.closest('button[data-kind]');
    if (!btn) return;
    const kind = btn.dataset.kind;
    closeAttachMenu();
    target.pick(kind);
  });
  const onDown = e => {
    if (!el.contains(e.target) && !anchor.contains(e.target)) closeAttachMenu();
  };
  const onKey = e => { if (e.key === 'Escape') closeAttachMenu(); };
  document.addEventListener('mousedown', onDown, true);
  document.addEventListener('keydown', onKey, true);
  attachMenu = { el, anchor, onDown, onKey };
}

// #44: files and folders can be dropped on the editor or on the reply bar. The zone shows a
// sign while something is dragged over it, and hides it again on leave or drop.
function setupDropZone(zone, target, skipFor) {
  // The editor already carries its sign in the HTML shell; the reply bar gets one here.
  let overlay = zone.querySelector(':scope > .drop-overlay');
  if (!overlay) {
    overlay = document.createElement('div');
    overlay.className = 'drop-overlay';
    overlay.innerHTML = `<span class="drop-box">${esc(t('cmp.dropHere'))}</span>`;
    zone.appendChild(overlay);
  }
  let depth = 0;
  const carriesFiles = e => Array.from(e.dataTransfer?.types || []).includes('Files');
  zone.addEventListener('dragenter', e => {
    if (!carriesFiles(e)) return;
    e.preventDefault();
    depth++;
    zone.classList.add('dropping');
  });
  zone.addEventListener('dragover', e => {
    if (!carriesFiles(e)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
  });
  zone.addEventListener('dragleave', () => {
    if (--depth <= 0) { depth = 0; zone.classList.remove('dropping'); }
  });
  zone.addEventListener('drop', e => {
    if (!carriesFiles(e)) return;
    e.preventDefault();
    depth = 0;
    zone.classList.remove('dropping');
    target.drop(e, skipFor ? skipFor(e) : null);
  });
}

// What was dropped: a file keeps its name, a folder brings every file inside it with its
// path within the folder. The webview gives no disk path for a dropped file, so the bytes
// travel base64-encoded and the backend writes them (or zips the folder).
async function droppedItems(e, skip) {
  const empty = [];
  const items = [];
  for (const entry of droppedEntries(e.dataTransfer)) {
    const files = await entryFiles(entry, '');
    if (!files.length) { if (entry.isDirectory) empty.push(entry.name); continue; }
    items.push({ name: entry.name, dir: !!entry.isDirectory, files });
  }
  if (empty.length) showToast(t('cmp.attachEmpty', { name: empty.join(', ') }));
  return skip ? items.filter(it => !skip(it)) : items;
}

function droppedEntries(dt) {
  const items = [...(dt?.items || [])].filter(i => i.kind === 'file');
  const entries = items.map(i => (typeof i.webkitGetAsEntry === 'function' ? i.webkitGetAsEntry() : null));
  if (entries.length && entries.every(Boolean)) return entries;
  // Without the entry API a folder cannot be walked: keep the files that came through.
  return [...(dt?.files || [])].map(f => ({ name: f.name, isFile: true, isDirectory: false, _file: f }));
}

async function entryFiles(entry, base) {
  if (entry.isDirectory) {
    const out = [];
    for (const child of await readFolder(entry)) {
      out.push(...await entryFiles(child, base + entry.name + '/'));
    }
    return out;
  }
  const file = entry._file || await new Promise((res, rej) => entry.file(res, rej));
  return [{ path: base + entry.name, data: await readAsDataUrl(file) }];
}

// readEntries hands back a batch at a time (about a hundred), so ask until it answers none.
async function readFolder(dirEntry) {
  const reader = dirEntry.createReader();
  const all = [];
  for (;;) {
    const batch = await new Promise((res, rej) => reader.readEntries(res, rej));
    if (!batch.length) return all;
    all.push(...batch);
  }
}

const readAsDataUrl = file => new Promise((res, rej) => {
  const reader = new FileReader();
  reader.onload = () => res(String(reader.result));   // the backend keeps the payload only
  reader.onerror = () => rej(reader.error || new Error(file.name));
  reader.readAsDataURL(file);
});

const isImageName = name => /\.(png|jpe?g|gif|webp|bmp|svg|heic|tiff?)$/i.test(name);

// ── Discussion mode: the reply bar ──
// Reply-all to the latest message. The From/To/Cc bubbles appear once something is typed;
// Send goes through the usual send path, the expand icon moves the reply to the full editor.
const replyAttachments = [];   // { name, size, path }, sent with the reply
let replyAttach = null;        // the open bar's attach target

function replyBar(d) {
  replyAttachments.length = 0;
  replyDraft = {
    id: d.id, email: d.reply.email, subject: d.reply.subject, from: d.reply.from,
    to: [...d.reply.to], cc: [...d.reply.cc], bcc: [], text: '', more: false, adding: false,
  };
  const bar = document.createElement('footer');
  bar.className = 'dm-reply';
  bar.innerHTML = `
    <div class="dm-recips hidden"></div>
    <div class="dm-compose">
      <textarea class="dm-text" rows="1" placeholder="${esc(t(postOffice() ? 'dm.replyCard' : 'dm.reply'))}"></textarea>
      <button class="dm-attach" title="${esc(t('cmp.attach'))}">${ICON.clip}</button>
      <button class="dm-expand" title="${esc(t('cmp.expand'))}">${ICON.expand}</button>
      <button class="dm-send" title="${esc(t('cmp.sendTitle'))}">${esc(t('cmp.send'))}</button>
    </div>
    <div class="attach-list dm-attach-list hidden"></div>`;
  const text = bar.querySelector('.dm-text');
  text.addEventListener('input', () => {
    replyDraft.text = text.value;
    if (text.value.trim()) bar.querySelector('.dm-recips').classList.remove('hidden');
  });
  text.addEventListener('keydown', e => { if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); sendReply(bar); } });
  bar.querySelector('.dm-send').addEventListener('click', () => sendReply(bar));
  replyAttach = attachTarget(bar.querySelector('.dm-attach-list'), replyAttachments, msg => showToast(msg));
  const clip = bar.querySelector('.dm-attach');
  clip.addEventListener('click', () => openAttachMenu(clip, replyAttach));
  setupDropZone(bar, replyAttach);
  bar.querySelector('.dm-expand').addEventListener('click', () => {
    const r = replyDraft;
    openCompose({
      mode: 'reply', email: r.email, subject: r.subject, account: r.from, html: replyHtml(r.text),
      to: addressList(r.to), cc: addressList(r.cc), bcc: addressList(r.bcc),
      attachments: replyAttachments.map(a => ({ ...a })),
    });
    text.value = r.text = '';
    replyAttach.clear();   // they went to the editor with the text
  });
  bar.querySelector('.dm-recips').addEventListener('click', e => replyRecipientClick(bar, e));
  bar.querySelector('.dm-recips').addEventListener('change', e => {
    if (e.target.matches('.dm-from select')) replyDraft.from = Number(e.target.value) || 0;
  });
  renderReplyRecipients(bar);
  return bar;
}

function renderReplyRecipients(bar) {
  const r = replyDraft;
  const all = ['to', 'cc', 'bcc'].flatMap(kind => r[kind].map(p => ({ kind, p })));
  const shown = r.more ? all : all.slice(0, REPLY_PILLS);
  const pill = ({ kind, p }) => `
    <span class="dm-pill" title="${esc(p.address)}">${esc(p.label)}<button class="dm-x" data-kind="${kind}" data-address="${esc(p.address)}" title="${esc(t('dm.remove'))}">✕</button></span>`;
  bar.querySelector('.dm-recips').innerHTML = `
    <button class="dm-add" title="${esc(t('dm.add'))}">${postOffice() ? esc(t('dm.addCard')) : '+'}</button>
    ${composeAccounts.length > 1 ? `<label class="dm-pill dm-from">${esc(t('meta.from'))}
      <select>${composeAccounts.map((a, i) => `<option value="${i}"${i === r.from ? ' selected' : ''}>${esc(a.from)}</option>`).join('')}</select>
    </label>` : ''}
    ${['to', 'cc', 'bcc'].map(kind => {
      const pills = shown.filter(x => x.kind === kind);
      return pills.length ? `<span class="dm-grp"><span class="dm-lbl">${esc(t(`cmp.${kind}`))}</span>${pills.map(pill).join('')}</span>` : '';
    }).join('')}
    ${all.length > shown.length ? `<button class="dm-pill dm-more" title="${esc(all.slice(shown.length).map(x => x.p.address).join(', '))}">+${all.length - shown.length}</button>` : ''}
    ${r.adding ? `<span class="dm-adding">
      <select class="dm-kind">${['to', 'cc', 'bcc'].map(k => `<option value="${k}">${esc(t(`cmp.${k}`))}</option>`).join('')}</select>
      <input class="dm-address" type="text" autocomplete="off" spellcheck="false">
    </span>` : ''}`;
  const input = bar.querySelector('.dm-address');
  if (!input) return;
  input.focus();
  input.addEventListener('keydown', e => {
    if (e.key === 'Escape') { r.adding = false; renderReplyRecipients(bar); }
    if (e.key !== 'Enter') return;
    e.preventDefault();
    const kind = bar.querySelector('.dm-kind').value;
    const known = new Set(all.map(x => x.p.address.toLowerCase()));
    for (const a of CourrielCore.splitAddresses(input.value)) {
      if (!known.has(a.address.toLowerCase())) { r[kind].push(a); known.add(a.address.toLowerCase()); }
    }
    r.adding = false;
    CourrielCore.bubbleLabels([...r.to, ...r.cc, ...r.bcc]);
    renderReplyRecipients(bar);
  });
}

function replyRecipientClick(bar, e) {
  const r = replyDraft;
  const x = e.target.closest('.dm-x');
  if (x) {
    r[x.dataset.kind] = r[x.dataset.kind].filter(p => p.address !== x.dataset.address);
    CourrielCore.bubbleLabels([...r.to, ...r.cc, ...r.bcc]);
  } else if (e.target.closest('.dm-more')) r.more = true;
  else if (e.target.closest('.dm-add')) r.adding = !r.adding;
  else return;
  renderReplyRecipients(bar);
}

const replyHtml = text => `<div>${esc(text.trim()).replace(/\n/g, '<br>') || '<br>'}</div><div><br></div>`;
const addressList = people => people
  .map(p => (p.name ? `"${p.name.replace(/["\\]/g, '')}" <${p.address}>` : p.address)).join(', ');

async function sendReply(bar) {
  const r = replyDraft;
  if (!r.text.trim()) return;
  if (!r.to.length && !r.cc.length && !r.bcc.length) { showToast(t('cmp.needRecipient')); return; }
  const btn = bar.querySelector('.dm-send');
  btn.disabled = true;
  try {
    await cmd('send_email', {
      to: addressList(r.to), cc: addressList(r.cc), bcc: addressList(r.bcc), subject: r.subject,
      html: replyHtml(r.text) + signatureBlock(r.from, 'reply') + replyQuote(r.email),
      inReplyTo: r.email.message_id || null, account: r.from,
      attachments: replyAttachments.map(a => a.path),
    });
    showToast(t('cmp.sent'));
    bar.querySelector('.dm-text').value = r.text = '';
    if (replyAttach) replyAttach.clear();
  } catch (e) {
    showToast(t('cmp.sendFailed', { err: e }));
  }
  btn.disabled = false;
}

// One item of a discussion: a note, a day separator or a bubble (name once and avatar once per run).
function discussionItem(item) {
  const time = ts => new Date(ts * 1000);
  if (item.kind === 'earlier') return `<div class="dm-earlier">${esc(t('dm.earlier'))}</div>`;
  if (item.kind === 'day') {
    const day = item.day === 'today' ? t('dm.today')
      : item.day === 'weekday' ? fmt({ weekday: 'long' }).format(time(item.date_ts))
      : fmtDay(item.date_ts);
    return `<div class="dm-sep">${esc(day)}</div>`;
  }
  if (item.kind === 'people') {
    const names = list => new Intl.ListFormat(langInfo().locale, { type: 'conjunction' })
      .format(list.map(p => (p.cc ? t('dm.cc', { name: p.short }) : p.short)));
    const who = item.by.short;
    const lines = [
      item.added.length && t(item.mine ? 'dm.youAdded' : 'dm.added', { who, names: names(item.added) }),
      item.removed.length && t(item.mine ? 'dm.youRemoved' : 'dm.removed', { who, names: names(item.removed) }),
    ].filter(Boolean);
    return lines.map(line => `<div class="dm-people-note">${esc(line)}</div>`).join('');
  }
  if (item.kind === 'draft') {
    return `
    <div class="dm-row me first last draft">
      <div class="dm-av-slot"></div>
      <div class="dm-msg">
        <div class="dm-band"><small>${esc(t('dm.notSent'))}</small></div>
        <span class="dm-postmark">${esc(t('dm.draft'))}</span>
        <div class="dm-bubble">${esc(item.text)}</div>
        <div class="dm-time">${esc(t('dm.draft'))}</div>
      </div>
    </div>`;
  }
  // Images and files stay out of .dm-bubble: the find bar marks text inside bubbles.
  const content = item.kind === 'message' ? `<div class="dm-bubble">${bubbleHtml(item.text)}</div>`
    : item.kind === 'image' ? `<img class="dm-image" src="${esc(item.src)}" alt="" />`
    : item.kind === 'attachment' ? `<div class="dm-file"><span class="dm-file-icon">📎</span>`
      + `<span class="dm-file-name">${esc(item.name || t('dm.file'))}</span><span class="dm-file-size">${esc(fmtBytes(item.size))}</span></div>`
    : null;
  if (!content) return '';
  const s = item.sender;
  return `
    <div class="dm-row${item.mine ? ' me' : ''}${item.first ? ' first' : ''}${item.last ? ' last' : ''}" style="--pc:${s.colour}" data-folder="${esc(item.folder)}" data-uid="${esc(item.email.uid)}">
      <div class="dm-av-slot">${!item.mine && item.last ? avatar(s) : ''}</div>
      <div class="dm-msg">
        ${item.first ? postcardTop(s, time(item.date_ts)) : ''}
        <div class="dm-name">${esc(s.name || s.address)}</div>
        ${content}
        ${item.task ? taskBox(item.task) : ''}
        <div class="dm-time">${esc(fmt({ hour: '2-digit', minute: '2-digit' }).format(time(item.date_ts)))}</div>
      </div>
    </div>`;
}

// The task taken from a message: its status, set from here, and the way to it on the board.
// Not done is the board's Todo; Given up stays the board's own, shown read-only when the task is there.
function taskBox(task) {
  const labels = { todo: t('dm.notDone'), in_progress: t('kanban.inProgress'), done: t('kanban.done') };
  return `<div class="dm-task" data-task="${esc(task.id)}">`
    + `<span class="dm-task-title">✓ ${esc(task.title)}</span>`
    + (task.status === 'given_up' ? `<span class="dm-task-given-up">${esc(t('kanban.givenUp'))}</span>` : '')
    + `<span class="dm-task-status" role="group" aria-label="${esc(t('dm.taskStatus'))}">`
    + Object.entries(labels).map(([s, label]) => `<button data-status="${s}"${s === task.status ? ' class="on" aria-pressed="true"' : ' aria-pressed="false"'}>${esc(label)}</button>`).join('')
    + `</span><button class="dm-task-go">${esc(t('dm.openTask'))}</button>`
    + `<button class="dm-task-del" title="${esc(t('dm.deleteTask'))}" aria-label="${esc(t('dm.deleteTask'))}">🗑</button></div>`;
}

// Same command as the board's delete; the board and the discussion both redraw from setTasks.
async function deleteTask(id) {
  if (await askInApp(t('dm.deleteTaskAsk'), t('rp.delete'))) setTasks(await cmd('delete_task', { id }));
}

// The top of a postcard (only the Brutalist look shows it): "Greetings from ORG", the stamp, the postmark.
function postcardTop(s, date) {
  return `<div class="dm-band"><small>${esc(t('dm.greetings'))}</small> <b>${esc(s.org || s.name || s.address)}</b></div>`
    + `<span class="dm-stamp">${esc(s.initials)}</span>`
    + `<span class="dm-postmark"><span>${esc(fmt({ day: 'numeric', month: 'short' }).format(date))}</span>`
    + `<span>${esc(fmt({ hour: '2-digit', minute: '2-digit' }).format(date))}</span></span>`;
}

// A file size the way the language writes it: 12 B, 51 kB, 1.2 MB.
function fmtBytes(n) {
  const [unit, value] = n >= 1e6 ? ['megabyte', n / 1e6] : n >= 1e3 ? ['kilobyte', n / 1e3] : ['byte', n];
  return new Intl.NumberFormat(langInfo().locale, { style: 'unit', unit, unitDisplay: 'short', maximumFractionDigits: value < 10 ? 1 : 0 }).format(value);
}

const avatar = p => `<span class="dm-av" style="--pc:${p.colour}" title="${esc(p.name || p.address)}">${esc(p.initials)}</span>`;
const avatarStack = people => `<span class="dm-av-stack">${people.slice(0, 3).map(avatar).join('')}</span>`;

// The whole-discussion buttons of the header: a line icon (drawn in the text colour, so every
// theme recolours it) and the action's name.
const DM_ACT_ICONS = {
  read: '<path d="M4 12.5l5 5L20 6.5"/>',
  unread: '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M3 7l9 6 9-6"/><circle cx="19" cy="5" r="3" fill="currentColor"/>',
  archive: '<rect x="3" y="4" width="18" height="5" rx="1"/><path d="M5 9v10h14V9M10 13h4"/>',
  delete: '<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 11v6M14 11v6"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.5v.5"/>',
};
const dmActButton = (action, labelKey, icon, on = false) => `
  <button class="dm-act${on ? ' on' : ''}" data-dm="${action}" title="${esc(t(labelKey))}">
    <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${DM_ACT_ICONS[icon]}</svg>
    <span class="dm-act-label">${esc(t(labelKey))}</span>
  </button>`;

// The marks a list line shows at a glance, as small line icons drawn in the text colour (so every
// theme recolours them): the AI's urgent mark at the start of the line, then a reply owed, the
// files and the tasks the discussion holds after its subject.
const DM_MARK_ICONS = {
  urgent: '<path d="M12 4.5L21 19H3z"/><path d="M12 10v4.5M12 17v.5"/>',
  reply: '<path d="M9 15l-5-5 5-5"/><path d="M4 10h10a6 6 0 0 1 6 6v3"/>',
  files: '<path d="M20.5 11.5l-8.2 8.2a5.5 5.5 0 0 1-7.8-7.8l8.2-8.2a3.7 3.7 0 0 1 5.2 5.2l-8.2 8.2a1.8 1.8 0 0 1-2.6-2.6l7.6-7.6"/>',
  tasks: '<path d="M9 11l3 3L21 5"/><path d="M20 12v6a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h11"/>',
};
const dmMark = (kind, key) => `<span class="dm-mark dm-mark-${kind}" title="${esc(t(key))}">`
  + `<svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${DM_MARK_ICONS[kind]}</svg></span>`;
const dmUrgent = d => (d.priority === 'urgent' ? dmMark('urgent', 'dm.urgent') : '');
const dmMarks = d => {
  const marks = [
    d.needsReply && dmMark('reply', 'dm.needsReply'),
    d.hasFiles && dmMark('files', 'dm.hasFiles'),
    d.hasTasks && dmMark('tasks', 'dm.hasTasks'),
  ].filter(Boolean);
  return marks.length ? `<span class="dm-marks">${marks.join('')}</span>` : '';
};

// The people of a discussion as first-name labels in their own colour, the last one "+N" counting
// those the line has no room for (fitPeopleLabels). No line lays out more than PEOPLE_LABELS names.
const PEOPLE_LABELS = 12;
const peopleLabels = people => people.slice(0, PEOPLE_LABELS).map(p =>
  `<span class="dm-tag" style="--pc:${p.colour}" title="${esc(p.name || p.address)}">${esc(p.short)}</span>`).join('')
  + (people.length > 1 ? `<span class="dm-rest">+${esc(fmtNumber(people.length - 1))}</span>` : '');

// How many of those labels fit: every one is laid out once, their widths (and the gaps between them)
// summed against the room the line has, and the ones past that room are hidden. The widths are all
// read before anything is hidden, so a whole list is measured in one layout, not one per line.
function fitPeopleLabels(root) {
  if (!root) return;
  const read = [...root.querySelectorAll('.dm-people[data-people]')].map(box => {
    for (const el of box.querySelectorAll('[hidden]')) el.hidden = false;   // measured visible again
    const tags = [...box.querySelectorAll('.dm-tag')];
    const rest = box.querySelector('.dm-rest');
    const gap = parseFloat(getComputedStyle(box).columnGap) || 0;
    return {
      box, tags, rest, gap, room: box.clientWidth, total: Number(box.dataset.people),
      widths: tags.map(el => el.offsetWidth), restW: rest ? rest.offsetWidth + gap : 0,
    };
  });
  for (const r of read) {
    if (!r.room) continue;   // not laid out (the panel is hidden): leave the labels as they are
    const used = k => r.widths.slice(0, k).reduce((a, w) => a + w, 0) + r.gap * (k - 1);
    let shown = r.widths.length;
    // Hiding a person leaves room for "+N" too; the first label always stays, however long it is.
    while (shown > 1 && used(shown) + (shown < r.total ? r.restW : 0) > r.room) shown--;
    r.tags.forEach((el, i) => { el.hidden = i >= shown; });
    r.box.classList.toggle('single', shown === 1);
    if (!r.rest) continue;
    const hasRoom = used(shown) + r.restW <= r.room;
    r.rest.hidden = !(r.total > shown && hasRoom);
    if (!r.rest.hidden) r.rest.textContent = `+${fmtNumber(r.total - shown)}`;
  }
}

// A narrower window changes how many names fit: the lines and the header are measured again.
let refitNames = null;
addEventListener('resize', () => {
  clearTimeout(refitNames);
  refitNames = setTimeout(() => {
    if (activePanel !== 'discussions') return;
    fitPeopleLabels(document.getElementById('dm-lines'));
    fitPeopleLabels(document.querySelector('.dm-head'));
  }, 200);
});

// ── Discussion details (ⓘ sheet), whole-discussion actions and the message menu ──
let dmInfoOpen = false;

// The emails of a discussion (each once, with its folder) that sit in one of these folders.
function dmEmails(d, folders) {
  const seen = new Map();
  for (const i of d.items) if (i.email && folders.includes(i.folder)) seen.set(i.email, i.folder);
  return [...seen].map(([email, folder]) => ({ email, folder }));
}

// The item a .dm-row shows (null for drafts and notes).
function dmItemOf(d, row) {
  if (!row) return null;
  return d.items.find(i => i.email && i.kind !== 'draft' && i.folder === row.dataset.folder && String(i.email.uid) === row.dataset.uid) || null;
}

// An email leaves its folder in the app state (the server move is already done).
function dropFromFolder(email, folder) {
  if (folder === 'inbox') CourrielCore.removeEmailFromState(appState, email.uid, null);
  else appState[`${folder}_emails`] = appState[`${folder}_emails`].filter(e => e !== email);
}

// Moves emails to the trash or the archive, one by one; stops at the first failure.
async function moveEmails(list, command) {
  try {
    for (const { email, folder } of list) {
      await cmd(command, { uid: email.uid, folder });
      dropFromFolder(email, folder);
    }
  } catch (e) {
    showToast(String(e));
  }
  renderAll();
}

// Header buttons: mark read/unread, archive, delete, details.
// Archive and delete take the inbox and sent copies too: a discussion is listed while either remains.
async function discussionAction(d, act) {
  if (act === 'info') { showDiscussionInfo(d, !dmInfoOpen); return; }
  if (act === 'read') {
    const inbox = dmEmails(d, ['inbox']).map(x => x.email);
    if (d.unread) setReadAll(inbox, true);
    else {   // like a messaging app: the latest message from someone else comes back as new
      const last = inbox.filter(e => !d.items.some(i => i.email === e && i.mine)).at(-1) || inbox.at(-1);
      setRead(last, false);
    }
    renderDiscussions();
    return;
  }
  if (act === 'archive') await moveEmails(dmEmails(d, ['inbox', 'sent']), 'archive_email');
  if (act === 'delete') {
    const list = dmEmails(d, ['inbox', 'sent', 'spam']);
    if (!await askInApp(t('dm.deleteAsk', { n: list.length }), t('rp.delete'))) return;
    await moveEmails(list, 'trash_email');
  }
}

// Opens or closes the ⓘ sheet without rebuilding the conversation.
function showDiscussionInfo(d, open) {
  dmInfoOpen = open;
  renderDiscussionInfo(d);
  document.querySelector('#dm-conv .dm-act[data-dm="info"]')?.classList.toggle('on', open);
}

function renderDiscussionInfo(d) {
  const sheet = document.getElementById('dm-info');
  const open = !!d && dmInfoOpen;
  sheet.classList.toggle('hidden', !open);
  if (!open) { sheet.innerHTML = ''; return; }
  const info = d.info;
  const account = n => composeAccounts[n] ? splitSender(composeAccounts[n].from).email : '';
  const where = w => [account(w.account), w.folders.map(f => `${t(`nav.${f.folder}`)} (${fmtNumber(f.count)})`).join(', ')]
    .filter(Boolean).join(' · ');
  const tile = f => `<button class="dm-tile" data-folder="${esc(f.folder)}" data-uid="${esc(f.email.uid)}" title="${esc(f.name || f.email.subject || '')}">`
    + (f.kind === 'image' ? `<img src="${esc(f.src)}" alt="" />`
      : `<span class="dm-tile-icon">📎</span><span class="dm-tile-name">${esc(f.name || t('dm.file'))}</span>`)
    + '</button>';
  sheet.innerHTML = `
    <div class="dm-info-top"><b>${esc(t('dm.details'))}</b><button class="dm-info-close" title="${esc(t('cmp.close'))}">✕</button></div>
    <h3>${esc(t('dm.people'))}</h3>
    ${info.people.map(p => `<div class="dm-info-person">${avatar(p)}<div class="dm-info-who">
      <div class="dm-info-name">${esc(p.mine ? t('dm.me') : p.name || p.address)}</div>
      <div class="dm-info-addr">${esc(p.address)}</div></div></div>`).join('')}
    <h3>${esc(t('dm.subjects'))}</h3>
    ${info.subjects.map(s => `<div class="dm-info-line">${esc(s)}</div>`).join('')}
    <h3>${esc(t('dm.where'))}</h3>
    ${info.where.map(w => `<div class="dm-info-line">${esc(where(w))}</div>`).join('')}
    ${info.summary ? `<h3>${esc(t('dm.summary'))}</h3><p class="dm-info-summary">${esc(info.summary)}</p>` : ''}
    ${info.files.length ? `<h3>${esc(t('dm.files'))}</h3><div class="dm-info-files">${info.files.map(tile).join('')}</div>` : ''}`;
  sheet.querySelector('.dm-info-close').addEventListener('click', () => showDiscussionInfo(d, false));
  sheet.querySelectorAll('.dm-tile').forEach(btn => btn.addEventListener('click', () => {
    const row = document.querySelector(`#dm-conv .dm-row[data-folder="${btn.dataset.folder}"][data-uid="${btn.dataset.uid}"]`);
    if (!row) return;
    row.classList.remove('dm-landed');
    void row.offsetWidth;   // restart the highlight
    row.classList.add('dm-landed');
    row.scrollIntoView({ block: 'center' });
  }));
}

// Right-click on a message: reply to this one, forward, delete, add as task, show original.
function messageMenu(d, item, x, y) {
  document.querySelectorAll('.dm-menu').forEach(m => m.remove());
  const { email, folder } = item;
  const hasTask = d.items.some(i => i.email === email && i.task);
  const acts = [
    ['reply', t('rp.reply'), () => openCompose({ mode: 'reply', email })],
    ['forward', t('rp.forward'), () => openCompose({ mode: 'forward', email })],
    folder !== 'trash' && ['delete', t('rp.delete'), async () => {
      if (await askInApp(t('dm.deleteOneAsk'), t('rp.delete'))) await moveEmails([{ email, folder }], 'trash_email');
    }],
    folder === 'inbox' && !hasTask && ['task', t('dm.addTask'), async () => {
      try {
        setTasks(await cmd('create_task', {
          title: email.task || email.subject || email.summary || t('cal.newTask'), project: email.project || null, dueDate: null, sourceUid: email.uid,
        }));
        showToast(t('dm.taskAdded'));
      } catch (e) { showToast(String(e)); }
    }],
    ['original', t('dm.original'), () => showOriginal(email, folder)],
  ].filter(Boolean);
  const menu = document.createElement('div');
  menu.className = 'rp-menu dm-menu';
  menu.innerHTML = acts.map(([act, label]) => `<button data-act="${act}">${esc(label)}</button>`).join('');
  document.body.appendChild(menu);
  menu.style.left = `${Math.max(0, Math.min(x, innerWidth - menu.offsetWidth - 4))}px`;
  menu.style.top = `${Math.max(0, Math.min(y, innerHeight - menu.offsetHeight - 4))}px`;
  menu.querySelectorAll('button').forEach((btn, i) => btn.addEventListener('click', () => { menu.remove(); acts[i][2](); }));
}

// The complete email: every header as received, the full layout and the quoted history.
async function showOriginal(email, folder) {
  const overlay = document.getElementById('original-overlay');
  document.getElementById('original-subject').textContent = email.subject || '';
  document.getElementById('original-meta').innerHTML = metaHtml(email);
  const headers = document.getElementById('original-headers');
  headers.textContent = '…';
  setIframeContent(document.getElementById('original-body'), email);
  overlay.classList.remove('hidden');
  try { headers.textContent = await cmd('email_headers', { uid: email.uid, folder }); }
  catch (e) { headers.textContent = String(e); }
}
document.getElementById('original-close').addEventListener('click', () => document.getElementById('original-overlay').classList.add('hidden'));
document.addEventListener('keydown', e => {
  if (e.key === 'Escape') document.getElementById('original-overlay').classList.add('hidden');
});

// Go to task, under a message: the task on its board, scrolled to and highlighted.
function openTask(id) {
  const task = appState.tasks.find(x => x.id === id);
  if (!task) return;
  activeBoard = task.project || null;
  switchPanel('tasks');
  const card = document.querySelector(`.kanban-card[data-id="${id}"]`);
  if (!card) return;
  card.classList.add('kanban-landed');
  card.scrollIntoView({ block: 'center' });
}

// ── Overview ────────────────────────────────────────────────────────────────
function renderOverview() {
  if (!appState) return;
  renderOverviewOpenedToggle();
  document.getElementById('ov-greeting').textContent = appState.user_name !== undefined ? greetingText(appState) : appState.greeting;
  document.getElementById('ov-briefing').textContent = appState.briefing ? briefingText(appState.briefing) : appState.briefing_time;
  // Nothing gets sorted without the AI: offer to set it up (also when it fails, e.g. a wrong key).
  const aiKind = appState.briefing?.kind;
  document.getElementById('ov-setup-ai').classList.toggle('hidden', aiKind !== 'ai_missing' && aiKind !== 'ai_error');

  // Urgent + High
  const urgent = CourrielCore.attentionEmails(appState, showOpenedAttention);
  const urgentList = document.getElementById('ov-urgent-list');
  renderEmailRows(urgentList, urgent, uid => switchToEmail(uid));
  attachSwipes(urgentList, urgent, e => trashFromOverview(e.uid));

  // Deadlines
  const dl = document.getElementById('ov-deadlines-list');
  dl.innerHTML = '';
  for (const d of appState.deadlines) {
    const row = document.createElement('div');
    row.className = 'email-row';
    row.innerHTML = `<div class="email-row-top"><span class="email-from">${esc(d.subject)}</span><span class="email-date">${esc(d.date_iso ? fmtDayMonth(d.date_iso) : d.deadline_str)}</span></div><div class="email-summary">${esc(d.project)} · ${d.days_until >= 0 ? t('ov.inDays', { n: d.days_until }) : t('ov.daysAgo', { n: -d.days_until })}</div>`;
    row.addEventListener('click', () => switchToEmail(d.uid));
    dl.appendChild(row);
  }
  attachSwipes(dl, appState.deadlines, d => dismissFromOverview('deadline', d.uid));

  // Tasks
  const taskList = CourrielCore.activeTasks(appState);
  const tasksEl = document.getElementById('ov-tasks-list');
  renderTaskRows(tasksEl, taskList);
  attachSwipes(tasksEl, taskList, t => setTaskStatus(t.id, 'given_up'));

  // Meetings
  const meetingsEl = document.getElementById('ov-meetings-list');
  renderEmailRows(meetingsEl, appState.meeting_items, uid => switchToEmail(uid));
  attachSwipes(meetingsEl, appState.meeting_items, e => dismissFromOverview('meeting', e.uid));

  // Week strip
  renderWeekStrip();
  updateUnreadCount();
}

// Unread inbox emails, shown on the app icon and in the window title. Runs after every
// change of the list (refresh, read/unread, deletion), which all redraw the Overview.
let shownUnreadCount = null;
function updateUnreadCount() {
  const count = appState.all_emails.filter(e => !e.is_read).length;
  if (count === shownUnreadCount) return;
  shownUnreadCount = count;
  cmd('set_unread_count', { count }).catch(() => {});
}

// ── Swipe right to delete (Overview) ───────────────────────────────────────
const SWIPE_THRESHOLD = 0.35;   // fraction of the row width that arms the delete

// Rows in `container` are in the same order as `items`.
function attachSwipes(container, items, onDelete) {
  Array.from(container.children).forEach((row, i) => {
    if (items[i]) makeSwipeable(row, () => onDelete(items[i]));
  });
}

function makeSwipeable(row, onDelete) {
  let startX = null, dx = 0, dragging = false, wheelTimer = null;
  const armed = () => dx > row.offsetWidth * SWIPE_THRESHOLD;
  const setX = x => {
    dx = Math.max(0, x);
    row.style.transform = `translateX(${dx}px)`;
    row.classList.toggle('swipe-armed', armed());
  };
  const finish = () => {
    row.classList.remove('swiping');
    if (armed()) {
      commitSwipe(row, onDelete);
    } else {
      row.style.transform = '';
      row.classList.remove('swipe-armed');
    }
    dx = 0;
  };

  // Mouse drag / touch
  row.addEventListener('pointerdown', e => {
    if (e.button !== 0) return;
    startX = e.clientX;
    dragging = false;
  });
  row.addEventListener('pointermove', e => {
    if (startX === null) return;
    const d = e.clientX - startX;
    if (!dragging && d > 5) {
      dragging = true;
      row.setPointerCapture(e.pointerId);
      row.classList.add('swiping');
    }
    if (dragging) setX(d);
  });
  const release = () => {
    if (startX === null) return;
    startX = null;
    if (dragging) {
      row.dataset.justSwiped = '1';   // swallow the click that follows the drag
      finish();
    }
  };
  row.addEventListener('pointerup', release);
  row.addEventListener('pointercancel', release);

  // Two-finger touchpad swipe (horizontal wheel)
  row.addEventListener('wheel', e => {
    if (Math.abs(e.deltaX) <= Math.abs(e.deltaY)) return;
    e.preventDefault();
    row.classList.add('swiping');
    setX(dx - e.deltaX);
    clearTimeout(wheelTimer);
    wheelTimer = setTimeout(finish, 150);
  }, { passive: false });
}

// Slide out in plain red, collapse, then run the delete action.
function commitSwipe(row, onDelete) {
  row.classList.add('swipe-armed', 'swipe-out');
  row.style.transform = `translateX(${row.offsetWidth}px)`;
  setTimeout(() => {
    row.style.height = `${row.offsetHeight}px`;
    void row.offsetHeight;
    row.classList.add('swipe-collapse');
    row.style.height = '0px';
    setTimeout(async () => {
      try {
        await onDelete();
      } catch (e) {
        renderAll();   // put the row back
        document.getElementById('ov-briefing').textContent = t('ov.couldNotDelete', { err: e });
      }
    }, 180);
  }, 180);
}

// Clicks right after a drag must not open the email.
document.addEventListener('click', e => {
  const row = e.target.closest && e.target.closest('[data-just-swiped]');
  if (row) {
    e.stopPropagation();
    e.preventDefault();
    delete row.dataset.justSwiped;
  }
}, true);

async function trashFromOverview(uid) {
  await cmd('trash_email', { uid });
  removeFromState(uid, null);
}

async function dismissFromOverview(kind, uid) {
  await cmd('dismiss_item', { kind, uid });
  removeFromState(uid, kind);
}

// kind = null: the email is gone, drop it everywhere; otherwise only from that list.
function removeFromState(uid, kind) {
  CourrielCore.removeEmailFromState(appState, uid, kind);
  renderAll();
}

function renderTaskRows(container, items) {
  container.innerHTML = '';
  for (const task of items) {
    const row = document.createElement('div');
    row.className = 'email-row';
    const meta = [task.status === 'in_progress' ? t('kanban.inProgress') : t('kanban.todo'), task.project, task.source_from,
      task.created_ts ? t('kanban.appeared', { date: fmtDay(task.created_ts) }) : '']
      .filter(Boolean).join(' · ');
    row.innerHTML = `<div class="email-row-top"><span class="email-from">${esc(task.title)}</span></div><div class="email-summary">${esc(meta)}</div>`;
    row.addEventListener('click', () => {
      if (task.source_uid) switchToEmail(task.source_uid);
      else switchPanel('tasks');
    });
    container.appendChild(row);
  }
}

function renderEmailRows(container, emails, onClickFn) {
  container.innerHTML = '';
  for (const e of emails) {
    const row = emailRow(e);
    row.addEventListener('click', () => onClickFn(e.uid));
    container.appendChild(row);
  }
}

function renderWeekStrip() {
  const strip = document.getElementById('ov-week-strip');
  strip.innerHTML = '';
  for (const day of appState.week_days) {
    const col = document.createElement('div');
    col.className = 'week-day' + (day.is_today ? ' today' : '');
    const slots = [
      [day.dl1_subject, day.dl1_uid],
      [day.dl2_subject, day.dl2_uid],
      [day.dl3_subject, day.dl3_uid],
    ].filter(([s]) => s);
    col.innerHTML = `<div class="week-day-header"><span class="week-day-name">${esc(day.date_iso ? fmtWeekday(day.date_iso) : day.day_name)}</span><span class="week-day-num">${day.day_number}</span></div>` +
      slots.map(([s, uid]) => `<div class="week-dl-pill" data-uid="${uid}">${esc(s)}</div>`).join('');
    col.querySelectorAll('.week-dl-pill').forEach(p =>
      p.addEventListener('click', () => switchToEmail(+p.dataset.uid))
    );
    strip.appendChild(col);
  }
}

// ── Split panel rendering ──────────────────────────────────────────────────
// Unfolded discussions ("panel:message-id"), kept when the list is redrawn.
const openThreads = new Set();
function renderSplitPanel(panelId, meta) {
  const section = document.getElementById(`panel-${panelId}`);
  const sort = emailSort[panelId];
  const emails = CourrielCore.sortEmails(appState[meta.key], sort, langInfo().locale);
  const listEl = section.querySelector('.email-list');
  const countEl = section.querySelector('.list-count');
  const sortCurrent = section.querySelector('.list-sort-current');
  const sortMenu = section.querySelector('.list-sort-menu');
  const directionBtn = section.querySelector('.list-sort-direction');
  countEl.textContent = emails.length;
  const selectedSort = EMAIL_SORT_OPTIONS.find(([value]) => value === sort.key);
  sortCurrent.textContent = t(selectedSort?.[1] || 'mail.sort.date');
  sortMenu.innerHTML = EMAIL_SORT_OPTIONS
    .map(([value, key]) => `<button class="list-sort-option${value === sort.key ? ' active' : ''}" data-sort="${value}"><span>${t(key)}</span>${value === sort.key ? '<span>✓</span>' : ''}</button>`)
    .join('');
  directionBtn.textContent = sort.ascending ? '↑' : '↓';
  directionBtn.title = sort.key === 'date'
    ? (sort.ascending ? t('mail.oldestFirst') : t('mail.newestFirst'))
    : t(sort.ascending ? 'mail.ascending' : 'mail.descending');

  // Remember active uid
  const active = section.querySelector('.email-row.active');
  const activeUid = active ? +active.dataset.uid : null;

  listEl.innerHTML = '';
  const addRow = e => {
    const row = emailRow(e);
    row.dataset.uid = e.uid;
    if (e.uid === activeUid) row.classList.add('active');
    row.addEventListener('click', () => {
      section.querySelectorAll('.email-row').forEach(r => r.classList.remove('active'));
      row.classList.add('active');
      showEmail(section, e);
    });
    listEl.appendChild(row);
    return row;
  };
  // A discussion shows its latest email; the arrow unfolds the other ones below it.
  for (const { lead, replies } of CourrielCore.groupThreads(emails)) {
    const leadRow = addRow(lead);
    if (!replies.length) continue;
    const key = `${panelId}:${lead.message_id || lead.uid}`;
    if (replies.some(e => e.uid === activeUid)) openThreads.add(key);
    const childRows = replies.map(e => {
      const row = addRow(e);
      row.classList.add('thread-child');
      return row;
    });
    const toggle = document.createElement('button');
    toggle.className = 'thread-toggle';
    toggle.title = t('mail.thread', { n: replies.length + 1 });
    toggle.innerHTML = `<span class="thread-arrow">▸</span>${replies.length + 1}`;
    const setOpen = open => {
      leadRow.classList.toggle('thread-open', open);
      toggle.setAttribute('aria-expanded', String(open));
      childRows.forEach(r => r.classList.toggle('collapsed', !open));
    };
    setOpen(openThreads.has(key));
    toggle.addEventListener('pointerdown', e => e.stopPropagation());   // not the start of a swipe
    toggle.addEventListener('click', e => {
      e.stopPropagation();   // don't open the email
      const open = !openThreads.has(key);
      if (open) openThreads.add(key); else openThreads.delete(key);
      setOpen(open);
    });
    leadRow.querySelector('.email-row-top').prepend(toggle);
  }
}

function showEmail(section, email) {
  const placeholder = section.querySelector('.rp-placeholder');
  const content = section.querySelector('.rp-content');
  placeholder.style.display = 'none';
  content.classList.remove('hidden');

  section.querySelector('.rp-subject').textContent = email.subject;
  section.querySelector('.rp-meta').innerHTML = metaHtml(email);

  const iframe = section.querySelector('.rp-iframe');
  setIframeContent(iframe, email);

  renderActions(section.querySelector('.rp-actions'), email, () => {
    content.classList.add('hidden');
    placeholder.style.display = '';
  });
  setRead(email, true);
}

// Mark inbox emails read: locally right away, on the server in the background, one after
// the other (each opens its own IMAP connection and rewrites the cache).
const NON_INBOX_PANELS = '#panel-sent, #panel-drafts, #panel-trash, #panel-spam';
let readQueue = Promise.resolve();
function setRead(email, read) { setReadAll([email], read); }
function setReadAll(emails, read) {
  if (!appState) return;
  const changed = [...emails].filter(email => email.is_read !== read && panelForEmail(email) === 'inbox');
  if (!changed.length) return;
  discussionReads++;
  // The same email appears in several lists (all_emails, urgent_emails, …).
  const skip = ['sent_emails', 'drafts_emails', 'trash_emails', 'spam_emails'];
  for (const email of changed) {
    const uid = email.uid;
    email.is_read = read;
    for (const [key, val] of Object.entries(appState)) {
      if (!Array.isArray(val) || skip.includes(key)) continue;
      for (const e of val) if (e.uid === uid && 'is_read' in e) e.is_read = read;
    }
    document.querySelectorAll(`.email-row[data-uid="${uid}"]`).forEach(row => {
      if (row.closest(NON_INBOX_PANELS)) return;
      row.classList.toggle('unread', !read);
      row.querySelector('button.row-mark')?.setAttribute('title', readToggleTitle(read));
    });
    readQueue = readQueue.then(() => cmd('set_read', { uid, read })).catch(() => {});   // cmd() logs the error
  }
  renderOverview();
}

// ── Reading-pane actions ───────────────────────────────────────────────────
// Reply · Forward · Spam · Delete · Project ▾ · ⋯ (Mark as unread, Save as…, Print)
// Spam, Delete, Project and Mark as unread only apply to inbox emails; Not spam to the Spam folder.
function renderActions(bar, email, onGone) {
  const folder = panelForEmail(email);
  const inInbox = folder === 'inbox';
  bar.innerHTML = `
    <button class="rp-btn" data-act="reply">${t('rp.reply')}</button>
    <button class="rp-btn" data-act="forward">${t('rp.forward')}</button>
    ${folder === 'spam' ? `<button class="rp-btn" data-act="notSpam">${t('rp.notSpam')}</button>` : ''}
    ${inInbox ? `
      <button class="rp-btn danger" data-act="spam">${t('rp.spam')}</button>
      <button class="rp-btn danger" data-act="delete">${t('rp.delete')}</button>
      <select class="rp-project" title="${t('rp.project')}"></select>` : ''}
    <span class="rp-status"></span>
    <div class="rp-more">
      <button class="rp-btn" data-act="more" title="${t('rp.more')}">⋯</button>
      <div class="rp-menu hidden">
        ${inInbox ? `<button data-act="unread">${t('rp.markUnread')}</button>` : ''}
        <button data-act="save">${t('rp.saveAs')}</button>
        <button data-act="print">${t('rp.print')}</button>
      </div>
    </div>`;

  const status = bar.querySelector('.rp-status');
  const flash = (msg, isError = false) => {
    status.textContent = msg;
    status.classList.toggle('error', isError);
    clearTimeout(status._t);
    status._t = setTimeout(() => { status.textContent = ''; }, 4000);
  };
  const menu = bar.querySelector('.rp-menu');

  const select = bar.querySelector('.rp-project');
  if (select) {
    select.innerHTML = `<option value="">${t('rp.noProject')}</option>` +
      appState.projects.map(p => `<option value="${esc(p.name)}">${esc(p.name)}</option>`).join('');
    select.value = email.project || '';
    select.addEventListener('change', async () => {
      const project = select.value || null;
      try {
        await cmd('set_email_project', { uid: email.uid, project });
        retagInState(email.uid, project || '');
        flash(project ? t('rp.movedTo', { project }) : t('rp.removedFromProject'));
      } catch (e) {
        select.value = email.project || '';
        flash(String(e), true);
      }
    });
  }

  const actions = {
    reply: () => openCompose({ mode: 'reply', email }),
    forward: () => openCompose({ mode: 'forward', email }),
    spam: () => moveAway('spam_email', 'Moved to spam'),
    notSpam: async () => {
      try {
        await cmd('not_spam_email', { uid: email.uid });
        // Only the spam entry goes: the same UID may be another email in the inbox.
        appState.spam_emails = appState.spam_emails.filter(e => e !== email);
        onGone();
        showToast(t('rp.movedToInbox'));
        cmd('refresh_emails').catch(() => {});   // bring it into the inbox now
      } catch (e) {
        flash(String(e), true);
      }
    },
    delete: () => moveAway('trash_email', 'Moved to trash'),
    more: () => menu.classList.toggle('hidden'),
    unread: () => { setRead(email, false); flash(t('rp.markedUnread')); },
    save: () => saveEmail(email).then(msg => msg && flash(msg)).catch(e => flash(String(e), true)),
    print: () => printEmail(email),
  };
  async function moveAway(command, doneMsg) {
    try {
      await cmd(command, { uid: email.uid });
      removeFromState(email.uid, null);
      onGone();
    } catch (e) {
      flash(String(e), true);
    }
  }

  bar.querySelectorAll('[data-act]').forEach(btn => {
    btn.addEventListener('click', e => {
      e.stopPropagation();
      if (btn.dataset.act !== 'more') menu.classList.add('hidden');
      actions[btn.dataset.act]();
    });
  });
}

// Close any open ⋯ menu on an outside click or Escape.
document.addEventListener('click', () => document.querySelectorAll('.rp-menu').forEach(m => m.classList.add('hidden')));
document.addEventListener('keydown', e => {
  if (e.key === 'Escape') document.querySelectorAll('.rp-menu').forEach(m => m.classList.add('hidden'));
});

function retagInState(uid, project) {
  const skip = ['sent_emails', 'drafts_emails', 'trash_emails', 'spam_emails', 'tasks'];
  for (const [key, val] of Object.entries(appState)) {
    if (!Array.isArray(val) || skip.includes(key)) continue;
    for (const e of val) if (e.uid === uid && 'project' in e) e.project = project;
  }
  for (const t of appState.tasks) if (t.source_uid === uid) t.project = project || null;
  setRowProject(uid, project);
  for (const p of appState.projects) {
    p.email_count = appState.all_emails.filter(e => e.project === p.name).length;
  }
  renderOverview();
  if (activePanel === 'projects') renderProjects();
}

// Returns a message for the status line, or null if the user cancelled.
async function saveEmail(email) {
  const safe = (email.subject || 'email').replace(/[\\/:*?"<>|]+/g, '_').slice(0, 80).trim() || 'email';
  const path = await window.__TAURI__.dialog.save({
    defaultPath: `${safe}.eml`,
    filters: [{ name: t('rp.emailFilter'), extensions: ['eml'] }],
  });
  if (!path) return null;
  await cmd('save_email', { uid: email.uid, folder: panelForEmail(email), path });
  return t('rp.savedTo', { path });
}

// Print only the email, from a throwaway sandboxed frame (no scripts allowed).
function printEmail(email) {
  const header = `<div style="font-family:system-ui,sans-serif;padding:0 0 12px;border-bottom:1px solid #ccc;margin-bottom:12px">
    <div style="font-size:18px;font-weight:700;margin-bottom:6px">${esc(email.subject)}</div>
    <div style="font-size:12px;color:#333">${metaHtml(email)}</div></div>`;
  const style = `<style>body{font-family:system-ui,sans-serif;font-size:14px;color:#111}
    .rp-date{font-weight:600;margin-bottom:4px}.rp-meta-line{margin:2px 0}
    .rp-meta-label{font-weight:600;display:inline-block;min-width:70px}
    .rp-person{margin-right:8px}.rp-person-mail{color:#666}</style>`;
  const body = email.body_html
    ? email.body_html
    : `<div style="white-space:pre-wrap">${esc(email.body_text)}</div>`;
  const html = `<!DOCTYPE html><html><head><meta charset="utf-8">${style}</head><body>${header}${body}</body></html>`;

  const frame = document.createElement('iframe');
  frame.setAttribute('sandbox', 'allow-same-origin allow-modals');
  frame.style.cssText = 'position:fixed;width:0;height:0;border:0;left:-9999px';
  const url = URL.createObjectURL(new Blob([html], { type: 'text/html' }));
  frame.onload = () => {
    URL.revokeObjectURL(url);
    try { frame.contentWindow.focus(); frame.contentWindow.print(); }
    finally { setTimeout(() => frame.remove(), 1000); }
  };
  frame.src = url;
  document.body.appendChild(frame);
}

// Full date + everyone involved (From / To / Cc / Bcc / Reply-To) for the reading pane header.
function metaHtml(email) {
  const date = emailDateFull(email);
  let html = `<div class="rp-date">${esc(date)}</div>`;

  const from = email.from_email && email.from_email !== email.from_name
    ? `${email.from_name} <${email.from_email}>`
    : email.from_name;
  const rows = [
    [t('meta.from'), from],
    [t('meta.to'), email.to],
    [t('meta.cc'), email.cc],
    [t('meta.bcc'), email.bcc],
    [t('meta.replyTo'), email.reply_to],
  ];
  for (const [label, value] of rows) {
    const people = splitAddresses(value);
    if (!people.length) continue;
    html += `<div class="rp-meta-line"><span class="rp-meta-label">${label}</span>` +
      `<span class="rp-people">${people.map(personChip).join('')}</span></div>`;
  }
  return html;
}

// Split an address-list header on commas that are not inside quotes or <…>.
function splitAddresses(value) {
  if (!value) return [];
  const out = [];
  let cur = '', inQuote = false, inAngle = false;
  for (const ch of value) {
    if (ch === '"') inQuote = !inQuote;
    else if (ch === '<' && !inQuote) inAngle = true;
    else if (ch === '>' && !inQuote) inAngle = false;
    if (ch === ',' && !inQuote && !inAngle) { out.push(cur); cur = ''; }
    else cur += ch;
  }
  out.push(cur);
  return out.map(s => s.trim()).filter(Boolean);
}

function personChip(addr) {
  const m = addr.match(/^(.*?)\s*<([^>]+)>$/);
  const name = m ? m[1].trim().replace(/^"|"$/g, '') : '';
  const mail = m ? m[2].trim() : addr;
  return name
    ? `<span class="rp-person" title="${esc(mail)}"><b>${esc(name)}</b> <span class="rp-person-mail">${esc(mail)}</span></span>`
    : `<span class="rp-person">${esc(mail)}</span>`;
}

function setIframeContent(iframe, email) {
  const html = email.body_html
    ? wrapHtml(email.body_html)
    : `<html><body style="font-family:system-ui,sans-serif;font-size:14px;padding:16px;white-space:pre-wrap;color:#111">${esc(email.body_text)}</body></html>`;
  // The backend has already decoded MIME charsets into a Unicode string. Declare
  // UTF-8 explicitly so stale charset meta tags in the original mail cannot turn
  // French characters such as é into mojibake.
  const blob = new Blob([html], { type: 'text/html;charset=utf-8' });
  const url = URL.createObjectURL(blob);
  iframe.src = url;
  iframe.onload = () => URL.revokeObjectURL(url);
}

function wrapHtml(raw) {
  // If already a full document, use as-is; otherwise wrap.
  if (/^\s*<!DOCTYPE/i.test(raw) || /^\s*<html/i.test(raw)) return raw;
  return `<!DOCTYPE html><html><head><meta charset="utf-8"><style>body{font-family:system-ui,sans-serif;font-size:14px;padding:16px;}</style></head><body>${raw}</body></html>`;
}

// ── Helper: switch to an email across all panels ───────────────────────────
function switchToEmail(uid) {
  if (!appState) return;
  const email = appState.all_emails.find(e => e.uid === uid)
    || [...appState.sent_emails, ...appState.drafts_emails, ...appState.trash_emails, ...appState.spam_emails].find(e => e.uid === uid);
  if (!email) return;

  // Pick best panel
  const targetPanel = panelForEmail(email);
  switchPanel(targetPanel);

  // After panel switch, click the matching row
  setTimeout(() => {
    const section = document.getElementById(`panel-${targetPanel}`);
    if (!section) return;
    const row = section.querySelector(`.email-row[data-uid="${uid}"]`);
    if (row) row.click();
  }, 0);
}

function openEmailIn(panel, uid) {
  switchPanel(panel);
  setTimeout(() => {
    const row = document.querySelector(`#panel-${panel} .email-row[data-uid="${uid}"]`);
    if (row) { row.click(); row.scrollIntoView({ block: 'nearest' }); }
  }, 0);
}

// Each folder numbers its UIDs separately, so the same UID can exist in two folders:
// look for the email object itself first, and only then compare UIDs.
const OTHER_FOLDERS = [['sent', 'sent_emails'], ['drafts', 'drafts_emails'], ['trash', 'trash_emails'], ['spam', 'spam_emails']];
function panelForEmail(email) {
  const own = OTHER_FOLDERS.find(([, key]) => appState[key].includes(email));
  if (own) return own[0];
  // Taken from an inbox list (all, important, urgent, meetings…).
  if (Object.values(appState).some(v => Array.isArray(v) && v.includes(email))) return 'inbox';
  const byUid = OTHER_FOLDERS.find(([, key]) => appState[key].some(e => e.uid === email.uid));
  return byUid ? byUid[0] : 'inbox';
}

// ── Search & AI assistant ───────────────────────────────────────────────────
// Stage 1: instant local ranking on every keystroke (backend search_local).
// Stage 2: Enter / Ctrl+Enter / ✨ asks the AI, which finds, drafts, summarises or answers.
const search = { hits: [], active: -1, mode: 'auto', selected: new Set(), timer: null, seq: 0, aiSeq: 0 };
const $s = id => document.getElementById(id);

function setupSearch() {
  $s('btn-search').addEventListener('click', openSearch);
  document.addEventListener('keydown', e => {
    const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName) || e.target.isContentEditable;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); openSearch(); }
    else if (e.key === '/' && !typing && $s('search-overlay').classList.contains('hidden')) { e.preventDefault(); openSearch(); }
  });
  $s('search-overlay').addEventListener('click', e => { if (e.target === e.currentTarget) closeSearch(); });
  $s('search-input').addEventListener('input', () => {
    clearTimeout(search.timer);
    search.timer = setTimeout(runLocalSearch, 80);
  });
  $s('search-input').addEventListener('keydown', onSearchKey);
  $s('search-ai-btn').addEventListener('click', () => runAiSearch());
  document.querySelectorAll('.search-chip').forEach(chip => chip.addEventListener('click', () => {
    search.mode = chip.dataset.mode;
    document.querySelectorAll('.search-chip').forEach(c => c.classList.toggle('active', c === chip));
    $s('search-input').focus();
  }));
  $s('search-server').addEventListener('change', () => { if ($s('search-server').checked) runServerSearch(); });
  $s('search-summarize-sel').addEventListener('click', () => runAiSearch([...search.selected], 'summarize'));
  $s('search-preview-reply').addEventListener('click', () => {
    if (search.previewEmail) { closeSearch(); openCompose({ mode: 'reply', email: search.previewEmail }); }
  });
}

function openSearch() {
  $s('search-overlay').classList.remove('hidden');
  const input = $s('search-input');
  const mail = postOffice() && document.body.classList.contains('discussion-mode');
  input.placeholder = t(mail ? 'dm.searchMail' : 'search.placeholder');
  input.focus();
  input.select();
  runLocalSearch();
}

function closeSearch() {
  $s('search-overlay').classList.add('hidden');
}

function setSearchStatus(msg, isError = false) {
  $s('search-status').textContent = msg;
  $s('search-status').classList.toggle('error', isError);
}

function showSearchSide(which) {
  $s('search-help').classList.toggle('hidden', which !== 'help');
  $s('search-answer').classList.toggle('hidden', which !== 'answer');
  $s('search-preview').classList.toggle('hidden', which !== 'preview');
}

async function runLocalSearch() {
  const seq = ++search.seq;
  const query = $s('search-input').value;
  let hits = [];
  try { hits = await cmd('search_local', { query }); } catch (_) {}
  if (seq !== search.seq) return;   // a newer keystroke already answered
  renderSearchHits(hits);
  setSearchStatus(hits.length || !query.trim() ? '' : t('search.noResults'));
  if (!query.trim()) showSearchSide('help');
  if ($s('search-server').checked && query.trim()) runServerSearch();
}

async function runServerSearch() {
  const query = $s('search-input').value;
  if (!query.trim()) return;
  setSearchStatus(t('search.serverSearching'));
  try {
    const serverHits = await cmd('search_server', { query });
    if (query !== $s('search-input').value) return;
    const known = new Set(search.hits.map(h => h.id));
    renderSearchHits([...search.hits, ...serverHits.filter(h => !known.has(h.id))]);
    setSearchStatus(tn('search.serverFound', serverHits.length));
  } catch (e) {
    setSearchStatus(String(e), true);
  }
}

// Words to highlight: the query minus filters and short words.
function searchTerms() {
  return $s('search-input').value.split(/\s+/)
    .filter(w => w && !w.includes(':') && w.length > 2)
    .map(w => w.toLowerCase());
}

// Wrap matches in <mark>, accent-insensitive; every piece of text is escaped.
function highlight(text, terms) {
  text = text || '';
  const fold = CourrielCore.fold;
  const folded = fold(text);
  if (!terms.length || folded.length !== text.length) return esc(text);
  const ranges = [];
  for (const term of terms.map(fold)) {
    for (let i = folded.indexOf(term); i !== -1; i = folded.indexOf(term, i + term.length)) {
      ranges.push([i, i + term.length]);
    }
  }
  ranges.sort((a, b) => a[0] - b[0]);
  let out = '', pos = 0;
  for (const [a, b] of ranges) {
    if (a < pos) continue;   // overlapping match
    out += esc(text.slice(pos, a)) + '<mark>' + esc(text.slice(a, b)) + '</mark>';
    pos = b;
  }
  return out + esc(text.slice(pos));
}

function renderSearchHits(hits) {
  search.hits = hits;
  search.active = hits.length ? 0 : -1;
  const list = $s('search-results');
  const terms = searchTerms();
  list.innerHTML = '';
  hits.forEach((h, i) => {
    const row = document.createElement('div');
    row.className = 'search-hit' + (h.is_read ? '' : ' unread') + (i === search.active ? ' active' : '');
    row.dataset.index = i;
    row.innerHTML = `
      <input type="checkbox" class="search-hit-check" ${search.selected.has(h.id) ? 'checked' : ''} />
      <div class="search-hit-main">
        <div class="search-hit-top">
          <span class="search-hit-from">${highlight(h.from_name, terms)}</span>
          <span class="search-hit-folder f-${h.folder}">${esc(t('search.folder.' + h.folder))}</span>
          <span class="search-hit-date">${esc(h.date_ts ? fmtShort(h.date_ts) : h.date_str)}</span>
        </div>
        <div class="search-hit-subject">${highlight(h.subject, terms)}</div>
        <div class="search-hit-snippet">${highlight(h.summary || h.snippet, terms)}</div>
      </div>`;
    row.querySelector('.search-hit-check').addEventListener('click', e => {
      e.stopPropagation();
      if (e.target.checked) search.selected.add(h.id); else search.selected.delete(h.id);
      updateSelectionButton();
    });
    row.addEventListener('click', () => openSearchHit(h));
    row.addEventListener('mousemove', () => setActiveHit(i));
    list.appendChild(row);
  });
  updateSelectionButton();
}

function setActiveHit(i) {
  if (i === search.active) return;
  search.active = i;
  document.querySelectorAll('.search-hit').forEach(r => r.classList.toggle('active', +r.dataset.index === i));
  document.querySelector('.search-hit.active')?.scrollIntoView({ block: 'nearest' });
}

function updateSelectionButton() {
  const btn = $s('search-summarize-sel');
  btn.disabled = search.selected.size === 0;
  btn.textContent = t('search.summarizeSel', { n: search.selected.size });
}

function onSearchKey(e) {
  if (e.key === 'Escape') { e.preventDefault(); closeSearch(); return; }
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    e.preventDefault();
    if (!search.hits.length) return;
    const d = e.key === 'ArrowDown' ? 1 : -1;
    setActiveHit((search.active + d + search.hits.length) % search.hits.length);
    return;
  }
  if (e.key === 'Enter') {
    e.preventDefault();
    const askAi = e.ctrlKey || e.metaKey || search.mode !== 'auto' || !search.hits.length || looksLikeRequest();
    if (askAi) runAiSearch();
    else if (search.active >= 0) openSearchHit(search.hits[search.active]);
  }
}

// A sentence rather than keywords: send it to the AI on Enter.
function looksLikeRequest() {
  const q = $s('search-input').value.trim().toLowerCase();
  return q.split(/\s+/).length >= 4 || /^(r[ée]sum|synth|[ée]cri|r[ée]dig|r[ée]pond|qu|combien|quand|o[uù] |summar|write|draft|reply|what|when|who|how|which|find all)/.test(q);
}

async function openSearchHit(h) {
  if (h.folder === 'server') {
    // Older email, not in the panels: preview it inside the dialog.
    const email = await cmd('get_search_email', { id: h.id });
    if (!email) return;
    search.previewEmail = email;
    const pane = $s('search-preview');
    pane.querySelector('.rp-subject').textContent = email.subject;
    pane.querySelector('.rp-meta').innerHTML = metaHtml(email);
    setIframeContent(pane.querySelector('.rp-iframe'), email);
    showSearchSide('preview');
    return;
  }
  closeSearch();
  // Discussion mode: the result opens its discussion, scrolled to it and highlighted.
  if (document.body.classList.contains('discussion-mode') && openHitInDiscussion(h)) return;
  const panel = { inbox: 'inbox', sent: 'sent', drafts: 'drafts', trash: 'trash', spam: 'spam' }[h.folder] || 'inbox';
  openEmailIn(panel, h.uid);
}

async function runAiSearch(selectedIds = [], forcedMode = null) {
  const query = $s('search-input').value.trim();
  if (!query && !selectedIds.length) { setSearchStatus(t('search.empty'), true); return; }
  const mode = forcedMode || search.mode;
  const seq = ++search.aiSeq;
  setSearchStatus(t('search.thinking'));
  $s('search-ai-btn').disabled = true;
  let result;
  try {
    result = await cmd('ai_assist', { query: query || t('search.mode.summarize'), mode, selectedIds });
  } catch (e) {
    if (seq === search.aiSeq) setSearchStatus(String(e), true);
    return;
  } finally {
    $s('search-ai-btn').disabled = false;
  }
  if (seq !== search.aiSeq) return;
  setSearchStatus('');
  await showAiResult(result);
}

async function showAiResult(r) {
  if (r.intent === 'compose' && r.compose) {
    const c = r.compose;
    let inReplyTo = null;
    if (c.reply_to_id) inReplyTo = (await cmd('get_search_email', { id: c.reply_to_id }))?.message_id || null;
    closeSearch();
    openCompose({
      to: c.to, cc: c.cc, subject: c.subject,
      html: sanitizeHtml(c.body_html || '') || '<div><br></div>', inReplyTo,
    });
    showToast(t('search.composeOpened'));
    return;
  }
  // find: show the AI's picks in its order; summarize/answer: the text, with the emails cited.
  const emails = (await Promise.all(r.email_ids.map(id => cmd('get_search_email', { id }).then(e => e && { id, e }))))
    .filter(Boolean);
  if (r.intent === 'find' && emails.length) {
    renderSearchHits(emails.map(({ id, e }) => ({
      id, folder: id.split(':')[0], uid: e.uid, subject: e.subject, from_name: e.from_name,
      date_str: e.date_str, date_ts: e.date_ts, summary: e.summary, snippet: '', is_read: e.is_read,
    })));
  }
  const box = $s('search-answer');
  const paragraphs = (r.answer || '').split(/\n{2,}/).map(p =>
    /^\s*[-•*] /m.test(p)
      ? '<ul>' + p.split('\n').filter(l => l.trim()).map(l => `<li>${esc(l.replace(/^\s*[-•*]\s*/, ''))}</li>`).join('') + '</ul>'
      : `<p>${esc(p).replace(/\n/g, '<br>')}</p>`).join('');
  box.innerHTML = `<div class="search-answer-title">✨ ${esc(t('search.mode.' + r.intent))}</div>${paragraphs}` +
    (emails.length ? `<div class="search-cited-title">${esc(t('search.cited'))}</div><div class="search-cited"></div>` : '');
  const cited = box.querySelector('.search-cited');
  for (const { id, e } of emails) {
    const chip = document.createElement('button');
    chip.className = 'search-cited-chip';
    chip.textContent = `${e.from_name} — ${e.subject}`;
    chip.title = emailDateFull(e);
    chip.addEventListener('click', () => openSearchHit({ id, folder: id.split(':')[0], uid: e.uid }));
    cited.appendChild(chip);
  }
  showSearchSide('answer');
}

// ── Calendar ───────────────────────────────────────────────────────────────
let calYear, calMonth;

function setupCalendarNav() {
  document.getElementById('cal-prev').addEventListener('click', () => {
    calMonth--;
    if (calMonth < 1) { calMonth = 12; calYear--; }
    renderCalendar();
  });
  document.getElementById('cal-next').addEventListener('click', () => {
    calMonth++;
    if (calMonth > 12) { calMonth = 1; calYear++; }
    renderCalendar();
  });
}

function setupCalendarTaskDialog() {
  const overlay = document.getElementById('calendar-task-overlay');
  const title = document.getElementById('calendar-task-title');
  const date = document.getElementById('calendar-task-date');
  const project = document.getElementById('calendar-task-project');
  const error = document.getElementById('calendar-task-error');
  const create = document.getElementById('calendar-task-create');

  const close = () => {
    overlay.classList.add('hidden');
    error.textContent = '';
  };
  const save = async () => {
    const taskTitle = title.value.trim();
    if (!taskTitle) { error.textContent = t('cal.enterTask'); return; }
    create.disabled = true;
    try {
      const tasks = await cmd('create_task', {
        title: taskTitle,
        project: project.value || null,
        dueDate: date.value,
      });
      close();
      setTasks(tasks);
    } catch (e) {
      error.textContent = String(e);
    } finally {
      create.disabled = false;
    }
  };

  document.getElementById('calendar-task-close').onclick = close;
  document.getElementById('calendar-task-cancel').onclick = close;
  create.onclick = save;
  title.onkeydown = e => {
    if (e.key === 'Enter') save();
    if (e.key === 'Escape') close();
  };
  overlay.addEventListener('click', e => { if (e.target === overlay) close(); });
  document.addEventListener('keydown', e => {
    if (e.key === 'Escape' && !overlay.classList.contains('hidden')) close();
  });
}

function openCalendarTaskDialog(iso) {
  const overlay = document.getElementById('calendar-task-overlay');
  const project = document.getElementById('calendar-task-project');
  document.getElementById('calendar-task-title').value = '';
  document.getElementById('calendar-task-date').value = iso;
  document.getElementById('calendar-task-error').textContent = '';
  project.innerHTML = `<option value="">${t('cal.noProject')}</option>`
    + appState.projects.map(p => `<option value="${esc(p.name)}">${esc(p.name)}</option>`).join('');
  overlay.classList.remove('hidden');
  document.getElementById('calendar-task-title').focus();
}

function renderCalendar() {
  if (!appState) return;
  if (!calYear) { calYear = appState.calendar_year; calMonth = appState.calendar_month; }

  document.getElementById('cal-title').textContent = fmtMonthYear(calYear, calMonth);

  const grid = document.getElementById('cal-grid');
  grid.innerHTML = '';

  // Day labels
  for (const d of weekdayNames()) {
    const cell = document.createElement('div');
    cell.className = 'cal-day-label';
    cell.textContent = d;
    grid.appendChild(cell);
  }

  const firstDay = new Date(calYear, calMonth - 1, 1);
  // Monday-based: 0=Mon … 6=Sun
  let startOffset = (firstDay.getDay() + 6) % 7;
  const daysInMonth = new Date(calYear, calMonth, 0).getDate();
  const prevDays = new Date(calYear, calMonth - 1, 0).getDate();
  const today = new Date();

  // Deadlines by YYYY-MM-DD, placed in their day's cell.
  const byDay = {};
  for (const d of appState.deadlines) {
    (byDay[d.date_iso] ||= []).push({ ...d, kind: 'email' });
  }
  for (const task of appState.tasks) {
    if (task.due_date && (task.status === 'todo' || task.status === 'in_progress')) {
      (byDay[task.due_date] ||= []).push({
        kind: 'task', id: task.id, subject: task.title, project: task.project || '',
      });
    }
  }
  const urgencyOf = Object.fromEntries(appState.projects.map(p => [p.name, p.urgency]));
  const pad = n => String(n).padStart(2, '0');
  const isoOf = (y, m, d) => `${y}-${pad(m)}-${pad(d)}`;
  const todayIso = isoOf(today.getFullYear(), today.getMonth() + 1, today.getDate());

  const totalCells = Math.ceil((startOffset + daysInMonth) / 7) * 7;
  for (let i = 0; i < totalCells; i++) {
    const cell = document.createElement('div');
    let day, month, year, otherMonth = false;
    if (i < startOffset) {
      day = prevDays - startOffset + i + 1;
      month = calMonth - 1 || 12;
      year = calMonth === 1 ? calYear - 1 : calYear;
      otherMonth = true;
    } else {
      day = i - startOffset + 1;
      month = calMonth;
      year = calYear;
      if (day > daysInMonth) {
        day -= daysInMonth;
        month = calMonth + 1 > 12 ? 1 : calMonth + 1;
        year = calMonth === 12 ? calYear + 1 : calYear;
        otherMonth = true;
      }
    }
    const isToday = !otherMonth && day === today.getDate() && calMonth === today.getMonth() + 1 && calYear === today.getFullYear();
    cell.className = 'cal-cell' + (otherMonth ? ' other-month' : '') + (isToday ? ' today' : '');
    cell.innerHTML = `<div class="cal-day-num">${day}</div>`;
    const iso = isoOf(year, month, day);
    renderCalendarEvents(cell, byDay[iso] || [], iso < todayIso, urgencyOf);
    cell.addEventListener('click', () => openCalendarTaskDialog(iso));
    grid.appendChild(cell);
  }
}

const CAL_MAX_PILLS = 3;

function renderCalendarEvents(cell, events, past, urgencyOf, expanded = false) {
  cell.querySelectorAll('.cal-dl-pill, .cal-more').forEach(el => el.remove());
  cell.classList.toggle('expanded', expanded);
  const shown = expanded ? events : events.slice(0, CAL_MAX_PILLS);
  for (const d of shown) {
    const pill = document.createElement('div');
    pill.className = 'cal-dl-pill'
      + (d.kind === 'task' ? ' task' : '')
      + (urgencyOf[d.project] === 'high' ? ' high' : '')
      + (past ? ' past' : '');
    pill.textContent = d.subject;
    pill.title = d.project ? `${d.project} · ${d.subject}` : d.subject;
    pill.addEventListener('click', e => {
      e.stopPropagation();
      if (d.kind === 'task') {
        activeBoard = d.project || null;
        switchPanel('tasks');
      } else {
        switchToEmail(d.uid);
      }
    });
    cell.appendChild(pill);
  }
  const hidden = events.length - shown.length;
  if (hidden > 0) {
    const more = document.createElement('div');
    more.className = 'cal-more';
    more.textContent = t('cal.more', { n: hidden });
    more.addEventListener('click', e => {
      e.stopPropagation();
      renderCalendarEvents(cell, events, past, urgencyOf, true);
    });
    cell.appendChild(more);
  }
}

// ── Tasks kanban ───────────────────────────────────────────────────────────
let activeBoard = null;   // null = "All tasks", otherwise a project name
let taskJustDropped = false;  // swallows the click that ends a drag

function setupKanban() {
  const input = document.getElementById('kanban-new-title');
  input.addEventListener('keydown', async e => {
    if (e.key === 'Escape') { input.value = ''; input.blur(); return; }
    if (e.key !== 'Enter') return;
    const title = input.value.trim();
    if (!title) return;
    try {
      setTasks(await cmd('create_task', { title, project: activeBoard, dueDate: null }));
      input.value = '';
    } catch (err) {
      input.placeholder = String(err);
    }
  });
}

function setTasks(tasks) {
  appState.tasks = tasks;
  renderAll();
  if (activePanel !== 'tasks') renderKanban();
}

async function setTaskStatus(id, status) {
  setTasks(await cmd('set_task_status', { id, status }));
}

function renderKanban() {
  if (!appState) return;

  // Board selector: All tasks + one per project.
  const boards = document.getElementById('kanban-boards');
  boards.innerHTML = '';
  const names = appState.projects.map(p => p.name);
  if (activeBoard !== null && !names.includes(activeBoard)) activeBoard = null;
  for (const name of [null, ...names]) {
    const btn = document.createElement('button');
    btn.className = 'kanban-board-btn' + (activeBoard === name ? ' active' : '');
    const count = appState.tasks.filter(t =>
      (name === null || t.project === name) && (t.status === 'todo' || t.status === 'in_progress')).length;
    btn.innerHTML = `${esc(name ?? t('kanban.all'))} <span class="kanban-board-count">${count}</span>`;
    btn.addEventListener('click', () => { activeBoard = name; renderKanban(); });
    boards.appendChild(btn);
  }

  document.getElementById('kanban-new-title').placeholder =
    activeBoard ? t('kanban.addTo', { project: activeBoard }) : t('kanban.add');

  const visible = appState.tasks.filter(t => activeBoard === null || t.project === activeBoard);
  document.querySelectorAll('.kanban-col').forEach(col => {
    const items = visible.filter(t => t.status === col.dataset.status);
    col.querySelector('.kanban-count').textContent = items.length;
    const list = col.querySelector('.kanban-cards');
    list.innerHTML = '';
    // Newest first.
    for (const t of [...items].sort((a, b) => b.id - a.id)) list.appendChild(kanbanCard(t));
  });
}

function kanbanCard(task) {
  const card = document.createElement('div');
  card.className = 'kanban-card';
  card.dataset.id = task.id;

  const chips = [];
  if (activeBoard === null && task.project) chips.push(`<span class="kanban-chip">${esc(task.project)}</span>`);
  if (task.due_date) chips.push(`<span class="kanban-chip">${esc(fmtDayMonth(task.due_date))}</span>`);
  if (task.ai_verdict) chips.push(`<span class="kanban-chip ai" title="${t('kanban.aiMoved')}">${esc(task.ai_verdict === 'done' ? t('kanban.aiDone') : t('kanban.aiObsolete'))}</span>`);
  const source = task.source_uid
    ? `<div class="kanban-source" title="${t('kanban.openEmail')}"><span class="kanban-source-icon">✉</span><span class="kanban-source-text">${esc(task.source_from)} · ${esc(task.source_subject)}</span></div>`
    : '';
  // When the task appeared: its email's date, or the day it was added by hand.
  const appeared = task.created_ts
    ? `<div class="kanban-date">${esc(t('kanban.appeared', { date: fmtDay(task.created_ts) }))}</div>`
    : '';

  card.innerHTML = `
    <div class="kanban-card-top">
      <div class="kanban-card-title">${esc(task.title)}</div>
      <button class="kanban-delete" title="${t('kanban.delete')}">×</button>
    </div>
    ${source}
    ${appeared}
    ${chips.length ? `<div class="kanban-chips">${chips.join('')}</div>` : ''}
  `;

  // Dragged with the pointer, not the browser's drag and drop, which draws its own cursor:
  // the hand stays "grabbing" until the task is dropped on a column (#18).
  card.addEventListener('pointerdown', e => {
    if (e.button !== 0 || e.target.closest('.kanban-delete')) return;
    const r = card.getBoundingClientRect();
    const dx = e.clientX - r.left, dy = e.clientY - r.top;
    let ghost = null, over = null, x = 0, y = 0, scrolling = 0;
    const target = () => {
      const col = document.elementFromPoint(x, y)?.closest('.kanban-col') ?? null;
      if (col !== over) {
        over?.classList.remove('drop-target');
        col?.classList.add('drop-target');
        over = col;
      }
    };
    // Held near the board's left or right edge: scroll it, so columns out of view can be reached.
    const board = document.getElementById('kanban');
    const autoScroll = () => {
      const b = board.getBoundingClientRect();
      const step = x < b.left + 40 ? -12 : x > b.right - 40 ? 12 : 0;
      if (!ghost || !step) { scrolling = 0; return; }
      board.scrollLeft += step;
      target();
      scrolling = requestAnimationFrame(autoScroll);
    };
    const move = ev => {
      if (!(ev.buttons & 1)) return end(ev); // the release was lost (e.g. alt-tab): cancel
      if (!ghost) {
        if (Math.hypot(ev.clientX - e.clientX, ev.clientY - e.clientY) < 5) return;
        ghost = card.cloneNode(true);
        ghost.classList.add('kanban-ghost');
        ghost.style.width = r.width + 'px';
        document.body.appendChild(ghost);
        document.body.classList.add('task-dragging');
        card.classList.add('dragging');
      }
      ghost.style.left = (ev.clientX - dx) + 'px';
      ghost.style.top = (ev.clientY - dy) + 'px';
      x = ev.clientX; y = ev.clientY;
      target();
      if (!scrolling) autoScroll();
    };
    const escape = ev => { if (ev.key === 'Escape') end(ev); };
    const end = ev => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', end);
      window.removeEventListener('pointercancel', end);
      window.removeEventListener('blur', end);
      window.removeEventListener('keydown', escape);
      if (!ghost) return;
      cancelAnimationFrame(scrolling);
      ghost.remove();
      document.body.classList.remove('task-dragging');
      card.classList.remove('dragging');
      over?.classList.remove('drop-target');
      taskJustDropped = true;
      setTimeout(() => { taskJustDropped = false; });
      if (ev.type === 'pointerup' && over && over.dataset.status !== task.status) {
        setTaskStatus(task.id, over.dataset.status);
      }
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', end);
    window.addEventListener('pointercancel', end);
    window.addEventListener('blur', end);
    window.addEventListener('keydown', escape);
  });

  card.querySelector('.kanban-delete').addEventListener('click', async e => {
    e.stopPropagation();
    setTasks(await cmd('delete_task', { id: task.id }));
  });
  const src = card.querySelector('.kanban-source');
  if (src) src.addEventListener('click', () => { if (!taskJustDropped) switchToEmail(task.source_uid); });
  return card;
}

// ── Projects ───────────────────────────────────────────────────────────────
let activeProject = null;

function renderProjects() {
  if (!appState) return;
  const list = document.getElementById('projects-list');
  list.innerHTML = '';
  for (const p of appState.projects) {
    const row = document.createElement('div');
    row.className = 'project-row' + (activeProject === p.name ? ' active' : '');
    row.innerHTML = `<div>${esc(p.name)}</div><div class="project-urgency ${p.urgency}">${t('proj.urgency.' + (p.urgency === 'high' ? 'high' : 'normal'))} · ${tn('proj.emails', p.email_count)}</div>`;
    row.addEventListener('click', () => { activeProject = p.name; renderProjects(); showProject(p); });
    list.appendChild(row);
  }
}

function setupNewProject() {
  const form = document.getElementById('projects-new-form');
  const sentenceEl = document.getElementById('projects-new-sentence');
  const errEl = document.getElementById('projects-new-error');
  const createBtn = document.getElementById('projects-new-create');
  const cancelBtn = document.getElementById('projects-new-cancel');
  let creating = false;

  const closeForm = () => {
    if (creating) return;
    form.classList.add('hidden');
    sentenceEl.value = '';
    errEl.textContent = '';
  };

  const create = async () => {
    if (creating) return;
    const sentence = sentenceEl.value.trim();
    if (!sentence) { errEl.textContent = t('proj.enterSentence'); return; }
    creating = true;
    createBtn.disabled = true;
    cancelBtn.disabled = true;
    createBtn.textContent = t('proj.analysing');
    let project;
    try {
      project = await cmd('add_project', { sentence });
    } catch (e) {
      errEl.textContent = String(e);
      return;
    } finally {
      creating = false;
      createBtn.disabled = false;
      cancelBtn.disabled = false;
      createBtn.textContent = t('proj.create');
    }
    // The backend already redrew the lists with the project and the emails filed into it.
    if (appState && !appState.projects.some(x => x.name === project.name)) appState.projects.push(project);
    closeForm();
    activeProject = project.name;
    renderProjects();
    showProject(project);
    document.getElementById('projects-desc').focus();
    showToast(project.warning || t('proj.filed', { n: project.email_count }));
  };

  document.getElementById('projects-new').addEventListener('click', () => {
    form.classList.remove('hidden');
    sentenceEl.focus();
  });
  cancelBtn.addEventListener('click', closeForm);
  createBtn.addEventListener('click', create);
  sentenceEl.addEventListener('keydown', e => {
    if (e.key === 'Enter') create();
    if (e.key === 'Escape') closeForm();
  });
}

function showProject(p) {
  document.getElementById('projects-detail').classList.remove('hidden');
  document.getElementById('projects-detail-name').textContent = p.name;
  document.getElementById('projects-detail-urgency').textContent = t('proj.urgency.' + (p.urgency === 'high' ? 'high' : 'normal'));
  document.getElementById('projects-desc').value = p.description || '';

  const renameBtn = document.getElementById('projects-rename');
  const deleteBtn = document.getElementById('projects-delete');
  const titleDisplay = document.getElementById('projects-title-display');
  const renameForm = document.getElementById('projects-rename-form');
  const renameInput = document.getElementById('projects-rename-input');
  const renameError = document.getElementById('projects-rename-error');
  const renameSave = document.getElementById('projects-rename-save');
  const renameCancel = document.getElementById('projects-rename-cancel');
  deleteBtn.setAttribute('aria-label', t('proj.delete'));
  titleDisplay.classList.remove('hidden');
  renameForm.classList.add('hidden');
  renameBtn.classList.remove('hidden');
  renameError.textContent = '';
  renameBtn.disabled = false;
  deleteBtn.disabled = false;
  renameBtn.onclick = () => {
    renameInput.value = p.name;
    titleDisplay.classList.add('hidden');
    renameForm.classList.remove('hidden');
    renameBtn.classList.add('hidden');
    renameInput.focus();
    renameInput.select();
  };
  const closeRename = () => {
    titleDisplay.classList.remove('hidden');
    renameForm.classList.add('hidden');
    renameBtn.classList.remove('hidden');
    renameError.textContent = '';
  };
  const saveRename = async () => {
    const oldName = p.name;
    const newName = renameInput.value.trim();
    if (!newName) { renameError.textContent = t('proj.enterName'); return; }
    if (newName === oldName) { closeRename(); return; }
    renameInput.disabled = true;
    renameSave.disabled = true;
    renameCancel.disabled = true;
    deleteBtn.disabled = true;
    try {
      p.name = await cmd('rename_project', { name: oldName, newName });
      for (const [key, value] of Object.entries(appState)) {
        if (key === 'projects' || !Array.isArray(value)) continue;
        for (const item of value) if (item?.project === oldName) item.project = p.name;
      }
      renameProjectChips(oldName, p.name);
      if (activeBoard === oldName) activeBoard = p.name;
      activeProject = p.name;
      renderProjects();
      showProject(p);
    } catch (e) {
      renameError.textContent = String(e);
      renameInput.disabled = false;
      renameSave.disabled = false;
      renameCancel.disabled = false;
      deleteBtn.disabled = false;
    }
  };
  renameInput.disabled = false;
  renameSave.disabled = false;
  renameCancel.disabled = false;
  renameSave.onclick = saveRename;
  renameCancel.onclick = closeRename;
  renameInput.onkeydown = e => {
    if (e.key === 'Enter') saveRename();
    if (e.key === 'Escape') closeRename();
  };
  deleteBtn.onclick = async () => {
    if (!(await askInApp(t('proj.deleteConfirm', { name: p.name }), t('proj.delete')))) return;
    deleteBtn.disabled = true;
    try {
      await cmd('delete_project', { name: p.name });
      appState.projects = appState.projects.filter(project => project.name !== p.name);
      for (const [key, value] of Object.entries(appState)) {
        if (key === 'projects' || key === 'tasks' || !Array.isArray(value)) continue;
        for (const item of value) if (item?.project === p.name) item.project = '';
      }
      for (const task of appState.tasks) if (task.project === p.name) task.project = null;
      renameProjectChips(p.name, null);
      if (activeBoard === p.name) activeBoard = null;
      activeProject = null;
      document.getElementById('projects-detail').classList.add('hidden');
      document.getElementById('projects-reading-pane').classList.add('hidden');
      renderProjects();
    } catch (e) {
      deleteBtn.disabled = false;
      showToast(String(e));
    }
  };

  document.getElementById('projects-save-desc').onclick = async () => {
    const desc = document.getElementById('projects-desc').value;
    const btn = document.getElementById('projects-save-desc');
    btn.disabled = true;
    try {
      // Saving also files the emails already there that match the new description.
      const n = await cmd('save_project_description', { name: p.name, description: desc });
      showToast(t('proj.filed', { n }));
    } catch (e) {
      showToast(String(e));
    } finally {
      btn.disabled = false;
    }
  };

  const emailList = document.getElementById('projects-emails-list');
  const emails = appState.all_emails.filter(e => e.project === p.name);
  emailList.innerHTML = '';
  for (const e of emails) {
    const row = emailRow(e);
    row.addEventListener('click', () => showProjectEmail(e));
    emailList.appendChild(row);
  }
}

function showProjectEmail(email) {
  const pane = document.getElementById('projects-reading-pane');
  pane.classList.remove('hidden');
  const header = document.getElementById('proj-rp-header');
  header.innerHTML = `<div class="rp-header"><div class="rp-subject">${esc(email.subject)}</div><div class="rp-meta">${metaHtml(email)}</div><div class="rp-actions"></div></div>`;
  const iframe = document.getElementById('proj-rp-iframe');
  setIframeContent(iframe, email);
  renderActions(header.querySelector('.rp-actions'), email, () => {
    pane.classList.add('hidden');
    const p = appState.projects.find(p => p.name === activeProject);
    if (p) showProject(p);
  });
  setRead(email, true);
}

// ── Composer (floating, rich text) ─────────────────────────────────────────
let composeInReplyTo = null;   // Message-ID of the email being answered
let composeDirty = false;      // user typed something since the composer opened
let composeRange = null;       // last selection inside the body (toolbar widgets steal focus)
let composeOriginal = null;    // email being replied to / forwarded
let composeMode = 'new';
let composeAttachments = [];   // files and folders to send: { name, size, path }
let composeAttach = null;      // their target (list, picker, drop zone)

const COMPOSE_COLORS = ['#000000', '#444444', '#888888', '#cccccc', '#c62828', '#ef6c00',
  '#f9a825', '#2e7d32', '#00838f', '#1565c0', '#6a1b9a', '#ad1457'];
const MAX_IMAGE_BYTES = 5 * 1024 * 1024;

const $c = id => document.getElementById(id);

function setupCompose() {
  const body = $c('compose-body');

  $c('compose-from').addEventListener('change', () => {
    placeSignature(body, Number($c('compose-from').value) || 0);
    composeDirty = true;
  });
  $c('composer-close').addEventListener('click', () => requestCloseCompose());
  $c('compose-discard').addEventListener('click', () => requestCloseCompose(true));
  $c('compose-discard-yes').addEventListener('click', closeCompose);
  $c('compose-discard-no').addEventListener('click', () => $c('compose-discard-confirm').classList.add('hidden'));
  $c('composer-min').addEventListener('click', () => {
    $c('composer').classList.toggle('minimized');
    $c('composer').classList.remove('expanded');
  });
  $c('composer-max').addEventListener('click', () => {
    const win = $c('composer');
    win.classList.toggle('expanded');
    win.classList.remove('minimized');
    // The icon turns into a collapse one (styles.css), and so does the tooltip.
    const on = win.classList.contains('expanded');
    const max = $c('composer-max');
    max.title = t(on ? 'cmp.collapse' : 'cmp.expand');
    max.dataset.i18nTitle = on ? 'cmp.collapse' : 'cmp.expand';
  });
  // Clicking the title bar of a minimised composer restores it.
  document.querySelector('.composer-header').addEventListener('click', e => {
    if (e.target.closest('button')) return;
    $c('composer').classList.remove('minimized');
  });
  $c('compose-show-cc').addEventListener('click', () => showRecipientRow('cc'));
  $c('compose-show-bcc').addEventListener('click', () => showRecipientRow('bcc'));
  $c('compose-send').addEventListener('click', sendEmail);

  $c('composer').addEventListener('keydown', e => {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); sendEmail(); }
  });
  $c('composer').addEventListener('input', () => { composeDirty = true; });

  // Remember the selection inside the body, and light up active toolbar buttons.
  document.addEventListener('selectionchange', () => {
    const sel = window.getSelection();
    if (sel.rangeCount && body.contains(sel.anchorNode)) {
      composeRange = sel.getRangeAt(0).cloneRange();
      updateToolbarState();
    }
  });

  setupComposeToolbar();
  setupComposeAttach();
  setupComposeImages();
  setupComposeGifs();
  setupComposeAi();
  setupComposeResize();
}

function showRecipientRow(kind) {
  $c(`compose-${kind}-row`).classList.remove('hidden');
  $c(`compose-show-${kind}`).classList.add('hidden');
  $c(`compose-${kind}`).focus();
}

// Put the caret/selection back in the body before running a command.
function restoreComposeSelection() {
  const body = $c('compose-body');
  body.focus();
  if (composeRange) {
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(composeRange);
  }
}

function execCompose(command, value = null) {
  restoreComposeSelection();
  if (command === 'blockquote') {
    document.execCommand('formatBlock', false, 'blockquote');
  } else {
    document.execCommand(command, false, value);
  }
  composeDirty = true;
  updateToolbarState();
}

const STATEFUL_COMMANDS = ['bold', 'italic', 'underline', 'strikeThrough', 'justifyLeft',
  'justifyCenter', 'justifyRight', 'justifyFull', 'insertUnorderedList', 'insertOrderedList'];

function updateToolbarState() {
  document.querySelectorAll('#composer-toolbar button[data-cmd]').forEach(btn => {
    if (!STATEFUL_COMMANDS.includes(btn.dataset.cmd)) return;
    let on = false;
    try { on = document.queryCommandState(btn.dataset.cmd); } catch (_) {}
    btn.classList.toggle('active', on);
  });
}

function setupComposeToolbar() {
  const toolbar = $c('composer-toolbar');

  toolbar.querySelectorAll('button[data-cmd]').forEach(btn => {
    btn.addEventListener('mousedown', e => e.preventDefault());   // keep the selection
    btn.addEventListener('click', () => execCompose(btn.dataset.cmd));
  });
  toolbar.querySelectorAll('select[data-cmd]').forEach(sel => {
    sel.addEventListener('change', () => execCompose(sel.dataset.cmd, sel.value));
  });

  // Text colour palette
  const pop = $c('tb-color-pop');
  pop.innerHTML = COMPOSE_COLORS.map(c =>
    `<button class="tb-swatch" data-color="${c}" style="background:${c}" title="${c}"></button>`).join('');
  pop.querySelectorAll('.tb-swatch').forEach(sw => {
    sw.addEventListener('mousedown', e => e.preventDefault());
    sw.addEventListener('click', () => {
      execCompose('foreColor', sw.dataset.color);
      document.querySelector('.tb-color-a').style.borderBottomColor = sw.dataset.color;
      pop.classList.add('hidden');
    });
  });
  $c('tb-color').addEventListener('mousedown', e => e.preventDefault());
  $c('tb-color').addEventListener('click', e => {
    e.stopPropagation();
    $c('tb-link-pop').classList.add('hidden');
    pop.classList.toggle('hidden');
  });

  // Link popover
  $c('tb-link').addEventListener('mousedown', e => e.preventDefault());
  $c('tb-link').addEventListener('click', e => { e.stopPropagation(); openLinkPopover(); });
  $c('tb-link-ok').addEventListener('click', applyLink);
  $c('tb-link-remove').addEventListener('click', () => {
    const a = linkAtSelection();
    if (a) a.replaceWith(...a.childNodes);
    composeDirty = true;
    $c('tb-link-pop').classList.add('hidden');
  });
  [$c('tb-link-text'), $c('tb-link-url')].forEach(inp => inp.addEventListener('keydown', e => {
    if (e.key === 'Enter') { e.preventDefault(); applyLink(); }
    if (e.key === 'Escape') { e.stopPropagation(); $c('tb-link-pop').classList.add('hidden'); restoreComposeSelection(); }
  }));
  // Clicking a link in the body opens the popover to edit it.
  $c('compose-body').addEventListener('click', e => {
    const a = e.target.closest('a');
    if (!a) return;
    // selectionchange may not have fired yet, so point the saved range at this link.
    const range = document.createRange();
    range.selectNodeContents(a);
    composeRange = range;
    openLinkPopover();
  });

  // Popovers close on outside clicks.
  document.addEventListener('click', e => {
    if (!e.target.closest('.tb-pop-wrap')) {
      document.querySelectorAll('.tb-pop').forEach(p => p.classList.add('hidden'));
    }
  });
}

function linkAtSelection() {
  if (!composeRange) return null;
  let node = composeRange.startContainer;
  if (node.nodeType === Node.TEXT_NODE) node = node.parentNode;
  const a = node.closest && node.closest('a');
  return a && $c('compose-body').contains(a) ? a : null;
}

function openLinkPopover() {
  const a = linkAtSelection();
  $c('tb-link-text').value = a ? a.textContent : (composeRange ? composeRange.toString() : '');
  $c('tb-link-url').value = a ? a.getAttribute('href') : '';
  $c('tb-link-remove').classList.toggle('hidden', !a);
  $c('tb-color-pop').classList.add('hidden');
  $c('tb-link-pop').classList.remove('hidden');
  ($c('tb-link-text').value ? $c('tb-link-url') : $c('tb-link-text')).focus();
}

function normalizeUrl(url) {
  url = url.trim();
  if (!url) return '';
  if (/^(https?:|mailto:|tel:)/i.test(url)) return url;
  if (/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(url)) return `mailto:${url}`;
  return `https://${url}`;
}

function applyLink() {
  const url = normalizeUrl($c('tb-link-url').value);
  const text = $c('tb-link-text').value.trim() || url;
  $c('tb-link-pop').classList.add('hidden');
  if (!url) return;
  const existing = linkAtSelection();
  if (existing) {
    existing.setAttribute('href', url);
    existing.textContent = text;
  } else if (composeRange && !composeRange.collapsed && composeRange.toString() === text) {
    execCompose('createLink', url);
  } else {
    execCompose('insertHTML', `<a href="${esc(url)}">${esc(text)}</a>&nbsp;`);
  }
  composeDirty = true;
}

// #43 and #44 in the editor: the paperclip picks files or a folder, and anything dropped on
// the window joins them. An image dropped inside the text stays inline (setupComposeImages).
function setupComposeAttach() {
  composeAttach = attachTarget($c('compose-attach'), composeAttachments,
    msg => setComposeStatus(msg, true), () => { composeDirty = true; });
  const clip = $c('tb-attach');
  clip.addEventListener('mousedown', e => e.preventDefault());   // keep the body selection
  clip.addEventListener('click', () => openAttachMenu(clip, composeAttach));
  setupDropZone($c('composer'), composeAttach,
    e => (e.target.closest && e.target.closest('#compose-body')
      ? (item => !item.dir && isImageName(item.name)) : null));
}

function setupComposeImages() {
  const body = $c('compose-body');
  const fileInput = $c('tb-image-file');
  $c('tb-image').addEventListener('mousedown', e => e.preventDefault());
  $c('tb-image').addEventListener('click', () => fileInput.click());
  fileInput.addEventListener('change', () => {
    for (const f of fileInput.files) insertImageFile(f);
    fileInput.value = '';
  });
  body.addEventListener('paste', e => {
    const files = Array.from(e.clipboardData?.files || []).filter(f => f.type.startsWith('image/'));
    if (!files.length) return;
    e.preventDefault();
    files.forEach(insertImageFile);
  });
  body.addEventListener('dragover', e => {
    if (Array.from(e.dataTransfer?.types || []).includes('Files')) e.preventDefault();
  });
  body.addEventListener('drop', e => {
    const files = Array.from(e.dataTransfer?.files || []).filter(f => f.type.startsWith('image/'));
    if (!files.length) return;
    e.preventDefault();
    files.forEach(insertImageFile);
  });
}

function insertImageFile(file) {
  if (!file.type.startsWith('image/')) return;
  if (file.size > MAX_IMAGE_BYTES) {
    setComposeStatus(t('cmp.imageTooLarge', { name: file.name }), true);
    return;
  }
  const reader = new FileReader();
  reader.onload = () => execCompose('insertHTML',
    `<img src="${reader.result}" alt="${esc(file.name)}" style="max-width:100%">`);
  reader.readAsDataURL(file);
}

// GIF picker: trending on open, search as you type (debounced).
let gifSearchSeq = 0;

function setupComposeGifs() {
  const pop = $c('tb-gif-pop');
  const input = $c('tb-gif-search');
  let timer = null;

  $c('tb-gif').addEventListener('mousedown', e => e.preventDefault());
  $c('tb-gif').addEventListener('click', e => {
    e.stopPropagation();
    $c('tb-color-pop').classList.add('hidden');
    $c('tb-link-pop').classList.add('hidden');
    const opening = pop.classList.contains('hidden');
    pop.classList.toggle('hidden');
    if (opening) {
      input.focus();
      if (!$c('tb-gif-grid').children.length) loadGifs(input.value);
    }
  });
  input.addEventListener('input', () => {
    clearTimeout(timer);
    timer = setTimeout(() => loadGifs(input.value), 350);
  });
  input.addEventListener('keydown', e => {
    if (e.key === 'Enter') { e.preventDefault(); clearTimeout(timer); loadGifs(input.value); }
    if (e.key === 'Escape') { e.stopPropagation(); pop.classList.add('hidden'); restoreComposeSelection(); }
  });
}

async function loadGifs(query) {
  const grid = $c('tb-gif-grid');
  const status = $c('tb-gif-status');
  const seq = ++gifSearchSeq;
  status.textContent = t('gif.loading');
  status.classList.remove('error');
  let gifs;
  try {
    gifs = await cmd('search_gifs', { query });
  } catch (e) {
    if (seq !== gifSearchSeq) return;
    grid.innerHTML = '';
    status.textContent = String(e);
    status.classList.add('error');
    return;
  }
  if (seq !== gifSearchSeq) return;   // a newer search finished first
  status.textContent = gifs.length ? (query.trim() ? '' : t('gif.trending')) : t('gif.none');
  grid.innerHTML = '';
  for (const g of gifs) {
    const img = document.createElement('img');
    img.src = g.preview_url;
    img.alt = g.title;
    img.title = g.title;
    img.loading = 'lazy';
    img.addEventListener('mousedown', e => e.preventDefault());
    img.addEventListener('click', () => insertGif(g));
    grid.appendChild(img);
  }
}

// GIPHY's terms forbid copying its media: the email links to the GIF on GIPHY (URL kept
// exactly as returned), like other mail tools do. Where the recipient's mail client blocks
// remote images, the alt text says what it is.
function insertGif(gif) {
  const status = $c('tb-gif-status');
  try {
    $c('tb-gif-pop').classList.add('hidden');
    status.textContent = '';
    const h = gif.height || 200;
    const alt = t('gif.alt', { title: gif.title || 'GIPHY' });
    execCompose('insertHTML',
      `<img src="${esc(gif.url)}" alt="${esc(alt)}" height="${h}" style="max-width:100%;height:auto">&nbsp;`);
  } catch (e) {
    status.textContent = String(e);
    status.classList.add('error');
  }
}

// ── AI writing tools ──
function setupComposeAi() {
  const pop = $c('tb-ai-pop');
  $c('tb-ai').addEventListener('mousedown', e => e.preventDefault());
  $c('tb-ai').addEventListener('click', e => {
    e.stopPropagation();
    document.querySelectorAll('.tb-pop').forEach(p => { if (p !== pop) p.classList.add('hidden'); });
    pop.classList.toggle('hidden');
  });
  pop.querySelectorAll('button[data-ai]').forEach(btn => {
    btn.addEventListener('mousedown', e => e.preventDefault());
    btn.addEventListener('click', () => {
      pop.classList.add('hidden');
      runComposeAi(btn.dataset.ai);
    });
  });
}

// The part of the body the AI may rewrite: the selection if there is one
// (outside the quoted original), otherwise everything above the quote.
function composeAiRange(action) {
  const body = $c('compose-body');
  // The AI only touches what the user wrote: never the signature or the quoted original.
  const quoted = body.querySelector('.compose-quoted');
  const signature = body.querySelector('.compose-signature');
  const inside = el => el && el.contains(composeRange.commonAncestorContainer);
  if (action !== 'auto_reply' && composeRange && !composeRange.collapsed &&
      body.contains(composeRange.commonAncestorContainer) && !inside(quoted) && !inside(signature)) {
    return composeRange.cloneRange();
  }
  const range = document.createRange();
  range.setStart(body, 0);
  const stop = signature || quoted;   // the signature sits above the quote
  if (stop) range.setEndBefore(stop);
  else range.setEnd(body, body.childNodes.length);
  return range;
}

let composeAiBusy = false;

async function runComposeAi(action) {
  if (composeAiBusy) return;
  const body = $c('compose-body');
  const range = composeAiRange(action);

  // Images go to the AI as placeholders (data: URIs would waste tokens) and are put back after.
  const holder = document.createElement('div');
  holder.appendChild(range.cloneContents());
  const images = [];
  holder.querySelectorAll('img').forEach(img => {
    img.setAttribute('data-ai-img', images.length);
    images.push(img.getAttribute('src'));
    img.removeAttribute('src');
  });
  const hasText = holder.textContent.trim().length > 0;
  if (action !== 'auto_reply' && !hasText) {
    setComposeStatus(t('ai.writeFirst'), true);
    return;
  }

  const o = composeOriginal;
  const args = {
    action,
    html: hasText || images.length ? holder.innerHTML : '',
    originalSubject: o ? o.subject : null,
    originalFrom: o ? senderLabel(o) : null,
    originalDate: o ? (o.date_full || o.date_str) : null,
    originalText: o && action === 'auto_reply' ? (o.body_text || '') : null,
  };

  composeAiBusy = true;
  body.contentEditable = 'false';   // the range must not change while the AI works
  $c('tb-ai').disabled = true;
  setComposeStatus(`✨ ${t('ai.busy.' + action)}…`);

  let result;
  try {
    result = await cmd('ai_compose', args);
  } catch (e) {
    setComposeStatus(String(e), true);
    return;
  } finally {
    composeAiBusy = false;
    body.contentEditable = 'true';
    $c('tb-ai').disabled = false;
  }

  // Clean the answer like any foreign HTML, then restore the images.
  const out = document.createElement('div');
  out.innerHTML = sanitizeHtml(result);
  out.querySelectorAll('img').forEach(img => {
    const i = img.getAttribute('data-ai-img');
    if (i !== null && images[+i] !== undefined) {
      img.setAttribute('src', images[+i]);
      img.removeAttribute('data-ai-img');
    } else {
      img.remove();   // never keep images the AI made up
    }
  });
  let html = out.innerHTML;
  if (action === 'auto_reply' && body.querySelector('.compose-quoted')) html += '<div><br></div>';

  // Replace through execCommand so Ctrl+Z restores the previous text.
  body.focus();
  const sel = window.getSelection();
  sel.removeAllRanges();
  sel.addRange(range);
  document.execCommand('insertHTML', false, html);
  composeDirty = true;
  setComposeStatus(t('ai.done'));
}

// Drag the top-left corner to resize; the size is remembered.
function setupComposeResize() {
  const win = $c('composer');
  try {
    const saved = JSON.parse(localStorage.getItem('composerSize') || 'null');
    if (saved) { win.style.width = `${saved.w}px`; win.style.height = `${saved.h}px`; }
  } catch (_) {}

  const handle = win.querySelector('.composer-resize');
  let start = null;
  handle.addEventListener('pointerdown', e => {
    start = { x: e.clientX, y: e.clientY, w: win.offsetWidth, h: win.offsetHeight };
    handle.setPointerCapture(e.pointerId);
    e.preventDefault();
  });
  handle.addEventListener('pointermove', e => {
    if (!start) return;
    // The handle is on the outer top corner: top-left, or top-right in right-to-left languages.
    const dx = document.documentElement.dir === 'rtl' ? e.clientX - start.x : start.x - e.clientX;
    const w = Math.min(window.innerWidth - 40, Math.max(380, start.w + dx));
    const h = Math.min(window.innerHeight - 40, Math.max(320, start.h + (start.y - e.clientY)));
    win.style.width = `${w}px`;
    win.style.height = `${h}px`;
  });
  const end = () => {
    if (!start) return;
    start = null;
    try { localStorage.setItem('composerSize', JSON.stringify({ w: win.offsetWidth, h: win.offsetHeight })); } catch (_) {}
  };
  handle.addEventListener('pointerup', end);
  handle.addEventListener('pointercancel', end);
}

// Strip anything executable from an email's HTML before it enters the editor,
// which lives in the main window. DOMParser never runs scripts or loads frames.
function sanitizeHtml(html) {
  const doc = new DOMParser().parseFromString(html, 'text/html');
  doc.querySelectorAll('script, style, iframe, frame, frameset, object, embed, applet, form, input, button, textarea, select, link, meta, base, svg, math')
    .forEach(el => el.remove());
  doc.querySelectorAll('*').forEach(el => {
    for (const attr of Array.from(el.attributes)) {
      const name = attr.name.toLowerCase();
      const value = attr.value.replace(/[\s\u0000-\u001f]/g, '').toLowerCase();
      if (name.startsWith('on') || name === 'srcdoc' || name === 'formaction' ||
          (['href', 'src', 'action', 'xlink:href', 'background'].includes(name) &&
           (value.startsWith('javascript:') || value.startsWith('vbscript:') || value.startsWith('data:text')))) {
        el.removeAttribute(attr.name);
      }
    }
  });
  return doc.body.innerHTML;
}

function quotedOriginal(email) {
  return email.body_html
    ? sanitizeHtml(email.body_html)
    : esc(email.body_text || '').replace(/\n/g, '<br>');
}

// "On {date}, {from} wrote:" with the date and sender isolated (<bdi>), so an address
// like "Name <a@b>" keeps its order inside a right-to-left sentence.
function quoteIntro(email) {
  const text = esc(t('cmp.wrote', { date: '\u0001', from: '\u0002' }));
  return text
    .replace('\u0001', `<bdi>${esc(emailDateFull(email))}</bdi>`)
    .replace('\u0002', `<bdi>${esc(senderLabel(email))}</bdi>`);
}

function senderLabel(email) {
  return email.from_email && email.from_email !== email.from_name
    ? `${email.from_name} <${email.from_email}>`
    : email.from_name;
}

// openCompose({ to, subject })           new message with prefilled fields
// openCompose()                         new message
// openCompose({ mode: 'reply', email })  reply with the original quoted
// openCompose({ mode: 'forward', email }) forward with the original below
// Each call opens a new window, so several emails can be written at the same time.
function openCompose(options = {}) {
  const { mode = 'new', email = null, subject = '' } = options;
  const title = email && mode === 'reply' ? `Re: ${email.subject}`
    : email && mode === 'forward' ? `Fwd: ${email.subject}`
    : subject || t('cmp.newMessage');
  cmd('open_compose_window', { args: options, title }).catch(e => showToast(String(e)));
}

// Runs in a composer window: the composer fills the window, nothing else is set up.
async function startComposeWindow(options) {
  document.body.classList.add('compose-window');
  initLang();
  applyStaticI18n();
  updateTranslateButton();
  setupCompose();
  try {
    const s = await invoke('get_settings');
    fontScale = s.font_scale || 1.0;
    document.documentElement.style.fontSize = `${fontScale * 14}px`;
    applyTheme(s.theme);
    if (s.language && s.language !== LANG) applyLanguage(s.language);
  } catch (_) {}
  await loadComposeAccounts();
  fillCompose(options);

  const win = window.__TAURI__.window.getCurrentWindow();
  const setTitle = () => win.setTitle($c('compose-subject').value.trim() || t('cmp.newMessage')).catch(() => {});
  $c('compose-subject').addEventListener('input', setTitle);
  setTitle();
  // Closing the window with something typed asks first, like the discard button.
  win.onCloseRequested(e => {
    if (composeDirty && !composeClosing) { e.preventDefault(); requestCloseCompose(); }
  });
}
let composeClosing = false;   // the composer window is closing on purpose (sent or discarded)

// The original below a reply: "On …, … wrote:" and the quoted message.
function replyQuote(email) {
  return `
      <div class="compose-quoted">
      <div class="quote-intro">${quoteIntro(email)}</div>
      <blockquote style="margin:0 0 0 .8ex;border-left:1px solid #ccc;padding-left:1ex">${quotedOriginal(email)}</blockquote>
      </div>`;
}

// A reply can come prefilled (the Discussion mode reply bar expanding into the full editor):
// `to`, `cc`, `bcc`, `subject`, `account` and the typed `html` replace the defaults.
function fillCompose({ mode = 'new', email = null, to: presetTo = '', subject: presetSubject = '',
                       cc: presetCc = '', bcc: presetBcc = '', html: presetHtml = '',
                       inReplyTo: presetInReplyTo = null, account: presetAccount = null,
                       attachments: presetAttachments = [] } = {}) {
  let to = presetTo, subject = presetSubject, html = presetHtml || '<div><br></div>';
  composeInReplyTo = presetInReplyTo;

  if (email && mode === 'reply') {
    to = presetTo || email.reply_to || email.from_email || email.from_name;
    subject = presetSubject || (/^re:/i.test(email.subject) ? email.subject : `Re: ${email.subject}`);
    composeInReplyTo = email.message_id || null;
    html = (presetHtml || '<div><br></div><div><br></div>') + replyQuote(email);
  } else if (email && mode === 'forward') {
    subject = /^(fwd?|tr):/i.test(email.subject) ? email.subject : `Fwd: ${email.subject}`;
    const rows = [
      [t('meta.from'), senderLabel(email)],
      [t('meta.date'), emailDateFull(email)],
      [t('meta.subject'), email.subject],
      [t('meta.to'), email.to],
      [t('meta.cc'), email.cc],
    ].filter(([, v]) => v).map(([k, v]) => `${k}: ${esc(v)}<br>`).join('');
    html = `<div><br></div><div><br></div>
      <div class="compose-quoted">
      <div>${t('cmp.forwarded')}<br>${rows}</div><br>
      <div>${quotedOriginal(email)}</div>
      </div>`;
  }

  // Sending account: the one the original was sent to, else the first. Its signature goes
  // above the quoted original in replies/forwards, at the end of new messages.
  const accountIndex = presetAccount ?? (email ? accountForEmail(email) : 0);
  $c('compose-from').value = String(accountIndex);
  const sigHtml = signatureBlock(accountIndex, email ? 'reply' : 'new');
  if (sigHtml) {
    const quoteStart = html.indexOf('<div class="compose-quoted">');
    html = quoteStart < 0 ? html + sigHtml : html.slice(0, quoteStart) + sigHtml + html.slice(quoteStart);
  }

  composeOriginal = email;
  composeMode = email ? mode : 'new';
  // Files brought from the reply bar, or none for a new message.
  composeAttachments.length = 0;
  if (Array.isArray(presetAttachments)) {
    presetAttachments.forEach(a => { if (a && a.name && a.path) composeAttachments.push(a); });
  }
  if (composeAttach) composeAttach.show();
  // Auto answer needs an email to answer.
  $c('tb-ai-auto').disabled = composeMode !== 'reply';
  $c('tb-ai-auto').title = composeMode === 'reply' ? t('ai.autoTitle') : t('ai.autoDisabled');

  $c('compose-to').value = to;
  $c('compose-cc').value = '';
  $c('compose-bcc').value = '';
  ['cc', 'bcc'].forEach(k => {
    $c(`compose-${k}-row`).classList.add('hidden');
    $c(`compose-show-${k}`).classList.remove('hidden');
  });
  Object.entries({ cc: presetCc, bcc: presetBcc }).filter(([, v]) => v).forEach(([k, v]) => {
    $c(`compose-${k}`).value = v;
    $c(`compose-${k}-row`).classList.remove('hidden');
    $c(`compose-show-${k}`).classList.add('hidden');
  });
  $c('compose-subject').value = subject;
  $c('compose-body').innerHTML = html;
  $c('composer-title').textContent =
    mode === 'reply' ? subject : mode === 'forward' ? subject : t('cmp.newMessage');
  $c('compose-discard-confirm').classList.add('hidden');
  setComposeStatus('');
  $c('compose-send').disabled = false;

  const win = $c('composer');
  win.classList.remove('hidden', 'minimized');
  composeDirty = false;
  composeRange = null;

  // Reply, or recipient and subject already filled: start typing the message.
  if (mode === 'reply' || (to && subject)) {
    // Caret at the very top of the body, above the quote.
    const body = $c('compose-body');
    body.focus();
    const range = document.createRange();
    range.setStart(body.firstChild || body, 0);
    range.collapse(true);
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
    body.scrollTop = 0;
  } else {
    $c('compose-to').focus();
  }
}

function requestCloseCompose(force = false) {
  if (composeDirty || force) {
    $c('compose-discard-confirm').classList.remove('hidden');
  } else {
    closeCompose();
  }
}

function closeCompose() {
  if (window.__COMPOSE__) {
    composeClosing = true;
    window.__TAURI__.window.getCurrentWindow().close();
    return;
  }
  $c('composer').classList.add('hidden');
  $c('composer').classList.remove('minimized', 'expanded');
  $c('composer-max').title = t('cmp.expand');
  $c('composer-max').dataset.i18nTitle = 'cmp.expand';
  $c('compose-body').innerHTML = '';
  composeDirty = false;
  composeAttachments.length = 0;
  if (composeAttach) composeAttach.show();
}

function setComposeStatus(msg, isError = false) {
  const st = $c('compose-status');
  st.textContent = msg;
  st.classList.toggle('error', isError);
}

async function sendEmail() {
  const to = $c('compose-to').value.trim();
  const cc = $c('compose-cc').value.trim();
  const bcc = $c('compose-bcc').value.trim();
  const subject = $c('compose-subject').value.trim();
  const html = $c('compose-body').innerHTML;
  if (!to && !cc && !bcc) { setComposeStatus(t('cmp.needRecipient'), true); $c('compose-to').focus(); return; }

  const btn = $c('compose-send');
  btn.disabled = true;
  setComposeStatus(t('cmp.sending'));
  try {
    await cmd('send_email', { to, cc, bcc, subject, html, inReplyTo: composeInReplyTo,
                              account: Number($c('compose-from').value) || 0,
                              attachments: composeAttachments.map(a => a.path) });
    if (window.__COMPOSE__) window.__TAURI__.event.emit('compose-sent').catch(() => {});
    else showToast(t('cmp.sent'));
    closeCompose();
  } catch (e) {
    btn.disabled = false;
    setComposeStatus(t('cmp.sendFailed', { err: e }), true);
  }
}

function showToast(msg) {
  let toast = document.getElementById('toast');
  if (!toast) {
    toast = document.createElement('div');
    toast.id = 'toast';
    document.body.appendChild(toast);
  }
  toast.textContent = msg;
  toast.classList.add('show');
  clearTimeout(toast._t);
  toast._t = setTimeout(() => toast.classList.remove('show'), 3000);
}

// ── Settings ───────────────────────────────────────────────────────────────
let settingsData = null;

// ── Themes ──
const THEMES = [
  { id: 'midnight',  name: 'Midnight' },
  { id: 'premium',   name: 'Premium' },
  { id: 'modern',    name: 'Modern' },
  { id: 'brutalist', name: 'Brutalist' },
  { id: 'aero',      name: 'Frutiger Aero' },
  { id: 'terminal',  name: 'Terminal' },
];
let savedTheme = 'brutalist';      // last saved; restored if Settings closes without saving
let savedFontScale = 1.0;

function applyTheme(id) {
  if (id === 'white') id = 'premium';   // Premium replaced the White theme
  if (!THEMES.some(t => t.id === id)) id = 'brutalist';
  document.documentElement.dataset.theme = id;
  try { localStorage.setItem('theme', id); } catch (_) {}
  document.querySelectorAll('.theme-card').forEach(c =>
    c.classList.toggle('selected', c.dataset.theme === id));
  if (appState && activePanel === 'discussions') renderDiscussions();   // the post office wording
}

function renderThemePicker() {
  const grid = document.getElementById('s-theme-grid');
  grid.innerHTML = '';
  const current = document.documentElement.dataset.theme;
  for (const theme of THEMES) {
    // Each card carries its own data-theme, so it previews that theme's variables.
    const card = document.createElement('button');
    card.type = 'button';
    card.className = 'theme-card' + (theme.id === current ? ' selected' : '');
    card.dataset.theme = theme.id;
    card.innerHTML = `
      <div class="theme-card-preview">
        <div class="theme-card-side"></div>
        <div class="theme-card-main"><div class="theme-card-bar"></div><div class="theme-card-paper"></div></div>
      </div>
      <span class="theme-card-name">${esc(t('theme.' + theme.id))}</span>`;
    card.addEventListener('click', () => applyTheme(theme.id));   // live preview
    grid.appendChild(card);
  }
}

// Switch the UI language: static texts, then everything rendered from data.
// The composer's second translate option targets the UI language (French when the UI is English).
function updateTranslateButton() {
  const btn = document.getElementById('tb-ai-translate-ui');
  if (!btn) return;
  if (LANG === 'en') {
    btn.dataset.ai = 'translate_fr';
    btn.textContent = t('ai.translateFr');
  } else {
    btn.dataset.ai = 'translate_ui';
    btn.textContent = t('ai.translateUi', { lang: languageName() });
  }
}

function applyLanguage(code) {
  setLang(code);
  applyStaticI18n();
  updateTranslateButton();
  if (appState) renderAll();
  if (!document.getElementById('settings-overlay').classList.contains('hidden')) {
    renderThemePicker();
    stashAiFields();
    showAiProvider();
    settingsData.accounts = collectAccounts();   // keep unsaved edits while re-rendering labels
    renderAccountBlocks(settingsData.accounts);
  }
  if (!document.getElementById('setup-overlay').classList.contains('hidden')) {
    renderSetupHint(document.getElementById('setup-email').value);
    if (!document.getElementById('setup-scan').classList.contains('hidden')) renderScanRows();
    if (!document.getElementById('setup-result').classList.contains('hidden')) {
      renderSetupArea(document.getElementById('setup-email').value);
    }
  }
}

let savedLang = 'en';   // last saved; restored if Settings closes without saving

async function openSettings() {
  settingsData = await cmd('get_settings');
  // Passwords shown last time are hidden again.
  document.querySelectorAll('#settings-overlay input[data-pw]').forEach(i => { i.type = 'password'; });
  document.querySelectorAll('#settings-overlay .pw-toggle.on').forEach(b => b.classList.remove('on'));
  savedTheme = settingsData.theme || 'brutalist';
  savedLang = settingsData.language || 'en';
  const langSelect = document.getElementById('s-language');
  langSelect.innerHTML = LANGUAGES.map(l => `<option value="${l.id}">${esc(l.name)}</option>`).join('');
  langSelect.value = LANG;
  document.getElementById('s-notify').checked = settingsData.notify_new_mail !== false;
  document.getElementById('s-discussion').checked = !!settingsData.discussion_mode;
  savedFontScale = settingsData.font_scale || 1.0;
  renderThemePicker();
  fontScale = settingsData.font_scale;
  const slider = document.getElementById('s-font-scale');
  slider.value = fontScale;
  document.getElementById('s-font-scale-val').textContent = fontScale.toFixed(2) + '×';
  loadAiSettings(settingsData.ai);
  document.getElementById('s-giphy-key').value = settingsData.giphy_key || '';
  document.getElementById('s-cloud-url').value = settingsData.supabase_url || '';
  document.getElementById('s-cloud-anon').value = settingsData.supabase_anon_key || '';
  document.getElementById('s-cloud-password').value = '';
  setCloudStatus('');
  renderCloudStatus(await cmd('cloud_status'));
  document.getElementById('s-update-auto').checked = autoUpdatesEnabled();
  document.getElementById('s-update-current').textContent = await cmd('app_version');

  renderAccountBlocks(settingsData.accounts);
  settingsBaseline = settingsSnapshot();
  document.getElementById('settings-overlay').classList.remove('hidden');
  // Default to general tab
  switchSettingsTab('general');
}

// "Remi Lhoste <remi@x.fr>" → { name: 'Remi Lhoste', email: 'remi@x.fr' }; a bare address has no name.
function splitSender(from) {
  const m = /^\s*"?([^"<]*?)"?\s*<([^>]*)>\s*$/.exec(from || '');
  return m ? { name: m[1].trim(), email: m[2].trim() } : { name: '', email: (from || '').trim() };
}

const isEmail = s => /^[^\s@]+@[^\s@]+$/.test((s || '').trim());

// The sender address comes from a login that is an email (sending first); else it must be typed.
function loginEmail(block) {
  return [block.querySelector('.a-smtp-user').value, block.querySelector('.a-imap-user').value]
    .map(s => s.trim()).find(isEmail) || '';
}

// Show the address field only when no login is an email address, or when it holds
// another address than the login (an alias). Updates the block title too.
function refreshSenderEmail(block) {
  const typed = block.querySelector('.a-sender-email').value.trim();
  const login = loginEmail(block);
  block.querySelector('.sender-email-row').classList.toggle('hidden', !!login && (!typed || typed === login));
  block.querySelector('.account-title').textContent =
    typed || login || t('set.account', { n: +block.dataset.idx + 1 });
}

function renderAccountBlocks(accounts) {
  const container = document.getElementById('s-accounts-container');
  container.innerHTML = '';
  accounts.forEach((acc, idx) => {
    const block = document.createElement('div');
    block.className = 'account-block';
    block.dataset.idx = idx;
    block.dataset.label = acc.label;
    block.dataset.sigFile = acc.signature_file || '';
    const sender = splitSender(acc.smtp_from);
    block.innerHTML = `
      <div class="account-block-title">
        <span class="account-title"></span>
        <div class="account-actions">
          <button type="button" class="collapse-account-btn" title="${esc(t('set.collapse'))}" aria-expanded="true">▾</button>
          <button type="button" class="remove-account-btn" data-idx="${idx}" title="${esc(t('set.removeAccount'))}" aria-label="${esc(t('set.removeAccount'))}">✕</button>
        </div>
      </div>
      <label>${t('set.yourName')}</label>
      <input type="text" class="a-sender-name" value="${esc(sender.name)}" placeholder="${esc(t('set.yourNamePh'))}" />
      <div class="sender-email-row">
        <label>${t('set.senderEmail')}</label>
        <input type="text" class="a-sender-email" value="${esc(sender.email)}" placeholder="${esc(t('set.usernamePh'))}" />
        <p class="settings-note">${t('set.senderEmailHint')}</p>
      </div>

      <fieldset class="account-section">
        <legend>${t('set.incoming')} <span class="conn-status" data-kind="imap"></span></legend>
        <div class="field-row">
          <div class="field-grow"><label>${t('set.server')}</label>
            <input type="text" class="a-imap-host" value="${esc(acc.imap_host)}" placeholder="imap.example.com" /></div>
          <div class="field-port"><label>${t('set.port')}</label>
            <input type="text" class="a-imap-port" inputmode="numeric" value="${esc(acc.imap_port)}" placeholder="993" /></div>
        </div>
        <label>${t('set.username')}</label>
        <input type="text" class="a-imap-user" value="${esc(acc.imap_user)}" placeholder="${esc(t('set.usernamePh'))}" autocomplete="off" />
        <label>${t('set.password')}</label>
        <input type="password" class="a-imap-pass" value="${esc(acc.imap_pass)}" placeholder=" " autocomplete="new-password" />
        <p class="conn-error" data-kind="imap"></p>
      </fieldset>

      <fieldset class="account-section">
        <legend>${t('set.outgoing')} <span class="conn-status" data-kind="smtp"></span></legend>
        <div class="field-row">
          <div class="field-grow"><label>${t('set.server')}</label>
            <input type="text" class="a-smtp-host" value="${esc(acc.smtp_host)}" placeholder="smtp.example.com" /></div>
          <div class="field-port"><label>${t('set.port')}</label>
            <input type="text" class="a-smtp-port" inputmode="numeric" value="${esc(acc.smtp_port)}" placeholder="587" /></div>
        </div>
        <label>${t('set.username')}</label>
        <input type="text" class="a-smtp-user" value="${esc(acc.smtp_user)}" placeholder="${esc(t('set.sameAsIncoming'))}" autocomplete="off" />
        <label>${t('set.password')}</label>
        <input type="password" class="a-smtp-pass" value="${esc(acc.smtp_pass)}" placeholder="${esc(t('set.sameAsIncoming'))}" autocomplete="new-password" />
        <p class="conn-error" data-kind="smtp"></p>
      </fieldset>

      <div class="db-test-row">
        <button type="button" class="secondary-btn account-test-btn">${t('set.testConn')}</button>
      </div>

      <details class="account-advanced">
        <summary>${t('set.advanced')} <span class="sig-marker hidden">${t('set.sigMarker')}</span></summary>
        <label>${t('set.imapMailbox')}</label>
        <input type="text" class="a-imap-mailbox" value="${esc(acc.imap_mailbox)}" placeholder="INBOX" />
        <label>${t('set.fetchCount')}</label>
        <input type="text" class="a-imap-fetch" inputmode="numeric" value="${esc(acc.imap_fetch)}" placeholder="50" />

        <div class="signature-section">
          <label>${t('set.signature')}</label>
          <iframe class="sig-preview hidden" sandbox="" title="${esc(t('set.signature'))}"></iframe>
          <p class="settings-note sig-hint">${t('set.sigHint')}</p>
          <div class="db-test-row">
            <button type="button" class="secondary-btn sig-choose">${t('set.sigChoose')}</button>
            <button type="button" class="secondary-btn sig-remove hidden">${t('set.sigRemove')}</button>
            <span class="db-status sig-status"></span>
          </div>
          <label class="checkbox-label"><input type="checkbox" class="a-sig-on-new" ${acc.signature_on_new ? 'checked' : ''} /> <span>${t('set.sigOnNew')}</span></label>
          <label class="checkbox-label"><input type="checkbox" class="a-sig-on-reply" ${acc.signature_on_reply ? 'checked' : ''} /> <span>${t('set.sigOnReply')}</span></label>
        </div>
      </details>
    `;
    // ▾ / ▸ shrinks the account to its title line (clicking the title does the same).
    const toggle = () => {
      const collapsed = block.classList.toggle('collapsed');
      const btn = block.querySelector('.collapse-account-btn');
      btn.textContent = collapsed ? '▸' : '▾';
      btn.title = t(collapsed ? 'set.expand' : 'set.collapse');
      btn.setAttribute('aria-expanded', String(!collapsed));
    };
    block.querySelector('.collapse-account-btn').addEventListener('click', toggle);
    block.querySelector('.account-title').addEventListener('click', toggle);
    block.querySelector('.remove-account-btn').addEventListener('click', async () => {
      // A block with nothing typed in it goes without asking; any other asks first.
      const filled = [...block.querySelectorAll('input:not([type=checkbox])')].some(i => i.value.trim());
      const name = block.querySelector('.account-title').textContent.trim();
      if (filled && !(await askInApp(t('set.removeAccountConfirm', { name }), t('set.removeAccount')))) return;
      settingsData.accounts = collectAccounts();   // keep what was typed in the other blocks
      settingsData.accounts.splice(idx, 1);
      renderAccountBlocks(settingsData.accounts);
    });
    block.querySelectorAll('.a-smtp-user, .a-imap-user, .a-sender-email')
      .forEach(el => el.addEventListener('input', () => refreshSenderEmail(block)));
    block.querySelector('.account-test-btn').addEventListener('click', () => testAccount(block));
    // A result no longer holds once the fields change (blank SMTP fields reuse the IMAP ones).
    block.querySelectorAll('.account-section').forEach((fs, i) => fs.addEventListener('input', () => {
      setConnStatus(block, 'smtp', '');
      if (i === 0) setConnStatus(block, 'imap', '');
    }));
    refreshSenderEmail(block);
    addPasswordToggles(block);
    setupAccountSignature(block);
    container.appendChild(block);
  });
}

// ── Settings → Accounts → connection test ──
// state: '' (not tested), 'busy', 'ok' or 'error'; `msg` is the tooltip / error sentence,
// `detail` the server's own message, shown smaller under an error.
function setConnStatus(block, kind, state, msg = '', detail = '') {
  const icon = block.querySelector(`.conn-status[data-kind="${kind}"]`);
  icon.textContent = { busy: '⏳', ok: '✅', error: '❌' }[state] || '';
  icon.title = detail ? `${msg}\n${detail}` : msg;
  icon.dataset.state = state;
  const err = block.querySelector(`.conn-error[data-kind="${kind}"]`);
  err.innerHTML = state === 'error' ? `${esc(msg)}${detail ? `<small>${esc(detail)}</small>` : ''}` : '';
}

// The backend's ConnError { kind, detail } → a sentence in the UI language.
function connErrorText(e) {
  const kind = e && e.kind ? e.kind : 'other';
  return [t(`set.connErr.${kind}`), e && e.kind ? e.detail : String(e)];
}

async function testAccount(block) {
  const account = collectAccount(block);
  const btn = block.querySelector('.account-test-btn');
  btn.disabled = true;
  setConnStatus(block, 'imap', 'busy', t('set.testing'));
  setConnStatus(block, 'smtp', 'busy', t('set.testing'));
  await Promise.all([
    invoke('account_test_imap', { account })
      .then(n => setConnStatus(block, 'imap', 'ok', t('set.imapOk', { n, folder: account.imap_mailbox || 'INBOX' })))
      .catch(e => setConnStatus(block, 'imap', 'error', ...connErrorText(e))),
    invoke('account_test_smtp', { account })
      .then(() => setConnStatus(block, 'smtp', 'ok', t('set.smtpOk')))
      .catch(e => setConnStatus(block, 'smtp', 'error', ...connErrorText(e))),
  ]);
  btn.disabled = false;
}

// An eye button next to each password field shows or hides what was typed.
function addPasswordToggles(root) {
  root.querySelectorAll('input[type="password"]:not([data-pw])').forEach(input => {
    input.dataset.pw = '1';
    const wrap = document.createElement('span');
    wrap.className = 'pw-wrap';
    input.replaceWith(wrap);
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'pw-toggle';
    btn.dataset.i18nTitle = 'set.togglePassword';
    btn.title = t('set.togglePassword');
    btn.textContent = '👁';
    btn.addEventListener('click', () => {
      const show = input.type === 'password';
      input.type = show ? 'text' : 'password';
      btn.classList.toggle('on', show);
    });
    wrap.append(input, btn);
  });
}

function setupSettings() {
  addPasswordToggles(document.getElementById('settings-overlay'));
  // Only the × closes Settings: a click beside the window must not lose what was typed.
  // With unsaved changes it asks first, listing them.
  document.getElementById('settings-close').addEventListener('click', askCloseSettings);
  document.getElementById('unsaved-save').addEventListener('click', () => { hideUnsaved(); saveSettings(); });
  document.getElementById('unsaved-discard').addEventListener('click', () => { hideUnsaved(); closeSettings(); });
  document.getElementById('unsaved-back').addEventListener('click', hideUnsaved);
  keepFocusIn(document.getElementById('unsaved-overlay'), hideUnsaved);
  keepFocusIn(document.getElementById('confirm-overlay'), () => document.getElementById('confirm-cancel').click());

  document.querySelectorAll('.tab').forEach(t => {
    t.addEventListener('click', () => switchSettingsTab(t.dataset.tab));
  });

  document.getElementById('s-font-scale').addEventListener('input', e => {
    fontScale = parseFloat(e.target.value);
    document.getElementById('s-font-scale-val').textContent = fontScale.toFixed(2) + '×';
    document.documentElement.style.fontSize = `${fontScale * 14}px`;
  });

  document.getElementById('s-add-account').addEventListener('click', openSetupDialog);

  // The setup dialog: name, email and password, then Auto-setup looks for the servers.
  addPasswordToggles(document.getElementById('setup-overlay'));
  document.getElementById('setup-close').addEventListener('click', closeSetupDialog);
  document.getElementById('setup-next').addEventListener('click', setupNext);
  document.getElementById('setup-email').addEventListener('input', e => {
    renderSetupHint(e.target.value);
    document.getElementById('setup-result').classList.add('hidden');   // the result was for the old address
  });
  document.getElementById('setup-use').addEventListener('click', () => setupAddAccount(true));
  document.getElementById('setup-manual').addEventListener('click', () => setupAddAccount(false));
  // Step 1: the accounts Auto-scan found in another mail app.
  document.getElementById('setup-scan-add').addEventListener('click', setupScanAdd);
  document.getElementById('setup-scan-skip').addEventListener('click', setupScanSkip);
  // Enter goes on with what was typed: it adds the ticked accounts on step 1.
  document.getElementById('setup-overlay').addEventListener('keydown', e => {
    if (e.key !== 'Enter' || e.target.tagName === 'A') return;
    if (!document.getElementById('setup-scan').classList.contains('hidden')) { setupScanAdd(); return; }
    if (document.getElementById('setup-result').classList.contains('hidden')) setupNext();
  });

  document.getElementById('settings-save').addEventListener('click', saveSettings);
  document.getElementById('s-language').addEventListener('change', e => applyLanguage(e.target.value));   // live preview
  setupCloudSettings();
  setupAiSettings();
  setupUpdateSettings();

  document.getElementById('s-show-qr').addEventListener('click', async () => {
    const qr = await cmd('get_qr');
    const out = document.getElementById('s-qr-output');
    out.textContent = qr;
    out.style.display = out.style.display === 'none' ? 'block' : 'none';
  });
}

function autoUpdatesEnabled() {
  return CourrielCore.autoUpdatesEnabled(localStorage);
}

function setupUpdateSettings() {
  document.getElementById('s-update-auto').addEventListener('change', e => {
    CourrielCore.saveAutoUpdates(localStorage, e.target.checked);
  });
  document.getElementById('s-update-check').addEventListener('click', () => checkForUpdates(true));
  document.getElementById('s-update-install').addEventListener('click', installAvailableUpdate);
}

async function checkForUpdates(manual = false) {
  const checkBtn = document.getElementById('s-update-check');
  const installBtn = document.getElementById('s-update-install');
  const status = document.getElementById('s-update-status');
  const notes = document.getElementById('s-update-notes');
  if (manual) {
    checkBtn.disabled = true;
    status.textContent = t('set.updateChecking');
    notes.classList.add('hidden');
  }
  try {
    const info = await cmd('check_for_update');
    document.getElementById('s-update-current').textContent = info.current_version;
    if (!info.available) {
      installBtn.classList.add('hidden');
      if (manual) status.textContent = t('set.updateLatest');
      return info;
    }
    installBtn.classList.remove('hidden');
    status.textContent = t('set.updateAvailable', { version: info.version });
    notes.textContent = info.notes || '';
    notes.classList.toggle('hidden', !info.notes);
    return info;
  } catch (e) {
    if (manual) status.textContent = t('set.updateError', { err: String(e) });
    else console.warn('automatic update check', e);
    return null;
  } finally {
    checkBtn.disabled = false;
  }
}

async function installAvailableUpdate() {
  const checkBtn = document.getElementById('s-update-check');
  const installBtn = document.getElementById('s-update-install');
  const status = document.getElementById('s-update-status');
  checkBtn.disabled = true;
  installBtn.disabled = true;
  status.textContent = t('set.updateDownloading');
  document.getElementById('s-update-progress').classList.remove('hidden');
  try {
    await cmd('install_update');
  } catch (e) {
    status.textContent = t('set.updateError', { err: String(e) });
    checkBtn.disabled = false;
    installBtn.disabled = false;
  }
}

function renderUpdateProgress(progress) {
  const bar = document.getElementById('s-update-progress');
  const fill = bar.querySelector('div');
  bar.classList.remove('hidden');
  const percent = CourrielCore.updateProgressPercent(progress);
  if (progress.finished) {
    fill.style.width = `${percent}%`;
    document.getElementById('s-update-status').textContent = t('set.updateRestarting');
  } else if (percent !== null) {
    fill.style.width = `${percent}%`;
  }
}

async function checkAutomaticUpdates() {
  if (!autoUpdatesEnabled()) return;
  const info = await checkForUpdates(false);
  if (info?.available) installAvailableUpdate();
}

function switchSettingsTab(tab) {
  document.querySelectorAll('.tab').forEach(t => t.classList.toggle('active', t.dataset.tab === tab));
  document.querySelectorAll('.tab-content').forEach(c => c.classList.toggle('active', c.id === `tab-${tab}`));
}

// ── Account setup dialog (Auto-setup) ───────────────────────────────────────
const GMAIL_APP_PASSWORDS_URL = 'https://myaccount.google.com/apppasswords';
// Google only offers app passwords once 2-Step Verification is on; before that its page
// says "the setting you are looking for is not available for your account".
const GMAIL_2SV_URL = 'https://myaccount.google.com/signinoptions/twosv';

let setupFound;   // what Auto-setup returned: undefined = not asked yet, null = nothing found
let setupNote;    // { key, vars }: one quiet line shown instead of a result

// What a provider needs beyond a password, in one place (Auto-scan reuses this):
// 'gmail' → an app password, 'ms' → no password any more, so not supported yet.
// The key is an email address or an incoming server's host; null = nothing special.
function providerHint(key) {
  const s = (key || '').toLowerCase().trim();
  const labels = (s.includes('@') ? s.slice(s.lastIndexOf('@') + 1) : s).split('.');
  const listed = list => labels.some(l => list.some(d => l === d || l.startsWith(`${d}-`)));
  if (listed(['gmail', 'googlemail', 'google'])) return 'gmail';
  if (listed(['outlook', 'hotmail', 'live', 'msn', 'office365', 'microsoftonline'])) return 'ms';
  return null;
}

// The note a provider needs beyond a password, written into the element it is given:
// Auto-setup's password field and every Auto-scan row have one of their own.
function fillHint(el, key) {
  const kind = providerHint(key);
  el.innerHTML = '';
  el.classList.toggle('hidden', !kind);
  if (!kind) return;
  el.textContent = t(kind === 'gmail' ? 'setup.hint.gmail' : 'setup.hint.ms');
  if (kind !== 'gmail') return;
  // The two steps, in order, each one a link to its Google page.
  [['setup.hint.gmail2sv', GMAIL_2SV_URL], ['setup.hint.gmailLink', GMAIL_APP_PASSWORDS_URL]].forEach(([k, url]) => {
    const link = document.createElement('a');
    link.className = 'setup-link';
    link.href = url;
    link.textContent = t(k);
    // Opens in the system browser (Tauri shell plugin), not inside the app window.
    link.addEventListener('click', e => {
      e.preventDefault();
      window.__TAURI__.shell.open(url).catch(err => console.error('open Google page', err));
    });
    const step = document.createElement('span');
    step.className = 'setup-step';
    step.append(link);
    el.append(step);
  });
}

// What the hint is about: a custom domain hosted by Google or Microsoft only shows who it
// is through its incoming server.
function hintKey(email, host) {
  return providerHint(email) ? email : (host || '');
}

// The note under the password field.
function renderSetupHint(key) {
  fillHint(document.getElementById('setup-hint'), key);
}

// Step 1 (Auto-scan) when another mail app holds an account this one does not have yet,
// step 2 (Auto-setup) otherwise.
async function openSetupDialog() {
  setupFound = undefined;
  setupNote = null;
  document.getElementById('setup-name').value = '';
  document.getElementById('setup-email').value = '';
  const pass = document.getElementById('setup-pass');
  pass.value = '';
  pass.type = 'password';   // the eye button keeps its place
  document.getElementById('setup-next').disabled = false;
  document.getElementById('setup-result').classList.add('hidden');
  renderSetupHint('');
  scanFound = await scanMailClients();
  scanBoxes = [];
  renderScanStep();
  document.getElementById('setup-overlay').classList.remove('hidden');
  document.getElementById(scanFound.length ? 'setup-scan-add' : 'setup-email').focus();
}

function closeSetupDialog() {
  document.getElementById('setup-overlay').classList.add('hidden');
}

// ── Auto-scan ─────────────────────────────────────────────────────────────────

let scanFound = [];   // the accounts the other mail apps hold, none of them in this app
let scanBoxes = [];   // their checkboxes, in the same order

// The accounts Auto-scan found, less the ones already set up here. A mail app that is not
// installed gives nothing, so the dialog opens on Auto-setup as if none was found.
async function scanMailClients() {
  let found;
  try {
    found = await cmd('scan_mail_clients', {});
  } catch (e) {
    console.error('scan mail clients', e);
    return [];
  }
  const known = new Set(collectAccounts()
    .flatMap(a => [splitSender(a.smtp_from).email, a.imap_user])
    .map(s => (s || '').trim().toLowerCase())
    .filter(Boolean));
  return (found || []).filter(a => !known.has((a.email || '').toLowerCase()));
}

// Which step to show: the list only when it has something to show.
function renderScanStep() {
  const found = !!scanFound.length;
  document.getElementById('setup-scan').classList.toggle('hidden', !found);
  document.getElementById('setup-form').classList.toggle('hidden', found);
  if (found) renderScanRows();
}

// One row per account: its name and address, the app it was found in, what it needs and
// what it cannot do. Ticking is kept when the labels are redrawn in another language.
function renderScanRows() {
  const was = scanBoxes.map(b => b.checked);
  const list = document.getElementById('setup-scan-list');
  list.innerHTML = '';
  scanBoxes = [];
  scanFound.forEach((a, i) => {
    const key = hintKey(a.email, a.imap ? a.imap.host : '');
    const row = document.createElement('label');
    row.className = 'setup-scan-row';
    const box = document.createElement('input');
    box.type = 'checkbox';
    // Everything usable starts ticked, except what no longer accepts a password.
    box.checked = was[i] === undefined ? (!a.pop3 && !!a.imap && providerHint(key) !== 'ms') : was[i];
    // POP3, or an incoming server this app cannot reach: nothing a block could use.
    if (a.pop3 || !a.imap) {
      box.checked = false;
      box.disabled = true;
      row.classList.add('disabled');
    }
    const main = document.createElement('span');
    main.className = 'setup-scan-main';
    const name = document.createElement('span');
    name.className = 'setup-scan-name';
    name.textContent = `${a.label} \u2014 ${a.email}`;
    const client = document.createElement('span');
    client.className = 'setup-scan-client';
    client.textContent = a.client;
    main.append(name, client);
    const problems = (a.pop3 ? t('setup.scan.pop3') + ' ' : '') + issueLines(a);
    if (problems.trim()) {
      const issue = document.createElement('p');
      issue.className = 'setup-scan-issue';
      issue.textContent = problems.trim();
      main.append(issue);
    }
    const hint = document.createElement('p');
    hint.className = 'settings-note setup-hint';
    fillHint(hint, key);
    main.append(hint);
    row.append(box, main);
    list.append(row);
    scanBoxes.push(box);
  });
}

// Add selected: one filled block each, without any password. Nothing is saved: the user
// still presses Save. Nothing ticked goes on to Auto-setup, like Skip.
function setupScanAdd() {
  const picked = scanFound.filter((a, i) => scanBoxes[i] && scanBoxes[i].checked);
  if (!picked.length) { setupScanSkip(); return; }
  settingsData.accounts = collectAccounts();   // keep what was typed in the other blocks
  const first = settingsData.accounts.length;
  picked.forEach(a => settingsData.accounts.push(accountFromServers({
    label: a.label,
    name: (a.name || '').replace(/["<>]/g, ''),
    email: a.email,
    imap: a.imap,
    smtp: a.smtp,
    pass: '',   // a password never comes from the other app
  })));
  closeSetupDialog();
  renderAccountBlocks(settingsData.accounts);
  const block = document.querySelectorAll('.account-block')[first];
  if (block) block.querySelector('.a-imap-pass').focus();   // the one thing still missing
}

// Skip: on to Auto-setup.
function setupScanSkip() {
  document.getElementById('setup-scan').classList.add('hidden');
  document.getElementById('setup-form').classList.remove('hidden');
  document.getElementById('setup-email').focus();
}

// Next: only the domain of the address goes to the backend, which tries its sources
// one after the other.
async function setupNext() {
  const email = document.getElementById('setup-email').value.trim();
  const next = document.getElementById('setup-next');
  renderSetupHint(email);
  setupFound = undefined;
  // A domain without a dot ("o2game" instead of "o2game.com") cannot be looked up.
  if (!isEmail(email) || !email.split('@').pop().includes('.')) { setupNote = { key: 'setup.badEmail' }; renderSetupArea(email); return; }
  setupNote = { key: 'setup.looking' };
  renderSetupArea(email);
  next.disabled = true;
  try {
    setupFound = await cmd('lookup_mail_settings', { email });   // null: nobody knows that domain
    setupNote = null;
  } catch (e) {
    setupNote = { key: 'setup.failed', vars: { err: e } };
  }
  next.disabled = false;
  renderSetupArea(email);
}

// The result area: what was found, why one side is missing, where the settings came
// from, then Use these / Enter manually.
function renderSetupArea(email) {
  const domain = (email.split('@').pop() || '').toLowerCase();
  const line = document.getElementById('setup-line');
  const issues = document.getElementById('setup-issues');
  const source = document.getElementById('setup-source');
  const use = document.getElementById('setup-use');
  const manual = document.getElementById('setup-manual');
  document.getElementById('setup-result').classList.remove('hidden');
  issues.textContent = '';
  source.textContent = '';
  use.classList.add('hidden');
  manual.classList.add('hidden');
  line.textContent = setupNote ? t(setupNote.key, setupNote.vars || {}) : '';
  if (setupFound === undefined) {
    // A failed lookup still leaves a way forward.
    if (setupNote && setupNote.key === 'setup.failed') manual.classList.remove('hidden');
    return;
  }
  if (setupFound === null) {
    line.textContent = t('setup.notFound', { domain });
    manual.classList.remove('hidden');
    return;
  }
  const s = setupFound;
  const servers = [s.imap, s.smtp].filter(Boolean).map(v => `${v.host}:${v.port}`).join(' · ');
  line.textContent = servers ? t('setup.found', { provider: s.provider || domain, servers }) : '';
  source.textContent = t('setup.source', { host: s.source });
  issues.textContent = issueLines(s);
  manual.classList.remove('hidden');
  // A custom domain hosted by Google or Microsoft only shows who it is through its servers.
  const key = hintKey(email, s.imap ? s.imap.host : '');
  renderSetupHint(key);
  if ((s.imap || s.smtp) && providerHint(key) !== 'ms') use.classList.remove('hidden');
}

// Why one side of what was found is missing: the same sentences for Auto-setup and
// Auto-scan.
function issueLines(a) {
  const problems = [];
  if (!a.imap && a.imap_issue) problems.push(t(`setup.imapIssue.${a.imap_issue}`));
  if (!a.smtp && a.smtp_issue) problems.push(t(`setup.smtpIssue.${a.smtp_issue}`));
  return problems.join(' ');
}

// The AccountDto of a block whose servers are known already: what Auto-setup looked up and
// what Auto-scan read from a profile are the same three values. No block is ever filled
// with a password it did not get from the user.
function accountFromServers({ label, name, email, imap, smtp, pass }) {
  const field = (side, f) => (side ? side[f] : '');
  return {
    label: label || email,
    imap_host: field(imap, 'host'), imap_port: field(imap, 'port'),
    imap_user: imap ? imap.user : email, imap_pass: pass,
    imap_mailbox: '', imap_fetch: '',
    smtp_host: field(smtp, 'host'), smtp_port: field(smtp, 'port'),
    smtp_user: smtp ? smtp.user : email, smtp_pass: pass,
    smtp_from: name ? `"${name}" <${email}>` : email,
    signature_file: '', signature_on_new: true, signature_on_reply: true,
  };
}

// The usual account block, filled with what is known, then the usual Test. Nothing is
// saved: the user still presses Save.
function setupAddAccount(withSettings) {
  const name = document.getElementById('setup-name').value.trim().replace(/["<>]/g, '');
  const email = document.getElementById('setup-email').value.trim();
  const pass = document.getElementById('setup-pass').value;
  const s = withSettings ? setupFound : null;
  settingsData.accounts = collectAccounts();   // keep what was typed in the other blocks
  settingsData.accounts.push(accountFromServers({
    label: email, name, email,
    imap: s ? s.imap : null, smtp: s ? s.smtp : null, pass,
  }));
  closeSetupDialog();
  renderAccountBlocks(settingsData.accounts);
  const blocks = document.querySelectorAll('.account-block');
  if (s && (s.imap || s.smtp)) testAccount(blocks[blocks.length - 1]);
}

function collectAccounts() {
  return Array.from(document.querySelectorAll('.account-block')).map(collectAccount);
}

// One account block as typed (the AccountDto the backend expects).
function collectAccount(block) {
  const v = cls => block.querySelector(cls).value;
  // The sender: "Name <address>", the address being the typed one or the login's.
  const email = v('.a-sender-email').trim() || loginEmail(block);
  const name = v('.a-sender-name').trim().replace(/["<>]/g, '');
  const from = email ? (name ? `"${name}" <${email}>` : email) : '';
  return {
    label:        email || block.dataset.label,   // only names the account inside the app
    imap_host:    v('.a-imap-host'),
    imap_port:    v('.a-imap-port'),
    imap_user:    v('.a-imap-user'),
    imap_pass:    v('.a-imap-pass'),
    imap_mailbox: v('.a-imap-mailbox'),
    imap_fetch:   v('.a-imap-fetch'),
    smtp_host:    v('.a-smtp-host'),
    smtp_port:    v('.a-smtp-port'),
    smtp_user:    v('.a-smtp-user'),
    smtp_pass:    v('.a-smtp-pass'),
    smtp_from:    from,
    signature_file:     block.dataset.sigFile || '',
    signature_on_new:   block.querySelector('.a-sig-on-new').checked,
    signature_on_reply: block.querySelector('.a-sig-on-reply').checked,
  };
}

async function saveSettings() {
  const giphyKey     = document.getElementById('s-giphy-key').value;
  const accounts     = collectAccounts();
  try {
    await cmd('save_settings', {
      accounts, ai: collectAiSettings(), giphyKey, fontScale,
      theme: document.documentElement.dataset.theme,
      language: LANG,
      notifyNewMail: document.getElementById('s-notify').checked,
      discussionMode: document.getElementById('s-discussion').checked,
    });
    await saveCloudProject();   // Supabase settings are local-only: they have their own command
    savedTheme = document.documentElement.dataset.theme;
    savedFontScale = fontScale;
    savedLang = LANG;
    await loadComposeAccounts();   // senders and signatures may have changed
    applyDiscussionMode(document.getElementById('s-discussion').checked);
    closeSettings();
  } catch (e) {
    showToast(t('set.saveFailed', { err: e }));   // alert() is not shown in the app window
  }
}

// ── Sending account & signature ──
// Each account's sender and signature (compose_accounts), reloaded after saving Settings.
let composeAccounts = [];

async function loadComposeAccounts() {
  try { composeAccounts = await invoke('compose_accounts'); }
  catch (_) { composeAccounts = []; }
  $c('compose-from').innerHTML = composeAccounts
    .map((a, i) => `<option value="${i}">${esc(a.from.replace(/"/g, ''))}</option>`).join('');
  // Only a choice with two accounts or more; with one, the hidden select still sends from it.
  $c('compose-from-row').classList.toggle('hidden', composeAccounts.length < 2);
}

// The account that received an email: the one its uid belongs to (To/Cc miss mailing lists,
// Bcc and aliases), else the one whose address is in To or Cc, else the first.
function accountForEmail(email) {
  const received = Math.floor(Number(email.uid) / 2 ** 32);
  if (received >= 0 && received < composeAccounts.length) return received;
  const recipients = `${email.to || ''} ${email.cc || ''}`.toLowerCase();
  const i = composeAccounts.findIndex(a => {
    const address = splitSender(a.from).email.toLowerCase();
    return address.includes('@') && recipients.includes(address);
  });
  return Math.max(i, 0);
}

// The signature block of account `index`, or '' when it has none / it is off for this kind.
function signatureBlock(index, kind) {
  const a = composeAccounts[index];
  if (!a || !a.signature_html.trim()) return '';
  if (kind === 'new' ? !a.signature_on_new : !a.signature_on_reply) return '';
  return `<div class="compose-signature"><br>-- <br>${sanitizeHtml(a.signature_html)}</div>`;
}

// After a change of sending account: its signature takes the place of the previous one
// (or goes above the quoted original, or at the end).
function placeSignature(body, index) {
  const old = body.querySelector('.compose-signature');
  const before = old ? old.nextSibling : body.querySelector('.compose-quoted');
  old?.remove();
  const html = signatureBlock(index, composeMode === 'new' ? 'new' : 'reply');
  if (!html) return;
  const tpl = document.createElement('template');
  tpl.innerHTML = html;
  body.insertBefore(tpl.content.firstElementChild, before || null);
}

// Settings → account → Options avancées → Signature. The file name is kept on the block
// (dataset.sigFile) and saved with the account.
function signaturePreviewDoc(html) {
  return `<!DOCTYPE html><html><head><meta charset="utf-8"></head><body style="margin:10px;font-family:Arial,sans-serif;font-size:14px;color:#222;background:#fff">${sanitizeHtml(html)}</body></html>`;
}

function showAccountSignature(block, html) {
  const has = !!block.dataset.sigFile && !!html.trim();
  const frame = block.querySelector('.sig-preview');
  frame.classList.toggle('hidden', !has);
  frame.srcdoc = has ? signaturePreviewDoc(html) : '';
  block.querySelector('.sig-hint').classList.toggle('hidden', has);
  block.querySelector('.sig-remove').classList.toggle('hidden', !has);
  block.querySelector('.sig-marker').classList.toggle('hidden', !has);
}

function setupAccountSignature(block) {
  const status = (msg, kind = '') => {
    const el = block.querySelector('.sig-status');
    el.textContent = msg;
    el.classList.toggle('ok', kind === 'ok');
    el.classList.toggle('error', kind === 'error');
  };
  const file = block.dataset.sigFile;
  showAccountSignature(block, '');
  if (file) invoke('signature_get', { file }).then(html => showAccountSignature(block, html)).catch(() => {});
  block.querySelector('.sig-choose').addEventListener('click', async () => {
    try {
      const path = await window.__TAURI__.dialog.open({
        multiple: false,
        filters: [{ name: t('set.sigFileFilter'), extensions: ['html', 'htm'] }],
      });
      if (!path) return;
      const [name, html] = await cmd('signature_import', { path });
      block.dataset.sigFile = name;
      showAccountSignature(block, html);
      status(t('set.sigImported'), 'ok');
    } catch (e) {
      status(String(e), 'error');
    }
  });
  // The file itself is deleted when Settings are saved (no longer used by any account).
  block.querySelector('.sig-remove').addEventListener('click', () => {
    block.dataset.sigFile = '';
    showAccountSignature(block, '');
    status(t('set.sigRemoved'), 'ok');
  });
}

// ── Settings → AI provider ──
// Keys and models are remembered per provider while the dialog is open, so switching
// back and forth doesn't lose what was typed.
const AI_DEFAULT_URL = {
  anthropic: 'https://api.anthropic.com/v1', openai: 'https://api.openai.com/v1',
  mistral: 'https://api.mistral.ai/v1', ollama: 'http://localhost:11434/v1', custom: '',
};
const CLAUDE_CODE = 'claude_code';   // "Claude (subscription)", through the user's Claude Code
const ANTHROPIC_KEYS_URL = 'https://console.anthropic.com/settings/keys';
const ai = { provider: 'anthropic', keys: {}, models: {}, urls: {} };
const $ai = id => document.getElementById(id);

function loadAiSettings(s) {
  ai.provider = s?.provider || 'anthropic';
  ai.keys = { ...(s?.keys || {}) };
  ai.models = { [ai.provider]: s?.model || '' };
  ai.urls = { [ai.provider]: s?.base_url || '' };
  $ai('s-ai-provider').value = ai.provider;
  showAiProvider();
  refreshClaudeCode();
}

// ── Claude (subscription), through the user's Claude Code ──
// The choice is greyed out when Claude Code isn't installed.
let ccPoll = null;

async function refreshClaudeCode() {
  let s;
  try { s = await cmd('claude_code_status'); } catch { s = { installed: false }; }
  $ai('s-ai-provider').querySelector(`option[value="${CLAUDE_CODE}"]`).disabled = !s.installed;
  const el = $ai('s-ai-cc-status');
  el.textContent = s.paid ? t('set.aiSignedIn', { email: s.email })
    : ccPoll ? t('set.aiSigningIn')
    : s.signed_in ? t('set.aiNeedsPlan') : '';
  el.classList.toggle('ok', !!s.paid);
  el.classList.toggle('error', !!s.signed_in && !s.paid);
  $ai('s-ai-cc-sign-in').classList.toggle('hidden', !!s.paid);
  // Signed in with the wrong account (free plan, API key): sign out, then sign in again.
  $ai('s-ai-cc-sign-out').classList.toggle('hidden', !s.signed_in);
  return s;
}

async function signOutOfClaude() {
  stopClaudeCodePoll();
  try {
    await cmd('claude_code_sign_out');
  } catch (e) {
    $ai('s-ai-cc-status').textContent = String(e);
    return;
  }
  refreshClaudeCode();
}

function stopClaudeCodePoll() {
  clearTimeout(ccPoll);
  ccPoll = null;
}

// Claude Code opens the browser; check every few seconds until a paid plan is signed in (10 min max).
async function signInWithClaude() {
  if (ccPoll) return;   // a sign-in is already under way
  try {
    await cmd('claude_code_sign_in');
  } catch (e) {
    $ai('s-ai-cc-status').textContent = String(e);
    return;
  }
  $ai('s-ai-cc-status').textContent = t('set.aiSigningIn');
  const until = Date.now() + 10 * 60 * 1000;
  const check = async () => {
    const s = await refreshClaudeCode();
    if (!ccPoll) return;
    if (s.paid || Date.now() > until || ai.provider !== CLAUDE_CODE) {
      stopClaudeCodePoll();
      refreshClaudeCode();
    } else {
      ccPoll = setTimeout(check, 3000);
    }
  };
  ccPoll = setTimeout(check, 3000);
}

// Store what is typed for the current provider.
function stashAiFields() {
  ai.keys[ai.provider] = $ai('s-ai-key').value.trim();
  ai.models[ai.provider] = $ai('s-ai-model').value.trim();
  ai.urls[ai.provider] = $ai('s-ai-url').value.trim();
}

function showAiProvider() {
  const p = ai.provider;
  $ai('s-ai-key').value = ai.keys[p] || '';
  $ai('s-ai-model').value = ai.models[p] || '';   // empty until the user picks a model
  $ai('s-ai-url').value = ai.urls[p] || '';
  $ai('s-ai-url').placeholder = AI_DEFAULT_URL[p] || 'https://api.example.com/v1';
  $ai('s-ai-key-row').classList.toggle('hidden', ['ollama', CLAUDE_CODE].includes(p));
  $ai('s-ai-get-key').classList.toggle('hidden', p !== 'anthropic');
  $ai('s-ai-cc-row').classList.toggle('hidden', p !== CLAUDE_CODE);
  $ai('s-ai-url-row').classList.toggle('hidden', !['ollama', 'custom'].includes(p));
  $ai('s-ai-models').innerHTML = '';
  setAiStatus('');
  const label = $ai('s-ai-provider').selectedOptions[0]?.textContent || p;
  $ai('s-ai-note').textContent = p === 'ollama' ? t('set.aiNoteLocal') : t('set.aiNoteCloud', { provider: label });
}

function setAiStatus(msg, kind = '') {
  const el = $ai('s-ai-status');
  el.textContent = msg;
  el.classList.toggle('ok', kind === 'ok');
  el.classList.toggle('error', kind === 'error');
}

function aiFields() {
  stashAiFields();
  return { provider: ai.provider, model: ai.models[ai.provider] || '', baseUrl: ai.urls[ai.provider] || '', apiKey: ai.keys[ai.provider] || '' };
}

function collectAiSettings() {
  stashAiFields();
  return { provider: ai.provider, model: ai.models[ai.provider] || '', base_url: ai.urls[ai.provider] || '', keys: ai.keys };
}

async function loadAiModels(quiet = false) {
  const f = aiFields();
  if (!quiet) setAiStatus(t('set.aiTesting'));
  try {
    const models = await cmd('ai_list_models', { provider: f.provider, baseUrl: f.baseUrl, apiKey: f.apiKey });
    $ai('s-ai-models').innerHTML = models.map(m => `<option value="${esc(m)}"></option>`).join('');
    setAiStatus(models.length ? t('set.aiModelsLoaded', { n: models.length }) : t('set.aiNoModels'), models.length ? 'ok' : 'error');
  } catch (e) {
    if (!quiet) setAiStatus(String(e), 'error');
  }
}

function setupAiSettings() {
  $ai('s-ai-provider').addEventListener('change', () => {
    stashAiFields();
    ai.provider = $ai('s-ai-provider').value;
    showAiProvider();
    // Ollama needs no key: list the installed models straight away.
    if (ai.provider === 'ollama') loadAiModels(true);
    if (ai.provider === CLAUDE_CODE) { loadAiModels(true); refreshClaudeCode(); }
  });
  $ai('s-ai-load').addEventListener('click', () => loadAiModels());
  $ai('s-ai-cc-sign-in').addEventListener('click', signInWithClaude);
  $ai('s-ai-cc-sign-out').addEventListener('click', signOutOfClaude);
  $ai('s-ai-get-key').addEventListener('click', () =>
    window.__TAURI__.shell.open(ANTHROPIC_KEYS_URL).catch(e => console.error('open keys page', e)));
  // Check a pasted or typed key straight away: listing the models needs a valid key.
  let checkedKey = '';
  const checkKey = () => {
    const key = $ai('s-ai-key').value.trim();
    if (ai.provider !== 'anthropic' || !key || key === checkedKey) return;
    checkedKey = key;
    loadAiModels();
  };
  $ai('s-ai-key').addEventListener('paste', () => setTimeout(checkKey));   // the value lands after the event
  $ai('s-ai-key').addEventListener('change', checkKey);
  $ai('s-ai-test').addEventListener('click', async () => {
    const f = aiFields();
    setAiStatus(t('set.aiTesting'));
    $ai('s-ai-test').disabled = true;
    try {
      const [answer, ms] = await cmd('ai_test', { provider: f.provider, model: f.model, baseUrl: f.baseUrl, apiKey: f.apiKey });
      setAiStatus(t('set.aiTestOk', { answer, seconds: fmtNumber(Math.round(ms / 100) / 10) }), 'ok');
    } catch (e) {
      setAiStatus(String(e), 'error');
    } finally {
      $ai('s-ai-test').disabled = false;
    }
  });
}

// ── Settings → Cloud (Supabase) ──
function setCloudStatus(msg, kind = '') {
  const el = document.getElementById('s-cloud-status');
  el.textContent = msg;
  el.classList.toggle('ok', kind === 'ok');
  el.classList.toggle('error', kind === 'error');
}

function renderCloudStatus(status) {
  document.getElementById('s-cloud-signed-out').classList.toggle('hidden', status.signed_in);
  document.getElementById('s-cloud-signed-in').classList.toggle('hidden', !status.signed_in);
  document.getElementById('s-cloud-who').textContent = status.email;
}

// Save the project URL/key if they changed (local only), before any account action.
async function saveCloudProject() {
  const url = document.getElementById('s-cloud-url').value.trim();
  const anonKey = document.getElementById('s-cloud-anon').value.trim();
  if (url === (settingsData.supabase_url || '') && anonKey === (settingsData.supabase_anon_key || '')) return;
  renderCloudStatus(await cmd('set_cloud', { url, anonKey }));
  settingsData.supabase_url = url;
  settingsData.supabase_anon_key = anonKey;
  if (settingsBaseline) settingsBaseline.cloud = cloudProjectFields();   // saved now: no longer unsaved
}

function setupCloudSettings() {
  const run = async (fn) => {
    setCloudStatus(t('set.cloudWorking'));
    try { await fn(); } catch (e) { setCloudStatus(String(e), 'error'); }
  };
  const creds = () => {
    const email = document.getElementById('s-cloud-email').value.trim();
    const password = document.getElementById('s-cloud-password').value;
    if (!document.getElementById('s-cloud-url').value.trim() || !document.getElementById('s-cloud-anon').value.trim()) {
      throw t('set.cloudNeedProject');
    }
    if (!email || !password) throw t('set.cloudNeedCreds');
    return { email, password };
  };
  document.getElementById('s-cloud-signin').addEventListener('click', () => run(async () => {
    const c = creds();
    await saveCloudProject();
    renderCloudStatus(await cmd('cloud_sign_in', c));
    document.getElementById('s-cloud-password').value = '';
    setCloudStatus(t('set.cloudSynced'), 'ok');
  }));
  document.getElementById('s-cloud-signup').addEventListener('click', () => run(async () => {
    const c = creds();
    await saveCloudProject();
    const signedIn = await cmd('cloud_sign_up', c);
    renderCloudStatus(await cmd('cloud_status'));
    setCloudStatus(signedIn ? t('set.cloudSynced') : t('set.cloudConfirm'), 'ok');
  }));
  document.getElementById('s-cloud-sync').addEventListener('click', () => run(async () => {
    await cmd('cloud_sync_now');
    setCloudStatus(t('set.cloudSynced'), 'ok');
  }));
  document.getElementById('s-cloud-signout').addEventListener('click', () => run(async () => {
    renderCloudStatus(await cmd('cloud_sign_out'));
    setCloudStatus('');
  }));
}

// Yes/no question in the app's own look (the webview's confirm() is not shown in the
// app window, and a system dialog does not follow the theme). Resolves true on `okLabel`.
function askInApp(message, okLabel) {
  const overlay = document.getElementById('confirm-overlay');
  document.getElementById('confirm-text').textContent = message;
  const ok = document.getElementById('confirm-ok'), cancel = document.getElementById('confirm-cancel');
  ok.textContent = okLabel;
  cancel.textContent = t('proj.cancel');
  overlay.classList.remove('hidden');
  cancel.focus();
  return new Promise(resolve => {
    const done = answer => { overlay.classList.add('hidden'); resolve(answer); };
    ok.onclick = () => done(true);
    cancel.onclick = () => done(false);
  });
}

// What Settings holds, in the shape the unsaved-changes recap compares.
let settingsBaseline = null;   // taken when Settings opens

function settingsSnapshot() {
  return {
    theme: document.documentElement.dataset.theme, fontScale, language: LANG,
    notify: document.getElementById('s-notify').checked,
    discussion: document.getElementById('s-discussion').checked,
    accounts: collectAccounts(),
    // A provider only looked at leaves an empty key behind: not a change.
    ai: JSON.stringify(collectAiSettings(), (k, v) => v === '' ? undefined : v),
    giphy: document.getElementById('s-giphy-key').value,
    cloud: cloudProjectFields(),
  };
}

function cloudProjectFields() {
  return document.getElementById('s-cloud-url').value.trim() + '\n' + document.getElementById('s-cloud-anon').value.trim();
}

// One line per change since Settings opened; accounts are matched by their name in the app.
function unsavedChanges() {
  const a = settingsBaseline, b = settingsSnapshot(), lines = [];
  if (b.theme !== a.theme) lines.push(t('set.theme'));
  if (b.fontScale !== a.fontScale) lines.push(t('set.fontScale'));
  if (b.language !== a.language) lines.push(t('set.language'));
  if (b.notify !== a.notify) lines.push(t('set.notify'));
  if (b.discussion !== a.discussion) lines.push(t('set.discussion'));
  const name = (acc, i) => acc.label || t('set.account', { n: i + 1 });
  const before = new Map(a.accounts.map((acc, i) => [name(acc, i), JSON.stringify(acc)]));
  const after = new Map(b.accounts.map((acc, i) => [name(acc, i), JSON.stringify(acc)]));
  for (const [n, json] of after) {
    if (!before.has(n)) lines.push(t('set.unsavedAdded', { name: n }));
    else if (before.get(n) !== json) lines.push(t('set.unsavedChanged', { name: n }));
  }
  for (const n of before.keys()) if (!after.has(n)) lines.push(t('set.unsavedRemoved', { name: n }));
  // Two accounts with the same name hide each other above: still say the accounts changed.
  if (!lines.length && JSON.stringify(a.accounts) !== JSON.stringify(b.accounts)) lines.push(t('set.accounts'));
  if (b.ai !== a.ai) lines.push(t('set.ai'));
  if (b.giphy !== a.giphy) lines.push(t('set.giphyKey'));
  if (b.cloud !== a.cloud) lines.push(t('set.cloud'));
  return lines;
}

function askCloseSettings() {
  const lines = unsavedChanges();
  if (!lines.length) return closeSettings();
  document.getElementById('unsaved-list').innerHTML = lines.map(l => `<li dir="auto">${esc(l)}</li>`).join('');
  document.getElementById('unsaved-overlay').classList.remove('hidden');
  document.getElementById('unsaved-back').focus();
}

// While a question is open, Tab stays on its buttons and Escape answers "no".
function keepFocusIn(overlay, onEscape) {
  overlay.addEventListener('keydown', e => {
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); onEscape(); }
    if (e.key !== 'Tab') return;
    e.preventDefault();
    const btns = [...overlay.querySelectorAll('button')];
    const i = btns.indexOf(document.activeElement);
    btns[(i + (e.shiftKey ? -1 : 1) + btns.length) % btns.length].focus();
  });
}

function hideUnsaved() {
  document.getElementById('unsaved-overlay').classList.add('hidden');
}

// Closing without saving undoes the live previews (theme, font size).
function closeSettings() {
  document.getElementById('settings-overlay').classList.add('hidden');
  if (LANG !== savedLang) applyLanguage(savedLang);
  applyTheme(savedTheme);
  fontScale = savedFontScale;
  document.documentElement.style.fontSize = `${fontScale * 14}px`;
}

// ── Email row builder ──────────────────────────────────────────────────────
function emailRow(email) {
  const row = document.createElement('div');
  // Two separate signs: the importance is the coloured bar on the row's edge (p-<priority>);
  // the square mark is filled while unread and, in the inbox, toggles read/unread
  // (other folders number UIDs separately).
  row.className = `email-row has-mark p-${email.priority}` + (email.is_read ? '' : ' unread');
  row.dataset.uid = email.uid;
  const inInbox = appState && panelForEmail(email) === 'inbox';
  const mark = inInbox
    ? `<button class="row-mark" title="${readToggleTitle(email.is_read)}" aria-label="${t('rp.toggleRead')}"></button>`
    : `<span class="row-mark"></span>`;
  row.innerHTML = `
    ${mark}
    <div class="email-row-body">
      <div class="email-row-top">
        <span class="email-from">${esc(email.from_name)}</span>
        ${projectChip(email.project)}
        <span class="email-date">${esc(emailDateShort(email))}</span>
      </div>
      <div class="email-subject">${esc(email.subject)}</div>
      <div class="email-summary">${esc(email.summary)}</div>
    </div>
  `;
  const btn = row.querySelector('button.row-mark');
  if (btn) {
    btn.addEventListener('pointerdown', e => e.stopPropagation());   // not the start of a swipe
    btn.addEventListener('click', e => {
      e.stopPropagation();   // don't open the email
      setRead(email, !email.is_read);
    });
  }
  return row;
}

function projectChip(project) {
  return project ? `<span class="email-project" title="${esc(project)}">${esc(project)}</span>` : '';
}

// Updates the project chip of the inbox rows showing this email.
function setRowProject(uid, project) {
  document.querySelectorAll(`.email-row[data-uid="${uid}"]`).forEach(row => {
    if (row.closest(NON_INBOX_PANELS)) return;
    row.querySelector('.email-project')?.remove();
    row.querySelector('.email-date')?.insertAdjacentHTML('beforebegin', projectChip(project));
  });
}

// After a project is renamed (newName) or deleted (null).
function renameProjectChips(oldName, newName) {
  document.querySelectorAll('.email-project').forEach(chip => {
    if (chip.textContent !== oldName) return;
    if (newName) { chip.textContent = newName; chip.title = newName; } else chip.remove();
  });
}

function readToggleTitle(isRead) {
  return isRead ? t('rp.markUnread') : t('rp.markRead');
}

// ── Utils ──────────────────────────────────────────────────────────────────
function esc(str) {
  if (!str) return '';
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}
