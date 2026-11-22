const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { mkdtempSync, readFileSync, rmSync } = require('node:fs');
const { once } = require('node:events');
const { test } = require('node:test');
const { openHttpOrigin } = require('./chrome-origin.cjs');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');

test('TC-MACOS-LIST-CALLS: danh sách hội thoại, tiêu đề khung chat, tin cuộc gọi & menu ngữ cảnh ở 390/700/1440 px', { timeout: 30000 }, async (t) => {
  for (const banned of ['innerHTML', 'outerHTML', 'insertAdjacentHTML', 'localStorage', 'sessionStorage', 'gw_token', 'GATEWAY_TOKEN', '@font-face']) {
    assert.equal(html.includes(banned), false, banned);
  }

  const nowSec = Math.floor(Date.now() / 1000);
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

  const chats = [
    {
      id: -1001,
      title: '星驰公益',
      kind: 'group',
      unread: 14682,
      unread_mentions: 3,
      muted: true,
      pinned: false,
      member_count: 1200,
      last: { id: 10, date: nowSec - 60, outgoing: false, kind: 'text', sender: '星驰管家', text: '@devan_m_p…' },
    },
    {
      id: 42,
      title: '<script>alert(1)</script> Đại ca',
      kind: 'private',
      user_id: 77,
      muted: true,
      auto_delete_seconds: 86400,
      was_online: nowSec - 900,
      last_read_outbox_message_id: 202,
      last: {
        id: 205,
        date: nowSec - 120,
        outgoing: false,
        kind: 'call',
        text: 'Cuộc gọi video đến (18 giây)',
        call: { is_video: true, outgoing: false, duration: 18, reason: 'hung_up', status: 'incoming', missed: false, label: 'Cuộc gọi video đến', duration_text: '18 giây' },
      },
    },
    {
      id: -1002,
      title: 'HÀ NỘI 24H Channel',
      kind: 'channel',
      unread: 185,
      muted: true,
      last: { id: 301, date: nowSec - 3600, outgoing: false, kind: 'animation', thumb_file: 55, caption: 'Phượt đêm, chỉ 1 giây mất l…', text: 'GIF: Phượt đêm' },
    },
    {
      id: 777,
      title: 'Telegram',
      kind: 'private',
      user_id: 777,
      verified: true,
      online: true,
      pinned: true,
      last: { id: 401, date: nowSec - 86400 * 3, outgoing: false, kind: 'text', text: 'Confirm login to the Telegram Gateway' },
    },
    {
      id: 88,
      title: 'Anh Tuan Chị Ngọc',
      kind: 'private',
      user_id: 88,
      last: { id: 501, date: nowSec - 86400 * 20, outgoing: false, kind: 'service', text: 'Anh Tuan Chị Ngọc đã tham gia Telegram' },
    },
    {
      id: 99,
      title: 'Cuộc gọi nhỡ test',
      kind: 'private',
      user_id: 99,
      last: {
        id: 601,
        date: nowSec - 90,
        outgoing: false,
        kind: 'call',
        text: 'Cuộc gọi nhỡ',
        call: { is_video: false, outgoing: false, duration: 0, reason: 'missed', status: 'missed', missed: true, label: 'Cuộc gọi nhỡ', duration_text: '' },
      },
    },
  ];

  const messages42 = [
    { id: 201, chat_id: 42, date: nowSec - 300, outgoing: true, read: true, kind: 'text', text: 'Xin chào <script>xss()</script>', entities: [] },
    {
      id: 202,
      chat_id: 42,
      date: nowSec - 240,
      outgoing: false,
      kind: 'call',
      summary: 'Cuộc gọi video đến (19 giây)',
      call: { is_video: true, outgoing: false, duration: 19, reason: 'hung_up', status: 'incoming', missed: false, label: 'Cuộc gọi video đến', duration_text: '19 giây' },
    },
    {
      id: 203,
      chat_id: 42,
      date: nowSec - 180,
      outgoing: false,
      kind: 'call',
      summary: 'Cuộc gọi nhỡ',
      call: { is_video: false, outgoing: false, duration: 0, reason: 'missed', status: 'missed', missed: true, label: 'Cuộc gọi nhỡ', duration_text: '' },
    },
    {
      id: 204,
      chat_id: 42,
      date: nowSec - 150,
      outgoing: true,
      kind: 'service',
      summary: 'Bạn đã đặt hẹn giờ tự hủy cho mọi cuộc trò chuyện sau 1 ngày',
    },
  ];

  const routes = {
    '/ui/api/auth/status': { dang_nhap: true, da_thiet_lap: true },
    '/api/accounts': { tai_khoan: [{ phone: '+84900000000', status: 'online', display_name: 'An Lê' }] },
    '/api/me': { id: 7, name: 'An Lê', phone: '84900000000' },
    '/api/settings/appearance': { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' },
    '/api/settings/storage': storagePolicy,
    '/api/storage/usage': { total_bytes: 0, by_type: {}, top_chats: [] },
    '/api/chats': { chats },
    '/api/chats/42/messages': { messages: messages42 },
    '/api/chats/-1001/messages': { messages: [{ id: 10, chat_id: -1001, date: nowSec - 60, outgoing: false, sender: '星驰管家', kind: 'text', text: '@devan_m_p…', entities: [] }] },
    '/api/chats/42/info': {
      chat_id: 42,
      title: '<script>alert(1)</script> Đại ca',
      kind: 'private',
      user_id: 77,
      status: 'offline',
      online: false,
      was_online: nowSec - 900,
      verified: false,
      auto_delete_seconds: 86400,
      notification: { use_default_mute_for: false, mute_for: 2147483647, use_default_show_preview: true, show_preview: true },
      pinned: false,
      archived: false,
      marked_unread: false,
    },
    '/api/live/calls': { calls: [] },
    '/api/feed': { items: [], cursor: 0, reset: false },
  };

  const mock = `
    window.macosTestWrites = [];
    window.macosTestGets = [];
    const tinyPng = new Uint8Array([137,80,78,71,13,10,26,10,0,0,0,13,73,72,68,82,0,0,0,1,0,0,0,1,8,6,0,0,0,31,21,196,137,0,0,0,10,73,68,65,84,120,156,99,0,1,0,0,5,0,1,13,10,45,180,0,0,0,0,73,69,78,68,174,66,96,130]);
    window.fetch = async (path, options = {}) => {
      const routes = ${JSON.stringify(routes)};
      const url = new URL(path, 'http://localhost');
      if (url.pathname === '/api/file/55') {
        return new Response(new Blob([tinyPng], { type: 'image/png' }), { status: 200 });
      }
      if ((options.method || 'GET') === 'GET') {
        window.macosTestGets.push(url.pathname + url.search);
      }
      let payload = routes[url.pathname] || {};
      if (url.pathname === '/api/chats/42/messages' && url.searchParams.get('q')) {
        const q = url.searchParams.get('q').toLowerCase();
        const found = routes['/api/chats/42/messages'].messages.filter((m) => (m.text || m.summary || '').toLowerCase().includes(q));
        payload = { messages: found, total_count: found.length };
      }
      if ((options.method || 'GET') === 'POST') {
        const data = JSON.parse(options.body || '{}');
        window.macosTestWrites.push([url.pathname, data, options.headers['X-Requested-With'], url.searchParams.get('account')]);
        if (url.pathname.endsWith('/settings')) payload = { ok: true, ...data };
        else if (url.pathname.endsWith('/clear_history')) payload = { ok: true, deleted: true };
        else if (url.pathname.endsWith('/delete')) payload = { ok: true, deleted: true };
      }
      if (url.pathname === '/api/feed') await new Promise((resolve) => setTimeout(resolve, 400));
      return new Response(JSON.stringify(payload));
    };
  `;

  const profile = mkdtempSync(`${require('node:os').tmpdir()}/tele-macos-browser-`);
  const browser = spawn(process.env.CHROME_BIN || 'google-chrome', [
    '--headless', '--no-sandbox', '--disable-gpu', '--no-first-run', '--disable-extensions',
    '--no-proxy-server', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank',
  ]);
  t.after(async () => {
    if (browser.exitCode === null && browser.signalCode === null) { browser.kill(); await once(browser, 'exit'); }
    try { rmSync(profile, { recursive: true, force: true, maxRetries: 20, retryDelay: 150 }); } catch { /* dọn thư mục tạm không được làm hỏng kết quả test: Chrome còn ghi sau khi thoát; hồ sơ nằm ở thư mục tạm của hệ điều hành */ }
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

  const page = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((target) => target.type === 'page'); // KHÔNG lấy mục đầu: Chrome bản mới liệt kê trang nền của tiện ích trước
  assert.ok(page, 'không thấy mục tiêu type=page');
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
  const writes = () => evaluate('window.macosTestWrites');
  const dimension = (width) => cdp('Emulation.setDeviceMetricsOverride', { width, height: 844, deviceScaleFactor: 1, mobile: width < 700 });

  await cdp('Runtime.enable');
  await dimension(1440);
  const { frameTree } = await cdp('Page.getFrameTree');
  const script = /<script>([\s\S]*?)<\/script>/.exec(html);
  await openHttpOrigin(cdp, ws, html.replace(script[0], ''));
  await evaluate(mock + script[1]);
  await until('document.querySelectorAll("#list .chat").length === 6');
  await until('Boolean(document.querySelector("#list .p-thumb img"))');

  // 1. Kiểm tra danh sách ở 1440 px: đủ huy hiệu, 14682 không cắt 999+, @, muted, 1ng, verified, online, thumbnail, màu đỏ nhỡ, chống XSS.
  const listChecks = await evaluate(`(() => {
    const rows = [...document.querySelectorAll('#list .chat')];
    return {
      scripts: document.querySelectorAll('#list script').length,
      unread0: rows[0].querySelector('.badge')?.textContent,
      muted0: rows[0].querySelector('.badge')?.classList.contains('mute'),
      mention0: rows[0].querySelector('.mention-badge')?.textContent,
      muteIcon0: Boolean(rows[0].querySelector('.muted-icon')),
      sender0: rows[0].querySelector('.p-sender')?.textContent,
      ad1: rows[1].querySelector('.ad-badge')?.textContent,
      call1: rows[1].querySelector('.p')?.textContent,
      thumb2: Boolean(rows[2].querySelector('.p-thumb img')),
      cap2: rows[2].querySelector('.p')?.textContent,
      verified3: Boolean(rows[3].querySelector('.verified-badge')),
      online3: Boolean(rows[3].querySelector('.online-dot')),
      pin3: Boolean(rows[3].querySelector('.pin-badge')),
      svc4: rows[4].querySelector('.p')?.classList.contains('preview-service'),
      missed5: rows[5].querySelector('.p')?.classList.contains('is-missed'),
      overflow: document.documentElement.scrollWidth > window.innerWidth,
    };
  })()`);
  assert.equal(listChecks.scripts, 0);
  assert.equal(listChecks.unread0, '14682');
  assert.equal(listChecks.muted0, true);
  assert.equal(listChecks.mention0, '@');
  assert.equal(listChecks.muteIcon0, true);
  assert.equal(listChecks.sender0, '星驰管家: ');
  assert.match(listChecks.ad1, /1ng/);
  assert.match(listChecks.call1, /Cuộc gọi video đến \(18 giây\)/);
  assert.equal(listChecks.thumb2, true);
  assert.match(listChecks.cap2, /Phượt đêm/);
  assert.equal(listChecks.verified3, true);
  assert.equal(listChecks.online3, true);
  assert.equal(listChecks.pin3, true);
  assert.equal(listChecks.svc4, true);
  assert.equal(listChecks.missed5, true);
  assert.equal(listChecks.overflow, false);

  // 2. Mở hội thoại 42 và mở ngăn Thông tin ở 1440 px -> kiểm 3 cột desktop macOS không tràn.
  await evaluate('document.querySelectorAll("#list .chat")[1].click()');
  await until('document.querySelectorAll("#msgs .m, #msgs .m-service").length === 4');
  await click('chat-info-open');
  await until('!document.getElementById("chat-info-drawer").hidden');

  const desktopCols = await evaluate(`(() => {
    const aside = document.querySelector('aside').getBoundingClientRect();
    const main = document.querySelector('main').getBoundingClientRect();
    const info = document.getElementById('chat-info-drawer').getBoundingClientRect();
    return {
      asideW: aside.width,
      mainW: main.width,
      infoW: info.width,
      noOverlap1: aside.right <= main.left + 1,
      noOverlap2: main.right <= info.left + 1,
      headSub: document.getElementById('head-sub').textContent,
      headAd: document.getElementById('head-autodelete').textContent,
      dayText: document.querySelector('#msgs .day')?.textContent,
      callTitle: document.querySelector('#msgs .m-call .call-title')?.textContent,
      callTime: document.querySelector('#msgs .m-call .call-time')?.textContent,
      missedBubble: Boolean(document.querySelector('#msgs .m-call.call-missed')),
      svcBubble: document.querySelector('#msgs .m-service')?.textContent,
      readTick: document.querySelector('#msgs .msg-tick.read')?.textContent,
      msgScripts: document.querySelectorAll('#msgs script').length,
      overflow: document.documentElement.scrollWidth > window.innerWidth,
    };
  })()`);
  assert.equal(desktopCols.noOverlap1, true);
  assert.equal(desktopCols.noOverlap2, true);
  assert.ok(desktopCols.asideW > 0 && desktopCols.mainW > 0 && desktopCols.infoW > 0);
  assert.equal(desktopCols.headSub, 'hoạt động 15 phút trước');
  assert.match(desktopCols.headAd, /1ng/);
  assert.match(desktopCols.dayText, /\d+ tháng \d+/);
  assert.equal(desktopCols.callTitle, 'Cuộc gọi video đến');
  assert.match(desktopCols.callTime, /19 giây/);
  assert.equal(desktopCols.missedBubble, true);
  assert.match(desktopCols.svcBubble, /Bạn đã đặt hẹn giờ tự hủy/);
  assert.equal(desktopCols.readTick, '✓✓');
  assert.equal(desktopCols.msgScripts, 0);
  assert.equal(desktopCols.overflow, false);
  await click('chat-info-close');

  // 3. Tìm kiếm trong chat & menu ⋮ (Bật âm, Xoá lịch sử & Xoá chat có confirm).
  await click('chat-search-open');
  await until('!document.getElementById("chat-search-bar").hidden');
  await evaluate(`{
    const inp = document.getElementById('chat-search-input');
    inp.value = 'Xin chào';
    inp.dispatchEvent(new Event('input'));
  }`);
  await until('document.getElementById("chat-search-count").textContent.includes("1/1")');
  assert.equal(await evaluate('document.querySelectorAll("#chat-search-results script").length'), 0);
  await click('chat-search-close');

  await click('chat-menu-open');
  await until('!document.getElementById("chat-ctx-menu").hidden');
  await click('ctx-mute');
  await until('window.macosTestWrites.some((w) => w[0] === "/api/chats/42/settings" && w[1].notification?.mute_for === 0)');

  const writesBeforeClear = (await writes()).length;
  await click('chat-menu-open');
  await click('ctx-clear-history');
  await until('document.getElementById("ci-confirm-dialog").open === true');
  assert.equal((await writes()).length, writesBeforeClear);
  await click('ci-confirm-cancel');
  await click('chat-menu-open');
  await click('ctx-clear-history');
  await until('document.getElementById("ci-confirm-dialog").open === true');
  await click('ci-confirm-ok');
  await until('window.macosTestWrites.some((w) => w[0] === "/api/chats/42/clear_history" && w[1].confirm === true)');

  const writesBeforeDel = (await writes()).length;
  await click('chat-menu-open');
  await click('ctx-delete-chat');
  await until('document.getElementById("ci-confirm-dialog").open === true');
  assert.equal((await writes()).length, writesBeforeDel);
  await click('ci-confirm-cancel');
  await click('chat-menu-open');
  await click('ctx-delete-chat');
  await until('document.getElementById("ci-confirm-dialog").open === true');
  await click('ci-confirm-ok');
  await until('window.macosTestWrites.some((w) => w[0] === "/api/chats/42/delete" && w[1].confirm === true)');

  // 4. Kiểm tra 700 px và 390 px: không tràn ngang, đủ thành phần danh sách và khung chat.
  await dimension(700);
  assert.equal(await evaluate('document.documentElement.scrollWidth <= window.innerWidth'), true, '700 px không tràn ngang');
  await dimension(390);
  assert.equal(await evaluate('document.documentElement.scrollWidth <= window.innerWidth'), true, '390 px (danh sách) không tràn ngang');
  await evaluate('document.querySelector("#list .chat").click()');
  await until('document.getElementById("app").classList.contains("chatting")');
  assert.equal(await evaluate('document.documentElement.scrollWidth <= window.innerWidth'), true, '390 px (khung chat) không tràn ngang');

  assert.equal(await evaluate('localStorage.length + sessionStorage.length'), 0);
  assert.deepEqual(errors, []);
});
