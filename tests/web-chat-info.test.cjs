const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const vm = require('node:vm');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const fullScript = /<script>([\s\S]*?)<\/script>/.exec(html)[1];
const settingsMarker = '  // ---- cài đặt chung, giao diện và hồ sơ';
const storageMarker = '  // ---- dữ liệu và lưu trữ';
const chatInfoMarker = '  // ---- thông tin hội thoại và phông khung chat';
const startMarker = '  start();';

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

class MockNode extends EventTarget {
  constructor(tag = 'div', className = '', text = '') {
    super();
    this.tag = tag;
    this.className = className;
    this.children = text === undefined || text === '' ? [] : [String(text)];
    this.attributes = new Map();
    this.dataset = {};
    this.classList = new MockClassList();
    this.style = {
      props: new Map(),
      setProperty(k, v) { this.props.set(k, v); },
      getPropertyValue(k) { return this.props.get(k) || ''; },
    };
    this.value = '';
    this.checked = false;
    this.disabled = false;
    this.hidden = false;
    this.open = false;
    this.focused = false;
    this._firstChild = { textContent: '' };
  }
  get firstChild() { return this.children[0] && typeof this.children[0] === 'object' ? this.children[0] : this._firstChild; }
  get options() { return this.children; }
  get textContent() { return this.children.map((c) => (typeof c === 'string' ? c : c.textContent)).join(''); }
  set textContent(v) { this.children = v === '' || v === undefined || v === null ? [] : [String(v)]; }
  get selectedOptions() {
    const picked = this.children.find((c) => c.value === this.value);
    return picked ? [picked] : (this.children[0] ? [this.children[0]] : []);
  }
  append(...nodes) { this.children.push(...nodes); }
  appendChild(node) { this.children.push(node); return node; }
  replaceChildren(...nodes) { this.children = nodes; }
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
    return this.children.flatMap((c) => (c instanceof MockNode ? [...(c.tag === tag ? [c] : []), ...c.find(tag)] : []));
  }
}

function defaultStoragePolicy() {
  return {
    mode: 'local',
    keep: { private: 604800, groups: 604800, channels: null },
    exceptions: [],
    max_cache_bytes: 2 * 1024 * 1024 * 1024,
    stream_ttl_s: 600,
    auto_download: Object.fromEntries(['private', 'groups', 'channels'].map((chat) => [
      chat,
      Object.fromEntries(['photo', 'video', 'file', 'voice', 'video_note', 'gif'].map((k) => [k, { on: false, max_bytes: 10485760 }])),
    ])),
  };
}

