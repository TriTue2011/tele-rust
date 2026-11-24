// Video (và animation/video_note) hiện TRỰC TIẾP trong khung chat như Zalo, không cần bấm nút: tự tải khi tin
// nằm trong màn hình (IntersectionObserver). Thoại/nhạc vẫn giữ nút bấm thủ công (không tự phát khi lướt lịch sử).
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { mkdtempSync, readFileSync, rmSync } = require('node:fs');
const { once } = require('node:events');
const { test } = require('node:test');
const { openHttpOrigin } = require('./chrome-origin.cjs');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const emojiData = readFileSync(`${__dirname}/../web/emoji-data.js`, 'utf8');
const emojiJs = readFileSync(`${__dirname}/../web/emoji.js`, 'utf8');
const storagePolicy = { mode: 'local', keep: { private: 604800, groups: 604800, channels: null }, exceptions: [], max_cache_bytes: 2147483648, stream_ttl_s: 600,
  auto_download: Object.fromEntries(['private', 'groups', 'channels'].map((c) => [c, Object.fromEntries(['photo', 'video', 'file', 'voice', 'video_note', 'gif'].map((k) => [k, { on: false, max_bytes: 10485760 }]))])) };
const msg = (id, extra = {}) => ({ id, chat_id: 42, date: 1700000000 + id, outgoing: false, sender: 'Bình Trần', kind: 'text', text: '', entities: [], summary: '', reply_to: 0, reactions: [], file: null, ...extra });
const MP4 = 'AAAAGGZ0eXBpc29tAAACAGlzb21pZWMy'; // vài byte đầu hợp lệ của một hộp ftyp; đủ để tạo Blob video/mp4, không cần phát thật trong test

