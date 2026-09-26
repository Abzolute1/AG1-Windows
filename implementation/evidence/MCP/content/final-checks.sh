#!/bin/bash
set -eu
export CARGO_TARGET_DIR=/home/alex/Projects/AG1/.local/p14-target
export CARGO_INCREMENTAL=0
cargo_opts=(--offline --locked --manifest-path product/Cargo.toml --config 'profile.mcp-b3-final.inherits="dev"' --config 'profile.mcp-b3-final.debug=0' --profile mcp-b3-final)
rustfmt --check --edition 2024 --config skip_children=true product/crates/ag1-core/src/tools/mcp.rs product/crates/ag1-core/src/tools/mcp/capability.rs product/crates/ag1-core/tests/mcp_capability_contract.rs product/crates/ag1-platform/src/mcp/stdio.rs product/crates/ag1-platform/src/mcp/http.rs product/crates/ag1-platform/src/mcp/operation.rs product/crates/ag1-platform/src/mcp/manager.rs product/crates/ag1-platform/src/mcp/manager/selection.rs product/crates/ag1-platform/src/mcp/manager/execution.rs product/crates/ag1-platform/src/mcp/manager/capability.rs product/crates/ag1-platform/tests/mcp_execution_contract.rs product/crates/ag1-platform/tests/mcp_content/mod.rs > .local/mcp-b3/final-format.log 2>&1
cargo test "${cargo_opts[@]}" -p ag1-core --test mcp_registry_contract --test mcp_capability_contract -p ag1-platform --test mcp_execution_contract --test mcp_manager_contract --test mcp_http_contract --test mcp_stdio_contract --test mcp_foundation_contract --test platform_contract --test github_local_contract -p ag1-storage --test mcp_storage_contract --test storage_contract --test tool_storage_contract -- --test-threads=1 > .local/mcp-b3/final-contracts.log 2>&1
cargo test "${cargo_opts[@]}" -p ag1-platform --lib mcp:: -- --test-threads=1 > .local/mcp-b3/final-units.log 2>&1
cargo clippy "${cargo_opts[@]}" -p ag1-platform --lib > .local/mcp-b3/final-clippy.log 2>&1