function setupHarness(options = {}) {
  const settingsStart = html.indexOf(settingsMarker);
  const storageStart = html.indexOf(storageMarker);
  const chatInfoStart = html.indexOf(chatInfoMarker);
  const startPos = html.indexOf(startMarker, chatInfoStart > -1 ? chatInfoStart : 0);
  assert.ok(settingsStart > -1 && storageStart > settingsStart, 'Không tìm thấy khối settingsCode');
  assert.ok(chatInfoStart > -1 && startPos > chatInfoStart, 'Chưa có khối // ---- thông tin hội thoại và phông khung chat trong web/app.html');

  const settingsCode = html.slice(settingsStart, storageStart);
  const rawChatInfoCode = html.slice(chatInfoStart, startPos);
  const chatInfoCode = options.mutateChatInfo ? options.mutateChatInfo(rawChatInfoCode) : rawChatInfoCode;

  const nodes = new Map();
  const root = new MockNode('html');
  const navItems = ['profile', 'appearance', 'storage', 'calls', 'accounts', 'notifications', 'privacy', 'folders', 'devices']
    .map((section) => {
      const btn = new MockNode('button', 'settings-item');
      btn.dataset.section = section;
      return btn;
    });
  const doc = Object.assign(new EventTarget(), {
    documentElement: root,
    createTextNode: (t) => String(t),
    querySelectorAll: (selector) => (selector === '#settings-dialog .settings-item' ? navItems : []),
  });

  const chats = options.chats || [
    { id: 42, title: '<script>alert(1)</script> An', kind: 'private', user_id: 77, pinned: false, muted: false },
    { id: -1001, title: 'Nhóm thử <script>g()</script>', kind: 'group', pinned: true, muted: false },
  ];
  const infoMap = options.infoMap || {
    42: {
      chat_id: 42,
      title: '<script>alert(1)</script> An',
      kind: 'private',
      username: 'an_xss',
      bio: '<img src=x onerror=alert(1)> Tiểu sử',
      phone_masked: '+849•••••007',
      user_id: 77,
      first_name: 'An',
      last_name: 'Lê',
      status: 'online',
      is_contact: false,
      is_blocked: false,
      is_bot: false,
      avatar_file: null,
      notification: { use_default_mute_for: true, mute_for: 0, use_default_show_preview: true, show_preview: true },
      auto_delete_seconds: 0,
      pinned: false,
      archived: false,
      marked_unread: false,
      unread: 0,
      has_protected_content: false,
    },
    [-1001]: {
      chat_id: -1001,
      title: 'Nhóm thử <script>g()</script>',
      kind: 'group',
      username: 'nhom_thu',
      bio: '<svg onload=alert(1)> Mô tả nhóm',
      member_count: 12,
      can_get_members: true,
      my_role: 'admin',
      is_admin: true,
      my_permissions: {
        can_send_basic_messages: true,
        can_change_info: true,
        can_invite_users: true,
        can_pin_messages: true,
        can_delete_messages: false,
      },
      invite_link: 'https://t.me/+moi_thu',
      notification: { use_default_mute_for: false, mute_for: 3600, use_default_show_preview: false, show_preview: false },
      auto_delete_seconds: 604800,
      pinned: true,
      archived: false,
      marked_unread: false,
      unread: 3,
      has_protected_content: false,
    },
  };
  const membersMap = options.membersMap || {
    [-1001]: [
      { user_id: 1, name: 'Chủ Nhóm', username: 'chu_nhom', role: 'creator', custom_title: 'Founder', is_bot: false, status: 'online' },
      { user_id: 2, name: '<script>m()</script> Mod', username: 'mod_a', role: 'admin', custom_title: '<img onerror=1> Bảo vệ', is_bot: false, status: 'offline' },
      { user_id: 3, name: 'Thành viên B', username: 'mem_b', role: 'member', custom_title: '', is_bot: false, status: 'recently' },
    ],
  };

  let serverAppearance = { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' };
  let serverStorage = defaultStoragePolicy();
  const requests = [];
  const toasts = [];
  const clipboardWrites = [];
  let chatsReloaded = 0;

  const ctx = {
    $: (id) => {
      if (!nodes.has(id)) {
        const tag = id === 'account' || id.endsWith('-font') || id.endsWith('-family') || id === 'ci-mute' || id === 'ci-autodelete' || id === 'ci-cache-keep' ? 'select' : 'div';
        nodes.set(id, new MockNode(tag));
      }
      return nodes.get(id);
    },
    el: (tag, cls, text) => new MockNode(tag, cls, text),
    document: doc,
    account: '+84900000000',
    initials: (s) => (s || '?')[0].toUpperCase(),
    chats,
    cur: options.cur ?? 42,
    curKind: options.curKind ?? 'private',
    toast: (m) => toasts.push(m),
    navigator: { clipboard: { writeText: async (t) => { clipboardWrites.push(t); } } },
    loadChats: async () => { chatsReloaded++; },
    scheduleChats: () => {},
    renderList: () => {},
    maskPhone: undefined,
    api: async (method, path, body) => {
      requests.push({ method, path, body: body === undefined ? undefined : structuredClone(body) });
      const u = new URL(path, 'http://localhost');
      if (method === 'GET' && u.pathname === '/api/settings/appearance') return structuredClone(serverAppearance);
      if (method === 'POST' && u.pathname === '/api/settings/appearance') {
        serverAppearance = structuredClone(body);
        return structuredClone(serverAppearance);
      }
      if (method === 'GET' && u.pathname === '/api/profile') {
        return { id: 7, name: 'An Lê', first_name: 'An', last_name: 'Lê', bio: 'Xin chào', username: 'an_le', phone_masked: '+849•••••000' };
      }
      if (method === 'GET' && u.pathname === '/api/storage/usage') {
        return { total_bytes: 5 * 1024 * 1024, by_type: {}, top_chats: [{ chat_id: 42, bytes: 2048 }, { chat_id: -1001, bytes: 4096 }] };
      }
      if (method === 'GET' && u.pathname === '/api/settings/storage') return structuredClone(serverStorage);
      if (method === 'POST' && u.pathname === '/api/settings/storage') {
        serverStorage = structuredClone(body);
        return structuredClone(serverStorage);
      }
      const infoMatch = /^\/api\/chats\/(-?\d+)\/info$/.exec(u.pathname);
      if (method === 'GET' && infoMatch) {
        return structuredClone(infoMap[Number(infoMatch[1])]);
      }
      const membersMatch = /^\/api\/chats\/(-?\d+)\/members$/.exec(u.pathname);
      if (method === 'GET' && membersMatch) {
        const chatId = Number(membersMatch[1]);
        const q = (u.searchParams.get('q') || '').trim().toLowerCase();
        const limit = Number(u.searchParams.get('limit') || 50);
        const list = (membersMap[chatId] || []).filter((m) => !q || m.name.toLowerCase().includes(q) || m.username.toLowerCase().includes(q)).slice(0, limit);
        return { chat_id: chatId, total_count: list.length, members: structuredClone(list) };
      }
      const settingsMatch = /^\/api\/chats\/(-?\d+)\/settings$/.exec(u.pathname);
      if (method === 'POST' && settingsMatch) {
        return { ok: true, chat_id: Number(settingsMatch[1]), ...structuredClone(body) };
      }
      const clearMatch = /^\/api\/chats\/(-?\d+)\/clear_history$/.exec(u.pathname);
      if (method === 'POST' && clearMatch) {
        assert.equal(body.confirm, true, 'clear_history phải kèm confirm=true');
        return { ok: true, chat_id: Number(clearMatch[1]), deleted: true };
      }
      const leaveMatch = /^\/api\/chats\/(-?\d+)\/leave$/.exec(u.pathname);
      if (method === 'POST' && leaveMatch) {
        assert.equal(body.confirm, true, 'leave phải kèm confirm=true');
        return { ok: true, chat_id: Number(leaveMatch[1]), left: true };
      }
      if (method === 'POST' && (u.pathname === '/api/do/block_user' || u.pathname === '/api/do/unblock_user')) {
        return { user_id: body.user_id, blocked: u.pathname.endsWith('block_user') };
      }
      if (method === 'POST' && u.pathname === '/api/do/add_contact') {
        return { user_id: body.user_id, added: true };
      }
      throw new Error(`unexpected api ${method} ${path}`);
    },
    Event,
    URL,
    Set,
    Map,
    Number,
    Math,
    Object,
    String,
    Array,
    Boolean,
    Promise,
    setTimeout,
    clearTimeout,
    structuredClone,
    encodeURIComponent,
  };

  for (const id of [
    'app', 'head', 'title', 'font-open', 'chat-info-open',
    'font-popover', 'font-pop-close', 'font-pop-family', 'font-pop-size', 'font-pop-size-dec',
    'font-pop-size-inc', 'font-pop-size-val', 'font-pop-lh-compact', 'font-pop-lh-normal',
    'font-pop-lh-relaxed', 'font-pop-status', 'font-pop-error', 'font-pop-reset',
    'chat-info-drawer', 'chat-info-close', 'chat-info-status', 'chat-info-error',
    'chat-info-avatar', 'chat-info-title', 'chat-info-kind', 'chat-info-username',
    'chat-info-members-count', 'chat-info-sub', 'chat-info-bio',
    'ci-mute', 'ci-preview', 'ci-autodelete', 'ci-pin', 'ci-archive', 'ci-unread',
    'ci-cache-usage', 'ci-cache-forever', 'ci-cache-keep', 'ci-cache-status',
    'chat-info-private', 'ci-block', 'ci-add-contact', 'ci-clear-history',
    'chat-info-group', 'ci-my-role', 'ci-my-perms', 'ci-invite-box', 'ci-invite-link',
    'ci-invite-copy', 'ci-members-section', 'ci-member-search', 'ci-members-status', 'ci-members-list', 'ci-leave',
    'ci-confirm-dialog', 'ci-confirm-title', 'ci-confirm-summary', 'ci-confirm-revoke-wrap',
    'ci-confirm-revoke', 'ci-confirm-error', 'ci-confirm-cancel', 'ci-confirm-ok',
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
    'lh-compact', 'lh-normal', 'lh-relaxed', 'me', 'mesub', 'account',
  ]) {
    ctx.$(id);
  }
  for (const hiddenId of [
    'font-popover', 'chat-info-drawer', 'ci-invite-box', 'ci-add-contact',
    'chat-info-private', 'chat-info-group', 'ci-confirm-revoke-wrap',
  ]) {
    ctx.$(hiddenId).hidden = true;
  }
  ctx.$('account').append(Object.assign(new MockNode('option', '', 'An · online'), { value: '+84900000000' }));
  ctx.$('account').value = '+84900000000';

  vm.runInNewContext(
    `${settingsCode}\n${chatInfoCode}\nthis.exports = { loadAppearance, applyAppearance, openFontPopover, closeFontPopover, syncFontPopover, openChatInfo, closeChatInfo, setCur: (id, kind) => { cur = id; curKind = kind; } };`,
    ctx,
  );
  const flush = () => new Promise((r) => setTimeout(r, 10));
  return {
    ...ctx.exports,
    nodes,
    root,
    doc,
    requests,
    toasts,
    clipboardWrites,
    chatsReloaded: () => chatsReloaded,
    flush,
    submitAppearance: async () => {
      const ev = new Event('submit');
      ev.preventDefault = () => {};
      nodes.get('appearance-form').dispatchEvent(ev);
      await flush();
    },
    setCurChat: (id, kind) => { ctx.exports.setCur(id, kind); },
  };
}

test('web/app.html sạch bảo mật, không nhúng phông ngoài và giữ đúng cú pháp', () => {
  new vm.Script(fullScript);
  for (const banned of ['innerHTML', 'outerHTML', 'insertAdjacentHTML', 'localStorage', 'sessionStorage', 'gw_token', 'GATEWAY_TOKEN', '@font-face', 'fonts.googleapis', 'fonts.gstatic']) {
    assert.equal(html.includes(banned), false, `không được chứa ${banned}`);
  }
  assert.match(html, /id="font-open"/);
  assert.match(html, /id="chat-info-open"/);
  assert.match(html, /id="font-popover"/);
  assert.match(html, /id="chat-info-drawer"/);
  assert.match(html, /--chat-line-height/);
});

test('popover phông «Aa» áp ngay, lưu chung và đồng bộ 2 chiều với Cài đặt › Giao diện', async () => {
  const h = setupHarness();
  await h.loadAppearance();

  h.nodes.get('font-open').click();
  assert.equal(h.nodes.get('font-popover').hidden, false);
  assert.equal(h.nodes.get('font-open').getAttribute('aria-expanded'), 'true');

  // Đổi kiểu chữ, cỡ chữ, giãn dòng từ popover -> áp CSS ngay, đồng bộ sang trang Cài đặt và POST chung.
  h.nodes.get('font-pop-family').value = 'mono';
  h.nodes.get('font-pop-family').dispatchEvent(new Event('change'));
  await h.flush();
  assert.match(h.root.style.getPropertyValue('--chat-font'), /monospace/);
  assert.equal(h.nodes.get('appearance-font').value, 'mono');

  h.nodes.get('font-pop-size').value = '19';
  h.nodes.get('font-pop-size').dispatchEvent(new Event('input'));
  h.nodes.get('font-pop-size').dispatchEvent(new Event('change'));
  await h.flush();
  assert.equal(h.root.style.getPropertyValue('--chat-font-size'), '19px');
  assert.equal(h.nodes.get('appearance-size').value, '19');
  assert.equal(h.nodes.get('appearance-size-value').textContent, '19 px');

  h.nodes.get('font-pop-lh-relaxed').click();
  await h.flush();
  assert.equal(h.root.style.getPropertyValue('--chat-line-height'), '1.7');
  assert.equal(h.nodes.get('lh-relaxed').getAttribute('aria-pressed'), 'true');

  const lastPost = h.requests.filter((r) => r.method === 'POST' && r.path === '/api/settings/appearance').at(-1);
  assert.deepEqual(lastPost.body, {
    font_family: 'mono',
    font_size: 19,
    theme: 'system',
    bubble_style: 'classic',
    accent: 'blue',
    line_height: 'relaxed',
  });

  // Đổi từ Cài đặt › Giao diện -> đồng bộ ngược sang popover «Aa».
  h.nodes.get('lh-compact').click();
  h.nodes.get('appearance-size').value = '17';
  h.nodes.get('appearance-size').dispatchEvent(new Event('input'));
  await h.submitAppearance();
  assert.equal(h.nodes.get('font-pop-size').value, '17');
  assert.equal(h.nodes.get('font-pop-size-val').textContent, '17 px');
  assert.equal(h.nodes.get('font-pop-lh-compact').getAttribute('aria-pressed'), 'true');
  assert.equal(h.root.style.getPropertyValue('--chat-line-height'), '1.3');

  // Đặt lại từ popover «Aa».
  h.nodes.get('font-pop-reset').click();
  await h.flush();
  assert.equal(h.root.style.getPropertyValue('--chat-font-size'), '15px');
  assert.equal(h.root.style.getPropertyValue('--chat-line-height'), '1.45');
  assert.equal(h.nodes.get('appearance-font').value, 'system');
  assert.equal(h.nodes.get('lh-normal').getAttribute('aria-pressed'), 'true');
});

test('ngăn Thông tin hội thoại (Cá nhân): XSS an toàn, thông báo, tự xoá có confirm, bộ nhớ đệm, chặn/thêm danh bạ/xoá lịch sử', async () => {
  const h = setupHarness();
  await h.openChatInfo();
  await h.flush();

  const drawer = h.nodes.get('chat-info-drawer');
  assert.equal(drawer.hidden, false);
  assert.equal(drawer.find('script').length, 0);
  assert.equal(drawer.find('img').length, 0);
  assert.match(h.nodes.get('chat-info-title').textContent, /<script>alert\(1\)<\/script>/);
  assert.match(h.nodes.get('chat-info-bio').textContent, /<img src=x onerror=alert\(1\)>/);
  assert.equal(h.nodes.get('chat-info-username').textContent, '@an_xss');
  assert.equal(h.nodes.get('chat-info-private').hidden, false);
  assert.equal(h.nodes.get('chat-info-group').hidden, true);
  assert.equal(h.nodes.get('ci-add-contact').hidden, false);

  // Thông báo riêng: tắt tiếng & xem trước.
  h.nodes.get('ci-mute').value = '3600';
  h.nodes.get('ci-mute').dispatchEvent(new Event('change'));
  await h.flush();
  const muteReq = h.requests.filter((r) => r.path === '/api/chats/42/settings').at(-1);
  assert.deepEqual(muteReq.body.notification, { use_default_mute_for: false, mute_for: 3600 });

  h.nodes.get('ci-preview').checked = false;
  h.nodes.get('ci-preview').dispatchEvent(new Event('change'));
  await h.flush();
  const prevReq = h.requests.filter((r) => r.path === '/api/chats/42/settings').at(-1);
  assert.deepEqual(prevReq.body.notification, { use_default_show_preview: false, show_preview: false });

  // Tự xoá: bắt buộc hộp xác nhận rõ ràng, huỷ thì hoàn tác và không POST.
  const postsBeforeAuto = h.requests.filter((r) => r.method === 'POST').length;
  h.nodes.get('ci-autodelete').value = '86400';
  h.nodes.get('ci-autodelete').dispatchEvent(new Event('change'));
  assert.equal(h.nodes.get('ci-confirm-dialog').open, true);
  assert.match(h.nodes.get('ci-confirm-summary').textContent, /tin nhắn sẽ bị xoá thật ở cả hai bên và không lấy lại được/);
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, postsBeforeAuto);

  h.nodes.get('ci-confirm-cancel').click();
  assert.equal(h.nodes.get('ci-confirm-dialog').open, false);
  assert.equal(h.nodes.get('ci-autodelete').value, '0');
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, postsBeforeAuto);

  h.nodes.get('ci-autodelete').value = '86400';
  h.nodes.get('ci-autodelete').dispatchEvent(new Event('change'));
  h.nodes.get('ci-confirm-ok').click();
  await h.flush();
  const autoReq = h.requests.filter((r) => r.path === '/api/chats/42/settings').at(-1);
  assert.deepEqual(autoReq.body, { auto_delete: { seconds: 86400, confirm: true } });

  // Ghim, Lưu trữ, Đánh dấu chưa đọc.
  h.nodes.get('ci-pin').click();
  await h.flush();
  assert.equal(h.requests.at(-1).body.pin, true);
  h.nodes.get('ci-archive').click();
  await h.flush();
  assert.equal(h.requests.at(-1).body.archive, true);
  h.nodes.get('ci-unread').click();
  await h.flush();
  assert.equal(h.requests.at(-1).body.mark_unread, true);

  // Bộ nhớ đệm: hiển thị dung lượng + công tắc Mãi mãi tái dùng POST /api/settings/storage.
  assert.match(h.nodes.get('ci-cache-usage').textContent, /2 KB/);
  h.nodes.get('ci-cache-forever').checked = true;
  h.nodes.get('ci-cache-forever').dispatchEvent(new Event('change'));
  await h.flush();
  const storagePost = h.requests.filter((r) => r.method === 'POST' && r.path === '/api/settings/storage').at(-1);
  assert.deepEqual(storagePost.body.exceptions, [{ chat_id: 42, keep: null }]);
  assert.equal(storagePost.body.mode, 'local');

  // Chặn/Bỏ chặn & Thêm vào danh bạ.
  h.nodes.get('ci-block').click();
  await h.flush();
  assert.equal(h.requests.at(-1).path, '/api/do/block_user');
  assert.equal(h.requests.at(-1).body.user_id, 77);
  h.nodes.get('ci-block').click();
  await h.flush();
  assert.equal(h.requests.at(-1).path, '/api/do/unblock_user');

  h.nodes.get('ci-add-contact').click();
  await h.flush();
  assert.equal(h.requests.at(-1).path, '/api/do/add_contact');
  assert.equal(h.nodes.get('ci-add-contact').hidden, true);

  // Xoá lịch sử: bắt buộc confirm.
  const postsBeforeClear = h.requests.filter((r) => r.method === 'POST').length;
  h.nodes.get('ci-clear-history').click();
  assert.equal(h.nodes.get('ci-confirm-dialog').open, true);
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, postsBeforeClear);
  h.nodes.get('ci-confirm-cancel').click();
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, postsBeforeClear);

  h.nodes.get('ci-clear-history').click();
  h.nodes.get('ci-confirm-revoke').checked = true;
  h.nodes.get('ci-confirm-ok').click();
  await h.flush();
  const clearReq = h.requests.filter((r) => r.path === '/api/chats/42/clear_history').at(-1);
  assert.deepEqual(clearReq.body, { confirm: true, revoke: true });
});

