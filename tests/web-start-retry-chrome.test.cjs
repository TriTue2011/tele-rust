// Chủ máy báo (22/11): «Tele thỉnh thoảng báo hết token, bấm vào thì về trang quản lý, phải bấm mở tài khoản mới được».
// Gốc: start() gặp ACCOUNT_OFFLINE (tài khoản chớp offline / gateway vừa khởi động lại) thì KHOÁ giao diện và bắt người dùng làm lại bằng tay.
// Yêu cầu mới: tình trạng tạm thời ⇒ hiện «đang kết nối lại» và TỰ thử lại tới khi vào được, không cần bấm gì.
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { mkdtempSync, readFileSync, rmSync } = require('node:fs');
const { once } = require('node:events');
const { test } = require('node:test');
const { openHttpOrigin } = require('./chrome-origin.cjs');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');

test('TC-START-RETRY: tài khoản tạm offline ⇒ tự thử lại và vào được, không khoá giao diện', { timeout: 60000 }, async (t) => {
  const profile = mkdtempSync(`${require('node:os').tmpdir()}/tele-startretry-browser-`);
  const browser = spawn(process.env.CHROME_BIN || 'google-chrome', ['--headless', '--no-sandbox', '--disable-gpu',
    '--no-first-run', '--disable-extensions', '--no-proxy-server', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank']);
  t.after(async () => {
    if (browser.exitCode === null && browser.signalCode === null) { browser.kill(); await once(browser, 'exit'); }
    try { rmSync(profile, { recursive: true, force: true, maxRetries: 20, retryDelay: 150 }); } catch { /* dọn thư mục tạm không được làm hỏng kết quả test */ }
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
  const page = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((target) => target.type === 'page');
  assert.ok(page, 'không thấy mục tiêu type=page');
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await once(ws, 'open');
  t.after(() => ws.close());
  const pending = new Map();
  let seq = 0;
  ws.addEventListener('message', ({ data }) => { const msg = JSON.parse(data); if (msg.id) { const resolve = pending.get(msg.id); pending.delete(msg.id); resolve(msg); } });
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
  async function until(expression, ms = 3000) {
    for (let n = 0; n < ms / 25; n++) { if (await evaluate(expression)) return; await new Promise((resolve) => setTimeout(resolve, 25)); }
    assert.fail(`quá hạn chờ: ${expression}`);
  }

  const routes = {
    '/ui/api/auth/status': { dang_nhap: true, da_thiet_lap: true },
    '/api/accounts': { tai_khoan: [{ phone: '+84900000000', status: 'online', display_name: 'An Lê' }] },
    '/api/me': { id: 7, name: 'An Lê', phone: '84900000000' },
    '/api/settings/appearance': { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' },
    '/api/chats': { chats: [{ id: 42, title: 'Hội thoại thử', kind: 'private', user_id: 9 }] },
    '/api/live/calls': { calls: [] },
    '/api/feed': { items: [], cursor: 0, reset: false },
  };
  const mock = `
    window.meCalls = 0;
    window.fetch = async (path, options = {}) => {
      const routes = ${JSON.stringify(routes)};
      const url = new URL(path, 'http://localhost');
      if (url.pathname === '/api/me') {
        window.meCalls++;
        if (window.meCalls <= 2) return new Response(JSON.stringify({ error: { code: 'ACCOUNT_OFFLINE', message: 'Tài khoản đang mất kết nối tới Telegram.' } }), { status: 503 });
      }
      if (url.pathname === '/api/feed') await new Promise((resolve) => setTimeout(resolve, 400));
      return new Response(JSON.stringify(routes[url.pathname] || {}));
    };
  `;

  await cdp('Runtime.enable');
  await cdp('Emulation.setDeviceMetricsOverride', { width: 700, height: 844, deviceScaleFactor: 1, mobile: false });
  const script = /<script>([\s\S]*?)<\/script>/.exec(html);
  await openHttpOrigin(cdp, ws, html.replace(script[0], ''));
  await evaluate(mock + script[1]);

  // Lần đầu /api/me trả 503 ACCOUNT_OFFLINE: phải hiện thông báo «kết nối lại… tự thử lại», KHÔNG phải «Vào Tài khoản để đăng nhập».
  await until('!document.getElementById("gate").hidden && /tự thử lại/.test(document.getElementById("gate-msg").textContent)');
  assert.doesNotMatch(await evaluate('document.getElementById("gate-msg").textContent'), /Chưa có tài khoản Telegram đăng nhập/);
  // Không cần bấm gì: sau khi /api/me hết lỗi (lần thử thứ 3) giao diện tự mở khoá và nạp hội thoại.
  await until('document.getElementById("gate").hidden && !document.getElementById("app").classList.contains("locked") && Boolean(document.querySelector("#list .chat"))', 15000);
  assert.ok(await evaluate('window.meCalls') >= 3, 'phải thử lại ít nhất hai lần');
  assert.equal(await evaluate('document.getElementById("me").firstChild.textContent'), 'An Lê');
});
