const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { mkdtempSync, readFileSync, rmSync } = require('node:fs');
const { once } = require('node:events');
const { test } = require('node:test');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const policy = {
  mode: 'local', keep: { private: 604800, groups: 604800, channels: null },
  exceptions: [], max_cache_bytes: 2147483648, stream_ttl_s: 600,
  auto_download: Object.fromEntries(['private', 'groups', 'channels'].map((chat) => [chat,
    Object.fromEntries(['photo', 'video', 'file', 'voice', 'video_note', 'gif'].map((kind) => [kind, { on: false, max_bytes: 10485760 }]))])),
};

test('TC-7: lưu chế độ, sửa thời hạn và ngoại lệ, xác nhận xoá, không tràn ngang ở 390/700 px', { timeout: 30000 }, async (t) => {
  assert.doesNotMatch(html.split('// ---- dữ liệu và lưu trữ')[1], /\binnerHTML\b/);
  const routes = {
    '/ui/api/auth/status': { dang_nhap: true, da_thiet_lap: true },
    '/api/accounts': { tai_khoan: [{ phone: '+84900000001', status: 'online', display_name: 'Tài khoản thử' }] },
    '/api/me': { id: 1, name: 'Người dùng thử' },
    '/api/chats': { chats: [{ id: 42, title: '<em>Hội thoại thử</em>' }] },
    '/api/live/calls': { calls: [] }, '/api/feed': { items: [] },
    '/api/settings/storage': policy,
    '/api/storage/usage': { total_bytes: 2048, by_type: { photo: 1024, video: 1024 }, top_chats: [{ chat_id: 42, bytes: 2048 }] },
    '/api/storage/janitor': {}, '/api/storage/clear': { freed_bytes: 2048 },
  };
  const mock = `
    window.storageTestWrites = [];
    window.fetch = async (path, options) => {
      const routes = ${JSON.stringify(routes)};
      const url = new URL(path, 'http://localhost');
      let payload = routes[url.pathname] || {};
      if (options.method === 'POST') {
        const data = JSON.parse(options.body);
        window.storageTestWrites.push([url.pathname, data, options.headers['X-Requested-With'], url.searchParams.get('account')]);
        if (url.pathname === '/api/settings/storage') payload = data;
      }
      if (url.pathname === '/api/feed') await new Promise((resolve) => setTimeout(resolve, 500));
      return new Response(JSON.stringify(payload));
    };
  `;
  const profile = mkdtempSync(`${__dirname}/.storage-browser-`);
  const browser = spawn(process.env.CHROME_BIN || 'google-chrome', ['--headless', '--no-sandbox', '--disable-gpu',
    '--no-first-run', '--disable-extensions', '--no-proxy-server', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank']);
  t.after(async () => {
    if (browser.exitCode === null && browser.signalCode === null) { browser.kill(); await once(browser, 'exit'); }
    rmSync(profile, { recursive: true, force: true });
  });
  let debugOutput = '';
  const port = await new Promise((resolve, reject) => {
    browser.once('error', reject);
    browser.once('exit', () => reject(new Error(debugOutput)));
    browser.stderr.on('data', (chunk) => {
      debugOutput += chunk;
      const match = /DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)/.exec(debugOutput);
      if (match) resolve(match[1]);
    });
  });
  const [page] = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await once(ws, 'open');
  t.after(() => ws.close());
  const pending = new Map(), errors = [];
  let seq = 0;
  ws.addEventListener('message', ({ data }) => {
    const msg = JSON.parse(data);
    if (msg.method === 'Runtime.exceptionThrown') errors.push(msg.params.exceptionDetails.text);
    if (msg.method === 'Runtime.consoleAPICalled' && ['error', 'warning'].includes(msg.params.type)) errors.push(msg.params.type);
    if (msg.id) { const resolve = pending.get(msg.id); pending.delete(msg.id); resolve(msg); }
  });
  async function cdp(method, params = {}) {
    const id = ++seq;
    const msg = await new Promise((resolve) => { pending.set(id, resolve); ws.send(JSON.stringify({ id, method, params })); });
    assert.equal(msg.error, undefined, JSON.stringify(msg.error));
    return msg.result;
  }
  async function evaluate(expression) {
    const result = await cdp('Runtime.evaluate', { expression, returnByValue: true });
    assert.equal(result.exceptionDetails, undefined, JSON.stringify(result.exceptionDetails));
    return result.result.value;
  }
  async function until(expression) {
    for (let n = 0; n < 100; n++) {
      if (await evaluate(expression)) return;
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
    assert.fail(`${expression}: ${JSON.stringify(errors)} ${await evaluate('document.body?.innerText.slice(0, 400)')}`);
  }
  const click = (id) => evaluate(`document.getElementById(${JSON.stringify(id)}).click()`);
  const writes = () => evaluate('window.storageTestWrites');
  const dimension = (width) => cdp('Emulation.setDeviceMetricsOverride', { width, height: 820, deviceScaleFactor: 1, mobile: true });
  await cdp('Runtime.enable'); await dimension(390);
  const { frameTree } = await cdp('Page.getFrameTree');
  const script = /<script>([\s\S]*?)<\/script>/.exec(html);
  await cdp('Page.setDocumentContent', { frameId: frameTree.frame.id, html: html.replace(script[0], '') });
  await evaluate(mock + script[1]);
  await until('Boolean(document.querySelector("#list .chat"))');
  await click('storage-open');
  await until('document.getElementById("storage-save").disabled === false');
  await click('storage-stream');
  assert.ok(await evaluate('document.getElementById("storage-local-settings").hidden'));
  await click('storage-save');
  await until('document.getElementById("storage-status").textContent.includes("Đã lưu")');
  const [stream] = await writes();
  assert.equal(stream[1].mode, 'stream');
  assert.deepEqual(stream.slice(2), ['fetch', '+84900000001']);
  await click('storage-local');
  await evaluate(`{
    document.getElementById('storage-exception-count').click();
    const keep = document.querySelector('#storage-keep select'); keep.value = '86400'; keep.dispatchEvent(new Event('change'));
    document.getElementById('storage-exception-add').click();
  }`);
  assert.equal(await evaluate('document.querySelector("#storage-exceptions em") === null'), true);
  await click('storage-save');
  await until('document.getElementById("storage-save").disabled === false');
  const [, local] = await writes();
  assert.equal(local[1].keep.private, 86400);
  assert.deepEqual(local[1].exceptions, [{ chat_id: 42, keep: null }]);
  for (const width of [390, 700]) {
    await dimension(width);
    assert.ok(await evaluate(`(() => {
      const dialog = document.getElementById('storage-dialog');
      return innerWidth === ${width} && dialog.scrollWidth <= dialog.clientWidth &&
        document.documentElement.scrollWidth <= innerWidth && [...dialog.querySelectorAll('button,select')]
          .every((node) => !node.getClientRects().length || node.getBoundingClientRect().height >= 44);
    })()`), `${width} px`);
  }
  await click('storage-clear-open');
  assert.match(await evaluate('document.getElementById("storage-confirm-summary").textContent'), /2 KB/);
  assert.equal((await writes()).length, 2);
  await click('storage-cancel'); assert.equal((await writes()).length, 2);
  await click('storage-clear-open'); await click('storage-confirm-clear');
  await until('document.getElementById("storage-status").textContent.includes("Đã giải phóng")');
  assert.deepEqual((await writes())[2].slice(0, 2), ['/api/storage/clear', { scope: 'all', confirm: true }]);
  assert.deepEqual(errors, []);
});
