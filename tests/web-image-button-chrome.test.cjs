// Nút «Gửi ảnh» (🖼️) riêng bên cạnh nút đính kèm chung (📎), như Zalo (btn-img/btn-attach): lọc sẵn image/*,
// bấm thì mở hộp chọn ảnh ngay, không phải lọc trong danh sách mọi loại tệp.
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
const msg = (id, extra = {}) => ({ id, chat_id: 42, date: 1700000000 + id, outgoing: false, sender: 'Bình Trần', kind: 'text', text: 'Tin số ' + id, entities: [], summary: 'Tin số ' + id, reply_to: 0, reactions: [], file: null, ...extra });

test('TC-IMG-BTN: nút «Ảnh/Video» riêng (accept image/* + video/*), tách khỏi nút đính kèm chung; cả hai cùng gửi qua send_file đúng kind', { timeout: 60000 }, async (t) => {
  for (const banned of ['innerHTML', 'outerHTML', 'insertAdjacentHTML', 'document.write', 'localStorage', 'sessionStorage']) assert.equal(html.includes(banned), false, banned);
  const routes = {
    '/ui/api/auth/status': { dang_nhap: true, da_thiet_lap: true },
    '/api/accounts': { tai_khoan: [{ phone: '+84900000000', status: 'online', display_name: 'An Lê' }] },
    '/api/me': { id: 7, name: 'An Lê', phone: '84900000000' },
    '/api/settings/appearance': { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' },
    '/api/settings/storage': storagePolicy, '/api/storage/usage': { total_bytes: 0, by_type: {}, top_chats: [] },
    '/api/chats': { chats: [{ id: 42, title: 'Bình Trần', kind: 'private', user_id: 9, pinned: false, muted: false }] },
    '/api/chats/42/messages': { messages: [msg(1)] },
    '/api/chats/42/background': { background: null },
  };
  const mock = `
    window.__calls = [];
    const routes = ${JSON.stringify(routes)};
    window.fetch = async (input, init = {}) => {
      const url = new URL(typeof input === 'string' ? input : input.url, location.href), method = (init.method || 'GET').toUpperCase();
      if (url.pathname === '/api/chats/42/send_file') { window.__calls.push({ p: url.pathname, kind: url.searchParams.get('kind'), name: url.searchParams.get('name'), len: init.body.byteLength }); return new Response('{}', { status: 200, headers: { 'Content-Type': 'application/json' } }); }
      const payload = routes[url.pathname] || {};
      return new Response(JSON.stringify(payload), { status: 200, headers: { 'Content-Type': 'application/json' } });
    };`;
  const profile = mkdtempSync(`${require('node:os').tmpdir()}/tele-imgbtn-`);
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
  const script = /<script>([\s\S]*?)<\/script>/.exec(html);
  await openHttpOrigin(cdp, ws, html.replace(script[0], '').replace(/<script src="\/ui\/emoji[^>]*><\/script>/g, ''));
  await evaluate(emojiData + ';' + emojiJs + ';' + mock + script[1]);
  await until('Boolean(document.querySelector("#list .chat"))');
  await evaluate('document.querySelectorAll("#list .chat")[0].click()');
  await until('!document.getElementById("composer").hidden');

  // nút ảnh riêng, tách khỏi nút đính kèm chung, lọc sẵn image/*
  assert.notEqual(await evaluate('document.getElementById("img-btn")'), null, 'phải có nút «Gửi ảnh» riêng');
  // Chủ máy 26/11: nút Ảnh/Video như Zalo — nhận ảnh VÀ video (video gửi thành video xem được), vẫn tách khỏi nút tệp chung.
  const accept = await evaluate('document.getElementById("img-file").accept');
  assert.ok(accept.split(',').includes('image/*') && accept.split(',').includes('video/*'), 'nút Ảnh/Video nhận cả image/* và video/*: ' + accept);
  assert.equal(await evaluate('document.getElementById("file").accept'), '', 'input đính kèm chung không lọc loại tệp');

  // bấm nút ảnh mở đúng input ẩn tương ứng (không mở input tệp chung)
  await evaluate(`
    window.__clicked = [];
    document.getElementById("img-file").addEventListener("click", () => window.__clicked.push("img-file"));
    document.getElementById("file").addEventListener("click", () => window.__clicked.push("file"));
    document.getElementById("img-btn").click();
  `);
  assert.deepEqual(await evaluate('window.__clicked'), ['img-file']);
  await evaluate('document.getElementById("attach").click()');
  assert.deepEqual(await evaluate('window.__clicked'), ['img-file', 'file']);

  // gửi qua nút ảnh: dùng chung đường send_file, kind=photo đúng theo mime
  await evaluate(`
    const dt = new DataTransfer(); dt.items.add(new File([new Uint8Array([1,2,3,4,5])], "p.png", { type: "image/png" }));
    const i = document.getElementById("img-file"); i.files = dt.files; i.dispatchEvent(new Event("change"));
  `);
  // Như Zalo: chọn xong vào KHAY chờ (xem trước, gõ chú thích), bấm Gửi mới gửi.
  await until('document.getElementById("attach-tray") && !document.getElementById("attach-tray").hidden');
  await evaluate('document.getElementById("send").click()');
  await until('window.__calls.some((c) => c.p === "/api/chats/42/send_file")');
  const sent = await evaluate('window.__calls.find((c) => c.p === "/api/chats/42/send_file")');
  assert.equal(sent.kind, 'photo'); assert.equal(sent.name, 'p.png'); assert.equal(sent.len, 5);

  assert.deepEqual(errors, [], 'không có lỗi JS');
});
