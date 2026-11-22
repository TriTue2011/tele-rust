const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const vm = require('node:vm');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const fullScript = /<script>([\s\S]*?)<\/script>/.exec(html)[1];
const utilsMarker = '  // ---- tiện ích';
const listMarker = '  // ---- danh sách hội thoại';
const messagesMarker = '  // ---- tin nhắn';
const urlReMarker = '  const URL_RE';
const lastDayMarker = '  let lastDay';
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
    this.scrolledIntoView = false;
    this.tabIndex = 0;
    this.title = '';
    this.scrollTop = 0;
    this.scrollHeight = 0;
    this.clientHeight = 500;
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
  getBoundingClientRect() {
    return { left: 100, top: 80, right: 200, bottom: 120, width: 100, height: 40 };
  }
  scrollIntoView() { this.scrolledIntoView = true; }
  showModal() { this.open = true; }
  close() { this.open = false; this.dispatchEvent(new Event('close')); }
  focus() { this.focused = true; }
  click() { this.dispatchEvent(new Event('click')); }
  hasClass(cls) {
    return String(this.className || '').split(/\s+/).includes(cls) || this.classList.contains(cls);
  }
  find(tag) {
    return this.children.flatMap((c) => (c instanceof MockNode ? [...(c.tag === tag ? [c] : []), ...c.find(tag)] : []));
  }
  findByClass(cls) {
    return this.children.flatMap((c) => {
      if (!(c instanceof MockNode)) return [];
      return [...(c.hasClass(cls) ? [c] : []), ...c.findByClass(cls)];
    });
  }
  querySelector(selector) {
    if (selector.startsWith('.')) return this.findByClass(selector.slice(1))[0] || null;
    if (selector.startsWith('#')) {
      return this.children.flatMap((c) => (c instanceof MockNode ? [...(c.id === selector.slice(1) ? [c] : []), ...(c.querySelector(selector) ? [c.querySelector(selector)] : [])] : []))[0] || null;
    }
    return this.find(selector)[0] || null;
  }
  querySelectorAll(selector) {
    if (selector.startsWith('.')) return this.findByClass(selector.slice(1));
    return this.find(selector);
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

function setupMacosHarness(options = {}) {
  const utilsStart = html.indexOf(utilsMarker);
  const listStart = html.indexOf(listMarker);
  const messagesStart = html.indexOf(messagesMarker);
  const urlReStart = html.indexOf(urlReMarker);
  const lastDayStart = html.indexOf(lastDayMarker);
  const openChatStart = html.indexOf(openChatMarker);
  const loadOlderStart = html.indexOf(loadOlderMarker);
  const feedStart = html.indexOf(feedMarker);
  const searchStart = html.indexOf(searchMarker);
  const settingsStart = html.indexOf(settingsMarker);
  const storageStart = html.indexOf(storageMarker);
  const chatInfoStart = html.indexOf(chatInfoMarker);
  const startPos = html.indexOf(startMarker, chatInfoStart > -1 ? chatInfoStart : 0);

  assert.ok(utilsStart > -1 && listStart > utilsStart, 'Thiếu mốc tiện ích / danh sách');
  assert.ok(messagesStart > listStart && urlReStart >= messagesStart && lastDayStart > urlReStart, 'Thiếu mốc tin nhắn');
  assert.ok(openChatStart > lastDayStart && loadOlderStart > openChatStart, 'Thiếu mốc openChat');
  assert.ok(feedStart > loadOlderStart && searchStart > feedStart, 'Thiếu mốc feed');
  assert.ok(settingsStart > searchStart && storageStart > settingsStart, 'Thiếu mốc settings');
  assert.ok(chatInfoStart > storageStart && startPos > chatInfoStart, 'Thiếu mốc chatInfo');

  let utilsCode = html.slice(utilsStart, listStart);
  let listCode = html.slice(listStart, messagesStart);
  let rendererCode = html.slice(urlReStart, lastDayStart);
  let openChatCode = html.slice(openChatStart, loadOlderStart);
  let feedCode = html.slice(feedStart, searchStart);
  const settingsCode = html.slice(settingsStart, storageStart);
  let chatInfoCode = html.slice(chatInfoStart, startPos);

  if (options.mutateUtils) utilsCode = options.mutateUtils(utilsCode);
  if (options.mutateList) listCode = options.mutateList(listCode);
  if (options.mutateRenderer) rendererCode = options.mutateRenderer(rendererCode);
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

  const getNode = (id) => {
    if (!nodes.has(id)) {
      let node;
      if (id === 'ci-autodelete') {
        node = makeSelectWithOptions([['0', 'Tắt'], ['86400', '1 ngày'], ['604800', '1 tuần'], ['2678400', '1 tháng'], ['custom', 'Tùy chỉnh…']]);
      } else if (id === 'ci-cache-keep') {
        node = makeSelectWithOptions([['default', 'Theo mặc định loại hội thoại'], ['86400', '1 ngày'], ['604800', '7 ngày'], ['2678400', '1 tháng'], ['forever', 'Mãi mãi (không dọn)']]);
      } else if (id === 'ci-mute') {
        node = makeSelectWithOptions([['default', 'Dùng mặc định'], ['unmuted', 'Bật thông báo'], ['3600', 'Tắt tiếng 1 giờ'], ['2147483647', 'Tắt tiếng mãi mãi']]);
      } else {
        const tag = id === 'account' || id.endsWith('-font') || id.endsWith('-family') ? 'select' : 'div';
        node = new MockNode(tag);
      }
      node.id = id;
      nodes.set(id, node);
    }
    return nodes.get(id);
  };

  const doc = Object.assign(new EventTarget(), {
    documentElement: root,
    title: 'Telegram',
    createElement: (tag) => new MockNode(tag),
    createTextNode: (t) => String(t),
    createDocumentFragment: () => new MockNode('fragment'),
    querySelectorAll: (selector) => (selector === '#settings-dialog .settings-item' ? navItems : []),
  });

  const chats = options.chats || [];
  const messagesMap = options.messagesMap || {};
  const infoMap = options.infoMap || {};
  const searchResultsMap = options.searchResultsMap || {};
  const requests = [];
  const toasts = [];
  const startedCalls = [];

  const mockApi = async (method, path, body) => {
    requests.push({ method, path, body: body === undefined ? undefined : structuredClone(body) });
    const u = new URL(path, 'http://localhost');
    if (method === 'GET' && u.pathname === '/api/chats') {
      return { chats: structuredClone(chats) };
    }
    const msgMatch = /^\/api\/chats\/(-?\d+)\/messages$/.exec(u.pathname);
    if (method === 'GET' && msgMatch) {
      const cid = Number(msgMatch[1]);
      const q = (u.searchParams.get('q') || '').trim().toLowerCase();
      if (q) {
        const list = searchResultsMap[cid] || (messagesMap[cid] || []).filter((m) => (m.text || m.summary || '').toLowerCase().includes(q));
        return { messages: structuredClone(list), total_count: list.length };
      }
      return { messages: structuredClone(messagesMap[cid] || []) };
    }
    const infoMatch = /^\/api\/chats\/(-?\d+)\/info$/.exec(u.pathname);
    if (method === 'GET' && infoMatch) {
      const cid = Number(infoMatch[1]);
      return structuredClone(infoMap[cid] || { chat_id: cid, title: 'Hội thoại', kind: 'private', notification: {}, auto_delete_seconds: 0 });
    }
    const membersMatch = /^\/api\/chats\/(-?\d+)\/members$/.exec(u.pathname);
    if (method === 'GET' && membersMatch) {
      return { chat_id: Number(membersMatch[1]), total_count: 0, members: [] };
    }
    if (method === 'GET' && u.pathname === '/api/storage/usage') {
      return { total_bytes: 0, by_type: {}, top_chats: [] };
    }
    if (method === 'GET' && u.pathname === '/api/settings/storage') {
      return defaultStoragePolicy();
    }
    if (method === 'GET' && u.pathname === '/api/settings/appearance') {
      return { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' };
    }
    if (method === 'GET' && u.pathname === '/api/profile') {
      return { id: 7, name: 'An', first_name: 'An', last_name: '', bio: '', username: '', phone_masked: '' };
    }
    const settingsMatch = /^\/api\/chats\/(-?\d+)\/settings$/.exec(u.pathname);
    if (method === 'POST' && settingsMatch) {
      const cid = Number(settingsMatch[1]);
      const chat = chats.find((c) => c.id === cid);
      if (chat) {
        if (body.notification) {
          chat.muted = Number(body.notification.mute_for || 0) > 0;
        }
        if (body.pin !== undefined) chat.pinned = Boolean(body.pin);
        if (body.archive !== undefined) chat.archived = Boolean(body.archive);
        if (body.mark_unread !== undefined) chat.marked_unread = Boolean(body.mark_unread);
      }
      return {
        ok: true,
        chat_id: cid,
        ...(body.pin !== undefined ? { pinned: body.pin } : {}),
        ...(body.archive !== undefined ? { archived: body.archive } : {}),
        ...(body.mark_unread !== undefined ? { marked_unread: body.mark_unread } : {}),
        ...(body.notification ? { notification: { ...body.notification } } : {}),
      };
    }
    const clearMatch = /^\/api\/chats\/(-?\d+)\/clear_history$/.exec(u.pathname);
    if (method === 'POST' && clearMatch) {
      assert.equal(body.confirm, true, 'clear_history phải kèm confirm:true');
      return { ok: true, chat_id: Number(clearMatch[1]), deleted: true };
    }
    const delMatch = /^\/api\/chats\/(-?\d+)\/delete$/.exec(u.pathname);
    if (method === 'POST' && delMatch) {
      assert.equal(body.confirm, true, 'delete phải kèm confirm:true');
      return { ok: true, chat_id: Number(delMatch[1]), deleted: true };
    }
    throw new Error(`unexpected api ${method} ${path}`);
  };

  const mockBlobUrl = async (id) => `blob:mock/${id}`;

  const ctx = {
    $: getNode,
    el: (tag, cls, text) => new MockNode(tag, cls, text),
    Image: class MockImage extends MockNode { constructor() { super('img'); } },
    document: doc,
    window: Object.assign(new EventTarget(), { innerWidth: 1440, innerHeight: 900, open: () => {} }),
    COLORS: ['#e17076', '#7bc862', '#e5ca77', '#65aadd'],
    account: '+84900000000',
    chats,
    cur: options.cur ?? null,
    curKind: options.curKind ?? '',
    replyTo: 0,
    tab: 'chats',
    oldest: 0,
    lastDay: null,
    rendered: new Map(),
    blobs: new Map(),
    msgData: new Map(),
    myId: 7,
    listTimer: null,
    typingTimer: null,
    unreadTotal: 0,
    __mockApi: mockApi,
    __mockBlobUrl: mockBlobUrl,
    fetch: async () => ({ ok: true, json: async () => ({ items: [], cursor: 0, reset: false }) }),
    AbortController,
    accountPath: (p) => p,
    showGate: () => {},
    startCall: (video) => startedCalls.push({ video, cur: ctx.cur }),
    updateComposePreview: () => {},
    markRead: () => {},
    openContact: () => {},
    appendMessages: (list, prepend) => {
      const box = getNode('msgs');
      let day = prepend ? null : ctx.lastDay;
      for (const m of list) {
        if (ctx.rendered.has(m.id)) continue;
        ctx.msgData.set(m.id, m);
        const d = ctx.exports.dayLabel(m.date);
        if (d !== day) { box.appendChild(new MockNode('div', 'day', d)); day = d; }
        const node = ctx.exports.renderMessage(m);
        ctx.rendered.set(m.id, node);
        box.appendChild(node);
      }
      ctx.lastDay = day;
    },
    reloadCurrent: async () => {},
    Event,
    URL,
    Date,
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
    navigator: { clipboard: { writeText: async () => {} } },
  };

  for (const id of [
    'app', 'gate', 'placeholder', 'head', 'head-peer', 'head-avatar', 'head-online', 'title',
    'head-verified', 'head-muted', 'head-sub', 'head-autodelete', 'msgs', 'typing', 'reply',
    'composer', 'text', 'call-a', 'call-v', 'q', 'list', 'font-open', 'chat-info-open',
    'chat-search-open', 'chat-menu-open', 'chat-search-bar', 'chat-search-input',
    'chat-search-count', 'chat-search-prev', 'chat-search-next', 'chat-search-close', 'chat-search-results',
    'chat-ctx-menu', 'ctx-mute', 'ctx-video-call', 'ctx-search', 'ctx-pin', 'ctx-archive',
    'ctx-unread', 'ctx-clear-history', 'ctx-delete-chat',
    'font-popover', 'font-pop-close', 'font-pop-family', 'font-pop-size', 'font-pop-size-dec',
    'font-pop-size-inc', 'font-pop-size-val', 'font-pop-lh-compact', 'font-pop-lh-normal',
    'font-pop-lh-relaxed', 'font-pop-status', 'font-pop-error', 'font-pop-reset',
    'chat-info-drawer', 'chat-info-close', 'chat-info-status', 'chat-info-error',
    'chat-info-avatar', 'chat-info-title', 'chat-info-kind', 'chat-info-username',
    'chat-info-members-count', 'chat-info-sub', 'chat-info-bio',
    'ci-mute', 'ci-preview', 'ci-autodelete', 'ci-autodelete-current', 'ci-autodelete-custom-row',
    'ci-autodelete-days', 'ci-autodelete-apply', 'ci-autodelete-custom-cancel', 'ci-autodelete-warning',
    'ci-pin', 'ci-archive', 'ci-unread', 'ci-cache-usage', 'ci-cache-forever', 'ci-cache-keep', 'ci-cache-status',
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
    'lh-compact', 'lh-normal', 'lh-relaxed', 'me', 'mesub', 'account', 'toast',
  ]) {
    getNode(id);
  }
  for (const hiddenId of [
    'font-popover', 'chat-info-drawer', 'ci-invite-box', 'ci-add-contact',
    'chat-info-private', 'chat-info-group', 'ci-confirm-revoke-wrap',
    'head-autodelete', 'head-online', 'head-verified', 'head-muted',
    'chat-search-bar', 'chat-search-results', 'chat-ctx-menu',
    'ci-autodelete-custom-row', 'ci-autodelete-warning',
  ]) {
    getNode(hiddenId).hidden = true;
  }
  getNode('account').append(Object.assign(new MockNode('option', '', 'An · online'), { value: '+84900000000' }));
  getNode('account').value = '+84900000000';

  const code = `
    ${utilsCode}
    api = __mockApi;
    blobUrl = __mockBlobUrl;
    ${listCode}
    ${rendererCode}
    ${openChatCode}
    ${feedCode}
    ${settingsCode}
    ${chatInfoCode}
    this.exports = {
      hhmm, dayLabel, listTime, formatUnreadCount, formatCallDuration, formatPeerSubtitle,
      renderList, loadChats, renderMessage, openChat, onFeed, openChatInfo, closeChatInfo,
      openChatContextMenu, closeChatContextMenu, openChatSearch, closeChatSearch,
      chats, getCur: () => cur,
    };
  `;
  vm.runInNewContext(code, ctx);
  const flush = () => new Promise((r) => setTimeout(r, 20));
  return { ...ctx.exports, nodes, doc, requests, toasts, startedCalls, flush, setCur: (id, kind) => { ctx.cur = id; ctx.curKind = kind; } };
}

test('HTML sạch bảo mật, đúng phạm vi Telegram macOS (bỏ bottom nav/birthday/đổi hình nền) và đủ phần tử mới', () => {
  new vm.Script(fullScript);
  for (const banned of ['innerHTML', 'outerHTML', 'insertAdjacentHTML', 'localStorage', 'sessionStorage', 'gw_token', 'GATEWAY_TOKEN', '@font-face', 'fonts.googleapis', 'fonts.gstatic']) {
    assert.equal(html.includes(banned), false, `không được chứa ${banned}`);
  }
  assert.equal(html.includes('Đổi hình nền'), false, 'đã bỏ menu Đổi hình nền theo chỉnh 22/11');
  assert.equal(html.includes('Cập nhật sinh nhật'), false, 'đã bỏ banner sinh nhật theo chỉnh 22/11');
  assert.equal(html.includes('bottom-nav'), false, 'đã bỏ thanh điều hướng dưới kiểu điện thoại');
  for (const id of [
    'head-avatar', 'head-sub', 'head-verified', 'head-muted', 'head-online',
    'chat-search-open', 'chat-search-bar', 'chat-search-input', 'chat-menu-open',
    'chat-ctx-menu', 'ctx-mute', 'ctx-video-call', 'ctx-search',
    'ctx-pin', 'ctx-archive', 'ctx-unread', 'ctx-clear-history', 'ctx-delete-chat',
  ]) {
    assert.match(html, new RegExp(`id="${id}"`), `thiếu phần tử #${id}`);
  }
});

test('bảng ngày giờ kiểu Telegram macOS (đồng hồ cố định) và rút gọn số chưa đọc', () => {
  const h = setupMacosHarness();
  const sunNow = new Date(2026, 10, 22, 12, 30, 0).getTime(); // Chủ nhật 22/11/2026
  const ts = (y, m, d, hh = 10, mm = 0) => Math.floor(new Date(y, m - 1, d, hh, mm, 0).getTime() / 1000);

  assert.equal(h.listTime(ts(2026, 11, 22, 12, 15), sunNow), '12:15');
  assert.equal(h.listTime(ts(2026, 11, 21, 10, 0), sunNow), 'Th 7');
  assert.equal(h.listTime(ts(2026, 11, 20, 10, 0), sunNow), 'Th 6');
  assert.equal(h.listTime(ts(2026, 11, 19, 10, 0), sunNow), 'Th 5');
  assert.equal(h.listTime(ts(2026, 11, 18, 10, 0), sunNow), 'Th 4');
  assert.equal(h.listTime(ts(2026, 11, 17, 10, 0), sunNow), 'Th 3');
  assert.equal(h.listTime(ts(2026, 11, 16, 10, 0), sunNow), 'Th 2');
  const satNow = new Date(2026, 10, 21, 12, 30, 0).getTime();
  assert.equal(h.listTime(ts(2026, 11, 15, 10, 0), satNow), 'CN');
  assert.equal(h.listTime(ts(2026, 10, 29, 9, 0), sunNow), '29 tháng 10');
  assert.equal(h.listTime(ts(2025, 12, 5, 9, 0), sunNow), '05/12/25');

  assert.equal(h.dayLabel(ts(2026, 11, 22, 8, 48), sunNow), '22 tháng 11');
  assert.equal(h.dayLabel(ts(2025, 12, 5, 9, 0), sunNow), '5 tháng 12, 2025');

  assert.equal(h.formatUnreadCount(0), '');
  assert.equal(h.formatUnreadCount(1), '1');
  assert.equal(h.formatUnreadCount(185), '185');
  assert.equal(h.formatUnreadCount(999), '999');
  assert.equal(h.formatUnreadCount(1000), '1000');
  assert.equal(h.formatUnreadCount(14682), '14682');
  assert.equal(h.formatUnreadCount(99999), '99999');
  assert.equal(h.formatUnreadCount(100000), '100K');
  assert.equal(h.formatUnreadCount(123456), '123,5K');
});

test('bảng messageCall (đi/đến/nhỡ/từ chối × thoại/video × thời lượng) trong danh sách và khung chat', () => {
  const callTs = Math.floor(new Date(2026, 10, 22, 12, 15, 0).getTime() / 1000);
  const cases = [
    {
      name: 'đến + video + 18 giây',
      outgoing: false, is_video: true, reason: 'hung_up', duration: 18,
      listText: 'Cuộc gọi video đến (18 giây)', missed: false,
      bubbleTitle: 'Cuộc gọi video đến', arrow: '↙', subTime: '12:15, 18 giây',
    },
    {
      name: 'đi + thoại + 125 giây',
      outgoing: true, is_video: false, reason: 'hung_up', duration: 125,
      listText: 'Cuộc gọi đi (2 phút 5 giây)', missed: false,
      bubbleTitle: 'Cuộc gọi đi', arrow: '↗', subTime: '12:15, 2 phút 5 giây',
    },
    {
      name: 'nhỡ + thoại + 0 giây',
      outgoing: false, is_video: false, reason: 'missed', duration: 0,
      listText: 'Cuộc gọi nhỡ', missed: true,
      bubbleTitle: 'Cuộc gọi nhỡ', arrow: '↙', subTime: '12:15',
    },
    {
      name: 'nhỡ + video + 0 giây',
      outgoing: false, is_video: true, reason: 'missed', duration: 0,
      listText: 'Cuộc gọi video nhỡ', missed: true,
      bubbleTitle: 'Cuộc gọi video nhỡ', arrow: '↙', subTime: '12:15',
    },
    {
      name: 'bị từ chối + thoại + 0 giây',
      outgoing: false, is_video: false, reason: 'declined', duration: 0,
      listText: 'Cuộc gọi bị từ chối', missed: false,
      bubbleTitle: 'Cuộc gọi bị từ chối', arrow: '↙', subTime: '12:15',
    },
    {
      name: 'bị từ chối + video + 0 giây',
      outgoing: true, is_video: true, reason: 'declined', duration: 0,
      listText: 'Cuộc gọi video bị từ chối', missed: false,
      bubbleTitle: 'Cuộc gọi video bị từ chối', arrow: '↗', subTime: '12:15',
    },
  ];

  const chats = cases.map((c, idx) => ({
    id: idx + 1,
    title: `Liên hệ ${idx + 1}`,
    kind: 'private',
    user_id: idx + 10,
    last: {
      id: idx + 100,
      date: callTs,
      outgoing: c.outgoing,
      kind: 'call',
      text: c.listText,
      call: {
        is_video: c.is_video,
        outgoing: c.outgoing,
        duration: c.duration,
        reason: c.reason,
        status: c.missed ? 'missed' : c.reason === 'declined' ? 'declined' : (c.outgoing ? 'outgoing' : 'incoming'),
        missed: c.missed,
        label: c.bubbleTitle,
        duration_text: c.duration === 18 ? '18 giây' : c.duration === 125 ? '2 phút 5 giây' : '',
      },
    },
  }));

  const h = setupMacosHarness({ chats, cur: 1, curKind: 'private' });
  h.renderList(chats, false);
  const rows = h.nodes.get('list').findByClass('chat');
  assert.equal(rows.length, cases.length);

  cases.forEach((c, idx) => {
    const preview = rows[idx].findByClass('p')[0];
    assert.ok(preview.textContent.includes(c.listText), `${c.name}: ${preview.textContent}`);
    assert.equal(preview.textContent.startsWith('Bạn: '), false, `${c.name}: không được thêm Bạn:`);
    assert.equal(preview.hasClass('preview-call'), true);
    assert.equal(preview.hasClass('is-missed'), c.missed, `${c.name}: is-missed`);
    assert.equal(preview.hasClass('is-call'), !c.missed, `${c.name}: is-call`);

    const msgNode = h.renderMessage({
      id: idx + 500,
      chat_id: idx + 1,
      date: callTs,
      outgoing: c.outgoing,
      kind: 'call',
      summary: c.listText,
      call: chats[idx].last.call,
    });
    assert.equal(msgNode.hasClass('m-call'), true);
    assert.equal(msgNode.hasClass('call-missed'), c.missed);
    assert.equal(msgNode.findByClass('call-title')[0].textContent, c.bubbleTitle);
    const arrow = msgNode.findByClass('call-arrow')[0];
    assert.equal(arrow.textContent, c.arrow);
    assert.equal(arrow.hasClass('missed'), c.missed);
    assert.equal(arrow.hasClass('ok'), !c.missed);
    assert.equal(msgNode.findByClass('call-time')[0].textContent, c.subTime);
  });
});

test('danh sách hội thoại hiển thị đủ huy hiệu (14682, @, tắt tiếng, xác minh, online, tự xoá, tick, thumbnail, tin hệ thống, XSS an toàn)', async () => {
  const nowSec = Math.floor(Date.now() / 1000);
  const chats = [
    {
      id: 10,
      title: '星驰公益 <script>alert(1)</script>',
      kind: 'group',
      unread: 14682,
      unread_mentions: 3,
      muted: true,
      pinned: false,
      auto_delete_seconds: 0,
      last: { id: 1, date: nowSec - 60, outgoing: false, kind: 'text', sender: '星驰管家 <script>', text: '@devan_m_p…' },
    },
    {
      id: 20,
      title: 'Telegram',
      kind: 'private',
      user_id: 777,
      unread: 5,
      unread_mentions: 0,
      muted: false,
      pinned: true,
      verified: true,
      online: true,
      auto_delete_seconds: 86400,
      last: { id: 2, date: nowSec - 30, outgoing: true, read: true, kind: 'text', text: 'Xin chào' },
    },
    {
      id: 30,
      title: 'HÀ NỘI 24H Channel',
      kind: 'channel',
      unread: 185,
      muted: true,
      auto_delete_seconds: 604800,
      last: { id: 3, date: nowSec - 120, outgoing: false, kind: 'animation', thumb_file: 55, caption: 'Phượt đêm, chỉ 1 giây mất l…', text: 'GIF: Phượt đêm' },
    },
    {
      id: 40,
      title: 'VPhong',
      kind: 'private',
      user_id: 88,
      auto_delete_seconds: 2678400,
      last: { id: 4, date: nowSec - 180, outgoing: true, kind: 'service', text: 'Bạn đã đặt hẹn giờ tự hủy cho mọi cuộc trò chuyện sau 1 tháng' },
    },
  ];
  const h = setupMacosHarness({ chats });
  h.renderList(chats, false);
  await h.flush();

  const list = h.nodes.get('list');
  assert.equal(list.find('script').length, 0);
  const rows = list.findByClass('chat');
  assert.equal(rows.length, 4);

  // Dòng 1: nhóm tắt tiếng + sender + @ + 14682 xám.
  assert.equal(rows[0].findByClass('muted-icon').length, 1);
  assert.equal(rows[0].findByClass('p-sender')[0].textContent, '星驰管家 <script>: ');
  assert.equal(rows[0].findByClass('mention-badge')[0].textContent, '@');
  const b0 = rows[0].findByClass('badge')[0];
  assert.equal(b0.textContent, '14682');
  assert.equal(b0.hasClass('mute'), true);

  // Dòng 2: verified + online + pinned + 1ng + ✓✓ + badge xanh 5.
  assert.equal(rows[1].findByClass('verified-badge').length, 1);
  assert.equal(rows[1].findByClass('online-dot').length, 1);
  assert.equal(rows[1].findByClass('online-dot')[0].hasClass('top'), true);
  assert.match(rows[1].findByClass('ad-badge')[0].textContent, /1ng/);
  assert.equal(rows[1].findByClass('pin-badge').length, 1);
  const tick = rows[1].findByClass('list-tick')[0];
  assert.equal(tick.textContent, '✓✓');
  assert.equal(tick.hasClass('read'), true);
  const b1 = rows[1].findByClass('badge')[0];
  assert.equal(b1.textContent, '5');
  assert.equal(b1.hasClass('mute'), false);

  // Dòng 3: thumbnail + chú thích + 1t.
  assert.equal(rows[2].findByClass('p-thumb').length, 1);
  assert.equal(rows[2].findByClass('p-thumb')[0].find('img').length, 1);
  assert.match(rows[2].findByClass('p')[0].textContent, /Phượt đêm, chỉ 1 giây mất l/);
  assert.match(rows[2].findByClass('ad-badge')[0].textContent, /1t/);

  // Dòng 4: tin hệ thống màu xanh, không tiền tố "Bạn: ", huy hiệu 1th.
  const svcPreview = rows[3].findByClass('p')[0];
  assert.equal(svcPreview.hasClass('preview-service'), true);
  assert.equal(svcPreview.textContent, 'Bạn đã đặt hẹn giờ tự hủy cho mọi cuộc trò chuyện sau 1 tháng');
  assert.match(rows[3].findByClass('ad-badge')[0].textContent, /1th/);

  // renderMessage cho tin hệ thống dựng .m-service căn giữa.
  const svcBubble = h.renderMessage({ id: 9, kind: 'service', summary: 'Anh Tuan Chị Ngọc đã tham gia Telegram', date: nowSec });
  assert.equal(svcBubble.hasClass('m-service'), true);
  assert.equal(svcBubble.textContent, 'Anh Tuan Chị Ngọc đã tham gia Telegram');
});

test('tiêu đề khung chat, tìm kiếm trong chat, menu ngữ cảnh & xác nhận Xoá lịch sử / Xoá chat', async () => {
  const nowMs = Date.now();
  const nowSec = Math.floor(nowMs / 1000);
  const chats = [
    {
      id: 42,
      title: 'Đại ca',
      kind: 'private',
      user_id: 77,
      online: false,
      status: 'offline',
      was_online: nowSec - 900,
      verified: true,
      muted: false,
      pinned: false,
      archived: false,
      marked_unread: false,
      auto_delete_seconds: 86400,
      last_read_outbox_message_id: 100,
      last: { id: 101, date: nowSec - 60, outgoing: true, read: false, kind: 'text', text: 'Tin cuối' },
    },
    {
      id: -1001,
      title: 'Nhóm lớn',
      kind: 'group',
      member_count: 1200,
      muted: true,
      auto_delete_seconds: 0,
      last: { id: 201, date: nowSec - 120, outgoing: false, kind: 'text', text: 'Chào cả nhà' },
    },
  ];
  const messagesMap = {
    42: [
      { id: 100, chat_id: 42, date: nowSec - 300, outgoing: true, kind: 'text', text: 'Tin cũ đã đọc', entities: [] },
      { id: 101, chat_id: 42, date: nowSec - 60, outgoing: true, kind: 'text', text: 'Tìm thấy từ khoá <script>x()</script>', entities: [] },
    ],
    [-1001]: [
      { id: 201, chat_id: -1001, date: nowSec - 120, outgoing: false, sender: 'An', kind: 'text', text: 'Chào cả nhà', entities: [] },
    ],
  };

  const h = setupMacosHarness({ chats, messagesMap });
  assert.equal(h.formatPeerSubtitle({ kind: 'private', online: true }, nowMs), 'đang hoạt động');
  assert.equal(h.formatPeerSubtitle({ kind: 'private', was_online: nowSec - 900 }, nowMs), 'hoạt động 15 phút trước');
  assert.match(h.formatPeerSubtitle({ kind: 'group', member_count: 1200 }, nowMs), /1[.,]200 thành viên/);

  await h.openChat(chats[0]);
  await h.flush();
  assert.equal(h.nodes.get('title').textContent, 'Đại ca');
  assert.equal(h.nodes.get('head-verified').hidden, false);
  assert.equal(h.nodes.get('head-sub').textContent, 'hoạt động 15 phút trước');
  assert.match(h.nodes.get('head-autodelete').textContent, /1ng/);

  // Tìm kiếm trong chat.
  h.nodes.get('chat-search-open').click();
  assert.equal(h.nodes.get('chat-search-bar').hidden, false);
  h.nodes.get('chat-search-input').value = 'từ khoá';
  h.nodes.get('chat-search-input').dispatchEvent(new Event('input'));
  await new Promise((r) => setTimeout(r, 260));
  const searchReq = h.requests.find((r) => r.path.includes('/api/chats/42/messages?q='));
  assert.ok(searchReq, 'phải gọi API tìm kiếm tin nhắn');
  assert.equal(h.nodes.get('chat-search-results').find('script').length, 0);
  assert.match(h.nodes.get('chat-search-count').textContent, /1/);
  h.nodes.get('chat-search-close').click();
  assert.equal(h.nodes.get('chat-search-bar').hidden, true);

  // Mở menu ⋮ -> Tắt âm gửi TDLib ngay, Ghim / Lưu trữ / Đánh dấu chưa đọc gửi ngay.
  h.nodes.get('chat-menu-open').click();
  assert.equal(h.nodes.get('chat-ctx-menu').hidden, false);
  h.nodes.get('ctx-mute').click();
  await h.flush();
  assert.deepEqual(
    h.requests.filter((r) => r.path === '/api/chats/42/settings').at(-1).body,
    { notification: { use_default_mute_for: false, mute_for: 2147483647 } },
  );
  assert.equal(h.nodes.get('head-muted').hidden, false);

  h.nodes.get('chat-menu-open').click();
  h.nodes.get('ctx-pin').click();
  await h.flush();
  assert.deepEqual(h.requests.filter((r) => r.path === '/api/chats/42/settings').at(-1).body, { pin: true });

  h.nodes.get('chat-menu-open').click();
  h.nodes.get('ctx-archive').click();
  await h.flush();
  assert.deepEqual(h.requests.filter((r) => r.path === '/api/chats/42/settings').at(-1).body, { archive: true });

  h.nodes.get('chat-menu-open').click();
  h.nodes.get('ctx-unread').click();
  await h.flush();
  assert.deepEqual(h.requests.filter((r) => r.path === '/api/chats/42/settings').at(-1).body, { mark_unread: true });

  // Chuột phải (contextmenu) trên dòng danh sách nhóm -> kiểm tra Xoá lịch sử & Xoá chat bắt buộc hộp xác nhận.
  const listRows = h.nodes.get('list').findByClass('chat');
  const cmEvent = new Event('contextmenu');
  cmEvent.preventDefault = () => {};
  cmEvent.clientX = 120;
  cmEvent.clientY = 240;
  listRows[1].dispatchEvent(cmEvent);
  assert.equal(h.nodes.get('chat-ctx-menu').hidden, false);

  const postsBeforeClear = h.requests.filter((r) => r.method === 'POST').length;
  h.nodes.get('ctx-clear-history').click();
  assert.equal(h.nodes.get('ci-confirm-dialog').open, true);
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, postsBeforeClear);
  h.nodes.get('ci-confirm-cancel').click();
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, postsBeforeClear);

  listRows[1].dispatchEvent(cmEvent);
  h.nodes.get('ctx-clear-history').click();
  h.nodes.get('ci-confirm-ok').click();
  await h.flush();
  const clearPost = h.requests.filter((r) => r.path === '/api/chats/-1001/clear_history').at(-1);
  assert.deepEqual(clearPost.body, { confirm: true, revoke: false });

  // Xoá chat bắt buộc hộp xác nhận.
  const postsBeforeDel = h.requests.filter((r) => r.method === 'POST').length;
  h.nodes.get('chat-menu-open').click();
  h.nodes.get('ctx-delete-chat').click();
  assert.equal(h.nodes.get('ci-confirm-dialog').open, true);
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, postsBeforeDel);
  h.nodes.get('ci-confirm-cancel').click();
  assert.equal(h.requests.filter((r) => r.method === 'POST').length, postsBeforeDel);

  h.nodes.get('chat-menu-open').click();
  h.nodes.get('ctx-delete-chat').click();
  h.nodes.get('ci-confirm-revoke').checked = true;
  h.nodes.get('ci-confirm-ok').click();
  await h.flush();
  const delPost = h.requests.filter((r) => r.path === '/api/chats/42/delete').at(-1);
  assert.deepEqual(delPost.body, { confirm: true, revoke: true });
});

