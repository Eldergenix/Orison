//! Build-script sandbox interface.
//!
//! Orison packages may run small build scripts at install time (for code
//! generation, native compilation, etc.). Allowing those scripts to run with
//! the same privileges as the package manager would be a supply-chain disaster
//! — any compromised dependency could exfiltrate credentials or alter the host
//! at will. We mitigate this by running build scripts inside an OS-level
//! sandbox whose ambient authority is limited by an explicit [`SandboxPolicy`].
//!
//! The full implementation requires per-OS syscall filtering (`seccomp` on
//! Linux, `sandbox-exec` on macOS, `AppContainer`/`Job Object` on Windows)
//! which in turn requires platform-specific crates that are out of scope for
//! the bootstrap dependency policy (D002). This module therefore ships the
//! **interface and policy validator** today and emits a structured
//! [`SandboxResult`] in a "would-have-run" mode so callers and tests can lock
//! the contract now. The real enforcement is staged for milestone M32+ and is
//! captured in `docs/compiler/REGISTRY_PROTOCOL.md` under "Sandbox roadmap".
//!
//! The schema of [`SandboxResult`] is published as
//! `schemas/sandbox-result.schema.json` and is part of the public contract.
//! Any field added later is additive (the schema uses
//! `additionalProperties: false`, so changes will require a `v2` bump).

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Stable schema identifier for [`SandboxResult`].
pub const SANDBOX_RESULT_SCHEMA: &str = "ori.sandbox_result.v1";

/// Default build-script timeout in seconds.
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 60;

/// Sandbox policy. A new policy denies everything except what is explicitly
/// listed in the allow lists. Paths are matched as prefixes (any read/write
/// whose target path begins with one of the allowed paths is permitted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxPolicy {
    /// Filesystem paths the sandboxed script may read from.
    pub allow_fs_read: Vec<PathBuf>,
    /// Filesystem paths the sandboxed script may write to.
    pub allow_fs_write: Vec<PathBuf>,
    /// Environment variable names the script may read.
    pub allow_env_read: Vec<String>,
    /// Whether outbound network access is permitted.
    pub allow_net: bool,
    /// Hard timeout in seconds. Zero is normalised to
    /// [`DEFAULT_TIMEOUT_SECONDS`] by [`validate_policy`].
    pub max_runtime_seconds: u64,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        default_policy()
    }
}

/// Return a restrictive default policy: no reads, no writes, no env reads,
/// no network, 60 second timeout.
pub fn default_policy() -> SandboxPolicy {
    SandboxPolicy {
        allow_fs_read: Vec::new(),
        allow_fs_write: Vec::new(),
        allow_env_read: Vec::new(),
        allow_net: false,
        max_runtime_seconds: DEFAULT_TIMEOUT_SECONDS,
    }
}

/// Categorised violation observed during a (real or simulated) sandbox run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "target")]
pub enum PolicyViolation {
    /// The script tried to read a path not on the allow list.
    FsRead(String),
    /// The script tried to write a path not on the allow list.
    FsWrite(String),
    /// The script tried to read an env var not on the allow list.
    EnvRead(String),
    /// The script attempted network I/O while `allow_net` was false.
    Network(String),
    /// The script exceeded the runtime budget.
    Timeout(String),
}

impl fmt::Display for PolicyViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PolicyViolation::FsRead(p) => write!(f, "fs.read denied: {p}"),
            PolicyViolation::FsWrite(p) => write!(f, "fs.write denied: {p}"),
            PolicyViolation::EnvRead(v) => write!(f, "env.read denied: {v}"),
            PolicyViolation::Network(t) => write!(f, "network denied: {t}"),
            PolicyViolation::Timeout(t) => write!(f, "timeout: {t}"),
        }
    }
}