test('TC-VIDEO-INLINE: tin video hiện trực tiếp (không nút); lỗi tải thì bấm để thử lại; thoại vẫn giữ nút bấm', { timeout: 60000 }, async (t) => {
  for (const banned of ['innerHTML', 'outerHTML', 'insertAdjacentHTML', 'document.write', 'localStorage', 'sessionStorage']) assert.equal(html.includes(banned), false, banned);
  const routes = {
    '/ui/api/auth/status': { dang_nhap: true, da_thiet_lap: true },
    '/api/accounts': { tai_khoan: [{ phone: '+84900000000', status: 'online', display_name: 'An Lê' }] },
    '/api/me': { id: 7, name: 'An Lê', phone: '84900000000' },
    '/api/settings/appearance': { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' },
    '/api/settings/storage': storagePolicy, '/api/storage/usage': { total_bytes: 0, by_type: {}, top_chats: [] },
    '/api/chats': { chats: [{ id: 42, title: 'Bình Trần', kind: 'private', user_id: 9, pinned: false, muted: false }] },
    '/api/chats/42/messages': { messages: [
      msg(1, { kind: 'video', file: { id: 77, size: 123456 } }),
      msg(2, { kind: 'voice', file: { id: 88, duration: 4 } }),
      msg(3, { kind: 'video', file: { id: 99, size: 222 } }),
    ] },
    '/api/chats/42/background': { background: null },
  };
  const mock = `
    window.__calls = []; window.__fail99 = true;
    const routes = ${JSON.stringify(routes)}; const mp4 = Uint8Array.from(atob(${JSON.stringify(MP4)}), (c) => c.charCodeAt(0));
    window.fetch = async (input, init = {}) => {
      const url = new URL(typeof input === 'string' ? input : input.url, location.href), method = (init.method || 'GET').toUpperCase();
      if (url.pathname === '/api/file/77' || url.pathname === '/api/file/88') { window.__calls.push(url.pathname); return new Response(mp4, { status: 200, headers: { 'Content-Type': 'video/mp4' } }); }
      if (url.pathname === '/api/file/99') { window.__calls.push(url.pathname); if (window.__fail99) return new Response('{}', { status: 503 }); return new Response(mp4, { status: 200, headers: { 'Content-Type': 'video/mp4' } }); }
      const payload = routes[url.pathname] || {};
      return new Response(JSON.stringify(payload), { status: 200, headers: { 'Content-Type': 'application/json' } });
    };`;
  const profile = mkdtempSync(`${require('node:os').tmpdir()}/tele-video-`);
  const browser = spawn(process.env.CHROME_BIN || 'google-chrome', ['--headless', '--no-sandbox', '--disable-gpu', '--no-first-run', '--disable-extensions', '--no-proxy-server', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank']);
  t.after(async () => { if (browser.exitCode === null && browser.signalCode === null) { browser.kill(); await once(browser, 'exit'); } try { rmSync(profile, { recursive: true, force: true, maxRetries: 20, retryDelay: 150 }); } catch { /* dọn tốt nhất có thể */ } });
  let dbg = ''; const port = await new Promise((res, rej) => { browser.once('error', rej); browser.once('exit', () => rej(new Error(dbg))); browser.stderr.on('data', (c) => { dbg += c; const m = /DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)/.exec(dbg); if (m) res(m[1]); }); });
  const page = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((x) => x.type === 'page');
  const ws = new WebSocket(page.webSocketDebuggerUrl); await once(ws, 'open'); t.after(() => ws.close());
  const pending = new Map(), errors = []; let seq = 0;
  ws.addEventListener('message', ({ data }) => { const m = JSON.parse(data); if (m.method === 'Runtime.exceptionThrown') errors.push(m.params.exceptionDetails.text); if (m.id) { pending.get(m.id)(m); pending.delete(m.id); } });
  const cdp = async (method, params = {}) => { const id = ++seq; const m = await new Promise((r) => { pending.set(id, r); ws.send(JSON.stringify({ id, method, params })); }); assert.equal(m.error, undefined, JSON.stringify(m.error)); return m.result; };
  const evaluate = async (expression) => { const r = await cdp('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }); assert.equal(r.exceptionDetails, undefined, JSON.stringify(r.exceptionDetails)); return r.result.value; };
  const until = async (expr) => { for (let n = 0; n < 160; n++) { if (await evaluate(expr)) return; await new Promise((r) => setTimeout(r, 25)); } assert.fail(`${expr}: ${JSON.stringify(errors)}`); };
  await cdp('Runtime.enable');
  await cdp('Emulation.setDeviceMetricsOverride', { width: 1100, height: 800, deviceScaleFactor: 1, mobile: false });
  const script = /<script>([\s\S]*?)<\/script>/.exec(html);
  await openHttpOrigin(cdp, ws, html.replace(script[0], '').replace(/<script src="\/ui\/emoji[^>]*><\/script>/g, ''));
  await evaluate(emojiData + ';' + emojiJs + ';' + mock + script[1]);
  await until('Boolean(document.querySelector("#list .chat"))');
  await evaluate('document.querySelectorAll("#list .chat")[0].click()');
  await until('document.querySelectorAll("#msgs .m").length === 3');

  // tin video (msg 1): KHÔNG có nút, tự hiện <video> thật — không cần bấm gì cả
  await until('Boolean(document.querySelectorAll("#msgs .m")[0].querySelector("video"))');
  assert.equal(await evaluate('Boolean(document.querySelectorAll("#msgs .m")[0].querySelector(".lazy"))'), false, 'video không còn khung .lazy kiểu nút bấm cũ');
  assert.match(await evaluate('document.querySelectorAll("#msgs .m")[0].querySelector("video").src'), /^blob:/);

  // tin video thứ hai (msg 3): máy chủ lỗi ⇒ tự hiện «Thử lại» (không phải video), không cần ai bấm gì để BIẾT lỗi
  await until('/Thử lại/.test(document.querySelectorAll("#msgs .m")[2].textContent)');
  assert.equal(await evaluate('Boolean(document.querySelectorAll("#msgs .m")[2].querySelector("video"))'), false);
  assert.deepEqual(await evaluate('window.__calls.slice().sort()'), ['/api/file/77', '/api/file/99'], 'cả hai video trong màn hình đều tự tải (99 lỗi)');

  // tin thoại (msg 2): vẫn giữ nút «▶ Nghe», CHƯA tự tải
  assert.equal(await evaluate('Boolean(document.querySelectorAll("#msgs .m")[1].querySelector("audio"))'), false, 'thoại chưa tự phát');
  assert.equal(await evaluate('document.querySelectorAll("#msgs .m")[1].querySelector("button").textContent'), '▶ Nghe');
  await evaluate('window.__calls.length = 0; document.querySelectorAll("#msgs .m")[1].querySelector("button").click()');
  await until('Boolean(document.querySelectorAll("#msgs .m")[1].querySelector("audio"))');
  assert.deepEqual(await evaluate('window.__calls'), ['/api/file/88'], 'bấm nút mới tải thoại');

  // bấm vào «Thử lại» của msg 3 ⇒ thử lại, máy chủ hết lỗi ⇒ hiện video thật
  await evaluate('window.__fail99 = false; window.__calls.length = 0; document.querySelectorAll("#msgs .m")[2].querySelector(".vidph").click()');
  await until('Boolean(document.querySelectorAll("#msgs .m")[2].querySelector("video"))');
  assert.deepEqual(await evaluate('window.__calls'), ['/api/file/99'], 'thử lại gọi đúng tệp');

  assert.deepEqual(errors, [], 'không có lỗi JS');
});