test('sự kiện chat_read_outbox trong onFeed đồng bộ ✓✓ ngay lập tức và cô lập tài khoản', async () => {
  const nowSec = Math.floor(Date.now() / 1000);
  const chats = [
    {
      id: 42, title: 'Đại ca', kind: 'private', user_id: 77,
      last_read_outbox_message_id: 100,
      last: { id: 200, date: nowSec, outgoing: true, read: false, kind: 'text', text: 'Alo' },
    },
  ];
  const messagesMap = {
    42: [{ id: 200, chat_id: 42, date: nowSec, outgoing: true, read: false, kind: 'text', text: 'Alo', entities: [] }],
  };
  const h = setupMacosHarness({ chats, messagesMap });
  await h.openChat(chats[0]);
  await h.flush();

  assert.equal(h.nodes.get('list').findByClass('list-tick')[0].textContent, '✓');
  assert.equal(h.nodes.get('msgs').findByClass('msg-tick')[0].textContent, '✓');

  // Tài khoản khác bị bỏ qua.
  h.onFeed({ type: 'chat_read_outbox', account: '+84999999999', chat_id: 42, last_read_outbox_message_id: 200 });
  assert.equal(h.nodes.get('list').findByClass('list-tick')[0].textContent, '✓');

  // Đúng tài khoản -> chuyển ✓✓ ở cả danh sách và khung chat.
  h.onFeed({ type: 'chat_read_outbox', account: '+84900000000', chat_id: 42, last_read_outbox_message_id: 200 });
  assert.equal(h.nodes.get('list').findByClass('list-tick')[0].textContent, '✓✓');
  assert.equal(h.nodes.get('list').findByClass('list-tick')[0].hasClass('read'), true);
  assert.equal(h.nodes.get('msgs').findByClass('msg-tick')[0].textContent, '✓✓');
  assert.equal(h.nodes.get('msgs').findByClass('msg-tick')[0].hasClass('read'), true);
});