test('ngăn Thông tin hội thoại (Nhóm/Kênh): thành viên + tìm kiếm, quyền chỉ đọc, liên kết mời chỉ cho quản trị, rời nhóm có confirm', async () => {
  const h = setupHarness({ cur: -1001, curKind: 'group' });
  await h.openChatInfo();
  await h.flush();

  assert.equal(h.nodes.get('chat-info-private').hidden, true);
  assert.equal(h.nodes.get('chat-info-group').hidden, false);
  assert.equal(h.nodes.get('ci-invite-box').hidden, false);
  assert.equal(h.nodes.get('ci-invite-link').textContent, 'https://t.me/+moi_thu');
  assert.equal(h.nodes.get('ci-my-perms').find('input').length, 0);

  const memberList = h.nodes.get('ci-members-list');
  assert.equal(memberList.find('script').length, 0);
  assert.equal(memberList.find('img').length, 0);
  assert.match(memberList.textContent, /Chủ/);
  assert.match(memberList.textContent, /Quản trị/);
  assert.match(memberList.textContent, /<script>m\(\)<\/script>/);

  h.nodes.get('ci-invite-copy').click();
  await h.flush();
  assert.deepEqual(h.clipboardWrites, ['https://t.me/+moi_thu']);

  h.nodes.get('ci-member-search').value = 'mod';
  h.nodes.get('ci-member-search').dispatchEvent(new Event('input'));
  await new Promise((r) => setTimeout(r, 260));
  const memReq = h.requests.filter((r) => r.path.startsWith('/api/chats/-1001/members')).at(-1);
  assert.match(memReq.path, /q=mod/);
  assert.match(memReq.path, /limit=50/);

  // Rời nhóm: bắt buộc confirm.
  const postsBeforeLeave = h.requests.filter((r) => r.method === 'POST').length;
  h.nodes.get('ci-leave').click();
  assert.equal(h.nodes.get('ci-confirm-dialog').open, true);
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, postsBeforeLeave);
  h.nodes.get('ci-confirm-cancel').click();
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, postsBeforeLeave);

  h.nodes.get('ci-leave').click();
  h.nodes.get('ci-confirm-ok').click();
  await h.flush();
  assert.deepEqual(h.requests.filter((r) => r.path === '/api/chats/-1001/leave').at(-1).body, { confirm: true });

  // Khi không phải quản trị: phải ẩn liên kết mời kể cả khi đối tượng có trường invite_link.
  const hMember = setupHarness({
    cur: -1001,
    curKind: 'group',
    infoMap: {
      [-1001]: {
        chat_id: -1001, title: 'Nhóm thường', kind: 'group', member_count: 5,
        can_get_members: true, my_role: 'member', is_admin: false,
        my_permissions: { can_send_basic_messages: true },
        invite_link: 'https://t.me/+bi_mat',
        notification: { use_default_mute_for: true, mute_for: 0, use_default_show_preview: true, show_preview: true },
        auto_delete_seconds: 0, pinned: false, archived: false, marked_unread: false,
      },
    },
  });
  await hMember.openChatInfo();
  await hMember.flush();
  assert.equal(hMember.nodes.get('ci-invite-box').hidden, true);
  assert.equal(hMember.nodes.get('ci-invite-link').textContent, '');
});

