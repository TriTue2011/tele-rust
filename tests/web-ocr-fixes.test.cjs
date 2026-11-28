const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const vm = require('node:vm');
const path = require('node:path');

const ROOT = path.resolve(__dirname, '..');
const appHtml = readFileSync(path.join(ROOT, 'web/app.html'), 'utf8');
const emojiJs = readFileSync(path.join(ROOT, 'web/emoji.js'), 'utf8');
const indexHtml = readFileSync(path.join(ROOT, 'web/index.html'), 'utf8');

function slice(src, from, to) {
  const a = src.indexOf(from);
  assert.ok(a > -1, `Không thấy mốc đầu: ${from}`);
  const b = to ? src.indexOf(to, a) : src.length;
  assert.ok(b > a, `Không thấy mốc cuối: ${to}`);
  return src.slice(a, b);
}

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
    const list = this.parentNode.children;
    const i = list.indexOf(this);
    return i >= 0 && i + 1 < list.length ? list[i + 1] : null;
  }
  splitText(offset) {
    if (offset < 0 || offset > this.nodeValue.length) {
      const err = new Error('IndexSizeError');
      err.name = 'IndexSizeError';
      throw err;
    }
    const tail = new MockText(this.nodeValue.slice(offset));
    this.nodeValue = this.nodeValue.slice(0, offset);
    if (this.parentNode) {
      const idx = this.parentNode.children.indexOf(this);
      this.parentNode.children.splice(idx + 1, 0, tail);
      tail.parentNode = this.parentNode;
    }
    return tail;
  }
  remove() {
    if (!this.parentNode) return;
    const idx = this.parentNode.children.indexOf(this);
    if (idx >= 0) this.parentNode.children.splice(idx, 1);
    this.parentNode = null;
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
    this.checked = false;
    this.disabled = false;
    this.hidden = false;
    this.open = false;
    this.focused = false;
    this.src = '';
    this.alt = '';
    this.scrollTop = 0;
    this.scrollHeight = 0;
    this._root = false;
    if (text !== undefined && text !== '') {
      this.appendChild(new MockText(text));
    }
  }
  get isConnected() {
    let p = this;
    while (p) {
      if (p._root) return true;
      p = p.parentNode;
    }
    return false;
  }
  get childNodes() { return this.children; }
  get firstChild() { return this.children[0] || null; }
  get lastChild() { return this.children[this.children.length - 1] || null; }
  get nextSibling() {
    if (!this.parentNode) return null;
    const list = this.parentNode.children;
    const i = list.indexOf(this);
    return i >= 0 && i + 1 < list.length ? list[i + 1] : null;
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
  get options() { return this.children; }
  append(...nodes) {
    for (const n of nodes) this.appendChild(n);
  }
  appendChild(node) {
    const item = typeof node === 'string' ? new MockText(node) : node;
    if (item && typeof item === 'object') {
      if (item.tag === 'fragment') {
        const kids = [...item.children];
        item.children = [];
        for (const k of kids) this.appendChild(k);
        return item;
      }
      if (item.parentNode) item.remove();
      item.parentNode = this;
    }
    this.children.push(item);
    return item;
  }
  insertBefore(node, ref) {
    if (!ref) return this.appendChild(node);
    const items = node && node.tag === 'fragment' ? [...node.children] : [node];
    if (node && node.tag === 'fragment') node.children = [];
    for (const item of items) {
      if (item && typeof item === 'object') {
        if (item.parentNode) item.remove();
        item.parentNode = this;
      }
      const idx = this.children.indexOf(ref);
      if (idx === -1) this.children.push(item);
      else this.children.splice(idx, 0, item);
    }
    return node;
  }
  replaceChildren(...nodes) {
    for (const c of this.children) {
      if (c && typeof c === 'object') c.parentNode = null;
    }
    this.children = [];
    for (const n of nodes) this.appendChild(n);
  }
  cloneNode(deep = false) {
    const copy = new MockNode(this.tag, this.className);
    copy.style = { ...this.style, props: new Map(this.style.props) };
    if (deep) {
      for (const c of this.children) {
        copy.appendChild(c instanceof MockText ? new MockText(c.nodeValue) : c.cloneNode(true));
      }
    }
    return copy;
  }
  after(node) {
    if (!this.parentNode) return;
    const idx = this.parentNode.children.indexOf(this);
    if (idx >= 0) {
      if (node && typeof node === 'object') node.parentNode = this.parentNode;
      this.parentNode.children.splice(idx + 1, 0, node);
    }
  }
  remove() {
    if (!this.parentNode) return;
    const idx = this.parentNode.children.indexOf(this);
    if (idx >= 0) this.parentNode.children.splice(idx, 1);
    this.parentNode = null;
  }
  contains(node) {
    if (!node) return false;
    if (this === node) return true;
    return this.children.some((c) => (c instanceof MockNode ? c.contains(node) : c === node));
  }
  setAttribute(k, v) { this.attributes.set(k, String(v)); }
  getAttribute(k) { return this.attributes.get(k); }
  removeAttribute(k) { this.attributes.delete(k); }
  showModal() { this.open = true; }
  close() { this.open = false; this.dispatchEvent(new Event('close')); }
  focus() { this.focused = true; }
  click() { this.dispatchEvent(new Event('click')); }
  getBoundingClientRect() { return { top: 200, bottom: 220, left: 100, right: 200, width: 100, height: 20 }; }
  get offsetWidth() { return 200; }
  get offsetHeight() { return 120; }
  scrollIntoView() {}
  closest(sel) {
    if (sel.startsWith('.')) {
      const cls = sel.slice(1);
      let p = this;
      while (p) {
        if (p instanceof MockNode && (String(p.className || '').split(/\s+/).includes(cls) || p.classList?.contains(cls))) return p;
        p = p.parentNode;
      }
    }
    return null;
  }
  querySelector(sel) {
    return this.querySelectorAll(sel)[0] || null;
  }
  querySelectorAll(sel) {
    const parts = sel.split(',').map((s) => s.trim());
    const out = [];
    const match = (n, s) => {
      if (s.startsWith('.')) {
        const cls = s.slice(1);
        return String(n.className || '').split(/\s+/).includes(cls) || n.classList?.contains(cls);
      }
      if (s.startsWith('#')) return n.id === s.slice(1);
      return n.tag === s.toLowerCase();
    };
    const walk = (n) => {
      for (const c of n.children) {
        if (c instanceof MockNode) {
          if (parts.some((p) => match(c, p))) out.push(c);
          walk(c);
        }
      }
    };
    walk(this);
    return out;
  }
}

