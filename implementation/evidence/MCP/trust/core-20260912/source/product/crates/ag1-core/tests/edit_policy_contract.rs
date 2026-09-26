use ag1_core::tools::policy::{self, Mode, Policy};
use serde_json::json;
#[test]
fn edit_family_respects_each_approval_mode_and_workspace_boundary() {
    for mode in [
        Mode::Interactive,
        Mode::Autonomous,
        Mode::Planning,
        Mode::Headless,
        Mode::Disabled,
    ] {
        let p = Policy {
            mode,
            cwd: "/project".into(),
            planning_read_only_tools: None,
            trusted_tools: vec![],
            extension_tools: vec![],
        };
        let preview = policy::evaluate(
            "editPreview",
            &json!({"path":"file.ts","oldText":"old","newText":"new"}),
            &p,
            true,
        )
        .unwrap();
        assert!(preview.allow);
        assert!(preview.approval_request.is_none());
        for tool in ["edit", "editApply", "append", "write"] {
            let d = policy::evaluate(tool, &json!({"path":"file.ts"}), &p, true).unwrap();
            assert_eq!(
                d.allow,
                matches!(mode, Mode::Interactive | Mode::Autonomous | Mode::Disabled),
                "{mode:?} {tool}"
            );
            assert_eq!(d.approval_request.is_some(), mode == Mode::Interactive);
            let outside =
                policy::evaluate(tool, &json!({"path":"../outside.ts"}), &p, true).unwrap();
            assert_eq!(
                outside.allow,
                matches!(mode, Mode::Interactive | Mode::Disabled)
            );
            assert_eq!(
                outside.approval_request.is_some(),
                mode == Mode::Interactive
            );
        }
    }
    let p = Policy {
        mode: Mode::Planning,
        cwd: "/project".into(),
        planning_read_only_tools: Some("block".into()),
        trusted_tools: vec![],
        extension_tools: vec![],
    };
    assert!(
        !policy::evaluate("editPreview", &json!({"path":"file"}), &p, true)
            .unwrap()
            .allow
    );
    let p = Policy {
        mode: Mode::Autonomous,
        planning_read_only_tools: None,
        ..p
    };
    assert!(
        policy::evaluate("diff", &json!({}), &p, true)
            .unwrap()
            .allow
    );
    assert!(
        policy::evaluate("runtimeObservation", &json!({}), &p, true)
            .unwrap()
            .allow
    );
    assert!(
        !policy::evaluate("unreviewedExtension", &json!({}), &p, true)
            .unwrap()
            .allow
    );
}
