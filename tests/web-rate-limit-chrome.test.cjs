// 22/11: chủ máy gặp màn «Quá 120 request/phút cho token này» (429) làm KHOÁ giao diện, và avatar bị hoán đổi sau khi gateway khởi động lại
// (file id TDLib đánh lại). Hai yêu cầu: (1) 429 ⇒ chờ Retry-After rồi tự gọi lại, không khoá; (2) cờ reset của feed ⇒ bỏ bộ nhớ đệm ảnh.
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { mkdtempSync, readFileSync, rmSync } = require('node:fs');
const { once } = require('node:events');
const { test } = require('node:test');
const { openHttpOrigin } = require('./chrome-origin.cjs');
const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');

async function withPage(t, mockBody, run) {
  const profile = mkdtempSync(`${require('node:os').tmpdir()}/tele-ratelimit-browser-`);
  const browser = spawn(process.env.CHROME_BIN || 'google-chrome', ['--headless', '--no-sandbox', '--disable-gpu', '--no-first-run', '--disable-extensions',
    '--no-proxy-server', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank']);
  t.after(async () => {
    if (browser.exitCode === null && browser.signalCode === null) { browser.kill(); await once(browser, 'exit'); }
    try { rmSync(profile, { recursive: true, force: true, maxRetries: 20, retryDelay: 150 }); } catch { /* dọn tạm không làm hỏng test */ }
  });
  let debugOutput = '';
  const port = await new Promise((resolve, reject) => {
    browser.once('error', reject); browser.once('exit', () => reject(new Error(debugOutput)));
    browser.stderr.on('data', (chunk) => { debugOutput += chunk; const m = /DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)/.exec(debugOutput); if (m) resolve(m[1]); });
  });
  const page = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((target) => target.type === 'page');
  const ws = new WebSocket(page.webSocketDebuggerUrl); await once(ws, 'open'); t.after(() => ws.close());
  const pending = new Map(); let seq = 0;
  ws.addEventListener('message', ({ data }) => { const msg = JSON.parse(data); if (msg.id) { const resolve = pending.get(msg.id); pending.delete(msg.id); resolve(msg); } });
  const cdp = async (method, params = {}) => { const id = ++seq; const msg = await new Promise((resolve) => { pending.set(id, resolve); ws.send(JSON.stringify({ id, method, params })); }); assert.equal(msg.error, undefined, JSON.stringify(msg.error)); return msg.result; };
  const evaluate = async (expression) => { const r = await cdp('Runtime.evaluate', { expression, returnByValue: true }); assert.equal(r.exceptionDetails, undefined, JSON.stringify(r.exceptionDetails)); return r.result.value; };
  const until = async (expression, ms = 3000) => { for (let n = 0; n < ms / 25; n++) { if (await evaluate(expression)) return; await new Promise((resolve) => setTimeout(resolve, 25)); } assert.fail(`quá hạn chờ: ${expression}`); };
  await cdp('Runtime.enable'); await cdp('Emulation.setDeviceMetricsOverride', { width: 700, height: 844, deviceScaleFactor: 1, mobile: false });
  const script = /<script>([\s\S]*?)<\/script>/.exec(html);
  await openHttpOrigin(cdp, ws, html.replace(script[0], ''));
  await evaluate(mockBody + script[1]);
  await run({ evaluate, until });
}

const baseRoutes = `{
  '/ui/api/auth/status': { dang_nhap: true, da_thiet_lap: true },
  '/api/accounts': { tai_khoan: [{ phone: '+84900000000', status: 'online', display_name: 'An Lê' }] },
  '/api/me': { id: 7, name: 'An Lê', phone: '84900000000' },
  '/api/settings/appearance': { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' },
  '/api/chats': { chats: [{ id: 42, title: 'Hội thoại thử', kind: 'private', user_id: 9, avatar_file: 5 }] },
  '/api/live/calls': { calls: [] },
}`;

test('TC-429: vượt hạn mức ⇒ chờ Retry-After rồi tự gọi lại, giao diện không bị khoá', { timeout: 60000 }, async (t) => {
  const mock = `
    window.chatsCalls = 0;
    window.fetch = async (path, options = {}) => {
      const routes = ${baseRoutes};
      const url = new URL(path, 'http://localhost');
      if (url.pathname === '/api/me') {
        window.chatsCalls++;
        if (window.chatsCalls <= 2) return new Response(JSON.stringify({ error: { code: 'RATE_LIMITED', message: 'Quá 120 request/phút cho token này.', retry_after_seconds: 1 } }), { status: 429, headers: { 'Retry-After': '1' } });
      }
      if (url.pathname.startsWith('/api/file/')) return new Response(new Blob(['x']));
      if (url.pathname === '/api/feed') { await new Promise((resolve) => setTimeout(resolve, 400)); return new Response(JSON.stringify({ items: [], cursor: 0, reset: false })); }
      return new Response(JSON.stringify(routes[url.pathname] || {}));
    };`;
  await withPage(t, mock, async ({ evaluate, until }) => {
    await until('Boolean(document.querySelector("#list .chat"))', 15000);
    assert.equal(await evaluate('document.getElementById("gate").hidden'), true, 'giao diện KHÔNG được khoá khi gặp 429');
    assert.ok(await evaluate('window.chatsCalls') >= 3, 'phải gọi lại sau Retry-After');
    assert.doesNotMatch(await evaluate('document.getElementById("gate-msg").textContent'), /Quá 120/);
  });
});

test('TC-RESET: gateway khởi động lại (feed reset) ⇒ bỏ bộ nhớ đệm ảnh và tải lại đúng ảnh theo số file id mới', { timeout: 60000 }, async (t) => {
  const mock = `
    window.fileFetches = []; window.feedCalls = 0;
    window.fetch = async (path, options = {}) => {
      const routes = ${baseRoutes};
      const url = new URL(path, 'http://localhost');
      if (url.pathname.startsWith('/api/file/')) { window.fileFetches.push(url.pathname); return new Response(new Blob(['ảnh ' + window.fileFetches.length])); }
      if (url.pathname === '/api/feed') {
        window.feedCalls++;
        await new Promise((resolve) => setTimeout(resolve, 300));
        return new Response(JSON.stringify({ items: [], cursor: window.feedCalls, reset: window.feedCalls === 2 }));
      }
      return new Response(JSON.stringify(routes[url.pathname] || {}));
    };`;
  await withPage(t, mock, async ({ evaluate, until }) => {
    await until('window.fileFetches.length >= 1', 8000);
    // Sau khi feed báo reset (lượt thứ 2), danh sách được tải lại và cùng số file id phải được TẢI LẠI (không dùng ảnh nhớ theo số cũ).
    await until('window.fileFetches.length >= 2', 15000);
    assert.deepEqual(await evaluate('window.fileFetches.slice(0, 2)'), ['/api/file/5', '/api/file/5']);
  });
});
