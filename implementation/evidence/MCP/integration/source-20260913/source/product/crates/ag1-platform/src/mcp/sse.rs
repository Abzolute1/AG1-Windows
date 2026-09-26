//! Incremental SSE framing; no reconnection, URL routing or opaque-state replay.
use super::{
    invalid,
    wire::{Limits, Message, decode_message},
};
use ag1_core::DomainError;

const MAX_EVENT: usize = 1024 * 1024;
pub(crate) struct Decoder {
    line: Vec<u8>,
    data: Vec<u8>,
    event: Option<String>,
    skip_lf: bool,
    first: bool,
    bytes: usize,
    events: usize,
}
impl Default for Decoder {
    fn default() -> Self {
        Self {
            line: vec![],
            data: vec![],
            event: None,
            skip_lf: false,
            first: true,
            bytes: 0,
            events: 0,
        }
    }
}
impl Decoder {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<Vec<Message>, DomainError> {
        let mut messages = Vec::new();
        self.bytes = self.bytes.saturating_add(bytes.len());
        if self.bytes > 8 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("MCP SSE bytes".into()));
        }
        for byte in bytes {
            if self.skip_lf {
                self.skip_lf = false;
                if *byte == b'\n' {
                    continue;
                }
            }
            if matches!(*byte, b'\n' | b'\r') {
                self.skip_lf = *byte == b'\r';
                if let Some(message) = self.line()? {
                    messages.push(message);
                }
            } else {
                if self.line.len() + self.data.len() >= MAX_EVENT {
                    return Err(DomainError::LimitExceeded("MCP SSE event".into()));
                }
                self.line.push(*byte);
            }
        }
        Ok(messages)
    }
    fn line(&mut self) -> Result<Option<Message>, DomainError> {
        let raw = std::mem::take(&mut self.line);
        let mut line = std::str::from_utf8(&raw).map_err(|_| invalid("SSE UTF-8"))?;
        if self.first {
            self.first = false;
            line = line.strip_prefix('\u{feff}').unwrap_or(line);
        }
        if line.is_empty() {
            self.events += 1;
            if self.events > 128 {
                return Err(DomainError::LimitExceeded("MCP SSE events".into()));
            }
            let event = self.event.take();
            if self.data.is_empty() {
                return Ok(None);
            }
            if event
                .as_deref()
                .is_some_and(|e| !e.is_empty() && e != "message")
            {
                return Err(invalid("unsupported SSE event type"));
            }
            self.data.pop(); // SSE joins data fields with LF and removes the last LF.
            if self.data.is_empty() {
                return Ok(None);
            } // legacy stream priming event
            let message = decode_message(&self.data, Limits::default())?;
            self.data.clear();
            return Ok(Some(message));
        }
        if line.starts_with(':') {
            return Ok(None);
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "data" => {
                if self.data.len() + value.len() + 1 > MAX_EVENT {
                    return Err(DomainError::LimitExceeded("MCP SSE data".into()));
                }
                self.data.extend_from_slice(value.as_bytes());
                self.data.push(b'\n');
            }
            "event" => {
                if value.len() > 128 {
                    return Err(invalid("SSE event name"));
                }
                self.event = Some(value.into());
            }
            "id" => {
                if value.len() > 1024 || value.contains('\0') {
                    return Err(invalid("SSE event ID"));
                }
                // Never use IDs as URLs or send Last-Event-ID without a separate
                // reviewed legacy resumption design.
            }
            "retry" => {
                if value.len() > 16 {
                    return Err(invalid("SSE retry field"));
                }
            }
            _ => {}
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_utf8_crlf_comments_and_multiline_data() {
        let mut decoder = Decoder::default();
        let input = "\u{feff}: keepalive\r\nevent: message\rdata: {\"jsonrpc\":\"2.0\",\r\ndata: \"id\":\"a\",\"result\":{\"text\":\"ø\"}}\r\n\r\n";
        let mut messages = vec![];
        for byte in input.as_bytes() {
            messages.extend(decoder.push(&[*byte]).unwrap());
        }
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].body["result"]["text"], "ø");
    }
    #[test]
    fn event_size_type_and_flood_are_bounded() {
        assert!(Decoder::default().push(&vec![b'x'; MAX_EVENT + 1]).is_err());
        assert!(
            Decoder::default()
                .push(b"event: endpoint\ndata: http://127.0.0.1/\n\n")
                .is_err()
        );
        assert!(Decoder::default().push(&b"\n".repeat(129)).is_err());
        assert!(Decoder::default().push(&[255, b'\n']).is_err());
    }
}
