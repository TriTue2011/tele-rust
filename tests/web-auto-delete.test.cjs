const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const vm = require('node:vm');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const fullScript = /<script>([\s\S]*?)<\/script>/.exec(html)[1];
const listMarker = '  // ---- danh sách hội thoại';
const messagesMarker = '  // ---- tin nhắn';
const openChatMarker = '  async function openChat(c)';
const loadOlderMarker = '  async function loadOlder()';
const feedMarker = '  // ---- cập nhật theo thời gian thực';
const searchMarker = '  // ---- tìm kiếm, tab';
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
    this.parentNode = null;
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
    this.tabIndex = 0;
    this.title = '';
    this.scrollTop = 0;
    this.scrollHeight = 0;
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
  append(...nodes) {
    for (const n of nodes) {
      if (n instanceof MockNode) n.parentNode = this;
      this.children.push(n);
    }
  }
  appendChild(node) {
    if (node instanceof MockNode) node.parentNode = this;
    this.children.push(node);
    return node;
  }
  insertBefore(node, ref) {
    if (node instanceof MockNode) node.parentNode = this;
    const idx = this.children.indexOf(ref);
    if (idx === -1) this.children.push(node);
    else this.children.splice(idx, 0, node);
    return node;
  }
  remove() {
    if (!this.parentNode) return;
    this.parentNode.children = this.parentNode.children.filter((c) => c !== this);
    this.parentNode = null;
  }
  replaceChildren(...nodes) {
    for (const n of nodes) {
      if (n instanceof MockNode) n.parentNode = this;
    }
    this.children = nodes;
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
    return this.children.flatMap((c) => (c instanceof MockNode ? [...(c.tag === tag ? [c] : []), ...c.find(tag)] : []));
  }
  findByClass(cls) {
    return this.children.flatMap((c) => {
      if (!(c instanceof MockNode)) return [];
      const has = String(c.className || '').split(/\s+/).includes(cls);
      return [...(has ? [c] : []), ...c.findByClass(cls)];
    });
  }
}

function defaultStoragePolicy(exceptions = []) {
  return {
    mode: 'local',
    keep: { private: 604800, groups: 604800, channels: null },
    exceptions,
    max_cache_bytes: 2 * 1024 * 1024 * 1024,
    stream_ttl_s: 600,
    auto_download: Object.fromEntries(['private', 'groups', 'channels'].map((chat) => [
      chat,
      Object.fromEntries(['photo', 'video', 'file', 'voice', 'video_note', 'gif'].map((k) => [k, { on: false, max_bytes: 10485760 }])),
    ])),
  };
}

function makeSelectWithOptions(values) {
  const sel = new MockNode('select');
  for (const [val, label] of values) {
    const opt = new MockNode('option', '', label);
    opt.value = val;
    sel.appendChild(opt);
  }
  sel.value = values[0]?.[0] ?? '';
  return sel;
}

