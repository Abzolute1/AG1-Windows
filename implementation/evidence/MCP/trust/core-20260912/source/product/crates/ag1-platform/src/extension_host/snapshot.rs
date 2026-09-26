use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::io::Write;

const MAX_SNAPSHOT: usize = 8 * 1024 * 1024;
struct Buffer(Vec<u8>);
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_SNAPSHOT - self.0.len() {
            return Err(std::io::Error::other("extension snapshot byte limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl ExtensionHost {
    /// Transfer large inspection context without increasing callback, result or
    /// action frame limits. The host mutex and scope cover the entire transfer.
    pub fn call_with_snapshot(
        &mut self,
        scope: &Scope,
        method: &str,
        args: &Value,
        snapshot: &Value,
        control: &ScanControl<'_>,
        bridge: &mut dyn Bridge,
    ) -> Result<Option<Value>, DomainError> {
        let mut args = args
            .as_object()
            .cloned()
            .ok_or_else(|| invalid("extension arguments"))?;
        if args.contains_key("snapshot")
            || args.contains_key("snapshotRef")
            || !snapshot.is_object()
        {
            return Err(invalid("extension snapshot arguments"));
        }
        let mut buffer = Buffer(Vec::new());
        serde_json::to_writer(&mut buffer, snapshot)
            .map_err(|_| DomainError::LimitExceeded("extension snapshot".into()))?;
        if buffer.0.len() <= 256 * 1024 {
            args.insert("snapshot".into(), snapshot.clone());
            return self.call(scope, method, &Value::Object(args), control, bridge);
        }
        let id = format!(
            "snapshot-{}-{}",
            self.incarnation,
            self.serial.checked_add(1).ok_or(DomainError::Conflict)?
        );
        let result = (|| {
            let ack = self.call(
                scope,
                "snapshot.begin",
                &json!({"id":id,"bytes":buffer.0.len(),"sha256":ag1_storage::digest(&buffer.0)}),
                control,
                &mut DenyBridge,
            )?;
            if ack != Some(json!({"id":id,"offset":0})) {
                return Err(DomainError::Conflict);
            }
            let mut offset = 0;
            for chunk in buffer.0.chunks(65536) {
                let ack = self.call(
                    scope,
                    "snapshot.append",
                    &json!({"id":id,"offset":offset,"data":STANDARD.encode(chunk)}),
                    control,
                    &mut DenyBridge,
                )?;
                offset += chunk.len();
                if ack != Some(json!({"id":id,"offset":offset})) {
                    return Err(DomainError::Conflict);
                }
            }
            args.insert("snapshotRef".into(), json!(id));
            self.call(scope, method, &Value::Object(args), control, bridge)
        })();
        if result.is_err() {
            let _ = self.stop();
        }
        result
    }
}
