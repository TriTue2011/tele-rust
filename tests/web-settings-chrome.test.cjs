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

test('TC-CAIDAT: khung cài đặt 390/700 px, giao diện, hồ sơ, định dạng entities và hồi quy', { timeout: 30000 }, async (t) => {
  assert.equal(html.includes('innerHTML'), false);
  assert.equal(html.includes('localStorage'), false);
  assert.equal(html.includes('sessionStorage'), false);

  const routes = {
    '/ui/api/auth/status': { dang_nhap: true, da_thiet_lap: true },
    '/api/accounts': { tai_khoan: [{ phone: '+84900000000', status: 'online', display_name: 'An Lê' }] },
    '/api/me': { id: 7, name: 'An Lê', phone: '84900000000' },
    '/api/profile': {
      id: 7, name: 'An Lê', first_name: 'An', last_name: 'Lê',
      bio: '<em>Tiểu sử cũ</em>', username: 'an_le', phone_masked: '+849•••••000',
    },
    '/api/settings/appearance': { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' },
    '/api/settings/storage': storagePolicy,
    '/api/storage/usage': { total_bytes: 1024, by_type: { photo: 1024 }, top_chats: [] },
    '/api/storage/janitor': {},
    '/api/live/settings': { camera: 'black', mic: 'silence', mic_format: 'auto', speaker: '', playback: 'file' },
    '/api/chats': { chats: [{ id: 42, title: 'Hội thoại thử', kind: 'private', user_id: 9 }] },
    '/api/chats/42/messages': { messages: [] },
    '/api/live/calls': { calls: [] },
    '/api/feed': { items: [], cursor: 0, reset: false },
  };

  const mock = `
    window.settingsTestWrites = [];
    window.fetch = async (path, options = {}) => {
      const routes = ${JSON.stringify(routes)};
      const url = new URL(path, 'http://localhost');
      let payload = routes[url.pathname] || {};
      if ((options.method || 'GET') === 'POST') {
        const data = JSON.parse(options.body || '{}');
        window.settingsTestWrites.push([url.pathname, data, options.headers['X-Requested-With'], url.searchParams.get('account')]);
        if (url.pathname === '/api/settings/appearance') {
          routes['/api/settings/appearance'] = data;
          payload = data;
        } else if (url.pathname === '/api/profile') {
          const next = { ...routes['/api/profile'], ...data };
          delete next.confirm;
          next.name = (next.first_name + ' ' + next.last_name).trim();
          routes['/api/profile'] = next;
          payload = next;
        } else if (url.pathname === '/api/chats/42/send') {
          payload = { id: 501, chat_id: 42, date: 1700000000, outgoing: true, kind: 'text', text: data.text, entities: data.entities || [] };
        }
      }
      if (url.pathname === '/api/feed') await new Promise((resolve) => setTimeout(resolve, 400));
      return new Response(JSON.stringify(payload));
    };
  `;

  const profile = mkdtempSync(`${__dirname}/.settings-browser-`);
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
  const writes = () => evaluate('window.settingsTestWrites');
  const dimension = (width) => cdp('Emulation.setDeviceMetricsOverride', { width, height: 844, deviceScaleFactor: 1, mobile: width < 700 });

  await cdp('Runtime.enable');
  await dimension(390);
  const { frameTree } = await cdp('Page.getFrameTree');
  const script = /<script>([\s\S]*?)<\/script>/.exec(html);
  await cdp('Page.setDocumentContent', { frameId: frameTree.frame.id, html: html.replace(script[0], '') });
  await evaluate(mock + script[1]);
  await until('Boolean(document.querySelector("#list .chat"))');

  // 1. Mobile 390 px: mở Cài đặt hiện danh sách trước, chạm mục vào trang chi tiết, bấm Quay lại về danh sách.
  await click('settings-open');
  await until('document.getElementById("settings-dialog").open === true');
  assert.equal(await evaluate('document.querySelector(".settings-nav").getBoundingClientRect().width > 0'), true);
  assert.equal(await evaluate('document.querySelector(".settings-detail").getBoundingClientRect().width'), 0);

  await evaluate('document.querySelector(\'#settings-dialog [data-section="appearance"]\').click()');
  assert.equal(await evaluate('document.querySelector(".settings-detail").getBoundingClientRect().width > 0'), true);
  assert.equal(await evaluate('document.getElementById("settings-back").getBoundingClientRect().height >= 44'), true);
  await click('settings-back');
  assert.equal(await evaluate('document.querySelector(".settings-nav").getBoundingClientRect().width > 0'), true);
  assert.equal(await evaluate('document.querySelector(".settings-detail").getBoundingClientRect().width'), 0);
  await evaluate('document.querySelector(\'#settings-dialog [data-section="appearance"]\').click()');

  // Kiểm tra tràn ngang & vùng chạm ở 390 px và 700 px.
  for (const width of [390, 700]) {
    await dimension(width);
    assert.ok(await evaluate(`(() => {
      const d = document.getElementById('settings-dialog');
      return innerWidth === ${width} && d.scrollWidth <= d.clientWidth && document.documentElement.scrollWidth <= innerWidth &&
        [...d.querySelectorAll('button,select')].every((node) => !node.getClientRects().length || node.getBoundingClientRect().height >= 44);
    })()`), `layout ${width} px`);
  }
  // Tại 700 px: 2 cột đồng thời hiện, #settings-back ẩn.
  assert.equal(await evaluate('document.querySelector(".settings-nav").getBoundingClientRect().width > 0'), true);
  assert.equal(await evaluate('document.querySelector(".settings-detail").getBoundingClientRect().width > 0'), true);
  assert.equal(await evaluate('document.getElementById("settings-back").getBoundingClientRect().width'), 0);

  // 2. Giao diện áp ngay lên danh sách, khung chat và khung xem trước; lưu lên máy chủ.
  await evaluate(`{
    document.getElementById('theme-dark').click();
    document.getElementById('bubble-rounded').click();
    document.getElementById('accent-green').click();
    const font = document.getElementById('appearance-font'); font.value = 'mono'; font.dispatchEvent(new Event('change'));
    const size = document.getElementById('appearance-size'); size.value = '18'; size.dispatchEvent(new Event('input'));
  }`);
  assert.equal(await evaluate('getComputedStyle(document.getElementById("msgs")).fontSize'), '18px');
  assert.equal(await evaluate('getComputedStyle(document.querySelector("#list .t")).fontSize'), '18px');
  assert.equal(await evaluate('getComputedStyle(document.getElementById("appearance-preview")).fontSize'), '18px');
  assert.equal(await evaluate('document.documentElement.dataset.theme'), 'dark');
  await click('appearance-save');
  await until('document.getElementById("appearance-status").textContent.includes("Đã lưu")');
  assert.equal((await writes())[0][0], '/api/settings/appearance');
  assert.equal((await writes())[0][1].font_size, 18);

  // 3. Hồ sơ: che số điện thoại, chống XSS, xác nhận trước khi lưu.
  await evaluate('document.querySelector(\'#settings-dialog [data-section="profile"]\').click()');
  await until('document.getElementById("profile-phone-masked").textContent === "+849•••••000"');
  assert.equal(await evaluate('document.getElementById("settings-dialog").querySelector("em") === null'), true);
  await evaluate(`{
    document.getElementById('profile-first-name').value = 'Minh';
    document.getElementById('profile-bio').value = 'Tiểu sử mới';
    document.getElementById('profile-form').requestSubmit();
  }`);
  await until('document.getElementById("profile-confirm").open === true');
  assert.equal((await writes()).length, 1);
  await click('profile-confirm-cancel');
  assert.equal((await writes()).length, 1);
  await evaluate('document.getElementById("profile-form").requestSubmit()');
  await until('document.getElementById("profile-confirm").open === true');
  await click('profile-confirm-ok');
  await until('document.getElementById("profile-status").textContent.includes("Đã cập nhật")');
  assert.deepEqual((await writes())[1].slice(0, 2), ['/api/profile', { confirm: true, first_name: 'Minh', bio: 'Tiểu sử mới' }]);

  // 4. Mục G3/G4/G6 hiển thị Sắp có và không có điều khiển giả.
  await evaluate('document.querySelector(\'#settings-dialog [data-section="notifications"]\').click()');
  assert.equal(await evaluate('document.getElementById("settings-panel-soon").hidden'), false);
  assert.equal(await evaluate('document.querySelectorAll("#settings-panel-soon input, #settings-panel-soon select, #settings-panel-soon textarea").length'), 0);

  // 5. Hồi quy nút Dữ liệu và lưu trữ & Cuộc gọi bên trong Cài đặt.
  await evaluate('document.querySelector(\'#settings-dialog [data-section="storage"]\').click()');
  await click('storage-open');
  await until('document.getElementById("storage-dialog").open === true');
  await click('storage-close');
  await evaluate('document.querySelector(\'#settings-dialog [data-section="calls"]\').click()');
  await click('call-settings-open');
  await until('document.getElementById("call-settings").open === true');
  await click('call-settings-close');
  await click('settings-close');

  // 6. Chiều gửi định dạng WYSIWYG chữ đậm (không chèn '*') + entities trong khung chat.
  await evaluate('document.querySelector("#list .chat").click()');
  await until('document.getElementById("composer").hidden === false');
  await evaluate(`{
    const t = document.getElementById('text');
    t.focus();
    t.textContent = 'Xin chào';
    const r = document.createRange();
    r.setStart(t.firstChild, 4);
    r.setEnd(t.firstChild, 8);
    const s = window.getSelection();
    s.removeAllRanges();
    s.addRange(r);
    document.getElementById('fmt-bold').click();
  }`);
  assert.equal(await evaluate('document.getElementById("text").textContent.includes("*")'), false);
  assert.equal(await evaluate('document.querySelector("#text strong")?.textContent'), 'chào');
  await click('send');
  await until('document.querySelectorAll("#msgs .m").length === 1');
  const sendWrite = (await writes()).at(-1);
  assert.equal(sendWrite[0], '/api/chats/42/send');
  assert.equal(sendWrite[1].text, 'Xin chào');
  assert.deepEqual(sendWrite[1].entities, [{ type: 'bold', offset: 4, length: 4 }]);
  assert.equal(await evaluate('document.querySelector("#msgs .m strong")?.textContent'), 'chào');
  assert.equal(await evaluate('localStorage.length + sessionStorage.length'), 0);
  assert.deepEqual(errors, []);
});
