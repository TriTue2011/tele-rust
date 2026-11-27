// Mã tệp TDLib cũ (gateway khởi động lại) ⇒ /api/file/<id> 400: KHÔNG gọi lại mã cũ mỗi lần vẽ, làm mới cuộc trò chuyện MỘT lần.
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const html = fs.readFileSync(`${__dirname}/../web/app.html`, "utf8");
const a = html.indexOf("  async function blobUrl(fileId) {"), b = html.indexOf("  const avQueue", a);
assert.ok(a > 0 && b > a, "không trích được blobUrl/refreshStaleFiles");
test("400 ⇒ giữ lỗi trong đệm + mở lại cuộc trò chuyện 1 lần (không lặp)", async () => {
  let fetches = 0, opened = 0, scheduled = 0;
  const env = {
    fetch: async () => { fetches += 1; return { ok: false, status: 400 }; },
    accountPath: (p) => p, account: "+84", blobs: new Map(), URL: { createObjectURL: () => "blob:x" },
    chats: [{ id: 7 }], cur: 7, openChat: async () => { opened += 1; }, scheduleChats: () => { scheduled += 1; },
    setTimeout: (fn) => fn(),
  };
  const f = new Function(...Object.keys(env), html.slice(a, b) + "\nreturn blobUrl;")(...Object.values(env));
  for (let i = 0; i < 3; i++) await f(1949).catch(() => {});
  await new Promise((r) => setImmediate(r));
  assert.equal(opened, 1, "mở lại cuộc trò chuyện đúng 1 lần");
  assert.equal(scheduled, 1);
  await f(1949).catch(() => {});
  await new Promise((r) => setImmediate(r));
  assert.equal(opened, 1, "trong 1 phút không mở lại nữa dù vẫn 400");
});
