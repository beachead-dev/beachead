//! Property-based tests for sbx CLI command construction.
//!
//! Property 7: sbx run command construction
//! - Validates that constructed commands include correct agent identifier,
//!   all --kit flags, -t flag, workspace mount, -- separator with args.

use proptest::prelude::*;
use std::path::PathBuf;

use crate::sbx::{SbxCli, SbxRunArgs};

/// Strategy for generating valid agent identifiers.
fn agent_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("claude".to_string()),
        Just("codex".to_string()),
        Just("copilot".to_string()),
        Just("cursor".to_string()),
        Just("droid".to_string()),
        Just("gemini".to_string()),
        Just("kiro".to_string()),
        Just("opencode".to_string()),
        Just("docker-agent".to_string()),
        Just("shell".to_string()),
        "[a-z][a-z0-9-]{1,20}".prop_map(|s| s),
    ]
}

/// Strategy for generating workspace paths.
fn workspace_strategy() -> impl Strategy<Value = PathBuf> {
    prop_oneof![
        Just(PathBuf::from("/home/user/project")),
        Just(PathBuf::from("/tmp/workspace")),
        "[a-z/]{3,30}".prop_map(|s| PathBuf::from(format!("/{}", s))),
    ]
}

/// Strategy for generating kit paths (0 to 3 kits).
fn kit_paths_strategy() -> impl Strategy<Value = Vec<PathBuf>> {
    prop::collection::vec(
        prop_oneof![
            Just(PathBuf::from("/tmp/kits/persona-kit")),
            Just(PathBuf::from("/home/user/.beachead/kits/my-kit")),
            "[a-z/]{5,25}".prop_map(|s| PathBuf::from(format!("/kits/{}", s))),
        ],
        0..=3,
    )
}

/// Strategy for generating optional template names.
fn template_strategy() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        "[a-z][a-z0-9-]{2,15}".prop_map(|s| Some(s)),
    ]
}

/// Strategy for generating optional agent CLI args.
fn agent_args_strategy() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec("[a-z0-9-]{1,10}".prop_map(|s| s), 0..=4)
}

/// Strategy for generating optional sandbox names.
fn name_strategy() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        "[a-z][a-z0-9-]{2,15}".prop_map(|s| Some(s)),
    ]
}

/// Strategy for generating complete SbxRunArgs.
fn sbx_run_args_strategy() -> impl Strategy<Value = SbxRunArgs> {
    (
        agent_strategy(),
        kit_paths_strategy(),
        workspace_strategy(),
        name_strategy(),
        template_strategy(),
        agent_args_strategy(),
    )
        .prop_map(
            |(agent, kit_paths, workspace, name, template, agent_args)| SbxRunArgs {
                agent,
                kit_paths,
                workspace,
                name,
                template,
                agent_args,
            },
        )
}

proptest! {
    /// Property 7: The constructed sbx run command always contains the agent identifier.
    #[test]
    fn run_command_contains_agent(args in sbx_run_args_strategy()) {
        let cmd = SbxCli::build_run_args(&args);
        // First arg is "run", second is the agent
        prop_assert_eq!(&cmd[0], "run");
        prop_assert_eq!(&cmd[1], &args.agent);
    }

    /// Property 7: All kit paths appear with --kit flags.
    #[test]
    fn run_command_contains_all_kit_flags(args in sbx_run_args_strategy()) {
        let cmd = SbxCli::build_run_args(&args);

        for kit_path in &args.kit_paths {
            let kit_str = kit_path.to_string_lossy().to_string();
            // Find --kit followed by the path
            let has_kit = cmd.windows(2).any(|w| w[0] == "--kit" && w[1] == kit_str);
            prop_assert!(has_kit, "Missing --kit flag for path: {}", kit_str);
        }

        // Count of --kit flags matches number of kit paths
        let kit_count = cmd.iter().filter(|a| *a == "--kit").count();
        prop_assert_eq!(kit_count, args.kit_paths.len());
    }

    /// Property 7: Workspace path is always present as a positional argument.
    #[test]
    fn run_command_contains_workspace_mount(args in sbx_run_args_strategy()) {
        let cmd = SbxCli::build_run_args(&args);
        let workspace_str = args.workspace.to_string_lossy().to_string();

        // Workspace should appear as a positional arg (not preceded by -v)
        let has_workspace = cmd.iter().any(|a| a == &workspace_str);
        prop_assert!(has_workspace, "Missing workspace path in command");

        // Should NOT have -v flag in the command portion (before --)
        let separator_pos = cmd.iter().position(|a| a == "--");
        let cmd_before_sep = match separator_pos {
            Some(pos) => &cmd[..pos],
            None => &cmd[..],
        };
        let has_v = cmd_before_sep.iter().any(|a| a == "-v");
        prop_assert!(!has_v, "Should not use -v flag for workspace mount");
    }

    /// Property 7: Template flag is present only when template is Some.
    #[test]
    fn run_command_template_flag(args in sbx_run_args_strategy()) {
        let cmd = SbxCli::build_run_args(&args);

        // Only check for -t in the portion before the -- separator (if any),
        // since agent_args after -- could contain arbitrary strings.
        let separator_pos = cmd.iter().position(|a| a == "--");
        let cmd_before_sep = match separator_pos {
            Some(pos) => &cmd[..pos],
            None => &cmd[..],
        };

        match &args.template {
            Some(template) => {
                let has_template = cmd_before_sep.windows(2).any(|w| w[0] == "-t" && w[1] == *template);
                prop_assert!(has_template, "Missing -t flag for template: {}", template);
            }
            None => {
                let has_t = cmd_before_sep.iter().any(|a| a == "-t");
                prop_assert!(!has_t, "Unexpected -t flag when no template specified");
            }
        }
    }

    /// Property 7: Agent args appear after -- separator, and only when non-empty.
    #[test]
    fn run_command_agent_args_after_separator(args in sbx_run_args_strategy()) {
        let cmd = SbxCli::build_run_args(&args);

        if args.agent_args.is_empty() {
            let has_separator = cmd.iter().any(|a| a == "--");
            prop_assert!(!has_separator, "Unexpected -- separator with no agent args");
        } else {
            let sep_pos = cmd.iter().position(|a| a == "--");
            prop_assert!(sep_pos.is_some(), "Missing -- separator before agent args");

            let sep_idx = sep_pos.unwrap();
            let trailing = &cmd[sep_idx + 1..];
            prop_assert_eq!(trailing, &args.agent_args[..]);
        }
    }

    /// Property 7: Name flag is present only when name is Some.
    #[test]
    fn run_command_name_flag(args in sbx_run_args_strategy()) {
        let cmd = SbxCli::build_run_args(&args);

        // Only check before the -- separator
        let separator_pos = cmd.iter().position(|a| a == "--");
        let cmd_before_sep = match separator_pos {
            Some(pos) => &cmd[..pos],
            None => &cmd[..],
        };

        match &args.name {
            Some(name) => {
                let has_name = cmd_before_sep.windows(2).any(|w| w[0] == "--name" && w[1] == *name);
                prop_assert!(has_name, "Missing --name flag for: {}", name);
            }
            None => {
                let has_name = cmd_before_sep.iter().any(|a| a == "--name");
                prop_assert!(!has_name, "Unexpected --name flag when no name specified");
            }
        }
    }
}
