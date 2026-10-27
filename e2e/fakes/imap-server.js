'use strict';

// A small IMAPS server for the end-to-end tests. It speaks just the part of IMAP4rev1
// the app uses (src-tauri/src/imap_client.rs): LOGIN, LIST, SELECT, FETCH, UID FETCH,
// UID SEARCH, UID STORE, UID MOVE/COPY, APPEND, CREATE, LOGOUT. Mailboxes live in memory.

const tls = require('node:tls');

const ROLES = { Sent: '\\Sent', Drafts: '\\Drafts', Trash: '\\Trash', Junk: '\\Junk' };

class FakeImap {
  /** `cert`/`key`: PEM strings. `user`/`pass`: the only login accepted. */
  constructor({ cert, key, user, pass }) {
    this.user = user;
    this.pass = pass;
    this.mailboxes = { INBOX: [], Sent: [], Drafts: [], Trash: [], Junk: [] };
    this.nextUid = 100;
    this.logins = 0;
    this.server = tls.createServer({ cert, key }, sock => this.session(sock));
  }

  listen() {
    return new Promise(resolve => this.server.listen(0, '127.0.0.1', () => resolve(this.server.address().port)));
  }

  close() {
    return new Promise(resolve => this.server.close(() => resolve()));
  }

  /** Deliver a message (RFC 822 text). Flags: e.g. ['\\Seen']. */
  append(mailbox, raw, flags = []) {
    const msg = { uid: this.nextUid++, flags: [...flags], raw: Buffer.from(raw.replace(/\r?\n/g, '\r\n')) };
    (this.mailboxes[mailbox] ||= []).push(msg);
    return msg.uid;
  }

  session(sock) {
    let buf = Buffer.alloc(0);
    let selected = null;
    let literal = null;   // { need, tag, rest, onDone } while an APPEND literal is being read
    const send = s => sock.write(s);

    sock.on('error', () => {});
    send('* OK [CAPABILITY IMAP4rev1 UIDPLUS MOVE] Fake IMAP ready\r\n');

    const handle = (tag, cmd, args, data) => {
      const box = () => this.mailboxes[selected] || [];
      switch (cmd) {
        case 'CAPABILITY':
          send('* CAPABILITY IMAP4rev1 UIDPLUS MOVE\r\n');
          return send(`${tag} OK CAPABILITY completed\r\n`);
        case 'NOOP':
          return send(`${tag} OK NOOP completed\r\n`);
        case 'LOGIN':
          if (args[0] !== this.user || args[1] !== this.pass) {
            return send(`${tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n`);
          }
          this.logins++;
          return send(`${tag} OK [CAPABILITY IMAP4rev1 UIDPLUS MOVE] Logged in\r\n`);
        case 'LIST':
          for (const name of Object.keys(this.mailboxes)) {
            const role = ROLES[name] ? ` ${ROLES[name]}` : '';
            send(`* LIST (\\HasNoChildren${role}) "/" "${name}"\r\n`);
          }
          return send(`${tag} OK LIST completed\r\n`);
        case 'SELECT':
        case 'EXAMINE': {
          if (!this.mailboxes[args[0]]) return send(`${tag} NO Mailbox does not exist\r\n`);
          selected = args[0];
          send('* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n');
          send(`* ${box().length} EXISTS\r\n* 0 RECENT\r\n`);
          send(`* OK [UIDVALIDITY 1] UIDs valid\r\n* OK [UIDNEXT ${this.nextUid}] Predicted next UID\r\n`);
          return send(`${tag} OK [READ-WRITE] SELECT completed\r\n`);
        }
        case 'FETCH':
          return this.fetch(send, tag, box(), args[0], false, 'FETCH');
        case 'CREATE':
          this.mailboxes[args[0]] ||= [];
          return send(`${tag} OK CREATE completed\r\n`);
        case 'SUBSCRIBE':
          return send(`${tag} OK SUBSCRIBE completed\r\n`);
        case 'APPEND': {
          const flags = args.find(a => Array.isArray(a)) || [];
          this.append(args[0], data.toString('utf8'), flags);
          return send(`${tag} OK APPEND completed\r\n`);
        }
        case 'UID': {
          const sub = String(args[0]).toUpperCase();
          if (sub === 'FETCH') return this.fetch(send, tag, box(), args[1], true, 'UID FETCH');
          if (sub === 'SEARCH') {
            send(`* SEARCH ${box().map(m => m.uid).join(' ')}\r\n`.replace(' \r', '\r'));
            return send(`${tag} OK SEARCH completed\r\n`);
          }
          if (sub === 'STORE') {
            const [set, op, flags] = [args[1], String(args[2]).toUpperCase(), args[3] || []];
            box().forEach((m, i) => {
              if (!inSet(m.uid, set, true)) return;
              if (op.startsWith('+')) m.flags = [...new Set([...m.flags, ...flags])];
              else if (op.startsWith('-')) m.flags = m.flags.filter(f => !flags.includes(f));
              else m.flags = [...flags];
              send(`* ${i + 1} FETCH (UID ${m.uid} FLAGS (${m.flags.join(' ')}))\r\n`);
            });
            return send(`${tag} OK STORE completed\r\n`);
          }
          if (sub === 'MOVE' || sub === 'COPY') {
            const target = (this.mailboxes[args[2]] ||= []);
            for (const m of box().filter(m => inSet(m.uid, args[1], true))) {
              target.push({ ...m, uid: this.nextUid++ });
              if (sub === 'MOVE') this.mailboxes[selected] = this.mailboxes[selected].filter(x => x !== m);
            }
            return send(`${tag} OK ${sub} completed\r\n`);
          }
          if (sub === 'EXPUNGE') {
            this.mailboxes[selected] = box().filter(m => !(inSet(m.uid, args[1], true) && m.flags.includes('\\Deleted')));
            return send(`${tag} OK EXPUNGE completed\r\n`);
          }
          return send(`${tag} BAD Unsupported UID command\r\n`);
        }
        case 'LOGOUT':
          send('* BYE Logging out\r\n');
          send(`${tag} OK LOGOUT completed\r\n`);
          return sock.end();
        default:
          return send(`${tag} BAD Unknown command ${cmd}\r\n`);
      }
    };

    sock.on('data', chunk => {
      buf = Buffer.concat([buf, chunk]);
      for (;;) {
        if (literal) {
          if (buf.length < literal.need) return;
          const data = buf.subarray(0, literal.need);
          buf = buf.subarray(literal.need);
          const end = buf.indexOf('\r\n');
          if (end < 0) { buf = Buffer.concat([data, buf]); return; }   // wait for the closing CRLF
          buf = buf.subarray(end + 2);
          const { tag, cmd, args } = literal;
          literal = null;
          handle(tag, cmd, args, data);
          continue;
        }
        const end = buf.indexOf('\r\n');
        if (end < 0) return;
        const line = buf.subarray(0, end).toString('utf8');
        buf = buf.subarray(end + 2);
        const lit = line.match(/\{(\d+)\+?\}$/);
        const tokens = tokenize(lit ? line.slice(0, lit.index) : line);
        const [tag, cmd, ...args] = tokens;
        if (!tag || !cmd) continue;
        if (lit) {
          literal = { need: Number(lit[1]), tag, cmd: String(cmd).toUpperCase(), args };
          send('+ Ready for literal data\r\n');
          continue;
        }
        handle(tag, String(cmd).toUpperCase(), args);
      }
    });
  }