function setupAutoDeleteHarness(options = {}) {
  const listStart = html.indexOf(listMarker);
  const messagesStart = html.indexOf(messagesMarker);
  const openChatStart = html.indexOf(openChatMarker);
  const loadOlderStart = html.indexOf(loadOlderMarker);
  const feedStart = html.indexOf(feedMarker);
  const searchStart = html.indexOf(searchMarker);
  const settingsStart = html.indexOf(settingsMarker);
  const storageStart = html.indexOf(storageMarker);
  const chatInfoStart = html.indexOf(chatInfoMarker);
  const startPos = html.indexOf(startMarker, chatInfoStart > -1 ? chatInfoStart : 0);

  assert.ok(listStart > -1 && messagesStart > listStart);
  assert.ok(openChatStart > -1 && loadOlderStart > openChatStart);
  assert.ok(feedStart > -1 && searchStart > feedStart);
  assert.ok(settingsStart > -1 && storageStart > settingsStart);
  assert.ok(chatInfoStart > -1 && startPos > chatInfoStart);

  let listCode = html.slice(listStart, messagesStart);
  let openChatCode = html.slice(openChatStart, loadOlderStart);
  let feedCode = html.slice(feedStart, searchStart);
  let settingsCode = html.slice(settingsStart, storageStart);
  let chatInfoCode = html.slice(chatInfoStart, startPos);

  if (options.mutateFeed) feedCode = options.mutateFeed(feedCode);
  if (options.mutateChatInfo) chatInfoCode = options.mutateChatInfo(chatInfoCode);

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
    title: 'Telegram',
    createTextNode: (t) => String(t),
    querySelectorAll: (selector) => (selector === '#settings-dialog .settings-item' ? navItems : []),
  });

  const chats = options.chats || [
    { id: 42, title: '<script>alert(1)</script> Đại ca', kind: 'private', user_id: 77, pinned: false, muted: false, auto_delete_seconds: 86400 },
    { id: 99, title: 'VPhong', kind: 'private', user_id: 88, pinned: false, muted: false, auto_delete_seconds: 0 },
  ];
  const infoMap = options.infoMap || {
    42: {
      chat_id: 42,
      title: '<script>alert(1)</script> Đại ca',
      kind: 'private',
      username: 'dai_ca',
      bio: 'Thử nghiệm tự xoá',
      phone_masked: '+849•••••007',
      user_id: 77,
      first_name: 'Đại',
      last_name: 'Ca',
      status: 'online',
      is_contact: true,
      is_blocked: false,
      is_bot: false,
      avatar_file: null,
      notification: { use_default_mute_for: true, mute_for: 0, use_default_show_preview: true, show_preview: true },
      auto_delete_seconds: 86400,
      pinned: false,
      archived: false,
      marked_unread: false,
      unread: 0,
      has_protected_content: false,
    },
  };

  let serverStorage = defaultStoragePolicy(options.exceptions || []);
  const requests = [];
  const toasts = [];

  const getNode = (id) => {
    if (!nodes.has(id)) {
      if (id === 'ci-autodelete') {
        nodes.set(id, makeSelectWithOptions([
          ['0', 'Tắt'],
          ['86400', '1 ngày'],
          ['604800', '1 tuần'],
          ['2678400', '1 tháng'],
          ['custom', 'Tùy chỉnh…'],
        ]));
      } else if (id === 'ci-cache-keep') {
        nodes.set(id, makeSelectWithOptions([
          ['default', 'Theo mặc định loại hội thoại'],
          ['86400', '1 ngày'],
          ['604800', '7 ngày'],
          ['2678400', '1 tháng'],
          ['forever', 'Mãi mãi (không dọn)'],
        ]));
      } else if (id === 'ci-mute') {
        nodes.set(id, makeSelectWithOptions([
          ['default', 'Dùng mặc định'],
          ['unmuted', 'Bật thông báo'],
          ['3600', 'Tắt tiếng 1 giờ'],
        ]));
      } else {
        const tag = id === 'account' || id.endsWith('-font') || id.endsWith('-family') ? 'select' : 'div';
        nodes.set(id, new MockNode(tag));
      }
    }
    return nodes.get(id);
  };

  const ctx = {
    $: getNode,
    el: (tag, cls, text) => new MockNode(tag, cls, text),
    document: doc,
    window: new EventTarget(),
    COLORS: ['#e17076', '#7bc862'],
    account: '+84900000000',
    initials: (s) => (s || '?')[0].toUpperCase(),
    avatar: (title) => new MockNode('div', 'av', (title || '?')[0].toUpperCase()),
    listTime: () => '12:00',
    chats,
    cur: options.cur ?? null,
    curKind: options.curKind ?? '',
    replyTo: 0,
    tab: 'chats',
    oldest: 0,
    lastDay: null,
    rendered: new Map(),
    msgData: new Map(),
    listTimer: null,
    typingTimer: null,
    unreadTotal: 0,
    toast: (m) => toasts.push(m),
    appendMessages: () => {},
    markRead: () => {},
    updateComposePreview: () => {},
    reloadCurrent: async () => {},
    openContact: () => {},
    fetch: async () => ({ ok: true, json: async () => ({ items: [], cursor: 0, reset: false }) }),
    AbortController,
    accountPath: (p) => p,
    showGate: () => {},
    api: async (method, path, body) => {
      requests.push({ method, path, body: body === undefined ? undefined : structuredClone(body) });
      const u = new URL(path, 'http://localhost');
      if (method === 'GET' && u.pathname === '/api/chats') {
        return { chats: structuredClone(chats) };
      }
      if (method === 'GET' && /^\/api\/chats\/(-?\d+)\/messages$/.test(u.pathname)) {
        return { messages: [] };
      }
      if (method === 'GET' && u.pathname === '/api/storage/usage') {
        return { total_bytes: 2048, by_type: {}, top_chats: [{ chat_id: 42, bytes: 1024 }] };
      }
      if (method === 'GET' && u.pathname === '/api/settings/storage') {
        return structuredClone(serverStorage);
      }
      if (method === 'POST' && u.pathname === '/api/settings/storage') {
        serverStorage = structuredClone(body);
        return structuredClone(serverStorage);
      }
      const infoMatch = /^\/api\/chats\/(-?\d+)\/info$/.exec(u.pathname);
      if (method === 'GET' && infoMatch) {
        return structuredClone(infoMap[Number(infoMatch[1])]);
      }
      const settingsMatch = /^\/api\/chats\/(-?\d+)\/settings$/.exec(u.pathname);
      if (method === 'POST' && settingsMatch) {
        const cid = Number(settingsMatch[1]);
        if (body.auto_delete && infoMap[cid]) {
          infoMap[cid].auto_delete_seconds = body.auto_delete.seconds;
        }
        return { ok: true, chat_id: cid, ...(body.auto_delete ? { auto_delete_seconds: body.auto_delete.seconds } : {}) };
      }
      if (method === 'GET' && u.pathname === '/api/settings/appearance') {
        return { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' };
      }
      if (method === 'GET' && u.pathname === '/api/profile') {
        return { id: 7, name: 'An', first_name: 'An', last_name: '', bio: '', username: '', phone_masked: '' };
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
    'app', 'gate', 'placeholder', 'head', 'title', 'head-autodelete', 'msgs', 'typing', 'reply', 'composer', 'text',
    'call-a', 'call-v', 'q', 'list', 'font-open', 'chat-info-open',
    'font-popover', 'font-pop-close', 'font-pop-family', 'font-pop-size', 'font-pop-size-dec',
    'font-pop-size-inc', 'font-pop-size-val', 'font-pop-lh-compact', 'font-pop-lh-normal',
    'font-pop-lh-relaxed', 'font-pop-status', 'font-pop-error', 'font-pop-reset',
    'chat-info-drawer', 'chat-info-close', 'chat-info-status', 'chat-info-error',
    'chat-info-avatar', 'chat-info-title', 'chat-info-kind', 'chat-info-username',
    'chat-info-members-count', 'chat-info-sub', 'chat-info-bio',
    'ci-mute', 'ci-preview', 'ci-autodelete', 'ci-autodelete-current', 'ci-autodelete-custom-row',
    'ci-autodelete-days', 'ci-autodelete-apply', 'ci-autodelete-custom-cancel', 'ci-autodelete-warning',
    'ci-pin', 'ci-archive', 'ci-unread',
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
    getNode(id);
  }
  for (const hiddenId of [
    'font-popover', 'chat-info-drawer', 'ci-invite-box', 'ci-add-contact',
    'chat-info-private', 'chat-info-group', 'ci-confirm-revoke-wrap',
    'head-autodelete', 'ci-autodelete-custom-row', 'ci-autodelete-warning',
  ]) {
    getNode(hiddenId).hidden = true;
  }
  getNode('account').append(Object.assign(new MockNode('option', '', 'An · online'), { value: '+84900000000' }));
  getNode('account').value = '+84900000000';

  vm.runInNewContext(
    `${listCode}\n${openChatCode}\n${feedCode}\n${settingsCode}\n${chatInfoCode}\nthis.exports = { renderList, loadChats, openChat, onFeed, openChatInfo, closeChatInfo, formatAutoDeleteDuration, formatAutoDeleteLabel, formatAutoDeleteShort, chats, getCur: () => cur };`,
    ctx,
  );
  const flush = () => new Promise((r) => setTimeout(r, 15));
  return { ...ctx.exports, nodes, requests, toasts, flush };
}

test('HTML sạch bảo mật và có chú thích cố định + phần tử đồng bộ tự xoá', () => {
  new vm.Script(fullScript);
  for (const banned of ['innerHTML', 'outerHTML', 'insertAdjacentHTML', 'localStorage', 'sessionStorage', 'gw_token', 'GATEWAY_TOKEN', '@font-face', 'fonts.googleapis', 'fonts.gstatic']) {
    assert.equal(html.includes(banned), false, `không được chứa ${banned}`);
  }
  assert.match(html, /id="head-autodelete"/);
  assert.match(html, /id="ci-autodelete-current"/);
  assert.match(html, /id="ci-autodelete-warning"/);
  assert.match(html, /id="ci-autodelete-note"/);
  assert.ok(
    html.includes('Tự xoá tin nhắn là cài đặt của Telegram, áp cho TIN MỚI GỬI, xoá thật ở cả hai bên; khác với ngoại lệ «Mãi mãi» của bộ nhớ đệm (chỉ giữ tệp đã tải về máy chủ, không giữ tin nhắn)'),
    'phải chứa đúng dòng chú thích cố định về Tự xoá và Mãi mãi',
  );
});

test('định dạng thời hạn tự xoá từ Telegram và nhận sự kiện chat_auto_delete cập nhật tức thì mọi chỗ', async () => {
  const h = setupAutoDeleteHarness();
  assert.equal(h.formatAutoDeleteLabel(0), 'Tắt');
  assert.equal(h.formatAutoDeleteLabel(86400), '1 ngày');
  assert.equal(h.formatAutoDeleteLabel(604800), '1 tuần');
  assert.equal(h.formatAutoDeleteLabel(2678400), '1 tháng');
  assert.equal(h.formatAutoDeleteLabel(172800), 'Tùy chỉnh 2 ngày');
  assert.equal(h.formatAutoDeleteLabel(90000), 'Tùy chỉnh 1 ngày 1 giờ');
  assert.equal(h.formatAutoDeleteLabel(3600), 'Tùy chỉnh 1 giờ');
  assert.equal(h.formatAutoDeleteShort(86400), '1ng');
  assert.equal(h.formatAutoDeleteShort(604800), '1t');
  assert.equal(h.formatAutoDeleteShort(2678400), '1th');
  assert.equal(h.formatAutoDeleteShort(90000), '1ng');
  assert.equal(h.formatAutoDeleteShort(3600), '1g');

  await h.loadChats();
  let listBadges = h.nodes.get('list').findByClass('ad-badge');
  assert.equal(listBadges.length, 1);
  assert.match(listBadges[0].textContent, /1ng/);
  assert.equal(h.nodes.get('list').find('script').length, 0);

  // Mở hội thoại 42 -> tiêu đề hiện biểu tượng đồng hồ #head-autodelete; mở ngăn Thông tin -> Hiện tại: 1 ngày.
  await h.openChat(h.chats[0]);
  await h.flush();
  assert.equal(h.nodes.get('head-autodelete').hidden, false);
  assert.match(h.nodes.get('head-autodelete').textContent, /1ng/);
  await h.openChatInfo();
  await h.flush();
  assert.equal(h.nodes.get('ci-autodelete-current').textContent, 'Hiện tại (Telegram): 1 ngày');
  assert.equal(h.nodes.get('ci-autodelete').value, '86400');

  // Sự kiện của tài khoản khác bị bỏ qua.
  h.onFeed({ type: 'chat_auto_delete', account: '+84999999999', chat_id: 42, seconds: 0 });
  assert.equal(h.nodes.get('ci-autodelete-current').textContent, 'Hiện tại (Telegram): 1 ngày');
  assert.equal(h.nodes.get('head-autodelete').hidden, false);

  // Nhận sự kiện từ Telegram (đổi trên điện thoại sang Tùy chỉnh 1 ngày 1 giờ = 90000 giây) -> đổi ngay không cần reload.
  h.onFeed({ type: 'chat_auto_delete', account: '+84900000000', chat_id: 42, seconds: 90000 });
  assert.equal(h.nodes.get('ci-autodelete-current').textContent, 'Hiện tại (Telegram): Tùy chỉnh 1 ngày 1 giờ');
  assert.equal(h.nodes.get('ci-autodelete').value, '90000');
  assert.equal(h.nodes.get('head-autodelete').hidden, false);
  assert.match(h.nodes.get('head-autodelete').textContent, /1ng/);

  // Sự kiện bật tự xoá cho hội thoại 99 -> danh sách hiện thêm đồng hồ cho 99.
  h.onFeed({ type: 'chat_auto_delete', chat_id: 99, seconds: 604800 });
  listBadges = h.nodes.get('list').findByClass('ad-badge');
  assert.equal(listBadges.length, 2);
  assert.match(listBadges[1].textContent, /1t/);

  // Nhận sự kiện tắt (0) cho hội thoại 42 -> ẩn đồng hồ ở tiêu đề, bỏ ở danh sách cho 42, ngăn Thông tin chuyển Tắt.
  h.onFeed({ type: 'chat_auto_delete', chat_id: 42, seconds: 0 });
  assert.equal(h.nodes.get('head-autodelete').hidden, true);
  assert.equal(h.nodes.get('ci-autodelete-current').textContent, 'Hiện tại (Telegram): Tắt');
  assert.equal(h.nodes.get('ci-autodelete').value, '0');
  assert.equal(h.nodes.get('list').findByClass('ad-badge').length, 1);
});

test('cảnh báo vàng khi vừa tự xoá vừa có ngoại lệ Mãi mãi và cập nhật tức thì theo sự kiện hoặc khi đổi công tắc', async () => {
  const h = setupAutoDeleteHarness({ exceptions: [{ chat_id: 42, keep: null }] });
  await h.openChat(h.chats[0]);
  await h.openChatInfo();
  await h.flush();

  const warn = h.nodes.get('ci-autodelete-warning');
  assert.equal(warn.hidden, false);
  assert.equal(
    warn.textContent,
    'Tin nhắn vẫn sẽ bị Telegram xoá sau 1 ngày; Mãi mãi chỉ giữ tệp đã tải',
  );

  // Đổi thời hạn tự xoá từ sự kiện Telegram -> nội dung cảnh báo vàng tự cập nhật.
  h.onFeed({ type: 'chat_auto_delete', chat_id: 42, seconds: 604800 });
  assert.equal(warn.hidden, false);
  assert.equal(
    warn.textContent,
    'Tin nhắn vẫn sẽ bị Telegram xoá sau 1 tuần; Mãi mãi chỉ giữ tệp đã tải',
  );

  // Tắt ngoại lệ Mãi mãi -> mở hộp xác nhận, đồng ý thì mới lưu và ẩn cảnh báo vàng.
  h.nodes.get('ci-cache-forever').checked = false;
  h.nodes.get('ci-cache-forever').dispatchEvent(new Event('change'));
  assert.equal(h.nodes.get('ci-confirm-dialog').open, true);
  h.nodes.get('ci-confirm-ok').click();
  await h.flush();
  assert.equal(warn.hidden, true);

  // Bật lại Mãi mãi -> hiện lại cảnh báo vàng.
  h.nodes.get('ci-cache-forever').checked = true;
  h.nodes.get('ci-cache-forever').dispatchEvent(new Event('change'));
  await h.flush();
  assert.equal(warn.hidden, false);

  // Nhận sự kiện Telegram tắt tự xoá -> ẩn cảnh báo vàng dù vẫn có ngoại lệ Mãi mãi.
  h.onFeed({ type: 'chat_auto_delete', chat_id: 42, seconds: 0 });
  assert.equal(warn.hidden, true);
});

test('đặt tuỳ chỉnh trong giới hạn Telegram (1..365 ngày) bắt buộc confirm và cập nhật UI', async () => {
  const h = setupAutoDeleteHarness();
  await h.openChat(h.chats[0]);
  await h.openChatInfo();
  await h.flush();

  // Chọn Tùy chỉnh... -> hiện hàng nhập ngày, chưa mở confirm hay gọi POST.
  const postsBefore = h.requests.filter((r) => r.method === 'POST').length;
  h.nodes.get('ci-autodelete').value = 'custom';
  h.nodes.get('ci-autodelete').dispatchEvent(new Event('change'));
  assert.equal(h.nodes.get('ci-autodelete-custom-row').hidden, false);
  assert.equal(h.nodes.get('ci-confirm-dialog').open, false);
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, postsBefore);

  // Nhập ngoài giới hạn Telegram (0 và 366 ngày) -> báo lỗi, không mở confirm, không gọi POST.
  for (const badDays of ['0', '366', '1.5', '-1']) {
    h.nodes.get('ci-autodelete-days').value = badDays;
    h.nodes.get('ci-autodelete-apply').click();
    assert.equal(h.nodes.get('ci-confirm-dialog').open, false);
    assert.notEqual(h.nodes.get('chat-info-error').textContent, '');
    assert.equal(h.requests.filter((r) => r.method === 'POST').length, postsBefore);
  }

  // Nhập 3 ngày hợp lệ -> bắt buộc mở hộp xác nhận, huỷ không POST, đồng ý mới POST với confirm: true.
  h.nodes.get('ci-autodelete-days').value = '3';
  h.nodes.get('ci-autodelete-apply').click();
  assert.equal(h.nodes.get('ci-confirm-dialog').open, true);
  assert.match(h.nodes.get('ci-confirm-summary').textContent, /Tùy chỉnh 3 ngày/);
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, postsBefore);

  h.nodes.get('ci-confirm-cancel').click();
  assert.equal(h.nodes.get('ci-confirm-dialog').open, false);
  assert.equal(h.nodes.get('ci-autodelete').value, '86400');
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, postsBefore);

  h.nodes.get('ci-autodelete').value = 'custom';
  h.nodes.get('ci-autodelete').dispatchEvent(new Event('change'));
  h.nodes.get('ci-autodelete-days').value = '3';
  h.nodes.get('ci-autodelete-apply').click();
  h.nodes.get('ci-confirm-ok').click();
  await h.flush();

  const req = h.requests.filter((r) => r.method === 'POST' && r.path === '/api/chats/42/settings').at(-1);
  assert.deepEqual(req.body, { auto_delete: { seconds: 259200, confirm: true } });
  assert.equal(h.nodes.get('ci-autodelete-custom-row').hidden, true);
  assert.equal(h.nodes.get('ci-autodelete-current').textContent, 'Hiện tại (Telegram): Tùy chỉnh 3 ngày');
  assert.match(h.nodes.get('head-autodelete').textContent, /3ng/);
});

