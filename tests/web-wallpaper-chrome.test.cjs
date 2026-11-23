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


const PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';
const tmsg = (id, chat, extra = {}) => ({ id, chat_id: chat, date: 1700000000 + id, outgoing: false, sender: 'Bình Trần', kind: 'text', text: 'Tin số ' + id, entities: [], summary: 'Tin số ' + id, reply_to: 0, reactions: [], file: null, ...extra });

test('TC-WALLPAPER: hình nền hội thoại theo Telegram — áp dụng, bộ sưu tập, màu, chuyển màu, ảnh tự gán, cả hai bên, độ tối, bỏ, lỗi', { timeout: 60000 }, async (t) => {
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
    const gallery = [{ id: '111', name: 'Núi', kind: 'wallpaper', thumb_file: 8, file: 9, dark: false }, { id: '222', name: 'Hoàng hôn', kind: 'fill', fill: { kind: 'gradient', colors: ['#ff0000', '#0000ff'], angle: 90 } }];
    window.fetch = async (input, init = {}) => {
      const url = new URL(typeof input === 'string' ? input : input.url, location.href), method = (init.method || 'GET').toUpperCase(); let payload = routes[url.pathname] || {}, status = 200;
      if (url.pathname.startsWith('/api/file/')) return new Response(png, { status: 200, headers: { 'Content-Type': 'image/png' } });
      if (url.pathname === '/api/backgrounds') { window.__calls.push({ p: url.pathname, dark: url.searchParams.get('dark') }); payload = { backgrounds: url.searchParams.get('dark') === '1' ? [gallery[0]] : gallery }; }
      else if (url.pathname === '/api/wallpapers/upload') { window.__calls.push({ p: url.pathname, m: method, len: init.body.byteLength }); payload = { ok: true, file: 'ab'.repeat(16) + '.png' }; }
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
  await openHttpOrigin(cdp, ws, html.replace(script[0], '').replace(/<script src="\/ui\/emoji[^>]*><\/script>/g, ''));
  await evaluate(emojiData + ';' + emojiJs + ';' + mock + script[1]);
  await until('Boolean(document.querySelector("#list .chat"))');
  const openMenu = async (index) => { await evaluate(`document.querySelectorAll('#list .chat')[${index}].dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 50, clientY: 50 }))`); await until('!document.getElementById("chat-ctx-menu").hidden'); };

  // 1) mở hội thoại có nền (ảnh Telegram, độ tối 40): ban ngày KHÔNG làm tối; có lớp ảnh
  await evaluate('document.querySelectorAll("#list .chat")[0].click()');
  await until('document.getElementById("msgs").classList.contains("wp")');
  assert.match(await evaluate('document.getElementById("msgs").style.background'), /blob:/, 'nền ảnh nạp từ tệp Telegram');
  assert.equal(await evaluate('document.getElementById("msgs").style.background.includes("rgba(0, 0, 0")'), false, 'ban ngày không phủ tối');
  // ban đêm: phủ tối theo dimming
  await evaluate('document.documentElement.dataset.theme = "dark"; document.querySelectorAll("#list .chat")[2].click()');
  await until('!document.getElementById("msgs").classList.contains("wp")');
  assert.equal(await evaluate('document.getElementById("msgs").style.background'), '', 'hội thoại không có nền ⇒ về nền mặc định');
  await evaluate('document.querySelectorAll("#list .chat")[0].click()');
  await until('document.getElementById("msgs").classList.contains("wp")');
  assert.match(await evaluate('document.getElementById("msgs").style.background'), /rgba\(0, 0, 0, 0\.4\)/, 'ban đêm phủ tối 40%');
  await evaluate('document.documentElement.dataset.theme = "system"');

  // 2) mở hộp từ menu ⋮ → có ô máy ảnh, ô «không có», 2 hình nền; cả hai bên mặc định bật (1-1)
  await openMenu(0); await evaluate('document.getElementById("ctx-wallpaper").click()');
  await until('Boolean(document.getElementById("wp-dialog")) && document.querySelectorAll(".wpd-t").length === 4');
  assert.deepEqual(await evaluate('[...document.querySelectorAll(".wpd-t")].map((b) => b.getAttribute("aria-label"))'), ['Chọn ảnh của tôi', 'Không có hình nền', 'Núi', 'Hoàng hôn']);
  assert.equal(await evaluate('document.getElementById("wpd-both").checked'), true);
  assert.equal(await evaluate('document.getElementById("wpd-dim").value'), '40', 'độ tối hiện tại nạp từ Telegram');
  await until('document.querySelectorAll(".wpd-t.bg")[0].style.backgroundImage.includes("blob:")');
  await evaluate('document.querySelectorAll(".wpd-t.bg")[1].click()');
  assert.match(await evaluate('document.querySelector(".wpd-preview").style.background'), /linear-gradient/, 'xem trước đổi ngay');
  await evaluate('document.getElementById("wpd-dim").value = "55"; document.getElementById("wpd-dim").dispatchEvent(new Event("input")); document.getElementById("wpd-both").click(); document.getElementById("wpd-save").click()');
  await until('window.__calls.some((c) => c.m === "POST" && /background$/.test(c.p))');
  assert.deepEqual(await evaluate('window.__calls.find((c) => c.m === "POST" && /background$/.test(c.p)).body'), { dimming: 55, for_both: false, background_id: '222' });
  await until('!document.getElementById("wp-dialog")');

  // 3) ban đêm xem bộ sưu tập ban đêm; màu và chuyển màu gửi đúng thân
  await evaluate('window.__calls.length = 0'); await openMenu(0); await evaluate('document.getElementById("ctx-wallpaper").click()');
  await until('Boolean(document.getElementById("wp-dialog")) && document.querySelectorAll(".wpd-t").length === 4');
  await evaluate('document.getElementById("wpd-night").click()');
  await until('document.querySelectorAll(".wpd-t").length === 3');
  assert.equal(await evaluate('window.__calls.filter((c) => c.p === "/api/backgrounds").at(-1).dark'), '1');
  await evaluate('document.querySelectorAll(".wpd-tabs button")[1].click(); document.querySelectorAll(".wpd-sw button")[2].click(); document.getElementById("wpd-save").click()');
  await until('window.__calls.some((c) => c.m === "POST")');
  assert.deepEqual(await evaluate('window.__calls.find((c) => c.m === "POST").body'), { dimming: 40, for_both: true, color: '#e3f5ea' });
  await evaluate('window.__calls.length = 0'); await openMenu(0); await evaluate('document.getElementById("ctx-wallpaper").click()');
  await until('Boolean(document.getElementById("wp-dialog"))');
  await evaluate('document.querySelectorAll(".wpd-tabs button")[2].click()');
  await evaluate('(() => { const g = document.querySelector(\'input[aria-label="Góc"]\'); g.value = "90"; g.dispatchEvent(new Event("input")); })(); document.getElementById("wpd-save").click()');
  await until('window.__calls.some((c) => c.m === "POST")');
  assert.deepEqual(await evaluate('window.__calls.find((c) => c.m === "POST").body.gradient'), { from: '#a1c4fd', to: '#c2e9fb', angle: 90 });

  // 4) ảnh tự gán: nén + tải lên thân thô, rồi Lưu gửi file_path
  await evaluate('window.__calls.length = 0'); await openMenu(0); await evaluate('document.getElementById("ctx-wallpaper").click()');
  await until('Boolean(document.getElementById("wpd-file"))');
  await evaluate(`(async () => { const c = document.createElement('canvas'); c.width = 60; c.height = 40; c.getContext('2d').fillRect(0, 0, 60, 40); const b = await new Promise((r) => c.toBlob(r, 'image/png'));
    const dt = new DataTransfer(); dt.items.add(new File([b], 'p.png', { type: 'image/png' })); const i = document.getElementById('wpd-file'); i.files = dt.files; i.dispatchEvent(new Event('change')); })()`);
  await until('window.__calls.some((c) => c.p === "/api/wallpapers/upload")');
  assert.ok(await evaluate('window.__calls.find((c) => c.p === "/api/wallpapers/upload").len') > 50, 'tải thân thô có nội dung ảnh');
  await until('document.querySelector(".wpd-preview").style.background.includes("blob:")');
  await evaluate('document.getElementById("wpd-save").click()');
  await until('window.__calls.some((c) => c.m === "POST" && /background$/.test(c.p))');
  assert.equal(await evaluate('window.__calls.find((c) => c.m === "POST" && /background$/.test(c.p)).body.file_path'), 'ab'.repeat(16) + '.png');
  // ảnh sai loại: báo lỗi, không tải
  await evaluate('window.__calls.length = 0'); await openMenu(0); await evaluate('document.getElementById("ctx-wallpaper").click()');
  await until('Boolean(document.getElementById("wpd-file"))');
  await evaluate(`const dt = new DataTransfer(); dt.items.add(new File(['x'], 'a.gif', { type: 'image/gif' })); const i = document.getElementById('wpd-file'); i.files = dt.files; i.dispatchEvent(new Event('change'));`);
  await until('/JPEG hoặc PNG/.test(document.querySelector(".wpd-msg").textContent)');
  assert.equal(await evaluate('window.__calls.some((c) => c.p === "/api/wallpapers/upload")'), false);

  // 5) lưu khi chưa chọn gì ⇒ nhắc; lỗi từ Telegram ⇒ hiện lỗi, giữ hộp; «Bỏ hình nền» gửi remove; Esc đóng
  await evaluate('document.getElementById("wpd-cancel").click()'); await evaluate('window.__calls.length = 0'); await openMenu(2); await evaluate('document.getElementById("ctx-wallpaper").click()');
  await until('Boolean(document.getElementById("wp-dialog"))');
  await evaluate('document.getElementById("wpd-save").click()');
  assert.match(await evaluate('document.querySelector(".wpd-msg").textContent'), /chọn một hình nền/);
  assert.equal(await evaluate('window.__calls.some((c) => c.m === "POST")'), false);
  await evaluate('window.__failSet = true; document.querySelectorAll(".wpd-t.bg")[1].click(); document.getElementById("wpd-save").click()');
  await until('/Telegram từ chối/.test(document.querySelector(".wpd-msg").textContent)');
  assert.equal(await evaluate('Boolean(document.getElementById("wp-dialog"))'), true, 'lỗi ⇒ giữ hộp để thử lại');
  await evaluate('window.__failSet = false; window.__calls.length = 0; document.getElementById("wpd-clear").click()');
  await until('window.__calls.some((c) => c.m === "POST")');
  assert.deepEqual(await evaluate('window.__calls.find((c) => c.m === "POST").body'), { remove: true, restore_previous: false });
  await until('!document.getElementById("wp-dialog")');
  await openMenu(2); await evaluate('document.getElementById("ctx-wallpaper").click()'); await until('Boolean(document.getElementById("wp-dialog"))');
  await evaluate('document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))');
  assert.equal(await evaluate('document.getElementById("wp-dialog")'), null, 'Esc đóng');

  // 6) nhóm: không có ô «cả hai bên», for_both luôn false
  await evaluate('window.__calls.length = 0; document.querySelectorAll("#list .chat")[1].click()');
  await until('document.querySelectorAll("#msgs .m").length === 1 && document.getElementById("title").textContent === "Nhóm Bạn"');
  await evaluate('document.getElementById("chat-menu-open").click()'); await until('!document.getElementById("chat-ctx-menu").hidden');
  await evaluate('document.getElementById("ctx-wallpaper").click()'); await until('Boolean(document.getElementById("wp-dialog"))');
  assert.equal(await evaluate('document.getElementById("wpd-both")'), null);
  await evaluate('document.querySelectorAll(".wpd-t.bg")[0].click(); document.getElementById("wpd-save").click()');
  await until('window.__calls.some((c) => c.m === "POST")');
  assert.deepEqual(await evaluate('window.__calls.find((c) => c.m === "POST").body'), { dimming: 0, for_both: false, background_id: '111' });
  // 7) chống chạy đua: nền của hội thoại A trả về MUỘN sau khi đã sang hội thoại B không được áp vào B
  await evaluate('window.__slow42 = true; document.querySelectorAll("#list .chat")[0].click()');
  await evaluate('document.querySelectorAll("#list .chat")[2].click()');
  await until('document.getElementById("title").textContent === "Chưa có nền" && document.querySelectorAll("#msgs .m").length === 1');
  await new Promise((r) => setTimeout(r, 800));
  assert.equal(await evaluate('document.getElementById("msgs").classList.contains("wp")'), false, 'nền muộn của hội thoại trước không được áp sang hội thoại sau');
  await evaluate('window.__slow42 = false');
  // 8) đổi ngày/đêm trong hộp ⇒ bỏ lựa chọn đang giữ (hình nền ngày/đêm là hai bộ khác nhau)
  await openMenu(2); await evaluate('document.getElementById("ctx-wallpaper").click()'); await until('document.querySelectorAll(".wpd-t").length >= 3');
  await evaluate('document.querySelectorAll(".wpd-t.bg")[0].click()'); await until('document.getElementById("wpd-night") !== null');
  await evaluate('document.getElementById("wpd-night").click()'); await until('window.__calls.filter((c) => c.p === "/api/backgrounds").length >= 1');
  await evaluate('document.getElementById("wpd-save").click()');
  assert.match(await evaluate('document.querySelector(".wpd-msg").textContent'), /chọn một hình nền/, 'đổi ngày/đêm xoá lựa chọn cũ');
  assert.deepEqual(errors, [], 'không có lỗi JS');
});
