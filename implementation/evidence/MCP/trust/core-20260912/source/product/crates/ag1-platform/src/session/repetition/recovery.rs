//! Conservative semantic recovery: count actual native failures, not proposed
//! actions or changing assistant narration. A corrected action always gets its
//! normal validation/execution opportunity until a durable pause is requested.
use ag1_core::runtime::repetition::{Event, MAX_RECORDS};
use serde_json::{Value, json};
use std::{collections::VecDeque, path::Path};

pub(super) struct Candidate {
    key: String,
    revision: String,
    failed: bool,
    redundant: bool,
    remedy: &'static str,
}
struct Record {
    key: String,
    revision: String,
    count: u64,
    call: String,
    failed: bool,
}
#[derive(Default)]
pub(super) struct Guard {
    records: VecDeque<Record>,
    pub(super) stop: Option<String>,
}
impl Guard {
    pub(super) fn observe(
        &mut self,
        tool: &str,
        call: &str,
        candidate: Option<Candidate>,
    ) -> Vec<Event> {
        let Some(candidate) = candidate else {
            return vec![];
        };
        if self.stop.is_some() || call.len() > 128 {
            return vec![];
        }
        let previous = self
            .records
            .iter()
            .position(|r| r.key == candidate.key)
            .and_then(|i| self.records.remove(i));
        let same = previous
            .as_ref()
            .is_some_and(|r| r.revision == candidate.revision && r.failed == candidate.failed);
        let count = if candidate.failed || candidate.redundant && same {
            if same {
                previous.as_ref().unwrap().count.saturating_add(1)
            } else {
                1
            }
        } else {
            0
        };
        let previous_call = previous
            .as_ref()
            .map_or(call, |r| r.call.as_str())
            .to_owned();
        self.records.push_back(Record {
            key: candidate.key.clone(),
            revision: candidate.revision,
            count,
            call: call.into(),
            failed: candidate.failed,
        });
        if self.records.len() > MAX_RECORDS {
            self.records.pop_front();
        }
        if count < 3 {
            return vec![];
        }
        self.stop = Some(format!(
            "Paused after three repeated {} in the same recovery case without observed recovery. The task is incomplete. Last native result: {tool} {call}. {} No success is inferred from this pause. Provide a new instruction to resume.",
            if candidate.failed {
                "native failures"
            } else {
                "no-progress checklist updates"
            },
            candidate.remedy
        ));
        vec![Event {
            action: "paused",
            tool_name: tool.into(),
            tool_call_id: call.into(),
            previous_tool_call_id: previous_call,
            signature: candidate.key,
            blocked_attempts: 0,
        }]
    }
}
fn digest(value: &Value) -> String {
    ag1_storage::digest(value.to_string().as_bytes())
}
pub(super) fn candidate(
    root: &Path,
    tool: &str,
    args: &Value,
    result: &Value,
    todo: Option<&Value>,
) -> Option<Candidate> {
    let failed = result["isError"] == true;
    // Bounded diagnostics only. Never hash/narration-match arbitrary model text.
    let text = result["content"]
        .as_array()?
        .iter()
        .filter_map(|b| b["text"].as_str())
        .flat_map(str::chars)
        .take(8192)
        .collect::<String>();
    let (family, target, revision, redundant, remedy) = match tool {
        "edit" | "editApply" | "editPreview" | "write" | "append" => {
            let path = super::resolve(root, args["path"].as_str()?);
            let revision = super::revision(root, &path)?;
            if failed
                && !(result["details"]["rejectedBeforeExecution"] == true
                    && (text.starts_with("Could not find")
                        || text.starts_with("Found ")
                        || text.starts_with("No changes made")
                        || text.contains("overlap")))
            {
                return None;
            }
            if !failed && tool == "editPreview" {
                return None;
            }
            (
                "file-edit",
                json!(path),
                revision,
                false,
                "Read a small current range and use unique exact oldText with the real indentation; do not retry stale or overlapping replacements.",
            )
        }
        "todo" => {
            if failed && !text.starts_with("Todo tool failed: InvalidInput(") {
                return None;
            }
            if !failed
                && args.get("todos").is_none()
                && args.get("planAction").is_none()
                && args["clear"] != true
            {
                return None;
            }
            let state = todo.cloned().unwrap_or(Value::Null);
            let revision = digest(&json!([state["items"], state["planId"], state["phase"]]));
            (
                "todo",
                json!("session-checklist"),
                revision,
                !failed,
                "Query todo with no arguments for current IDs and statuses. Keep completed identities; make a meaningful status change or explicitly archive/start a prospective plan.",
            )
        }
        "bash" => {
            let command = args["command"].as_str()?.trim();
            if command.is_empty() {
                return None;
            }
            let simple = command
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c));
            if simple
                && (!failed
                    || result["details"]["exitCode"] == 127 && text.contains("command not found"))
            {
                (
                    "missing-command",
                    json!(command),
                    "native-exit-127".into(),
                    false,
                    "The shell exists, but this executable was not found. Use the configured toolchain or confirm an available executable; do not attempt an installer or repeat the missing command.",
                )
            } else {
                let details = &result["details"];
                let code = details["exitCode"].as_i64()?;
                if details["termination"] != "exit" || failed && code == 0 {
                    return None;
                }
                // Exact executed command and COMPLETE output identity only.
                // Never equate programs by removing quoting, flags, pipes or
                // whitespace inside the command; never compare output excerpts.
                if details["fullOutputRetained"] != true {
                    return None;
                }
                let output = sha(&details["rawOutputArtifact"]["sha256"])?;
                (
                    "shell-failure",
                    json!([command, args["timeout"]]),
                    digest(&json!([code, output])),
                    false,
                    "The same command returned the same complete failure output repeatedly. Inspect the retained output and current source; change the failing input or prerequisite before retrying. Prior command effects remain real and are not rolled back.",
                )
            }
        }
        "check" => {
            let details = &result["details"];
            let source = details["sourceRevision"].as_str()?;
            let current = details["currentRevision"].as_str()?;
            let exits = details["exits"].as_array()?;
            if source.is_empty()
                || source != current
                || exits.is_empty()
                || exits.iter().any(|code| code.as_i64().is_none())
                || !matches!(details["lifecycle"].as_str(), Some("exited" | "failed"))
                || failed
                    && !exits
                        .iter()
                        .any(|code| code.as_i64().is_some_and(|code| code != 0))
            {
                return None;
            }
            let diagnostics = details["diagnostics"].as_array()?;
            let logs = details["logs"].as_array()?;
            if logs.is_empty() {
                return None;
            }
            // The Problems parser has byte/count/message caps. Only use its
            // structured identity when safely below those caps and it actually
            // located a compiler error, not just a generic process-exit notice.
            let structured = diagnostics.len() < 128
                && diagnostics.iter().any(|d| {
                    d["severity"] == "error" && (d["file"].is_string() || d["code"].is_string())
                })
                && diagnostics.iter().all(|d| {
                    d["message"]
                        .as_str()
                        .is_some_and(|m| m.chars().count() < 2048)
                })
                && logs.iter().all(|log| {
                    log["bytes"]
                        .as_u64()
                        .is_some_and(|bytes| bytes < 1024 * 1024)
                });
            let evidence = if !structured {
                json!(
                    logs.iter()
                        .map(|log| sha(&log["sha256"]))
                        .collect::<Option<Vec<_>>>()?
                )
            } else {
                json!(diagnostics)
            };
            (
                "compiler-check",
                json!([args, details["commands"]]),
                digest(&json!([source, exits, evidence])),
                false,
                "The same compiler diagnostics persist at the same source revision. Read the reported file/range and repair the current error, or resolve the configured toolchain prerequisite, before checking again. No passing check has been established.",
            )
        }
        "explore" => {
            let details = &result["details"];
            if !matches!(
                details["status"].as_str(),
                Some("failed" | "partial" | "completed")
            ) {
                return None;
            }
            let questions = args["questions"].as_array()?;
            if questions.is_empty() {
                return None;
            }
            let all_failed = failed && details["status"] == "failed" && details["completed"] == 0;
            let failures = details["recoveryFailures"].as_array()?;
            if all_failed
                && (failures.len() != questions.len()
                    || failures
                        .iter()
                        .any(|f| f["state"] != "failed" || sha(&f["errorDigest"]).is_none()))
            {
                return None;
            }
            // Partial groups must reset the same question group's failure
            // streak, never masquerade as all-failed or discarded findings.
            return Some(Candidate {
                key: digest(&json!(["explore", questions])),
                revision: if all_failed {
                    digest(&json!(failures))
                } else {
                    "progress".into()
                },
                failed: all_failed,
                redundant: false,
                remedy: "All workers failed repeatedly for the same questions. Inspect the retained worker errors, resolve their prerequisite or narrow the questions before launching more workers.",
            });
        }
        "task" => {
            if failed && result["details"]["state"] != "failed" {
                return None;
            }
            let prompt = args["prompt"].as_str()?;
            (
                "delegated-task",
                json!([
                    args["type"],
                    prompt,
                    result["details"]["model"],
                    args["model"],
                    args["resumeFrom"],
                    args["timeout"],
                    args["maxTurns"]
                ]),
                "settled-task".into(),
                false,
                "Inspect the retained child error/checkpoint, narrow the delegated task or resolve its prerequisite before resuming; do not launch the same failed task again.",
            )
        }
        _ => return None,
    };
    Some(Candidate {
        key: digest(&json!([family, target])),
        revision,
        failed,
        redundant,
        remedy,
    })
}

