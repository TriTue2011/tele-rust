// Hình cuộc gọi: luồng MJPEG trong <img> (không hỏi ảnh chụp lần lượt) và cuộc gọi THOẠI không bắt chọn camera.
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const vm = require('node:vm');
const { test } = require('node:test');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const slice = (from, to) => {
  const a = html.indexOf(from), b = html.indexOf(to, a);
  assert.ok(a > 0 && b > a, `không thấy đoạn ${from}`);
  return html.slice(a, b);
};

test('TC-CALL-VIEW: hình bên kia là luồng MJPEG, đứt luồng thì thử lại, không dùng vòng hỏi ảnh chụp', async () => {
  const code = slice('  function openVideoStream(state) {', '  function hideCall() {');
  assert.ok(!code.includes('/api/live/snapshot/'), 'không được quay về vòng hỏi ảnh chụp');
  const img = { hidden: true, src: '', removeAttribute() { this.src = ''; } };
  const wait = { hidden: false, textContent: '' };
  const els = { 'call-image': img, 'call-wait': wait };
  const ctx = { $: (id) => els[id], accountPath: (p) => p + '?account=%2B84', snapshot: null, stopSnapshot() {}, AbortController, Date, setTimeout, clearTimeout, encodeURIComponent };
  vm.createContext(ctx);
  vm.runInContext(code + '\nthis.start = startSnapshot;', ctx);
  ctx.start('c1');
  assert.match(img.src, /^\/api\/live\/view\/c1\?account=%2B84&_=\d+$/, img.src);
  img.onload(); assert.equal(img.hidden, false); assert.equal(wait.hidden, true, 'có khung đầu thì hiện hình');
  const first = img.src;
  img.onerror();                                                // gateway chưa có hình / đứt luồng
  assert.equal(img.hidden, true); assert.equal(wait.hidden, false);
  await new Promise((resolve) => setTimeout(resolve, 1700));
  assert.notEqual(img.src, first, 'sau 1,5 s phải mở lại luồng với địa chỉ mới');
  assert.match(img.src, /\/api\/live\/view\/c1\?account=%2B84&_=\d+$/);
});

test('TC-CALL-VOICE: gọi thoại ẩn ô camera, gọi video hiện', () => {
  const code = slice('  function setPickerVideo(video) {', '  function renderDevicePicker');
  const label = { hidden: false }, select = { hidden: false }, help = { hidden: false }, url = { hidden: false };
  const els = { 'device-camera': select, 'device-camera-help': help };
  const ctx = { document: { querySelector: () => label }, $: (id) => els[id], pickerUrls: { camera: url }, updateDeviceUrl: () => { ctx.updated = true; } };
  vm.createContext(ctx);
  vm.runInContext(code + '\nthis.setPickerVideo = setPickerVideo;', ctx);
  ctx.setPickerVideo(false);
  assert.deepEqual([label.hidden, select.hidden, help.hidden, url.hidden], [true, true, true, true], 'gọi thoại: không hiện camera');
  ctx.setPickerVideo(true);
  assert.deepEqual([label.hidden, select.hidden, help.hidden], [false, false, false], 'gọi video: hiện camera');
  assert.equal(ctx.updated, true);
  assert.ok(/#call-window\.voice-call/.test(html) && html.includes('classList.toggle("voice-call", !video)'), 'cửa sổ thoại gọn');
});
