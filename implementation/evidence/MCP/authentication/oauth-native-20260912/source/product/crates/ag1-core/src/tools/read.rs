//! Bounded text window projection. Evidence references are attached by the byte-snapshot host.
use super::truncate::{self, Limits};
use crate::DomainError;
use serde_json::{Value, json};
fn number(args: &Value, name: &str) -> Result<Option<usize>, DomainError> {
    let Some(value) = args.get(name) else {
        return Ok(None);
    };
    let n = value
        .as_f64()
        .ok_or_else(|| DomainError::InvalidInput(format!("{name} must be an integer")))?;
    if !n.is_finite()
        || n.fract() != 0.0
        || n < 0.0
        || n > u32::MAX as f64
        || name == "limit" && n == 0.0
    {
        return Err(DomainError::InvalidInput(format!(
            "{name} must be a {} integer",
            if name == "limit" {
                "positive"
            } else {
                "nonnegative"
            }
        )));
    }
    Ok(Some(n as usize))
}
fn size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
pub fn text(args: &Value, bytes: &[u8]) -> Result<Value, DomainError> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("text snapshot bytes".into()));
    }
    let mut window = TextWindow::new(args)?;
    for chunk in bytes.chunks(65536) {
        window.push(chunk)?;
    }
    window.finish()
}
/// Incremental UTF-8 text projection. The host hashes exactly the same raw chunks
/// and admits the result only after descriptor identity/metadata validation.
pub struct TextWindow {
    path: String,
    offset: Option<usize>,
    start: usize,
    limit: Option<usize>,
    line: usize,
    total: usize,
    raw_bytes: usize,
    pending: Vec<u8>,
    initial: bool,
    stopped: bool,
    current: String,
    current_bytes: usize,
    selected: Vec<String>,
    sample_bytes: usize,
    first_size: usize,
    first_exact: bool,
}
impl TextWindow {
    pub fn new(args: &Value) -> Result<Self, DomainError> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("path".into()))?;
        if path.len() > 4096 {
            return Err(DomainError::LimitExceeded("read path".into()));
        }
        let offset = number(args, "offset")?;
        let start = offset.unwrap_or(1).saturating_sub(1);
        let limit = number(args, "limit")?;
        Ok(Self {
            path: path.into(),
            offset,
            start,
            limit,
            line: 0,
            total: 1,
            raw_bytes: 0,
            pending: vec![],
            initial: true,
            stopped: false,
            current: String::new(),
            current_bytes: 0,
            selected: vec![],
            sample_bytes: 0,
            first_size: 0,
            first_exact: true,
        })
    }
    pub fn resident_bytes(&self) -> usize {
        self.current.capacity()
            + self.selected.iter().map(String::capacity).sum::<usize>()
            + self.pending.capacity()
            + self.selected.capacity() * std::mem::size_of::<String>()
            + self.path.capacity()
            + std::mem::size_of::<Self>()
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), DomainError> {
        if bytes.len() > 65536 || self.raw_bytes.saturating_add(bytes.len()) > 64 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("streamed text bytes".into()));
        }
        self.raw_bytes += bytes.len();
        self.total += bytes.iter().filter(|b| **b == b'\n').count();
        if self.stopped {
            return Ok(());
        }
        let mut buffer = std::mem::take(&mut self.pending);
        buffer.extend_from_slice(bytes);
        let mut offset = 0;
        while offset < buffer.len() && !self.stopped {
            match std::str::from_utf8(&buffer[offset..]) {
                Ok(text) => {
                    for c in text.chars() {
                        self.character(c);
                        if self.stopped {
                            break;
                        }
                    }
                    offset = buffer.len();
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    for c in std::str::from_utf8(&buffer[offset..offset + valid])
                        .unwrap()
                        .chars()
                    {
                        self.character(c);
                        if self.stopped {
                            break;
                        }
                    }
                    offset += valid;
                    if let Some(length) = error.error_len() {
                        self.character('\u{fffd}');
                        offset += length;
                    } else {
                        self.pending.extend_from_slice(&buffer[offset..]);
                        break;
                    }
                }
            }
        }
        Ok(())
    }
    fn capture_line(&self) -> bool {
        self.line >= self.start && self.line - self.start < self.limit.unwrap_or(usize::MAX)
    }
    fn character(&mut self, c: char) {
        if self.stopped {
            return;
        }
        if self.initial {
            self.initial = false;
            if c == '\u{feff}' {
                return;
            }
        }
        if c == '\n' {
            if self.capture_line() {
                self.push_line();
            }
            self.line += 1;
            self.current.clear();
            self.current_bytes = 0;
            return;
        }
        if self.capture_line() {
            self.current_bytes += c.len_utf8();
            if self.current.len() <= 100 * 1024 {
                self.current.push(c);
            }
            if self.line == self.start && self.current_bytes > 100 * 1024 {
                self.first_exact = false;
                self.push_line();
            }
        }
    }
    fn push_line(&mut self) {
        if self.selected.is_empty() {
            self.first_size = self.current_bytes;
        }
        self.sample_bytes += self.current_bytes + usize::from(!self.selected.is_empty());
        self.selected.push(std::mem::take(&mut self.current));
        self.current_bytes = 0;
        if self.selected.len() >= self.limit.unwrap_or(2001).min(2001)
            || self.sample_bytes > 100 * 1024
        {
            self.stopped = true;
        }
    }
    pub fn finish(mut self) -> Result<Value, DomainError> {
        if !self.stopped && !self.pending.is_empty() {
            self.character('\u{fffd}');
            self.pending.clear();
        }
        if !self.stopped && self.capture_line() {
            self.push_line();
        }
        let path = &self.path;
        let offset = self.offset;
        let limit = self.limit;
        let start = self.start;
        let start_display = start + 1;
        let total = self.total;
        if start >= total {
            return Err(DomainError::InvalidInput(format!(
                "Offset {} is beyond end of file ({total} lines total)",
                offset.unwrap_or(1)
            )));
        }
        let selected_total = limit.unwrap_or(total - start).min(total - start);
        let sampled = self.selected.join("\n");
        let first_exact = self.first_exact;
        let first_size = self.first_size;
        let mut truncated = truncate::truncate(&sampled, Limits::default(), false);
        truncated.total_lines = selected_total;
        // A very long later line cannot fit the result, so count its bytes without
        // retaining its whole body. The reference's truncation accounting stays exact.
        truncated.total_bytes = self.sample_bytes;
        let (output, progress, include_truncation) = if truncated.first_line_exceeds_limit {
            let first_size = if first_exact {
                size(first_size)
            } else {
                "over 50.0KB".into()
            };
            let output = format!(
                "[Line {start_display} is {first_size}, exceeds 50.0KB limit. Use bash: sed -n '{start_display}p' {path} | head -c 51200]"
            );
            (
                output,
                json!({"startLine":start_display,"endLine":start_display,"totalLines":total,"hasMore":start_display<=total,"nextOffset":start_display,"completion":"partial","reason":"line_too_large"}),
                true,
            )
        } else if truncated.truncated {
            let end = start_display + truncated.output_lines - 1;
            let next = end + 1;
            let by_lines = truncated.truncated_by.as_deref() == Some("lines");
            let notice = if by_lines {
                format!("[lines {start_display}-{end}/{total}; next offset={next}]")
            } else {
                format!("[lines {start_display}-{end}/{total}; byte limit hit; next offset={next}]")
            };
            (
                format!("{}\n\n{notice}", truncated.content),
                json!({"startLine":start_display,"endLine":end,"totalLines":total,"hasMore":next<=total,"nextOffset":next,"completion":"partial","reason":if by_lines{"line_limit"}else{"byte_limit"}}),
                true,
            )
        } else if limit.is_some() && selected_total < total - start {
            let next = start_display + selected_total;
            let end = next - 1;
            (
                format!(
                    "{}\n\n[{} more lines; next offset={next}]",
                    truncated.content,
                    total - end
                ),
                json!({"startLine":start_display,"endLine":end,"totalLines":total,"hasMore":next<=total,"nextOffset":next,"completion":"partial","reason":"user_limit"}),
                false,
            )
        } else {
            let count = if truncated.content.is_empty() {
                0
            } else {
                truncated.content.split('\n').count()
            };
            let end = if count > 0 {
                start_display + count - 1
            } else {
                start_display
            };
            (
                truncated.content.clone(),
                json!({"startLine":start_display,"endLine":end,"totalLines":total,"hasMore":false,"completion":"complete","reason":"complete"}),
                false,
            )
        };
        let mut details = json!({"progress":progress});
        if include_truncation {
            details["truncation"] = json!(truncated);
        }
        Ok(json!({"content":[{"type":"text","text":output}],"details":details}))
    }
}
