const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { test } = require('node:test');
const root = path.resolve(__dirname, '..');

test('login and setup pages load scripts only from this origin', () => {
  for (const file of ['web/index.html', 'web/app.html']) {
    const html = readFileSync(path.join(root, file), 'utf8');
    for (const match of html.matchAll(/<script\b[^>]*\bsrc\s*=\s*["']([^"']+)/gi)) {
      assert.ok(match[1].startsWith('/') && !match[1].startsWith('//'), `${file}: external script ${match[1]}`);
    }
    assert.doesNotMatch(html, /\b(?:localStorage|sessionStorage|innerHTML|outerHTML|insertAdjacentHTML|document\.write)\b/);
  }
  assert.doesNotMatch(readFileSync(path.join(root, 'src/web.rs'), 'utf8'), /https:\/\//);
});

test('secret byte comparisons keep the constant-time primitive', () => {
  for (const file of ['src/ui_auth.rs', 'src/rest.rs']) {
    const source = readFileSync(path.join(root, file), 'utf8');
    const body = source.match(/fn ct_eq\(a: &\[u8\], b: &\[u8\]\) -> bool\s*\{([^}]+)\}/);
    assert.ok(body, `${file}: comparison helper missing`);
    assert.match(body[1], /bool::from\(a\.ct_eq\(b\)\)/, `${file}: use subtle::ConstantTimeEq`);
    assert.doesNotMatch(body[1], /==|!=/);
  }
});

test('vendored QR library matches version 1.4.4 and generates a GIF locally', () => {
  const source = readFileSync(path.join(root, 'web/qrcode.min.js'), 'utf8').split('\n').slice(1).join('\n').trimEnd();
  assert.equal(createHash('sha256').update(source).digest('hex'), '164fc2c1c9eaf0a03aa8dfdb855e82e41a5c6922fbad8bb3238116207e26bef7');
  const qr = require('../web/qrcode.min.js')(0, 'M');
  qr.addData('tg://login?token=fake-token-value');
  qr.make();
  assert.ok(qr.getModuleCount() > 20);
  assert.ok(Buffer.from(qr.createDataURL().split(',')[1], 'base64').subarray(0, 6).equals(Buffer.from('GIF87a')));
});