class MockRange {
  constructor() {
    this.startContainer = null;
    this.startOffset = 0;
    this.endContainer = null;
    this.endOffset = 0;
  }
  get collapsed() {
    return this.startContainer === this.endContainer && this.startOffset === this.endOffset;
  }
  get commonAncestorContainer() {
    if (this.startContainer === this.endContainer) return this.startContainer;
    let p = this.startContainer;
    while (p) {
      if (p instanceof MockNode && p.contains(this.endContainer)) return p;
      p = p.parentNode;
    }
    return null;
  }
  setStart(node, offset) {
    if (node.nodeType === 3 && offset > node.nodeValue.length) {
      const e = new Error('IndexSizeError'); e.name = 'IndexSizeError'; throw e;
    }
    if (node.nodeType === 1 && offset > node.children.length) {
      const e = new Error('IndexSizeError'); e.name = 'IndexSizeError'; throw e;
    }
    this.startContainer = node;
    this.startOffset = offset;
  }
  setEnd(node, offset) {
    if (node.nodeType === 3 && offset > node.nodeValue.length) {
      const e = new Error('IndexSizeError'); e.name = 'IndexSizeError'; throw e;
    }
    if (node.nodeType === 1 && offset > node.children.length) {
      const e = new Error('IndexSizeError'); e.name = 'IndexSizeError'; throw e;
    }
    this.endContainer = node;
    this.endOffset = offset;
  }
  setStartBefore(node) {
    const parent = node.parentNode;
    const idx = parent.children.indexOf(node);
    this.setStart(parent, idx);
  }
  collapse(toStart) {
    if (toStart) {
      this.endContainer = this.startContainer;
      this.endOffset = this.startOffset;
    } else {
      this.startContainer = this.endContainer;
      this.startOffset = this.endOffset;
    }
  }
  selectNodeContents(node) {
    this.startContainer = node;
    this.startOffset = 0;
    this.endContainer = node;
    this.endOffset = node.nodeType === 3 ? node.nodeValue.length : node.children.length;
  }
  cloneRange() {
    const r = new MockRange();
    r.startContainer = this.startContainer;
    r.startOffset = this.startOffset;
    r.endContainer = this.endContainer;
    r.endOffset = this.endOffset;
    return r;
  }
  extractContents() {
    const frag = new MockNode('fragment');
    if (this.startContainer.nodeType !== 1) return frag;
    const anc = this.startContainer;
    const startIdx = this.startOffset;
    let endIdx;
    if (this.endContainer === anc) {
      endIdx = this.endOffset;
    } else {
      // endContainer is a child (or descendant) inside anc
      let direct = this.endContainer;
      while (direct && direct.parentNode !== anc) direct = direct.parentNode;
      const idx = anc.children.indexOf(direct);
      if (this.endContainer.nodeType === 3) {
        if (this.endOffset === 0) {
          endIdx = idx;
        } else if (this.endOffset === this.endContainer.nodeValue.length) {
          endIdx = idx + 1;
        } else {
          this.endContainer.splitText(this.endOffset);
          endIdx = idx + 1;
        }
      } else {
        endIdx = idx;
      }
    }
    const moved = anc.children.slice(startIdx, endIdx);
    for (const child of moved) {
      child.remove();
      frag.appendChild(child);
    }
    return frag;
  }
}

