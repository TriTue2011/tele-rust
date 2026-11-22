const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const vm = require('node:vm');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const fullScript = /<script>([\s\S]*?)<\/script>/.exec(html)[1];
const rendererCode = html.slice(html.indexOf('  const URL_RE'), html.indexOf('  let lastDay'));
const composerCode = html.slice(html.indexOf('  // ---- định dạng soạn tin'), html.indexOf('  // ---- cập nhật theo thời gian thực'));
const settingsCode = html.slice(html.indexOf('  // ---- cài đặt chung, giao diện và hồ sơ'), html.indexOf('  // ---- dữ liệu và lưu trữ'));

class MockClassList {
  constructor() { this.set = new Set(); }
  add(...names) { names.forEach((n) => this.set.add(n)); }
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
    this.selectionStart = 0;
    this.selectionEnd = 0;
    this.scrollHeight = 40;
    this._firstChild = { textContent: '' };
    if (text !== undefined && text !== '') {
      this.appendChild(new MockText(text));
    }
  }
  get childNodes() { return this.children; }
  get firstChild() { return this.children[0] || this._firstChild; }
  get textContent() { return this.children.map((c) => (typeof c === 'string' ? c : c.textContent)).join(''); }
  set textContent(v) {
    this.children = [];
    if (v !== '' && v !== undefined && v !== null) this.appendChild(new MockText(String(v)));
  }
  get selectedOptions() {
    const picked = this.children.find((c) => c.value === this.value);
    return picked ? [picked] : (this.children[0] ? [this.children[0]] : []);
  }
  append(...nodes) { for (const n of nodes) this.appendChild(n); }
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
  setSelectionRange(s, e) { this.selectionStart = s; this.selectionEnd = e; }
  showModal() { this.open = true; }
  close() { this.open = false; this.dispatchEvent(new Event('close')); }
  focus() { this.focused = true; }
  click() { this.dispatchEvent(new Event('click')); }
  find(tag) {
    const t = String(tag).toLowerCase();
    return this.children.flatMap((c) => (c instanceof MockNode ? [...(c.tag === t ? [c] : []), ...c.find(t)] : []));
  }
}

function setupComposerHarness() {
  const nodes = new Map();
  for (const id of ['text', 'send', 'reply', 'reply-txt', 'reply-x', 'attach', 'file',
    'fmt-bold', 'fmt-italic', 'fmt-underline', 'fmt-strike', 'fmt-code', 'fmt-clear']) {
    nodes.set(id, new MockNode('div'));
  }
  const posts = [];
  const appended = [];
  const doc = {
    createTextNode: (t) => new MockText(String(t)),
    createElement: (tag) => new MockNode(tag),
    createDocumentFragment: () => new MockNode('fragment'),
    getSelection: () => null,
  };
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
      return { id: 900, chat_id: 42, date: 1700000000, text: body.text, entities: body.entities || [], kind: 'text', outgoing: true };
    },
    appendMessages: (list) => appended.push(...list),
    scheduleChats: () => {},
    toast: () => {},
    Set, Map, Event, Date, Math, Number, Object, Array, String, Boolean, JSON, RegExp,
  };
  vm.runInNewContext(
    `${rendererCode}\n${composerCode}\nthis.exports = { extractComposerContent, applyComposerFormat, renderText, send };`,
    ctx,
  );
  return { ...ctx.exports, nodes, posts, appended };
}

