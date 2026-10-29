// JavaScript lint: `npm run lint`. Only rules that catch bugs (undefined names, unreachable
// code, duplicate keys…), not style.
const fs = require('fs');
const js = require('@eslint/js');
const globals = require('globals');

// The UI is plain browser scripts loaded in order by index.html: what i18n.js declares at the
// top level (t, fmtDay, STRINGS…) is a global for the scripts after it.
const declared = file => Object.fromEntries(
  [...fs.readFileSync(file, 'utf8').matchAll(/^(?:async\s+)?(?:function\s*\*?|const|let|var)\s+([\w$]+)/gm)]
    .map(m => [m[1], 'readonly']),
);
const i18n = declared('ui/i18n.js');

module.exports = [
  { ignores: ['node_modules/', 'src-tauri/target/'] },
  js.configs.recommended,
  {
    rules: {
      'no-unused-vars': ['warn', { caughtErrors: 'none' }],
      'no-empty': ['error', { allowEmptyCatch: true }],
    },
  },
  {
    files: ['ui/**/*.js'],
    languageOptions: {
      sourceType: 'script',
      globals: { ...globals.browser, module: 'readonly' },
    },
    rules: {
      // Top-level names are shared with the other scripts, so "unused" there means nothing.
      'no-unused-vars': ['warn', { vars: 'local', caughtErrors: 'none' }],
      // globals.browser lists experimental names (e.g. `ai`) that the app may define itself.
      'no-redeclare': ['error', { builtinGlobals: false }],
      // The email sanitizer strips control characters on purpose.
      'no-control-regex': 'off',
    },
  },
  {
    files: ['ui/**/*.js'],
    ignores: ['ui/i18n.js'],
    languageOptions: { globals: { ...i18n, CourrielCore: 'readonly' } },
  },
  {
    files: ['ui/tests/**', 'ui/locales/check.js', 'ui/locales/test-format.js', 'e2e/**', 'eslint.config.js'],
    languageOptions: { sourceType: 'commonjs', globals: { ...globals.node } },
  },
  {
    // Code passed to browser.execute() runs inside the app window.
    files: ['e2e/**'],
    languageOptions: {
      globals: { ...globals.browser, ...i18n, browser: 'readonly', appState: 'readonly', setupAddAccount: 'readonly' },
    },
  },
];
