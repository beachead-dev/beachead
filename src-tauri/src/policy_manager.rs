//! Policy Manager: wraps `sbx policy` CLI commands for network policy management.
//!
//! Provides a thin abstraction over SbxCli policy methods, handling input
//! validation and delegating all operations to the sbx CLI.

use std::sync::Arc;

use crate::error::OrchestratorError;
use crate::sbx::{PolicyDefault, PolicyLogEntry, PolicyState, SbxCli};

/// Manages network policies via the `sbx policy` CLI.
///
/// This struct delegates to `SbxCli` policy methods and adds input validation.
pub struct PolicyManager {
    sbx: Arc<SbxCli>,
}

impl PolicyManager {
    /// Create a new PolicyManager wrapping the given SbxCli instance.
    pub fn new(sbx: Arc<SbxCli>) -> Self {
        Self { sbx }
    }

    /// Get the current policy state (default policy + active rules).
    ///
    /// Invokes `sbx policy ls --json` (sbx 0.35.0+) and returns the parsed
    /// `PolicyState`.
    pub async fn get_state(&self) -> Result<PolicyState, OrchestratorError> {
        self.sbx.policy_ls().await
    }

    /// Set the default policy mode.
    ///
    /// Invokes `sbx policy set-default <mode>`.
    pub async fn set_default(&self, mode: PolicyDefault) -> Result<(), OrchestratorError> {
        self.sbx.policy_set_default(mode).await
    }

    /// Add a network allow or deny rule.
    ///
    /// Invokes `sbx policy allow network "<target>"` or
    /// `sbx policy deny network "<target>"` based on the action parameter.
    ///
    /// # Arguments
    /// * `action` - Must be "allow" or "deny"
    /// * `target` - The network target (e.g., "127.0.0.1:8080" or "*.example.com")
    pub async fn add_rule(&self, action: &str, target: &str) -> Result<(), OrchestratorError> {
        if action.trim().is_empty() {
            return Err(OrchestratorError::Validation(
                "Action cannot be empty".to_string(),
            ));
        }
        if target.trim().is_empty() {
            return Err(OrchestratorError::Validation(
                "Target cannot be empty".to_string(),
            ));
        }

        match action {
            "allow" => self.sbx.policy_allow_network(target).await,
            "deny" => self.sbx.policy_deny_network(target).await,
            _ => Err(OrchestratorError::Validation(format!(
                "Invalid action '{}': must be 'allow' or 'deny'",
                action
            ))),
        }
    }

    /// Remove a policy rule by its ID.
    ///
    /// Resolves the rule's scope from `sbx policy ls --json` and invokes
    /// `sbx policy rm network [--sandbox <name>] --id <rule_id>` (sbx 0.35.0+).
    pub async fn remove_rule(&self, rule_id: &str) -> Result<(), OrchestratorError> {
        if rule_id.trim().is_empty() {
            return Err(OrchestratorError::Validation(
                "Rule ID cannot be empty".to_string(),
            ));
        }

        self.sbx.policy_remove_rule(rule_id).await
    }

