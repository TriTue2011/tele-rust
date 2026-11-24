const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { mkdtempSync, readFileSync, rmSync } = require('node:fs');
const { once } = require('node:events');
const { test } = require('node:test');
const { openHttpOrigin } = require('./chrome-origin.cjs');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const emojiData = readFileSync(`${__dirname}/../web/emoji-data.js`, 'utf8');
const emojiJs = readFileSync(`${__dirname}/../web/emoji.js`, 'utf8');
const artJs = readFileSync(`${__dirname}/../web/wallpaper-art.js`, 'utf8');
const storagePolicy = { mode: 'local', keep: { private: 604800, groups: 604800, channels: null }, exceptions: [], max_cache_bytes: 2147483648, stream_ttl_s: 600,
  auto_download: Object.fromEntries(['private', 'groups', 'channels'].map((c) => [c, Object.fromEntries(['photo', 'video', 'file', 'voice', 'video_note', 'gif'].map((k) => [k, { on: false, max_bytes: 10485760 }]))])) };
const msg = (id, extra = {}) => ({ id, chat_id: 42, date: 1700000000 + id, outgoing: false, sender: 'Bình Trần', kind: 'text', text: 'Tin số ' + id, entities: [], summary: 'Tin số ' + id, reply_to: 0, reactions: [], file: null, ...extra });


const PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';
const tmsg = (id, chat, extra = {}) => ({ id, chat_id: chat, date: 1700000000 + id, outgoing: false, sender: 'Bình Trần', kind: 'text', text: 'Tin số ' + id, entities: [], summary: 'Tin số ' + id, reply_to: 0, reactions: [], file: null, ...extra });

