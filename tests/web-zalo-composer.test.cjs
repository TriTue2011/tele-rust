const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const vm = require('node:vm');
const path = require('node:path');

const ROOT = path.resolve(__dirname, '..');
const html = readFileSync(path.join(ROOT, 'web/app.html'), 'utf8');
const styleBlock = /<style>([\s\S]*?)<\/style>/.exec(html)?.[1] || '';
const fullScript = /<script>([\s\S]*?)<\/script>/.exec(html)?.[1] || '';

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
    this.style = {};
    this.value = '';
    this.disabled = false;
    this.hidden = false;
    this.src = '';
    this.alt = '';
    if (text !== undefined && text !== '') {
      this.appendChild(new MockText(text));
    }
  }
  get childNodes() { return this.children; }
  get firstChild() { return this.children[0] || null; }
  get textContent() {
    return this.children.map((c) => (typeof c === 'string' ? c : c.textContent)).join('');
  }
  set textContent(v) {
    this.children = [];
    if (v !== '' && v !== undefined && v !== null) {
      this.appendChild(new MockText(String(v)));
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
  after(node) {
    if (!this.parentNode) return;
    const idx = this.parentNode.children.indexOf(this);
    if (idx >= 0) this.parentNode.children.splice(idx + 1, 0, node);
  }
  remove() {
    if (!this.parentNode) return;
    const idx = this.parentNode.children.indexOf(this);
    if (idx >= 0) this.parentNode.children.splice(idx, 1);
    this.parentNode = null;
  }
  focus() { this.focused = true; }
  click() { this.dispatchEvent(new Event('click')); }
  querySelector(sel) {
    if (sel.startsWith('.')) {
      const cls = sel.slice(1);
      const walk = (n) => {
        for (const c of n.children) {
          if (c instanceof MockNode) {
            if (c.className && c.className.split(/\s+/).includes(cls)) return c;
            const sub = walk(c);
            if (sub) return sub;
          }
        }
        return null;
      };
      return walk(this);
    }
    const tag = sel.toLowerCase();
    const walk = (n) => {
      for (const c of n.children) {
        if (c instanceof MockNode) {
          if (c.tag === tag) return c;
          const sub = walk(c);
          if (sub) return sub;
        }
      }
      return null;
    };
    return walk(this);
  }
  querySelectorAll(sel) {
    const out = [];
    if (sel.startsWith('.')) {
      const cls = sel.slice(1);
      const walk = (n) => {
        for (const c of n.children) {
          if (c instanceof MockNode) {
            if (c.className && c.className.split(/\s+/).includes(cls)) out.push(c);
            walk(c);
          }
        }
      };
      walk(this);
      return out;
    }
    const tag = sel.toLowerCase();
    const walk = (n) => {
      for (const c of n.children) {
        if (c instanceof MockNode) {
          if (c.tag === tag) out.push(c);
          walk(c);
        }
      }
    };
    walk(this);
    return out;
  }
}

function parseAttributes(attrString) {
  const attrs = new Map();
  const re = /([a-zA-Z0-9_-]+)(?:="([^"]*)")?/g;
  let m;
  while ((m = re.exec(attrString)) !== null) {
    attrs.set(m[1], m[2] !== undefined ? m[2] : '');
  }
  return attrs;
}