function setupSettingsHarness(source = settingsCode, initialProfile) {
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
    createTextNode: (t) => String(t),
    querySelectorAll: (selector) => (selector === '#settings-dialog .settings-item' ? navItems : []),
  };
  const profileData = initialProfile || {
    id: 7, name: 'An Lê', first_name: 'An', last_name: 'Lê',
    bio: 'Xin chào', username: 'an_le', phone_masked: '+849•••••000',
  };
  let serverAppearance = { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' };
  const requests = [];
  const ctx = {
    $: (id) => {
      if (!nodes.has(id)) nodes.set(id, new MockNode(id === 'account' ? 'select' : 'div'));
      return nodes.get(id);
    },
    el: (tag, cls, text) => new MockNode(tag, cls, text),
    document: doc,
    account: '+84900000000',
    initials: (s) => (s || '?')[0].toUpperCase(),
    api: async (method, path, body) => {
      requests.push({ method, path, body: body === undefined ? undefined : structuredClone(body) });
      if (method === 'GET' && path === '/api/settings/appearance') return structuredClone(serverAppearance);
      if (method === 'POST' && path === '/api/settings/appearance') {
        serverAppearance = structuredClone(body);
        return structuredClone(serverAppearance);
      }
      if (method === 'GET' && path === '/api/profile') return structuredClone(profileData);
      if (method === 'POST' && path === '/api/profile') {
        assert.equal(body.confirm, true, 'POST /api/profile phải kèm confirm=true');
        if (body.first_name !== undefined) profileData.first_name = body.first_name;
        if (body.last_name !== undefined) profileData.last_name = body.last_name;
        if (body.bio !== undefined) profileData.bio = body.bio;
        if (body.username !== undefined) profileData.username = body.username;
        profileData.name = `${profileData.first_name} ${profileData.last_name}`.trim();
        return structuredClone(profileData);
      }
      throw new Error(`unexpected api ${method} ${path}`);
    },
    Event,
    Number,
    Math,
    Object,
    String,
    Array,
    Boolean,
  };
  const accSelect = ctx.$('account');
  accSelect.append(Object.assign(new MockNode('option', '', 'An Lê · online'), { value: '+84900000000' }));
  accSelect.value = '+84900000000';
  for (const id of [
    'settings-dialog', 'settings-shell', 'settings-open', 'settings-close', 'settings-back', 'settings-title',
    'settings-nav-profile-card', 'settings-nav-avatar', 'settings-nav-name', 'settings-nav-sub',
    'settings-panel-profile', 'settings-panel-appearance', 'settings-panel-storage', 'settings-panel-calls',
    'settings-panel-accounts', 'settings-panel-soon', 'soon-heading', 'soon-description',
    'profile-form', 'profile-phone-masked', 'profile-first-name', 'profile-last-name', 'profile-bio',
    'profile-bio-count', 'profile-username', 'profile-status', 'profile-error', 'profile-save', 'profile-revert',
    'profile-confirm', 'profile-confirm-summary', 'profile-confirm-error', 'profile-confirm-ok', 'profile-confirm-cancel',
    'appearance-preview', 'appearance-form', 'appearance-font', 'appearance-size', 'appearance-size-value',
    'appearance-size-dec', 'appearance-size-inc', 'appearance-status', 'appearance-error', 'appearance-save',
    'appearance-reset', 'theme-system', 'theme-light', 'theme-dark', 'bubble-classic', 'bubble-rounded',
    'bubble-compact', 'accent-blue', 'accent-green', 'accent-purple', 'accent-orange', 'accent-pink', 'accent-teal',
    'me', 'mesub',
  ]) {
    ctx.$(id);
  }

  vm.runInNewContext(
    `${source}\nthis.exports = { maskPhone, normalizeAppearance, applyAppearance, loadAppearance, selectSettingsSection };`,
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
    submitProfile: async () => {
      const ev = new Event('submit');
      ev.preventDefault = () => {};
      nodes.get('profile-form').dispatchEvent(ev);
      await flush();
    },
  };
}

test('web/app.html sạch bảo mật, không có chuỗi cấm và giữ nguyên cấu trúc G3/G4/G6 Sắp có', () => {
  new vm.Script(fullScript);
  assert.ok(rendererCode.length > 0 && composerCode.length > 0 && settingsCode.length > 0);
  for (const banned of ['innerHTML', 'localStorage', 'sessionStorage', 'gw_token', 'GATEWAY_TOKEN']) {
    assert.equal(html.includes(banned), false, `không được chứa ${banned}`);
  }
  const soonSection = html.match(/<section id="settings-panel-soon"[\s\S]*?<\/section>/)[0];
  assert.doesNotMatch(soonSection, /<(input|select|textarea)\b/i);
  for (const key of ['notifications', 'privacy', 'folders', 'devices']) {
    const re = new RegExp(`data-section="${key}"[^>]*>[\\s\\S]*?Sắp có`);
    assert.match(html, re);
  }
  assert.match(html, /@media \(max-width:699px\)/);
  assert.match(html, /--chat-font-size/);
  assert.match(html, /data-theme="dark"/);
  assert.match(html, /data-bubble="rounded"/);
  assert.match(html, /data-accent="green"/);
});

const clone = (v) => structuredClone(v);