test('đột biến 1: cắt 999+ ở formatUnreadCount làm test 14682 thất bại', () => {
  const h = setupMacosHarness({
    mutateUtils: (code) => {
      assert.ok(code.includes('if (val > 99999)'), 'Không tìm thấy ngưỡng > 99999 trong formatUnreadCount');
      return code.replace(/function formatUnreadCount[\s\S]*?\n  \}/, 'function formatUnreadCount(n) { const val = Number(n) || 0; return val > 999 ? "999+" : (val > 0 ? String(val) : ""); }');
    },
  });
  assert.equal(h.formatUnreadCount(14682), '999+');
});

test('đột biến 2: bỏ chốt xác nhận trước khi xoá chat cho phép POST /delete không cần hộp thoại', async () => {
  const chats = [{ id: 42, title: 'Đại ca', kind: 'private', user_id: 77 }];
  const h = setupMacosHarness({
    chats,
    mutateChatInfo: (code) => {
      assert.ok(code.includes('confirmDeleteChat(target)'), 'Không tìm thấy confirmDeleteChat(target)');
      return code.replace('confirmDeleteChat(target)', 'api("POST", "/api/chats/" + target.id + "/delete", { confirm: true, revoke: false })');
    },
  });
  await h.openChat(chats[0]);
  await h.flush();
  h.nodes.get('chat-menu-open').click();
  h.nodes.get('ctx-delete-chat').click();
  await h.flush();
  assert.equal(h.nodes.get('ci-confirm-dialog').open, false);
  assert.equal(h.requests.filter((r) => r.path === '/api/chats/42/delete').length, 1);
});

