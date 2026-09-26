use super::*;

#[derive(Default)]
pub(in crate::session) struct Start {
    pub(in crate::session) messages: Vec<Value>,
    configuration: Option<Configuration>,
    system: Option<String>,
}
impl Start {
    pub(in crate::session) fn prepare(
        &self,
        request: &mut PreparationAction,
    ) -> Result<(), DomainError> {
        let current = request.configuration();
        if let (Some(base), Some(system)) = (&self.configuration, &self.system)
            && base.model == current.model
            && base.tools == current.tools
            && base.thinking == current.thinking
            && base.system_prompt == current.system_prompt
        {
            request.set_system_prompt(system.clone())?;
        }
        Ok(())
    }
}
impl NativeSession {
    pub(in crate::session) async fn before_agent_start(
        &self,
        tools: Arc<dyn ToolRunner>,
        input: &super::super::TurnInput,
        cancel: Arc<AtomicBool>,
        operation: &str,
        planning: Option<&super::super::planning::Turn>,
    ) -> Result<Start, DomainError> {
        if !tools.handles_session_event("before_agent_start") {
            return Ok(Start::default());
        }
        self.wait_for_commit().await?;
        let mut capture = self.capture_callback()?;
        if let Some(planning) = planning {
            capture.configuration = planning.configuration(&capture.configuration)?;
        }
        let mut event = json!({"type":"before_agent_start","prompt":ag1_core::context::hygiene::content(&input.message,"\n"),"systemPrompt":capture.configuration.system_prompt});
        let images = images(&input.message);
        if !images.is_empty() {
            event["images"] = json!(images);
        }
        let activity = self.retain_work()?;
        let origin = self.origin.clone();
        let operation = operation.to_owned();
        let timestamp = input.timestamp.clone();
        self.pool.submit(move||{
            let _activity=activity;
            let mut artifacts=ContextStore::new(&capture.store,capture.stamp.session().clone(),&origin,&operation,&timestamp,Default::default())?;
            let Some(result)=capture.call(tools.as_ref(),&event,&cancel,&mut artifacts)? else {return Ok(Start::default());};
            if !result.is_object(){return Err(DomainError::InvalidInput("before-agent result".into()));}
            let system=match result.get("systemPrompt") {
                None=>None,
                Some(value)=>Some(value.as_str().ok_or_else(||DomainError::InvalidInput("before-agent system prompt".into()))?.to_owned()),
            };
            if let Some(system)=&system {
                let mut checked=capture.configuration.clone();checked.system_prompt=system.clone();checked.validate()?;
            }
            let records=match result.get("messages") {
                None=>Vec::new(),
                Some(value)=>value.as_array().ok_or_else(||DomainError::InvalidInput("before-agent messages".into()))?.clone(),
            };
            if records.len()>128{return Err(DomainError::LimitExceeded("before-agent messages".into()));}
            let now=chrono::DateTime::parse_from_rfc3339(&timestamp).map_err(|_|DomainError::InvalidInput("before-agent timestamp".into()))?.timestamp_millis();
            let mut messages=Vec::with_capacity(records.len());
            for record in records {
                if record["customType"].as_str().is_none_or(|s|s.is_empty()||s.len()>128)
                    || !record["display"].is_boolean()
                    || !(record["content"].is_string()||record["content"].is_array()) {
                    return Err(DomainError::InvalidInput("before-agent custom message".into()));
                }
                let mut message=json!({"role":"custom","customType":record["customType"],"content":record["content"],"display":record["display"],"timestamp":now});
                if let Some(details)=record.get("details"){message["details"]=details.clone();}
                messages.push(message);
            }
            ag1_core::context::validate_history(&messages)?;
            Ok(Start {messages,configuration:Some(capture.configuration),system})
        }).await
    }
}
