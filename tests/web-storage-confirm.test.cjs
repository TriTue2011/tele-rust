const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const vm = require('node:vm');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const fullScript = /<script>([\s\S]*?)<\/script>/.exec(html)[1];
const storageCode = html.slice(
  html.indexOf('  // ---- dữ liệu và lưu trữ'),
  html.indexOf('  $("account").addEventListener'),
);

class MockNode extends EventTarget {
  constructor(tag = 'div', className = '', text = '') {
    super();
    this.tag = tag;
    this.className = className;
    this.textContent = text;
    this.children = [];
    this.attributes = new Map();
    this.style = {};
    this.value = '';
    this.checked = false;
    this.disabled = false;
    this.hidden = false;
    this.open = false;
    this.focused = false;
  }
  get selectedOptions() {
    const picked = this.children.find((child) => child.value === this.value);
    return picked ? [picked] : (this.children[0] ? [this.children[0]] : []);
  }
  append(...nodes) { this.children.push(...nodes); }
  replaceChildren(...nodes) { this.children = nodes; }
  setAttribute(k, v) { this.attributes.set(k, v); }
  getAttribute(k) { return this.attributes.get(k); }
  showModal() { this.open = true; }
  close() { this.open = false; }
  focus() { this.focused = true; }
  click() { this.dispatchEvent(new Event('click')); }
}

function defaultPolicy() {
  return {
    mode: 'local',
    keep: { private: 604800, groups: 604800, channels: null },
    exceptions: [],
    max_cache_bytes: 2147483648,
    stream_ttl_s: 600,
    auto_download: Object.fromEntries(['private', 'groups', 'channels'].map((chat) => [
      chat,
      Object.fromEntries(['photo', 'video', 'file', 'voice', 'video_note', 'gif'].map((kind) => [
        kind, { on: false, max_bytes: 10485760 },
      ])),
    ])),
  };
}

async function setupStorageHarness(source = storageCode) {
  const nodes = new Map();
  const ids = [
    'storage-open', 'storage-dialog', 'storage-account', 'storage-fields', 'storage-save',
    'storage-clear-open', 'storage-error', 'storage-mode-error', 'storage-status', 'storage-usage',
    'storage-meter', 'storage-legend', 'storage-janitor', 'storage-close', 'storage-local',
    'storage-stream', 'storage-local-settings', 'storage-keep', 'storage-exceptions',
    'storage-exception-count', 'storage-exception-chat', 'storage-exception-add', 'storage-max',
    'storage-auto', 'storage-form', 'storage-mode-confirm', 'storage-mode-confirm-summary',
    'storage-mode-cancel', 'storage-mode-confirm-ok', 'storage-clear-scope', 'storage-confirm',
    'storage-clear-error', 'storage-confirm-summary', 'storage-cancel', 'storage-confirm-clear',
    'account',
  ];
  for (const id of ids) nodes.set(id, new MockNode(id === 'account' ? 'select' : 'div'));
  nodes.get('account').append(Object.assign(new MockNode('option', '', 'Tài khoản thử'), { value: 'test-storage-acc' }));
  nodes.get('account').value = 'test-storage-acc';

  const writes = [];
  let currentPolicy = defaultPolicy();
  let clearResult = { freed_bytes: 1024, freed_files: 1, remaining_bytes: 0, remaining_files: 0, limited: false };

  async function api(method, path, body) {
    if (method === 'GET' && path === '/api/settings/storage') return structuredClone(currentPolicy);
    if (method === 'GET' && path === '/api/storage/usage') {
      return { total_bytes: 2048, by_type: { photo: 2048 }, top_chats: [{ chat_id: 42, bytes: 2048 }] };
    }
    if (method === 'GET' && path === '/api/storage/janitor') return {};
    if (method === 'POST' && path === '/api/settings/storage') {
      writes.push(structuredClone(body));
      const { confirm, ...saved } = body;
      currentPolicy = saved;
      return structuredClone(saved);
    }
    if (method === 'POST' && path === '/api/storage/clear') {
      writes.push(structuredClone(body));
      return clearResult;
    }
    throw new Error(`unexpected api ${method} ${path}`);
  }

  vm.runInNewContext(source, {
    $: (id) => {
      if (!nodes.has(id)) nodes.set(id, new MockNode());
      return nodes.get(id);
    },
    el: (tag, cls, text) => new MockNode(tag, cls, text),
    api,
    chats: [{ id: 42, title: 'Hội thoại thử' }],
    Event,
    Date,
    Number,
    Math,
    Object,
  });

  nodes.get('storage-open').click();
  await new Promise(setImmediate);
  await new Promise(setImmediate);
  return {
    nodes,
    writes,
    setClearResult: (next) => { clearResult = next; },
    submitForm: async () => {
      const event = new Event('submit');
      event.preventDefault = () => {};
      nodes.get('storage-form').dispatchEvent(event);
      await new Promise(setImmediate);
    },
    clickConfirm: async (id) => {
      nodes.get(id).click();
      await new Promise(setImmediate);
    },
  };
}