test('đột biến 1: bỏ xử lý sự kiện chat_auto_delete trong onFeed khiến UI không đổi theo sự kiện', async () => {
  const h = setupAutoDeleteHarness({
    mutateFeed: (code) => {
      assert.ok(code.includes('chat_auto_delete'), 'Không tìm thấy nhánh chat_auto_delete trong onFeed');
      return code.replace('it.type === "chat_auto_delete"', 'false');
    },
  });
  await h.openChat(h.chats[0]);
  await h.openChatInfo();
  await h.flush();
  h.onFeed({ type: 'chat_auto_delete', chat_id: 42, seconds: 604800 });
  assert.equal(h.nodes.get('ci-autodelete-current').textContent, 'Hiện tại (Telegram): 1 ngày');
});

test('đột biến 2: bỏ kiểm tra Mãi mãi + tự xoá cùng lúc khiến cảnh báo vàng hiển thị sai', async () => {
  const h = setupAutoDeleteHarness({
    exceptions: [{ chat_id: 42, keep: null }],
    mutateChatInfo: (code) => {
      assert.ok(code.includes('if (sec > 0 && forever)'), 'Không tìm thấy điều kiện sec > 0 && forever');
      return code.replace('if (sec > 0 && forever)', 'if (false)');
    },
  });
  await h.openChat(h.chats[0]);
  await h.openChatInfo();
  await h.flush();
  assert.equal(h.nodes.get('ci-autodelete-warning').hidden, true);
});

test('đột biến 3: bỏ kiểm tra giới hạn 1..365 ngày cho phép gửi số ngày ngoài giới hạn Telegram', async () => {
  const h = setupAutoDeleteHarness({
    mutateChatInfo: (code) => {
      assert.ok(code.includes('days < 1 || days > AUTO_DELETE_MAX_DAYS'), 'Không tìm thấy chốt giới hạn ngày Telegram');
      return code.replace('days < 1 || days > AUTO_DELETE_MAX_DAYS', 'false');
    },
  });
  await h.openChat(h.chats[0]);
  await h.openChatInfo();
  await h.flush();
  h.nodes.get('ci-autodelete').value = 'custom';
  h.nodes.get('ci-autodelete').dispatchEvent(new Event('change'));
  h.nodes.get('ci-autodelete-days').value = '366';
  h.nodes.get('ci-autodelete-apply').click();
  assert.equal(h.nodes.get('ci-confirm-dialog').open, true);
});
