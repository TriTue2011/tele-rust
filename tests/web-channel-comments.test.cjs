const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const vm = require('node:vm');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const thamChieu = readFileSync(`${__dirname}/../docs/THAM-CHIEU.md`, 'utf8');
const urlReMarker = '  const URL_RE';
const lastDayMarker = '  let lastDay';
const commentsMarker = '  // ---- bình luận của kênh';
const startMarker = '  start();';

class MockClassList {
  constructor() { this.set = new Set(); }
  add(...names) { names.forEach((n) => n && this.set.add(n)); }
  remove(...names) { names.forEach((n) => n && this.set.delete(n)); }
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
    this.disabled = false;
    this.hidden = false;
    this.focused = false;
    this.tabIndex = 0;
    this.title = '';
    this.scrollTop = 0;
    this.scrollHeight = 200;
    this.clientHeight = 200;
  }
  get textContent() {
    return this.children.map((c) => (typeof c === 'string' ? c : c.textContent)).join('');
  }
  set textContent(v) {
    this.children = v === '' || v === undefined || v === null ? [] : [String(v)];
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
  replaceChildren(...nodes) {
    for (const n of nodes) {
      if (n instanceof MockNode) n.parentNode = this;
    }
    this.children = nodes;
  }
  remove() {
    if (!this.parentNode) return;
    this.parentNode.children = this.parentNode.children.filter((c) => c !== this);
    this.parentNode = null;
  }
  setAttribute(k, v) { this.attributes.set(k, String(v)); }
  getAttribute(k) { return this.attributes.get(k); }
  removeAttribute(k) { this.attributes.delete(k); }
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
    return this.find(selector)[0] || null;
  }
  querySelectorAll(selector) {
    if (selector.startsWith('.')) return this.findByClass(selector.slice(1));
    return this.find(selector);
  }
}

function setupHarness(options = {}) {
  const urlStart = html.indexOf(urlReMarker);
  const lastDayStart = html.indexOf(lastDayMarker);
  const commentsStart = html.indexOf(commentsMarker);
  const startPos = html.indexOf(startMarker, commentsStart > -1 ? commentsStart : 0);

  assert.ok(urlStart > -1 && lastDayStart > urlStart, 'Không tìm thấy khối renderer tin nhắn');
  assert.ok(commentsStart > -1 && startPos > commentsStart, 'Chưa có khối // ---- bình luận của kênh trong web/app.html');

  let rendererCode = html.slice(urlStart, lastDayStart);
  let commentsCode = html.slice(commentsStart, startPos);
  if (options.mutateRenderer) rendererCode = options.mutateRenderer(rendererCode);
  if (options.mutateComments) commentsCode = options.mutateComments(commentsCode);

  const nodes = new Map();
  const getNode = (id) => {
    if (!nodes.has(id)) {
      const node = new MockNode('div');
      node.id = id;
      nodes.set(id, node);
    }
    return nodes.get(id);
  };
  for (const id of [
    'comments-drawer', 'comments-title', 'comments-sub', 'comments-close',
    'comments-post-preview', 'comments-body', 'comments-more', 'comments-status',
    'comments-error', 'comments-list', 'comments-reply-bar', 'comments-reply-txt',
    'comments-reply-clear', 'comments-form', 'comments-input', 'comments-send',
    'chat-info-drawer', 'chat-info-open',
  ]) {
    getNode(id);
  }
  getNode('comments-drawer').hidden = true;
  getNode('chat-info-drawer').hidden = true;
  getNode('comments-more').hidden = true;
  getNode('comments-reply-bar').hidden = true;

  const doc = Object.assign(new EventTarget(), {
    createElement: (tag) => new MockNode(tag),
    createTextNode: (t) => String(t),
    createDocumentFragment: () => new MockNode('fragment'),
  });

  const requests = [];
  const toasts = [];
  const loadedAvatars = [];
  const rendered = new Map();
  const msgData = new Map();
  let closeChatInfoCount = 0;

  const ctx = {
    $: getNode,
    el: (tag, cls, text) => new MockNode(tag, cls, text),
    document: doc,
    URL,
    COLORS: ['#e17076', '#7bc862', '#e5ca77', '#65aadd'],
    cur: options.cur ?? -100100,
    curKind: options.curKind ?? 'channel',
    rendered,
    msgData,
    initials: (s) => (s || '?').trim().slice(0, 1).toUpperCase(),
    hhmm: (ts) => `10:${String(Number(ts || 0) % 60).padStart(2, '0')}`,
    loadAvatar: (box, fid) => {
      loadedAvatars.push([box, fid]);
    },
    toast: (m) => toasts.push(m),
    closeChatInfo: () => {
      closeChatInfoCount++;
      getNode('chat-info-drawer').hidden = true;
    },
    api: async (method, path, body) => {
      requests.push({ method, path, body: body === undefined ? undefined : structuredClone(body) });
      if (options.apiHandler) return options.apiHandler(method, path, body);
      throw new Error(`unexpected api ${method} ${path}`);
    },
    Event,
    Set,
    Map,
    Number,
    Math,
    Object,
    String,
    Array,
    Boolean,
    Promise,
    Date,
    setTimeout,
    clearTimeout,
    structuredClone,
    encodeURIComponent,
  };

  vm.runInNewContext(
    `${rendererCode}\n${commentsCode}\nglobalThis.__exports = { renderMessage, openCommentsDrawer, closeCommentsDrawer, sendComment, setCommentReply, clearCommentReply };`,
    ctx,
  );

  return {
    ...ctx.__exports,
    $: getNode,
    getNode,
    doc,
    requests,
    toasts,
    loadedAvatars,
    rendered,
    msgData,
    getCloseChatInfoCount: () => closeChatInfoCount,
  };
}