test('đột biến 1: bỏ chốt xác nhận tự xoá tin nhắn làm test bảo vệ thất bại', async () => {
  const h = setupHarness({
    mutateChatInfo: (code) => {
      assert.ok(code.includes('ciConfirmAutoDelete(seconds, prev)'), 'Không tìm thấy ciConfirmAutoDelete');
      return code.replace(
        'ciConfirmAutoDelete(seconds, prev)',
        'saveChatSettings({ auto_delete: { seconds, confirm: true } })',
      );
    },
  });
  await h.openChatInfo();
  await h.flush();
  h.nodes.get('ci-autodelete').value = '86400';
  h.nodes.get('ci-autodelete').dispatchEvent(new Event('change'));
  await h.flush();
  assert.equal(h.nodes.get('ci-confirm-dialog').open, false);
  assert.equal(h.requests.filter((r) => r.path === '/api/chats/42/settings').length, 1);
});

test('đột biến 2: bỏ chốt xác nhận xoá lịch sử làm test bảo vệ thất bại', async () => {
  const h = setupHarness({
    mutateChatInfo: (code) => {
      assert.ok(code.includes('ciConfirmClearHistory()'), 'Không tìm thấy ciConfirmClearHistory');
      return code.replace(
        'ciConfirmClearHistory()',
        'api("POST", "/api/chats/" + chatInfo.chat_id + "/clear_history", { confirm: true, revoke: false })',
      );
    },
  });
  await h.openChatInfo();
  await h.flush();
  h.nodes.get('ci-clear-history').click();
  await h.flush();
  assert.equal(h.nodes.get('ci-confirm-dialog').open, false);
  assert.equal(h.requests.filter((r) => r.path === '/api/chats/42/clear_history').length, 1);
});