test('định dạng soạn tin WYSIWYG tạo đúng UTF-16 entities, giữ nguyên ** tự gõ, chống XSS và gửi qua /send', async () => {
  const { extractComposerContent, renderText, send, nodes, posts } = setupComposerHarness();
  const makeEl = (tag, ...items) => {
    const n = new MockNode(tag);
    for (const item of items) {
      if (typeof item === 'string') n.appendChild(new MockText(item));
      else n.appendChild(item);
    }
    return n;
  };

  const parsed = extractComposerContent(
    makeEl('div', '🌙 ', makeEl('strong', 'Chào'), ' ', makeEl('em', 'bạn'), ' ', makeEl('code', 'code')),
  );
  assert.equal(parsed.text, '🌙 Chào bạn code');
  assert.deepEqual(clone(parsed.entities), [
    { type: 'bold', offset: 3, length: 4 },
    { type: 'italic', offset: 8, length: 3 },
    { type: 'code', offset: 12, length: 4 },
  ]);

  const preNode = makeEl('pre', 'ls -la');
  preNode.dataset.language = 'sh';
  const nested = extractComposerContent(
    makeEl('div', makeEl('strong', 'Đậm ', makeEl('em', 'lồng')), ' và', preNode),
  );
  assert.equal(nested.text, 'Đậm lồng và\nls -la');
  assert.deepEqual(clone(nested.entities), [
    { type: 'bold', offset: 0, length: 8 },
    { type: 'italic', offset: 4, length: 4 },
    { type: 'pre', offset: 12, length: 6, language: 'sh' },
  ]);

  const previewNode = new MockNode('div');
  renderText(previewNode, nested.text, nested.entities);
  assert.equal(previewNode.find('strong')[0]?.textContent, 'Đậm lồng');
  assert.equal(previewNode.find('strong')[0]?.find('em')[0]?.textContent, 'lồng');
  assert.equal(previewNode.find('pre')[0]?.find('code')[0]?.dataset.language, 'sh');

  const literal = extractComposerContent(makeEl('div', '**giữ nguyên** và https://example.test/a__b__c'));
  assert.deepEqual(clone(literal.entities), []);
  assert.equal(literal.text, '**giữ nguyên** và https://example.test/a__b__c');

  const xss = extractComposerContent(makeEl('div', makeEl('strong', '<script>alert(1)</script>')));
  const xssNode = new MockNode('div');
  renderText(xssNode, xss.text, xss.entities);
  assert.equal(xssNode.find('script').length, 0);
  assert.equal(xssNode.find('strong')[0]?.textContent, '<script>alert(1)</script>');

  const input = nodes.get('text');
  input.appendChild(new MockText('Xin '));
  input.appendChild(makeEl('em', 'chào'));
  await send();
  assert.equal(posts.length, 1);
  assert.equal(posts[0].path, '/api/chats/42/send');
  assert.equal(posts[0].body.text, 'Xin chào');
  assert.deepEqual(posts[0].body.entities, [{ type: 'italic', offset: 4, length: 4 }]);
  assert.equal(input.textContent, '');
});

