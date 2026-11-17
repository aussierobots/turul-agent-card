//! The lambda's `catalog.json` is produced by the CLI producer commands, never hand-written.
//! This file is the one place the recorded commands live. A deployer who regenerates the
//! catalog updates `COMMANDS` in the same change.

use std::process::Command;

const BUNDLE: &[u8] = include_bytes!("../../turul-ai-catalog-lambda/catalog.json");

const COMMANDS: &[&[&str]] = &[
    &[
        "init",
        "--file",
        "catalog.json",
        "--host-display-name",
        "Acme Services Inc.",
        "--host-identifier",
        "did:web:acme-corp.com",
        "--host-documentation-url",
        "https://docs.acme-corp.com/agents",
    ],
    &[
        "add",
        "--file",
        "catalog.json",
        "--identifier",
        "urn:air:acme-corp.com:agent:finance",
        "--type",
        "application/a2a-agent-card+json",
        "--url",
        "https://api.acme-corp.com/agents/acme-finance-agent.json",
        "--description",
        "Multi-protocol finance agent.",
        "--tag",
        "finance",
        "--tag",
        "trading",
    ],
    &[
        "add",
        "--file",
        "catalog.json",
        "--identifier",
        "urn:air:acme-corp.com:data:market-2026q1",
        "--type",
        "application/parquet",
        "--url",
        "https://data.acme-corp.com/datasets/market-dataset-2026q1.parquet",
        "--display-name",
        "Market Dataset Q1 2026",
        "--description",
        "Artifact-focused card for training and evaluation.",
        "--tag",
        "dataset",
        "--tag",
        "finance",
    ],
    &[
        "add",
        "--file",
        "catalog.json",
        "--identifier",
        "urn:air:acme-corp.com:plugin:finance-assistant-plugin",
        "--type",
        "application/agent-plugins+zip",
        "--url",
        "https://plugins.acme-corp.com/finance-assistant.zip",
        "--display-name",
        "Finance Assistant Plugin",
        "--description",
        "Agent Plugin bundle with skills and MCP server for finance workflows.",
        "--tag",
        "finance",
        "--tag",
        "plugin",
        "--tag",
        "skills",
        "--tag",
        "mcp",
    ],
];

// #producer-behavior, #level-2-discoverable-catalog
#[test]
fn lambda_bundle_is_reproduced_by_the_recorded_commands() {
    let dir = std::env::temp_dir().join(format!("turul-bundle-provenance-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");

    for args in COMMANDS {
        let out = Command::new(env!("CARGO_BIN_EXE_turul-ai-catalog"))
            .args(*args)
            .current_dir(&dir)
            .output()
            .expect("binary runs");
        assert!(
            out.status.success(),
            "{args:?} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let produced = std::fs::read(dir.join("catalog.json")).expect("catalog.json produced");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        produced, BUNDLE,
        "committed bundle differs from the recorded commands' output"
    );
}
