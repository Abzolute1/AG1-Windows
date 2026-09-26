//! Literal artifact-line scanning with bounded scratch space. Full lines never
//! need to be materialized: matching continues after the transcript preview cap.
use crate::DomainError;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::{io::Read, sync::LazyLock};

#[derive(Clone, Debug)]
pub struct LiteralHit {
    pub line: u64,
    pub text: String,
    pub text_truncated: bool,
    pub byte_offset: u64,
    pub context_start: u64,
    pub context_end: u64,
}
pub struct LiteralScan {
    pub hits: Vec<LiteralHit>,
    pub total_matches: u64,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Copy, Default)]
struct MatchState {
    matched: usize,
    found: bool,
}
struct Pattern {
    chars: Vec<char>,
    prefix: Vec<usize>,
    ignore_case: bool,
    sigma: bool,
}
impl Pattern {
    fn new(query: &str, ignore_case: bool) -> Result<Self, DomainError> {
        if query.is_empty() || query.encode_utf16().count() > 512 {
            return Err(DomainError::InvalidInput(
                "Artifact literal query must contain 1..512 UTF-16 units".into(),
            ));
        }
        let query = if ignore_case {
            query.to_lowercase()
        } else {
            query.to_owned()
        };
        let chars = query.chars().collect::<Vec<_>>();
        let mut prefix = vec![0; chars.len()];
        for i in 1..chars.len() {
            let mut j = prefix[i - 1];
            while j > 0 && chars[i] != chars[j] {
                j = prefix[j - 1];
            }
            if chars[i] == chars[j] {
                j += 1;
            }
            prefix[i] = j;
        }
        let sigma = ignore_case && chars.iter().any(|ch| matches!(ch, 'σ' | 'ς'));
        Ok(Self {
            chars,
            prefix,
            ignore_case,
            sigma,
        })
    }
    fn push(&self, state: &mut MatchState, ch: char) {
        if state.found {
            return;
        }
        while state.matched > 0 && self.chars[state.matched] != ch {
            state.matched = self.prefix[state.matched - 1];
        }
        if self.chars[state.matched] == ch {
            state.matched += 1;
        }
        if state.matched == self.chars.len() {
            state.found = true;
        }
    }
    fn lowered(&self, state: &mut MatchState, ch: char) {
        if self.ignore_case {
            for ch in ch.to_lowercase() {
                self.push(state, ch);
            }
        } else {
            self.push(state, ch);
        }
    }
}
static CASE_IGNORABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\p{Case_Ignorable}$").unwrap());
static CASED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\p{Cased}$").unwrap());
#[derive(Default)]
struct Line {
    state: MatchState,
    // Sigma's lowercase form depends on arbitrarily distant case-ignorable
    // characters. Two tiny matcher states avoid buffering that whole suffix.
    sigma: Option<(MatchState, MatchState)>,
    preceded_by_cased: bool,
    preview: String,
    preview_units: usize,
}
impl Line {
    fn push(&mut self, pattern: &Pattern, ch: char) {
        let units = ch.len_utf16();
        if self.preview_units + units <= 4000 {
            self.preview.push(ch);
        }
        self.preview_units = (self.preview_units + units).min(4001);
        if !pattern.sigma {
            pattern.lowered(&mut self.state, ch);
            return;
        }
        let mut utf8 = [0; 4];
        let encoded = ch.encode_utf8(&mut utf8);
        let ignored = CASE_IGNORABLE.is_match(encoded);
        let cased = CASED.is_match(encoded);
        if let Some((mut normal, mut final_sigma)) = self.sigma.take() {
            if ignored {
                pattern.lowered(&mut normal, ch);
                pattern.lowered(&mut final_sigma, ch);
                self.sigma = Some((normal, final_sigma));
                return;
            }
            self.state = if cased { normal } else { final_sigma };
        }
        if ch == 'Σ' && self.preceded_by_cased {
            let (mut normal, mut final_sigma) = (self.state, self.state);
            pattern.push(&mut normal, 'σ');
            pattern.push(&mut final_sigma, 'ς');
            self.sigma = Some((normal, final_sigma));
        } else {
            pattern.lowered(&mut self.state, ch);
        }
        if !ignored {
            self.preceded_by_cased = cased;
        }
    }
    fn found(&self) -> bool {
        self.sigma
            .map(|(_, final_sigma)| final_sigma.found)
            .unwrap_or(self.state.found)
    }
}
struct Lines {
    pattern: Pattern,
    line: Line,
    line_number: u64,
    start: u64,
    previous_start: u64,
    offset: u64,
    pending_cr: bool,
    pending_context: Option<usize>,
    hits: Vec<LiteralHit>,
    total: u64,
    limit: usize,
}
impl Lines {
    fn finish_line(&mut self) {
        self.line_number += 1;
        if let Some(index) = self.pending_context.take() {
            self.hits[index].context_end = self.offset;
        }
        if self.line.found() {
            self.total += 1;
            if self.hits.len() < self.limit {
                self.pending_context = Some(self.hits.len());
                self.hits.push(LiteralHit {
                    line: self.line_number,
                    text: std::mem::take(&mut self.line.preview),
                    text_truncated: self.line.preview_units > 4000,
                    byte_offset: self.start,
                    context_start: self.previous_start,
                    context_end: self.offset,
                });
            }
        }
        self.previous_start = self.start;
        self.start = self.offset;
        self.line = Line::default();
        self.pending_cr = false;
    }
    fn push(&mut self, ch: char) {
        self.offset += ch.len_utf8() as u64;
        if ch == '\n' {
            self.finish_line();
            return;
        }
        if self.pending_cr {
            self.line.push(&self.pattern, '\r');
        }
        self.pending_cr = ch == '\r';
        if !self.pending_cr {
            self.line.push(&self.pattern, ch);
        }
    }
}
pub fn scan(
    mut input: impl Read,
    query: &str,
    ignore_case: bool,
    max_matches: usize,
    mut check: impl FnMut() -> Result<(), DomainError>,
) -> Result<LiteralScan, DomainError> {
    if max_matches == 0 || max_matches > 100 {
        return Err(DomainError::InvalidInput("Artifact match limit".into()));
    }
    let pattern = Pattern::new(query, ignore_case)?;
    let mut lines = Lines {
        pattern,
        line: Line::default(),
        line_number: 0,
        start: 0,
        previous_start: 0,
        offset: 0,
        pending_cr: false,
        pending_context: None,
        hits: Vec::new(),
        total: 0,
        limit: max_matches,
    };
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 65540];
    let mut pending = 0;
    loop {
        check()?;
        let count = input
            .read(&mut chunk[pending..65536])
            .map_err(|error| DomainError::Io(error.to_string()))?;
        if count == 0 {
            if pending != 0 {
                return Err(DomainError::InvalidInput(
                    "Artifact UTF-8 ends inside a codepoint".into(),
                ));
            }
            break;
        }
        hasher.update(&chunk[pending..pending + count]);
        let len = pending + count;
        if lines.offset + len as u64 > 256 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded(
                "Artifact exceeds 256 MiB".into(),
            ));
        }
        let valid = match std::str::from_utf8(&chunk[..len]) {
            Ok(_) => len,
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => {
                return Err(DomainError::InvalidInput(
                    "Artifact is not valid UTF-8".into(),
                ));
            }
        };
        for ch in std::str::from_utf8(&chunk[..valid]).unwrap().chars() {
            lines.push(ch);
        }
        chunk.copy_within(valid..len, 0);
        pending = len - valid;
    }
    if lines.pending_cr {
        lines.line.push(&lines.pattern, '\r');
    }
    if lines.offset > lines.start {
        lines.finish_line();
    }
    check()?;
    Ok(LiteralScan {
        hits: lines.hits,
        total_matches: lines.total,
        bytes: lines.offset,
        sha256: format!("{:x}", hasher.finalize()),
    })
}
