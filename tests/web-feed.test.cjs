const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const vm = require('node:vm');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const feedCode = html.slice(html.indexOf('  // ---- cập nhật theo thời gian thực'), html.indexOf('  // ---- tìm kiếm, tab'));

const flush = () => new Promise(setImmediate);
const response = (cursor, items = [], reset = false) => ({ ok: true, json: async () => ({ cursor, items, reset }) });

async function start(reply, source = feedCode) {
  const document = Object.assign(new EventTarget(), { hidden: false });
  const window = new EventTarget();
  const calls = [], appended = [], refreshed = [], timers = new Map();
  let now = 0, nextTimer = 0, active = 0, peak = 0;
  async function fetch(path, { signal, credentials, headers = {} }) {
    const url = new URL(path, 'http://localhost');
    const call = { credentials, headers, wait: Number(url.searchParams.get('wait')), since: Number(url.searchParams.get('since')), signal, time: now };
    calls.push(call);
    peak = Math.max(peak, ++active);
    try {
      if (calls.length === 1) return response(7);
      const result = reply(call, calls.length);
      if (result) return result;
      return await new Promise((_, reject) => signal?.addEventListener('abort', () => reject(signal.reason), { once: true }));
    } finally { active--; }
  }
  vm.runInNewContext(source + '\npump();', {
    document, window, fetch, AbortController, accountPath: (p) => p, showGate() {}, cur: 5, account: '',
    setTimeout: (fn, delay) => { timers.set(++nextTimer, { fn, time: now + delay }); return nextTimer; },
    clearTimeout: (id) => timers.delete(id),
    appendMessages: (list, prepend) => { assert.equal(prepend, false); appended.push(...list); },
    scheduleChats() {}, loadChats: async () => refreshed.push('chats'), reloadCurrent: async () => refreshed.push('current'),
  });
  await flush();
  return {
    document, window, calls, appended, refreshed, peak: () => peak,
    async tick(ms) {
      const end = now + ms;
      for (;;) {
        const next = [...timers].sort((a, b) => a[1].time - b[1].time)[0];
        if (!next || next[1].time > end) break;
        const [id, timer] = next;
        now = timer.time; timers.delete(id); timer.fn();
        await flush();
      }
      now = end;
    },
  };
}

async function assertWake(event, source) {
  const message = { id: 42, outgoing: true };
  const app = await start(({ wait }) => wait === 0 ? response(8, [{ type: 'message', chat_id: 5, message }]) : undefined, source);
  const { document, window, calls, appended } = app;
  const last = calls.at(-1);
  assert.equal(last.wait, 20, 'a long poll is in flight');
  document.hidden = true;
  document.dispatchEvent(new Event('visibilitychange'));
  await flush();
  assert.equal(calls.at(-1), last, 'hiding leaves the long poll alone');
  document.hidden = false;
  (event === 'visibilitychange' ? document : window).dispatchEvent(new Event(event));
  await flush();
  await app.tick(1000);
  const catchUp = calls.find(({ wait, since }) => wait === 0 && since === 7);
  assert.ok(catchUp && catchUp.time <= 1000, 'catch-up starts within 1 second');
  assert.ok(last.signal.aborted, 'the stalled fetch was aborted');
  assert.deepEqual(appended, [message]);
  assert.equal(app.peak(), 1);
  assert.deepEqual([calls.at(-1).since, calls.at(-1).wait], [8, 20]);
}

test('TC-A1: visible, focus, online and pageshow interrupt a stalled poll and append fresh messages', async () => {
  for (const event of ['visibilitychange', 'focus', 'online', 'pageshow']) await assertWake(event);
});

test('TC-A1 mutation: removing wake abort makes TC-A1 fail', async () => {
  const mutant = feedCode.replace('controller?.abort();', '');
  assert.notEqual(mutant, feedCode, 'mutation found the wake abort');
  await assert.rejects(assertWake('visibilitychange', mutant), /catch-up starts within 1 second/);
});

test('TC-A2: 30 s deadline, capped backoff, one request at a time and no retries while hidden', async () => {
  const app = await start((_, i) => i >= 3 ? Promise.reject(new TypeError('network down')) : undefined);
  const { calls, document } = app;
  assert.equal(calls.at(-1).wait, 20);
  await app.tick(29999);
  assert.equal(calls.at(-1).signal?.aborted ?? false, false);
  await app.tick(1);
  assert.ok(calls.at(-1).signal?.aborted, 'the deadline aborts a hung fetch');
  await app.tick(13000);
  assert.deepEqual(calls.slice(2).map(({ time }) => time), [31000, 33000, 38000, 43000]);
  assert.equal(app.peak(), 1);
  document.hidden = true;
  document.dispatchEvent(new Event('visibilitychange'));
  const before = calls.length;
  await app.tick(60000);
  assert.equal(calls.length, before);
});

test('TC-A3: reset reloads the list and open conversation once and resumes from the new cursor', async () => {
  const app = await start((_, i) => i === 3 ? response(99, [], true) : undefined);
  app.window.dispatchEvent(new Event('pageshow'));
  await flush();
  assert.deepEqual(app.refreshed, ['chats', 'current']);
  assert.deepEqual([app.calls.at(-1).since, app.calls.at(-1).wait], [99, 20]);
});

test('TC-A4: luồng tin dùng cookie phiên (không Bearer), có tiêu đề chống CSRF', async () => {
  const { calls } = await start(() => undefined);
  const first = calls[0];
  assert.equal(first.credentials, 'same-origin');
  assert.equal(first.headers['X-Requested-With'], 'fetch');
  assert.equal(first.headers.Authorization, undefined, 'không gửi token qua tiêu đề Authorization');
});
