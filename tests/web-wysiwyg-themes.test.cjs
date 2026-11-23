const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const vm = require('node:vm');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const styleBlock = /<style>([\s\S]*?)<\/style>/.exec(html)?.[1] || '';
const fullScript = /<script>([\s\S]*?)<\/script>/.exec(html)?.[1] || '';
const rendererCode = html.slice(html.indexOf('  const URL_RE'), html.indexOf('  let lastDay'));
const composerCode = html.slice(html.indexOf('  // ---- định dạng soạn tin'), html.indexOf('  // ---- cập nhật theo thời gian thực'));
const settingsCode = html.slice(html.indexOf('  // ---- cài đặt chung, giao diện và hồ sơ'), html.indexOf('  // ---- dữ liệu và lưu trữ'));

class MockClassList {
  constructor() { this.set = new Set(); }
  add(...names) { names.forEach((n) => n && this.set.add(n)); }
  remove(...names) { names.forEach((n) => this.set.delete(n)); }
  toggle(name, force) {
    if (force === undefined) {
      if (this.set.has(name)) { this.set.delete(name); return false; }
      this.set.add(name); return true;
    }
    if (force) this.set.add(name); else this.set.delete(name);
    return Boolean(force);
  }
  contains(name) { return this.set.has(name); }
}

class MockText {
  constructor(text = '') {
    this.nodeType = 3;
    this.nodeValue = String(text);
    this.parentNode = null;
  }
  get textContent() { return this.nodeValue; }
  set textContent(v) { this.nodeValue = String(v ?? ''); }
  get nextSibling() {
    if (!this.parentNode) return null;
    const list = this.parentNode.childNodes;
    const i = list.indexOf(this);
    return i >= 0 && i + 1 < list.length ? list[i + 1] : null;
  }
  get previousSibling() {
    if (!this.parentNode) return null;
    const list = this.parentNode.childNodes;
    const i = list.indexOf(this);
    return i > 0 ? list[i - 1] : null;
  }
}

class MockNode extends EventTarget {
  constructor(tag = 'div', className = '', text = '') {
    super();
    this.nodeType = 1;
    this.tag = String(tag).toLowerCase();
    this.tagName = this.tag.toUpperCase();
    this.className = className;
    this.children = [];
    this.parentNode = null;
    this.attributes = new Map();
    this.dataset = {};
    this.classList = new MockClassList();
    this.style = {
      props: new Map(),
      fontWeight: '',
      fontStyle: '',
      textDecoration: '',
      textDecorationLine: '',
      setProperty(k, v) { this.props.set(k, v); },
      getPropertyValue(k) { return this.props.get(k) || ''; },
    };
    this.value = '';
    this.disabled = false;
    this.hidden = false;
    this.open = false;
    this.focused = false;
    this._firstChild = { textContent: '' };
    if (text !== undefined && text !== '') {
      this.appendChild(new MockText(text));
    }
  }
  get childNodes() { return this.children; }
  get firstChild() { return this.children[0] || this._firstChild; }
  get nextSibling() {
    if (!this.parentNode) return null;
    const list = this.parentNode.childNodes;
    const i = list.indexOf(this);
    return i >= 0 && i + 1 < list.length ? list[i + 1] : null;
  }
  get previousSibling() {
    if (!this.parentNode) return null;
    const list = this.parentNode.childNodes;
    const i = list.indexOf(this);
    return i > 0 ? list[i - 1] : null;
  }
  get textContent() {
    return this.children.map((c) => (typeof c === 'string' ? c : c.textContent)).join('');
  }
  set textContent(v) {
    this.children = [];
    if (v !== '' && v !== undefined && v !== null) {
      this.appendChild(new MockText(String(v)));
    }
  }
  get selectedOptions() {
    const picked = this.children.find((c) => c.value === this.value);
    return picked ? [picked] : (this.children[0] ? [this.children[0]] : []);
  }
  append(...nodes) {
    for (const n of nodes) {
      this.appendChild(typeof n === 'string' ? new MockText(n) : n);
    }
  }
  appendChild(node) {
    const item = typeof node === 'string' ? new MockText(node) : node;
    if (item && typeof item === 'object') item.parentNode = this;
    this.children.push(item);
    return item;
  }
  replaceChildren(...nodes) {
    this.children = [];
    for (const n of nodes) this.appendChild(n);
  }
  setAttribute(k, v) { this.attributes.set(k, String(v)); }
  getAttribute(k) { return this.attributes.get(k); }
  removeAttribute(k) { this.attributes.delete(k); }
  contains(node) {
    if (this === node) return true;
    return this.children.some((c) => c instanceof MockNode && c.contains(node));
  }
  showModal() { this.open = true; }
  close() { this.open = false; this.dispatchEvent(new Event('close')); }
  focus() { this.focused = true; }
  click() { this.dispatchEvent(new Event('click')); }
  find(tag) {
    const t = String(tag).toLowerCase();
    return this.children.flatMap((c) => (c instanceof MockNode ? [...(c.tag === t ? [c] : []), ...c.find(t)] : []));
  }
  findByClass(cls) {
    return this.children.flatMap((c) => {
      if (!(c instanceof MockNode)) return [];
      const has = String(c.className || '').split(/\s+/).includes(cls) || c.classList.contains(cls);
      return [...(has ? [c] : []), ...c.findByClass(cls)];
    });
  }
}