/// Structured outcome of a sandboxed build-script run. In bootstrap mode the
/// fields describe a planned run; once the real sandbox lands they will
/// describe the actual one. The shape is identical so callers do not need to
/// branch on the mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxResult {
    /// Stable schema tag (`"ori.sandbox_result.v1"`).
    pub schema: &'static str,
    /// The script's exit code. `None` if the sandbox could not run the script
    /// (or, in bootstrap mode, if the script was never actually run).
    pub exit_code: Option<i32>,
    /// Standard output collected from the script.
    pub stdout: String,
    /// Standard error collected from the script.
    pub stderr: String,
    /// Filesystem reads attempted (allowed or not).
    pub fs_reads: Vec<String>,
    /// Filesystem writes attempted (allowed or not).
    pub fs_writes: Vec<String>,
    /// Env vars the script tried to read.
    pub env_reads: Vec<String>,
    /// Policy violations the sandbox observed.
    pub policy_violations: Vec<PolicyViolation>,
}

/// Categorised sandbox error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxError {
    /// Policy was internally inconsistent (e.g. zero timeout, malformed path).
    InvalidPolicy(String),
    /// The script path does not exist or is not a regular file.
    BadScriptPath(String),
    /// Generic I/O error message (string-form to keep the type cheap to clone
    /// and compare in tests).
    Io(String),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SandboxError::InvalidPolicy(msg) => write!(f, "invalid sandbox policy: {msg}"),
            SandboxError::BadScriptPath(msg) => write!(f, "invalid script path: {msg}"),
            SandboxError::Io(msg) => write!(f, "sandbox io: {msg}"),
        }
    }
}

impl std::error::Error for SandboxError {}

/// Validate a [`SandboxPolicy`]. Returns `Ok(())` if the policy is internally
/// consistent. Currently checks for empty path components (which would match
/// every path as a prefix) and an explicit-zero timeout.
pub fn validate_policy(policy: &SandboxPolicy) -> Result<(), SandboxError> {
    if policy.max_runtime_seconds == 0 {
        return Err(SandboxError::InvalidPolicy(
            "max_runtime_seconds must be > 0".to_string(),
        ));
    }
    for p in policy
        .allow_fs_read
        .iter()
        .chain(policy.allow_fs_write.iter())
    {
        if p.as_os_str().is_empty() {
            return Err(SandboxError::InvalidPolicy(
                "fs allow list contains empty path".to_string(),
            ));
        }
    }
    // Reject duplicate env entries - the allow list is a set semantically.
    let mut seen: BTreeSet<&String> = BTreeSet::new();
    for v in &policy.allow_env_read {
        if v.is_empty() {
            return Err(SandboxError::InvalidPolicy(
                "env allow list contains empty entry".to_string(),
            ));
        }
        if !seen.insert(v) {
            return Err(SandboxError::InvalidPolicy(format!(
                "env allow list contains duplicate `{v}`"
            )));
        }
    }
    Ok(())
}

/// Test whether a candidate `path` is allowed by a prefix-allow list.
pub fn path_allowed(allow_list: &[PathBuf], candidate: &Path) -> bool {
    allow_list
        .iter()
        .any(|allowed| candidate.starts_with(allowed))
}

/// Run a build script inside the sandbox.
///
/// **Bootstrap status**: the real OS-level sandbox is not yet wired up. This
/// function validates the policy and the script path, then returns a
/// `SandboxResult` whose `exit_code` is `None` and whose `stderr` carries the
/// machine-readable marker `TODO(M32): real-sandbox-not-implemented`. Tests
/// and callers can rely on this marker so the contract is locked and the
/// switch to a real implementation is an internal change.
///
/// When the real implementation lands it will:
/// * On Linux, fork into a seccomp-bpf sandbox that rejects every syscall not
///   on a strict allow list (open/read/write to allowed paths only;
///   `connect()` denied unless `allow_net`).
/// * On macOS, use `sandbox-exec(1)` with an SBPL profile generated from the
///   policy.
/// * On Windows, launch the child inside a Job Object with restricted token
///   and AppContainer.
pub fn run_in_sandbox(
    policy: &SandboxPolicy,
    script_path: &Path,
) -> Result<SandboxResult, SandboxError> {
    validate_policy(policy)?;
    if script_path.as_os_str().is_empty() {
        return Err(SandboxError::BadScriptPath("path is empty".to_string()));
    }
    if !script_path.exists() {
        return Err(SandboxError::BadScriptPath(format!(
            "script path does not exist: {}",
            script_path.display()
        )));
    }
    if !script_path.is_file() {
        return Err(SandboxError::BadScriptPath(format!(
            "script path is not a regular file: {}",
            script_path.display()
        )));
    }

    // Bootstrap mode: return a deterministic "would-have-run" report. We
    // record the policy decisions a real sandbox would have taken if the
    // script attempted nothing, so tests can compare structure without
    // forking. Concrete violation reporting will be wired up alongside the
    // real syscall filter.
    Ok(SandboxResult {
        schema: SANDBOX_RESULT_SCHEMA,
        exit_code: None,
        stdout: String::new(),
        stderr: "TODO(M32): real-sandbox-not-implemented".to_string(),
        fs_reads: Vec::new(),
        fs_writes: Vec::new(),
        env_reads: Vec::new(),
        policy_violations: Vec::new(),
    })
}