test('kiểm tĩnh web/app.html an toàn XSS và không lộ thông tin nội bộ', () => {
  for (const banned of ['innerHTML', 'outerHTML', 'insertAdjacentHTML', 'document.write', 'localStorage', 'sessionStorage']) {
    assert.equal(html.includes(banned), false, `Không được chứa ${banned}`);
  }
  assert.doesNotMatch(
    html,
    new RegExp([['172', '16', ''].join('\\.'), ['ha', '200'].join(''), ['/opt', 'claude-c2a'].join('/')].join('|')),
  );
  assert.match(html, /id="comments-drawer"/);
  assert.match(thamChieu, /GET` \| `\/api\/chats\/:chat_id\/messages\/:message_id\/comments/);
  assert.match(thamChieu, /POST` \| `\/api\/chats\/:chat_id\/messages\/:message_id\/comments/);
});

test('renderMessage dựng hàng «N bình luận ›» kiểu macOS tối đa 3 avatar, chấm xanh và tương thích ngược', () => {
  const h = setupHarness();
  const channelPost = {
    id: 10,
    chat_id: -100100,
    date: 1700000005,
    outgoing: false,
    sender: 'Kênh Tin Tức',
    kind: 'text',
    text: 'Bài đăng kênh có bình luận',
    entities: [],
    summary: 'Bài đăng kênh có bình luận',
    reply_to: 0,
    reactions: [],
    file: null,
    comments: {
      count: 5,
      repliers: [
        { id: 1, name: '<script>xss()</script> An', avatar_file: 101 },
        { id: 2, name: 'Bình Trần', avatar_file: 102 },
        { id: 3, name: 'Chi Phạm', avatar_file: null },
        { id: 4, name: 'Người Thứ Tư', avatar_file: 104 },
      ],
      unread: true,
    },
  };
  const node = h.renderMessage(channelPost);
  const bar = node.querySelector('.msg-comments-bar');
  assert.ok(bar, 'Bài kênh có bình luận phải có .msg-comments-bar');
  assert.equal(bar.querySelectorAll('.comments-av').length, 3, 'Chỉ hiển thị tối đa 3 avatar gần nhất');
  assert.equal(bar.querySelector('.comments-count')?.textContent, '5 bình luận');
  assert.ok(bar.querySelector('.comments-unread-dot'), 'unread=true phải hiện chấm xanh chưa đọc');
  assert.equal(bar.querySelector('.comments-chevron')?.textContent, '›');
  assert.equal(node.find('script').length, 0, 'Không được chèn thẻ script từ tên người bình luận');
  assert.equal(h.loadedAvatars.length, 2, 'Chỉ nạp 2 avatar có avatar_file hợp lệ trong 3 người đầu');

  const readPost = structuredClone(channelPost);
  readPost.id = 11;
  readPost.comments.unread = false;
  const readNode = h.renderMessage(readPost);
  assert.equal(readNode.querySelector('.comments-unread-dot'), null, 'unread=false không hiện chấm xanh');

  const noComments = structuredClone(channelPost);
  noComments.id = 12;
  delete noComments.comments;
  assert.equal(h.renderMessage(noComments).querySelector('.msg-comments-bar'), null, 'Bài kênh không có bình luận giữ nguyên');

  const zeroComments = structuredClone(channelPost);
  zeroComments.id = 13;
  zeroComments.comments = { count: 0, repliers: [], unread: false };
  assert.equal(h.renderMessage(zeroComments).querySelector('.msg-comments-bar'), null, 'count=0 không dựng hàng bình luận');

  const privateHarness = setupHarness({ cur: 42, curKind: 'private' });
  const dm = { id: 20, chat_id: 42, date: 1700000010, outgoing: false, sender: 'Bạn', kind: 'text', text: 'Tin riêng', entities: [] };
  assert.equal(privateHarness.renderMessage(dm).querySelector('.msg-comments-bar'), null);
});

