use ag1_core::DomainError;
use std::collections::BTreeMap;

/// Amazon Event Stream CRCs cover both the prelude and full message separately.
/// Format reference: https://smithy.io/2.0/aws/amazon-eventstream.html
pub fn event_crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320u32 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}
#[derive(Debug, PartialEq)]
pub enum EventHeader {
    Bool(bool),
    Integer(i64),
    Bytes(Vec<u8>),
    Text(String),
    Timestamp(u64),
    Uuid([u8; 16]),
}
#[derive(Debug, PartialEq)]
pub struct AwsMessage {
    pub headers: BTreeMap<String, EventHeader>,
    pub payload: Vec<u8>,
}
impl AwsMessage {
    pub fn json_event(self) -> Result<serde_json::Value, DomainError> {
        let text = |key: &str| match self.headers.get(key) {
            Some(EventHeader::Text(v)) => Ok(v.as_str()),
            _ => Err(DomainError::InvalidInput("AWS event header".into())),
        };
        let kind = text(":message-type")?;
        let payload: serde_json::Value = serde_json::from_slice(&self.payload)
            .map_err(|_| DomainError::InvalidInput("AWS JSON payload".into()))?;
        let key = match kind {
            "event" => text(":event-type")?,
            "exception" => text(":exception-type")?,
            "error" => "error",
            _ => return Err(DomainError::InvalidInput("AWS event message type".into())),
        };
        Ok(serde_json::json!({key:payload}))
    }
}
pub struct AwsEventDecoder {
    buffer: Vec<u8>,
    max: usize,
    failed: bool,
}
impl AwsEventDecoder {
    /// `max` is an application resource budget, not a claimed service protocol limit.
    pub fn new(max: usize) -> Result<Self, DomainError> {
        if !(1024..=32 * 1024 * 1024).contains(&max) {
            return Err(DomainError::InvalidInput("AWS frame budget".into()));
        }
        Ok(Self {
            buffer: vec![],
            max,
            failed: false,
        })
    }
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<AwsMessage>, DomainError> {
        if self.failed {
            return Err(DomainError::Conflict);
        }
        let result = self.consume(bytes);
        if result.is_err() {
            self.failed = true;
            self.buffer.clear();
        }
        result
    }
    fn consume(&mut self, mut bytes: &[u8]) -> Result<Vec<AwsMessage>, DomainError> {
        if bytes.len() > 256 * 1024 {
            return Err(DomainError::LimitExceeded("HTTP read chunk".into()));
        }
        let mut messages = vec![];
        while !bytes.is_empty() {
            let needed = if self.buffer.len() < 12 {
                12
            } else {
                u32::from_be_bytes(self.buffer[..4].try_into().expect("prelude length")) as usize
            };
            let take = needed.saturating_sub(self.buffer.len()).min(bytes.len());
            self.buffer.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.buffer.len() < 12 {
                continue;
            }
            let total = u32::from_be_bytes(self.buffer[..4].try_into().expect("prelude")) as usize;
            let headers =
                u32::from_be_bytes(self.buffer[4..8].try_into().expect("prelude")) as usize;
            let expected = u32::from_be_bytes(self.buffer[8..12].try_into().expect("prelude"));
            if event_crc32(&self.buffer[..8]) != expected {
                return Err(DomainError::InvalidInput("AWS prelude checksum".into()));
            }
            if total < 16 || headers > total - 16 {
                return Err(DomainError::InvalidInput("AWS message lengths".into()));
            }
            if total > self.max {
                return Err(DomainError::LimitExceeded(
                    "AWS frame application budget".into(),
                ));
            }
            if self.buffer.len() < total {
                continue;
            }
            let checksum = u32::from_be_bytes(
                self.buffer[total - 4..]
                    .try_into()
                    .expect("message checksum"),
            );
            if event_crc32(&self.buffer[..total - 4]) != checksum {
                return Err(DomainError::InvalidInput("AWS message checksum".into()));
            }
            let fields = parse_headers(&self.buffer[12..12 + headers])?;
            messages.push(AwsMessage {
                headers: fields,
                payload: self.buffer[12 + headers..total - 4].to_vec(),
            });
            self.buffer.clear();
        }
        Ok(messages)
    }
    pub fn finish(&self) -> Result<(), DomainError> {
        if !self.failed && self.buffer.is_empty() {
            Ok(())
        } else {
            Err(DomainError::InvalidInput(
                "truncated or invalid AWS stream".into(),
            ))
        }
    }
}
fn take<'a>(bytes: &mut &'a [u8], count: usize) -> Result<&'a [u8], DomainError> {
    if bytes.len() < count {
        return Err(DomainError::InvalidInput("truncated AWS header".into()));
    }
    let (result, rest) = bytes.split_at(count);
    *bytes = rest;
    Ok(result)
}
fn parse_headers(mut bytes: &[u8]) -> Result<BTreeMap<String, EventHeader>, DomainError> {
    let mut headers = BTreeMap::new();
    while !bytes.is_empty() {
        let len = take(&mut bytes, 1)?[0] as usize;
        if len == 0 {
            return Err(DomainError::InvalidInput("empty AWS header name".into()));
        }
        let name = std::str::from_utf8(take(&mut bytes, len)?)
            .map_err(|_| DomainError::InvalidInput("AWS header UTF-8".into()))?
            .to_string();
        let kind = take(&mut bytes, 1)?[0];
        let value = match kind {
            0 => EventHeader::Bool(true),
            1 => EventHeader::Bool(false),
            2 => EventHeader::Integer(i8::from_be_bytes(
                take(&mut bytes, 1)?.try_into().expect("checked"),
            ) as i64),
            3 => EventHeader::Integer(i16::from_be_bytes(
                take(&mut bytes, 2)?.try_into().expect("checked"),
            ) as i64),
            4 => EventHeader::Integer(i32::from_be_bytes(
                take(&mut bytes, 4)?.try_into().expect("checked"),
            ) as i64),
            5 => EventHeader::Integer(i64::from_be_bytes(
                take(&mut bytes, 8)?.try_into().expect("checked"),
            )),
            6 | 7 => {
                let size =
                    u16::from_be_bytes(take(&mut bytes, 2)?.try_into().expect("checked")) as usize;
                let value = take(&mut bytes, size)?;
                if kind == 6 {
                    EventHeader::Bytes(value.to_vec())
                } else {
                    EventHeader::Text(
                        std::str::from_utf8(value)
                            .map_err(|_| DomainError::InvalidInput("AWS header UTF-8".into()))?
                            .into(),
                    )
                }
            }
            8 => EventHeader::Timestamp(u64::from_be_bytes(
                take(&mut bytes, 8)?.try_into().expect("checked"),
            )),
            9 => EventHeader::Uuid(take(&mut bytes, 16)?.try_into().expect("checked")),
            _ => return Err(DomainError::InvalidInput("unknown AWS header type".into())),
        };
        if headers.insert(name, value).is_some() {
            return Err(DomainError::InvalidInput("duplicate AWS header".into()));
        }
    }
    Ok(headers)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SseMessage {
    pub event: String,
    pub data: String,
}
/// Incremental UTF-8, CR/LF/CRLF and multiline SSE framing. Heartbeats carry no
/// semantic progress. Buffers are bounded independently of provider output.
pub struct SseDecoder {
    line: Vec<u8>,
    data: String,
    event: String,
    skip_lf: bool,
    max: usize,
}
impl SseDecoder {
    pub fn new(max_frame_bytes: usize) -> Result<Self, DomainError> {
        if !(1024..=2 * 1024 * 1024).contains(&max_frame_bytes) {
            return Err(DomainError::InvalidInput("SSE limit".into()));
        }
        Ok(Self {
            line: vec![],
            data: String::new(),
            event: String::new(),
            skip_lf: false,
            max: max_frame_bytes,
        })
    }
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<SseMessage>, DomainError> {
        if bytes.len() > 256 * 1024 {
            return Err(DomainError::LimitExceeded("HTTP read chunk".into()));
        }
        let mut out = vec![];
        for &byte in bytes {
            if self.skip_lf {
                self.skip_lf = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if byte == b'\r' || byte == b'\n' {
                self.finish_line(&mut out)?;
                self.skip_lf = byte == b'\r';
            } else {
                if self.line.len() + self.data.len() + self.event.len() >= self.max {
                    return Err(DomainError::LimitExceeded("SSE frame".into()));
                }
                self.line.push(byte);
            }
        }
        Ok(out)
    }
    fn finish_line(&mut self, out: &mut Vec<SseMessage>) -> Result<(), DomainError> {
        if self.line.is_empty() {
            if !self.data.is_empty() {
                self.data.pop();
                out.push(SseMessage {
                    event: std::mem::take(&mut self.event),
                    data: std::mem::take(&mut self.data),
                });
            } else {
                self.event.clear();
            }
            return Ok(());
        }
        let line = std::str::from_utf8(&self.line)
            .map_err(|_| DomainError::InvalidInput("SSE UTF-8".into()))?;
        let (key, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match key {
            "data" => {
                self.data.push_str(value);
                self.data.push('\n');
            }
            "event" => {
                self.event = value.into();
            }
            _ => (),
        }
        self.line.clear();
        Ok(())
    }
    /// An incomplete final frame is a transport error, never a fabricated terminal.
    pub fn finish(&self) -> Result<(), DomainError> {
        if self.line.is_empty() && self.data.is_empty() {
            Ok(())
        } else {
            Err(DomainError::InvalidInput("truncated SSE frame".into()))
        }
    }
    pub fn retained_bytes(&self) -> usize {
        self.line.len() + self.data.len() + self.event.len()
    }
}
