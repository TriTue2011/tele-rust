const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { mkdtempSync, readFileSync, rmSync } = require('node:fs');
const { once } = require('node:events');
const { test } = require('node:test');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const storagePolicy = {
  mode: 'local', keep: { private: 604800, groups: 604800, channels: null },
  exceptions: [], max_cache_bytes: 2147483648, stream_ttl_s: 600,
  auto_download: Object.fromEntries(['private', 'groups', 'channels'].map((chat) => [chat,
    Object.fromEntries(['photo', 'video', 'file', 'voice', 'video_note', 'gif'].map((kind) => [kind, { on: false, max_bytes: 10485760 }]))])),
};

test('TC-WYSIWYG-THEMES: trình soạn thảo WYSIWYG B/I/U/S/</>/Tx, paste/IME và 10 chủ đề ở 390/700/1440 px', { timeout: 30000 }, async (t) => {
  for (const banned of ['innerHTML', 'outerHTML', 'insertAdjacentHTML', 'document.write', 'localStorage', 'sessionStorage', 'gw_token', 'GATEWAY_TOKEN']) {
    assert.equal(html.includes(banned), false, banned);
  }

  const routes = {
    '/ui/api/auth/status': { dang_nhap: true, da_thiet_lap: true },
    '/api/accounts': { tai_khoan: [{ phone: '+84900000000', status: 'online', display_name: 'An Lê' }] },
    '/api/me': { id: 7, name: 'An Lê', phone: '84900000000' },
    '/api/profile': {
      id: 7, name: 'An Lê', first_name: 'An', last_name: 'Lê',
      bio: 'Xin chào', username: 'an_le', phone_masked: '+849•••••000',
    },
    '/api/settings/appearance': { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' },
    '/api/settings/storage': storagePolicy,
    '/api/storage/usage': { total_bytes: 0, by_type: {}, top_chats: [] },
    '/api/storage/janitor': {},
    '/api/live/settings': { camera: 'black', mic: 'silence', mic_format: 'auto', speaker: '', playback: 'file' },
    '/api/chats': { chats: [{ id: 42, title: 'Hội thoại thử', kind: 'private', user_id: 9 }] },
    '/api/chats/42/messages': { messages: [] },
    '/api/live/calls': { calls: [] },
    '/api/feed': { items: [], cursor: 0, reset: false },
  };

  const mock = `
    window.wysiwygWrites = [];
    window.fetch = async (path, options = {}) => {
      const routes = ${JSON.stringify(routes)};
      const url = new URL(path, 'http://localhost');
      let payload = routes[url.pathname] || {};
      if ((options.method || 'GET') === 'POST') {
        const data = JSON.parse(options.body || '{}');
        window.wysiwygWrites.push([url.pathname, data, options.headers['X-Requested-With'], url.searchParams.get('account')]);
        if (url.pathname === '/api/settings/appearance') {
          routes['/api/settings/appearance'] = data;
          payload = data;
        } else if (url.pathname === '/api/chats/42/send') {
          payload = { id: 500 + window.wysiwygWrites.length, chat_id: 42, date: 1700000000, outgoing: true, kind: 'text', text: data.text, entities: data.entities || [] };
        }
      }
      if (url.pathname === '/api/feed') await new Promise((resolve) => setTimeout(resolve, 400));
      return new Response(JSON.stringify(payload));
    };
  `;

  const profile = mkdtempSync(`${__dirname}/.wysiwyg-browser-`);
  const browser = spawn(process.env.CHROME_BIN || 'google-chrome', ['--headless', '--no-sandbox', '--disable-gpu',
    '--no-first-run', '--disable-extensions', '--no-proxy-server', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank']);
  t.after(async () => {
    if (browser.exitCode === null && browser.signalCode === null) { browser.kill(); await once(browser, 'exit'); }
    rmSync(profile, { recursive: true, force: true });
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

  const [page] = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await once(ws, 'open');
  t.after(() => ws.close());

  const pending = new Map(), errors = [];
  let seq = 0;
  ws.addEventListener('message', ({ data }) => {
    const msg = JSON.parse(data);
    if (msg.method === 'Runtime.exceptionThrown') errors.push(msg.params.exceptionDetails.text);
    if (msg.method === 'Runtime.consoleAPICalled' && ['error', 'warning'].includes(msg.params.type)) errors.push(msg.params.type);
    if (msg.id) { const resolve = pending.get(msg.id); pending.delete(msg.id); resolve(msg); }
  });

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
  async function until(expression) {
    for (let n = 0; n < 120; n++) {
      if (await evaluate(expression)) return;
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
    assert.fail(`${expression}: ${JSON.stringify(errors)}`);
  }
  const click = (id) => evaluate(`document.getElementById(${JSON.stringify(id)}).click()`);
  const writes = () => evaluate('window.wysiwygWrites');
  const dimension = (width) => cdp('Emulation.setDeviceMetricsOverride', { width, height: 844, deviceScaleFactor: 1, mobile: width < 700 });

  await cdp('Runtime.enable');
  await dimension(1440);
  const { frameTree } = await cdp('Page.getFrameTree');
  const script = /<script>([\s\S]*?)<\/script>/.exec(html);
  await cdp('Page.setDocumentContent', { frameId: frameTree.frame.id, html: html.replace(script[0], '') });
  await evaluate(mock + script[1]);
  await until('Boolean(document.querySelector("#list .chat"))');

  // Mở hội thoại thử ở 1440 px.
  await evaluate('document.querySelector("#list .chat").click()');
  await until('document.getElementById("composer").hidden === false');

  // 1. Chọn chữ và bấm B -> hiển thị trực tiếp font-weight đậm trong #text, không chèn '*'.
  await evaluate(`(() => {
    const ed = document.getElementById('text');
    ed.focus();
    ed.textContent = 'Xin chào bạn';
    const r = document.createRange();
    r.setStart(ed.firstChild, 4);
    r.setEnd(ed.firstChild, 8);
    const s = window.getSelection();
    s.removeAllRanges();
    s.addRange(r);
  })()`);
  await click('fmt-bold');
  const boldCheck = await evaluate(`(() => {
    const ed = document.getElementById('text');
    const st = ed.querySelector('strong');
    return {
      hasStar: ed.textContent.includes('*'),
      text: ed.textContent,
      boldText: st ? st.textContent : '',
      weight: st ? getComputedStyle(st).fontWeight : '',
      pressed: document.getElementById('fmt-bold').getAttribute('aria-pressed'),
    };
  })()`);
  assert.equal(boldCheck.hasStar, false);
  assert.equal(boldCheck.text, 'Xin chào bạn');
  assert.equal(boldCheck.boldText, 'chào');
  assert.ok(boldCheck.weight === 'bold' || Number(boldCheck.weight) >= 700);
  assert.equal(boldCheck.pressed, 'true');

  // 2. Bấm U và S -> text-decoration-line tương ứng; bấm B lần 2 -> tắt đậm.
  await click('fmt-underline');
  await click('fmt-strike');
  const decoCheck = await evaluate(`(() => {
    const ed = document.getElementById('text');
    const u = ed.querySelector('u');
    const s = ed.querySelector('s');
    return {
      uDeco: u ? getComputedStyle(u).textDecorationLine : '',
      sDeco: s ? getComputedStyle(s).textDecorationLine : '',
    };
  })()`);
  assert.match(decoCheck.uDeco, /underline/);
  assert.match(decoCheck.sDeco, /line-through/);

  await click('fmt-bold');
  assert.equal(await evaluate('Boolean(document.querySelector("#text strong"))'), false);
  assert.equal(await evaluate('document.getElementById("fmt-bold").getAttribute("aria-pressed")'), 'false');

  // 3. Bấm </> (mã) -> font monospace; bấm Tx (xoá định dạng) -> sạch thẻ định dạng, giữ nguyên văn bản.
  await click('fmt-code');
  assert.match(await evaluate('getComputedStyle(document.querySelector("#text code")).fontFamily'), /monospace/i);
  await click('fmt-clear');
  const afterClear = await evaluate(`(() => {
    const ed = document.getElementById('text');
    return { text: ed.textContent, tags: ed.querySelectorAll('strong,em,u,s,code,pre').length };
  })()`);
  assert.equal(afterClear.text, 'Xin chào bạn');
  assert.equal(afterClear.tags, 0);

  // 4. Emoji + chữ đậm + chữ nghiêng -> gửi nhận đúng UTF-16 offset và hiển thị trong #msgs không có '**'.
  await evaluate(`(() => {
    const ed = document.getElementById('text');
    ed.textContent = '🌙 Đậm nghiêng';
    const r = document.createRange();
    r.setStart(ed.firstChild, 3);
    r.setEnd(ed.firstChild, 6);
    const s = window.getSelection();
    s.removeAllRanges();
    s.addRange(r);
    document.getElementById('fmt-bold').click();
    const tail = [...ed.childNodes].find((n) => n.nodeType === 3 && n.nodeValue.includes('nghiêng'));
    const r2 = document.createRange();
    r2.setStart(tail, 1);
    r2.setEnd(tail, 8);
    s.removeAllRanges();
    s.addRange(r2);
    document.getElementById('fmt-italic').click();
  })()`);
  await click('send');
  await until('document.querySelectorAll("#msgs .m").length === 1');
  const send1 = (await writes()).at(-1);
  assert.equal(send1[0], '/api/chats/42/send');
  assert.equal(send1[1].text, '🌙 Đậm nghiêng');
  assert.deepEqual(send1[1].entities, [
    { type: 'bold', offset: 3, length: 3 },
    { type: 'italic', offset: 7, length: 7 },
  ]);
  assert.equal(await evaluate('document.querySelector("#msgs .m strong")?.textContent'), 'Đậm');
  assert.equal(await evaluate('document.querySelector("#msgs .m em")?.textContent'), 'nghiêng');
  assert.equal(await evaluate('document.querySelector("#msgs .m").textContent.includes("**")'), false);

  // 5. Người dùng tự gõ '**giữ nguyên**' -> giữ nguyên ký tự, không sinh entity.
  await evaluate('document.getElementById("text").textContent = "**giữ nguyên**"');
  await click('send');
  await until('document.querySelectorAll("#msgs .m").length === 2');
  const send2 = (await writes()).at(-1);
  assert.equal(send2[1].text, '**giữ nguyên**');
  assert.equal(send2[1].entities, undefined);

  // 6. Dán HTML chứa <script> và <img onerror> -> chỉ còn chữ thuần, không XSS.
  await evaluate(`(() => {
    const ed = document.getElementById('text');
    ed.textContent = '';
    ed.focus();
    const dt = new DataTransfer();
    dt.setData('text/html', '<script>window.__xss=1</script><b>Dán sạch</b><img src=x onerror="window.__xss=2">');
    dt.setData('text/plain', 'Dán sạch');
    ed.dispatchEvent(new ClipboardEvent('paste', { bubbles: true, cancelable: true, clipboardData: dt }));
  })()`);
  const pasteCheck = await evaluate(`(() => {
    const ed = document.getElementById('text');
    return {
      xss: window.__xss,
      text: ed.textContent,
      tags: ed.querySelectorAll('script,img,b,strong').length,
    };
  })()`);
  assert.equal(pasteCheck.xss, undefined);
  assert.equal(pasteCheck.text, 'Dán sạch');
  assert.equal(pasteCheck.tags, 0);

  // 7. Mô phỏng gõ tiếng Việt IME (compositionstart -> Enter không gửi -> compositionend -> Enter gửi).
  const writesBeforeIme = (await writes()).length;
  await evaluate(`(() => {
    const ed = document.getElementById('text');
    ed.textContent = '';
    ed.focus();
    ed.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true, data: '' }));
    ed.textContent = 'Tie';
    ed.dispatchEvent(new CompositionEvent('compositionupdate', { bubbles: true, data: 'Tie' }));
    ed.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  })()`);
  assert.equal((await writes()).length, writesBeforeIme);
  await evaluate(`(() => {
    const ed = document.getElementById('text');
    ed.textContent = 'Tiếng Việt có dấu';
    ed.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true, data: 'Tiếng Việt có dấu' }));
    ed.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  })()`);
  await until('document.querySelectorAll("#msgs .m").length === 3');
  assert.equal((await writes()).at(-1)[1].text, 'Tiếng Việt có dấu');

  // 8. Kiểm responsive 390 / 700 / 1440 px & đổi 10 chủ đề + nền khung chat + chủ đề khối mã.
  await click('settings-open');
  await until('document.getElementById("settings-dialog").open === true');
  await evaluate('document.querySelector(\'#settings-dialog [data-section="appearance"]\').click()');

  for (const width of [390, 700, 1440]) {
    await dimension(width);
    assert.ok(await evaluate(`(() => {
      const d = document.getElementById('settings-dialog');
      return innerWidth === ${width} && d.scrollWidth <= d.clientWidth && document.documentElement.scrollWidth <= innerWidth &&
        [...d.querySelectorAll('button,select')].every((n) => !n.getClientRects().length || n.getBoundingClientRect().height >= 44);
    })()`), `layout/touch target ${width} px`);
  }

  await click('theme-midnight');
  assert.equal(await evaluate('document.documentElement.dataset.theme'), 'midnight');
  const midnightBg = await evaluate('getComputedStyle(document.documentElement).getPropertyValue("--bg").trim()');
  await click('theme-telegram');
  const telegramBg = await evaluate('getComputedStyle(document.documentElement).getPropertyValue("--bg").trim()');
  assert.notEqual(midnightBg, telegramBg);

  await click('theme-high-contrast');
  assert.equal(await evaluate('document.documentElement.dataset.theme'), 'high-contrast');
  await click('chat-bg-pattern');
  assert.equal(await evaluate('document.documentElement.dataset.chatBg'), 'pattern');
  await click('code-theme-dark');
  assert.equal(await evaluate('document.documentElement.dataset.codeTheme'), 'dark');
  await click('appearance-save');
  await until('document.getElementById("appearance-status").textContent.includes("Đã lưu")');
  const savedApp = (await writes()).at(-1);
  assert.equal(savedApp[0], '/api/settings/appearance');
  assert.equal(savedApp[1].theme, 'high-contrast');
  assert.equal(savedApp[1].chat_bg, 'pattern');
  assert.equal(savedApp[1].code_theme, 'dark');

  await click('appearance-reset');
  assert.equal(await evaluate('document.documentElement.dataset.theme'), 'system');
  assert.equal(await evaluate('document.documentElement.dataset.chatBg'), 'plain');

  assert.equal(await evaluate('localStorage.length + sessionStorage.length'), 0);
  assert.deepEqual(errors, []);
});