function setupTestHarness() {
  const nodes = new Map();
  const calls = [];
  const getNode = (id) => {
    if (!nodes.has(id)) nodes.set(id, new MockNode('div', '', ''));
    return nodes.get(id);
  };

  const documentMock = {
    getElementById: (id) => getNode(id),
    createElement: (tag) => new MockNode(tag),
    createTextNode: (t) => new MockText(t),
    querySelector: (sel) => {
      if (sel.startsWith('#')) return getNode(sel.slice(1));
      return getNode('mock-' + sel);
    },
    addEventListener: () => {},
    removeEventListener: () => {},
  };

  const windowMock = {
    document: documentMock,
    URL: {
      createObjectURL: (f) => `blob:mock/${f?.name || 'file'}`,
      revokeObjectURL: () => {},
    },
    navigator: {
      clipboard: { writeText: async () => {} },
      mediaDevices: { getUserMedia: async () => ({ getTracks: () => [] }) },
    },
    MediaRecorder: class {
      constructor() {}
      start() {}
      stop() { if (this.onstop) this.onstop(); }
      static isTypeSupported() { return true; }
    },
    toast: (msg) => {},
    scheduleChats: () => {},
  };

  const rendererCode = html.slice(html.indexOf('  const URL_RE'), html.indexOf('  let lastDay'));
  const composerCode = html.slice(html.indexOf('  // ---- định dạng soạn tin'), html.indexOf('  // ---- cập nhật theo thời gian thực'));

  const sandbox = {
    window: windowMock,
    document: documentMock,
    navigator: windowMock.navigator,
    MediaRecorder: windowMock.MediaRecorder,
    URL: windowMock.URL,
    $: (id) => getNode(id),
    cur: 42,
    curKind: 'private',
    replyTo: 0,
    isComposing: false,
    toast: (m) => {},
    scheduleChats: () => {},
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      return { id: 99, date: Date.now() / 1000 };
    },
    appendMessages: () => {},
    Event,
    CustomEvent: Event,
  };

  const ctx = vm.createContext(sandbox);
  vm.runInContext(`${rendererCode}\n${composerCode}`, ctx);

  return { ctx, sandbox, nodes, calls, getNode };
}