/// Helper for callers (and tests) that want to record a hypothetical
/// `fs.read` and have the policy decide whether it would be allowed. Returns
/// `Some(violation)` if the read would be denied.
pub fn check_fs_read(policy: &SandboxPolicy, target: &Path) -> Option<PolicyViolation> {
    if path_allowed(&policy.allow_fs_read, target) {
        None
    } else {
        Some(PolicyViolation::FsRead(target.display().to_string()))
    }
}

/// As [`check_fs_read`] but for writes.
pub fn check_fs_write(policy: &SandboxPolicy, target: &Path) -> Option<PolicyViolation> {
    if path_allowed(&policy.allow_fs_write, target) {
        None
    } else {
        Some(PolicyViolation::FsWrite(target.display().to_string()))
    }
}

/// As [`check_fs_read`] but for env vars.
pub fn check_env_read(policy: &SandboxPolicy, name: &str) -> Option<PolicyViolation> {
    if policy.allow_env_read.iter().any(|n| n == name) {
        None
    } else {
        Some(PolicyViolation::EnvRead(name.to_string()))
    }
}

/// As [`check_fs_read`] but for network access.
pub fn check_network(policy: &SandboxPolicy, target: &str) -> Option<PolicyViolation> {
    if policy.allow_net {
        None
    } else {
        Some(PolicyViolation::Network(target.to_string()))
    }
}