    /// Get the policy traffic log.
    ///
    /// Invokes `sbx policy log [SANDBOX] [--limit <n>]`.
    pub async fn get_log(
        &self,
        sandbox_id: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<PolicyLogEntry>, OrchestratorError> {
        self.sbx.policy_log(sandbox_id, limit).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates a PolicyManager with a mock sbx binary (a shell script).
    fn create_test_manager(script_content: &str) -> (PolicyManager, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("sbx");

        #[cfg(unix)]
        {
            use std::fs;
            use std::io::Write;
            use std::os::unix::fs::PermissionsExt;
            let mut file = fs::File::create(&script_path).unwrap();
            file.write_all(script_content.as_bytes()).unwrap();
            file.sync_all().unwrap();
            drop(file);
            fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
            // Brief yield to ensure kernel releases write lock on the file
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        #[cfg(windows)]
        {
            let script_path = dir.path().join("sbx.bat");
            std::fs::write(&script_path, script_content).unwrap();
        }

        let sbx = Arc::new(SbxCli::with_path(script_path));
        let manager = PolicyManager::new(sbx);
        (manager, dir)
    }

    #[tokio::test]
    async fn test_get_state_parses_json_output() {
        // sbx 0.35.0: `policy_ls()` invokes `sbx policy ls --json`. The real JSON
        // shape is `{"rules":[{decision,resources,applies_to,resource_type,origin,
        // status,sandbox_id?}]}` with no top-level `default_policy`. Each rule is
        // flattened one `PolicyRule` per resource (action←decision, target←resource,
        // origin←applies_to). Covers one global (`applies_to:"all"`) and one
        // per-sandbox (`applies_to:"sandbox:ktest"`) rule (Requirement 3.3).
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ] && [ "$3" = "--json" ]; then
    cat <<'JSON'
{
  "rules": [
    {
      "id": "1e17bb98-582a-409a-aa6c-11b144c00938",
      "name": "1e17bb98-582a-409a-aa6c-11b144c00938",
      "policy_id": "local-policy",
      "scope": "global",
      "applies_to": "all",
      "resource_type": "network",
      "decision": "allow",
      "resources": ["**.kiro.dev:443"],
      "origin": "local",
      "status": "active",
      "editable": true
    },
    {
      "id": "b656a698-8713-442d-920c-bf95fbe979d4",
      "name": "b656a698-8713-442d-920c-bf95fbe979d4",
      "policy_id": "d8523707-740f-4d60-8385-a38e572d5639",
      "scope": "sandbox:ktest",
      "applies_to": "sandbox:ktest",
      "resource_type": "network",
      "decision": "allow",
      "resources": ["localhost:9100"],
      "origin": "scoped",
      "status": "active",
      "editable": true,
      "sandbox_id": "ktest"
    }
  ]
}
JSON
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let state = mgr.get_state().await.unwrap();

        assert_eq!(state.rules.len(), 2);

        // Global rule: flattened action←decision, target←resource, origin←applies_to.
        let global = state
            .rules
            .iter()
            .find(|r| r.target == "**.kiro.dev:443")
            .expect("global rule **.kiro.dev:443 should be present");
        assert_eq!(global.id, Some("1e17bb98-582a-409a-aa6c-11b144c00938".to_string()));
        assert_eq!(global.action, "allow");
        assert_eq!(global.origin.as_deref(), Some("all"));
        assert_eq!(global.rule_type.as_deref(), Some("network"));
        assert_eq!(global.provenance.as_deref(), Some("local"));
        assert_eq!(global.status.as_deref(), Some("active"));

        // Per-sandbox rule: origin carries the "sandbox:<name>" scope.
        let sandbox = state
            .rules
            .iter()
            .find(|r| r.target == "localhost:9100")
            .expect("per-sandbox rule localhost:9100 should be present");
        assert_eq!(sandbox.id, Some("b656a698-8713-442d-920c-bf95fbe979d4".to_string()));
        assert_eq!(sandbox.action, "allow");
        assert_eq!(sandbox.origin.as_deref(), Some("sandbox:ktest"));
        assert_eq!(sandbox.provenance.as_deref(), Some("scoped"));
    }

    #[tokio::test]
    async fn test_get_state_empty_rules() {
        // sbx 0.35.0 empty policy: `{"rules":[]}` → empty `PolicyState`, no error.
        // `default_policy` is inferred, not read (0.35.0 `ls` has no default-mode
        // field). With no rules at all there are no global network allow rules, so
        // the inference yields "deny-all".
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ] && [ "$3" = "--json" ]; then
    echo '{"rules":[]}'
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let state = mgr.get_state().await.unwrap();

        assert!(state.rules.is_empty());
        assert_eq!(state.default_policy, "deny-all");
    }

    #[tokio::test]
    async fn test_get_state_cli_failure() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ]; then
    echo "error: not logged in" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.get_state().await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }

    #[tokio::test]
    async fn test_set_default_allow() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "set-default" ] && [ "$3" = "allow-all" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.set_default(PolicyDefault::Allow).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_set_default_deny() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "set-default" ] && [ "$3" = "deny-all" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.set_default(PolicyDefault::Deny).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_set_default_balanced() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "set-default" ] && [ "$3" = "balanced" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.set_default(PolicyDefault::Balanced).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_set_default_cli_failure() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "set-default" ]; then
    echo "error: permission denied" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.set_default(PolicyDefault::Allow).await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }

    #[tokio::test]
    async fn test_add_rule_allow() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "allow" ] && [ "$3" = "network" ] && [ "$4" = "127.0.0.1:8080" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.add_rule("allow", "127.0.0.1:8080").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_add_rule_deny() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "deny" ] && [ "$3" = "network" ] && [ "$4" = "evil.com" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.add_rule("deny", "evil.com").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_add_rule_invalid_action() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.add_rule("block", "127.0.0.1:8080").await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_add_rule_empty_action() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.add_rule("", "127.0.0.1:8080").await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_add_rule_empty_target() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.add_rule("allow", "").await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_add_rule_whitespace_target() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.add_rule("allow", "   ").await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_add_rule_cli_failure() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "allow" ] && [ "$3" = "network" ]; then
    echo "error: invalid target" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.add_rule("allow", "bad-target").await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }

    #[tokio::test]
    async fn test_remove_rule_success() {
        // sbx 0.35.0: policy_ls() uses --json; removal keys on --id (not --resource).
        // Global rule (applies_to:"all") → no --sandbox scope.
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ] && [ "$3" = "--json" ]; then
    cat <<'JSON'
{"rules":[{"id":"rule-123","applies_to":"all","resource_type":"network","decision":"allow","resources":["example.com:443"],"origin":"local","status":"active"}]}
JSON
    exit 0
