const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const vm = require('node:vm');

const html = readFileSync(`${__dirname}/../web/app.html`, 'utf8');
const renderer = html.slice(html.indexOf('  const URL_RE'), html.indexOf('  let lastDay'));

class Element extends EventTarget {
  constructor(tag, className = '', text) {
    super();
    this.tag = tag;
    this.className = className;
    this.children = text === undefined ? [] : [text];
    this.dataset = {};
    this.attributes = new Map();
  }
  appendChild(child) { this.children.push(child); return child; }
  setAttribute(key, value) { this.attributes.set(key, value); }
  removeAttribute(key) { this.attributes.delete(key); }
  get textContent() { return this.children.map((child) => child.textContent ?? child).join(''); }
  find(tag) {
    return this.children.flatMap((child) => child instanceof Element ? [...(child.tag === tag ? [child] : []), ...child.find(tag)] : []);
  }
}

const { renderMessage } = vm.runInNewContext(`${renderer}\n({ renderMessage });`, {
  el: (...args) => new Element(...args), document: { createTextNode: (text) => text }, URL,
  curKind: 'private', hhmm: () => '12:00',
});
function bubble(text, entities) {
  return renderMessage({ id: 1, kind: 'text', text, entities }).children.find((child) => child.className === 'txt');
}
const range = (type, offset, length, data) => ({ type, offset, length, ...data });

test('TC-B2: formats the title after an emoji and treats embedded markup as text', () => {
  const text = '🌙 Tiêu đề\n\nnội dung <script>';
  const node = bubble(text, [range('bold', 3, 7), range('code', text.indexOf('<script>'), 8)]);
  assert.equal(node.textContent, text);
  assert.equal(node.find('strong')[0]?.textContent, 'Tiêu đề');
  assert.equal(node.find('code')[0]?.textContent, '<script>');
  assert.equal(node.find('script').length, 0);
});

test('TC-B2: HTTP(S) only for text links, without a nested auto-link or lost formatting', () => {
  const label = 'https://example.test/label';
  const invalid = 'https://example.test/plain';
  const raw = 'https://example.test/free';
  const text = `${label} ${invalid} ${raw}`;
  const node = bubble(text, [
    range('text_link', 0, label.length, { url: 'https://example.test/target' }),
    range('bold', 0, 5),
    range('text_link', text.indexOf(invalid), invalid.length, { url: 'javascript:alert(1)' }),
    range('italic', text.indexOf(raw) + 8, 7),
  ]);
  const links = node.find('a');
  assert.equal(node.textContent, text);
  assert.deepEqual(links.map((a) => [a.textContent, a.href, a.rel]), [
    [label, 'https://example.test/target', 'noopener noreferrer'],
    [raw, raw, 'noopener noreferrer'],
  ]);
  assert.equal(links[0].find('strong')[0]?.textContent, 'https');
  assert.equal(links[1].find('em')[0]?.textContent, 'example');
  assert.deepEqual(bubble(text).find('a').map((a) => a.href), [label, invalid, raw]);
});

test('TC-B2: nested and crossing formats keep their text, including blocks, mentions and spoilers', () => {
  const text = 'abcdef\nline\nAn bí mật';
  const node = bubble(text, [range('bold', 0, 4), range('italic', 2, 4), range('underline', 2, 1),
    range('strikethrough', 5, 1), range('blockquote', 7, 4), range('pre', 7, 4, { language: 'sh' }),
    range('text_mention', 12, 2, { user_id: 7 }), range('spoiler', 15, 6)]);
  assert.equal(node.textContent, text);
  const bold = node.find('strong')[0];
  assert.equal(bold.textContent, 'abcd');
  assert.equal(bold.find('em')[0]?.find('u')[0]?.textContent, 'c');
  assert.deepEqual(node.find('em').map((em) => em.textContent), ['cd', 'ef']);
  assert.equal(node.find('s')[0]?.textContent, 'f');
  const code = node.find('blockquote')[0]?.find('pre')[0]?.find('code')[0];
  assert.equal(code?.textContent, 'line');
  assert.equal(code.dataset.language, 'sh');
  const mention = node.find('span').find((span) => span.className === 'mention');
  assert.equal(mention.textContent, 'An');
  assert.equal(mention.dataset.userId, '7');
  const spoiler = node.find('span').find((span) => span.className === 'spoiler');
  assert.equal(spoiler.attributes.get('role'), 'button');
  spoiler.dispatchEvent(new Event('click', { cancelable: true }));
  assert.equal(spoiler.className, 'spoiler revealed');
});

test('TC-B3: service messages render centered .m-service label safely without XSS', () => {
  const plain = renderMessage({ id: 90, kind: 'service', summary: 'ℹ️ Tin nhắn hệ thống' });
  assert.equal(plain.tag, 'div');
  assert.equal(plain.className, 'm-service');
  assert.equal(plain.dataset.id, 90);
  assert.equal(plain.textContent, 'ℹ️ Tin nhắn hệ thống');
  assert.equal(plain.find('button').length, 0);

  const xss = renderMessage({ id: 91, kind: 'service', summary: 'ℹ️ <script>alert(1)</script>' });
  assert.equal(xss.className, 'm-service');
  assert.equal(xss.textContent, 'ℹ️ <script>alert(1)</script>');
  assert.equal(xss.find('script').length, 0);
});
