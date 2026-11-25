// Chủ máy báo 25/11: "tele không kéo xuống được mà cứ về đầu" — gốc là reloadSoon() (gọi khi MỘT video/ảnh lỗi tải, vd
// tệp tự gửi 404, hoặc một tin message_changed) từng gọi thẳng reloadCurrent() = dựng lại CẢ danh sách + ép cuộn xuống đáy,
// xoá sạch vị trí người dùng đã cuộn lên xem. Sửa: chỉ làm mới ĐÚNG tin nhắn đổi, giữ nguyên scrollTop.
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const vm = require('node:vm');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const code = html.slice(html.indexOf('  function onFeed(it) {'), html.indexOf('\n\n  // ---- đổi độ rộng'));

function start(messagesById, source = code) {
  const calls = [], rendered = new Map(), msgData = new Map(), timers = new Map();
  let nextTimer = 0, scrollTop = 500; // giả lập người dùng đã cuộn lên xem lịch sử (không phải 0, không phải ở đáy)
  const msgsEl = { get scrollTop() { return scrollTop; }, set scrollTop(v) { scrollTop = v; } };
  const nodeFor = (id) => ({ id, replaceWith(n) { rendered.set(id, n); } });
  for (const id of Object.keys(messagesById).map(Number)) rendered.set(id, nodeFor(id));
  const ctx = {
    cur: 5, account: '', document: { hidden: false },
    $: (id) => (id === 'msgs' ? msgsEl : { textContent: '' }),
    api: async (method, path) => { calls.push(path); return { messages: Object.values(messagesById) }; },
    renderMessage: (m) => ({ __node: true, id: m.id }),
    rendered, msgData,
    appendMessages() {}, markRead() {}, scheduleChats() {}, refreshInteractionsSoon() {}, removeMsg() {}, watchCall() {}, toast() {}, markCommentsPost() {},
    applyChatAutoDeleteUpdate() {}, applyChatReadOutboxUpdate() {},
    setTimeout: (fn, delay) => { const id = ++nextTimer; timers.set(id, { fn, delay }); return id; },
    clearTimeout: (id) => timers.delete(id),
  };
  vm.createContext(ctx);
  vm.runInContext(source, ctx);
  return {
    ctx, calls, rendered, msgData, msgsEl,
    onFeed: (it) => vm.runInContext('onFeed', ctx)(it),
    async runTimers() { for (const [id, t] of [...timers]) { timers.delete(id); t.fn(); } await new Promise((r) => setImmediate(r)); },
  };
}

test('TELE-RELOAD-SCROLL: message_changed chỉ vẽ lại ĐÚNG tin đó tại chỗ, giữ nguyên scrollTop (không tụt về đáy)', async () => {
  const app = start({ 9: { id: 9, text: 'bản sửa mới' } });
  app.onFeed({ type: 'message_changed', chat_id: 5, message_id: 9 });
  assert.equal(app.calls.length, 0, 'chưa gọi mạng ngay (còn gộp 600ms)');
  await app.runTimers();
  assert.deepEqual(app.calls, ['/api/chats/5/messages?limit=40'], 'chỉ tải lại DANH SÁCH để lấy bản mới của tin đó, không mở lại cả cuộc trò chuyện (openChat)');
  assert.equal(app.msgsEl.scrollTop, 500, 'scrollTop giữ nguyên như trước — KHÔNG bị ép về đáy/về đầu');
  assert.equal(app.msgData.get(9).text, 'bản sửa mới', 'nội dung tin đã cập nhật');
});

test('TELE-RELOAD-SCROLL: hai sự kiện đổi tin gần nhau gộp thành MỘT lần tải lại (debounce 600ms)', async () => {
  const app = start({ 9: { id: 9 }, 11: { id: 11 } });
  app.onFeed({ type: 'message_changed', chat_id: 5, message_id: 9 });
  app.onFeed({ type: 'message_changed', chat_id: 5, message_id: 11 });
  await app.runTimers();
  assert.equal(app.calls.length, 1, 'gộp lại, không gọi mạng 2 lần cho 2 sự kiện gần nhau');
  assert.ok(app.msgData.has(9) && app.msgData.has(11), 'cả hai tin đều được cập nhật trong một lượt');
});

test('TELE-RELOAD-SCROLL: tin đổi không còn trong 40 tin mới nhất (đã cuộn quá xa) thì bỏ qua, không lỗi', async () => {
  const app = start({ 1: { id: 1 } }); // chỉ tin id=1 được trả về, không có id=9
  app.rendered.set(9, { id: 9 }); // id=9 có render nhưng server không trả lại (ngoài limit 40)
  app.onFeed({ type: 'message_changed', chat_id: 5, message_id: 9 });
  await app.runTimers();
  assert.equal(app.msgData.has(9), false, 'không có gì để cập nhật, không ném lỗi');
});

test('đột biến: bỏ "if (!ids.has(m.id)) continue" thì mọi tin trong 40 tin đều bị vẽ lại (không chỉ tin vừa đổi)', async () => {
  const mutated = code.replace('if (!ids.has(m.id)) continue;\n', '');
  assert.notEqual(mutated, code);
  const app = start({ 9: { id: 9 }, 20: { id: 20 } }, mutated);
  app.onFeed({ type: 'message_changed', chat_id: 5, message_id: 9 });
  await app.runTimers();
  assert.ok(app.msgData.has(20), 'bản đột biến vẽ lại CẢ tin id=20 dù chỉ id=9 đổi — lộ đúng lỗi nếu bỏ dòng lọc');
});
