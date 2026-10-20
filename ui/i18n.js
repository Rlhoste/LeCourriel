/* UI translations. t('key', {vars}) returns the text in the current language,
   falling back to English, then to the key itself. Static HTML uses
   data-i18n (text), data-i18n-placeholder and data-i18n-title. */

const STRINGS = {
  en: {
    // Sidebar
    'nav.overview': 'Overview', 'nav.inbox': 'Inbox', 'nav.important': 'Important', 'nav.tasks': 'Tasks',
    'nav.meetings': 'Meetings', 'nav.calendar': 'Calendar', 'nav.projects': 'Projects',
    'nav.sent': 'Sent', 'nav.drafts': 'Drafts', 'nav.trash': 'Trash',
    'nav.sponsor': '❤ Sponsor',
    'nav.search': '🔍 Search',
    'search.placeholder': 'Search, or ask the AI…', 'search.ask': '✨ Ask AI', 'search.aiTitle': 'Ask the AI (Ctrl+Enter)',
    'search.mode.auto': 'Auto', 'search.mode.find': '🔎 Find', 'search.mode.compose': '✉ Write',
    'search.mode.summarize': '🧾 Summarise', 'search.mode.answer': '❓ Question',
    'search.server': 'Also search the server',
    'search.help1': 'Type to search instantly. Press Enter to ask the AI: find, write an email, summarise or answer.',
    'search.examples': 'Examples: “from:martin unread” · “summarise the ADUM emails” · “write to Martin to confirm the defence date”',
    'search.filters': 'Filters: from: to: project: before: after: folder: unread (French works too: de: à: projet: non lu…)',
    'search.keys': '↑↓ navigate · ↵ open · Ctrl+↵ AI · Esc close',
    'search.noResults': 'No email matches. Press Enter to ask the AI.',
    'search.thinking': '✨ The AI is thinking…', 'search.serverSearching': 'Searching the server…',
    'search.serverFound.one': '1 older email found on the server', 'search.serverFound.other': '{n} older emails found on the server',
    'search.summarizeSel': '🧾 Summarise selection ({n})', 'search.cited': 'Emails used',
    'search.composeOpened': 'Draft opened in the composer.', 'search.empty': 'Type something first.',
    'search.folder.inbox': 'Inbox', 'search.folder.sent': 'Sent', 'search.folder.drafts': 'Drafts',
    'search.folder.trash': 'Trash', 'search.folder.server': 'Server',
    'nav.reportBug': '🐞 Report a bug', 'nav.refresh': '↻ Refresh', 'nav.settings': '⚙ Settings', 'nav.compose': '+ Compose',

    // Overview
    'ov.greeting': 'Good morning.', 'ov.loading': 'Loading…', 'ov.noAccount': 'Welcome! Add your mail account in Settings to get started.',
    'ov.urgent': 'Urgent & High', 'ov.deadlines': 'Deadlines', 'ov.tasks': 'Tasks', 'ov.meetings': 'Meetings',
    'ov.inDays': 'in {n}d', 'ov.daysAgo': '{n}d ago', 'ov.couldNotDelete': 'Could not delete: {err}',

    // Tasks kanban
    'kanban.title': 'Tasks', 'kanban.todo': 'Todo', 'kanban.inProgress': 'In progress',
    'kanban.done': 'Done', 'kanban.givenUp': 'Given up',
    'kanban.add': '+ Add a task…', 'kanban.addTo': '+ Add a task to {project}…', 'kanban.all': 'All tasks',
    'kanban.delete': 'Delete task', 'kanban.openEmail': 'Open the email',
    'kanban.aiMoved': 'Moved automatically by the AI', 'kanban.aiDone': 'AI: done', 'kanban.aiObsolete': 'AI: obsolete',

    // Calendar
    'cal.title': 'Calendar', 'cal.more': '+{n} more',

    // Projects
    'proj.new': '+ New project', 'proj.namePlaceholder': 'Project name',
    'proj.urgency.normal': 'Normal', 'proj.urgency.high': 'High',
    'proj.create': 'Create', 'proj.cancel': 'Cancel', 'proj.descPlaceholder': 'Project description…', 'proj.save': 'Save',
    'proj.emails.one': '1 email', 'proj.emails.other': '{n} emails', 'proj.enterName': 'Please enter a name.',

    // Reading pane
    'rp.select': 'Select an email to read',
    'rp.reply': 'Reply', 'rp.forward': 'Forward', 'rp.spam': 'Spam', 'rp.delete': 'Delete',
    'rp.project': 'Project', 'rp.noProject': 'No project', 'rp.more': 'More actions',
    'rp.markUnread': 'Mark as unread', 'rp.markRead': 'Mark as read', 'rp.saveAs': 'Save as…', 'rp.print': 'Print',
    'rp.movedTo': 'Moved to {project}', 'rp.removedFromProject': 'Removed from project',
    'rp.markedUnread': 'Marked as unread', 'rp.savedTo': 'Saved to {path}', 'rp.emailFilter': 'Email',
    'rp.toggleRead': 'Toggle read',
    'meta.from': 'From', 'meta.to': 'To', 'meta.cc': 'Cc', 'meta.bcc': 'Bcc', 'meta.replyTo': 'Reply-To',
    'meta.date': 'Date', 'meta.subject': 'Subject',

    // Composer
    'cmp.newMessage': 'New Message', 'cmp.resize': 'Drag to resize',
    'cmp.minimise': 'Minimise', 'cmp.expand': 'Expand', 'cmp.close': 'Close',
    'cmp.to': 'To', 'cmp.cc': 'Cc', 'cmp.bcc': 'Bcc', 'cmp.subject': 'Subject',
    'cmp.font': 'Font', 'cmp.size': 'Size', 'cmp.fixedWidth': 'Fixed width',
    'cmp.small': 'Small', 'cmp.normal': 'Normal', 'cmp.large': 'Large', 'cmp.huge': 'Huge',
    'cmp.bold': 'Bold (Ctrl+B)', 'cmp.italic': 'Italic (Ctrl+I)', 'cmp.underline': 'Underline (Ctrl+U)',
    'cmp.strike': 'Strikethrough', 'cmp.color': 'Text colour',
    'cmp.alignLeft': 'Align left', 'cmp.center': 'Centre', 'cmp.alignRight': 'Align right', 'cmp.justify': 'Justify',
    'cmp.bullets': 'Bulleted list', 'cmp.numbered': 'Numbered list',
    'cmp.outdent': 'Decrease indent', 'cmp.indent': 'Increase indent', 'cmp.quote': 'Quote',
    'cmp.link': 'Insert link', 'cmp.linkText': 'Text to display', 'cmp.removeLink': 'Remove link', 'cmp.ok': 'OK',
    'cmp.image': 'Insert image', 'cmp.gif': 'Insert GIF', 'cmp.searchGifs': 'Search GIFs…',
    'cmp.removeFormat': 'Remove formatting',
    'cmp.send': 'Send', 'cmp.sendTitle': 'Send (Ctrl+Enter)',
    'cmp.discardQ': 'Discard draft?', 'cmp.discard': 'Discard', 'cmp.keep': 'Keep', 'cmp.discardTitle': 'Discard draft',
    'cmp.imageTooLarge': '{name} is larger than 5 MB',
    'cmp.needRecipient': 'Please enter a recipient.', 'cmp.sending': 'Sending…',
    'cmp.sent': 'Message sent', 'cmp.sendFailed': 'Failed to send: {err}',
    'cmp.wrote': 'On {date}, {from} wrote:', 'cmp.forwarded': '---------- Forwarded message ----------',
    'gif.loading': 'Loading…', 'gif.trending': 'Trending', 'gif.none': 'No GIFs found', 'gif.adding': 'Adding GIF…',
    'ai.button': '✨ AI', 'ai.title': 'AI writing tools',
    'ai.formal': 'Make it formal', 'ai.translateEn': 'Translate to English', 'ai.translateFr': 'Translate to French',
    'ai.correct': 'Correct spelling & grammar', 'ai.auto': 'Auto answer',
    'ai.hint': 'Works on the selected text, or on your whole message.',
    'ai.autoTitle': 'Write a reply with AI (uses your notes if any)', 'ai.autoDisabled': 'Only available when replying',
    'ai.writeFirst': 'Write something first, or select the text to change.',
    'ai.busy.formal': 'Making it formal', 'ai.busy.translate_en': 'Translating', 'ai.busy.translate_fr': 'Translating',
    'ai.busy.correct': 'Correcting', 'ai.busy.auto_reply': 'Writing a reply',
    'ai.done': 'Done — Ctrl+Z to undo',

    // Settings
    'set.title': 'Settings', 'set.general': 'General', 'set.accounts': 'Accounts', 'set.ai': 'AI',
    'set.displayName': 'Display Name', 'set.fontScale': 'Font Scale', 'set.theme': 'Theme', 'set.language': 'Language',
    'set.addAccount': '+ Add Account', 'set.claudeKey': 'Claude API Key', 'set.model': 'Model',
    'set.giphyKey': 'GIPHY API Key', 'set.giphyHint': '(for the GIF picker — free at developers.giphy.com)',
    'set.showQr': 'Show QR Code', 'set.save': 'Save', 'set.saveFailed': 'Save failed: {err}',
    'set.account': 'Account {n}', 'set.remove': 'Remove', 'set.label': 'Label',
    'set.imapHost': 'IMAP Host', 'set.imapPort': 'IMAP Port', 'set.imapUser': 'IMAP Username',
    'set.imapPass': 'IMAP Password', 'set.imapMailbox': 'IMAP Mailbox', 'set.fetchCount': 'Fetch Count',
    'set.smtpHost': 'SMTP Host', 'set.smtpPort': 'SMTP Port', 'set.smtpUser': 'SMTP Username',
    'set.smtpPass': 'SMTP Password', 'set.fromAddress': 'From Address',
    'set.cloud': 'Cloud', 'set.cloudUrl': 'Supabase project URL', 'set.cloudAnon': 'Anon (public) key',
    'set.cloudEmail': 'Email', 'set.cloudPassword': 'Password', 'set.cloudSignIn': 'Sign in', 'set.cloudSignUp': 'Create account',
    'set.cloudSignedInAs': 'Signed in as', 'set.cloudSync': 'Sync now', 'set.cloudSignOut': 'Sign out',
    'set.cloudWorking': 'Working…', 'set.cloudSynced': 'Synced.', 'set.cloudConfirm': 'Account created. Confirm it from the email Supabase sent you, then sign in.',
    'set.cloudNeedProject': 'Enter the project URL and anon key first.', 'set.cloudNeedCreds': 'Enter your email and password.',
    'set.cloudNote': 'Each account only sees its own data (row-level security). Synced: tasks, swipes, project tags, projects, preferences and AI results. Never synced: mail passwords, API keys and these Supabase settings.',
    'theme.midnight': 'Midnight', 'theme.white': 'White', 'theme.modern': 'Modern',
    'theme.brutalist': 'Brutalist', 'theme.aero': 'Frutiger Aero', 'theme.terminal': 'Terminal',
  },

  fr: {
    'nav.overview': "Vue d'ensemble", 'nav.inbox': 'Boîte de réception', 'nav.important': 'Important', 'nav.tasks': 'Tâches',
    'nav.meetings': 'Réunions', 'nav.calendar': 'Calendrier', 'nav.projects': 'Projets',
    'nav.sent': 'Envoyés', 'nav.drafts': 'Brouillons', 'nav.trash': 'Corbeille',
    'nav.sponsor': '❤ Soutenir',
    'nav.search': '🔍 Rechercher',
    'search.placeholder': "Rechercher, ou demander à l'IA…", 'search.ask': "✨ Demander à l'IA", 'search.aiTitle': "Demander à l'IA (Ctrl+Entrée)",
    'search.mode.auto': 'Auto', 'search.mode.find': '🔎 Rechercher', 'search.mode.compose': '✉ Rédiger',
    'search.mode.summarize': '🧾 Synthétiser', 'search.mode.answer': '❓ Question',
    'search.server': 'Chercher aussi sur le serveur',
    'search.help1': "Tapez pour chercher instantanément. Entrée pour demander à l'IA : retrouver, rédiger un e-mail, synthétiser ou répondre.",
    'search.examples': 'Exemples : « de:martin non lu » · « résume les mails de l\'ADUM » · « écris à Martin pour confirmer la date de soutenance »',
    'search.filters': 'Filtres : de: à: projet: avant: après: dossier: non lu',
    'search.keys': '↑↓ naviguer · ↵ ouvrir · Ctrl+↵ IA · Échap fermer',
    'search.noResults': "Aucun e-mail ne correspond. Entrée pour demander à l'IA.",
    'search.thinking': "✨ L'IA réfléchit…", 'search.serverSearching': 'Recherche sur le serveur…',
    'search.serverFound.one': '1 ancien e-mail trouvé sur le serveur', 'search.serverFound.other': '{n} anciens e-mails trouvés sur le serveur',
    'search.summarizeSel': '🧾 Synthétiser la sélection ({n})', 'search.cited': 'E-mails utilisés',
    'search.composeOpened': 'Brouillon ouvert dans la fenêtre de rédaction.', 'search.empty': "Tapez quelque chose d'abord.",
    'search.folder.inbox': 'Réception', 'search.folder.sent': 'Envoyés', 'search.folder.drafts': 'Brouillons',
    'search.folder.trash': 'Corbeille', 'search.folder.server': 'Serveur',
    'nav.reportBug': '🐞 Signaler un bug', 'nav.refresh': '↻ Actualiser', 'nav.settings': '⚙ Paramètres', 'nav.compose': '+ Nouveau message',

    'ov.greeting': 'Bonjour.', 'ov.loading': 'Chargement…', 'ov.noAccount': 'Bienvenue ! Ajoutez votre compte mail dans les Paramètres pour commencer.',
    'ov.urgent': 'Urgent et important', 'ov.deadlines': 'Échéances', 'ov.tasks': 'Tâches', 'ov.meetings': 'Réunions',
    'ov.inDays': 'dans {n} j', 'ov.daysAgo': 'il y a {n} j', 'ov.couldNotDelete': 'Suppression impossible : {err}',

    'kanban.title': 'Tâches', 'kanban.todo': 'À faire', 'kanban.inProgress': 'En cours',
    'kanban.done': 'Terminé', 'kanban.givenUp': 'Abandonné',
    'kanban.add': '+ Ajouter une tâche…', 'kanban.addTo': '+ Ajouter une tâche à {project}…', 'kanban.all': 'Toutes les tâches',
    'kanban.delete': 'Supprimer la tâche', 'kanban.openEmail': "Ouvrir l'e-mail",
    'kanban.aiMoved': "Déplacée automatiquement par l'IA", 'kanban.aiDone': 'IA : terminée', 'kanban.aiObsolete': 'IA : obsolète',

    'cal.title': 'Calendrier', 'cal.more': '+{n} autres',

    'proj.new': '+ Nouveau projet', 'proj.namePlaceholder': 'Nom du projet',
    'proj.urgency.normal': 'Normale', 'proj.urgency.high': 'Haute',
    'proj.create': 'Créer', 'proj.cancel': 'Annuler', 'proj.descPlaceholder': 'Description du projet…', 'proj.save': 'Enregistrer',
    'proj.emails.one': '1 e-mail', 'proj.emails.other': '{n} e-mails', 'proj.enterName': 'Veuillez saisir un nom.',

    'rp.select': 'Sélectionnez un e-mail à lire',
    'rp.reply': 'Répondre', 'rp.forward': 'Transférer', 'rp.spam': 'Spam', 'rp.delete': 'Supprimer',
    'rp.project': 'Projet', 'rp.noProject': 'Aucun projet', 'rp.more': "Plus d'actions",
    'rp.markUnread': 'Marquer comme non lu', 'rp.markRead': 'Marquer comme lu', 'rp.saveAs': 'Enregistrer sous…', 'rp.print': 'Imprimer',
    'rp.movedTo': 'Déplacé vers {project}', 'rp.removedFromProject': 'Retiré du projet',
    'rp.markedUnread': 'Marqué comme non lu', 'rp.savedTo': 'Enregistré dans {path}', 'rp.emailFilter': 'E-mail',
    'rp.toggleRead': 'Basculer lu / non lu',
    'meta.from': 'De', 'meta.to': 'À', 'meta.cc': 'Cc', 'meta.bcc': 'Cci', 'meta.replyTo': 'Répondre à',
    'meta.date': 'Date', 'meta.subject': 'Objet',

    'cmp.newMessage': 'Nouveau message', 'cmp.resize': 'Glisser pour redimensionner',
    'cmp.minimise': 'Réduire', 'cmp.expand': 'Agrandir', 'cmp.close': 'Fermer',
    'cmp.to': 'À', 'cmp.cc': 'Cc', 'cmp.bcc': 'Cci', 'cmp.subject': 'Objet',
    'cmp.font': 'Police', 'cmp.size': 'Taille', 'cmp.fixedWidth': 'Largeur fixe',
    'cmp.small': 'Petite', 'cmp.normal': 'Normale', 'cmp.large': 'Grande', 'cmp.huge': 'Très grande',
    'cmp.bold': 'Gras (Ctrl+B)', 'cmp.italic': 'Italique (Ctrl+I)', 'cmp.underline': 'Souligné (Ctrl+U)',
    'cmp.strike': 'Barré', 'cmp.color': 'Couleur du texte',
    'cmp.alignLeft': 'Aligner à gauche', 'cmp.center': 'Centrer', 'cmp.alignRight': 'Aligner à droite', 'cmp.justify': 'Justifier',
    'cmp.bullets': 'Liste à puces', 'cmp.numbered': 'Liste numérotée',
    'cmp.outdent': 'Diminuer le retrait', 'cmp.indent': 'Augmenter le retrait', 'cmp.quote': 'Citation',
    'cmp.link': 'Insérer un lien', 'cmp.linkText': 'Texte à afficher', 'cmp.removeLink': 'Supprimer le lien', 'cmp.ok': 'OK',
    'cmp.image': 'Insérer une image', 'cmp.gif': 'Insérer un GIF', 'cmp.searchGifs': 'Rechercher des GIF…',
    'cmp.removeFormat': 'Supprimer la mise en forme',
    'cmp.send': 'Envoyer', 'cmp.sendTitle': 'Envoyer (Ctrl+Entrée)',
    'cmp.discardQ': 'Supprimer le brouillon ?', 'cmp.discard': 'Supprimer', 'cmp.keep': 'Garder', 'cmp.discardTitle': 'Supprimer le brouillon',
    'cmp.imageTooLarge': '{name} dépasse 5 Mo',
    'cmp.needRecipient': 'Veuillez saisir un destinataire.', 'cmp.sending': 'Envoi…',
    'cmp.sent': 'Message envoyé', 'cmp.sendFailed': "Échec de l'envoi : {err}",
    'cmp.wrote': 'Le {date}, {from} a écrit :', 'cmp.forwarded': '---------- Message transféré ----------',
    'gif.loading': 'Chargement…', 'gif.trending': 'Tendances', 'gif.none': 'Aucun GIF trouvé', 'gif.adding': 'Ajout du GIF…',
    'ai.button': '✨ IA', 'ai.title': "Outils d'écriture IA",
    'ai.formal': 'Rendre formel', 'ai.translateEn': 'Traduire en anglais', 'ai.translateFr': 'Traduire en français',
    'ai.correct': "Corriger l'orthographe et la grammaire", 'ai.auto': 'Réponse automatique',
    'ai.hint': "S'applique au texte sélectionné, ou à tout votre message.",
    'ai.autoTitle': "Écrire une réponse avec l'IA (utilise vos notes s'il y en a)", 'ai.autoDisabled': 'Disponible uniquement pour une réponse',
    'ai.writeFirst': "Écrivez quelque chose d'abord, ou sélectionnez le texte à modifier.",
    'ai.busy.formal': 'Passage au ton formel', 'ai.busy.translate_en': 'Traduction', 'ai.busy.translate_fr': 'Traduction',
    'ai.busy.correct': 'Correction', 'ai.busy.auto_reply': "Rédaction d'une réponse",
    'ai.done': 'Terminé — Ctrl+Z pour annuler',

    'set.title': 'Paramètres', 'set.general': 'Général', 'set.accounts': 'Comptes', 'set.ai': 'IA',
    'set.displayName': 'Nom affiché', 'set.fontScale': 'Taille du texte', 'set.theme': 'Thème', 'set.language': 'Langue',
    'set.addAccount': '+ Ajouter un compte', 'set.claudeKey': 'Clé API Claude', 'set.model': 'Modèle',
    'set.giphyKey': 'Clé API GIPHY', 'set.giphyHint': '(pour le choix de GIF — gratuite sur developers.giphy.com)',
    'set.showQr': 'Afficher le QR code', 'set.save': 'Enregistrer', 'set.saveFailed': "Échec de l'enregistrement : {err}",
    'set.account': 'Compte {n}', 'set.remove': 'Retirer', 'set.label': 'Libellé',
    'set.imapHost': 'Serveur IMAP', 'set.imapPort': 'Port IMAP', 'set.imapUser': 'Identifiant IMAP',
    'set.imapPass': 'Mot de passe IMAP', 'set.imapMailbox': 'Dossier IMAP', 'set.fetchCount': "Nombre d'e-mails récupérés",
    'set.smtpHost': 'Serveur SMTP', 'set.smtpPort': 'Port SMTP', 'set.smtpUser': 'Identifiant SMTP',
    'set.smtpPass': 'Mot de passe SMTP', 'set.fromAddress': "Adresse d'expédition",
    'set.cloud': 'Cloud', 'set.cloudUrl': 'URL du projet Supabase', 'set.cloudAnon': 'Clé publique (anon)',
    'set.cloudEmail': 'E-mail', 'set.cloudPassword': 'Mot de passe', 'set.cloudSignIn': 'Se connecter', 'set.cloudSignUp': 'Créer un compte',
    'set.cloudSignedInAs': 'Connecté en tant que', 'set.cloudSync': 'Synchroniser', 'set.cloudSignOut': 'Se déconnecter',
    'set.cloudWorking': 'En cours…', 'set.cloudSynced': 'Synchronisé.', 'set.cloudConfirm': "Compte créé. Confirmez-le depuis l'e-mail envoyé par Supabase, puis connectez-vous.",
    'set.cloudNeedProject': "Saisissez d'abord l'URL du projet et la clé anon.", 'set.cloudNeedCreds': 'Saisissez votre e-mail et votre mot de passe.',
    'set.cloudNote': "Chaque compte ne voit que ses propres données (sécurité au niveau des lignes). Synchronisé : tâches, balayages, projets attribués, projets, préférences et résultats de l'IA. Jamais synchronisé : mots de passe mail, clés API et ces réglages Supabase.",
    'theme.midnight': 'Minuit', 'theme.white': 'Blanc', 'theme.modern': 'Moderne',
    'theme.brutalist': 'Brutaliste', 'theme.aero': 'Frutiger Aero', 'theme.terminal': 'Terminal',
  },
};