test('ngăn bình luận tải lịch sử, phân trang cũ hơn, trích dẫn, gửi bình luận và chống XSS', async () => {
  const post = {
    id: 10,
    chat_id: -100100,
    date: 1700000005,
    outgoing: false,
    sender: 'Kênh Tin Tức',
    kind: 'text',
    text: '<script>postXss()</script> Bài viết gốc rất hay',
    entities: [],
    summary: 'Bài viết gốc rất hay',
    comments: {
      count: 5,
      repliers: [{ id: 1, name: 'An', avatar_file: null }],
      unread: true,
    },
  };
  const h = setupHarness({
    apiHandler: async (method, path, body) => {
      if (method === 'GET' && path === '/api/chats/-100100/messages/10/comments?limit=30') {
        return {
          discussion_chat_id: -100200,
          thread_id: 500,
          root_message_id: 500,
          has_more: true,
          messages: [
            {
              id: 502, chat_id: -100200, date: 1700000012, outgoing: false,
              sender: '<img src=x onerror=xss()> Bình', kind: 'text',
              text: 'Bình luận đầu <script>alert(1)</script>', entities: [], reply_to: 500,
            },
            {
              id: 503, chat_id: -100200, date: 1700000015, outgoing: false,
              sender: 'Chi', kind: 'text',
              text: 'Trả lời lại bạn Bình', entities: [], reply_to: 502,
            },
          ],
        };
      }
      if (method === 'GET' && path === '/api/chats/-100100/messages/10/comments?limit=30&from=502') {
        return {
          discussion_chat_id: -100200,
          thread_id: 500,
          root_message_id: 500,
          has_more: false,
          messages: [
            {
              id: 501, chat_id: -100200, date: 1700000008, outgoing: false,
              sender: 'An', kind: 'text',
              text: 'Bình luận cũ hơn', entities: [], reply_to: 500,
            },
          ],
        };
      }
      if (method === 'POST' && path === '/api/chats/-100100/messages/10/comments') {
        return {
          id: 504,
          chat_id: -100200,
          date: 1700000020,
          outgoing: true,
          sender: '',
          kind: 'text',
          text: body.text,
          entities: [],
          reply_to: body.reply_to || 500,
        };
      }
      throw new Error(`unexpected ${method} ${path}`);
    },
  });

  const postEl = h.renderMessage(post);
  h.rendered.set(10, postEl);
  h.msgData.set(10, post);
  h.getNode('chat-info-drawer').hidden = false;

  // Bấm hàng bình luận trên bài viết
  postEl.querySelector('.msg-comments-bar').click();
  await new Promise((r) => setTimeout(r, 10));

  assert.equal(h.getNode('comments-drawer').hidden, false);
  assert.equal(h.getNode('chat-info-drawer').hidden, true);
  assert.equal(post.comments.unread, false, 'Mở ngăn bình luận phải xoá trạng thái chưa đọc');
  assert.equal(postEl.querySelector('.comments-unread-dot'), null);
  assert.equal(h.requests[0].path, '/api/chats/-100100/messages/10/comments?limit=30');
  assert.equal(h.getNode('comments-more').hidden, false);
  assert.equal(h.getNode('comments-list').find('script').length, 0);
  assert.equal(h.getNode('comments-list').find('img').length, 0);

  const items = h.getNode('comments-list').querySelectorAll('.comment-item');
  assert.equal(items.length, 2);
  assert.equal(items[0].querySelector('.comment-reply-quote'), null, 'Trả lời root_message_id không hiện trích dẫn thừa');
  assert.ok(items[1].querySelector('.comment-reply-quote')?.textContent.includes('Bình'), 'Trả lời bình luận khác phải hiện trích dẫn');

  // Tải thêm cũ hơn
  h.getNode('comments-more').click();
  await new Promise((r) => setTimeout(r, 10));
  assert.equal(h.requests[1].path, '/api/chats/-100100/messages/10/comments?limit=30&from=502');
  assert.equal(h.getNode('comments-more').hidden, true, 'has_more=false phải ẩn nút tải thêm');
  const afterOlder = h.getNode('comments-list').querySelectorAll('.comment-item');
  assert.equal(afterOlder.length, 3);
  assert.equal(afterOlder[0].dataset.id, '501', 'Bình luận cũ hơn phải đứng trước theo thứ tự tăng dần');

  // Chặn vượt quá 4096 ký tự phía client không gọi API
  h.getNode('comments-input').value = 'a'.repeat(4097);
  await h.sendComment();
  assert.equal(h.requests.length, 2, 'Không được gọi API khi vượt quá 4096 ký tự');
  assert.match(h.getNode('comments-error').textContent, /4096/);

  // Trả lời bình luận 503 và gửi thành công
  items[1].querySelector('.comment-reply-btn').click();
  assert.equal(h.getNode('comments-reply-bar').hidden, false);
  h.getNode('comments-input').value = 'Đồng ý với bạn';
  await h.sendComment();
  assert.equal(h.requests[2].method, 'POST');
  assert.equal(h.requests[2].path, '/api/chats/-100100/messages/10/comments');
  assert.deepEqual(h.requests[2].body, { text: 'Đồng ý với bạn', reply_to: 503 });
  assert.equal(h.getNode('comments-input').value, '');
  assert.equal(h.getNode('comments-reply-bar').hidden, true);
  assert.equal(post.comments.count, 6);
  assert.equal(postEl.querySelector('.comments-count').textContent, '6 bình luận');
});

