//! Presentation syntax only: every resulting action enters native admission.
use super::*;
pub fn parse(text: &str) -> Result<w::Command> {
    let (name, rest) = text.split_once(' ').unwrap_or((text, ""));
    let args = if rest.trim().starts_with('{') {
        serde_json::from_str(rest).map_err(json_error)?
    } else {
        json!({})
    };
    Ok(match name {
        "/compact" => w::Command::Session {
            action: "compact".into(),
            args: json!({"focus":if rest.is_empty(){None}else{Some(rest)}}),
        },
        "/new" => w::Command::Session {
            action: "new".into(),
            args,
        },
        "/resume" => w::Command::Session {
            action: "resume".into(),
            args: json!({"session":rest}),
        },
        "/fork" => w::Command::Session {
            action: "fork".into(),
            args: json!({"entry":rest}),
        },
        "/tree" => w::Command::Session {
            action: "tree".into(),
            args,
        },
        "/export" => w::Command::Session {
            action: "export".into(),
            args,
        },
        "/steer" | "/followup" => w::Command::Session {
            action: if name == "/steer" {
                "steer"
            } else {
                "follow_up"
            }
            .into(),
            args: json!({"input":{"kind":"inline","text":rest}}),
        },
        "/model" => {
            let (provider, model) = rest
                .split_once('/')
                .ok_or_else(|| DomainError::InvalidInput("/model provider/model-id".into()))?;
            w::Command::Model {
                action: "select".into(),
                args: json!({"provider":provider,"model_id":model}),
            }
        }
        "/thinking" => w::Command::Model {
            action: "thinking".into(),
            args: json!({"level":rest}),
        },
        "/plan" => w::Command::Plan {
            action: "draft".into(),
            args: json!({"input":{"kind":"inline","text":rest},"previous":null,"feedback":null}),
        },
        "/task" => w::Command::Task {
            action: "view".into(),
            args,
        },
        "/context" | "/diagnostics" => w::Command::Diagnostics {
            action: "status".into(),
            args,
        },
        "/workflow" => w::Command::Runtime {
            action: "workflow".into(),
            args: json!({"path":rest}),
        },
        _ if name.starts_with('/') => {
            return Err(DomainError::Unavailable(format!(
                "Unknown client action {name}; use the explicit protocol command API"
            )));
        }
        _ => w::Command::Prompt {
            input: w::PromptInput::Inline { text: text.into() },
        },
    })
}