fi
if [ "$1" = "policy" ] && [ "$2" = "rm" ] && [ "$3" = "network" ] && [ "$4" = "--id" ] && [ "$5" = "rule-123" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.remove_rule("rule-123").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_remove_rule_empty_id() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.remove_rule("").await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_remove_rule_whitespace_id() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.remove_rule("   ").await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_remove_rule_cli_failure() {
        // sbx 0.35.0: removal first lists via `policy ls --json` to resolve scope,
        // then issues `policy rm network --id <id>`. Here the rule IS found in the
        // listing but the `rm` command itself fails, which must surface as SbxError.
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ] && [ "$3" = "--json" ]; then
    cat <<'JSON'
{"rules":[{"id":"rule-boom","applies_to":"all","resource_type":"network","decision":"allow","resources":["example.com:443"],"origin":"local","status":"active"}]}
JSON
    exit 0
fi
if [ "$1" = "policy" ] && [ "$2" = "rm" ] && [ "$3" = "network" ]; then
    echo "error: rule not found" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.remove_rule("rule-boom").await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }

    #[tokio::test]
    async fn test_remove_rule_uses_id_for_global_rule() {
        // Supersedes the pre-0.35.0 `test_remove_rule_uses_resource_for_local_prefixed_id`:
        // in sbx 0.35.0 the JSON `id` is the stable, unique rule identifier accepted by
        // `sbx policy rm network --id`, so removal keys on --id (not --resource). The rule
        // id here is a real UUID as emitted by 0.35.0 `--json`.
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ] && [ "$3" = "--json" ]; then
    cat <<'JSON'
{"rules":[{"id":"5fa4ef3f-009e-4ffb-8812-1ca77e211eff","applies_to":"all","resource_type":"network","decision":"allow","resources":["test.com:443"],"origin":"local","status":"active"}]}
JSON
    exit 0
