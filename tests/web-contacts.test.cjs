const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const vm = require('node:vm');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const code = html.slice(html.indexOf('  // ---- danh bạ'), html.indexOf('  // ---- bộ lọc và chuyển tab'));

class Element extends EventTarget {
  constructor(tag, className = '', text = '') {
    super();
    this.tag = tag;
    this.className = className;
    this.textContent = text;
    this.children = [];
  }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  setAttribute() {}
  set innerHTML(_) { assert.fail('Dữ liệu danh bạ phải được dựng bằng textContent'); }
  find(predicate) {
    return this.children.flatMap((child) => [...(predicate(child) ? [child] : []), ...child.find(predicate)]);
  }
}

function setup(cookies) {
  const nodes = new Map();
  const document = new EventTarget();
  Object.defineProperty(document, 'cookie', cookies);
  const context = vm.createContext({
    $: (id) => { if (!nodes.has(id)) nodes.set(id, new Element('div')); return nodes.get(id); },
    el: (...args) => new Element(...args), avatar: () => new Element('img'),
    hhmm: () => '21:40', dayLabel: () => 'Hôm qua', document, location: { protocol: 'https:' },
  });
  return vm.runInContext(`${code}\n({ renderContactRows, readContactSort, saveContactSort });`, context);
}

test('TC-6: danh bạ nhóm theo chữ cái, giữ tên là văn bản và có thao tác theo trạng thái chặn', () => {
  const { renderContactRows } = setup({ get: () => '' });
  const box = new Element('div');
  const data = [
    { id: 1, name: 'Ánh<script>', section: 'A', status: { kind: 'online', text: 'đang trực tuyến' } },
    { id: 2, name: 'anh', section: 'A', status: { kind: 'offline', last_seen: 100 } },
    { id: 3, name: '☆☆', section: '#', blocked: true, status: { kind: 'hidden', text: 'Không rõ' } },
  ];
  renderContactRows(box, data, true);
  assert.deepEqual(box.find((e) => e.className === 'contact-section').map((e) => e.textContent), ['A', '#']);
  assert.equal(box.find((e) => e.tag === 'script').length, 0);
  assert.ok(box.find((e) => e.textContent === 'Ánh<script>').length);
  assert.equal(box.find((e) => e.className === 'contact-status online')[0].textContent, 'đang trực tuyến');
  assert.equal(box.find((e) => e.className === 'contact-status')[0].textContent, 'hôm qua lúc 21:40');
  const menu = (n) => box.find((e) => e.tag === 'details')[n].find((e) => e.tag === 'button').map((e) => e.textContent);
  assert.deepEqual(menu(0), ['Nhắn tin', 'Gọi', 'Sửa', 'Xoá', 'Chặn']);
  assert.deepEqual(menu(2), ['Nhắn tin', 'Gọi', 'Sửa', 'Xoá', 'Bỏ chặn']);
  assert.match(html, /class="[^"]*contact[^"]*"[^>]*>[\s\S]*Báo spam[\s\S]*Xoá hội thoại/);
  assert.match(html, /\.contact-status\.online\s*\{[^}]*color:/);
  renderContactRows(box, data, false);
  assert.equal(box.find((e) => e.className === 'contact-section').length, 0);
});

test('TC-6: trình duyệt nhớ cách xếp và mặc định an toàn khi chặn lưu trữ', () => {
  let cookie = 'another_preference=name';
  const cookies = { get: () => cookie, set: (value) => {
    assert.match(value, /; SameSite=Strict; Secure$/);
    cookie += '; ' + value.split(';')[0];
  } };
  assert.equal(setup(cookies).readContactSort(), 'last_seen');
  setup(cookies).saveContactSort('name');
  assert.equal(setup(cookies).readContactSort(), 'name');
  const denied = () => { throw new Error('cookies disabled'); };
  const fallback = setup({ get: denied, set: denied });
  assert.equal(fallback.readContactSort(), 'last_seen');
  assert.doesNotThrow(() => fallback.saveContactSort('name'));
});