function elNode(tag, ...items) {
  const n = new MockNode(tag);
  for (const item of items) {
    if (typeof item === 'string') n.appendChild(new MockText(item));
    else if (item && typeof item === 'object' && !(item instanceof MockNode) && !(item instanceof MockText)) {
      if (item.dataset) Object.assign(n.dataset, item.dataset);
      if (item.style) Object.assign(n.style, item.style);
    } else n.appendChild(item);
  }
  return n;
}

function setupComposerHarness(mutateComposer) {
  const code = mutateComposer ? mutateComposer(composerCode) : composerCode;
  const nodes = new Map();
  for (const id of ['text', 'send', 'reply', 'reply-txt', 'reply-x', 'attach', 'file',
    'fmt-bold', 'fmt-italic', 'fmt-underline', 'fmt-strike', 'fmt-code', 'fmt-clear']) {
    nodes.set(id, new MockNode('div'));
  }
  const posts = [];
  const appended = [];
  const doc = Object.assign(new EventTarget(), {
    createTextNode: (t) => new MockText(String(t)),
    createElement: (tag) => new MockNode(tag),
    createDocumentFragment: () => new MockNode('fragment'),
    getSelection: () => null,
  });
  const ctx = {
    $: (id) => nodes.get(id) || (nodes.set(id, new MockNode()), nodes.get(id)),
    el: (tag, cls, text) => new MockNode(tag, cls, text),
    document: doc,
    URL,
    curKind: 'private',
    hhmm: () => '12:00',
    cur: 42,
    replyTo: 0,
    api: async (method, path, body) => {
      posts.push({ method, path, body: structuredClone(body) });
      return { id: 901, chat_id: 42, date: 1700000000, text: body.text, entities: body.entities || [], kind: 'text', outgoing: true };
    },
    appendMessages: (list) => appended.push(...list),
    scheduleChats: () => {},
    toast: () => {},
    Set, Map, Event, Date, Math, Number, Object, Array, String, Boolean, JSON, RegExp,
  };
  vm.runInNewContext(
    `${rendererCode}\n${code}\nthis.exports = { extractComposerContent, extractPastePayload, applyComposerFormat, updateComposerToolbar, renderText, send };`,
    ctx,
  );
  return { ...ctx.exports, nodes, posts, appended };
}

