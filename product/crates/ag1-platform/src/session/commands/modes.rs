use super::*;

#[derive(Clone)]
pub(super) enum Request {
    Print,
    Resource {
        name: String,
        args: serde_json::Value,
    },
    Extension {
        method: String,
        args: serde_json::Value,
    },
    Planning(planning::Request),
    Compact {
        focus: Option<String>,
    },
    Tree(super::super::tree::TreeOptions),
}
impl Request {
    pub(super) fn compact(&self) -> bool {
        matches!(self, Self::Compact { .. } | Self::Tree(_))
    }
}
pub(super) fn queued(
    command: &w::Command,
) -> Result<Option<(Request, w::PromptInput)>, w::ProtocolError> {
    if let w::Command::Runtime { action, args } = command
        && action == "resource_action"
    {
        let name = args["name"]
            .as_str()
            .filter(|s| matches!(*s, "createSkill" | "managePackage" | "configurePackage"))
            .ok_or_else(|| w::ProtocolError::Malformed {
                detail: "resource helper name".into(),
            })?;
        return Ok(Some((
            Request::Resource {
                name: name.into(),
                args: args["arguments"].clone(),
            },
            w::PromptInput::Inline {
                text: String::new(),
            },
        )));
    }
    if let w::Command::Runtime { action, args } = command
        && matches!(
            action.as_str(),
            "extension_command" | "extension_shortcut" | "extension_flag"
        )
    {
        let method = match action.as_str() {
            "extension_command" => "command",
            "extension_flag" => "flags.set",
            _ => "shortcut",
        }
        .to_owned();
        let key = if method == "shortcut" { "key" } else { "name" };
        if !args[key]
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 256)
            || args.to_string().len() > 16384
        {
            return Err(w::ProtocolError::Malformed {
                detail: "extension client command".into(),
            });
        }
        return Ok(Some((
            Request::Extension {
                method,
                args: args.clone(),
            },
            w::PromptInput::Inline {
                text: String::new(),
            },
        )));
    }
    if let w::Command::Session { action, args } = command
        && action == "print"
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Print {
            input: w::PromptInput,
        }
        let args: Print =
            serde_json::from_value(args.clone()).map_err(|_| w::ProtocolError::Malformed {
                detail: "print input".into(),
            })?;
        return Ok(Some((Request::Print, args.input)));
    }
    if let Some((request, input)) = planning::queued(command)? {
        return Ok(Some((Request::Planning(request), input)));
    }
    if let w::Command::Session { action, args } = command
        && action == "tree"
    {
        let options: super::super::tree::TreeOptions = serde_json::from_value(args.clone())
            .map_err(|_| w::ProtocolError::Malformed {
                detail: "tree options".into(),
            })?;
        options.validate().map_err(super::domain)?;
        return Ok(Some((
            Request::Tree(options),
            w::PromptInput::Inline {
                text: String::new(),
            },
        )));
    }
    if let w::Command::Session { action, args } = command
        && action == "compact"
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Arguments {
            focus: Option<String>,
        }
        let args: Arguments =
            serde_json::from_value(args.clone()).map_err(|_| w::ProtocolError::Malformed {
                detail: "compaction arguments".into(),
            })?;
        if args
            .focus
            .as_ref()
            .is_some_and(|text| text.len() > 1024 * 1024)
        {
            return Err(w::ProtocolError::Malformed {
                detail: "compaction focus limit".into(),
            });
        }
        return Ok(Some((
            Request::Compact { focus: args.focus },
            w::PromptInput::Inline {
                text: String::new(),
            },
        )));
    }
    Ok(None)
}