test('khung cài đặt điều hướng 2 bước, mục sắp có trung thực, giao diện áp ngay và hoàn tác khi chưa lưu', async () => {
  const h = setupSettingsHarness();

  await h.loadAppearance();
  assert.equal(h.root.dataset.theme, 'system');
  assert.equal(h.root.style.getPropertyValue('--chat-font-size'), '15px');
  assert.deepEqual(clone(h.normalizeAppearance({ font_family: 'bad', font_size: 99, theme: 'neon', bubble_style: 'x', accent: 'red' })), {
    font_family: 'system', font_size: 20, theme: 'system', bubble_style: 'classic', accent: 'blue',
  });

  h.nodes.get('settings-open').click();
  await h.flush();
  assert.equal(h.nodes.get('settings-dialog').open, true);
  assert.equal(h.nodes.get('settings-shell').classList.contains('in-detail'), false);

  h.navItems.find((b) => b.dataset.section === 'notifications').click();
  assert.equal(h.nodes.get('settings-shell').classList.contains('in-detail'), true);
  assert.equal(h.nodes.get('settings-panel-soon').hidden, false);
  assert.equal(h.nodes.get('soon-heading').textContent, 'Thông báo và âm thanh');

  h.nodes.get('settings-back').click();
  assert.equal(h.nodes.get('settings-shell').classList.contains('in-detail'), false);

  h.navItems.find((b) => b.dataset.section === 'appearance').click();
  h.nodes.get('theme-dark').click();
  h.nodes.get('bubble-rounded').click();
  h.nodes.get('accent-green').click();
  h.nodes.get('appearance-font').value = 'mono';
  h.nodes.get('appearance-font').dispatchEvent(new Event('change'));
  h.nodes.get('appearance-size').value = '18';
  h.nodes.get('appearance-size').dispatchEvent(new Event('input'));

  assert.equal(h.root.dataset.theme, 'dark');
  assert.equal(h.root.dataset.bubble, 'rounded');
  assert.equal(h.root.dataset.accent, 'green');
  assert.equal(h.root.style.getPropertyValue('--chat-font-size'), '18px');
  assert.match(h.root.style.getPropertyValue('--chat-font'), /monospace/);

  // Đóng khi chưa lưu -> hoàn tác về giá trị đã lưu.
  h.nodes.get('settings-close').click();
  assert.equal(h.root.dataset.theme, 'system');
  assert.equal(h.root.style.getPropertyValue('--chat-font-size'), '15px');
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, 0);

  // Mở lại, sửa và bấm Lưu -> POST lên máy chủ.
  h.nodes.get('settings-open').click();
  await h.flush();
  h.nodes.get('theme-light').click();
  h.nodes.get('appearance-size-inc').click();
  await h.submitAppearance();
  const post = h.requests.find((r) => r.method === 'POST' && r.path === '/api/settings/appearance');
  assert.deepEqual(post.body, { font_family: 'system', font_size: 16, theme: 'light', bubble_style: 'classic', accent: 'blue' });
  h.nodes.get('settings-close').click();
  assert.equal(h.root.dataset.theme, 'light');
  assert.equal(h.root.style.getPropertyValue('--chat-font-size'), '16px');
});

test('hồ sơ che số điện thoại, kiểm dữ liệu cục bộ và bắt buộc xác nhận trước khi POST', async () => {
  const h = setupSettingsHarness();
  assert.equal(h.maskPhone('+84900000000'), '+849•••••000');

  h.nodes.get('settings-open').click();
  await h.flush();
  assert.equal(h.nodes.get('profile-phone-masked').textContent, '+849•••••000');
  assert.equal(h.nodes.get('profile-bio-count').textContent, '8/70');

  // Sai dữ liệu cục bộ -> không mở xác nhận, không gọi API.
  h.nodes.get('profile-username').value = 'bad-user';
  await h.submitProfile();
  assert.equal(h.nodes.get('profile-confirm').open, false);
  assert.match(h.nodes.get('profile-error').textContent, /Tên người dùng/);
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, 0);

  // Sửa hợp lệ -> mở hộp thoại xác nhận, chưa gửi POST cho đến khi bấm xác nhận.
  h.nodes.get('profile-first-name').value = 'Minh';
  h.nodes.get('profile-bio').value = 'Tiểu sử mới';
  h.nodes.get('profile-username').value = '@minh_le';
  await h.submitProfile();
  assert.equal(h.nodes.get('profile-confirm').open, true);
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, 0);
  assert.match(h.nodes.get('profile-confirm-summary').textContent, /Minh/);

  h.nodes.get('profile-confirm-cancel').click();
  assert.equal(h.nodes.get('profile-confirm').open, false);
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, 0);

  await h.submitProfile();
  h.nodes.get('profile-confirm-ok').click();
  await h.flush();
  assert.equal(h.nodes.get('profile-confirm').open, false);
  const post = h.requests.find((r) => r.method === 'POST' && r.path === '/api/profile');
  assert.deepEqual(post.body, { confirm: true, first_name: 'Minh', bio: 'Tiểu sử mới', username: 'minh_le' });
  assert.equal(h.nodes.get('settings-nav-name').textContent, 'Minh Lê');
});

test('đột biến: bỏ chốt xác nhận trước khi POST /api/profile làm test kiểm soát thất bại', async () => {
  const mutant = settingsCode.replace('$("profile-confirm").showModal();', '$("profile-confirm-ok").click();');
  assert.notEqual(mutant, settingsCode);
  const h = setupSettingsHarness(mutant);
  h.nodes.get('settings-open').click();
  await h.flush();
  h.nodes.get('profile-first-name').value = 'Bị lọt';
  await h.submitProfile();
  assert.equal(h.nodes.get('profile-confirm').open, false);
  assert.equal(h.requests.filter((r) => r.method === 'POST' && r.path === '/api/profile').length, 1);
});
