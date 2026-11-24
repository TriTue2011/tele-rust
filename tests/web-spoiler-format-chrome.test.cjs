// Nút định dạng «Ẩn chữ (spoiler)» trong thanh công cụ soạn tin (fmt-spoiler), như Telegram thật: bọc <tg-spoiler>,
// gửi lên bằng entity {type:"spoiler", offset, length}. Backend (src/chat/write.rs:62) đã nhận entity này từ trước,
// chỉ thiếu nút bấm ở composer — test này kiểm đúng phần vừa thêm.
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

test('TC-SPOILER-FMT: nút «Ẩn chữ» bọc <tg-spoiler>, gửi lên đúng entity spoiler, bấm lại thì bỏ', { timeout: 60000 }, async (t) => {
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
      if (url.pathname === '/api/chats/42/send' && method === 'POST') { window.__calls.push(JSON.parse(init.body)); return new Response(JSON.stringify({ id: 99 }), { status: 200, headers: { 'Content-Type': 'application/json' } }); }
      const payload = routes[url.pathname] || {};
      return new Response(JSON.stringify(payload), { status: 200, headers: { 'Content-Type': 'application/json' } });
    };`;
  const profile = mkdtempSync(`${require('node:os').tmpdir()}/tele-spoiler-`);
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

  assert.notEqual(await evaluate('document.getElementById("fmt-spoiler")'), null, 'phải có nút định dạng ẩn chữ (spoiler)');

  // gõ "mat khau bi mat", chọn đúng "bi mat", bấm nút spoiler
  await evaluate(`
    const t = document.getElementById("text"); t.focus();
    t.textContent = "mat khau bi mat";
    const r = document.createRange(); r.setStart(t.firstChild, 9); r.setEnd(t.firstChild, 15);
    const sel = window.getSelection(); sel.removeAllRanges(); sel.addRange(r);
    document.getElementById("fmt-spoiler").dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    document.getElementById("fmt-spoiler").click();
  `);
  assert.equal(await evaluate('document.getElementById("text").querySelectorAll("tg-spoiler").length'), 1, 'phải bọc đúng 1 <tg-spoiler>');
  assert.equal(await evaluate('document.getElementById("text").querySelector("tg-spoiler").textContent'), 'bi mat');

  // bấm lại nút khi vùng chọn vẫn trong chữ đã bọc: phải BỎ spoiler (toggle), không tăng thêm lớp bọc
  await evaluate(`
    const span2 = document.getElementById("text").querySelector("tg-spoiler");
    const r2 = document.createRange(); r2.selectNodeContents(span2);
    const sel2 = window.getSelection(); sel2.removeAllRanges(); sel2.addRange(r2);
    document.getElementById("fmt-spoiler").dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    document.getElementById("fmt-spoiler").click();
  `);
  assert.equal(await evaluate('document.getElementById("text").querySelectorAll("tg-spoiler").length'), 0, 'bấm lại thì bỏ định dạng spoiler');

  // gõ lại chữ mới (tránh phụ thuộc cấu trúc DOM sau khi bỏ bọc), bọc rồi gửi thật: kiểm payload gửi lên đúng entity spoiler
  await evaluate(`
    const t3 = document.getElementById("text");
    t3.textContent = "mat khau bi mat";
    const r3 = document.createRange(); r3.setStart(t3.firstChild, 9); r3.setEnd(t3.firstChild, 15);
    const sel3 = window.getSelection(); sel3.removeAllRanges(); sel3.addRange(r3);
    document.getElementById("fmt-spoiler").dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    document.getElementById("fmt-spoiler").click();
  `);
  assert.equal(await evaluate('document.getElementById("text").querySelectorAll("tg-spoiler").length'), 1, 'phải bọc lại được sau khi bỏ');
  await evaluate('document.getElementById("send").click();');
  await until('window.__calls.length > 0');
  const sent = await evaluate('window.__calls[0]');
  assert.equal(sent.text, 'mat khau bi mat');
  assert.deepEqual(sent.entities, [{ type: 'spoiler', offset: 9, length: 6 }]);

  assert.deepEqual(errors, [], 'không có lỗi JS');
});
