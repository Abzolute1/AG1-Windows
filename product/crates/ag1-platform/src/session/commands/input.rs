use super::*;
use serde_json::Value;
impl SessionCommands {
    fn expand_skill(&self, text: &str) -> Result<String, DomainError> {
        match self.runtime().configuration()?.prompt {
            Some(prompt) => prompt.expand_skill(text),
            None => Ok(text.into()),
        }
    }
    pub(super) fn input_message(
        &self,
        input: &w::PromptInput,
        now: u64,
    ) -> Result<Value, DomainError> {
        let text = match input {
            w::PromptInput::Inline { text } => text.clone(),
            w::PromptInput::Artifact { artifact }
            | w::PromptInput::MessageArtifact { artifact } => self
                .artifacts()?
                .bounded_text(artifact.as_str(), MAX_PROMPT_BYTES)?,
        };
        if matches!(input, w::PromptInput::MessageArtifact { .. }) {
            let mut message: Value = serde_json::from_str(&text)
                .map_err(|_| DomainError::InvalidInput("attachment message JSON".into()))?;
            let fields = message
                .as_object()
                .ok_or_else(|| DomainError::InvalidInput("attachment message".into()))?;
            if fields
                .keys()
                .any(|k| !["role", "content", "timestamp"].contains(&k.as_str()))
                || message["role"] != "user"
            {
                return Err(DomainError::InvalidInput("user message required".into()));
            }
            let content = message["content"]
                .as_array()
                .filter(|v| !v.is_empty() && v.len() <= 16)
                .ok_or_else(|| DomainError::InvalidInput("attachment content".into()))?;
            for block in content {
                match block["type"].as_str() {
                    Some("text") if block["text"].is_string() => (),
                    Some("image") => {
                        let configuration = self.runtime().configuration()?;
                        if !configuration.model["input"]
                            .as_array()
                            .is_some_and(|v| v.iter().any(|s| s == "image"))
                        {
                            return Err(DomainError::Unavailable(
                                "Selected model has no admitted vision capability".into(),
                            ));
                        }
                        if !matches!(
                            block["mimeType"].as_str(),
                            Some("image/png" | "image/jpeg" | "image/webp" | "image/gif")
                        ) {
                            return Err(DomainError::InvalidInput("image media type".into()));
                        }
                        let bytes =
                            STANDARD
                                .decode(block["data"].as_str().ok_or_else(|| {
                                    DomainError::InvalidInput("image data".into())
                                })?)
                                .map_err(|_| DomainError::InvalidInput("image base64".into()))?;
                        let format = image::guess_format(&bytes)
                            .map_err(|_| DomainError::InvalidInput("image format".into()))?;
                        if Some(format.to_mime_type()) != block["mimeType"].as_str() {
                            return Err(DomainError::InvalidInput(
                                "image MIME does not match content".into(),
                            ));
                        }
                        let dimensions = image::ImageReader::new(std::io::Cursor::new(&bytes))
                            .with_guessed_format()
                            .map_err(|_| DomainError::InvalidInput("image format".into()))?
                            .into_dimensions()
                            .map_err(|_| DomainError::InvalidInput("image dimensions".into()))?;
                        // Keep original desktop screenshots, including 4K/8K, while bounding decoded memory.
                        if dimensions.0 > 16384 || dimensions.1 > 16384
                            || u64::from(dimensions.0) * u64::from(dimensions.1) > 32 * 1024 * 1024 {
                            return Err(DomainError::LimitExceeded("image dimensions".into()));
                        }
                        image::load_from_memory(&bytes)
                            .map_err(|_| DomainError::InvalidInput("invalid image".into()))?;
                    }
                    _ => return Err(DomainError::InvalidInput("Unsupported user block".into())),
                }
            }
            let combined = content.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n");
            let expanded = self.expand_skill(&combined)?;
            if expanded != combined {
                // Keep image blocks intact and replace only the textual instruction.
                let blocks = message["content"].as_array_mut().unwrap();
                blocks.retain(|b| b["type"] != "text");
                blocks.insert(0, json!({"type":"text","text":expanded}));
            }
            message["timestamp"] = json!(now);
            Ok(message)
        } else {
            Ok(json!({"role":"user","content":[{"type":"text","text":self.expand_skill(&text)?}],"timestamp":now}))
        }
    }
}