function setupSettingsHarness(source = settingsCode) {
  const nodes = new Map();
  const root = new MockNode('html');
  const navItems = ['profile', 'appearance', 'storage', 'calls', 'accounts', 'notifications', 'privacy', 'folders', 'devices']
    .map((section) => {
      const btn = new MockNode('button', 'settings-item');
      btn.dataset.section = section;
      return btn;
    });
  const doc = {
    documentElement: root,
    createTextNode: (t) => new MockText(String(t)),
    querySelectorAll: (selector) => (selector === '#settings-dialog .settings-item' ? navItems : []),
  };
  let serverAppearance = { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' };
  const requests = [];
  const ctx = {
    $: (id) => {
      if (!nodes.has(id)) nodes.set(id, new MockNode(id === 'account' || id === 'appearance-font' ? 'select' : 'div'));
      return nodes.get(id);
    },
    el: (tag, cls, text) => new MockNode(tag, cls, text),
    document: doc,
    window: { matchMedia: () => ({ matches: false, addEventListener: () => {} }) },
    account: '+84900000000',
    initials: (s) => (s || '?')[0].toUpperCase(),
    api: async (method, path, body) => {
      requests.push({ method, path, body: body === undefined ? undefined : structuredClone(body) });
      if (method === 'GET' && path === '/api/settings/appearance') return structuredClone(serverAppearance);
      if (method === 'POST' && path === '/api/settings/appearance') {
        serverAppearance = structuredClone(body);
        return structuredClone(serverAppearance);
      }
      if (method === 'GET' && path === '/api/profile') {
        return { id: 7, name: 'An Lê', first_name: 'An', last_name: 'Lê', bio: 'Xin chào', username: 'an_le', phone_masked: '+849•••••000' };
      }
      throw new Error(`unexpected api ${method} ${path}`);
    },
    Event, Number, Math, Object, String, Array, Boolean, Set,
  };
  const accSelect = ctx.$('account');
  accSelect.append(Object.assign(new MockNode('option', '', 'An Lê · online'), { value: '+84900000000' }));
  accSelect.value = '+84900000000';
  vm.runInNewContext(
    `${source}\nthis.exports = { normalizeAppearance, applyAppearance, loadAppearance, selectSettingsSection };`,
    ctx,
  );
  const flush = () => new Promise(setImmediate);
  return {
    ...ctx.exports,
    nodes,
    navItems,
    root,
    requests,
    flush,
    submitAppearance: async () => {
      const ev = new Event('submit');
      ev.preventDefault = () => {};
      nodes.get('appearance-form').dispatchEvent(ev);
      await flush();
    },
  };
}

function parseThemeCssVars(cssText) {
  const blocks = [];
  const re = /([^{}]+)\{([^{}]+)\}/g;
  let m;
  while ((m = re.exec(cssText)) !== null) {
    const sel = m[1].trim();
    const vars = {};
    for (const decl of m[2].split(';')) {
      const idx = decl.indexOf(':');
      if (idx < 0) continue;
      const key = decl.slice(0, idx).trim();
      const val = decl.slice(idx + 1).trim();
      if (key.startsWith('--')) vars[key] = val;
    }
    if (Object.keys(vars).length) blocks.push({ sel, vars });
  }
  return blocks;
}

function resolveThemeVars(blocks, themeName, codeThemeOverride) {
  const out = {};
  const applySel = (predicate) => {
    for (const b of blocks) {
      const parts = b.sel.split(',').map((s) => s.trim());
      if (parts.some(predicate)) Object.assign(out, b.vars);
    }
  };
  applySel((s) => s === ':root');
  if (themeName === 'system-dark') {
    applySel((s) => s === ':root[data-theme="system"][data-resolved-theme="dark"]');
  } else if (themeName === 'system-light') {
    applySel((s) => s === ':root[data-theme="system"]');
  } else {
    applySel((s) => s === `:root[data-theme="${themeName}"]`);
  }
  if (codeThemeOverride === 'dark' || (codeThemeOverride !== 'light' && ['system-dark', 'dark', 'midnight', 'high-contrast'].includes(themeName))) {
    applySel((s) => s === ':root[data-code-theme="dark"]');
  }
  return out;
}

function hexLuminance(hex) {
  const clean = hex.replace('#', '').trim();
  const full = clean.length === 3 ? clean.split('').map((c) => c + c).join('') : clean;
  assert.match(full, /^[0-9a-fA-F]{6}$/, `Màu phải là hex 6 chữ số: ${hex}`);
  const channels = [0, 2, 4].map((i) => {
    const v = parseInt(full.slice(i, i + 2), 16) / 255;
    return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

function contrastRatio(hexA, hexB) {
  const l1 = hexLuminance(hexA);
  const l2 = hexLuminance(hexB);
  return (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05);
}

test('cấu trúc HTML sạch bảo mật, đủ nút WYSIWYG B/I/U/S/</>/Tx và 10 chủ đề + nền + khối mã', () => {
  new vm.Script(fullScript);
  for (const banned of [
    'innerHTML', 'outerHTML', 'insertAdjacentHTML', 'document.write',
    'localStorage', 'sessionStorage', 'gw_token', 'GATEWAY_TOKEN',
    '@font-face', 'fonts.googleapis', 'fonts.gstatic',
    'Cập nhật sinh nhật', 'bottom-nav', '999+', '📞 Cuộc gọi',
  ]) {
    assert.equal(html.includes(banned), false, `Không được chứa chuỗi cấm: ${banned}`);
  }
  assert.doesNotMatch(styleBlock, /url\(\s*["']?https?:/i, 'Không tải ảnh nền/tài nguyên ngoài');
  assert.doesNotMatch(styleBlock, /@media\s*\(\s*prefers-color-scheme:\s*dark\s*\)/, 'Chỉ dùng 1 bảng biến CSS duy nhất');

  assert.match(html, /<div[^>]*\bid="text"[^>]*contenteditable="true"/);
  for (const id of ['fmt-bold', 'fmt-italic', 'fmt-underline', 'fmt-strike', 'fmt-code', 'fmt-clear']) {
    assert.match(html, new RegExp(`id="${id}"`), `Thiếu nút ${id}`);
  }
  for (const theme of ['system', 'light', 'dark', 'midnight', 'telegram', 'green', 'purple', 'orange', 'pink', 'high-contrast']) {
    assert.match(html, new RegExp(`id="theme-${theme}"[^>]*class="[^"]*theme-card`), `Thiếu nút chủ đề ${theme}`);
  }
  for (const bg of ['plain', 'pattern', 'gradient']) {
    assert.match(html, new RegExp(`id="chat-bg-${bg}"`), `Thiếu tuỳ chọn nền ${bg}`);
  }
  for (const ct of ['auto', 'light', 'dark']) {
    assert.match(html, new RegExp(`id="code-theme-${ct}"`), `Thiếu tuỳ chọn khối mã ${ct}`);
  }
  assert.match(html, /id="appearance-reset"[^>]*>Đặt lại</);
});

test('extractComposerContent đọc đúng DOM thành { text, entities } với UTF-16, emoji, tiếng Việt tổ hợp, lồng/chồng lấn và xuống dòng', () => {
  const h = setupComposerHarness();

  // 1. Giữ nguyên ** tự gõ (không tự chuyển thành bold).
  const literal = elNode('div', '**chữ đậm** __nghiêng__ `mã`');
  assert.deepEqual(structuredClone(h.extractComposerContent(literal)), {
    text: '**chữ đậm** __nghiêng__ `mã`',
    entities: [],
  });

  // 2. Emoji (surrogate pair + ZWJ) và chữ Việt tổ hợp NFD -> offset/length tính bằng UTF-16 code units.
  const nfd = 'Tie\u0302\u0301ng Vie\u0323\u0302t';
  const emojiRoot = elNode(
    'div',
    '🌙 ',
    elNode('strong', 'Chào'),
    ' 👨‍👩‍👧‍👦 ',
    elNode('em', nfd),
    ' ',
    elNode('u', 'gạch chân'),
    ' ',
    elNode('s', 'gạch giữa'),
    ' ',
    elNode('code', 'x=1'),
  );
  const resEmoji = h.extractComposerContent(emojiRoot);
  const expectedText = `🌙 Chào 👨‍👩‍👧‍👦 ${nfd} gạch chân gạch giữa x=1`;
  assert.equal(resEmoji.text, expectedText);
  assert.deepEqual(structuredClone(resEmoji.entities), [
    { type: 'bold', offset: '🌙 '.length, length: 'Chào'.length },
    { type: 'italic', offset: '🌙 Chào 👨‍👩‍👧‍👦 '.length, length: nfd.length },
    { type: 'underline', offset: `🌙 Chào 👨‍👩‍👧‍👦 ${nfd} `.length, length: 'gạch chân'.length },
    { type: 'strikethrough', offset: `🌙 Chào 👨‍👩‍👧‍👦 ${nfd} gạch chân `.length, length: 'gạch giữa'.length },
    { type: 'code', offset: `🌙 Chào 👨‍👩‍👧‍👦 ${nfd} gạch chân gạch giữa `.length, length: 'x=1'.length },
  ]);

  // 3. Lồng nhau + gộp liền kề + bỏ định dạng rỗng / \u200B.
  const nested = elNode(
    'div',
    elNode('strong', 'Đậm ', elNode('em', 'nghiêng ', elNode('u', 'chân'))),
    elNode('strong', '!'),
    elNode('em', '\u200B'),
    elNode('u', ''),
  );
  assert.deepEqual(structuredClone(h.extractComposerContent(nested)), {
    text: 'Đậm nghiêng chân!',
    entities: [
      { type: 'bold', offset: 0, length: 17 },
      { type: 'italic', offset: 4, length: 12 },
      { type: 'underline', offset: 12, length: 4 },
    ],
  });

  // 4. Chồng lấn (crossing): <strong>ab<em>cd</em></strong><em>ef</em>
  const crossing = elNode('div', elNode('strong', 'ab', elNode('em', 'cd')), elNode('em', 'ef'));
  assert.deepEqual(structuredClone(h.extractComposerContent(crossing)), {
    text: 'abcdef',
    entities: [
      { type: 'bold', offset: 0, length: 4 },
      { type: 'italic', offset: 2, length: 4 },
    ],
  });

  // 5. Xuống dòng (<br>, <div>, <div><br></div>, <br> giữ chỗ khi rỗng) và pre language.
  assert.deepEqual(structuredClone(h.extractComposerContent(elNode('div', elNode('br')))), { text: '', entities: [] });
  const multiline = elNode(
    'div',
    elNode('strong', 'Dòng 1', elNode('br'), 'Dòng 2'),
    elNode('div', elNode('br')),
    elNode('div', 'Dòng 4'),
    elNode('pre', { dataset: { language: 'rs' } }, 'fn main() {}'),
  );
  assert.deepEqual(structuredClone(h.extractComposerContent(multiline)), {
    text: 'Dòng 1\nDòng 2\n\nDòng 4\nfn main() {}',
    entities: [
      { type: 'bold', offset: 0, length: 13 },
      { type: 'pre', offset: 22, length: 12, language: 'rs' },
    ],
  });

  // 6. Loại bỏ script/style không bao giờ đưa vào text.
  const withScript = elNode('div', 'An toàn', elNode('script', 'alert(1)'), elNode('style', 'body{}'));
  assert.deepEqual(structuredClone(h.extractComposerContent(withScript)), { text: 'An toàn', entities: [] });
});

test('send gửi entities chuẩn, IME composition chặn gửi dở và extractPastePayload chống XSS', async () => {
  const h = setupComposerHarness();
  const ed = h.nodes.get('text');
  ed.appendChild(new MockText('Xin '));
  ed.appendChild(elNode('strong', 'chào'));

  // Khi đang gõ IME (compositionstart) -> nhấn Enter KHÔNG gửi.
  ed.dispatchEvent(new Event('compositionstart'));
  const enterEvt = new Event('keydown');
  enterEvt.key = 'Enter';
  enterEvt.preventDefault = () => {};
  ed.dispatchEvent(enterEvt);
  assert.equal(h.posts.length, 0);

  // Kết thúc compose và chèn thêm chữ có dấu tiếng Việt -> không mất chữ và gửi đủ.
  ed.appendChild(new MockText(' tiếng Việt'));
  ed.dispatchEvent(new Event('compositionend'));
  ed.dispatchEvent(enterEvt);
  await new Promise(setImmediate);

  assert.equal(h.posts.length, 1);
  assert.equal(h.posts[0].path, '/api/chats/42/send');
  assert.deepEqual(h.posts[0].body, {
    text: 'Xin chào tiếng Việt',
    entities: [{ type: 'bold', offset: 4, length: 4 }],
    reply_to: 0,
  });
  assert.equal(ed.textContent, '');

  // Dán HTML có script từ ngoài -> chỉ giữ chữ thuần, không có entities.
  const extPaste = h.extractPastePayload({
    getData: (type) => (type === 'text/html'
      ? '<script>alert(1)</script><b>Dán ngoài</b><img src=x onerror=alert(2)>'
      : type === 'text/plain' ? 'Dán ngoài' : ''),
  });
  assert.deepEqual(structuredClone(extPaste), { text: 'Dán ngoài', entities: [] });

  const htmlOnlyPaste = h.extractPastePayload({
    getData: (type) => (type === 'text/html'
      ? '<img src=x onerror=alert(1)><script>evil()</script><style>.x{}</style><span>Xin chào</span>'
      : ''),
  });
  assert.deepEqual(structuredClone(htmlOnlyPaste), { text: 'Xin chào', entities: [] });

  // Dán nội bộ giữ định dạng cơ bản hợp lệ và lọc bỏ loại lạ / vượt biên.
  const intPaste = h.extractPastePayload({
    getData: (type) => (type === 'application/x-tggw-composer'
      ? JSON.stringify({
        text: '🌙 Đậm',
        entities: [
          { type: 'bold', offset: 3, length: 3 },
          { type: 'script', offset: 0, length: 2 },
          { type: 'italic', offset: 1, length: 2 },
        ],
      })
      : type === 'text/plain' ? '🌙 Đậm' : ''),
  });
  assert.deepEqual(structuredClone(intPaste), {
    text: '🌙 Đậm',
    entities: [{ type: 'bold', offset: 3, length: 3 }],
  });
});

test('bảng biến CSS đủ 10 chủ đề + 2 chủ đề khối mã và mọi cặp chữ/nền đạt tương phản WCAG >= 4.5:1', () => {
  const blocks = parseThemeCssVars(styleBlock);
  const requiredVars = [
    '--bg', '--panel', '--fg', '--mut', '--line', '--acc', '--btn-bg', '--btn-fg',
    '--in', '--out', '--badge', '--err', '--ok', '--code-bg', '--code-fg', '--code-border', '--code-font',
  ];
  for (const theme of ['system-light', 'system-dark', 'light', 'dark', 'midnight', 'telegram', 'green', 'purple', 'orange', 'pink', 'high-contrast']) {
    const vars = resolveThemeVars(blocks, theme);
    for (const v of requiredVars) {
      assert.ok(vars[v], `${theme} thiếu biến ${v}`);
    }
    const pairs = [
      ['fg/in', vars['--fg'], vars['--in']],
      ['fg/out', vars['--fg'], vars['--out']],
      ['fg/panel', vars['--fg'], vars['--panel']],
      ['fg/bg', vars['--fg'], vars['--bg']],
      ['mut/in', vars['--mut'], vars['--in']],
      ['mut/out', vars['--mut'], vars['--out']],
      ['mut/panel', vars['--mut'], vars['--panel']],
      ['acc/in', vars['--acc'], vars['--in']],
      ['acc/panel', vars['--acc'], vars['--panel']],
      ['btn-fg/btn-bg', vars['--btn-fg'], vars['--btn-bg']],
    ];
    for (const [label, fg, bg] of pairs) {
      const ratio = contrastRatio(fg, bg);
      const min = theme === 'high-contrast' && label.startsWith('fg/') ? 7.0 : 4.5;
      assert.ok(ratio >= min, `${theme} (${label}: ${fg} trên ${bg}) chỉ đạt ${ratio.toFixed(2)} < ${min}`);
    }
  }

  for (const codeMode of ['light', 'dark']) {
    const vars = resolveThemeVars(blocks, 'light', codeMode);
    assert.match(vars['--code-font'], /monospace/);
    for (const tok of ['--code-fg', '--code-kw', '--code-str', '--code-cm', '--code-num']) {
      const ratio = contrastRatio(vars[tok], vars['--code-bg']);
      assert.ok(ratio >= 4.5, `code-${codeMode} (${tok}: ${vars[tok]} trên ${vars['--code-bg']}) = ${ratio.toFixed(2)} < 4.5`);
    }
  }
});

test('renderText tô màu cú pháp cơ bản cho khối pre an toàn XSS', () => {
  const h = setupComposerHarness();
  const box = new MockNode('div');
  const codeSrc = 'const msg = "<script>alert(1)</script>"; // 2026\nreturn 42;';
  h.renderText(box, codeSrc, [{ type: 'pre', offset: 0, length: codeSrc.length, language: 'js' }]);
  assert.equal(box.find('script').length, 0);
  const pre = box.find('pre')[0];
  const code = pre?.find('code')[0];
  assert.equal(code?.dataset.language, 'js');
  assert.equal(code?.textContent, codeSrc);
  assert.ok(code.findByClass('tok-kw').some((n) => n.textContent === 'const'));
  assert.ok(code.findByClass('tok-str').some((n) => n.textContent.includes('<script>alert(1)</script>')));
  assert.ok(code.findByClass('tok-cm').some((n) => n.textContent.includes('// 2026')));
  assert.ok(code.findByClass('tok-num').some((n) => n.textContent === '42'));
});

test('chọn chủ đề mới, nền khung chat, chủ đề khối mã và nút Đặt lại hoạt động đồng bộ', async () => {
  const h = setupSettingsHarness();
  await h.loadAppearance();
  assert.equal(h.root.dataset.theme, 'system');
  assert.equal(h.root.dataset.chatBg, 'plain');
  assert.equal(h.root.dataset.codeTheme, 'light');

  h.nodes.get('settings-open').click();
  await h.flush();
  h.navItems.find((b) => b.dataset.section === 'appearance').click();

  h.nodes.get('theme-midnight').click();
  h.nodes.get('chat-bg-pattern').click();
  h.nodes.get('code-theme-dark').click();
  assert.equal(h.root.dataset.theme, 'midnight');
  assert.equal(h.root.dataset.surface, 'dark');
  assert.equal(h.root.dataset.chatBg, 'pattern');
  assert.equal(h.root.dataset.codeTheme, 'dark');

  // Đóng khi chưa lưu -> hoàn tác.
  h.nodes.get('settings-close').click();
  assert.equal(h.root.dataset.theme, 'system');
  assert.equal(h.root.dataset.chatBg, 'plain');
  assert.equal(h.root.dataset.codeTheme, 'light');

  // Lưu -> POST kèm các trường mới.
  h.nodes.get('settings-open').click();
  await h.flush();
  h.nodes.get('theme-telegram').click();
  h.nodes.get('chat-bg-gradient').click();
  h.nodes.get('code-theme-dark').click();
  await h.submitAppearance();
  const post = h.requests.find((r) => r.method === 'POST' && r.path === '/api/settings/appearance');
  assert.deepEqual(post.body, {
    font_family: 'system',
    font_size: 15,
    theme: 'telegram',
    bubble_style: 'classic',
    accent: 'blue',
    chat_bg: 'gradient',
    code_theme: 'dark',
  });

  // Bấm Đặt lại -> trả toàn bộ bản nháp về mặc định.
  h.nodes.get('appearance-reset').click();
  assert.equal(h.root.dataset.theme, 'system');
  assert.equal(h.root.dataset.chatBg, 'plain');
  assert.equal(h.root.dataset.codeTheme, 'light');
});

test('đột biến đối chứng: bỏ chốt IME, sai độ dài UTF-16 emoji và cặp màu tương phản kém đều bị bắt', () => {
  // Đột biến 1: bỏ chốt isComposing ở keydown và send khiến Enter đang compose gọi send() sớm.
  const imeMutant = setupComposerHarness((code) => {
    assert.ok(code.includes('if (e.isComposing || isComposing || e.keyCode === 229) return;'));
    assert.ok(code.includes('if (!t || !cur || isComposing) return;'));
    return code
      .replace('if (e.isComposing || isComposing || e.keyCode === 229) return;', '')
      .replace('if (!t || !cur || isComposing) return;', 'if (!t || !cur) return;');
  });
  const ed = imeMutant.nodes.get('text');
  ed.appendChild(new MockText('đang gõ'));
  ed.dispatchEvent(new Event('compositionstart'));
  const enterEvt = new Event('keydown');
  enterEvt.key = 'Enter';
  enterEvt.preventDefault = () => {};
  ed.dispatchEvent(enterEvt);
  assert.equal(imeMutant.posts.length, 1);

  // Đột biến 2: dùng Array.from(chunk).length (codepoint) thay vì chunk.length (UTF-16) làm lệch offset sau emoji.
  const utf16Mutant = setupComposerHarness((code) => {
    assert.ok(code.includes('outLen += chunk.length;'));
    return code.replace('outLen += chunk.length;', 'outLen += Array.from(chunk).length;');
  });
  const badRes = utf16Mutant.extractComposerContent(elNode('div', '🌙 ', elNode('strong', 'Chào')));
  assert.notEqual(badRes.entities[0].offset, 3);

  // Đột biến 3: hàm đo tương phản từ chối cặp màu thấp (#777777 trên #999999).
  assert.equal(contrastRatio('#777777', '#999999') < 4.5, true);
});