fi
if [ "$1" = "policy" ] && [ "$2" = "rm" ] && [ "$3" = "network" ] && [ "$4" = "--id" ] && [ "$5" = "5fa4ef3f-009e-4ffb-8812-1ca77e211eff" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr
            .remove_rule("5fa4ef3f-009e-4ffb-8812-1ca77e211eff")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_get_log_no_filters() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "log" ]; then
    echo '[{"timestamp":"2024-01-01T00:00:00Z","sandbox":"my-sandbox","host":"api.openai.com","action":"allowed","proxy":"http","rule":"rule-1","reason":"matched allow rule"}]'
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let entries = mgr.get_log(None, None).await.unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].host, Some("api.openai.com".to_string()));
        assert_eq!(entries[0].action, Some("allowed".to_string()));
        assert_eq!(entries[0].sandbox, Some("my-sandbox".to_string()));
    }

    #[tokio::test]
    async fn test_get_log_with_sandbox_filter() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "log" ] && [ "$3" = "test-sbx" ]; then
    echo '[{"timestamp":"2024-01-01T00:00:00Z","sandbox":"test-sbx","host":"example.com","action":"denied","proxy":"http","rule":null,"reason":"default deny"}]'
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let entries = mgr.get_log(Some("test-sbx"), None).await.unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].sandbox, Some("test-sbx".to_string()));
        assert_eq!(entries[0].action, Some("denied".to_string()));
    }

    #[tokio::test]
    async fn test_get_log_with_limit() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "log" ] && [ "$3" = "--limit" ] && [ "$4" = "10" ]; then
    echo '[]'
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let entries = mgr.get_log(None, Some(10)).await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_get_log_with_sandbox_and_limit() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "log" ] && [ "$3" = "sbx-1" ] && [ "$4" = "--limit" ] && [ "$5" = "5" ]; then
    echo '[]'
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let entries = mgr.get_log(Some("sbx-1"), Some(5)).await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_get_log_cli_failure() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "log" ]; then
    echo "error: not logged in" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.get_log(None, None).await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }

    // ─────────────────────────────────────────────────────────────────────
    // sbx 0.35.0 `sbx policy ls` redesign — regression coverage
    //
    // These started as EXPLORATORY bug-condition checks (bugfix workflow,
    // Task 1.1): they were written to FAIL against the pre-fix code (which
    // invoked `sbx policy ls` without `--json` and fell back to the legacy text
    // parser, dropping/misreading every rule). Now that `policy_ls()` uses
    // `--json` and `policy_remove_rule()` resolves scope from that JSON, they
    // PASS and serve as regressions guarding the 0.35.0 behavior:
    //   - `policy_ls()` calls `sbx policy ls --json` and faithfully maps the
    //     0.35.0 JSON rules into `PolicyState`.
    //   - `policy_remove_rule()` resolves the rule from that JSON and removes it.
    //
    // The mock below reproduces a real sbx 0.35.0 daemon:
    //   * `sbx policy ls --json` → the verified 0.35.0 JSON payload (design.md),
    //     one global `applies_to:"all"` rule and one `applies_to:"sandbox:ktest"` rule.
    //   * `sbx policy ls` (no --json) → the new summarized default overview
    //     (`POLICY SOURCE APPLIES TO SUMMARY`), retained to prove the fixed code
    //     never relies on the text path.
    //   * `sbx policy rm ...` → success (exit 0).
    const SBX_035_MOCK: &str = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ]; then
    if [ "$3" = "--json" ]; then
        cat <<'JSON'
{
  "rules": [
    {
      "id": "b656a698-8713-442d-920c-bf95fbe979d4",
      "name": "b656a698-8713-442d-920c-bf95fbe979d4",
      "policy_id": "d8523707-740f-4d60-8385-a38e572d5639",
      "scope": "sandbox:ktest",
      "applies_to": "sandbox:ktest",
      "resource_type": "network",
      "decision": "allow",
      "resources": ["localhost:9100"],
      "origin": "scoped",
      "status": "active",
      "editable": true,
      "sandbox_id": "ktest"
    },
    {
      "id": "1e17bb98-582a-409a-aa6c-11b144c00938",
      "name": "1e17bb98-582a-409a-aa6c-11b144c00938",
      "policy_id": "local-policy",
      "scope": "global",
      "applies_to": "all",
      "resource_type": "network",
      "decision": "allow",
      "resources": ["**.kiro.dev:443"],
      "origin": "local",
      "status": "active",
      "editable": true
    }
  ]
}
JSON
        exit 0
    fi
    cat <<'TXT'
POLICY         SOURCE   APPLIES TO       SUMMARY
local-policy   local    all              network: 155 allow, 1 deny
scoped-ktest   local    sandbox:ktest    network: 1 allow
TXT
    exit 0
fi
if [ "$1" = "policy" ] && [ "$2" = "rm" ]; then
    exit 0