test('bố trí ô soạn tin đúng mẫu Zalo: 2 hàng, đủ 5 nút công cụ đúng thứ tự và tự co trên điện thoại', () => {
  // 1. Kiểm tra an toàn bảo mật cơ bản: không chứa các hàm nguy hiểm bị cấm
  for (const banned of ['innerHTML', 'outerHTML', 'insertAdjacentHTML', 'document.write', 'localStorage', 'sessionStorage']) {
    assert.equal(html.includes(banned), false, `Không được chứa chuỗi cấm: ${banned}`);
  }

  // 2. Trích xuất khối #composer từ HTML
  const compMatch = html.match(/<div id="composer"[^>]*>([\s\S]*?)<\/div>\s*<\/main>/);
  assert.ok(compMatch, 'Không tìm thấy phần tử #composer trong web/app.html');
  const compHtml = compMatch[1];

  // 3. Có khay chờ gửi #attach-tray
  assert.match(compHtml, /<div id="attach-tray"\s+class="attach-tray"\s+hidden><\/div>/, 'Thiếu khay chờ gửi #attach-tray');

  // 4. Hàng trên: .compose-tools chứa đúng 5 nút công cụ
  const toolsMatch = compHtml.match(/<div class="compose-tools"[^>]*>([\s\S]*?)<\/div>/);
  assert.ok(toolsMatch, 'Thiếu khối .compose-tools');
  const toolsHtml = toolsMatch[1];

  // Trích xuất các nút bấm bên trong .compose-tools
  const buttonTags = [...toolsHtml.matchAll(/<button\b([^>]*)>([\s\S]*?)<\/button>/g)];
  assert.equal(buttonTags.length, 5, `Hàng công cụ trên phải có đúng 5 nút, thực tế: ${buttonTags.length}`);

  const toolButtons = buttonTags.map((b) => {
    const attrs = parseAttributes(b[1]);
    return {
      id: attrs.get('id') || '',
      ariaLabel: attrs.get('aria-label') || '',
      title: attrs.get('title') || '',
      content: b[2].trim(),
    };
  });

  // Thứ tự 5 nút công cụ Zalo: [Ảnh/Video] [Tệp] [Sticker] [Mic ghi âm] [Ứng dụng]
  assert.equal(toolButtons[0].id, 'img-btn', 'Nút 1 phải là Ảnh/Video (id="img-btn")');
  assert.match(toolButtons[0].ariaLabel + toolButtons[0].title, /Ảnh\/Video|ảnh|video/i, 'Nút 1 phải có nhãn Ảnh/Video');

  assert.equal(toolButtons[1].id, 'attach', 'Nút 2 phải là Tệp (id="attach")');
  assert.match(toolButtons[1].ariaLabel + toolButtons[1].title, /Tệp|tệp|file/i, 'Nút 2 phải có nhãn Tệp');

  assert.equal(toolButtons[2].id, 'btn-sticker', 'Nút 3 phải là Sticker (id="btn-sticker")');
  assert.match(toolButtons[2].ariaLabel + toolButtons[2].title, /Sticker|Nhãn dán/i, 'Nút 3 phải có nhãn Sticker');

  assert.equal(toolButtons[3].id, 'btn-rec', 'Nút 4 phải là Mic ghi âm (id="btn-rec")');
  assert.match(toolButtons[3].ariaLabel + toolButtons[3].title, /Mic|ghi âm/i, 'Nút 4 phải có nhãn Mic ghi âm');

  assert.equal(toolButtons[4].id, 'btn-app', 'Nút 5 phải là Ứng dụng (id="btn-app")');
  assert.match(toolButtons[4].ariaLabel + toolButtons[4].title, /Ứng dụng/i, 'Nút 5 phải có nhãn Ứng dụng');

  // Input ẩn nhận cả image/* và video/*
  assert.match(toolsHtml, /<input id="img-file"[^>]*accept="[^"]*image\/\*[^"]*video\/\*[^"]*"/, 'img-file phải nhận cả ảnh và video');
  assert.match(toolsHtml, /<input id="file"[^>]*type="file"/, 'Thiếu input ẩn id="file"');

  // 5. Hàng dưới: .compose-row chứa [Aa định dạng], ô «Nhập tin nhắn…», [emoji], [👍 / gửi]
  const rowStart = compHtml.indexOf('<div class="compose-row"');
  assert.ok(rowStart >= 0, 'Thiếu khối .compose-row');
  const rowHtml = compHtml.slice(rowStart);

  // Thứ tự các phần tử hàng dưới:
  const fmtIdx = rowHtml.indexOf('id="fmt-more-open"');
  const textIdx = rowHtml.indexOf('id="text"');
  const emojiIdx = rowHtml.indexOf('id="emoji-btn"');
  const likeIdx = rowHtml.indexOf('id="btn-like"');
  const sendIdx = rowHtml.indexOf('id="send"');

  assert.ok(fmtIdx >= 0, 'Thiếu nút Aa định dạng (id="fmt-more-open")');
  assert.ok(textIdx >= 0, 'Thiếu ô soạn tin (id="text")');
  assert.ok(emojiIdx >= 0, 'Thiếu nút emoji (id="emoji-btn")');
  assert.ok(likeIdx >= 0, 'Thiếu nút 👍 gửi nhanh (id="btn-like")');
  assert.ok(sendIdx >= 0, 'Thiếu nút gửi (id="send")');

  assert.ok(fmtIdx < textIdx, 'Nút Aa phải đứng trước ô nhập tin');
  assert.ok(textIdx < emojiIdx, 'Ô nhập tin phải đứng trước nút emoji');
  assert.ok(emojiIdx < likeIdx, 'Nút emoji phải đứng trước nút 👍');
  assert.ok(likeIdx < sendIdx, 'Nút 👍 và nút gửi đặt ở cuối hàng dưới');

  // 6. Kiểm tra quy tắc CSS tự co trên điện thoại
  assert.match(styleBlock, /@media\s*\(max-width:\s*760px\)/, 'Phải có luật CSS thích ứng cho điện thoại');
  assert.match(styleBlock, /#composer\s*\{[^}]*safe-area-inset-bottom/, 'CSS mobile phải hỗ trợ safe area dưới');
  assert.match(styleBlock, /\.attach-tray/, 'Phải có CSS cho khay chờ gửi');
  assert.match(styleBlock, /\.tray-item\s*video/, 'Phải có CSS cho video trong khay');
  assert.match(styleBlock, /\.tray-badge/, 'Phải có CSS cho huy hiệu video trong khay');
});

test('nút 👍 ↔ gửi đổi trạng thái linh hoạt theo nội dung văn bản và tệp trong khay', () => {
  const { sandbox, getNode, calls } = setupTestHarness();
  const textEd = getNode('text');
  const btnLike = getNode('btn-like');
  const btnSend = getNode('send');

  const hook = sandbox.window.__COMPOSER_TEST_HOOK__;
  assert.ok(hook, 'Phải có __COMPOSER_TEST_HOOK__ để kiểm thử trạng thái ô soạn');

  // 1. Trạng thái ban đầu: text rỗng, khay rỗng -> 👍 hiện, Gửi ẩn
  textEd.textContent = '';
  hook.updateSendButtons();
  assert.equal(btnLike.hidden, false, 'Khi không có chữ, nút 👍 phải hiển thị');
  assert.equal(btnSend.hidden, true, 'Khi không có chữ, nút Gửi phải ẩn');

  // 2. Gõ chữ vào ô nhập -> 👍 ẩn, Gửi hiện
  textEd.textContent = 'Xin chào bạn bè Zalo!';
  hook.updateSendButtons();
  assert.equal(btnLike.hidden, true, 'Khi có chữ, nút 👍 phải ẩn');
  assert.equal(btnSend.hidden, false, 'Khi có chữ, nút Gửi phải hiển thị');

  // 3. Xoá hết chữ -> 👍 hiện lại, Gửi ẩn đi
  textEd.textContent = '   ';
  hook.updateSendButtons();
  assert.equal(btnLike.hidden, false, 'Khi xoá hết chữ, nút 👍 phải hiển thị lại');
  assert.equal(btnSend.hidden, true, 'Khi xoá hết chữ, nút Gửi phải ẩn đi');

  // 4. Ô soạn rỗng nhưng thêm tệp vào khay -> Gửi hiện, 👍 ẩn
  textEd.textContent = '';
  const mockFile = { name: 'demo.txt', size: 120, type: 'text/plain' };
  hook.stageFiles([mockFile]);
  assert.equal(hook.getPending().length, 1);
  assert.equal(btnSend.hidden, false, 'Khi có tệp chờ gửi trong khay, nút Gửi phải hiển thị');
  assert.equal(btnLike.hidden, true, 'Khi có tệp trong khay, nút 👍 phải ẩn');

  // 5. Bỏ tệp khỏi khay -> 👍 hiện lại
  hook.getPending().pop();
  hook.renderTray();
  hook.updateSendButtons();
  assert.equal(btnLike.hidden, false, 'Khi khay trống và không có chữ, nút 👍 phải hiển thị lại');
  assert.equal(btnSend.hidden, true, 'Khi khay trống và không có chữ, nút Gửi phải ẩn');

  // 6. Bấm nút 👍 gửi nhanh tin nhắn "👍"
  calls.length = 0;
  btnLike.click();
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, '/api/chats/42/send');
  assert.equal(calls[0].body.text, '👍');
});