test('lỗi 409 NO_DISCUSSION hiện thông báo thân thiện và khoá ô nhập không ném ngoại lệ', async () => {
  const h = setupHarness({
    apiHandler: async () => {
      const err = new Error('Kênh này chưa liên kết nhóm thảo luận.');
      err.status = 409;
      err.code = 'NO_DISCUSSION';
      throw err;
    },
  });
  await h.openCommentsDrawer({ id: 99, chat_id: -100100, text: 'Bài không có nhóm thảo luận' });
  assert.equal(h.getNode('comments-drawer').hidden, false);
  assert.match(h.getNode('comments-error').textContent, /nhóm thảo luận/);
  assert.equal(h.getNode('comments-input').disabled, true);
  assert.equal(h.getNode('comments-send').disabled, true);
});

test('đối chứng 4 đột biến: bỏ route, bỏ giới hạn, bỏ escape, bỏ xử lý NO_DISCUSSION phải thất bại', async () => {
  // Đột biến 1: bỏ kiểm route (/comments -> /send) và bỏ route khỏi docs/THAM-CHIEU.md
  const mRoute = setupHarness({
    mutateComments: (src) => src.replace('/comments", payload', '/send", payload'),
    apiHandler: async (method) => {
      if (method === 'GET') return { discussion_chat_id: -100200, thread_id: 500, root_message_id: 500, has_more: false, messages: [] };
      return { id: 1, chat_id: -100200, date: 1, outgoing: true, text: 'x' };
    },
  });
  await mRoute.openCommentsDrawer({ id: 10, chat_id: -100100, text: 'Tin' });
  mRoute.getNode('comments-input').value = 'Thử đột biến route';
  await mRoute.sendComment();
  assert.throws(() => assert.equal(mRoute.requests.at(-1).path, '/api/chats/-100100/messages/10/comments'));
  assert.throws(() => assert.match(
    thamChieu.replace('/api/chats/:chat_id/messages/:message_id/comments', '/khac').replace('/api/chats/:chat_id/messages/:message_id/comments', '/khac'),
    /\/api\/chats\/:chat_id\/messages\/:message_id\/comments/,
  ));

  // Đột biến 2: bỏ giới hạn 3 avatar & 4096 ký tự
  const mLimit = setupHarness({
    mutateRenderer: (src) => src.replace('.slice(0, 3)', ''),
    mutateComments: (src) => src.replace('text.length > 4096', 'false'),
    apiHandler: async (method) => {
      if (method === 'GET') return { discussion_chat_id: -100200, thread_id: 500, root_message_id: 500, has_more: false, messages: [] };
      return { id: 1, chat_id: -100200, date: 1, outgoing: true, text: 'ok' };
    },
  });
  const fourNode = mLimit.renderMessage({
    id: 10, chat_id: -100100, date: 1, kind: 'text', text: 'x',
    comments: { count: 4, repliers: [1, 2, 3, 4].map((id) => ({ id, name: 'U' + id, avatar_file: null })), unread: false },
  });
  assert.throws(() => assert.equal(fourNode.querySelectorAll('.comments-av').length, 3), 'Đột biến bỏ slice(0,3) phải lộ 4 avatar');
  await mLimit.openCommentsDrawer({ id: 10, chat_id: -100100, text: 'x' });
  mLimit.getNode('comments-input').value = 'a'.repeat(4097);
  await mLimit.sendComment();
  assert.throws(() => assert.equal(mLimit.requests.length, 1), 'Đột biến bỏ giới hạn 4096 khiến POST vẫn bị gọi');

  // Đột biến 3: chèn innerHTML (bỏ escape an toàn DOM) vào mã web phải bị phát hiện ngay
  const mutatedHtml = html.replace('bodyText.textContent = "";', 'bodyText.innerHTML = "";');
  assert.throws(() => assert.equal(mutatedHtml.includes('innerHTML'), false));

  // Đột biến 4: bỏ xử lý khoá ô nhập khi 409 NO_DISCUSSION
  const mNoDisc = setupHarness({
    mutateComments: (src) => src.replace('err.code === "NO_DISCUSSION"', 'false'),
    apiHandler: async () => {
      const err = new Error('Không có nhóm thảo luận');
      err.status = 409;
      err.code = 'NO_DISCUSSION';
      throw err;
    },
  });
  await mNoDisc.openCommentsDrawer({ id: 99, chat_id: -100100, text: 'x' });
  assert.throws(() => assert.equal(mNoDisc.getNode('comments-input').disabled, true), 'Đột biến bỏ nhánh NO_DISCUSSION khiến ô nhập không bị khoá');
});
