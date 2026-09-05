use ag1_protocol::{fake::FakeHost, *};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use ts_rs::{Config, TS};

fn fixtures() -> Vec<WireMessage> {
    let mut host = FakeHost::new("fixture-codegen".to_owned().try_into().unwrap());
    let base = host.envelope(
        "request-1",
        Command::Prompt {
            input: PromptInput::Inline {
                text: "fixture only".into(),
            },
        },
    );
    let mut null = base.clone();
    null.extension = Some(Value::Null);
    let mut custom = base.clone();
    custom.extension =
        Some(json!({"future": [null, {"flag": true}], "decimal": "18446744073709551615"}));
    let mut messages = vec![
        WireMessage::Handshake(host.handshake(&Version::default()).unwrap()),
        WireMessage::Command(base.clone()),
        WireMessage::Command(null),
        WireMessage::Command(custom),
        WireMessage::Subscription(host.subscribe().unwrap()),
        WireMessage::Snapshot(host.snapshot()),
    ];
    let reply = host.command(base);
    messages.push(WireMessage::Result(reply));
    host.step();
    messages.push(WireMessage::Snapshot(host.snapshot()));
    let mut cursor = Cursor {
        scope: host.snapshot().scope,
        after: Sequence::new(0),
    };
    messages.push(WireMessage::Delivery(host.replay(&cursor)));
    cursor.after = Sequence::new(9_007_199_254_740_993);
    messages.push(WireMessage::Cursor(cursor.clone()));
    cursor.after = Sequence::new(u64::MAX);
    messages.push(WireMessage::Cursor(cursor));
    for error in [
        ProtocolError::IncompatibleVersion,
        ProtocolError::Malformed {
            detail: "fixture".into(),
        },
        ProtocolError::Oversize {
            limit: MAX_ENVELOPE as u32,
            use_artifact: true,
        },
        ProtocolError::QueueFull {
            queue: "session".into(),
            capacity: 256,
        },
        ProtocolError::Unavailable {
            family: "model".into(),
        },
        ProtocolError::StaleContext,
        ProtocolError::CommandConflict,
        ProtocolError::InvalidApproval,
        ProtocolError::NotFound,
        ProtocolError::ResyncRequired,
        ProtocolError::Capacity {
            resource: "subscriber".into(),
        },
    ] {
        messages.push(WireMessage::Error(error));
    }
    let scope = host.snapshot().scope;
    let bridge: BridgeSnapshot = serde_json::from_value(json!({
        "version": Version::default(), "scope": scope, "revision": "9007199254740993",
        "watermark": "9007199254740993", "workspace_path": "/fixture/project", "status": "awaiting_input",
        "active_operation": "operation-fixture", "messages": [{"id":"message-1","role":"assistant","text":"Review this request","evidence":"evidence-fixture"}],
        "message_count": 1, "pending": [{"binding":{"id":"approval-fixture","operation":"operation-fixture","argument_sha256":"argument-sha256","scope":scope,"revision":"9007199254740993","expires_ms":"18446744073709551615"},"title":"write approved.txt","kind":"approval","options":[],"evidence":"request-evidence","truncated":false}],
        "model":"preserved-profile", "diagnostic":null, "session_file":"/private/session.jsonl", "fixture_provider":true
    })).unwrap();
    messages.push(WireMessage::Bridge(BridgeFrame::Snapshot {
        snapshot: bridge,
    }));
    messages.push(WireMessage::Bridge(BridgeFrame::Reply {
        id: "bridge-query".to_owned().try_into().unwrap(),
        scope: scope.clone(),
        revision: Sequence::new(9_007_199_254_740_993),
        durable: false,
        status: "query".into(),
        data: json!({"models":[],"tuning":false}),
    }));
    messages.push(WireMessage::Bridge(BridgeFrame::Progress {
        scope,
        operation: "operation-fixture".to_owned().try_into().unwrap(),
        text: "Bounded streaming projection".into(),
    }));
    messages.push(WireMessage::Bridge(BridgeFrame::Fatal {
        detail: "Injected persistence failure".into(),
    }));
    messages
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--roundtrip") {
        let bytes = fs::read(args.get(1).ok_or("missing roundtrip path")?)?;
        if bytes.len() > 1024 * 1024 {
            return Err("roundtrip fixture too large".into());
        }
        let actual: Vec<WireMessage> = serde_json::from_slice(&bytes)?;
        if actual != fixtures() {
            return Err("TypeScript changed wire semantics".into());
        }
        println!("Rust accepted {} TypeScript round trips", actual.len());
        return Ok(());
    }
    if !args.is_empty() && args != ["--check"] {
        return Err("usage: protocol-codegen [--check | --roundtrip FILE]".into());
    }
    let cfg = Config::default();
    let mut declarations = String::from(
        "// Generated from ag1-protocol Rust DTOs. Run protocol-codegen; do not edit.\n",
    );
    macro_rules! decls { ($($ty:ty),+ $(,)?) => { $(declarations.push_str("export "); declarations.push_str(&<$ty>::decl(&cfg)); declarations.push('\n');)+ }; }
    decls!(
        Value,
        WorkspaceId,
        SessionId,
        BranchId,
        OperationId,
        ArtifactId,
        CommandId,
        InstanceId,
        SubscriptionId,
        ApprovalId,
        Sequence,
        Version,
        ProtocolError,
        Scope,
        CommandEnvelope,
        PromptInput,
        Command,
        ApprovalBinding,
        OperationStatus,
        OperationView,
        CommandResult,
        Reply,
        Snapshot,
        Event,
        Cursor,
        Delivery,
        Subscription,
        Handshake,
        BridgeMessage,
        BridgeQuestion,
        BridgeSnapshot,
        BridgeFrame,
        WireMessage
    );
    let fixture_json = serde_json::to_string_pretty(&fixtures())? + "\n";
    let fixture_ts = format!(
        "// Generated Rust codec fixtures.\nimport type {{ WireMessage }} from '../../product/apps/desktop/src/generated/protocol.js';\nexport const fixtures = {} satisfies WireMessage[];\n",
        fixture_json.trim_end()
    );
    let schema = serde_json::to_string_pretty(&schemars::schema_for!(WireMessage))? + "\n";
    for (path, contents) in [
        (
            root.join("product/apps/desktop/src/generated/protocol.ts"),
            declarations,
        ),
        (
            root.join("product/crates/ag1-protocol/schema/protocol.schema.json"),
            schema,
        ),
        (root.join("fixtures/protocol/roundtrip.json"), fixture_json),
        (root.join("fixtures/protocol/roundtrip.ts"), fixture_ts),
    ] {
        if args == ["--check"] {
            if fs::read_to_string(&path)? != contents {
                return Err(format!("generated drift: {}", path.display()).into());
            }
        } else {
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(path, contents)?;
        }
    }
    println!(
        "Protocol generation {} (schema {})",
        if args.is_empty() {
            "written"
        } else {
            "verified"
        },
        schema_hash()
    );
    Ok(())
}