test('đính kèm video a.mp4: nhận đúng kind=video, khay có ô xem trước video và gửi qua đường video của gateway', async () => {
  const { sandbox, getNode, calls } = setupTestHarness();
  const textEd = getNode('text');
  const tray = getNode('attach-tray');
  const btnLike = getNode('btn-like');
  const btnSend = getNode('send');

  const hook = sandbox.window.__COMPOSER_TEST_HOOK__;
  assert.ok(hook);

  // 1. Nhận diện định dạng tệp video qua isVideoFile
  assert.equal(hook.isVideoFile({ name: 'a.mp4', type: 'video/mp4' }), true);
  assert.equal(hook.isVideoFile({ name: 'a.mp4', type: '' }), true, 'Không có mime nhưng tên đuôi .mp4 vẫn là video');
  assert.equal(hook.isVideoFile({ name: 'holiday.mov', type: 'video/quicktime' }), true);
  assert.equal(hook.isVideoFile({ name: 'clip.webm', type: 'video/webm' }), true);
  assert.equal(hook.isVideoFile({ name: 'document.pdf', type: 'application/pdf' }), false);
  assert.equal(hook.isVideoFile({ name: 'photo.jpg', type: 'image/jpeg' }), false);

  // 2. Chọn tệp a.mp4 vào khay chờ gửi
  const fileA = {
    name: 'a.mp4',
    size: 5 * 1024 * 1024,
    type: 'video/mp4',
    arrayBuffer: async () => new Uint8Array([0x00, 0x00, 0x00, 0x20, 0x66, 0x74, 0x79, 0x70]).buffer,
  };

  hook.stageFiles([fileA]);

  // Khay hiển thị, nút gửi hiện
  assert.equal(tray.hidden, false, 'Khay chờ gửi phải hiển thị khi có tệp');
  assert.equal(btnSend.hidden, false, 'Nút gửi phải hiển thị khi có tệp trong khay');
  assert.equal(btnLike.hidden, true, 'Nút 👍 phải ẩn khi có tệp');

  // Khay có ô xem trước video: chứa thẻ <video> và huy hiệu video
  const videoElem = tray.querySelector('video');
  assert.ok(videoElem, 'Khay chờ gửi phải có thẻ <video> xem trước cho a.mp4');
  assert.equal(videoElem.src, 'blob:mock/a.mp4');
  const badge = tray.querySelector('.tray-badge');
  assert.ok(badge, 'Khay chờ gửi phải có huy hiệu video');
  assert.match(badge.textContent, /Video/);

  // 3. Gõ thêm chú thích vào ô soạn tin
  textEd.textContent = 'Mời bạn xem video giới thiệu';

  // 4. Bấm Gửi (hoặc gọi send)
  calls.length = 0;
  await hook.send();

  // Kiểm tra đường gửi VIDEO của gateway
  assert.equal(calls.length, 1);
  const sentCall = calls[0];
  assert.equal(sentCall.method, 'POST');
  assert.ok(sentCall.path.startsWith('/api/chats/42/send_file'), 'Phải gọi đường gửi send_file');

  const queryParams = new URLSearchParams(sentCall.path.split('?')[1]);
  assert.equal(queryParams.get('kind'), 'video', 'Video a.mp4 PHẢI gửi với kind=video');
  assert.equal(queryParams.get('name'), 'a.mp4');
  assert.equal(queryParams.get('caption'), 'Mời bạn xem video giới thiệu', 'Chú thích phải đi kèm video đơn');

  // Sau khi gửi: khay trống, ô soạn rỗng, nút quay lại 👍
  assert.equal(hook.getPending().length, 0, 'Khay chờ gửi phải dọn rỗng sau khi gửi');
  assert.equal(tray.hidden, true, 'Khay chờ gửi phải ẩn khi hết tệp');
  assert.equal(textEd.textContent, '', 'Ô soạn tin phải được xoá sạch sau khi gửi');
  assert.equal(btnLike.hidden, false, 'Nút 👍 phải hiện lại sau khi gửi hoàn tất');
  assert.equal(btnSend.hidden, true, 'Nút Gửi phải ẩn đi');
});

test('đính kèm nhiều tệp hỗn hợp (video mov + ảnh png): nhận đúng kind cho từng tệp', async () => {
  const { sandbox, getNode, calls } = setupTestHarness();
  const hook = sandbox.window.__COMPOSER_TEST_HOOK__;

  const movFile = { name: 'demo.mov', size: 1024, type: 'video/quicktime', arrayBuffer: async () => new Uint8Array([1, 2]).buffer };
  const pngFile = { name: 'shot.png', size: 512, type: 'image/png', arrayBuffer: async () => new Uint8Array([3, 4]).buffer };

  hook.stageFiles([movFile, pngFile]);
  assert.equal(hook.getPending().length, 2);

  calls.length = 0;
  await hook.send();

  assert.equal(calls.length, 2);
  const q0 = new URLSearchParams(calls[0].path.split('?')[1]);
  const q1 = new URLSearchParams(calls[1].path.split('?')[1]);

  assert.equal(q0.get('kind'), 'video');
  assert.equal(q0.get('name'), 'demo.mov');

  assert.equal(q1.get('kind'), 'photo');
  assert.equal(q1.get('name'), 'shot.png');
});