function makeDeferred() {
  let resolve, reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

function makeDocumentWithListeners() {
  const listeners = { keydown: [], mousedown: [], selectionchange: [] };
  const body = new MockNode('body');
  body._root = true;
  const nodes = new Map();
  const getNode = (id) => {
    if (!nodes.has(id)) nodes.set(id, new MockNode('div'));
    return nodes.get(id);
  };
  const doc = {
    body,
    documentElement: new MockNode('html'),
    activeElement: null,
    cookie: '',
    getElementById: (id) => {
      const walk = (n) => {
        if (n.id === id) return n;
        for (const c of n.children) {
          if (c instanceof MockNode) {
            const hit = walk(c);
            if (hit) return hit;
          }
        }
        return null;
      };
      return walk(body) || nodes.get(id) || null;
    },
    createElement: (tag) => new MockNode(tag),
    createTextNode: (t) => new MockText(t),
    createDocumentFragment: () => new MockNode('fragment'),
    createRange: () => new MockRange(),
    querySelector: (sel) => {
      if (sel.startsWith('#')) return doc.getElementById(sel.slice(1));
      return body.querySelector(sel);
    },
    querySelectorAll: (sel) => body.querySelectorAll(sel),
    addEventListener: (type, fn, cap) => {
      listeners[type] ||= [];
      listeners[type].push({ fn, cap: Boolean(cap) });
    },
    removeEventListener: (type, fn, cap) => {
      if (!listeners[type]) return;
      listeners[type] = listeners[type].filter((x) => !(x.fn === fn && x.cap === Boolean(cap)));
    },
    dispatch: (type, event) => {
      for (const item of [...(listeners[type] || [])]) {
        item.fn(event);
      }
    },
  };
  return { doc, body, nodes, getNode, listeners };
}

// ---------------------------------------------------------------------------
// 1. sendAttachedFiles & toggleRecord
// ---------------------------------------------------------------------------
test('1. sendAttachedFiles & toggleRecord: giữ nguyên targetChat/targetReplyTo khi đổi cur và chống double toggleRecord', async () => {
  const rendererCode = slice(appHtml, '  const URL_RE', '  let lastDay');
  const composerCode = slice(appHtml, '  // ---- định dạng soạn tin', '  // ---- cập nhật theo thời gian thực');
  const { doc, getNode } = makeDocumentWithListeners();
  const apiCalls = [];
  const recInstances = [];
  const stoppedTracks = [];
  const gumDeferred = makeDeferred();
  let gumCalls = 0;

  class MockMediaRecorder {
    constructor(stream, opts) {
      this.stream = stream;
      this.opts = opts;
      this.mimeType = 'audio/ogg';
      recInstances.push(this);
    }
    start() { this.started = true; }
    stop() {
      if (this.ondataavailable) this.ondataavailable({ data: { size: 16, type: 'audio/ogg' } });
      if (this.onstop) this.onstop();
    }
    static isTypeSupported() { return true; }
  }
  class MockBlob {
    constructor(parts, opts) { this.parts = parts; this.type = opts?.type || ''; this.size = 16; }
  }
  class MockFile {
    constructor(parts, name, opts) { this.parts = parts; this.name = name; this.type = opts?.type || ''; this.size = 16; }
    async arrayBuffer() { return new ArrayBuffer(16); }
  }

  const windowMock = {
    document: doc,
    URL: { createObjectURL: () => 'blob:x', revokeObjectURL: () => {} },
    navigator: {
      mediaDevices: {
        getUserMedia: async () => {
          gumCalls++;
          return gumDeferred.promise;
        },
      },
    },
    MediaRecorder: MockMediaRecorder,
  };
  const ctx = {
    window: windowMock,
    document: doc,
    navigator: windowMock.navigator,
    MediaRecorder: MockMediaRecorder,
    Blob: MockBlob,
    File: MockFile,
    URL: windowMock.URL,
    $: getNode,
    cur: 10,
    curKind: 'private',
    replyTo: 555,
    toast: () => {},
    scheduleChats: () => {},
    appendMessages: () => {},
    api: async (method, urlPath, body) => {
      apiCalls.push({ method, urlPath, body: body === undefined ? undefined : structuredClone(body) });
      return { id: 901, date: 1700000000 };
    },
    encodeURIComponent,
    Event, Set, Map, Date, Math, Number, Object, Array, String, Boolean, JSON, RegExp, Promise, setInterval, clearInterval,
  };
  vm.createContext(ctx);
  vm.runInContext(`${rendererCode}\n${composerCode}\nthis.exports = { sendAttachedFiles, toggleRecord, setCur: (v) => { cur = v; }, getCur: () => cur, setReplyTo: (v) => { replyTo = v; }, getReplyTo: () => replyTo };`, ctx);

  // 1a. Multi-file upload + caption while cur switches mid-arrayBuffer()
  const abDeferred = makeDeferred();
  const f1 = { name: 'a.png', size: 10, type: 'image/png', arrayBuffer: () => abDeferred.promise };
  const f2 = { name: 'b.png', size: 10, type: 'image/png', arrayBuffer: async () => new ArrayBuffer(8) };

  const sendPromise = ctx.exports.sendAttachedFiles([f1, f2], 'chú thích nhiều tệp');
  // Switch cur & replyTo while f1.arrayBuffer() is pending
  ctx.exports.setCur(20);
  ctx.exports.setReplyTo(777);
  abDeferred.resolve(new ArrayBuffer(8));
  await sendPromise;

  assert.equal(apiCalls.length, 3);
  assert.match(apiCalls[0].urlPath, /^\/api\/chats\/10\/send_file\?.*reply_to=555/);
  assert.match(apiCalls[1].urlPath, /^\/api\/chats\/10\/send_file\?.*reply_to=555/);
  assert.equal(apiCalls[2].urlPath, '/api/chats/10/send');
  assert.deepEqual(apiCalls[2].body, { text: 'chú thích nhiều tệp', reply_to: 555 });
  assert.equal(ctx.exports.getReplyTo(), 777, 'không được xoá replyTo của chat mới');

  // 1b. Two rapid toggleRecord() clicks while getUserMedia is pending
  apiCalls.length = 0;
  ctx.exports.setCur(10);
  ctx.exports.setReplyTo(88);
  const p1 = ctx.exports.toggleRecord();
  const p2 = ctx.exports.toggleRecord();
  gumDeferred.resolve({ getTracks: () => [{ stop: () => stoppedTracks.push(true) }] });
  await Promise.all([p1, p2]);

  assert.equal(gumCalls, 1, 'không được gọi getUserMedia 2 lần khi đang chờ khởi tạo');
  assert.equal(recInstances.length, 1, 'chỉ được tạo 1 MediaRecorder');

  // Switch chat while recording, then stop recording -> voice goes to original chat 10 with reply_to=88
  ctx.exports.setCur(30);
  ctx.exports.setReplyTo(99);
  await ctx.exports.toggleRecord();
  await new Promise((r) => setImmediate(r));
  assert.equal(apiCalls.length, 1);
  assert.match(apiCalls[0].urlPath, /^\/api\/chats\/10\/send_file\?kind=voice.*reply_to=88/);
  assert.equal(ctx.exports.getReplyTo(), 99);
});

// ---------------------------------------------------------------------------
// 2. applyComposerFormat collapsed bold toggle off
// ---------------------------------------------------------------------------
test('2. applyComposerFormat: tắt bold tại caret rỗng (offset=8 và offset=5) không ném IndexSizeError và giữ nguyên văn bản', () => {
  const rendererCode = slice(appHtml, '  const URL_RE', '  let lastDay');
  const composerCode = slice(appHtml, '  // ---- định dạng soạn tin', '  // ---- cập nhật theo thời gian thực');
  const { doc, getNode } = makeDocumentWithListeners();
  let currentRange = null;
  const selMock = {
    get rangeCount() { return currentRange ? 1 : 0; },
    getRangeAt: () => currentRange,
    removeAllRanges: () => { currentRange = null; },
    addRange: (r) => { currentRange = r; },
  };
  const windowMock = { document: doc, getSelection: () => selMock };
  const ctx = {
    window: windowMock,
    document: doc,
    $: getNode,
    cur: 1,
    curKind: 'private',
    replyTo: 0,
    toast: () => {},
    scheduleChats: () => {},
    appendMessages: () => {},
    api: async () => ({}),
    Event, Set, Map, Date, Math, Number, Object, Array, String, Boolean, JSON, RegExp,
  };
  vm.createContext(ctx);
  vm.runInContext(`${rendererCode}\n${composerCode}\nthis.exports = { applyComposerFormat };`, ctx);

  for (const [offset, expectedBefore, expectedAfter] of [[8, 'hello wo', 'rld'], [5, 'hello', ' world']]) {
    const ed = getNode('text');
    ed.replaceChildren();
    const strong = doc.createElement('strong');
    const txt = doc.createTextNode('hello world');
    strong.appendChild(txt);
    ed.appendChild(strong);

    const r = doc.createRange();
    r.setStart(txt, offset);
    r.collapse(true);
    currentRange = r;

    ctx.exports.applyComposerFormat('bold', ed);
    const strongTexts = ed.children.filter((c) => c.tag === 'strong').map((c) => c.textContent);
    assert.deepEqual(strongTexts, [expectedBefore, expectedAfter], `offset=${offset} phải tách đúng "${expectedBefore}" / "${expectedAfter}"`);
  }
});

// ---------------------------------------------------------------------------
// 3. loadOlder, send, sendQuickLike, openChat mid-flight cur switch & dedupe #more
// ---------------------------------------------------------------------------
test('3. loadOlder, send, sendQuickLike, openChat: bỏ qua phản hồi cũ khi đổi cur và không trùng nút #more', async () => {
  const rendererCode = slice(appHtml, '  const URL_RE', '  let lastDay');
  const openChatCode = slice(appHtml, '  async function openChat(c)', '  function markRead(chatId, msgs)');
  const composerCode = slice(appHtml, '  // ---- định dạng soạn tin', '  // ---- cập nhật theo thời gian thực');
  const { doc, getNode } = makeDocumentWithListeners();

  const chatAMsgs = Array.from({ length: 35 }, (_, i) => ({ id: 100 + i, chat_id: 10, date: 1700000000 + i, text: 'A' + i, outgoing: false }));
  const chatBMsgs = Array.from({ length: 35 }, (_, i) => ({ id: 200 + i, chat_id: 20, date: 1700000000 + i, text: 'B' + i, outgoing: false }));

  let apiHandler = null;
  const ctx = {
    window: { document: doc },
    document: doc,
    $: getNode,
    el: (tag, cls, text) => new MockNode(tag, cls, text),
    hhmm: () => '12:00',
    dayLabel: () => 'Hôm nay',
    COLORS: ['#1d68c4'],
    initials: (s) => (s || '?')[0],
    chats: [{ id: 10, title: 'Chat A', kind: 'private' }, { id: 20, title: 'Chat B', kind: 'private' }],
    cur: null,
    curKind: '',
    replyTo: 0,
    tab: 'chats',
    oldest: 0,
    lastDay: null,
    rendered: new Map(),
    msgData: new Map(),
    pendingFiles: [],
    toast: () => {},
    scheduleChats: () => {},
    renderList: () => {},
    markRead: () => {},
    renderHeadPeer: () => {},
    renderHeadAutoDelete: () => {},
    closeChatSearch: () => {},
    applyBackground: () => {},
    loadChatBackground: () => {},
    onChatOpened: () => {},
    renderTray: () => {},
    api: (method, urlPath, body) => apiHandler(method, urlPath, body),
    Event, Set, Map, Date, Math, Number, Object, Array, String, Boolean, JSON, RegExp, Promise,
  };
  vm.createContext(ctx);
  vm.runInContext(`${rendererCode}\n${openChatCode}\n${composerCode}\nthis.exports = { openChat, loadOlder, send, sendQuickLike, getCur: () => cur, getOldest: () => oldest, setReplyTo: (v) => { replyTo = v; }, getReplyTo: () => replyTo };`, ctx);

  // 3a. Concurrent openChat(chatA) calls -> only ONE #more button
  const d1 = makeDeferred();
  const d2 = makeDeferred();
  let callN = 0;
  apiHandler = () => (++callN === 1 ? d1.promise : d2.promise);
  const oc1 = ctx.exports.openChat({ id: 10, title: 'Chat A', kind: 'private' });
  const oc2 = ctx.exports.openChat({ id: 10, title: 'Chat A', kind: 'private' });
  d1.resolve({ messages: chatAMsgs });
  d2.resolve({ messages: chatAMsgs });
  await Promise.all([oc1, oc2]);
  const moreButtons = getNode('msgs').children.filter((c) => c.id === 'more');
  assert.equal(moreButtons.length, 1, 'openChat đồng thời chỉ được tạo đúng 1 nút #more');

  // 3b. loadOlder mid-flight switch from Chat A to Chat B
  const olderDef = makeDeferred();
  apiHandler = (method, urlPath) => {
    if (urlPath.includes('/api/chats/10/messages') && urlPath.includes('before=')) return olderDef.promise;
    if (urlPath.includes('/api/chats/20/messages')) return Promise.resolve({ messages: chatBMsgs });
    return Promise.resolve({ messages: [] });
  };
  const loPromise = ctx.exports.loadOlder();
  await ctx.exports.openChat({ id: 20, title: 'Chat B', kind: 'private' });
  assert.equal(ctx.exports.getOldest(), 200);
  olderDef.resolve({ messages: [{ id: 50, chat_id: 10, date: 1700000000, text: 'Stale A', outgoing: false }] });
  await loPromise;
  assert.equal(ctx.exports.getOldest(), 200, 'loadOlder cũ không được ghi đè oldest của Chat B');
  assert.equal(ctx.rendered.has(50), false, 'loadOlder cũ không được chèn tin Chat A vào Chat B');

  // 3c. send & sendQuickLike mid-flight cur switch
  const sendDef = makeDeferred();
  const likeDef = makeDeferred();
  apiHandler = (method, urlPath, body) => {
    if (urlPath === '/api/chats/10/send' && body.text === 'from A') return sendDef.promise;
    if (urlPath === '/api/chats/10/send' && body.text === '👍') return likeDef.promise;
    return Promise.resolve({ messages: [] });
  };
  await ctx.exports.openChat({ id: 10, title: 'Chat A', kind: 'private' });
  getNode('text').textContent = 'from A';
  ctx.exports.setReplyTo(111);
  const sendP = ctx.exports.send();
  // Switch to Chat B while send is pending
  await ctx.exports.openChat({ id: 20, title: 'Chat B', kind: 'private' });
  getNode('text').textContent = 'draft B';
  ctx.exports.setReplyTo(222);
  sendDef.resolve({ id: 901, chat_id: 10, date: 1700000100, text: 'from A', outgoing: true });
  await sendP;
  assert.equal(getNode('text').textContent, 'draft B', 'send cũ không được xoá nháp của Chat B');
  assert.equal(ctx.exports.getReplyTo(), 222, 'send cũ không được xoá replyTo của Chat B');
  assert.equal(ctx.rendered.has(901), false, 'send cũ không được chèn tin vào Chat B');

  await ctx.exports.openChat({ id: 10, title: 'Chat A', kind: 'private' });
  ctx.exports.setReplyTo(333);
  const likeP = ctx.exports.sendQuickLike();
  await ctx.exports.openChat({ id: 20, title: 'Chat B', kind: 'private' });
  ctx.exports.setReplyTo(444);
  likeDef.resolve({ id: 902, chat_id: 10, date: 1700000200, text: '👍', outgoing: true });
  await likeP;
  assert.equal(ctx.exports.getReplyTo(), 444, 'sendQuickLike cũ không được xoá replyTo của Chat B');
  assert.equal(ctx.rendered.has(902), false, 'sendQuickLike cũ không được chèn tin vào Chat B');
});

// ---------------------------------------------------------------------------
// 4. saveChatCache tightening confirmation
// ---------------------------------------------------------------------------
test('4. saveChatCache tightening: bỏ Mãi mãi hoặc giảm giữ tệp phải mở confirm, huỷ khôi phục UI, xác nhận gửi confirm: true', async () => {
  const settingsCode = slice(appHtml, '  // ---- cài đặt chung, giao diện và hồ sơ', '  // ---- dữ liệu và lưu trữ');
  const chatInfoCode = slice(appHtml, '  // ---- thông tin hội thoại và phông khung chat', '  start();');
  const { doc } = makeDocumentWithListeners();
  const nodes = new Map();
  const makeSelect = (opts) => {
    const s = new MockNode('select');
    for (const [v, t] of opts) {
      const o = new MockNode('option', '', t);
      o.value = v;
      s.appendChild(o);
    }
    return s;
  };
  const getNode = (id) => {
    if (!nodes.has(id)) {
      if (id === 'ci-cache-keep') {
        nodes.set(id, makeSelect([['default', 'Mặc định'], ['86400', '1 ngày'], ['604800', '7 ngày'], ['forever', 'Mãi mãi']]));
      } else if (id === 'ci-autodelete') {
        nodes.set(id, makeSelect([['0', 'Tắt'], ['86400', '1 ngày']]));
      } else if (id === 'ci-mute') {
        nodes.set(id, makeSelect([['default', 'Mặc định'], ['unmuted', 'Bật']]));
      } else {
        nodes.set(id, new MockNode(id === 'account' || id.endsWith('-font') || id.endsWith('-family') ? 'select' : 'div'));
      }
    }
    return nodes.get(id);
  };
  let storagePolicy = {
    mode: 'local',
    keep: { private: 604800, groups: 604800, channels: null },
    exceptions: [{ chat_id: 42, keep: null }],
    max_cache_bytes: 2 * 1024 * 1024 * 1024,
    stream_ttl_s: 600,
    auto_download: {},
  };
  const posts = [];
  const ctx = {
    $: getNode,
    el: (t, c, x) => new MockNode(t, c, x),
    document: doc,
    account: '+84900000000',
    initials: (s) => (s || '?')[0],
    chats: [{ id: 42, title: 'An', kind: 'private' }],
    cur: 42,
    curKind: 'private',
    toast: () => {},
    scheduleChats: () => {},
    renderList: () => {},
    api: async (method, urlPath, body) => {
      if (method === 'POST') posts.push({ urlPath, body: structuredClone(body) });
      if (method === 'GET' && urlPath === '/api/settings/appearance') return { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' };
      if (method === 'GET' && urlPath === '/api/profile') return { id: 1, name: 'An', first_name: 'An', last_name: '', bio: '', username: '', phone_masked: '' };
      if (method === 'GET' && urlPath === '/api/storage/usage') return { total_bytes: 100, by_type: {}, top_chats: [] };
      if (method === 'GET' && urlPath === '/api/settings/storage') return structuredClone(storagePolicy);
      if (method === 'POST' && urlPath === '/api/settings/storage') {
        const { confirm, ...saved } = body;
        storagePolicy = saved;
        return structuredClone(storagePolicy);
      }
      if (method === 'GET' && urlPath === '/api/chats/42/info') {
        return { chat_id: 42, title: 'An', kind: 'private', notification: { use_default_mute_for: true, mute_for: 0, use_default_show_preview: true, show_preview: true }, auto_delete_seconds: 0 };
      }
      return {};
    },
    Event, URL, Set, Map, Number, Math, Object, String, Array, Boolean, Promise, setTimeout, clearTimeout, structuredClone, encodeURIComponent,
    innerWidth: 400, innerHeight: 800,
  };
  getNode('account').appendChild(Object.assign(new MockNode('option', '', 'An'), { value: '+84900000000' }));
  getNode('account').value = '+84900000000';
  vm.createContext(ctx);
  vm.runInContext(`${settingsCode}\n${chatInfoCode}\nthis.exports = { openChatInfo };`, ctx);

  await ctx.exports.openChatInfo();
  await new Promise((r) => setTimeout(r, 15));
  assert.equal(getNode('ci-cache-forever').checked, true);
  assert.equal(getNode('ci-cache-keep').value, 'forever');

  // Uncheck #ci-cache-forever -> must open #ci-confirm-dialog without POST
  getNode('ci-cache-forever').checked = false;
  getNode('ci-cache-forever').dispatchEvent(new Event('change'));
  assert.equal(getNode('ci-confirm-dialog').open, true);
  assert.equal(posts.length, 0);

  // Cancel -> no POST and UI restored
  getNode('ci-confirm-cancel').click();
  assert.equal(getNode('ci-confirm-dialog').open, false);
  assert.equal(getNode('ci-cache-forever').checked, true);
  assert.equal(getNode('ci-cache-keep').value, 'forever');
  assert.equal(posts.length, 0);

  // Change #ci-cache-keep from "forever" to "86400" -> opens confirm, confirming sends confirm: true
  getNode('ci-cache-keep').value = '86400';
  getNode('ci-cache-keep').dispatchEvent(new Event('change'));
  assert.equal(getNode('ci-confirm-dialog').open, true);
  assert.equal(posts.length, 0);
  getNode('ci-confirm-ok').click();
  await new Promise((r) => setTimeout(r, 15));
  assert.equal(posts.length, 1);
  assert.equal(posts[0].body.confirm, true);
  assert.deepEqual(posts[0].body.exceptions, [{ chat_id: 42, keep: 86400 }]);

  // Change #ci-cache-keep to "default" from forever (setup new forever exception)
  storagePolicy.exceptions = [{ chat_id: 42, keep: null }];
  await ctx.exports.openChatInfo();
  await new Promise((r) => setTimeout(r, 15));
  getNode('ci-cache-keep').value = 'default';
  getNode('ci-cache-keep').dispatchEvent(new Event('change'));
  assert.equal(getNode('ci-confirm-dialog').open, true);
  getNode('ci-confirm-ok').click();
  await new Promise((r) => setTimeout(r, 15));
  assert.equal(posts.length, 2);
  assert.equal(posts[1].body.confirm, true);
  assert.deepEqual(posts[1].body.exceptions, []);
});

// ---------------------------------------------------------------------------
// 5. startLowLatencyOutput error & clean EOF + #call-audio-enable click
// ---------------------------------------------------------------------------
test('5. startLowLatencyOutput: xoá output.src khi lỗi luồng hoặc EOF sạch, bấm #call-audio-enable gọi renewBrowserOutput', async () => {
  const code = slice(appHtml, '  const LL_LEAD', '  async function sendBrowserPcm');
  const { getNode } = makeDocumentWithListeners();
  const callAudio = Object.assign(new MockNode('audio'), {
    pause() {},
    load() {},
    play: async () => {},
    error: null,
  });
  let fetchMode = 'eof';
  let renewApiCount = 0;
  class FakeAudioContext {
    constructor() { this.state = 'running'; this.currentTime = 0; this.destination = {}; }
    resume() { return Promise.resolve(); }
    close() { return Promise.resolve(); }
  }
  const ctx = {
    window: { AudioContext: FakeAudioContext, ReadableStream: class {} },
    AudioContext: FakeAudioContext,
    $: getNode,
    callAudio,
    browserOutput: null,
    location: { origin: 'http://localhost' },
    URL,
    AbortController,
    encodeURIComponent,
    setTimeout,
    clearTimeout,
    api: async () => {
      renewApiCount++;
      return { audio_out: { url: `/api/live/audio/out/c1?t=${'a'.repeat(32)}`, expires_in: 60 } };
    },
    fetch: async () => {
      if (fetchMode === 'error') throw new Error('network drop');
      return {
        ok: true,
        status: 200,
        body: { getReader: () => ({ read: async () => ({ value: undefined, done: true }) }) },
      };
    },
    Uint8Array, DataView, String, Math, Number, Error, Promise,
  };
  vm.createContext(ctx);
  vm.runInContext(`${code}\nthis.exports = { startLowLatencyOutput, getBrowserOutput: () => browserOutput, setBrowserOutput: (v) => { browserOutput = v; } };`, ctx);

  // Clean EOF
  fetchMode = 'eof';
  const out1 = { id: 'c1', src: '/api/live/audio/out/c1?t=' + 'x'.repeat(32), timer: null, loading: false };
  ctx.exports.setBrowserOutput(out1);
  ctx.exports.startLowLatencyOutput(out1, out1.src);
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(out1.src, '', 'EOF sạch phải xoá output.src');
  assert.equal(out1.lowLatency, false);
  assert.equal(getNode('call-audio-enable').hidden, false);

  // Clicking #call-audio-enable calls renewBrowserOutput (not playCallAudio on empty <audio>)
  renewApiCount = 0;
  getNode('call-audio-enable').click();
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(renewApiCount, 1, 'bấm #call-audio-enable phải gọi renewBrowserOutput');

  // Stream error
  fetchMode = 'error';
  const out2 = { id: 'c1', src: '/api/live/audio/out/c1?t=' + 'y'.repeat(32), timer: null, loading: false };
  ctx.exports.setBrowserOutput(out2);
  ctx.exports.startLowLatencyOutput(out2, out2.src);
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(out2.src, '', 'lỗi luồng phải xoá output.src');
  assert.equal(out2.lowLatency, false);
  assert.equal(getNode('call-audio-enable').hidden, false);
});

// ---------------------------------------------------------------------------
// 6. startBrowserCamera: stop while video.play() pending & late 404/409 isolation
// ---------------------------------------------------------------------------
test('6. startBrowserCamera: stopBrowserCamera khi đang chờ video.play() ngăn setInterval, và 404/409 trễ của Call A không giết Call B', async () => {
  const code = slice(appHtml, '  let browserCamera = null;', '  function syncBrowserAudio');
  let intervalCalls = 0;
  let playDeferred = makeDeferred();
  let pendingFetch = makeDeferred();
  let blobCallbacks = [];
  const doc = {
    createElement: (tag) => {
      if (tag === 'video') {
        return {
          muted: false, playsInline: false, srcObject: null, videoWidth: 320, videoHeight: 180,
          play: () => playDeferred.promise,
        };
      }
      return {
        width: 0, height: 0,
        getContext: () => ({ drawImage: () => {} }),
        toBlob: (cb) => { blobCallbacks.push(cb); },
      };
    },
  };
  const ctx = {
    document: doc,
    navigator: {
      mediaDevices: {
        getUserMedia: async () => ({ getTracks: () => [{ stop: () => {} }] }),
      },
    },
    accountPath: (p) => p,
    callError: () => {},
    AbortController,
    encodeURIComponent,
    fetch: () => pendingFetch.promise,
    setInterval: () => { intervalCalls++; return intervalCalls; },
    clearInterval: () => {},
    Math,
  };
  vm.createContext(ctx);
  vm.runInContext(`${code}\nthis.exports = { startBrowserCamera, stopBrowserCamera, sendCameraFrame, getCam: () => browserCamera };`, ctx);

  // 6a. stopBrowserCamera() while await video.play() is pending
  const p = ctx.exports.startBrowserCamera('callA');
  await new Promise((r) => setImmediate(r));
  ctx.exports.stopBrowserCamera();
  playDeferred.resolve();
  await p;
  assert.equal(intervalCalls, 0, 'không được tạo setInterval sau khi đã stopBrowserCamera() trong lúc chờ video.play()');

  // 6b. Late 404/409 from Call A must NOT kill Call B
  playDeferred = makeDeferred();
  playDeferred.resolve();
  await ctx.exports.startBrowserCamera('callA');
  const camA = ctx.exports.getCam();
  ctx.exports.sendCameraFrame(camA, { videoWidth: 320, videoHeight: 180 }, doc.createElement('canvas'));
  assert.equal(blobCallbacks.length, 1);

  // Switch to Call B before Call A's toBlob/fetch settles
  await ctx.exports.startBrowserCamera('callB');
  const camB = ctx.exports.getCam();
  assert.equal(camB.id, 'callB');

  // Even if Call A's callback runs and fetch returns 404 (or 409), Call B must survive
  const cbPromise = blobCallbacks[0](new Blob(['x']));
  pendingFetch.resolve({ status: 404, ok: false });
  await cbPromise;
  assert.equal(ctx.exports.getCam(), camB, '404/409 trễ của Call A không được dừng camera của Call B');
});

// ---------------------------------------------------------------------------
// 7. openWallpaperDialog, openReactions, Emoji, renderTray, renderChatInfo, index.html startQrPoll
// ---------------------------------------------------------------------------
test('7a. openWallpaperDialog: mở lại gọi __close() cũ, không rò rỉ keydown listener, Escape đóng dialog thứ 2', async () => {
  const code = slice(appHtml, '  const WP_COLORS =', '  // ---- Thả cảm xúc:');
  const { doc, body, listeners } = makeDocumentWithListeners();
  const ctx = {
    document: doc,
    window: { WallpaperArt: null },
    el: (t, c, x) => new MockNode(t, c, x),
    URL: { createObjectURL: () => 'blob:x', revokeObjectURL: () => {} },
    cur: 42,
    curKind: 'private',
    api: async () => ({ backgrounds: [] }),
    toast: () => {},
    loadChatBackground: () => {},
    blobUrl: async () => 'blob:x',
    JSON, Math, Number, String, Array, Object, Error, Promise, encodeURIComponent,
  };
  vm.createContext(ctx);
  vm.runInContext(`${code}\nthis.exports = { openWallpaperDialog };`, ctx);

  await ctx.exports.openWallpaperDialog();
  assert.equal(listeners.keydown.length, 1);
  await ctx.exports.openWallpaperDialog();
  assert.equal(listeners.keydown.length, 1, 'mở lại dialog không được làm rò rỉ keydown listener');
  assert.equal(body.querySelectorAll('#wp-dialog').length, 1);

  doc.dispatch('keydown', { key: 'Escape', stopPropagation: () => {} });
  assert.equal(body.querySelectorAll('#wp-dialog').length, 0, 'Escape phải đóng được dialog thứ 2');
  assert.equal(listeners.keydown.length, 0);
});

test('7b. openReactions / closeReactions: huỷ rxDownTimer và bỏ qua phản hồi async cũ', async () => {
  const code = slice(appHtml, '  let rxPop =', '  // Nhấp đúp vào tin =');
  const { doc, listeners } = makeDocumentWithListeners();
  const d1 = makeDeferred();
  const emojiMock = {
    reactionBar: () => new MockNode('div', 'rx'),
  };
  const ctx = {
    document: doc,
    window: {
      Emoji: emojiMock,
    },
    Emoji: emojiMock,
    el: (t, c, x) => new MockNode(t, c, x),
    cur: 42,
    innerWidth: 1000,
    innerHeight: 800,
    api: () => d1.promise,
    toast: () => {},
    encodeURIComponent,
    Set, Math, Set, setTimeout, clearTimeout,
  };
  vm.createContext(ctx);
  vm.runInContext(`${code}\nthis.exports = { openReactions, closeReactions, getRxPop: () => rxPop, setCur: (v) => { cur = v; } };`, ctx);

  // Stale async response ignored when cur changes or closeReactions is called
  const p1 = ctx.exports.openReactions({ id: 1, chat_id: 42, reactions: [] }, null);
  ctx.exports.closeReactions();
  d1.resolve({ top: ['👍'], recent: [], popular: [] });
  await p1;
  assert.equal(ctx.exports.getRxPop(), null, 'phải bỏ qua phản hồi async sau khi closeReactions()');

  // Immediate closeReactions after opening cancels rxDownTimer -> zero mousedown listeners leaked
  ctx.api = async () => ({ top: ['👍'], recent: [], popular: [] });
  await ctx.exports.openReactions({ id: 2, chat_id: 42, reactions: [] }, null);
  ctx.exports.closeReactions();
  await new Promise((r) => setTimeout(r, 15));
  assert.equal(listeners.mousedown.length, 0, 'không được rò rỉ mousedown listener khi đóng ngay');
});

test('7c. Emoji.open + close() ngay lập tức không rò rỉ mousedown listener', async () => {
  const { doc, listeners } = makeDocumentWithListeners();
  const ctx = {
    window: {
      document: doc,
      innerWidth: 1000,
      innerHeight: 800,
      EmojiData: { categories: [{ id: 's', icon: '😀', label: 'Smileys', items: [['😀', 'grin', 'cuoi']] }] },
    },
    document: doc,
    setTimeout,
    clearTimeout,
    Array, Object, String, Math, JSON,
  };
  ctx.window.window = ctx.window;
  vm.createContext(ctx);
  vm.runInContext(emojiJs, ctx);

  const handle = ctx.window.Emoji.open({});
  handle.close();
  await new Promise((r) => setTimeout(r, 15));
  assert.equal(listeners.mousedown.length, 0, 'Emoji.open + close() ngay không được rò rỉ mousedown listener');
  assert.equal(listeners.keydown.length, 0);
});

test('7d. renderTray thu hồi blob URL cũ khi vẽ lại khay đính kèm', () => {
  const rendererCode = slice(appHtml, '  const URL_RE', '  let lastDay');
  const composerCode = slice(appHtml, '  // ---- định dạng soạn tin', '  // ---- cập nhật theo thời gian thực');
  const { doc, getNode } = makeDocumentWithListeners();
  const created = [];
  const revoked = [];
  let seq = 0;
  const ctx = {
    window: { document: doc, URL: { createObjectURL: () => { const u = 'blob:t-' + (++seq); created.push(u); return u; }, revokeObjectURL: (u) => revoked.push(u) } },
    document: doc,
    URL: { createObjectURL: () => { const u = 'blob:t-' + (++seq); created.push(u); return u; }, revokeObjectURL: (u) => revoked.push(u) },
    $: getNode,
    cur: 1, curKind: 'private', replyTo: 0,
    toast: () => {}, scheduleChats: () => {}, appendMessages: () => {}, api: async () => ({}),
    Event, Set, Map, Date, Math, Number, Object, Array, String, Boolean, JSON, RegExp,
  };
  ctx.URL = ctx.window.URL;
  vm.createContext(ctx);
  vm.runInContext(`${rendererCode}\n${composerCode}\nthis.exports = { stageFiles, renderTray, setPending: (a) => { pendingFiles = a; } };`, ctx);

  ctx.exports.stageFiles([{ name: 'a.png', type: 'image/png', size: 10 }]);
  assert.deepEqual(created, ['blob:t-1']);
  assert.deepEqual(revoked, []);

  ctx.exports.stageFiles([{ name: 'b.mp4', type: 'video/mp4', size: 10 }]);
  assert.deepEqual(revoked, ['blob:t-1'], 'renderTray lần 2 phải thu hồi blob URL của lần 1');

  ctx.exports.setPending([]);
  ctx.exports.renderTray();
  assert.deepEqual(revoked, ['blob:t-1', 'blob:t-2', 'blob:t-3'], 'renderTray rỗng phải thu hồi toàn bộ blob URL cũ');
});

test('7e. renderChatInfo xoá av.dataset.avFile khi info.avatar_file là null', async () => {
  const settingsCode = slice(appHtml, '  // ---- cài đặt chung, giao diện và hồ sơ', '  // ---- dữ liệu và lưu trữ');
  const chatInfoCode = slice(appHtml, '  // ---- thông tin hội thoại và phông khung chat', '  start();');
  const { doc, getNode } = makeDocumentWithListeners();
  const ctx = {
    $: getNode,
    el: (t, c, x) => new MockNode(t, c, x),
    document: doc,
    account: '+84900000000',
    initials: (s) => (s || '?')[0],
    loadAvatar: (box, fileId) => {
      if (box.dataset) box.dataset.avFile = fileId ? String(fileId) : '';
    },
    chats: [],
    cur: 42, curKind: 'private',
    toast: () => {}, scheduleChats: () => {}, renderList: () => {},
    api: async () => ({}),
    Event, URL, Set, Map, Number, Math, Object, String, Array, Boolean, Promise, setTimeout, clearTimeout,
  };
  getNode('account').appendChild(Object.assign(new MockNode('option', '', 'An'), { value: '+84900000000' }));
  getNode('account').value = '+84900000000';
  vm.createContext(ctx);
  vm.runInContext(`${settingsCode}\n${chatInfoCode}\nthis.exports = { renderChatInfo };`, ctx);

  ctx.exports.renderChatInfo({ chat_id: 42, title: 'A', kind: 'private', avatar_file: 123, notification: {}, auto_delete_seconds: 0 });
  assert.equal(getNode('chat-info-avatar').dataset.avFile, '123');

  ctx.exports.renderChatInfo({ chat_id: 42, title: 'B', kind: 'private', avatar_file: null, notification: {}, auto_delete_seconds: 0 });
  assert.equal(getNode('chat-info-avatar').dataset.avFile, '', 'avatar_file null phải xoá av.dataset.avFile');
});

test('7f. web/index.html startQrPoll không chồng request và bỏ qua phản hồi sau khi loginId = null', async () => {
  const scripts = [...indexHtml.matchAll(/<script>([\s\S]*?)<\/script>/g)];
  assert.equal(scripts.length, 1);
  const scriptCode = scripts[0][1];
  const { doc } = makeDocumentWithListeners();
  const nodes = new Map();
  const getNode = (id) => {
    if (!nodes.has(id)) nodes.set(id, new MockNode('div'));
    return nodes.get(id);
  };
  let scheduledTimers = [];
  let nextTimer = 0;
  let pollDeferred = makeDeferred();
  let pollCalls = 0;
  const ctx = {
    document: doc,
    window: {},
    confirm: () => true,
    encodeURIComponent,
    URL,
    Event,
    setTimeout: (fn, ms) => { const id = ++nextTimer; scheduledTimers.push({ id, fn, ms }); return id; },
    clearTimeout: (id) => { scheduledTimers = scheduledTimers.filter((t) => t.id !== id); },
    setInterval: () => { throw new Error('startQrPoll không được dùng setInterval'); },
    clearInterval: () => {},
    fetch: async (url) => {
      if (url === '/ui/api/auth/status') return { ok: true, json: async () => ({ dang_nhap: false, da_thiet_lap: true }) };
      if (url.startsWith('/ui/api/accounts/')) {
        pollCalls++;
        return pollDeferred.promise;
      }
      return { ok: true, json: async () => ({}) };
    },
    Error, Object, String, Boolean, Promise,
  };
  // Replace document.getElementById with getNode so $ works
  doc.getElementById = getNode;
  vm.createContext(ctx);
  // Expose internal startQrPoll / stopQrPoll / loginId hooks for testing without modifying index.html behavior
  const instrumented = scriptCode.replace(
    'refresh().catch((e) => say("auth-msg", e.message, "err"));',
    'window.__test = { startQrPoll, stopQrPoll, setLoginId: (v) => { loginId = v; }, getLoginId: () => loginId }; refresh().catch((e) => say("auth-msg", e.message, "err"));',
  );
  vm.runInContext(instrumented, ctx);
  await new Promise((r) => setImmediate(r));

  ctx.window.__test.setLoginId('qr-1');
  ctx.window.__test.startQrPoll();
  assert.equal(scheduledTimers.length, 1);
  // Fire first poll timer
  const t1 = scheduledTimers.shift();
  const run1 = t1.fn();
  assert.equal(pollCalls, 1);
  // While poll 1 is pending, no second timer should be scheduled (no overlap)
  assert.equal(scheduledTimers.length, 0, 'không được xếp lịch poll tiếp theo khi request trước chưa xong');

  // Clear loginId before poll settles -> settled response must be ignored
  ctx.window.__test.setLoginId(null);
  pollDeferred.resolve({ ok: true, json: async () => ({ stage: 'qr', qr_link: 'tg://login?token=stale' }) });
  await run1;
  assert.equal(getNode('qr-link').textContent, '', 'phản hồi sau khi loginId = null phải bị bỏ qua');
  assert.equal(scheduledTimers.length, 0, 'không được xếp lịch thêm sau khi loginId = null');
});
