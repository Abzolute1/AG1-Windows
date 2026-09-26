# MCP desktop composition — source checkpoint, 2026-09-12

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

This historical B6 checkpoint is now followed by the installed [C1 manual-credential checkpoint](MCP-AUTHENTICATION.md). The B6 evidence and hashes below remain preserved.

B6 is an [accepted source checkpoint](../../MCP/desktop-integration.json), following the [B5 model discovery checkpoint](MCP-DISCOVERY.md). The full A–F task remains active. The [normal production distribution is installed and verified](../../MCP/desktop-installation.json) for this user. Restart AG1 from the application menu.

The desktop now composes one application-owned manager and separate native session connection leases. Manual local/remote definitions save inactive. The MCP navigation opens a native general-chat context without inference when no chat is selected. Users can review/cancel exact connection effects, test discovery only, connect, inspect tools/resources/templates/prompts, change scope availability, disconnect and remove definitions. Unsupported schemas remain inspectable. Private authentication and public Discover are explicitly unavailable at this checkpoint.

Availability follows the verified working directory's canonical path/device/inode identity, separately from the conversation journal owner. Changing a chat's working folder retires its old MCP lease and request/approval state while preserving the conversation. A directly opened project and that same project attached to general chat share the same availability identity. A missing working folder receives no MCP runtime until folder recovery. Connections remain independent even for two chats with the same server and catalog scope. Native execution identities additionally bind the conversation workspace, session and epoch; renderer/model fields cannot choose that binding. Disabling one scope preserves unrelated allowed connections. Whole-server edits/removal revoke all affected connections.

A native pending review uses an opaque one-use UUID, short expiry, exact definition revision and session ownership. Creating a review performs no network/process work. Confirmation revalidates the original target. Inspection connections cannot supply executable selections. Catalog projections preserve unsupported original metadata and exact native generation strings. Capabilities retain the existing unknown-risk approval policy; this candidate does not add trusted-tool bypass.

## Verification

- Final selected Rust contracts and MCP units: **228 passes**, including eight new MCP contracts and the existing native folder-grant regression.
- Core MCP contracts: 16 passes, including one new canonical-project/conversation-owner test.
- Complete frontend comparison: current 232 passes/2 failures versus isolated pre-B6 227 passes/2 failures. Both fail the existing slow-chat status-text and image-activity-label assertions. Five new MCP UI tests pass. An initial baseline layout omitted the SDK import; the corrected workspace layout restores those four tests. No frozen expectations were changed.
- Typecheck passes. Platform Clippy returns to the same 63 existing warnings after a new diagnostic warning was corrected.
- The final frozen native WebKit audit passed at 1440×900 and 900×600 using owned HTTP/stdio peers and no provider requests, tool calls, resource reads or prompt retrievals. Final screenshots were inspected: one-column narrow layout, literal metadata, explicit local launch review and exact generation strings. A separate owned compatibility audit also passes with a copy of the preserved bound process helper.
- The visual-test feature had four pre-existing missing-field compilation errors in GitHub fixture Reply constructors. They now explicitly provide absent rate-limit/resource metadata. Those fixture-only corrections do not change GitHub production behavior or expectations.

The [final regression commands](../../MCP/desktop/final-commands.json) use distinct immutable `mcp-b6-final` and `mcp-b6-before` profiles, 661 frozen source files and 101 preserved entries. Candidate and isolated B5 baseline both have **243 passes/16 session failures** and **22 passes/6 core failures**, with identical assertion bodies after normalizing only thread IDs and the once-per-process backtrace note. No new regression failures were added. Native screenshot review and accepted evidence publication are complete. The normal production package/build/install is complete for this user. Do not rebuild accepted profiles.

Full integration still requires private credentials/OAuth, reviewed scoped trust rules, public registry discovery and guided setup, supported input continuation, media presentation, child delegation scope composition, broader conformance/resource-budget acceptance and complete A–F verification. The other feature tasks and real-model/duration programmes remain paused.

## Production installation

The production desktop SHA is `c911cd9a9151a8df961c309766103c9ea64d68efb5a3c14676ff7287603c3dad`. All **1,144 payload files**, the receipt, 661 frozen sources and 101 protected entries were verified. A separate normal-production startup/manager/Add server smoke passed on a private display with synthetic XTest input, no visual-test broker and no inference.

System-wide sudo authentication was unavailable unattended. The standard per-user installer updated `/home/alex/.local/share/ag1/application/current` to release `9cf7588aef114d2e8d17c6520a4c93a8`, and the existing user `ag1.desktop` entry now uses `/home/alex/.local/bin/ag1-desktop`. Previous user and `/opt/ag1` releases remain unchanged. The original user desktop entry is backed up in the installation evidence. Existing open windows continue running their old binary until restarted.