fi
exit 1
"#;

    /// Regression (Requirement 1.1/1.2): the global `applies_to:"all"` allow rule
    /// `**.kiro.dev:443` from the real 0.35.0 output must appear in `PolicyState`
    /// with `action` from `decision` and `origin` from `applies_to`.
    #[tokio::test]
    async fn test_bug_repro_035_global_rule_dropped() {
        let (mgr, _dir) = create_test_manager(SBX_035_MOCK);
        let state = mgr.get_state().await.unwrap();

        let global = state.rules.iter().find(|r| r.target == "**.kiro.dev:443");
        assert!(
            global.is_some(),
            "global allow rule **.kiro.dev:443 was dropped/misread; got rules: {:?}",
            state.rules
        );
        let global = global.unwrap();
        assert_eq!(global.action, "allow");
        assert_eq!(global.origin.as_deref(), Some("all"));
    }

    /// Regression (Requirement 1.2): the per-sandbox `applies_to:"sandbox:ktest"`
    /// allow rule `localhost:9100` must appear with `origin == "sandbox:ktest"`.
    #[tokio::test]
    async fn test_bug_repro_035_sandbox_rule_misread() {
        let (mgr, _dir) = create_test_manager(SBX_035_MOCK);
        let state = mgr.get_state().await.unwrap();

        let sandbox = state.rules.iter().find(|r| r.target == "localhost:9100");
        assert!(
            sandbox.is_some(),
            "per-sandbox rule localhost:9100 was dropped/misread; got rules: {:?}",
            state.rules
        );
        assert_eq!(sandbox.unwrap().origin.as_deref(), Some("sandbox:ktest"));
    }

    /// Regression (Requirement 1.1): `policy_ls()` uses `--json`, so it returns
    /// exactly the two real rules (`**.kiro.dev:443`, `localhost:9100`) and never
    /// relies on the summarized default text overview.
    #[tokio::test]
    async fn test_bug_repro_035_summarized_text_misread() {
        let (mgr, _dir) = create_test_manager(SBX_035_MOCK);
        let state = mgr.get_state().await.unwrap();

        let has_global = state.rules.iter().any(|r| r.target == "**.kiro.dev:443");
        let has_sandbox = state.rules.iter().any(|r| r.target == "localhost:9100");
        assert!(
            has_global && has_sandbox && state.rules.len() == 2,
            "summarized-text fallback misread the policy; expected exactly the 2 real \
             rules (**.kiro.dev:443, localhost:9100), got: {:?}",
            state.rules
        );
    }

    /// Regression (Requirement 2.1/2.4): removing the global rule by its real UUID
    /// must succeed — `policy_remove_rule()` resolves the rule via `policy_ls()`
    /// (`--json`) and issues the scoped `policy rm network --id <uuid>`.
    #[tokio::test]
    async fn test_bug_repro_035_remove_existing_rule_notfound() {
        let (mgr, _dir) = create_test_manager(SBX_035_MOCK);
        let result = mgr
            .remove_rule("1e17bb98-582a-409a-aa6c-11b144c00938")
            .await;
        assert!(
            result.is_ok(),
            "expected removal of existing rule 1e17bb98-... to succeed, got: {:?}",
            result
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // Task 6.2 — Integration tests through `PolicyManager`
    //
    // These exercise the full PolicyManager → SbxCli → mock `sbx` path end to
    // end (not just the SbxCli unit level in sbx.rs). They confirm:
    //   * `get_state()` served to the API faithfully reflects the real 0.35.0
    //     `--json` output (rule count + every mapped field).
    //   * `remove_rule()` drives the complete list → resolve → scoped-rm chain
    //     for BOTH a global rule (no `--sandbox`) and a per-sandbox rule
    //     (`--sandbox <name>`). The per-sandbox path is not otherwise covered at
    //     the PolicyManager level.
    // _Requirements: 1.1, 2.2, 2.3_

    /// get_state() end-to-end: a mock emitting the verified 0.35.0 `--json`
    /// (one global + one per-sandbox rule) must produce a `PolicyState` whose
    /// rule count and every mapped field match what the API should serve.
    #[tokio::test]
    async fn test_integration_get_state_end_to_end() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ] && [ "$3" = "--json" ]; then
    cat <<'JSON'
{
  "rules": [
    {
      "id": "1e17bb98-582a-409a-aa6c-11b144c00938",
      "name": "1e17bb98-582a-409a-aa6c-11b144c00938",
      "policy_id": "local-policy",
      "scope": "global",
      "applies_to": "all",
      "resource_type": "network",
      "decision": "allow",
      "resources": ["**.kiro.dev:443"],
      "origin": "local",
      "status": "active",
      "editable": true
    },
    {
      "id": "b656a698-8713-442d-920c-bf95fbe979d4",
      "name": "b656a698-8713-442d-920c-bf95fbe979d4",
      "policy_id": "d8523707-740f-4d60-8385-a38e572d5639",
      "scope": "sandbox:ktest",
      "applies_to": "sandbox:ktest",
      "resource_type": "network",
      "decision": "deny",
      "resources": ["evil.example.com:443"],
      "origin": "scoped",
      "status": "active",
      "editable": true,
      "sandbox_id": "ktest"
    }
  ]
}
JSON
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let state = mgr.get_state().await.unwrap();

        // Rule count matches the JSON (one PolicyRule per resource).
        assert_eq!(state.rules.len(), 2);

        // Global rule: every field mapped from the JSON.
        let global = state
            .rules
            .iter()
            .find(|r| r.target == "**.kiro.dev:443")
            .expect("global rule should be served");
        assert_eq!(
            global.id,
            Some("1e17bb98-582a-409a-aa6c-11b144c00938".to_string())
        );
        assert_eq!(global.action, "allow");
        assert_eq!(global.origin.as_deref(), Some("all"));
        assert_eq!(global.rule_type.as_deref(), Some("network"));
        assert_eq!(global.provenance.as_deref(), Some("local"));
        assert_eq!(global.status.as_deref(), Some("active"));

        // Per-sandbox rule: origin carries the scope; decision maps to action.
        let sandbox = state
            .rules
            .iter()
            .find(|r| r.target == "evil.example.com:443")
            .expect("per-sandbox rule should be served");
        assert_eq!(
            sandbox.id,
            Some("b656a698-8713-442d-920c-bf95fbe979d4".to_string())
        );
        assert_eq!(sandbox.action, "deny");
        assert_eq!(sandbox.origin.as_deref(), Some("sandbox:ktest"));
        assert_eq!(sandbox.rule_type.as_deref(), Some("network"));
        assert_eq!(sandbox.provenance.as_deref(), Some("scoped"));
        assert_eq!(sandbox.status.as_deref(), Some("active"));
    }

    /// remove_rule() end-to-end for a GLOBAL rule: the mock lists a global
    /// (`applies_to:"all"`) rule via `--json`, then exits 0 ONLY for
    /// `policy rm network --id <id>` with no `--sandbox` scope (the rm branch
    /// requires `$4 = --id` and no 6th arg). Proves the full
    /// list → resolve → scoped-rm chain omits `--sandbox` for global rules.
    #[tokio::test]
    async fn test_integration_remove_global_rule_end_to_end() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ] && [ "$3" = "--json" ]; then
    cat <<'JSON'
{"rules":[{"id":"1e17bb98-582a-409a-aa6c-11b144c00938","applies_to":"all","resource_type":"network","decision":"allow","resources":["**.kiro.dev:443"],"origin":"local","status":"active"}]}
JSON
    exit 0
