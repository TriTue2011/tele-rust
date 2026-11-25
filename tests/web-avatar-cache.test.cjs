// Số file id của TDLib chỉ có nghĩa trong MỘT tài khoản: cùng số ở hai tài khoản là hai ảnh khác nhau (đo thật 22/11: id 1248 ⇒ 12.898 B ở TK A
// và 8.778 B ở TK B). Bộ nhớ đệm blob phải khoá theo (tài khoản, file id), nếu không đổi tài khoản hiện nhầm avatar (lỗi chủ máy báo: 星驰公益 hiện logo HÀ NỘI 24H).
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');

const html = readFileSync(path.join(__dirname, '..', 'web', 'app.html'), 'utf8');
const slice = (start, end) => { const i = html.indexOf(start); assert.ok(i !== -1, `không thấy ${start}`); const j = html.indexOf(end, i); assert.ok(j !== -1, `không thấy ${end}`); return html.slice(i, j); };

function makeContext(images) {
  const calls = [];
  const ctx = vm.createContext({
    blobs: new Map(), account: '', calls,
    URL: { createObjectURL: (b) => 'blob:' + b.tag },
    fetch: async (url) => { calls.push(url); const m = /account=([^&]*)/.exec(url); const acct = m ? decodeURIComponent(m[1]) : ''; const id = /\/api\/file\/(\d+)/.exec(url)[1]; return { ok: true, blob: async () => ({ tag: `${acct}#${id}:${images[`${acct}#${id}`]}` }) }; },
    encodeURIComponent,
  });
  vm.runInContext(slice('function accountPath(path) {', '\n  }\n') + '\n  }\n' + slice('async function blobUrl(fileId) {', '\n  const avQueue'), ctx);
  return ctx;
}

test('cùng file id ở hai tài khoản KHÔNG được dùng chung ảnh trong bộ nhớ đệm', async () => {
  const ctx = makeContext({ '+84111#1248': 'logo-A', '+84222#1248': 'xingchi-B' });
  ctx.account = '+84111';
  const a = await vm.runInContext('blobUrl(1248)', ctx);
  ctx.account = '+84222';
  const b = await vm.runInContext('blobUrl(1248)', ctx);
  assert.match(a, /logo-A/);
  assert.match(b, /xingchi-B/, 'đổi tài khoản phải tải lại đúng ảnh của tài khoản mới');
  assert.equal(ctx.calls.length, 2);
  ctx.account = '+84111';
  assert.match(await vm.runInContext('blobUrl(1248)', ctx), /logo-A/);
  assert.equal(ctx.calls.length, 2, 'cùng tài khoản + cùng file id thì dùng lại bộ nhớ đệm (không tải lại)');
});

test('404 (tệp chắc chắn không có thật) KHÔNG gọi lại — lỗi khác (mạng/5xx) vẫn thử lại lần vẽ sau', async () => {
  // Đo thật trên Network tab 25/11: cùng 3 file_id của video/ảnh tự gửi bị gọi lại hàng chục lần/phút (mỗi lần tin render
  // lại), kéo cả app bị giới hạn 429 theo — vì blobUrl() XOÁ khỏi đệm MỌI lỗi, kể cả 404 chắc chắn không đổi kết quả.
  const calls = [];
  let status = 404;
  const ctx = vm.createContext({ blobs: new Map(), account: '+84111', calls, URL: { createObjectURL: (b) => 'blob:' + b }, encodeURIComponent,
    fetch: async (url) => { calls.push(url); return { ok: status === 200, status, blob: async () => 'ok' }; } });
  vm.runInContext(slice('function accountPath(path) {', '\n  }\n') + '\n  }\n' + slice('async function blobUrl(fileId) {', '\n  const avQueue'), ctx);
  await assert.rejects(vm.runInContext('blobUrl(999)', ctx));
  await assert.rejects(vm.runInContext('blobUrl(999)', ctx));
  await assert.rejects(vm.runInContext('blobUrl(999)', ctx));
  assert.equal(calls.length, 1, "404 chỉ được gọi fetch ĐÚNG MỘT LẦN, các lần sau dùng lại promise đã hỏng");
  // lỗi 500 (có thể tạm thời) thì khác — vẫn cho thử lại ở lần vẽ sau.
  calls.length = 0; status = 500;
  await assert.rejects(vm.runInContext('blobUrl(1000)', ctx));
  await assert.rejects(vm.runInContext('blobUrl(1000)', ctx));
  assert.equal(calls.length, 2, "lỗi khác 404 vẫn phải thử lại, không khoá cứng như 404");
});

test('đột biến: khoá đệm chỉ theo file id thì test phải đỏ', async () => {
  const mutated = html.replace('const key = account + "|" + fileId;', 'const key = String(fileId);');
  assert.notEqual(mutated, html);
  const ctx = vm.createContext({ blobs: new Map(), account: '+84111', calls: [], URL: { createObjectURL: (b) => 'blob:' + b.tag }, encodeURIComponent,
    fetch: async (url) => { const acct = decodeURIComponent(/account=([^&]*)/.exec(url)[1]); return { ok: true, blob: async () => ({ tag: acct }) }; } });
  const sl = (s, e) => { const i = mutated.indexOf(s), j = mutated.indexOf(e, i); return mutated.slice(i, j); };
  vm.runInContext(sl('function accountPath(path) {', '\n  }\n') + '\n  }\n' + sl('async function blobUrl(fileId) {', '\n  const avQueue'), ctx);
  const a = await vm.runInContext('blobUrl(5)', ctx); ctx.account = '+84222'; const b = await vm.runInContext('blobUrl(5)', ctx);
  assert.equal(a, b, 'bản đột biến phải lộ lỗi dùng chung ảnh');
});