test('TC-WALLPAPER-SHARED: kho hình nền chung Zalo↔Tele — tab Mẫu chung + Biểu tượng vẽ ảnh JPEG rồi tải lên; tên mã vô nghĩa không hiện', { timeout: 60000 }, async (t) => {
  for (const banned of ['innerHTML', 'outerHTML', 'insertAdjacentHTML', 'document.write', 'localStorage', 'sessionStorage']) assert.equal(html.includes(banned), false, banned);
  const routes = {
    '/ui/api/auth/status': { dang_nhap: true, da_thiet_lap: true },
    '/api/accounts': { tai_khoan: [{ phone: '+84900000000', status: 'online', display_name: 'An Lê' }] },
    '/api/me': { id: 7, name: 'An Lê', phone: '84900000000' },
    '/api/settings/appearance': { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' },
    '/api/settings/storage': storagePolicy, '/api/storage/usage': { total_bytes: 0, by_type: {}, top_chats: [] },
    '/api/chats': { chats: [{ id: 42, title: 'Bình Trần', kind: 'private', user_id: 9, pinned: false, muted: false }, { id: -500, title: 'Nhóm Bạn', kind: 'group', member_count: 5, pinned: false, muted: false }, { id: 43, title: 'Chưa có nền', kind: 'private', user_id: 11, pinned: false, muted: false }] },
    '/api/chats/42/messages': { messages: [tmsg(1, 42)] }, '/api/chats/-500/messages': { messages: [tmsg(5, -500)] }, '/api/chats/43/messages': { messages: [tmsg(9, 43)] },
  };
  const mock = `
    window.__calls = []; window.__failSet = false;
    const routes = ${JSON.stringify(routes)}; const png = Uint8Array.from(atob(${JSON.stringify(PNG)}), (c) => c.charCodeAt(0));
    const gallery = [{ id: '333', name: 'aHrFlr08CFILAAAAkrXGwsGme_YFIJAAAA', kind: 'wallpaper', thumb_file: 8, file: 9, dark: false }, { id: '111', name: 'Núi', kind: 'wallpaper', thumb_file: 8, file: 9, dark: false }, { id: '222', name: 'Hoàng hôn', kind: 'fill', fill: { kind: 'gradient', colors: ['#ff0000', '#0000ff'], angle: 90 } }];
    window.fetch = async (input, init = {}) => {
      const url = new URL(typeof input === 'string' ? input : input.url, location.href), method = (init.method || 'GET').toUpperCase(); let payload = routes[url.pathname] || {}, status = 200;
      if (url.pathname.startsWith('/api/file/')) return new Response(png, { status: 200, headers: { 'Content-Type': 'image/png' } });
      if (url.pathname === '/api/backgrounds') { window.__calls.push({ p: url.pathname, dark: url.searchParams.get('dark') }); payload = { backgrounds: url.searchParams.get('dark') === '1' ? [gallery[0]] : gallery }; }
      else if (url.pathname === '/api/wallpapers/upload') { window.__calls.push({ p: url.pathname, m: method, len: init.body.byteLength, jpeg: new Uint8Array(init.body)[0] === 0xFF && new Uint8Array(init.body)[1] === 0xD8 }); payload = { ok: true, file: 'ab'.repeat(16) + '.png' }; }
      else if (/^\\/api\\/chats\\/-?\\d+\\/background$/.test(url.pathname)) {
        if (method === 'GET' && url.pathname === '/api/chats/42/background' && window.__slow42) await new Promise((r) => setTimeout(r, 400));
        if (method === 'GET') payload = url.pathname === '/api/chats/42/background' ? { background: { dimming: 40, background: gallery[0] } } : { background: null };
        else { window.__calls.push({ p: url.pathname, m: method, body: JSON.parse(init.body) }); if (window.__failSet) { status = 400; payload = { error: { code: 'BG', message: 'Telegram từ chối: cần nâng cấp nhóm.' } }; } else payload = { ok: true }; }
      } else if (url.pathname === '/api/feed') { await new Promise((r) => setTimeout(r, 400)); }
      return new Response(JSON.stringify(payload), { status, headers: { 'Content-Type': 'application/json' } });
    };`;
  const profile = mkdtempSync(`${require('node:os').tmpdir()}/tele-react-`);
  const browser = spawn(process.env.CHROME_BIN || 'google-chrome', ['--headless', '--no-sandbox', '--disable-gpu', '--no-first-run', '--disable-extensions', '--no-proxy-server', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank']);
  t.after(async () => { if (browser.exitCode === null && browser.signalCode === null) { browser.kill(); await once(browser, 'exit'); } try { rmSync(profile, { recursive: true, force: true, maxRetries: 20, retryDelay: 150 }); } catch { /* dọn tạm */ } });
  let dbg = ''; const port = await new Promise((res, rej) => { browser.once('error', rej); browser.once('exit', () => rej(new Error(dbg))); browser.stderr.on('data', (c) => { dbg += c; const m = /DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)/.exec(dbg); if (m) res(m[1]); }); });
  const page = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((x) => x.type === 'page');
  const ws = new WebSocket(page.webSocketDebuggerUrl); await once(ws, 'open'); t.after(() => ws.close());
  const pending = new Map(), errors = []; let seq = 0;
  ws.addEventListener('message', ({ data }) => { const m = JSON.parse(data); if (m.method === 'Runtime.exceptionThrown') errors.push(m.params.exceptionDetails.text + ' ' + JSON.stringify(m.params.exceptionDetails.exception && m.params.exceptionDetails.exception.description)); if (m.id) { const r = pending.get(m.id); pending.delete(m.id); r(m); } });
  const cdp = async (method, params = {}) => { const id = ++seq; const m = await new Promise((r) => { pending.set(id, r); ws.send(JSON.stringify({ id, method, params })); }); assert.equal(m.error, undefined, JSON.stringify(m.error)); return m.result; };
  const evaluate = async (expression) => { const r = await cdp('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }); assert.equal(r.exceptionDetails, undefined, JSON.stringify(r.exceptionDetails)); return r.result.value; };
  const until = async (expr) => { for (let n = 0; n < 160; n++) { if (await evaluate(expr)) return; await new Promise((r) => setTimeout(r, 25)); } assert.fail(`${expr}: ${JSON.stringify(errors)}`); };
  await cdp('Runtime.enable');
  await cdp('Emulation.setDeviceMetricsOverride', { width: 1100, height: 800, deviceScaleFactor: 1, mobile: false });
  const script = /<script>([\s\S]*?)<\/script>/.exec(html);
  await openHttpOrigin(cdp, ws, html.replace(script[0], '').replace(/<script src="\/ui\/(emoji|wallpaper-art)[^>]*><\/script>/g, ''));
  await evaluate(emojiData + ';' + emojiJs + ';' + artJs + ';' + mock + script[1]);
  await until('Boolean(document.querySelector("#list .chat"))');
  const openMenu = async (index) => { await evaluate(`document.querySelectorAll('#list .chat')[${index}].dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 50, clientY: 50 }))`); await until('!document.getElementById("chat-ctx-menu").hidden'); };
  await evaluate('document.querySelectorAll("#list .chat")[0].click()');
  await until('document.getElementById("msgs").classList.contains("wp")');
  await openMenu(0); await evaluate('document.getElementById("ctx-wallpaper").click()');
  await until('Boolean(document.getElementById("wp-dialog")) && document.querySelectorAll(".wpd-t").length >= 4');
  // tên mã vô nghĩa (aHrFlr08…) không hiện làm chú thích; tên thường vẫn hiện
  assert.deepEqual(await evaluate('[...document.querySelectorAll(".wpd-t.bg .wpd-cap")].map((c) => c.textContent)'), ['Núi', 'Hoàng hôn'], 'chú thích mã vô nghĩa bị ẩn');
  assert.deepEqual(await evaluate('[...document.querySelectorAll(".wpd-tabs button")].map((b) => b.textContent)'), ['Bộ sưu tập', 'Màu', 'Chuyển màu', 'Mẫu chung', 'Biểu tượng']);
  // tab «Mẫu chung»: 30 mẫu vẽ + 8 chuyển màu, mỗi ô có tên; chọn một mẫu ⇒ vẽ JPEG và tải lên rồi xem trước
  await evaluate('document.querySelectorAll(".wpd-tabs button")[3].click()');
  await until('document.querySelectorAll("#wp-dialog .wpd-t.bg").length === 38');
  assert.equal(await evaluate('[...document.querySelectorAll("#wp-dialog .wpd-t.bg .wpd-cap")].every((c) => c.textContent.length > 0)'), true, 'mọi mẫu chung có tên');
  assert.equal(await evaluate('window.__calls.some((c) => c.p === "/api/wallpapers/upload")'), false, 'chưa chọn thì chưa tải gì');
  await evaluate('document.querySelectorAll("#wp-dialog .wpd-t.bg")[2].click()');
  await until('window.__calls.some((c) => c.p === "/api/wallpapers/upload")');
  const up = await evaluate('window.__calls.find((c) => c.p === "/api/wallpapers/upload")');
  assert.equal(up.jpeg, true, 'mẫu được vẽ thành ảnh JPEG thật'); assert.ok(up.len > 5000, 'ảnh có nội dung: ' + up.len);
  await until('document.querySelector(".wpd-preview").style.background.includes("blob:")');
  // mẫu chuyển màu cũng thành ảnh
  await evaluate('window.__calls.length = 0; document.querySelectorAll("#wp-dialog .wpd-t.bg")[31].click()');
  await until('window.__calls.some((c) => c.p === "/api/wallpapers/upload")');
  assert.equal(await evaluate('window.__calls.find((c) => c.p === "/api/wallpapers/upload").jpeg'), true);
  // tab «Biểu tượng»: 12 biểu tượng + màu nền; chọn ⇒ ảnh lát biểu tượng
  await evaluate('window.__calls.length = 0; document.querySelectorAll(".wpd-tabs button")[4].click()');
  await until('document.querySelectorAll("#wp-dialog .wpd-t.bg").length === 12');
  await evaluate('document.querySelectorAll("#wp-dialog .wpd-t.bg")[1].click()');
  await until('window.__calls.some((c) => c.p === "/api/wallpapers/upload")');
  assert.equal(await evaluate('window.__calls.find((c) => c.p === "/api/wallpapers/upload").jpeg'), true, 'nền biểu tượng cũng là JPEG');
  // Lưu ⇒ đặt đúng ảnh vừa tải làm hình nền hội thoại
  await evaluate('window.__calls.length = 0; document.getElementById("wpd-save").click()');
  await until('window.__calls.some((c) => /\\/background$/.test(c.p) && c.m === "POST")');
  assert.equal(await evaluate('window.__calls.find((c) => /\\/background$/.test(c.p) && c.m === "POST").body.file_path'), 'ab'.repeat(16) + '.png');
});
