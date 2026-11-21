"""Run the eight required auth mutation controls through the approved remote builder.

Each mutation gets an isolated copy under this worktree and runs its exact
integration test. The real source is fingerprinted before and after every run.
"""

import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
from tempfile import TemporaryDirectory


ROOT = Path(__file__).resolve().parents[2]
MUTATIONS = [
    ("TC-01", "tc01_first_setup_requires_correct_gateway_token", "src/ui_auth.rs",
     'if self.gateway_token.is_empty() || !ct_eq(token.as_bytes(), self.gateway_token.as_bytes()) {',
     'if false && (self.gateway_token.is_empty() || !ct_eq(token.as_bytes(), self.gateway_token.as_bytes())) {'),
    ("TC-02", "tc02_setup_cannot_replace_an_existing_account", "src/ui_auth.rs",
     'if store.account.is_some() { return Err(error(409,',
     'if false && store.account.is_some() { return Err(error(409,'),
    ("TC-03", "tc03_login_sets_required_cookie_flags_and_remember_duration", "src/ui_auth.rs",
     'tggw_session={token}; Path=/; HttpOnly; SameSite=Lax',
     'tggw_session={token}; Path=/; SameSite=Lax'),
    ("TC-04", "tc04_wrong_password_and_unknown_user_have_similar_median_timing", "src/ui_auth.rs",
     'let valid = verify_password(password, salt, expected) && candidate.is_some();',
     'let valid = candidate.is_some() && verify_password(password, salt, expected);'),
    ("TC-05", "qa_tc05_parallel_wrong_logins_cannot_bypass_the_five_attempt_limit", "src/ui_auth.rs",
     'if attempts.len() >= MAX_FAILURES {',
     'if false && attempts.len() >= MAX_FAILURES {'),
    ("TC-07", "tc07_cookie_writes_require_csrf_header_or_matching_origin", "src/rest.rs",
     'fn csrf_valid(headers: &HeaderMap) -> bool {',
     'fn csrf_valid(headers: &HeaderMap) -> bool {\n    if !headers.is_empty() { return true; }'),
    ("TC-11", "tc11_credentials_and_raw_session_tokens_are_not_persisted", "src/ui_auth.rs",
     '        self.save_store(&updated)?;\n        *store = updated;\n        Ok(grant)\n    }\n\n    pub fn login',
     '''        self.save_store(&updated)?;
        if let Some(path) = &self.path {
            let mut leaked: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            leaked["password"] = json!(password);
            crate::accounts::atomic_json(path, &leaked).unwrap();
        }
        *store = updated;
        Ok(grant)
    }

    pub fn login'''),
    ("TC-12", "tc12_remembered_session_survives_server_auth_state_reload", "src/ui_auth.rs",
     'let sessions = store.sessions.iter().map(|session| json!({',
     'let sessions = std::iter::empty::<&Session>().map(|session| json!({'),
]


def fingerprint():
    digest = hashlib.sha256()
    files = [ROOT / name for name in ("Cargo.toml", "build.sh")]
    for directory in ("src", "web", "tests", "docker"):
        files.extend(path for path in (ROOT / directory).rglob("*")
                     if path.is_file() and "__pycache__" not in path.parts)
    for path in sorted(files):
        digest.update(str(path.relative_to(ROOT)).encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
    return digest.hexdigest()


def replace_once(path, before, after):
    text = path.read_text()
    assert text.count(before) == 1, f"Mutation target is not unique: {path.name}"
    path.write_text(text.replace(before, after, 1))


def copy_source(snapshot):
    for name in ("src", "web", "tests", "docker"):
        # Fresh mtimes force Cargo to rebuild after a different source snapshot.
        shutil.copytree(ROOT / name, snapshot / name, copy_function=shutil.copy,
                        ignore=shutil.ignore_patterns("__pycache__"))
    for name in ("Cargo.toml", "build.sh"):
        shutil.copy2(ROOT / name, snapshot / name)


def main():
    assert sys.argv[1:] in ([], ["--positive-only"]), "Use --positive-only or no arguments"
    source = fingerprint()
    print(f"SOURCE_SHA256 {source}", flush=True)
    for case, test, filename, before, after in ([] if sys.argv[1:] else MUTATIONS):
        assert fingerprint() == source, "Worktree changed during mutation verification"
        with TemporaryDirectory(prefix=".h2-mutant-", dir=ROOT) as directory:
            snapshot = Path(directory)
            copy_source(snapshot)
            replace_once(snapshot / filename, before, after)
            # Use the same build script/container; select just the named control.
            replace_once(snapshot / "build.sh", 'ARGS="cargo test --release 2>&1"',
                         f'ARGS="cargo clean -p telegram-gateway-rs && cargo test --release --test ui_auth {test} -- --exact 2>&1"')
            print(f"RUN {case}: {test}", flush=True)
            result = subprocess.run(["./build.sh", "test"], cwd=snapshot,
                                    env={**os.environ, "TGGW_BUILD_DIR": "/opt/tggw-build-dn"},
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            (ROOT / f".h2-mutation-{case.lower()}.log").write_text(result.stdout)
            assert fingerprint() == source, "Worktree changed during mutation verification"
            assert result.returncode == 101, f"{case}: expected test exit 101, got {result.returncode}"
            assert f"test {test} ... FAILED" in result.stdout, f"{case}: did not fail the intended test"
            assert "0 passed; 1 failed;" in result.stdout, f"{case}: missing failed-test summary"
            print(f"{case} CAUGHT: exit 101; 0 passed; 1 failed", flush=True)
    print("RUN pristine positive control (full test suite)", flush=True)
    with TemporaryDirectory(prefix=".h2-positive-", dir=ROOT) as directory:
        snapshot = Path(directory)
        copy_source(snapshot)
        result = subprocess.run(["./build.sh", "test"], cwd=snapshot,
                                env={**os.environ, "TGGW_BUILD_DIR": "/opt/tggw-build-dn"},
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        (ROOT / ".h2-mutation-positive.log").write_text(result.stdout)
        assert fingerprint() == source, "Worktree changed during positive control"
        print(f"POSITIVE_CONTROL_EXIT {result.returncode}", flush=True)
        if result.returncode:
            return result.returncode
    print("PASS: pristine full suite; source fingerprint unchanged" if sys.argv[1:] else
          "PASS: 8/8 mutation controls caught; pristine full suite; source fingerprint unchanged", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