test('web/app.html parses cleanly and contains no innerHTML or token storage', () => {
  new vm.Script(fullScript);
  assert.equal(html.includes('innerHTML'), false);
  assert.equal(html.includes('sessionStorage'), false);
  assert.equal(html.includes('localStorage'), false);
  assert.equal(html.includes('gw_token'), false);
  assert.equal(html.includes('GATEWAY_TOKEN'), false);
});

test('local to stream requires confirmation dialog before POSTing confirm=true', async () => {
  const { nodes, writes, submitForm, clickConfirm, setClearResult } = await setupStorageHarness();
  nodes.get('storage-stream').click();
  assert.equal(nodes.get('storage-local-settings').hidden, true);
  assert.equal(nodes.get('storage-exceptions').hidden, false);

  await submitForm();
  assert.equal(writes.length, 0, 'chưa xác nhận thì không được gửi POST');
  assert.equal(nodes.get('storage-mode-confirm').open, true);
  assert.match(nodes.get('storage-mode-confirm-summary').textContent, /tệp cục bộ sẽ bị dọn sau 10 phút/);

  nodes.get('storage-mode-cancel').click();
  assert.equal(nodes.get('storage-mode-confirm').open, false);
  assert.equal(writes.length, 0);

  await submitForm();
  assert.equal(nodes.get('storage-mode-confirm').open, true);
  await clickConfirm('storage-mode-confirm-ok');
  assert.equal(nodes.get('storage-mode-confirm').open, false);
  assert.equal(writes.length, 1);
  assert.equal(writes[0].mode, 'stream');
  assert.equal(writes[0].confirm, true);

  // Chuyển ngược stream -> local không cần hộp xác nhận.
  nodes.get('storage-local').click();
  await submitForm();
  assert.equal(nodes.get('storage-mode-confirm').open, false);
  assert.equal(writes.length, 2);
  assert.equal(writes[1].mode, 'local');
  assert.equal(writes[1].confirm, undefined);

  // Thông báo dọn theo lượt hiển thị phần còn lại khi chạm giới hạn.
  setClearResult({ freed_bytes: 2048, freed_files: 5000, remaining_bytes: 1024, remaining_files: 1000, limited: true });
  nodes.get('storage-clear-open').click();
  await clickConfirm('storage-confirm-clear');
  assert.match(nodes.get('storage-status').textContent, /1\s*000 tệp/);
});

test('mutation: removing local-to-stream confirmation check makes guard test fail', async () => {
  const mutant = storageCode.replace(
    'storageSavedMode === "local" && storageDraft.mode === "stream"',
    'false',
  );
  assert.notEqual(mutant, storageCode);
  const { nodes, writes, submitForm } = await setupStorageHarness(mutant);
  nodes.get('storage-stream').click();
  await submitForm();
  assert.notEqual(writes.length, 0, 'đột biến phải làm mất chốt chặn');
  assert.equal(nodes.get('storage-mode-confirm').open, false);
});
