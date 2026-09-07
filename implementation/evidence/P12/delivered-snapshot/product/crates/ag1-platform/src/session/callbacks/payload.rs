use super::*;

pub(in crate::session) struct PayloadInput {
    pub tools: Arc<dyn ToolRunner>,
    pub cancel: Arc<AtomicBool>,
    pub body: Vec<u8>,
    pub operation: String,
    pub timestamp: String,
}
impl NativeSession {
    pub(in crate::session) async fn payload_callback(
        &self,
        preparation: &PreparationAction,
        input: PayloadInput,
    ) -> Result<(Vec<u8>, u64), DomainError> {
        let mut capture = self.capture_callback()?;
        capture.request_view(preparation)?;
        let activity = self.retain_work()?;
        let origin = self.origin.clone();
        self.pool
            .submit(move || {
                let _activity = activity;
                let payload: Value = serde_json::from_slice(&input.body)
                    .map_err(|_| DomainError::InvalidInput("provider JSON payload".into()))?;
                if !payload.is_object() {
                    return Err(DomainError::InvalidInput("provider JSON payload".into()));
                }
                let mut artifacts = ContextStore::new(
                    &capture.store,
                    capture.stamp.session().clone(),
                    &origin,
                    &input.operation,
                    &input.timestamp,
                    Default::default(),
                )?;
                let Some(result) = capture.call(
                    input.tools.as_ref(),
                    &json!({"type":"before_provider_request","payload":payload}),
                    &input.cancel,
                    &mut artifacts,
                )?
                else {
                    return Ok((input.body, 0));
                };
                if !result.is_object() {
                    return Err(DomainError::InvalidInput(
                        "extension provider payload".into(),
                    ));
                }
                // Endpoint, credentials, model selection and executable tool schemas
                // belong to native services. Their dedicated APIs rebuild admission;
                // a wire-body callback cannot silently replace those capabilities.
                for pointer in [
                    "/model",
                    "/modelId",
                    "/tools",
                    "/functions",
                    "/toolConfig",
                    "/stream",
                    "/request/model",
                    "/request/tools",
                    "/request/toolConfig",
                ] {
                    if payload.pointer(pointer) != result.pointer(pointer) {
                        return Err(DomainError::PermissionDenied);
                    }
                }
                if result == payload {
                    return Ok((input.body, 0));
                }
                let extra = added_units(&payload, &result).div_ceil(3);
                let body = serde_json::to_vec(&result)
                    .map_err(|_| DomainError::InvalidInput("extension provider payload".into()))?;
                Ok((body, extra))
            })
            .await
    }
}
// Count every changed/new value without subtracting removed content. This is a
// conservative addition to the compiled estimate: removing a large encoded image
// must not provide free admission for newly inserted text. Unchanged media is not
// charged twice. The callback transport already bounds depth, nodes and bytes.
fn added_units(before: &Value, after: &Value) -> u64 {
    if before == after {
        return 0;
    }
    match after {
        Value::Object(fields) => {
            2 + fields
                .iter()
                .filter(|(key, value)| before.get(key) != Some(*value))
                .map(|(key, value)| {
                    key.encode_utf16().count() as u64
                        + 4
                        + added_units(before.get(key).unwrap_or(&Value::Null), value)
                })
                .sum::<u64>()
        }
        Value::Array(values) => {
            2 + values
                .iter()
                .enumerate()
                .filter(|(index, value)| before.get(*index) != Some(*value))
                .map(|(index, value)| {
                    1 + added_units(before.get(index).unwrap_or(&Value::Null), value)
                })
                .sum::<u64>()
        }
        Value::String(text) => 2 + text.encode_utf16().count() as u64,
        Value::Null => 4,
        value => value.to_string().len() as u64,
    }
}