  fetch(send, tag, box, set, byUid, what) {
    box.forEach((m, i) => {
      if (!inSet(byUid ? m.uid : i + 1, set, byUid, box)) return;
      send(`* ${i + 1} FETCH (UID ${m.uid} FLAGS (${m.flags.join(' ')}) BODY[] {${m.raw.length}}\r\n`);
      send(m.raw);
      send(')\r\n');
    });
    send(`${tag} OK ${what} completed\r\n`);
  }
}

/** Is `n` in an IMAP sequence set like "1:5,7,9:*"? */
function inSet(n, set, byUid) {
  return String(set).split(',').some(part => {
    const [a, b] = part.split(':');
    const lo = Number(a);
    if (b === undefined) return n === lo;
    const hi = b === '*' ? Infinity : Number(b);
    return n >= Math.min(lo, hi) && n <= Math.max(lo, hi);
  });
}

/** Split a command line into atoms, "quoted strings" and (parenthesised lists). */
function tokenize(line) {
  const out = [];
  const stack = [out];
  let i = 0;
  while (i < line.length) {
    const c = line[i];
    if (c === ' ') { i++; continue; }
    if (c === '(') { const l = []; stack.at(-1).push(l); stack.push(l); i++; continue; }
    if (c === ')') { stack.pop(); i++; continue; }
    if (c === '"') {
      let s = '';
      i++;
      while (i < line.length && line[i] !== '"') {
        if (line[i] === '\\') i++;
        s += line[i++];
      }
      i++;
      stack.at(-1).push(s);
      continue;
    }
    let j = i;
    while (j < line.length && !' ()'.includes(line[j])) j++;
    // Keep BODY.PEEK[...] style atoms whole even though they hold brackets.
    stack.at(-1).push(line.slice(i, j));
    i = j;
  }
  return out;
}

module.exports = { FakeImap };