const MONTH_NAMES = {
  en: ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'],
  fr: ['Janvier', 'Février', 'Mars', 'Avril', 'Mai', 'Juin', 'Juillet', 'Août', 'Septembre', 'Octobre', 'Novembre', 'Décembre'],
};
const WEEKDAY_SHORT = {
  en: ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'],
  fr: ['Lun', 'Mar', 'Mer', 'Jeu', 'Ven', 'Sam', 'Dim'],
};
const LANGUAGES = [{ id: 'en', name: 'English' }, { id: 'fr', name: 'Français' }];

let LANG = 'en';

function setLang(code) {
  LANG = STRINGS[code] ? code : 'en';
  document.documentElement.lang = LANG;   // also drives the composer's spellchecker
  try { localStorage.setItem('lang', LANG); } catch (_) {}
}

function t(key, vars = {}) {
  const s = STRINGS[LANG]?.[key] ?? STRINGS.en[key] ?? key;
  return s.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? vars[k] : m));
}

// Plural: uses key.one for n === 1, key.other otherwise.
function tn(key, n, vars = {}) {
  return t(`${key}.${n === 1 ? 'one' : 'other'}`, { n, ...vars });
}

function applyStaticI18n(root = document) {
  root.querySelectorAll('[data-i18n]').forEach(el => { el.textContent = t(el.dataset.i18n); });
  root.querySelectorAll('[data-i18n-placeholder]').forEach(el => { el.placeholder = t(el.dataset.i18nPlaceholder); });
  root.querySelectorAll('[data-i18n-title]').forEach(el => { el.title = t(el.dataset.i18nTitle); });
}

// Apply the last language before app.js renders anything.
try { setLang(localStorage.getItem('lang') || 'en'); } catch (_) {}
