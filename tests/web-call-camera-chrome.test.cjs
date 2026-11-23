// Bộ phát tiếng cuộc gọi độ trễ thấp (Web Audio) trong Chrome thật: tiếng bên kia KHÔNG đi qua thẻ <audio> (đệm 1–3 s) mà xếp lịch từng khối
// với độ trễ ~60 ms và bỏ khối khi hàng đợi > 350 ms. Trích ĐÚNG mã trong web/app.html rồi chạy với luồng WAV giả.
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { mkdtempSync, readFileSync, rmSync } = require('node:fs');
const { once } = require('node:events');
const { test } = require('node:test');
const { openHttpOrigin } = require('./chrome-origin.cjs');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');

test('TC-CALL-CAMERA: webcam của trình duyệt gửi khung JPEG lên gateway, dừng khi tắt hoặc gateway báo 409', async (t) => {
  const start = html.indexOf('  let browserCamera = null;');
  const end = html.indexOf('  function syncBrowserAudio');
  assert.ok(start > 0 && end > start, 'không thấy mã webcam trong app.html');
  const camera = html.slice(start, end);
  const mock = `
    var accountPath = (p) => p; var callError = (m) => { window.lastError = m; };
    window.posts = []; window.respStatus = 200;
    window.fetch = async (url, init) => { const bytes = new Uint8Array(await init.body.arrayBuffer()); window.posts.push({ url, bytes: bytes.length, head: [bytes[0], bytes[1]], tail: [bytes[bytes.length - 2], bytes[bytes.length - 1]], headers: init.headers, method: init.method }); return { status: window.respStatus, ok: window.respStatus < 400 }; };
    const canvas = document.createElement('canvas'); canvas.width = 320; canvas.height = 180; const g = canvas.getContext('2d');
    setInterval(() => { g.fillStyle = 'hsl(' + (Date.now() / 10 % 360) + ',80%,50%)'; g.fillRect(0, 0, 320, 180); }, 30);
    Object.defineProperty(navigator, 'mediaDevices', { value: { getUserMedia: async () => canvas.captureStream(15) }, configurable: true });
  ` + camera;
  const profile = mkdtempSync(`${require('node:os').tmpdir()}/tele-call-audio-`);
  const browser = spawn(process.env.CHROME_BIN || 'google-chrome', [
    '--headless', '--no-sandbox', '--disable-gpu', '--no-first-run', '--disable-extensions', '--autoplay-policy=no-user-gesture-required',
    '--no-proxy-server', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank',
  ]);
  t.after(async () => {
    if (browser.exitCode === null && browser.signalCode === null) { browser.kill(); await once(browser, 'exit'); }
    try { rmSync(profile, { recursive: true, force: true, maxRetries: 20, retryDelay: 150 }); } catch { /* không làm hỏng kết quả */ }
  });
  let debugOutput = '';
  const port = await new Promise((resolve, reject) => {
    browser.once('error', reject);
    browser.once('exit', () => reject(new Error(debugOutput)));
    browser.stderr.on('data', (chunk) => { debugOutput += chunk; const m = /DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)/.exec(debugOutput); if (m) resolve(m[1]); });
  });
  const page = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((target) => target.type === 'page');
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await once(ws, 'open');
  t.after(() => ws.close());
  const pending = new Map(); let seq = 0;
  ws.addEventListener('message', ({ data }) => { const msg = JSON.parse(data); if (msg.id) { const r = pending.get(msg.id); pending.delete(msg.id); r(msg); } });
  async function cdp(method, params = {}) {
    const id = ++seq;
    const msg = await new Promise((resolve) => { pending.set(id, resolve); ws.send(JSON.stringify({ id, method, params })); });
    assert.equal(msg.error, undefined, JSON.stringify(msg.error));
    return msg.result;
  }
  async function evaluate(expression) {
    const result = await cdp('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    assert.equal(result.exceptionDetails, undefined, JSON.stringify(result.exceptionDetails));
    return result.result.value;
  }
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

  await cdp('Runtime.enable');
  await openHttpOrigin(cdp, ws, '<!doctype html><title>t</title>');
  await evaluate(mock);
  await evaluate('startBrowserCamera("c1")');
  for (let n = 0; n < 100; n++) { if (await evaluate('window.posts.length >= 3')) break; await sleep(50); }
  const frames = await evaluate('window.posts');
  assert.ok(frames.length >= 3, `phải gửi được khung JPEG, thấy ${frames.length} (lỗi: ${await evaluate('window.lastError || ""')})`);
  for (const f of frames) {
    assert.equal(f.url, '/api/live/video/in/c1'); assert.equal(f.method, 'POST'); assert.equal(f.headers['X-Requested-With'], 'fetch');
    assert.deepEqual([f.head, f.tail], [[0xFF, 0xD8], [0xFF, 0xD9]], 'khung phải là JPEG hoàn chỉnh');
    assert.ok(f.bytes > 500 && f.bytes <= 256 * 1024, `kích thước khung hợp lệ, thấy ${f.bytes}`);
  }
  // 409 từ gateway (không còn dùng camera trình duyệt) ⇒ dừng gửi.
  await evaluate('window.respStatus = 409');
  await sleep(500);
  const afterStop = await evaluate('window.posts.length');
  await sleep(600);
  assert.equal(await evaluate('window.posts.length'), afterStop, 'sau 409 không được gửi thêm khung');
  assert.equal(await evaluate('browserCamera'), null);
  // Dừng chủ động giải phóng webcam.
  await evaluate('window.respStatus = 200; startBrowserCamera("c2")');
  for (let n = 0; n < 60; n++) { if (await evaluate('Boolean(browserCamera && browserCamera.stream)')) break; await sleep(50); }
  await evaluate('stopBrowserCamera()');
  assert.equal(await evaluate('browserCamera'), null);
});