fn sha(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_history_and_only_pauses_after_three_same_revision_failures() {
        let mut guard = Guard::default();
        let item = |key: &str, revision: &str, failed| {
            Some(Candidate {
                key: key.into(),
                revision: revision.into(),
                failed,
                redundant: false,
                remedy: "Recover current evidence.",
            })
        };
        for i in 0..100 {
            guard.observe("edit", "a", item(&i.to_string(), "v1", true));
        }
        assert_eq!(guard.records.len(), MAX_RECORDS);
        guard.observe("edit", "a", item("file", "v1", true));
        guard.observe("edit", "b", item("file", "v1", true));
        guard.observe("edit", "c", item("file", "v2", true));
        guard.observe("edit", "d", item("file", "v2", false));
        for call in ["e", "f"] {
            assert!(
                guard
                    .observe("edit", call, item("file", "v2", true))
                    .is_empty()
            );
        }
        assert_eq!(
            guard.observe("edit", "g", item("file", "v2", true))[0].action,
            "paused"
        );
        assert!(guard.stop.unwrap().contains("task is incomplete"));
    }
    #[test]
    fn classifications_do_not_confuse_success_polling_or_narration_with_failure() {
        let root = Path::new("/unused");
        let error = json!({"isError":true,"content":[{"type":"text","text":"command not found"}],"details":{"exitCode":127}});
        assert!(candidate(root, "job", &json!({}), &error, None).is_none());
        assert!(
            candidate(
                root,
                "bash",
                &json!({"command":"echo x; missing"}),
                &error,
                None
            )
            .is_none()
        );
        let a = candidate(
            root,
            "bash",
            &json!({"command":"missing", "description":"first"}),
            &error,
            None,
        )
        .unwrap();
        let b = candidate(
            root,
            "bash",
            &json!({"command":" missing ", "description":"different"}),
            &error,
            None,
        )
        .unwrap();
        assert_eq!(a.key, b.key);
        let success = json!({"content":[],"details":{"exitCode":127}});
        assert!(
            !candidate(root, "bash", &json!({"command":"missing"}), &success, None)
                .unwrap()
                .failed
        );
    }
    #[test]
    fn successful_checklist_correction_and_task_resume_are_fresh_recovery_cases() {
        let root = Path::new("/unused");
        let args = json!({"todos":[{"id":"t1","title":"Build","status":"in_progress"}]});
        let state = json!({"items":args["todos"]});
        let failed = json!({"isError":true,"content":[{"type":"text","text":"Todo tool failed: InvalidInput(\"old identity\")"}]});
        let mut guard = Guard::default();
        for call in ["one", "two"] {
            guard.observe(
                "todo",
                call,
                candidate(root, "todo", &args, &failed, Some(&state)),
            );
        }
        let success = json!({"content":[]});
        assert!(
            guard
                .observe(
                    "todo",
                    "corrected",
                    candidate(root, "todo", &args, &success, Some(&state))
                )
                .is_empty()
        );
        assert!(guard.stop.is_none());
        let failure =
            json!({"isError":true,"details":{"state":"failed","model":"fixture"},"content":[]});
        let mut task =
            json!({"type":"explore","prompt":"Inspect the source","description":"first"});
        let first = candidate(root, "task", &task, &failure, None).unwrap();
        task["description"] = json!("different narration");
        assert_eq!(
            first.key,
            candidate(root, "task", &task, &failure, None).unwrap().key
        );
        task["resumeFrom"] = json!("retained-checkpoint");
        assert_ne!(
            first.key,
            candidate(root, "task", &task, &failure, None).unwrap().key
        );
    }

    #[test]
    fn shell_programs_require_exact_inputs_and_complete_output_not_excerpts() {
        let root = Path::new("/unused");
        let args = json!({"command":"cd src && printf '%s' failure >&2; exit 2","timeout":20});
        let result = json!({"isError":true,"content":[{"type":"text","text":"same displayed excerpt"}],
            "details":{"exitCode":2,"termination":"exit","fullOutputRetained":true,"rawOutputArtifact":{"sha256":"a".repeat(64)}}});
        let first = candidate(root, "bash", &args, &result, None).unwrap();
        let mut changed = result.clone();
        changed["details"]["rawOutputArtifact"]["sha256"] = json!("b".repeat(64));
        assert_ne!(
            first.revision,
            candidate(root, "bash", &args, &changed, None)
                .unwrap()
                .revision
        );
        changed["details"]["fullOutputRetained"] = json!(false);
        assert!(candidate(root, "bash", &args, &changed, None).is_none());
        let mut different = args.clone();
        different["command"] = json!("cd src && printf '%s' different >&2; exit 2");
        assert_ne!(
            first.key,
            candidate(root, "bash", &different, &result, None)
                .unwrap()
                .key
        );
        let mut guard = Guard::default();
        for id in ["one", "two", "three"] {
            guard.observe("bash", id, candidate(root, "bash", &args, &result, None));
        }
        assert!(
            guard
                .stop
                .unwrap()
                .contains("Prior command effects remain real")
        );
    }

    #[test]
    fn compiler_changes_and_partial_exploration_reset_repeated_failure_cases() {
        let root = Path::new("/unused");
        let check = json!({"isError":true,"content":[],"details":{"sourceRevision":"source-a","currentRevision":"source-a","exits":[101],"commands":[["cargo","check"]],"lifecycle":"failed",
            "diagnostics":[{"severity":"error","file":"src/main.rs","message":"type mismatch","code":"E0308","line":2}],"logs":[{"bytes":100,"sha256":"a".repeat(64)}]}});
        let first = candidate(root, "check", &json!({}), &check, None).unwrap();
        let mut changed = check.clone();
        changed["details"]["diagnostics"][0]["line"] = json!(3);
        assert_ne!(
            first.revision,
            candidate(root, "check", &json!({}), &changed, None)
                .unwrap()
                .revision
        );
        changed = check.clone();
        changed["details"]["currentRevision"] = json!("source-b");
        assert!(candidate(root, "check", &json!({}), &changed, None).is_none());
        changed = check.clone();
        changed["details"]["logs"][0]["bytes"] = json!(2 * 1024 * 1024);
        let capped = candidate(root, "check", &json!({}), &changed, None).unwrap();
        changed["details"]["logs"][0]["sha256"] = json!("b".repeat(64));
        assert_ne!(
            capped.revision,
            candidate(root, "check", &json!({}), &changed, None)
                .unwrap()
                .revision
        );
        let mut guard = Guard::default();
        for id in ["one", "two", "three"] {
            guard.observe(
                "check",
                id,
                candidate(root, "check", &json!({}), &check, None),
            );
        }
        assert!(guard.stop.unwrap().contains("No passing check"));
        let args = json!({"questions":["Which parser?"]});
        let failed = json!({"isError":true,"content":[],"details":{"status":"failed","completed":0,"recoveryFailures":[{"state":"failed","errorDigest":"a".repeat(64),"model":"fixture"}]}});
        let mut different = failed.clone();
        different["details"]["recoveryFailures"][0]["errorDigest"] = json!("b".repeat(64));
        assert_ne!(
            candidate(root, "explore", &args, &failed, None)
                .unwrap()
                .revision,
            candidate(root, "explore", &args, &different, None)
                .unwrap()
                .revision
        );
        let mut guard = Guard::default();
        for id in ["one", "two"] {
            guard.observe(
                "explore",
                id,
                candidate(root, "explore", &args, &failed, None),
            );
        }
        let mut partial = failed.clone();
        partial["details"]["status"] = json!("partial");
        partial["details"]["completed"] = json!(1);
        partial["isError"] = json!(false);
        guard.observe(
            "explore",
            "partial",
            candidate(root, "explore", &args, &partial, None),
        );
        assert!(
            guard
                .observe(
                    "explore",
                    "retry",
                    candidate(root, "explore", &args, &failed, None)
                )
                .is_empty()
        );
        assert!(guard.stop.is_none());
        partial["details"]["status"] = json!("cancelled");
        assert!(candidate(root, "explore", &args, &partial, None).is_none());
    }
}
