const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { mkdtempSync, readFileSync, rmSync } = require('node:fs');
const { once } = require('node:events');
const { test } = require('node:test');
const { openHttpOrigin } = require('./chrome-origin.cjs');

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

test('TC-CHANNEL-COMMENTS: hàng bình luận kênh và ngăn bình luận ở 390/700/1440 px', { timeout: 30000 }, async (t) => {
  for (const banned of ['innerHTML', 'outerHTML', 'insertAdjacentHTML', 'document.write', 'localStorage', 'sessionStorage', 'gw_token', 'GATEWAY_TOKEN']) {
    assert.equal(html.includes(banned), false, banned);
  }

  const routes = {
    '/ui/api/auth/status': { dang_nhap: true, da_thiet_lap: true },
    '/api/accounts': { tai_khoan: [{ phone: '+84900000000', status: 'online', display_name: 'An Lê' }] },
    '/api/me': { id: 7, name: 'An Lê', phone: '84900000000' },
    '/api/settings/appearance': { font_family: 'system', font_size: 15, theme: 'system', bubble_style: 'classic', accent: 'blue' },
    '/api/settings/storage': storagePolicy,
    '/api/storage/usage': { total_bytes: 0, by_type: {}, top_chats: [] },
    '/api/chats': {
      chats: [
        { id: -100100, title: 'Kênh Tin Tức Mẫu', kind: 'channel', member_count: 520, pinned: false, muted: false },
        { id: 42, title: 'Bình Trần', kind: 'private', user_id: 9, pinned: false, muted: false },
      ],
    },
    '/api/chats/-100100/messages': {
      messages: [
        {
          id: 10,
          chat_id: -100100,
          date: 1700000000,
          outgoing: false,
          sender: 'Kênh Tin Tức Mẫu',
          kind: 'text',
          text: 'Bản tin kênh có 5 bình luận',
          entities: [],
          summary: 'Bản tin kênh có 5 bình luận',
          reply_to: 0,
          reactions: [],
          file: null,
          comments: {
            count: 5,
            repliers: [
              { id: 1, name: 'An Lê', avatar_file: null },
              { id: 2, name: '<script>xss()</script> Bình Trần', avatar_file: null },
              { id: 3, name: 'Chi Phạm', avatar_file: null },
            ],
            unread: true,
          },
        },
        {
          id: 11,
          chat_id: -100100,
          date: 1700000010,
          outgoing: false,
          sender: 'Kênh Tin Tức Mẫu',
          kind: 'text',
          text: 'Bản tin kênh không bật bình luận',
          entities: [],
          summary: 'Bản tin kênh không bật bình luận',
          reply_to: 0,
          reactions: [],
          file: null,
        },
        {
          id: 12,
          chat_id: -100100,
          date: 1700000015,
          outgoing: false,
          sender: 'Kênh Tin Tức Mẫu',
          kind: 'text',
          text: 'Bản tin lỗi không có nhóm thảo luận',
          entities: [],
          summary: 'Bản tin lỗi không có nhóm thảo luận',
          reply_to: 0,
          reactions: [],
          file: null,
          comments: { count: 1, repliers: [], unread: false },
        },
      ],
    },
    '/api/chats/42/messages': {
      messages: [
        { id: 201, chat_id: 42, date: 1700000020, outgoing: false, sender: 'Bình Trần', kind: 'text', text: 'Tin cá nhân', entities: [] },
      ],
    },
    '/api/live/calls': { calls: [] },
    '/api/feed': { items: [], cursor: 0, reset: false },
  };

  const mock = `
    window.commentsTestCalls = [];
    window.fetch = async (path, options = {}) => {
      const routes = ${JSON.stringify(routes)};
      const url = new URL(path, 'http://localhost');
      const method = (options.method || 'GET').toUpperCase();
      window.commentsTestCalls.push({
        method,
        pathname: url.pathname,
        search: url.search,
        body: options.body ? JSON.parse(options.body) : null,
        csrf: options.headers ? options.headers['X-Requested-With'] : null,
        account: url.searchParams.get('account'),
      });
      let status = 200;
      let payload = routes[url.pathname] || {};
      if (url.pathname === '/api/chats/-100100/messages/10/comments' && method === 'GET') {
        const from = Number(url.searchParams.get('from') || 0);
        if (from === 502) {
          payload = {
            discussion_chat_id: -100200,
            thread_id: 500,
            root_message_id: 500,
            has_more: false,
            messages: [
              { id: 501, chat_id: -100200, date: 1700000002, outgoing: false, sender: 'An Lê', kind: 'text', text: 'Bình luận cũ nhất', entities: [], reply_to: 500 },
            ],
          };
        } else {
          payload = {
            discussion_chat_id: -100200,
            thread_id: 500,
            root_message_id: 500,
            has_more: true,
            messages: [
              { id: 502, chat_id: -100200, date: 1700000004, outgoing: false, sender: '<img src=x onerror=alert(1)> Bình Trần', kind: 'text', text: 'Ý kiến đầu tiên <script>bad()</script>', entities: [], reply_to: 500 },
              { id: 503, chat_id: -100200, date: 1700000008, outgoing: false, sender: 'Chi Phạm', kind: 'text', text: 'Đồng tình với nhận xét trên', entities: [], reply_to: 502 },
            ],
          };
        }
      } else if (url.pathname === '/api/chats/-100100/messages/10/comments' && method === 'POST') {
        const data = JSON.parse(options.body || '{}');
        payload = {
          id: 504,
          chat_id: -100200,
          date: 1700000030,
          outgoing: true,
          sender: '',
          kind: 'text',
          text: data.text,
          entities: data.entities || [],
          reply_to: data.reply_to || 500,
        };
      } else if (url.pathname === '/api/chats/-100100/messages/11/comments' || url.pathname === '/api/chats/-100100/messages/12/comments') {
        status = 409;
        payload = { error: { code: 'NO_DISCUSSION', message: 'Kênh này chưa liên kết nhóm thảo luận.' } };
      } else if (url.pathname === '/api/feed') {
        await new Promise((resolve) => setTimeout(resolve, 400));
      }
      return new Response(JSON.stringify(payload), { status, headers: { 'Content-Type': 'application/json' } });
    };
  `;

  const profile = mkdtempSync(`${require('node:os').tmpdir()}/tele-comments-browser-`);
  const browser = spawn(process.env.CHROME_BIN || 'google-chrome', [
    '--headless', '--no-sandbox', '--disable-gpu', '--no-first-run', '--disable-extensions',
    '--no-proxy-server', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank',
  ]);
  t.after(async () => {
    if (browser.exitCode === null && browser.signalCode === null) { browser.kill(); await once(browser, 'exit'); }
    try { rmSync(profile, { recursive: true, force: true, maxRetries: 20, retryDelay: 150 }); } catch { /* dọn thư mục tạm không được làm hỏng kết quả test */ }
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

  const page = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((target) => target.type === 'page');
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
  const dimension = (width) => cdp('Emulation.setDeviceMetricsOverride', { width, height: 844, deviceScaleFactor: 1, mobile: width < 700 });

  await cdp('Runtime.enable');
  await dimension(390);
  const script = /<script>([\s\S]*?)<\/script>/.exec(html);
  await openHttpOrigin(cdp, ws, html.replace(script[0], ''));
  await evaluate(mock + script[1]);
  await until('Boolean(document.querySelector("#list .chat"))');

  // 1. Mở kênh ở 390 px: bài 10 có hàng «5 bình luận ›» + 3 avatar + chấm xanh; bài 11 không có.
  await evaluate('document.querySelector("#list .chat").click()');
  await until('document.querySelectorAll("#msgs .m").length === 3');
  const postBarCheck = await evaluate(`(() => {
    const msgs = [...document.querySelectorAll('#msgs .m')];
    const bar0 = msgs[0].querySelector('.msg-comments-bar');
    const bar1 = msgs[1].querySelector('.msg-comments-bar');
    return {
      hasBar0: Boolean(bar0),
      hasBar1: Boolean(bar1),
      avCount: bar0 ? bar0.querySelectorAll('.comments-av').length : 0,
      countText: bar0 ? bar0.querySelector('.comments-count')?.textContent : '',
      hasUnread: Boolean(bar0 && bar0.querySelector('.comments-unread-dot')),
      noOverflow: document.documentElement.scrollWidth <= window.innerWidth,
    };
  })()`);
  assert.deepEqual(postBarCheck, {
    hasBar0: true,
    hasBar1: false,
    avCount: 3,
    countText: '5 bình luận',
    hasUnread: true,
    noOverflow: true,
  });

  // 2. Bấm hàng bình luận -> mở ngăn toàn màn hình ở 390 px, gọi GET .../comments, xoá chấm xanh.
  await evaluate('document.querySelector("#msgs .m .msg-comments-bar").click()');
  await until('!document.getElementById("comments-drawer").hidden && document.querySelectorAll("#comments-list .comment-item").length === 2');
  assert.equal(await evaluate('Math.round(document.getElementById("comments-drawer").getBoundingClientRect().width)'), 390);
  assert.equal(await evaluate('document.documentElement.scrollWidth <= window.innerWidth'), true);
  assert.equal(await evaluate('document.querySelectorAll("#comments-drawer script, #comments-drawer img").length'), 0);
  assert.equal(await evaluate('Boolean(document.querySelector("#msgs .m .comments-unread-dot"))'), false);

  // 3. Bấm «Tải thêm cũ hơn» -> gọi GET ...?from=502 và chèn bình luận 501 lên đầu.
  await click('comments-more');
  await until('document.querySelectorAll("#comments-list .comment-item").length === 3 && document.getElementById("comments-more").hidden === true');
  assert.equal(await evaluate('document.querySelector("#comments-list .comment-item").dataset.id'), '501');

  // 4. Kiểm responsive ở 700 px và 1440 px: ngăn bám phải, không tràn ngang, vùng chạm >= 44 px.
  for (const width of [700, 1440]) {
    await dimension(width);
    assert.ok(await evaluate(`(() => {
      const r = document.getElementById('comments-drawer').getBoundingClientRect();
      return window.innerWidth === ${width} && Math.round(r.width) === ${width} && Math.round(r.height) === window.innerHeight && r.right <= ${width} + 1 && document.documentElement.scrollWidth <= ${width};
    })()`), `ngăn bình luận toàn màn hình ở ${width} px`);
    assert.ok(await evaluate(`(() => {
      const d = document.getElementById('comments-drawer');
      return [...d.querySelectorAll('button')].every((b) => !b.getClientRects().length || b.getBoundingClientRect().height >= 44 || b.classList.contains('comment-reply-btn'));
    })()`), `touch target ở ${width} px`);
  }

  // 5. Gửi bình luận mới -> gọi đúng POST /api/chats/-100100/messages/10/comments và tăng số đếm lên 6.
  await evaluate(`{
    const inp = document.getElementById('comments-input');
    inp.value = 'Bình luận từ Chrome test';
    document.getElementById('comments-send').click();
  }`);
  await until('document.querySelectorAll("#comments-list .comment-item").length === 4');
  assert.equal(await evaluate('document.querySelector("#msgs .m .comments-count").textContent'), '6 bình luận');
  const postCall = await evaluate('window.commentsTestCalls.find((c) => c.method === "POST" && c.pathname === "/api/chats/-100100/messages/10/comments")');
  assert.ok(postCall);
  assert.equal(postCall.body.text, 'Bình luận từ Chrome test');
  assert.equal(postCall.csrf, 'fetch');
  assert.equal(postCall.account, '+84900000000');

  // 6. Bấm bài 12 -> lỗi 409 NO_DISCUSSION hiển thị thân thiện, khoá ô nhập.
  await evaluate('document.querySelectorAll("#msgs .m")[2].querySelector(".msg-comments-bar").click()');
  await until('document.getElementById("comments-error").textContent.includes("nhóm thảo luận")');
  assert.equal(await evaluate('document.getElementById("comments-input").disabled'), true);
  assert.equal(await evaluate('document.getElementById("comments-send").disabled'), true);

  // 7. Chuyển sang hội thoại cá nhân -> ngăn bình luận tự đóng, tin nhắn cá nhân không có hàng bình luận.
  await evaluate('document.querySelectorAll("#list .chat")[1].click()');
  await until('document.getElementById("comments-drawer").hidden === true && document.querySelectorAll("#msgs .m").length === 1');
  assert.equal(await evaluate('Boolean(document.querySelector("#msgs .m .msg-comments-bar"))'), false);

  assert.equal(await evaluate('localStorage.length + sessionStorage.length'), 0);
  assert.deepEqual(errors, []);
});
