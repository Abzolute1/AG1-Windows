use super::*;

#[derive(Clone)]
pub(super) enum Request {
    Planning(planning::Request),
    Compact { focus: Option<String> },
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
