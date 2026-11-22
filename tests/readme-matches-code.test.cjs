const assert = require('node:assert/strict');
const { readdirSync, readFileSync } = require('node:fs');
const path = require('node:path');
const { test } = require('node:test');

const ROOT = path.resolve(__dirname, '..');

function walkRsFiles(dir) {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === 'tests') continue;
      out.push(...walkRsFiles(full));
    } else if (entry.isFile() && entry.name.endsWith('.rs') && entry.name !== 'tests.rs' && entry.name !== 'fake.rs') {
      out.push(full);
    }
  }
  return out.sort();
}

function stripCfgTestBlocks(source) {
  const lines = source.split('\n');
  const out = [];
  let skipAttr = false;
  let skippingBlock = false;
  let depth = 0;

  for (const line of lines) {
    const trimmed = line.trim();
    if (!skippingBlock && /^\#\[cfg\(test\)\]/.test(trimmed)) {
      skipAttr = true;
      continue;
    }
    if (skipAttr) {
      if (trimmed.startsWith('#[')) continue;
      if (trimmed.endsWith(';') && !trimmed.includes('{')) {
        skipAttr = false;
        continue;
      }
      if (trimmed.includes('{')) {
        skipAttr = false;
        skippingBlock = true;
        depth = 0;
        for (const ch of line) {
          if (ch === '{') depth += 1;
          else if (ch === '}') depth -= 1;
        }
        if (depth <= 0) skippingBlock = false;
        continue;
      }
      if (trimmed === '') continue;
      skipAttr = false;
    }
    if (skippingBlock) {
      for (const ch of line) {
        if (ch === '{') depth += 1;
        else if (ch === '}') depth -= 1;
      }
      if (depth <= 0) skippingBlock = false;
      continue;
    }
    out.push(line);
  }
  return out.join('\n');
}

