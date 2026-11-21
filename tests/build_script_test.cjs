const assert = require('node:assert/strict');
const { readFileSync, mkdtempSync, mkdirSync, writeFileSync, existsSync, rmSync } = require('node:fs');
const { spawnSync } = require('node:child_process');
const path = require('node:path');
const { test } = require('node:test');
const root = path.resolve(__dirname, '..');

test('builder copies its archive under the lock and forwards the cargo command', () => {
  const temporary = mkdtempSync(path.join(root, '.h2-build-test-'));
  try {
    const bin = path.join(temporary, 'bin');
    const destination = path.join(temporary, 'source');
    mkdirSync(bin);
    const scripts = {
      ssh: '#!/bin/bash\nset -eu\n[ "$1" = "fake-builder" ]\ncase "$2" in "docker image inspect"*) exit 0;; esac\nexec /bin/sh -c "$2"\n',
      flock: '#!/bin/bash\nset -eu\nshift\nexec "$@"\n',
      docker: '#!/bin/bash\nset -eu\nprintf "%s\\n" "$@"\n',
    };
    for (const [name, source] of Object.entries(scripts)) writeFileSync(path.join(bin, name), source, { mode: 0o700 });
    const result = spawnSync('bash', ['build.sh', 'test'], { cwd: root, encoding: 'utf8',
      env: { ...process.env, PATH: bin + ':' + process.env.PATH, TGGW_BUILD_HOST: 'fake-builder', TGGW_BUILD_DIR: destination } });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /cargo test --release 2>&1/);
    assert.ok(existsSync(path.join(destination, 'Cargo.toml')));
    assert.equal(readFileSync(path.join(destination, 'src/rest.rs'), 'utf8'), readFileSync(path.join(root, 'src/rest.rs'), 'utf8'));
  } finally { rmSync(temporary, { recursive: true, force: true }); }
});