test('đột biến 3: bỏ chốt xác nhận rời nhóm làm test bảo vệ thất bại', async () => {
  const h = setupHarness({
    cur: -1001,
    curKind: 'group',
    mutateChatInfo: (code) => {
      assert.ok(code.includes('ciConfirmLeaveChat()'), 'Không tìm thấy ciConfirmLeaveChat');
      return code.replace(
        'ciConfirmLeaveChat()',
        'api("POST", "/api/chats/" + chatInfo.chat_id + "/leave", { confirm: true })',
      );
    },
  });
  await h.openChatInfo();
  await h.flush();
  h.nodes.get('ci-leave').click();
  await h.flush();
  assert.equal(h.nodes.get('ci-confirm-dialog').open, false);
  assert.equal(h.requests.filter((r) => r.path === '/api/chats/-1001/leave').length, 1);
});

test('đột biến 4: bỏ kiểm tra is_admin trước khi hiển thị liên kết mời làm lộ liên kết', async () => {
  const h = setupHarness({
    cur: -1001,
    curKind: 'group',
    infoMap: {
      [-1001]: {
        chat_id: -1001, title: 'Nhóm thường', kind: 'group', member_count: 5,
        can_get_members: true, my_role: 'member', is_admin: false,
        my_permissions: {}, invite_link: 'https://t.me/+bi_mat',
        notification: { use_default_mute_for: true, mute_for: 0, use_default_show_preview: true, show_preview: true },
        auto_delete_seconds: 0, pinned: false, archived: false, marked_unread: false,
      },
    },
    mutateChatInfo: (code) => {
      assert.ok(code.includes('Boolean(info.is_admin && info.invite_link)'), 'Không tìm thấy điều kiện is_admin cho invite_link');
      return code.replace('Boolean(info.is_admin && info.invite_link)', 'Boolean(info.invite_link)');
    },
  });
  await h.openChatInfo();
  await h.flush();
  assert.equal(h.nodes.get('ci-invite-box').hidden, false);
});
