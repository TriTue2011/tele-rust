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

test('TC-REACTIONS: danh sách cảm xúc theo hội thoại, nhấp đúp, gỡ cảm xúc, lỗi/rỗng, bảng emoji khi soạn tin', { timeout: 40000 }, async (t) => {
  assert.equal(html.includes('"👍", "Thích"'), false, 'không còn nút 👍 gắn cứng');
  for (const banned of ['innerHTML', 'outerHTML', 'insertAdjacentHTML', 'document.write', 'localStorage', 'sessionStorage']) { assert.equal(html.includes(banned), false, banned); assert.equal(emojiJs.includes(banned + '('), false, 'emoji.js: ' + banned); }
  const routes = {
    '/ui/api/auth/status': { dang_nhap: true, da_thiet_lap: true },
    '/api/accounts': { tai_khoan: [{ phone: '+84900000000', status: 'online', display_name: 'An Lê' }] },
    '/api/me': { id: 7, name: 'An Lê', phone: '84900000000' },
    '/api/settings/appearance': { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' },
    '/api/settings/storage': storagePolicy, '/api/storage/usage': { total_bytes: 0, by_type: {}, top_chats: [] },
    '/api/chats': { chats: [{ id: 42, title: 'Bình Trần', kind: 'private', user_id: 9, pinned: false, muted: false }, { id: 43, title: 'Nhóm Không Cảm Xúc', kind: 'private', user_id: 10, pinned: false, muted: false }] },
    '/api/chats/42/messages': { messages: [msg(1), msg(2, { reactions: [{ emoji: '🔥', count: 2, mine: true }] }), msg(3)] },
    '/api/chats/43/messages': { messages: [msg(7, { chat_id: 43 }), msg(8, { chat_id: 43 })] },
  };
  const mock = `
    window.__calls = []; window.__reactResp = { top: ['❤️', '👍', '👎', '🔥', '🥰', '👏', '😁', '🎉'], recent: ['😢', '👍'], popular: ['🤩', '❤️'], allow_custom_emoji: false };
    const routes = ${JSON.stringify(routes)}; const nativeFetch = window.fetch.bind(window);
    window.fetch = async (input, init = {}) => {
      const url = new URL(typeof input === 'string' ? input : input.url, location.href), method = (init.method || 'GET').toUpperCase();
      let payload = routes[url.pathname] || {}, status = 200;
      if (url.pathname === '/api/messages/reactions') {
        window.__calls.push({ m: method, p: url.pathname, q: url.search });
        if (url.searchParams.get('chat_id') === '43') payload = { top: [], recent: [], popular: [], allow_custom_emoji: false };
        else if (url.searchParams.get('message_id') === '3') { status = 400; payload = { error: { code: 'BAD', message: 'Tin này không thả cảm xúc được.' } }; }
        else payload = window.__reactResp;
      } else if (url.pathname === '/api/messages/react') { window.__calls.push({ m: method, p: url.pathname, body: JSON.parse(init.body) }); payload = { ok: true }; }
      else if (url.pathname === '/api/feed') { await new Promise((r) => setTimeout(r, 400)); }
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
  await evaluate('document.querySelector("#list .chat").click()');
  await until('document.querySelectorAll("#msgs .m").length === 3');
  const acts = await evaluate(`[...document.querySelectorAll('#msgs .m')[0].querySelectorAll('.acts button')].map((b) => b.getAttribute('aria-label'))`);
  assert.ok(acts.includes('Thả cảm xúc') && !acts.includes('Thích'), 'thanh công cụ có «Thả cảm xúc», không còn «Thích»: ' + acts);

  // BỐ CỤC: chip cảm xúc dưới tin là viên thuốc NHỎ ôm sát «🔥 2» (CSS .rx của bảng chọn từng đè lên và kéo chip rộng hết bong bóng).
  const chip = await evaluate(`(() => { const c = document.querySelector('#msgs .m .rx span'), w = c.closest('.m').getBoundingClientRect().width; return { chip: c.getBoundingClientRect().width, bubble: w, grid: getComputedStyle(c.parentElement).display }; })()`);
  assert.equal(chip.grid, 'flex', 'hàng chip cảm xúc là flex, không phải grid của bảng chọn');
  assert.ok(chip.chip < 80, 'chip ôm sát nội dung (<80 px), hiện ' + Math.round(chip.chip) + ' px trong bong bóng ' + Math.round(chip.bubble) + ' px');

  // 1) danh sách được phép: 7 emoji nhanh đầu, «⌄» mở TOÀN BỘ đã gộp và bỏ trùng; chọn ❤️ ⇒ POST on:true
  await evaluate(`document.querySelectorAll('#msgs .m')[0].querySelector('.acts button[aria-label="Thả cảm xúc"]').click()`);
  await until('Boolean(document.querySelector(".rxpop .rx-b"))');
  const first = await evaluate(`window.__calls[0]`); const sp = new URLSearchParams(first.q);
  assert.deepEqual([first.m, first.p, sp.get('chat_id'), sp.get('message_id'), sp.get('account')], ['GET', '/api/messages/reactions', '42', '1', '+84900000000'], 'gọi đúng tin và đúng tài khoản');
  assert.deepEqual(await evaluate(`[...document.querySelectorAll('.rxpop .rx-row .rx-b')].map((b) => b.textContent)`), ['❤️', '👍', '👎', '🔥', '🥰', '👏', '😁'], '7 cảm xúc nhanh đúng theo thứ tự Telegram trả');
  assert.equal(await evaluate(`document.activeElement.classList.contains('rx-b')`), true, 'mở thanh ⇒ tiêu điểm vào cảm xúc đầu (dùng được bằng bàn phím)');
  assert.equal(await evaluate(`document.querySelector('.rxpop .rx-all').hidden`), true);
  await evaluate(`document.querySelector('.rxpop .rx-more').click()`);
  assert.deepEqual(await evaluate(`[...document.querySelectorAll('.rxpop .rx-all .rx-b')].map((b) => b.textContent)`), ['❤️', '👍', '👎', '🔥', '🥰', '👏', '😁', '🎉', '😢', '🤩'], 'gộp top + gần đây + phổ biến, bỏ trùng');
  assert.equal(await evaluate(`document.querySelectorAll('.rxpop .rx-b.mine').length`), 0, 'tin 1 chưa có cảm xúc của mình');
  await evaluate(`document.querySelector('.rxpop .rx-row .rx-b').click()`);
  await until('window.__calls.some((c) => c.p === "/api/messages/react")');
  assert.deepEqual(await evaluate(`window.__calls.find((c) => c.p === '/api/messages/react').body`), { chat_id: 42, message_id: 1, emoji: '❤️', on: true });
  assert.equal(await evaluate(`document.querySelector('.rxpop')`), null, 'chọn xong thì đóng');

  // 2) tin 2 đã có 🔥 của mình ⇒ làm nổi; chọn lại 🔥 ⇒ GỠ (on:false); chọn emoji khác ⇒ on:true
  await evaluate(`window.__calls.length = 0; document.querySelectorAll('#msgs .m')[1].querySelector('.acts button[aria-label="Thả cảm xúc"]').click()`);
  await until('Boolean(document.querySelector(".rxpop .rx-b"))');
  assert.equal(await evaluate(`document.querySelector('.rxpop .rx-b.mine').textContent`), '🔥');
  await evaluate(`document.querySelector('.rxpop .rx-b.mine').click()`);
  await until('window.__calls.some((c) => c.p === "/api/messages/react")');
  assert.equal(await evaluate(`window.__calls.find((c) => c.p === '/api/messages/react').body.on`), false, 'bấm lại cảm xúc của mình ⇒ gỡ');

  // 3) nhấp đúp = cảm xúc nhanh đầu tiên được phép; tin đã có cảm xúc ⇒ nhấp đúp gỡ
  await evaluate(`window.__calls.length = 0; document.querySelectorAll('#msgs .m')[0].dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))`);
  await until('window.__calls.some((c) => c.p === "/api/messages/react")');
  assert.deepEqual(await evaluate(`window.__calls.find((c) => c.p === '/api/messages/react').body`), { chat_id: 42, message_id: 1, emoji: '❤️', on: true });
  await evaluate(`window.__calls.length = 0; document.querySelectorAll('#msgs .m')[1].dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))`);
  await until('window.__calls.some((c) => c.p === "/api/messages/react")');
  assert.deepEqual(await evaluate(`window.__calls.find((c) => c.p === '/api/messages/react').body`), { chat_id: 42, message_id: 2, emoji: '🔥', on: false });

  // 4) lỗi từ máy chủ ⇒ báo, không mở thanh; Esc và bấm ra ngoài đóng thanh
  await evaluate(`document.querySelectorAll('#msgs .m')[2].querySelector('.acts button[aria-label="Thả cảm xúc"]').click()`);
  await until('/không thả cảm xúc được/.test(document.getElementById("toast").textContent)');
  assert.equal(await evaluate(`document.querySelector('.rxpop')`), null);
  await evaluate(`document.querySelectorAll('#msgs .m')[0].querySelector('.acts button[aria-label="Thả cảm xúc"]').click()`);
  await until('Boolean(document.querySelector(".rxpop"))');
  await evaluate(`document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))`);
  assert.equal(await evaluate(`document.querySelector('.rxpop')`), null, 'Esc đóng');
  await evaluate(`document.querySelectorAll('#msgs .m')[0].querySelector('.acts button[aria-label="Thả cảm xúc"]').click()`);
  await until('Boolean(document.querySelector(".rxpop"))');
  await evaluate(`document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))`);
  assert.equal(await evaluate(`document.querySelector('.rxpop')`), null, 'bấm ra ngoài đóng');

  // 5) hội thoại không cho thả cảm xúc nào ⇒ báo, không mở thanh rỗng
  await evaluate('document.querySelectorAll("#list .chat")[1].click()');
  await until('document.querySelectorAll("#msgs .m").length === 2');
  await evaluate(`document.querySelectorAll('#msgs .m')[0].querySelector('.acts button[aria-label="Thả cảm xúc"]').click()`);
  await until('/không cho thả cảm xúc/.test(document.getElementById("toast").textContent)');
  assert.equal(await evaluate(`document.querySelector('.rxpop')`), null);

  // 6) bảng emoji trong ô soạn tin: chèn tại con trỏ, giữ mở để chọn nhiều, «gần đây» nhớ bằng COOKIE (không localStorage)
  assert.equal(await evaluate(`Boolean(document.getElementById('emoji-btn'))`), true);
  await evaluate(`document.getElementById('composer').hidden = false; document.getElementById('text').focus(); document.getElementById('emoji-btn').click();`);
  await until('Boolean(document.getElementById("emoji-panel"))');
  await evaluate(`document.querySelector('.ep-e[aria-label$="— fire"]').click(); document.querySelector('.ep-e[aria-label$="— fire"]').click();`);
  assert.equal(await evaluate(`document.getElementById('text').textContent`), '🔥🔥', 'chèn vào ô soạn tin');
  assert.equal(await evaluate(`Boolean(document.getElementById('emoji-panel'))`), true, 'giữ mở');
  assert.match(await evaluate(`document.cookie`), /tggw_emoji=/, 'nhớ «gần đây» bằng cookie');
  assert.deepEqual(await evaluate(`Emoji.recent()`), ['🔥']);
  await evaluate(`document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))`);
  assert.equal(await evaluate(`document.getElementById('emoji-panel')`), null);
  await evaluate(`document.getElementById('emoji-btn').click(); document.getElementById('emoji-btn').click();`);       // bấm nút lần nữa = tắt
  assert.equal(await evaluate(`document.getElementById('emoji-panel')`), null, 'nút bật/tắt');
  assert.deepEqual(errors, [], 'không có lỗi JS');
});