#[cfg(test)]
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_file(label: &str, contents: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id() as u128;
        let count = COUNTER.fetch_add(1, Ordering::Relaxed) as u128;
        let path = env::temp_dir().join(format!("ori-sandbox-{label}-{pid}-{nanos}-{count}.sh"));
        if let Err(err) = fs::write(&path, contents) {
            assert!(false, "could not write temp file {}: {err}", path.display());
        }
        path
    }

    #[test]
    fn default_policy_is_restrictive() {
        let p = default_policy();
        assert!(p.allow_fs_read.is_empty());
        assert!(p.allow_fs_write.is_empty());
        assert!(p.allow_env_read.is_empty());
        assert!(!p.allow_net);
        assert_eq!(p.max_runtime_seconds, DEFAULT_TIMEOUT_SECONDS);
    }

    #[test]
    fn validate_policy_rejects_zero_timeout() {
        let mut p = default_policy();
        p.max_runtime_seconds = 0;
        match validate_policy(&p) {
            Err(SandboxError::InvalidPolicy(_)) => {}
            other => assert!(false, "expected InvalidPolicy, got {other:?}"),
        }
    }

    #[test]
    fn validate_policy_rejects_duplicate_env() {
        let mut p = default_policy();
        p.allow_env_read = vec!["FOO".into(), "FOO".into()];
        match validate_policy(&p) {
            Err(SandboxError::InvalidPolicy(msg)) => {
                assert!(msg.contains("duplicate"));
            }
            other => assert!(false, "expected InvalidPolicy, got {other:?}"),
        }
    }

    #[test]
    fn validate_policy_rejects_empty_env_entry() {
        let mut p = default_policy();
        p.allow_env_read = vec!["".into()];
        match validate_policy(&p) {
            Err(SandboxError::InvalidPolicy(_)) => {}
            other => assert!(false, "expected InvalidPolicy, got {other:?}"),
        }
    }

    #[test]
    fn fs_read_outside_allow_list_is_denied() {
        let policy = default_policy();
        let target = Path::new("/etc/passwd");
        match check_fs_read(&policy, target) {
            Some(PolicyViolation::FsRead(p)) => assert_eq!(p, "/etc/passwd"),
            other => assert!(false, "expected FsRead violation, got {other:?}"),
        }
    }

    #[test]
    fn fs_read_allow_list_permits_prefix() {
        let mut policy = default_policy();
        policy.allow_fs_read.push(PathBuf::from("/tmp/work"));
        let target = PathBuf::from("/tmp/work/cache/x.bin");
        assert!(check_fs_read(&policy, &target).is_none());
    }

    #[test]
    fn fs_write_outside_allow_list_is_denied() {
        let policy = default_policy();
        let target = Path::new("/tmp/leak.bin");
        match check_fs_write(&policy, target) {
            Some(PolicyViolation::FsWrite(_)) => {}
            other => assert!(false, "expected FsWrite violation, got {other:?}"),
        }
    }

    #[test]
    fn env_read_outside_allow_list_is_denied() {
        let policy = default_policy();
        match check_env_read(&policy, "AWS_SECRET_ACCESS_KEY") {
            Some(PolicyViolation::EnvRead(name)) => assert_eq!(name, "AWS_SECRET_ACCESS_KEY"),
            other => assert!(false, "expected EnvRead violation, got {other:?}"),
        }
    }

    #[test]
    fn env_read_allow_list_permits_match() {
        let mut policy = default_policy();
        policy.allow_env_read.push("PATH".into());
        assert!(check_env_read(&policy, "PATH").is_none());
    }

    #[test]
    fn network_is_denied_by_default() {
        let policy = default_policy();
        match check_network(&policy, "registry.example") {
            Some(PolicyViolation::Network(_)) => {}
            other => assert!(false, "expected Network violation, got {other:?}"),
        }
    }

    #[test]
    fn network_allowed_when_flag_set() {
        let mut policy = default_policy();
        policy.allow_net = true;
        assert!(check_network(&policy, "registry.example").is_none());
    }

    #[test]
    fn run_in_sandbox_validates_policy() {
        let mut policy = default_policy();
        policy.max_runtime_seconds = 0;
        let script = temp_file("validate", "true\n");
        match run_in_sandbox(&policy, &script) {
            Err(SandboxError::InvalidPolicy(_)) => {}
            other => assert!(false, "expected InvalidPolicy, got {other:?}"),
        }
    }

    #[test]
    fn run_in_sandbox_rejects_missing_script() {
        let policy = default_policy();
        let path = env::temp_dir().join("definitely-does-not-exist.ori-sandbox");
        match run_in_sandbox(&policy, &path) {
            Err(SandboxError::BadScriptPath(_)) => {}
            other => assert!(false, "expected BadScriptPath, got {other:?}"),
        }
    }

    #[test]
    fn run_in_sandbox_returns_would_have_run_marker() {
        let policy = default_policy();
        let script = temp_file("would-run", "true\n");
        let result = match run_in_sandbox(&policy, &script) {
            Ok(r) => r,
            Err(err) => {
                assert!(false, "run_in_sandbox failed: {err}");
                return;
            }
        };
        assert_eq!(result.schema, SANDBOX_RESULT_SCHEMA);
        assert!(result.exit_code.is_none());
        assert!(result.stderr.contains("TODO(M32)"));
        assert!(result.policy_violations.is_empty());
    }

    #[test]
    fn sandbox_result_serializes_with_schema_tag() {
        let result = SandboxResult {
            schema: SANDBOX_RESULT_SCHEMA,
            exit_code: Some(0),
            stdout: "ok".into(),
            stderr: String::new(),
            fs_reads: vec!["/tmp/in".into()],
            fs_writes: Vec::new(),
            env_reads: vec!["PATH".into()],
            policy_violations: vec![PolicyViolation::Network("example.com".into())],
        };
        let json = match serde_json::to_string(&result) {
            Ok(s) => s,
            Err(err) => {
                assert!(false, "serialize failed: {err}");
                return;
            }
        };
        assert!(json.contains("ori.sandbox_result.v1"));
        assert!(json.contains("Network"));
    }
}