function extractEnvVarsFromSrc() {
  const vars = new Set();
  for (const file of walkRsFiles(path.join(ROOT, 'src'))) {
    const prod = stripCfgTestBlocks(readFileSync(file, 'utf8'));
    for (const m of prod.matchAll(/(?:std::env::var(?:_os)?|env_num|number|get)\(\s*"([A-Z][A-Z0-9_]{2,})"/g)) {
      vars.add(m[1]);
    }
    for (const m of prod.matchAll(/alias\(\s*"([A-Z][A-Z0-9_]{2,})"\s*,\s*"([A-Z][A-Z0-9_]{2,})"/g)) {
      vars.add(m[1]);
      vars.add(m[2]);
    }
  }
  return vars;
}

function extractConfigRsEnvVars() {
  const prod = stripCfgTestBlocks(readFileSync(path.join(ROOT, 'src/config.rs'), 'utf8'));
  const vars = new Set();
  for (const m of prod.matchAll(/(?:std::env::var(?:_os)?|get)\(\s*"([A-Z][A-Z0-9_]{2,})"/g)) {
    vars.add(m[1]);
  }
  return vars;
}

function normalizeRoutePath(p) {
  return p.replace(/\{([A-Za-z0-9_]+)\}/g, ':$1');
}

function extractDelimited(text, openIdx, openCh, closeCh) {
  let depth = 0;
  let inString = false;
  let escape = false;
  for (let i = openIdx; i < text.length; i++) {
    const ch = text[i];
    if (inString) {
      if (escape) {
        escape = false;
      } else if (ch === '\\') {
        escape = true;
      } else if (ch === '"') {
        inString = false;
      }
      continue;
    }
    if (ch === '"') {
      inString = true;
    } else if (ch === openCh) {
      depth += 1;
    } else if (ch === closeCh) {
      depth -= 1;
      if (depth === 0) return { inner: text.slice(openIdx + 1, i), end: i + 1 };
    }
  }
  assert.fail(`Cặp dấu ${openCh}${closeCh} không cân bằng trong router_with`);
}

function extractRoutesFromRestRs() {
  const src = readFileSync(path.join(ROOT, 'src/rest.rs'), 'utf8');
  const start = src.indexOf('fn router_with(');
  assert.ok(start !== -1, 'Không tìm thấy hàm router_with trong src/rest.rs');
  const end = src.indexOf('\n/// Router HTTP hoàn chỉnh', start);
  assert.ok(end !== -1, 'Không tìm thấy cuối hàm router_with trong src/rest.rs');
  const body = src.slice(start, end);

  const routes = new Set();
  const addRoute = (method, p) => routes.add(`${method} ${normalizeRoutePath(p)}`);

  // Trích xuất mọi vòng for (...) in [...] { ... } bằng đếm ngoặc cân bằng
  const loopRanges = [];
  const loopRe = /for\s+([a-z_(),\s]+?)\s+in\s+\[([\s\S]*?)\]\s*\{/g;
  let lm;
  while ((lm = loopRe.exec(body)) !== null) {
    const varExpr = lm[1].trim();
    const listExpr = lm[2];
    const openIdx = lm.index + lm[0].length - 1;
    const { inner: block, end: closeIdx } = extractDelimited(body, openIdx, '{', '}');
    loopRanges.push([lm.index, closeIdx]);
    loopRe.lastIndex = closeIdx;

    const method = /\bpost\b/.test(block) ? 'POST' : 'GET';
    if (varExpr === 'action') {
      const actions = [...listExpr.matchAll(/"([^"]+)"/g)].map((x) => x[1]);
      const fmt = block.match(/format!\("([^"]+)\{action\}"\)/);
      assert.ok(fmt, `Không phân tích được vòng for action: ${block}`);
      for (const act of actions) addRoute(method, `${fmt[1]}${act}`);
    } else if (varExpr.startsWith('(')) {
      const pairs = [...listExpr.matchAll(/\("([^"]+)",\s*[A-Za-z0-9_]+\)/g)].map((x) => x[1]);
      for (const p of pairs) addRoute(method, p);
    } else {
      const items = [...listExpr.matchAll(/"([^"]+)"/g)].map((x) => x[1]);
      for (const item of items) addRoute(method, item);
    }
  }

  // Các lời gọi .route("/...", ...) ngoài vòng for, phân tích bằng đếm ngoặc tròn cân bằng
  let cleaned = '';
  let cursor = 0;
  for (const [s, e] of loopRanges) {
    cleaned += body.slice(cursor, s);
    cursor = e;
  }
  cleaned += body.slice(cursor);

  const routeCallRe = /\.route\(/g;
  let rm;
  while ((rm = routeCallRe.exec(cleaned)) !== null) {
    const parenIdx = rm.index + '.route'.length;
    const { inner, end: callEnd } = extractDelimited(cleaned, parenIdx, '(', ')');
    routeCallRe.lastIndex = callEnd;
    const mPath = inner.match(/^\s*"([^"]+)"\s*,([\s\S]+)$/);
    assert.ok(mPath, `Không phân tích được lời gọi .route(${inner})`);
    const p = mPath[1];
    const handler = mPath[2];
    if (/\badd\(/.test(handler)) {
      if (/\bget\b/.test(handler)) addRoute('GET', p);
      if (/\bpost\b/.test(handler)) addRoute('POST', p);
      if (/MethodFilter::DELETE/.test(handler)) addRoute('DELETE', p);
      if (/MethodFilter::PUT/.test(handler)) addRoute('PUT', p);
    } else if (/\bon\(\s*get\b/.test(handler) || /\broute\(\s*get\b/.test(handler)) {
      addRoute('GET', p);
    } else if (/\bon\(\s*post\b/.test(handler) || /\broute\(\s*post\b/.test(handler)) {
      addRoute('POST', p);
    } else {
      assert.fail(`Không rõ phương thức HTTP cho route ${p}: ${handler}`);
    }
  }

  return routes;
}

function sectionSlice(md, heading) {
  const marker = `\n## ${heading}\n`;
  const idx = md.indexOf(marker);
  assert.ok(idx !== -1, `README.md thiếu mục "## ${heading}"`);
  const after = md.slice(idx + marker.length);
  const next = after.search(/\n## /);
  return next === -1 ? after : after.slice(0, next);
}

function extractReadmeEnvVars(md) {
  const sec = sectionSlice(md, 'Biến môi trường');
  const vars = new Set();
  for (const line of sec.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed.startsWith('|') || trimmed.startsWith('| ---') || trimmed.startsWith('| Biến')) continue;
    const cells = trimmed.split('|').slice(1, -1).map((c) => c.trim());
    if (cells.length < 2) continue;
    for (const col of cells.slice(0, 2)) {
      for (const m of col.matchAll(/`([A-Z][A-Z0-9_]{2,})`/g)) {
        vars.add(m[1]);
      }
    }
  }
  return vars;
}

function extractReadmeRoutes(md) {
  const sec = sectionSlice(md, 'Bảng route REST');
  const routes = new Set();
  const rows = [];
  for (const line of sec.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed.startsWith('|') || trimmed.startsWith('| ---') || trimmed.startsWith('| Phương thức')) continue;
    const cells = trimmed.split('|').slice(1, -1).map((c) => c.trim());
    if (cells.length < 5) continue;
    const method = cells[0].replace(/`/g, '').trim();
    const rawPath = cells[1].replace(/`/g, '').trim();
    assert.ok(['GET', 'POST', 'DELETE', 'PUT', 'PATCH'].includes(method), `Phương thức lạ trong README: ${method}`);
    assert.ok(rawPath.startsWith('/'), `Đường dẫn lạ trong README: ${rawPath}`);
    const key = `${method} ${normalizeRoutePath(rawPath)}`;
    assert.ok(!routes.has(key), `Route bị lặp trong bảng README: ${key}`);
    routes.add(key);
    rows.push({ key, method, path: normalizeRoutePath(rawPath), auth: cells[2], params: cells[3], desc: cells[4] });
  }
  return { routes, rows };
}

function diffSets(expected, actual) {
  return {
    missing: [...expected].filter((x) => !actual.has(x)).sort(),
    extra: [...actual].filter((x) => !expected.has(x)).sort(),
  };
}

function assertReadmeMatchesCode(md, codeEnv, codeRoutes) {
  const readmeEnv = extractReadmeEnvVars(md);
  const envDiff = diffSets(codeEnv, readmeEnv);
  assert.deepEqual(envDiff, { missing: [], extra: [] },
    `Biến môi trường README lệch mã nguồn: thiếu=${JSON.stringify(envDiff.missing)}, thừa=${JSON.stringify(envDiff.extra)}`);

  const { routes: readmeRoutes, rows } = extractReadmeRoutes(md);
  const routeDiff = diffSets(codeRoutes, readmeRoutes);
  assert.deepEqual(routeDiff, { missing: [], extra: [] },
    `Bảng route REST trong README lệch src/rest.rs: thiếu=${JSON.stringify(routeDiff.missing)}, thừa=${JSON.stringify(routeDiff.extra)}`);

  return rows;
}

test('mọi biến môi trường trong src/ và src/config.rs khớp 100% bảng README.md', () => {
  const md = readFileSync(path.join(ROOT, 'README.md'), 'utf8');
  const codeEnv = extractEnvVarsFromSrc();
  const configEnv = extractConfigRsEnvVars();
  assert.ok(codeEnv.size >= 28, `Quá ít biến môi trường trích được: ${codeEnv.size}`);
  for (const v of configEnv) {
    assert.ok(codeEnv.has(v), `Biến trong src/config.rs (${v}) phải nằm trong tập biến nguồn`);
  }
  const codeRoutes = extractRoutesFromRestRs();
  assert.ok(codeRoutes.size >= 85, `Quá ít route trích được từ src/rest.rs: ${codeRoutes.size}`);
  assertReadmeMatchesCode(md, codeEnv, codeRoutes);
});

test('mọi route yêu cầu confirm=true đều ghi rõ confirm trong bảng README.md', () => {
  const md = readFileSync(path.join(ROOT, 'README.md'), 'utf8');
  const { rows } = extractReadmeRoutes(md);
  const byKey = new Map(rows.map((r) => [r.key, r]));
  const confirmRoutes = [
    'POST /api/profile',
    'POST /api/settings/storage',
    'POST /api/storage/clear',
    'POST /api/chats/:chat_id/settings',
    'POST /api/chats/:chat_id/clear_history',
    'POST /api/chats/:chat_id/delete',
    'POST /api/chats/:chat_id/leave',
    'POST /api/do/:name',
  ];
  for (const key of confirmRoutes) {
    const row = byKey.get(key);
    assert.ok(row, `Thiếu route ${key} trong bảng README`);
    assert.match(row.params, /\bconfirm\b/, `Route ${key} phải ghi rõ tham số confirm trong README`);
  }
});

test('phiên bản add-on khớp Cargo.toml và README nhắc đúng ảnh GHCR và trạng thái thực tế', () => {
  const cargo = readFileSync(path.join(ROOT, 'Cargo.toml'), 'utf8');
  const cargoVer = cargo.match(/^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1];
  assert.ok(cargoVer, 'Không đọc được version Cargo.toml');

  const addonCfg = readFileSync(path.join(ROOT, 'addon/tele-rs/config.yaml'), 'utf8');
  const addonVer = addonCfg.match(/^version\s*:\s*"?([^\s"]+)"?/m)?.[1];
  assert.equal(addonVer, cargoVer, 'version add-on phải khớp Cargo.toml');

  const md = readFileSync(path.join(ROOT, 'README.md'), 'utf8');
  assert.match(md, new RegExp(`v${cargoVer.replace(/\./g, '\\.')}`), `README.md phải nhắc phiên bản v${cargoVer}`);
  assert.ok(md.includes('ghcr.io/tritue2011/tele-rust'), 'README.md phải nhắc đúng tên ảnh ghcr.io/tritue2011/tele-rust');
  assert.doesNotMatch(md, /cột mốc 0|Giấy phép mã của kho này: chưa chọn/i, 'README.md vẫn còn ghi chú nháp cũ');
});

test('đột biến đối chứng: thiếu hoặc thừa biến môi trường / route phải bị phát hiện', () => {
  const md = readFileSync(path.join(ROOT, 'README.md'), 'utf8');
  const codeEnv = extractEnvVarsFromSrc();
  const codeRoutes = extractRoutesFromRestRs();

  const missingVarMd = md.replace('| `GATEWAY_MOCK` |', '| `BIEN_GIA_KHONG_TON_TAI` |');
  assert.throws(() => assertReadmeMatchesCode(missingVarMd, codeEnv, codeRoutes), /Biến môi trường README lệch/);

  const missingRouteMd = md.replace('| `GET` | `/health` |', '| `GET` | `/khong-ton-tai` |');
  assert.throws(() => assertReadmeMatchesCode(missingRouteMd, codeEnv, codeRoutes), /Bảng route REST trong README lệch/);
});
