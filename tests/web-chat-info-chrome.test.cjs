const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { mkdtempSync, readFileSync, rmSync } = require('node:fs');
const { once } = require('node:events');
const { test } = require('node:test');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const storagePolicy = {
  mode: 'local',
  keep: { private: 604800, groups: 604800, channels: null },
  exceptions: [],
  max_cache_bytes: 2147483648,
  stream_ttl_s: 600,
  auto_download: Object.fromEntries(['private', 'groups', 'channels'].map((chat) => [
    chat,
    Object.fromEntries(['photo', 'video', 'file', 'voice', 'video_note', 'gif'].map((k) => [k, { on: false, max_bytes: 10485760 }])),
  ])),
};

test('TC-CHATINFO-FONT: ngăn thông tin hội thoại và popover phông Aa ở 390/700 px', { timeout: 30000 }, async (t) => {
  for (const banned of ['innerHTML', 'outerHTML', 'insertAdjacentHTML', 'localStorage', 'sessionStorage', 'gw_token', 'GATEWAY_TOKEN', '@font-face']) {
    assert.equal(html.includes(banned), false, banned);
  }

  const routes = {
    '/ui/api/auth/status': { dang_nhap: true, da_thiet_lap: true },
    '/api/accounts': { tai_khoan: [{ phone: '+84900000000', status: 'online', display_name: 'An Lê' }] },
    '/api/me': { id: 7, name: 'An Lê', phone: '84900000000' },
    '/api/settings/appearance': { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' },
    '/api/settings/storage': storagePolicy,
    '/api/storage/usage': { total_bytes: 4096, by_type: { photo: 4096 }, top_chats: [{ chat_id: 42, bytes: 4096 }, { chat_id: -1001, bytes: 1024 }] },
    '/api/chats': {
      chats: [
        { id: 42, title: '<script>alert(1)</script> Bình', kind: 'private', user_id: 9, pinned: false, muted: false },
        { id: -1001, title: 'Nhóm dự án', kind: 'group', pinned: true, muted: false },
      ],
    },
    '/api/chats/42/messages': {
      messages: [{ id: 101, chat_id: 42, date: 1700000000, outgoing: false, kind: 'text', text: 'Tin nhắn kiểm tra cỡ chữ', entities: [] }],
    },
    '/api/chats/-1001/messages': {
      messages: [{ id: 201, chat_id: -1001, date: 1700000001, outgoing: true, kind: 'text', text: 'Tin nhắn trong nhóm', entities: [] }],
    },
    '/api/chats/42/info': {
      chat_id: 42,
      title: '<script>alert(1)</script> Bình',
      kind: 'private',
      username: 'binh_le',
      bio: '<script>xssBio()</script> Giới thiệu an toàn',
      phone_masked: '+849•••••009',
      user_id: 9,
      first_name: 'Bình',
      last_name: 'Lê',
      status: 'online',
      is_contact: false,
      is_blocked: false,
      notification: { use_default_mute_for: true, mute_for: 0, use_default_show_preview: true, show_preview: true },
      auto_delete_seconds: 0,
      pinned: false,
      archived: false,
      marked_unread: false,
    },
    '/api/chats/-1001/info': {
      chat_id: -1001,
      title: 'Nhóm dự án',
      kind: 'group',
      username: 'du_an',
      bio: 'Kênh làm việc',
      member_count: 3,
      can_get_members: true,
      my_role: 'admin',
      is_admin: true,
      my_permissions: { can_send_basic_messages: true, can_invite_users: true },
      invite_link: 'https://t.me/+du_an_moi',
      notification: { use_default_mute_for: true, mute_for: 0, use_default_show_preview: true, show_preview: true },
      auto_delete_seconds: 0,
      pinned: true,
      archived: false,
      marked_unread: false,
    },
    '/api/chats/-1001/members': {
      chat_id: -1001,
      total_count: 2,
      members: [
        { user_id: 1, name: 'Chủ nhóm', username: 'chu', role: 'creator', custom_title: 'Owner' },
        { user_id: 2, name: '<script>mod()</script> Quản trị', username: 'mod', role: 'admin', custom_title: 'Mod' },
      ],
    },
    '/api/live/calls': { calls: [] },
    '/api/feed': { items: [], cursor: 0, reset: false },
  };

  const mock = `
    window.chatInfoTestWrites = [];
    window.chatInfoFeedQueue = [];
    window.fetch = async (path, options = {}) => {
      const routes = ${JSON.stringify(routes)};
      const url = new URL(path, 'http://localhost');
      let payload = routes[url.pathname] || {};
      if ((options.method || 'GET') === 'POST') {
        const data = JSON.parse(options.body || '{}');
        window.chatInfoTestWrites.push([url.pathname, data, options.headers['X-Requested-With'], url.searchParams.get('account')]);
        if (url.pathname === '/api/settings/appearance') {
          routes['/api/settings/appearance'] = data;
          payload = data;
        } else if (url.pathname === '/api/settings/storage') {
          routes['/api/settings/storage'] = data;
          payload = data;
        } else if (url.pathname.endsWith('/settings')) {
          if (data.auto_delete && routes['/api/chats/42/info']) {
            routes['/api/chats/42/info'].auto_delete_seconds = data.auto_delete.seconds;
          }
          payload = { ok: true, ...data };
        } else if (url.pathname.endsWith('/clear_history')) {
          payload = { ok: true, deleted: true };
        } else if (url.pathname.endsWith('/leave')) {
          payload = { ok: true, left: true };
        }
      }
      if (url.pathname === '/api/feed') {
        const since = Number(url.searchParams.get('since') || 0);
        const wait = Number(url.searchParams.get('wait') || 0);
        if (window.chatInfoFeedQueue.length) {
          const items = window.chatInfoFeedQueue.splice(0);
          payload = { items, cursor: since + items.length, reset: false };
        } else {
          if (wait > 0) await new Promise((resolve) => setTimeout(resolve, 150));
          if (window.chatInfoFeedQueue.length) {
            const items = window.chatInfoFeedQueue.splice(0);
            payload = { items, cursor: since + items.length, reset: false };
          } else {
            payload = { items: [], cursor: since, reset: false };
          }
        }
      }
      return new Response(JSON.stringify(payload));
    };
  `;

  const profile = mkdtempSync(`${__dirname}/.chatinfo-browser-`);
  const browser = spawn(process.env.CHROME_BIN || 'google-chrome', [
    '--headless', '--no-sandbox', '--disable-gpu', '--no-first-run', '--disable-extensions',
    '--no-proxy-server', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank',
  ]);
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
  const writes = () => evaluate('window.chatInfoTestWrites');
  const dimension = (width) => cdp('Emulation.setDeviceMetricsOverride', { width, height: 844, deviceScaleFactor: 1, mobile: width < 700 });

  await cdp('Runtime.enable');
  await dimension(390);
  const { frameTree } = await cdp('Page.getFrameTree');
  const script = /<script>([\s\S]*?)<\/script>/.exec(html);
  await cdp('Page.setDocumentContent', { frameId: frameTree.frame.id, html: html.replace(script[0], '') });
  await evaluate(mock + script[1]);
  await until('Boolean(document.querySelector("#list .chat"))');

  // Mở hội thoại cá nhân đầu tiên.
  await evaluate('document.querySelector("#list .chat").click()');
  await until('document.querySelectorAll("#msgs .m").length === 1');
  assert.equal(await evaluate('document.documentElement.scrollWidth <= window.innerWidth'), true);

  // 1. Mobile 390 px: mở ngăn Thông tin hội thoại bằng tiêu đề, chiếm toàn màn hình, chống XSS.
  await click('title');
  await until('!document.getElementById("chat-info-drawer").hidden && document.getElementById("chat-info-title").textContent.includes("Bình")');
  assert.equal(await evaluate('document.querySelectorAll("#chat-info-drawer script").length'), 0);
  assert.match(await evaluate('document.getElementById("chat-info-bio").textContent'), /<script>xssBio\(\)<\/script>/);
  assert.equal(await evaluate('Math.round(document.getElementById("chat-info-drawer").getBoundingClientRect().width)'), 390);
  assert.equal(await evaluate('document.documentElement.scrollWidth <= window.innerWidth'), true);

  // Đổi tắt tiếng và kiểm soát tràn ngang / vùng chạm ở 390 px và 700 px.
  await evaluate(`{
    const m = document.getElementById('ci-mute');
    m.value = '3600';
    m.dispatchEvent(new Event('change'));
  }`);
  await until('window.chatInfoTestWrites.some((w) => w[0] === "/api/chats/42/settings" && w[1].notification?.mute_for === 3600)');
  assert.deepEqual((await writes()).find((w) => w[0] === '/api/chats/42/settings').slice(2), ['fetch', '+84900000000']);

  // Bấm tự xoá -> hộp xác nhận bắt buộc phải hiện đúng thông điệp; huỷ không POST, xác nhận mới POST.
  const countBeforeAuto = (await writes()).length;
  await evaluate(`{
    const a = document.getElementById('ci-autodelete');
    a.value = '86400';
    a.dispatchEvent(new Event('change'));
  }`);
  await until('document.getElementById("ci-confirm-dialog").open === true');
  assert.match(await evaluate('document.getElementById("ci-confirm-summary").textContent'), /tin nhắn sẽ bị xoá thật ở cả hai bên và không lấy lại được/);
  assert.equal((await writes()).length, countBeforeAuto);
  await click('ci-confirm-cancel');
  assert.equal(await evaluate('document.getElementById("ci-autodelete").value'), '0');
  assert.equal((await writes()).length, countBeforeAuto);
  await evaluate(`{
    const a = document.getElementById('ci-autodelete');
    a.value = '86400';
    a.dispatchEvent(new Event('change'));
  }`);
  await until('document.getElementById("ci-confirm-dialog").open === true');
  await click('ci-confirm-ok');
  await until('window.chatInfoTestWrites.some((w) => w[0] === "/api/chats/42/settings" && w[1].auto_delete?.confirm === true)');

  // Bật bộ nhớ đệm Mãi mãi cho hội thoại này -> POST /api/settings/storage & cảnh báo vàng tự xoá + Mãi mãi.
  await evaluate(`{
    const f = document.getElementById('ci-cache-forever');
    f.checked = true;
    f.dispatchEvent(new Event('change'));
  }`);
  await until('window.chatInfoTestWrites.some((w) => w[0] === "/api/settings/storage")');
  await until('!document.getElementById("ci-autodelete-warning").hidden');
  assert.equal(
    await evaluate('document.getElementById("ci-autodelete-warning").textContent'),
    'Tin nhắn vẫn sẽ bị Telegram xoá sau 1 ngày; Mãi mãi chỉ giữ tệp đã tải',
  );
  assert.ok(
    (await evaluate('document.getElementById("ci-autodelete-note").textContent'))
      .includes('Tự xoá tin nhắn là cài đặt của Telegram, áp cho TIN MỚI GỬI, xoá thật ở cả hai bên; khác với ngoại lệ «Mãi mãi» của bộ nhớ đệm (chỉ giữ tệp đã tải về máy chủ, không giữ tin nhắn)'),
  );

  // Kiểm tra đồng bộ realtime từ /api/feed (chat_auto_delete) -> đổi cả ngăn Thông tin, tiêu đề và danh sách.
  await evaluate('window.chatInfoFeedQueue.push({ type: "chat_auto_delete", chat_id: 42, seconds: 90000 })');
  await until('document.getElementById("ci-autodelete-current").textContent.includes("Tùy chỉnh 1 ngày 1 giờ")');
  assert.equal(await evaluate('!document.getElementById("head-autodelete").hidden'), true);
  assert.match(await evaluate('document.querySelector("#list .ad-badge").textContent'), /1ng/);
  assert.equal(
    await evaluate('document.getElementById("ci-autodelete-warning").textContent'),
    'Tin nhắn vẫn sẽ bị Telegram xoá sau 1 ngày 1 giờ; Mãi mãi chỉ giữ tệp đã tải',
  );

  // Đặt mức tuỳ chỉnh 3 ngày qua hộp xác nhận.
  await evaluate(`{
    const sel = document.getElementById('ci-autodelete');
    sel.value = 'custom';
    sel.dispatchEvent(new Event('change'));
  }`);
  await until('!document.getElementById("ci-autodelete-custom-row").hidden');
  await evaluate(`{
    document.getElementById('ci-autodelete-days').value = '3';
    document.getElementById('ci-autodelete-apply').click();
  }`);
  await until('document.getElementById("ci-confirm-dialog").open === true');
  await click('ci-confirm-ok');
  await until('window.chatInfoTestWrites.some((w) => w[0] === "/api/chats/42/settings" && w[1].auto_delete?.seconds === 259200)');

  // Kiểm ở 700 px: ngăn nằm bên phải màn hình và không tràn ngang.
  await dimension(700);
  assert.ok(await evaluate(`(() => {
    const r = document.getElementById('chat-info-drawer').getBoundingClientRect();
    return innerWidth === 700 && r.width < 700 && Math.round(r.right) === 700 && document.documentElement.scrollWidth <= 700;
  })()`), 'drawer 700 px');
  assert.ok(await evaluate(`(() => {
    const d = document.getElementById('chat-info-drawer');
    return [...d.querySelectorAll('button,select,input[type="text"],input[type="number"]')].every((n) => !n.getClientRects().length || n.getBoundingClientRect().height >= 44);
  })()`), 'touch target 700 px');

  // Tắt tự xoá qua sự kiện -> đồng hồ và cảnh báo vàng biến mất.
  await evaluate('window.chatInfoFeedQueue.push({ type: "chat_auto_delete", chat_id: 42, seconds: 0 })');
  await until('document.getElementById("head-autodelete").hidden === true');
  assert.equal(await evaluate('document.getElementById("ci-autodelete-warning").hidden'), true);

  await click('chat-info-close');
  await until('document.getElementById("chat-info-drawer").hidden === true');

  // 2. Popover phông «Aa» ngay trong khung chat: đổi cỡ chữ làm computed font-size của tin nhắn thay đổi thật.
  await click('font-open');
  await until('document.getElementById("font-popover").hidden === false');
  assert.equal(await evaluate('getComputedStyle(document.querySelector("#msgs .m")).fontSize'), '15px');
  await evaluate(`{
    const f = document.getElementById('font-pop-family');
    f.value = 'mono';
    f.dispatchEvent(new Event('change'));
    const s = document.getElementById('font-pop-size');
    s.value = '19';
    s.dispatchEvent(new Event('input'));
    s.dispatchEvent(new Event('change'));
  }`);
  assert.equal(await evaluate('getComputedStyle(document.querySelector("#msgs .m")).fontSize'), '19px');
  assert.match(await evaluate('getComputedStyle(document.querySelector("#msgs .m")).fontFamily'), /monospace/i);
  await until('window.chatInfoTestWrites.some((w) => w[0] === "/api/settings/appearance" && w[1].font_size === 19)');

  // Đồng bộ sang Cài đặt › Giao diện.
  assert.equal(await evaluate('document.getElementById("appearance-size").value'), '19');
  assert.equal(await evaluate('document.getElementById("appearance-font").value'), 'mono');

  await click('font-pop-reset');
  await until('getComputedStyle(document.querySelector("#msgs .m")).fontSize === "15px"');
  await click('font-pop-close');

  // 3. Mở hội thoại nhóm -> bấm nút ⓘ -> kiểm danh sách thành viên & liên kết mời.
  await evaluate('document.querySelectorAll("#list .chat")[1].click()');
  await until('document.querySelectorAll("#msgs .m").length === 1 && document.getElementById("title").textContent === "Nhóm dự án"');
  await click('chat-info-open');
  await until('!document.getElementById("chat-info-drawer").hidden && !document.getElementById("chat-info-group").hidden');
  assert.match(await evaluate('document.getElementById("ci-members-list").textContent'), /Chủ/);
  assert.match(await evaluate('document.getElementById("ci-members-list").textContent'), /Quản trị/);
  assert.equal(await evaluate('document.getElementById("ci-invite-link").textContent'), 'https://t.me/+du_an_moi');

  assert.equal(await evaluate('localStorage.length + sessionStorage.length'), 0);
  assert.deepEqual(errors, []);
});
