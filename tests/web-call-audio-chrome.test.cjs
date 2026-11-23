// Bộ phát tiếng cuộc gọi độ trễ thấp (Web Audio) trong Chrome thật: tiếng bên kia KHÔNG đi qua thẻ <audio> (đệm 1–3 s) mà xếp lịch từng khối
// với độ trễ ~60 ms và bỏ khối khi hàng đợi > 350 ms. Trích ĐÚNG mã trong web/app.html rồi chạy với luồng WAV giả.
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { mkdtempSync, readFileSync, rmSync } = require('node:fs');
const { once } = require('node:events');
const { test } = require('node:test');
const { openHttpOrigin } = require('./chrome-origin.cjs');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');

test('TC-CALL-AUDIO: tiếng cuộc gọi phát bằng Web Audio, độ trễ ≤ 0,4 s kể cả khi luồng dồn cục', async (t) => {
  const start = html.indexOf('  const LL_LEAD');
  const end = html.indexOf('  function stopBrowserOutput');
  assert.ok(start > 0 && end > start, 'không thấy bộ phát độ trễ thấp trong app.html');
  const player = html.slice(start, end);

  const mock = `
    var browserOutput = null; var $ = () => ({ hidden: false });
    window.rate = 16000;
    function wav(rate) {
      const h = new Uint8Array(44), v = new DataView(h.buffer);
      h.set([82, 73, 70, 70], 0); v.setUint32(4, 0xFFFFFFFF, true); h.set([87, 65, 86, 69, 102, 109, 116, 32], 8);
      v.setUint32(16, 16, true); v.setUint16(20, 1, true); v.setUint16(22, 1, true); v.setUint32(24, rate, true); v.setUint32(28, rate * 2, true);
      v.setUint16(32, 2, true); v.setUint16(34, 16, true); h.set([100, 97, 116, 97], 36); v.setUint32(40, 0xFFFFFFFF, true); return h;
    }
    const pcm = (ms) => { const b = new Uint8Array(window.rate / 1000 * ms * 2); for (let i = 0; i < b.length; i += 2) b[i + 1] = (i / 2) % 40 < 20 ? 40 : 216; return b; };
    window.feed = null;
    window.fetch = () => Promise.resolve({ ok: true, status: 200, body: new ReadableStream({ start(c) { window.feed = c; c.enqueue(wav(window.rate)); } }) });
    window.maxLead = 0;
  ` + player;

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
  await evaluate('browserOutput = { id: "c1" }; startLowLatencyOutput(browserOutput, "/api/live/audio/out/c1?t=x")');
  await until();
  async function until() { for (let n = 0; n < 100; n++) { if (await evaluate('Boolean(window.feed)')) return; await sleep(20); } assert.fail('luồng giả không được mở'); }

  // 1. Luồng đều thời gian thực (khối 20 ms mỗi 20 ms): độ trễ luôn ≤ 0,2 s, có xếp lịch.
  for (let n = 0; n < 40; n++) {
    await evaluate('window.feed.enqueue(pcm(20))');
    await sleep(20);
    await evaluate('window.maxLead = Math.max(window.maxLead, browserOutput.next - browserOutput.ctx.currentTime)');
  }
  const steady = await evaluate('({ scheduled: browserOutput.scheduled, maxLead: window.maxLead, state: browserOutput.ctx.state })');
  assert.ok(steady.scheduled >= 30, `phải xếp lịch được các khối, thấy ${JSON.stringify(steady)}`);
  assert.ok(steady.maxLead <= 0.2, `độ trễ luồng đều phải ≤ 0,2 s, thấy ${steady.maxLead}`);

  // 2. Dồn cục: 3 giây tiếng đến cùng lúc (mạng nghẽn rồi xả) ⇒ hàng đợi bị cắt, không để trễ 3 s.
  await evaluate('for (let i = 0; i < 150; i++) window.feed.enqueue(pcm(20))');
  await sleep(300);
  const burst = await evaluate('browserOutput.next - browserOutput.ctx.currentTime');
  assert.ok(burst <= 0.45, `sau khi dồn cục độ trễ phải ≤ 0,45 s, thấy ${burst}`);
});
