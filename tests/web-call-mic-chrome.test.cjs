// Bộ phát tiếng cuộc gọi độ trễ thấp (Web Audio) trong Chrome thật: tiếng bên kia KHÔNG đi qua thẻ <audio> (đệm 1–3 s) mà xếp lịch từng khối
// với độ trễ ~60 ms và bỏ khối khi hàng đợi > 350 ms. Trích ĐÚNG mã trong web/app.html rồi chạy với luồng WAV giả.
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { mkdtempSync, readFileSync, rmSync } = require('node:fs');
const { once } = require('node:events');
const { test } = require('node:test');
const { openHttpOrigin } = require('./chrome-origin.cjs');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');

test('TC-CALL-MIC: mic máy tính gửi khung 40 ms, không dồn hàng đợi khi gateway chậm', async (t) => {
  const start = html.indexOf('  const PCM_RATE');
  const end = html.indexOf('  // Webcam của máy (USB');
  assert.ok(start > 0 && end > start, 'không thấy mã mic trong app.html');
  const mic = html.slice(start, end);
  const mock = `
    var browserMic = null, browserOutput = null; var accountPath = (p) => p; var callError = (m) => { window.lastError = m; }; var watchCall = () => {};
    var $ = () => ({ hidden: false, textContent: '', addEventListener() {} }); var callAudio = { addEventListener() {}, pause() {}, removeAttribute() {}, load() {} };
    window.posts = []; window.delay = 0; window.inflight = 0; window.maxInflight = 0;
    window.fetch = async (url, init) => { window.inflight++; window.maxInflight = Math.max(window.maxInflight, window.inflight); window.posts.push({ url, bytes: init.body.byteLength, at: performance.now() });
      await new Promise((r) => setTimeout(r, window.delay)); window.inflight--; return { ok: true, status: 200 }; };
    const ctx0 = new AudioContext(); const osc = ctx0.createOscillator(); const dest = ctx0.createMediaStreamDestination(); osc.connect(dest); osc.start();
    Object.defineProperty(navigator, 'mediaDevices', { value: { getUserMedia: async () => dest.stream }, configurable: true });
  ` + mic;
  const profile = mkdtempSync(`${require('node:os').tmpdir()}/tele-call-audio-`);
  const browser = spawn(process.env.CHROME_BIN || 'google-chrome', [
    '--headless', '--no-sandbox', '--disable-gpu', '--no-first-run', '--disable-extensions', '--autoplay-policy=no-user-gesture-required',
    '--autoplay-policy=no-user-gesture-required', '--no-proxy-server', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank',
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
  await evaluate('startBrowserMic("c1")');
  await sleep(1500);
  const steady = await evaluate('window.posts.slice()');
  assert.ok(steady.length >= 20, `phải gửi đều khung, thấy ${steady.length} khung/1,5 s (lỗi: ${await evaluate('window.lastError || ""')})`);
  assert.ok(steady.every((p) => p.bytes === 1280 && p.url === '/api/live/audio/in/c1'), 'mỗi khung 40 ms = 640 mẫu × 2 byte = 1280 byte');
  // Gateway chậm (300 ms/yêu cầu): không được có hai yêu cầu cùng bay và không được dồn hàng đợi — số khung gửi bị giới hạn theo nhịp trả lời.
  await evaluate('window.delay = 300; window.posts.length = 0; window.maxInflight = 0');
  await sleep(1500);
  const slow = await evaluate('({ n: window.posts.length, max: window.maxInflight })');
  assert.equal(slow.max, 1, 'chỉ một yêu cầu gửi tại một thời điểm');
  assert.ok(slow.n <= 7, `khi gateway chậm không được dồn khung, thấy ${slow.n} yêu cầu/1,5 s`);
  assert.ok(slow.n >= 3, `vẫn phải gửi khung mới nhất sau mỗi lần trả lời, thấy ${slow.n}`);
});
