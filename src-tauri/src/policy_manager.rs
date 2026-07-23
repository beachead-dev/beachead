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
    /// Invokes `sbx policy ls` and returns the parsed `PolicyState`.
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
    /// Invokes `sbx policy remove <rule_id>`.
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
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ]; then
    echo '{"default_policy":"balanced","rules":[{"id":"rule-1","action":"allow","target":"127.0.0.1:8080"}]}'
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let state = mgr.get_state().await.unwrap();

        assert_eq!(state.default_policy, "balanced");
        assert_eq!(state.rules.len(), 1);
        assert_eq!(state.rules[0].id, Some("rule-1".to_string()));
        assert_eq!(state.rules[0].action, "allow");
        assert_eq!(state.rules[0].target, "127.0.0.1:8080");
    }

    #[tokio::test]
    async fn test_get_state_empty_rules() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ]; then
    echo '{"default_policy":"deny","rules":[]}'
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let state = mgr.get_state().await.unwrap();

        assert_eq!(state.default_policy, "deny");
        assert!(state.rules.is_empty());
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
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ]; then
    printf 'PROVENANCE   APPLIES_TO   POLICY/RULE   TYPE      DECISION   STATUS   RESOURCES\n'
    printf 'local        all          rule-123      network   allow      active   example.com:443\n'
    exit 0
fi
if [ "$1" = "policy" ] && [ "$2" = "rm" ] && [ "$3" = "network" ] && [ "$4" = "--resource" ] && [ "$5" = "example.com:443" ]; then
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
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "rm" ] && [ "$3" = "network" ]; then
    echo "error: rule not found" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.remove_rule("nonexistent").await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }

    #[tokio::test]
    async fn test_remove_rule_uses_resource_for_local_prefixed_id() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ]; then
    printf 'PROVENANCE   APPLIES_TO   POLICY/RULE                            TYPE      DECISION   STATUS   RESOURCES\n'
    printf 'local        all          local:5fa4ef3f-009e-4ffb-8812-1ca77e211eff   network   allow      active   test.com:443\n'
    exit 0
fi
if [ "$1" = "policy" ] && [ "$2" = "rm" ] && [ "$3" = "network" ] && [ "$4" = "--resource" ] && [ "$5" = "test.com:443" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        // Even with "local:" prefix in the rule name, removal uses --resource
        let result = mgr
            .remove_rule("local:5fa4ef3f-009e-4ffb-8812-1ca77e211eff")
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
    // Bug reproduction: sbx 0.35.0 `sbx policy ls` redesign
    //
    // These are EXPLORATORY bug-condition checks (bugfix workflow, Task 1.1).
    // They are EXPECTED TO FAIL against the current/unfixed code — that failure
    // documents the counterexample that the bug exists. Each asserts the correct
    // post-fix behavior:
    //   - `policy_ls()` should call `sbx policy ls --json` and faithfully map the
    //     0.35.0 JSON rules into `PolicyState`.
    //   - `policy_remove_rule()` should resolve the rule from that JSON and remove it.
    //
    // The mock below reproduces a real sbx 0.35.0 daemon:
    //   * `sbx policy ls --json` → the verified 0.35.0 JSON payload (design.md),
    //     one global `applies_to:"all"` rule and one `applies_to:"sandbox:ktest"` rule.
    //   * `sbx policy ls` (no --json) → the new summarized default overview
    //     (`POLICY SOURCE APPLIES TO SUMMARY`), which is what the CURRENT code hits.
    //   * `sbx policy rm ...` → success (exit 0).
    //
    // The current `policy_ls()` invokes `sbx policy ls` WITHOUT `--json`, so it
    // receives the summarized overview, fails the `serde_json::from_str::<PolicyState>`
    // attempt, and falls back to `parse_policy_text()` — which does not understand the
    // summarized layout and drops/misreads every rule.
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

    /// Bug repro (Requirement 1.1/1.2): the global `applies_to:"all"` allow rule
    /// `**.kiro.dev:443` from the real 0.35.0 output must appear in `PolicyState`.
    /// UNFIXED: dropped, because `policy_ls()` never passes `--json` and the text
    /// fallback misreads the summarized overview. EXPECTED TO FAIL.
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

    /// Bug repro (Requirement 1.2): the per-sandbox `applies_to:"sandbox:ktest"`
    /// allow rule `localhost:9100` must appear with `origin == "sandbox:ktest"`.
    /// UNFIXED: dropped/misread. EXPECTED TO FAIL.
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

    /// Bug repro (Requirement 1.1): feeding the summarized default text
    /// (`POLICY SOURCE APPLIES TO SUMMARY`) to the current text fallback yields a
    /// misread `PolicyState` instead of the two real rules. Correct behavior is
    /// exactly the two rules present in the real policy. EXPECTED TO FAIL.
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

    /// Bug repro (Requirement 2.1/2.4): removing the global rule by its real UUID
    /// must succeed. UNFIXED: `policy_remove_rule()` resolves the rule via the broken
    /// `policy_ls()`, cannot find the id, and returns `NotFound` for a rule that
    /// actually exists. EXPECTED TO FAIL.
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
}
