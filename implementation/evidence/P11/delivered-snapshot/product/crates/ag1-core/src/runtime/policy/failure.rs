//! Bounded failure diagnosis for repair guidance. A diagnosis is not a verified
//! patch receipt and cannot turn arbitrary tool text into completion evidence.
use super::pattern;
use crate::{DomainError, runtime::verification::trim};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::LazyLock};

static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;]*m").unwrap());
static NOISE: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"^tap version [0-9]+|^---$|^error:\s*\|-$|^#?\s*[0-9]+\.\.[0-9]+$|^#?\s*(tests|suites|pass|fail|cancelled|skipped|todo|duration_ms)\b",
    )
});
static PATHS: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"(?:^|\s)([A-Za-z0-9_.@/-]+\.(?:mjs|cjs|js|jsx|ts|tsx|json))(?::[0-9]+)?")
});
static TIMEOUT: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"timed?\s*out|timeout|exceeded.*timeout"));
static IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"err_module_not_found|cannot find module|module not found|failed to resolve|enoent.*(?:open|stat)",
    )
});
static SYNTAX: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"syntaxerror|unexpected token|unterminated|string literal|missing \)|ts[0-9]{4}")
});
static ENVIRONMENT: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"eacces|permission denied|network|connection refused|address already in use|spawn .* enoent",
    )
});
static ASSERTION: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"assertionerror|assert\.|expected|actual|not equal|strictequal|deepstrictequal|\bnot ok\b",
    )
});
static IMPORTANT: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"assertionerror|syntaxerror|err_module_not_found|cannot find module|timed?\s*out|timeout|error",
    )
});
static FALLBACK: LazyLock<Regex> = LazyLock::new(|| pattern(r"\bnot ok\b|expected|actual"));
static NUMBER: LazyLock<Regex> = LazyLock::new(|| pattern(r"\b[0-9]+(?:\.[0-9]+)?\b"));
static LOCATION: LazyLock<Regex> = LazyLock::new(|| pattern(r":[0-9]+:[0-9]+"));
static LINE: LazyLock<Regex> = LazyLock::new(|| pattern(r":[0-9]+"));
static SPACE: LazyLock<Regex> = LazyLock::new(|| pattern(r"\s+"));

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Assertion,
    Syntax,
    Import,
    Timeout,
    Environment,
    Unrelated,
    Unknown,
}
impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Assertion => "assertion",
            Self::Syntax => "syntax",
            Self::Import => "import",
            Self::Timeout => "timeout",
            Self::Environment => "environment",
            Self::Unrelated => "unrelated",
            Self::Unknown => "unknown",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub kind: Kind,
    pub summary: String,
    pub signature: String,
    pub paths: Vec<String>,
}
// Preserve Unicode scalars when the source's UTF-16 slice would split a pair.
// The complete original output remains in native custody outside this preview.
fn preview(text: &str, maximum: usize) -> String {
    let mut units = 0;
    text.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= maximum
        })
        .collect()
}
pub fn classify(output: &str, relevant_paths: &[String]) -> Result<Failure, DomainError> {
    if output.len() > 4 * 1024 * 1024
        || relevant_paths.len() > 1000
        || relevant_paths.iter().any(|p| p.len() > 4096)
    {
        return Err(DomainError::LimitExceeded(
            "verifier failure diagnosis".into(),
        ));
    }
    let cleaned = output.replace('\r', "");
    let cleaned = ANSI.replace_all(&cleaned, "");
    let mut paths = Vec::new();
    let mut seen = BTreeSet::new();
    for capture in PATHS.captures_iter(&cleaned) {
        let path = capture.get(1).unwrap().as_str();
        let path = path.strip_prefix("./").unwrap_or(path);
        if seen.insert(path) {
            if paths.len() == 4096 || path.len() > 4096 {
                return Err(DomainError::LimitExceeded("verifier failure paths".into()));
            }
            paths.push(path.to_owned());
        }
    }
    let raw_lower = output.to_ascii_lowercase();
    let unrelated = !paths.is_empty()
        && !relevant_paths.is_empty()
        && !paths.iter().any(|path| {
            relevant_paths.iter().any(|candidate| {
                path == candidate
                    || path.ends_with(&format!("/{candidate}"))
                    || candidate.ends_with(&format!("/{path}"))
            })
        });
    let kind = if TIMEOUT.is_match(&raw_lower) {
        Kind::Timeout
    } else if IMPORT.is_match(&raw_lower) {
        Kind::Import
    } else if SYNTAX.is_match(&raw_lower) {
        Kind::Syntax
    } else if ENVIRONMENT.is_match(&raw_lower) {
        Kind::Environment
    } else if unrelated {
        Kind::Unrelated
    } else if ASSERTION.is_match(&raw_lower) {
        Kind::Assertion
    } else {
        Kind::Unknown
    };
    // Two bounded scans avoid retaining a vector for every output line.
    let mut important = None;
    let mut fallback = None;
    for line in cleaned.split('\n').map(trim).filter(|s| !s.is_empty()) {
        let lower = line.to_ascii_lowercase();
        if NOISE.is_match(&lower) {
            continue;
        }
        if important.is_none() && IMPORTANT.is_match(&lower) {
            important = Some(line);
        }
        if fallback.is_none() && FALLBACK.is_match(&lower) {
            fallback = Some(line);
        }
    }
    let important = important.or(fallback);
    let rest = cleaned.split('\n').map(trim).filter(|line| {
        !line.is_empty() && Some(*line) != important && !NOISE.is_match(&line.to_ascii_lowercase())
    });
    let lines: Vec<_> = important.into_iter().chain(rest).take(4).collect();
    let summary = if lines.is_empty() {
        "verification failed without a useful textual error".into()
    } else {
        preview(&lines.join(" | "), 500)
    };
    let signature = format!("{}:{summary}", kind.as_str()).to_lowercase();
    let signature = ANSI.replace_all(&signature, "");
    let signature = NUMBER.replace_all(&signature, "<n>");
    let signature = LOCATION.replace_all(&signature, ":<n>:<n>");
    let signature = LINE.replace_all(&signature, ":<n>");
    let signature = SPACE.replace_all(&signature, " ");
    Ok(Failure {
        kind,
        summary,
        signature: preview(trim(&signature), 240),
        paths,
    })
}