test('đột biến 3: bỏ lớp đỏ cho cuộc gọi nhỡ làm kiểm soát màu đỏ thất bại', () => {
  const nowSec = Math.floor(Date.now() / 1000);
  const chats = [{
    id: 1, title: 'Đại ca', kind: 'private',
    last: { id: 10, date: nowSec, outgoing: false, kind: 'call', text: 'Cuộc gọi nhỡ', call: { is_video: false, outgoing: false, duration: 0, reason: 'missed', status: 'missed', missed: true, label: 'Cuộc gọi nhỡ', duration_text: '' } },
  }];
  const h = setupMacosHarness({
    chats,
    mutateList: (code) => {
      assert.ok(code.includes('call.missed ? " is-missed" : " is-call"'), 'Không tìm thấy nhánh class is-missed');
      return code.replace('call.missed ? " is-missed" : " is-call"', '" is-call"');
    },
  });
  h.renderList(chats, false);
  const preview = h.nodes.get('list').findByClass('p')[0];
  assert.equal(preview.hasClass('is-missed'), false);
});

test('đột biến 4: bỏ nhánh chat_read_outbox trong onFeed khiến ✓ không chuyển ✓✓', async () => {
  const nowSec = Math.floor(Date.now() / 1000);
  const chats = [{
    id: 42, title: 'Đại ca', kind: 'private',
    last_read_outbox_message_id: 100,
    last: { id: 200, date: nowSec, outgoing: true, read: false, kind: 'text', text: 'Alo' },
  }];
  const h = setupMacosHarness({
    chats,
    mutateFeed: (code) => {
      assert.ok(code.includes('it.type === "chat_read_outbox"'), 'Không tìm thấy nhánh chat_read_outbox');
      return code.replace('it.type === "chat_read_outbox"', 'false');
    },
  });
  await h.openChat(chats[0]);
  await h.flush();
  h.onFeed({ type: 'chat_read_outbox', chat_id: 42, last_read_outbox_message_id: 200 });
  assert.equal(h.nodes.get('list').findByClass('list-tick')[0].textContent, '✓');
});