fi
if [ "$1" = "policy" ] && [ "$2" = "rm" ] && [ "$3" = "network" ] && [ "$4" = "--id" ] && [ "$5" = "1e17bb98-582a-409a-aa6c-11b144c00938" ] && [ -z "$6" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr
            .remove_rule("1e17bb98-582a-409a-aa6c-11b144c00938")
            .await;
        assert!(
            result.is_ok(),
            "global removal should issue `policy rm network --id <id>` with no --sandbox, got: {:?}",
            result
        );
    }

    /// remove_rule() end-to-end for a PER-SANDBOX rule: the mock lists a
    /// `applies_to:"sandbox:ktest"` rule via `--json`, then exits 0 ONLY for
    /// `policy rm network --sandbox ktest --id <id>`. Exercises the full
    /// list → resolve → scoped-rm chain and proves the `--sandbox <name>` scope
    /// is derived from the JSON and passed through. This per-sandbox removal
    /// path is not otherwise covered at the PolicyManager level.
    #[tokio::test]
    async fn test_integration_remove_sandbox_rule_end_to_end() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ] && [ "$3" = "--json" ]; then
    cat <<'JSON'
{"rules":[{"id":"b656a698-8713-442d-920c-bf95fbe979d4","applies_to":"sandbox:ktest","resource_type":"network","decision":"allow","resources":["localhost:9100"],"origin":"scoped","status":"active","sandbox_id":"ktest"}]}
JSON
    exit 0
fi
if [ "$1" = "policy" ] && [ "$2" = "rm" ] && [ "$3" = "network" ] && [ "$4" = "--sandbox" ] && [ "$5" = "ktest" ] && [ "$6" = "--id" ] && [ "$7" = "b656a698-8713-442d-920c-bf95fbe979d4" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr
            .remove_rule("b656a698-8713-442d-920c-bf95fbe979d4")
            .await;
        assert!(
            result.is_ok(),
            "per-sandbox removal should issue `policy rm network --sandbox ktest --id <id>`, got: {:?}",
            result
        );
    }
}
