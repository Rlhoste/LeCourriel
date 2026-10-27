'use strict';

// A fake AI provider for the end-to-end tests, speaking the OpenAI Chat Completions
// format (what Settings → AI → "Custom" uses). Answers are fixed, keyed on words of
// the email, so a run always ends with the same Overview. Every request is recorded.

const http = require('node:http');

// First match wins. The deadline/meeting values are filled in per run (see fixtures.js).
function classify(subject, rules) {
  const rule = rules.find(r => r.match.test(subject));
  return rule ? rule.answer : {
    priority: 'other_priority', summary: `About: ${subject}`,
    project: null, deadline: null, task: null, meeting_link: null,
  };
}

class FakeAi {
  /** `key`: the only API key accepted. `rules`: [{ match: RegExp, answer: {...} }]. */
  constructor({ key, rules }) {
    this.key = key;
    this.rules = rules;
    this.requests = [];
    this.server = http.createServer((req, res) => this.handle(req, res));
  }

  listen() {
    return new Promise(resolve => this.server.listen(0, '127.0.0.1', () => resolve(this.server.address().port)));
  }

  close() {
    this.server.closeAllConnections?.();
    return new Promise(resolve => this.server.close(() => resolve()));
  }

  /** Classification requests received so far. */
  classifications() {
    return this.requests.filter(r => r.kind === 'classify');
  }

  handle(req, res) {
    let body = '';
    req.on('data', c => { body += c; });
    req.on('end', () => {
      const reply = (status, json) => {
        res.writeHead(status, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify(json));
      };
      const auth = req.headers.authorization || '';
      if (auth !== `Bearer ${this.key}`) {
        this.requests.push({ kind: 'rejected', path: req.url });
        return reply(401, { error: { message: 'Incorrect API key provided' } });
      }
      if (req.method === 'GET' && req.url.endsWith('/models')) {
        return reply(200, { data: [{ id: 'fake-model' }] });
      }
      if (req.method !== 'POST' || !req.url.endsWith('/chat/completions')) {
        return reply(404, { error: { message: `No route ${req.method} ${req.url}` } });
      }
      const payload = JSON.parse(body || '{}');
      const system = payload.messages?.[0]?.content || '';
      const user = payload.messages?.[1]?.content || '';
      let content;
      if (system.startsWith('You are an email priority classifier')) {
        const subject = (user.match(/^Subject: (.*)$/m) || [])[1] || '';
        this.requests.push({ kind: 'classify', subject });
        content = JSON.stringify(classify(subject, this.rules));
      } else if (system.startsWith('The user creates a project')) {
        // Echo which related emails the app sent, so the test can check them.
        this.requests.push({ kind: 'project', user });
        content = JSON.stringify(this.project || { name: 'Project', summary: 'A project', people: [], keywords: [] });
      } else if (system.startsWith('The user has a project')) {
        // Files the emails whose line matches `projectMatch` (set by the test).
        this.requests.push({ kind: 'project-match', user });
        const ids = user.split('\n')
          .filter(l => this.projectMatch && this.projectMatch.test(l))
          .map(l => Number((l.match(/^\[(\d+)\]/) || [])[1]))
          .filter(Boolean);
        content = JSON.stringify({ ids });
      } else if (system.startsWith("You track a user's to-do list")) {
        this.requests.push({ kind: 'task-check' });
        content = JSON.stringify({ done: [], obsolete: [] });
      } else {
        this.requests.push({ kind: 'other' });
        content = 'OK';
      }
      reply(200, { choices: [{ message: { role: 'assistant', content } }] });
    });
  }
}

module.exports = { FakeAi };
