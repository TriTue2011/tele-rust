"""Run auth and file-security mutation controls through the approved remote builder.

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
     'fn csrf_valid(headers: &HeaderMap, secure: bool) -> bool {',
     'fn csrf_valid(headers: &HeaderMap, secure: bool) -> bool {\n    if !headers.is_empty() { return true; }'),
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
    ("CAO-nosniff", "cao01_files_are_isolated_and_only_safe_media_can_be_inline", "src/rest.rs",
     '        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),\n', ''),
    ("CAO-csp", "cao01_files_are_isolated_and_only_safe_media_can_be_inline", "src/rest.rs",
     '        (header::CONTENT_SECURITY_POLICY, "default-src \'none\'; sandbox"),\n', ''),
    ("CAO-download", "cao01_files_are_isolated_and_only_safe_media_can_be_inline", "src/rest.rs",
     '            if !inline {', '            if false && !inline {'),
    ("LOG-login", "tc06_tc11_serve_accepts_cookie_on_account_and_event_routes_without_logging_secrets", "src/ui_auth.rs",
     'pub fn login(&self, username: &str, password: &str, remember: bool, ip: &str, secure: bool) -> Result<LoginGrant, AuthError> {',
     'pub fn login(&self, username: &str, password: &str, remember: bool, ip: &str, secure: bool) -> Result<LoginGrant, AuthError> {\n        log::info!("{}", password);'),
    ("LOG-password", "tc06_tc11_serve_accepts_cookie_on_account_and_event_routes_without_logging_secrets", "src/ui_auth.rs",
     'pub fn change_password(&self, token: &str, old: &str, new: &str) -> Result<(), AuthError> {',
     'pub fn change_password(&self, token: &str, old: &str, new: &str) -> Result<(), AuthError> {\n        log::info!("{} {}", old, new);'),
]
for action in ("logout", "logout-all", "password"):
    MUTATIONS.append((f"CSRF-{action}", "trung03_every_session_action_requires_csrf_and_keeps_session_on_rejection", "src/rest.rs",
        'if !csrf_valid(&parts.headers, auth.secure_cookie() || parts.uri.scheme_str() == Some("https"))',
        'if false && !csrf_valid(&parts.headers, auth.secure_cookie() || parts.uri.scheme_str() == Some("https"))'))


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


def replace_once(path, before, after, case=""):
    text = path.read_text()
    if case == "TC-02":
        assert text.count(before) == 2, "Both setup checks must be disabled for this control"
        path.write_text(text.replace(before, after))
    elif case.startswith("CSRF-"):
        start = text.index(f'"{case[5:]}" => {{')
        index = text.index(before, start)
        path.write_text(text[:index] + text[index:].replace(before, after, 1))
    else:
        assert text.count(before) == 1, f"Mutation target is not unique: {path.name}"
        path.write_text(text.replace(before, after, 1))


def copy_source(snapshot):
    for name in ("src", "web", "tests", "docker"):
        # Fresh mtimes force Cargo to rebuild after a different source snapshot.
        shutil.copytree(ROOT / name, snapshot / name, copy_function=shutil.copy,
                        ignore=shutil.ignore_patterns("__pycache__"))
    for name in ("Cargo.toml", "build.sh"):
        shutil.copy2(ROOT / name, snapshot / name)
    if (ROOT / ".tggw-build-host").is_file():
        shutil.copy2(ROOT / ".tggw-build-host", snapshot / ".tggw-build-host")


def main():
    assert sys.argv[1:] in ([], ["--positive-only"], ["--static-only"]), "Use --positive-only, --static-only or no arguments"
    source = fingerprint()
    build_env = {**os.environ, "TGGW_BUILD_DIR": "/opt/tggw-build-dn"}
    local_host = ROOT / ".tggw-build-host"
    if not build_env.get("TGGW_BUILD_HOST") and local_host.is_file():
        build_env["TGGW_BUILD_HOST"] = local_host.read_text().strip()
    print(f"SOURCE_SHA256 {source}", flush=True)
    for filename in (() if sys.argv[1:] == ["--positive-only"] else ("src/ui_auth.rs", "src/rest.rs")):
        with TemporaryDirectory(prefix=".h2-static-", dir=ROOT) as directory:
            snapshot = Path(directory)
            copy_source(snapshot)
            replace_once(snapshot / filename, "bool::from(a.ct_eq(b))", "a == b")
            result = subprocess.run(["node", "--test", "tests/security_test.cjs"], cwd=snapshot,
                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            assert result.returncode == 1 and "not ok 2 - secret byte comparisons" in result.stdout, result.stdout
            print(f"CT-{Path(filename).stem} CAUGHT: exit 1; constant-time source control failed", flush=True)
    if sys.argv[1:] == ["--static-only"]:
        assert fingerprint() == source
        return 0
    for case, test, filename, before, after in ([] if sys.argv[1:] else MUTATIONS):
        assert fingerprint() == source, "Worktree changed during mutation verification"
        with TemporaryDirectory(prefix=".h2-mutant-", dir=ROOT) as directory:
            snapshot = Path(directory)
            copy_source(snapshot)
            replace_once(snapshot / filename, before, after, case)
            # Use the same build script/container; select just the named control.
            suite = "security" if case.startswith(("CAO-", "CSRF-")) else "ui_auth"
            replace_once(snapshot / "build.sh", 'ARGS="cargo test --release 2>&1"',
                         f'ARGS="cargo test --release --test {suite} {test} -- --exact 2>&1"')
            print(f"RUN {case}: {test}", flush=True)
            result = subprocess.run(["./build.sh", "test"], cwd=snapshot,
                                    env=build_env,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            (ROOT / f".h2-mutation-{case.lower()}.log").write_text(result.stdout)
            assert fingerprint() == source, "Worktree changed during mutation verification"
            assert "Compiling telegram-gateway-rs" in result.stdout, f"{case}: source snapshot was not rebuilt"
            assert result.returncode == 101, f"{case}: expected test exit 101, got {result.returncode}"
            assert f"test {test} ... FAILED" in result.stdout, f"{case}: did not fail the intended test"
            assert "0 passed; 1 failed;" in result.stdout, f"{case}: missing failed-test summary"
            print(f"{case} CAUGHT: exit 101; 0 passed; 1 failed", flush=True)
    print("RUN pristine positive control (full test suite)", flush=True)
    with TemporaryDirectory(prefix=".h2-positive-", dir=ROOT) as directory:
        snapshot = Path(directory)
        copy_source(snapshot)
        result = subprocess.run(["./build.sh", "test"], cwd=snapshot,
                                env=build_env,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        (ROOT / ".h2-mutation-positive.log").write_text(result.stdout)
        assert fingerprint() == source, "Worktree changed during positive control"
        assert "Compiling telegram-gateway-rs" in result.stdout, "Positive source snapshot was not rebuilt"
        print(f"POSITIVE_CONTROL_EXIT {result.returncode}", flush=True)
        if result.returncode:
            return result.returncode
    print("PASS: pristine full suite; source fingerprint unchanged" if sys.argv[1:] else
          f"PASS: {len(MUTATIONS) + 2}/{len(MUTATIONS) + 2} mutation controls caught; pristine full suite; source fingerprint unchanged", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
