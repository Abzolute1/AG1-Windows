//! Controlled task intent and bounded evidence guidance from a captured index.
//! A policy is host-selected advice, not a filesystem permission or proof that
//! a verifier passed. The flexible runtime does not opt into tool restrictions.
use super::verification::Index;
use crate::{DomainError, tools::list::compare};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::LazyLock};

pub mod evidence;
pub mod failure;
pub mod orchestrator;
pub mod repair;
pub mod stop;

// The reference expressions use JavaScript /i without /u: ASCII word boundaries
// and ASCII case folding. Applying Rust's Unicode \b or /i changes task intent.
fn pattern(source: &str) -> Regex {
    Regex::new(
        &source
            .replace(r"\b", r"(?-u:\b)")
            .replace(r"\w", "[A-Za-z0-9_]")
            .replace(
                r"\s",
                r"[\t\n\x0B\x0C\r \u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000\uFEFF]",
            ),
    )
    .expect("pinned task policy expression")
}
static RESEARCH: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"\b(latest|current|today|recent|research|search online|web search|look up|citation|pricing|availability|breaking changes?)\b",
    )
});
static FRONTEND: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"\b(frontend|ui|ux|css|tailwind|component|responsive|browser|visual|screenshot|page layout)\b",
    )
});
static VERIFY: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"\b(verify|verification|check|validate|run tests?|test only|audit|review)\b")
});
static IMPLEMENT: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"\b(fix|debug|bug|patch|repair|implement|add|create|change|modify|edit|update|improve|refactor|rewrite|migrate)\b",
    )
});
static MULTI: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"\b(multi[- ]file|cross[- ]file|refactor|migration|across|all files|entire codebase)\b",
    )
});
static EXPLAIN: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"\b(explain|summarize|what does|how does|walk me through|describe)\b")
});
static DIRECT: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"\b(hello|hi|what is|which file|which directory|where is|list just|exact path|one[- ]sentence)\b",
    )
});
static NON_MUTATING: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"\b(?:read[- ]only|postmortem|retrospective|analysis|review|inspection)\s+(?:cockpit\s+)?(?:audit\s+)?only\b|\b(?:do not|don't|dont|never)\s+(?:use|call|run)\s+(?:any\s+)?tools?\b|\b(?:do not|don't|dont|never)\s+(?:edit|modify|change|write|patch|touch)\b(?:\s+[\w/-]+){0,4}\s+(?:files?|code|workspace|repository|repo)\b|\b(?:just|only)\s+(?:look|inspect|review|explain|analy[sz]e)\b[^.\n]*\b(?:do not|don't|dont)\s+(?:change|edit|modify|act)\b",
    )
});
static ANALYSIS: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"\b(?:postmortem|retrospective|analysis)\b"));
static FIX: LazyLock<Regex> = LazyLock::new(|| pattern(r"\b(fix|debug|bug|repair)\b"));
static GENERATED: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"(^|/)(generated|snapshots?|fixtures?|dist|build|coverage)(/|$)|\.snap(?:shot)?\.")
});
static CHANGE_EXTENSION: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"\.(mjs|cjs|js|jsx|ts|tsx|json|md|css|html)$"));

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Flexible,
    #[default]
    Controlled,
    StrictBenchmark,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    DirectAnswer,
    CodeExplanation,
    SmallPatch,
    DebugFix,
    MultiFileChange,
    VerificationOnly,
    Research,
    FrontendVisual,
    Ambiguous,
}
impl Intent {
    pub fn mutation_expected(self) -> bool {
        matches!(
            self,
            Self::SmallPatch | Self::DebugFix | Self::MultiFileChange | Self::FrontendVisual
        )
    }
}
pub fn classify(prompt: &str) -> Intent {
    let text = prompt.to_ascii_lowercase();
    if NON_MUTATING.is_match(&text) {
        if VERIFY.is_match(&text) {
            return Intent::VerificationOnly;
        }
        if EXPLAIN.is_match(&text) || ANALYSIS.is_match(&text) {
            return Intent::CodeExplanation;
        }
        return Intent::DirectAnswer;
    }
    if IMPLEMENT.is_match(&text) {
        if FRONTEND.is_match(&text) {
            return Intent::FrontendVisual;
        }
        if MULTI.is_match(&text) {
            return Intent::MultiFileChange;
        }
        return if FIX.is_match(&text) {
            Intent::DebugFix
        } else {
            Intent::SmallPatch
        };
    }
    for (expression, intent) in [
        (&*RESEARCH, Intent::Research),
        (&*VERIFY, Intent::VerificationOnly),
        (&*EXPLAIN, Intent::CodeExplanation),
        (&*DIRECT, Intent::DirectAnswer),
    ] {
        if expression.is_match(&text) {
            return intent;
        }
    }
    Intent::Ambiguous
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPolicy {
    pub mode: Mode,
    pub intent: Intent,
    pub mutation_expected: bool,
    pub web_relevant: bool,
    pub allowed_paths: Vec<String>,
    pub expected_changed_paths: Vec<String>,
    pub likely_tests: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification: Option<String>,
    pub max_reads_before_patch: u32,
    pub max_searches_before_patch: u32,
    pub max_patch_attempts: u32,
    pub stop_after_verifier_pass: bool,
}
fn budgets(intent: Intent, mode: Mode) -> (u32, u32, u32) {
    let strict = mode == Mode::StrictBenchmark;
    match intent {
        Intent::DirectAnswer => (2, 1, 0),
        Intent::CodeExplanation | Intent::VerificationOnly => {
            if strict {
                (3, 1, 0)
            } else {
                (5, 2, 0)
            }
        }
        Intent::MultiFileChange | Intent::FrontendVisual => {
            if strict {
                (6, 2, 2)
            } else {
                (10, 4, 4)
            }
        }
        Intent::Research => (0, if strict { 2 } else { 4 }, 0),
        _ => {
            if strict {
                (4, 1, 2)
            } else {
                (7, 3, 3)
            }
        }
    }
}
fn expected_paths(intent: Intent, files: &[&str], tests: &[&str], prompt: &str) -> Vec<String> {
    if matches!(
        intent,
        Intent::DirectAnswer
            | Intent::CodeExplanation
            | Intent::VerificationOnly
            | Intent::Research
    ) {
        return vec![];
    }
    let lower = prompt.to_lowercase();
    let terms: BTreeSet<_> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| s.len() >= 3)
        .collect();
    let candidates: Vec<_> = files
        .iter()
        .copied()
        .filter(|path| {
            !tests.contains(path)
                && !GENERATED.is_match(&path.to_ascii_lowercase())
                && CHANGE_EXTENSION.is_match(path)
        })
        .collect();
    let mut ranked: Vec<_> = candidates
        .iter()
        .copied()
        .filter_map(|path| {
            let lower = path.to_lowercase();
            let tokens: BTreeSet<_> = lower
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|s| !s.is_empty())
                .collect();
            let score: u32 = terms
                .iter()
                .map(|term| {
                    if tokens.contains(term) {
                        8
                    } else if lower.contains(term) {
                        3
                    } else {
                        0
                    }
                })
                .sum();
            (score >= 3).then_some((path, score))
        })
        .collect();
    ranked.sort_by(|(a, sa), (b, sb)| sb.cmp(sa).then_with(|| compare(a, b)));
    let paths: Vec<_> = if ranked.is_empty() {
        candidates
    } else {
        ranked.into_iter().map(|(path, _)| path).collect()
    };
    paths
        .into_iter()
        .take(if intent == Intent::MultiFileChange {
            6
        } else {
            3
        })
        .map(str::to_owned)
        .collect()
}
pub fn infer(prompt: &str, index: &Index, mode: Mode) -> Result<TaskPolicy, DomainError> {
    if prompt.len() > 4 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("task policy prompt".into()));
    }
    let intent = classify(prompt);
    let files = index.selected_files(prompt);
    let tests: Vec<_> = index
        .test_files
        .iter()
        .filter(|path| files.contains(&path.as_str()))
        .map(String::as_str)
        .collect();
    let mutation_expected = intent.mutation_expected();
    let verification = if mutation_expected || intent == Intent::VerificationOnly {
        index.verifier_with_tests(prompt, &tests)?
    } else {
        None
    };
    let (max_reads_before_patch, max_searches_before_patch, max_patch_attempts) =
        budgets(intent, mode);
    Ok(TaskPolicy {
        mode,
        intent,
        mutation_expected,
        web_relevant: intent == Intent::Research || RESEARCH.is_match(&prompt.to_ascii_lowercase()),
        expected_changed_paths: expected_paths(intent, &files, &tests, prompt),
        allowed_paths: files.into_iter().map(str::to_owned).collect(),
        likely_tests: tests.into_iter().map(str::to_owned).collect(),
        stop_after_verifier_pass: verification.is_some()
            && (mutation_expected || intent == Intent::VerificationOnly),
        verification,
        max_reads_before_patch,
        max_searches_before_patch,
        max_patch_attempts,
    })
}
impl TaskPolicy {
    pub fn validate(&self) -> Result<(), DomainError> {
        for paths in [
            &self.allowed_paths,
            &self.expected_changed_paths,
            &self.likely_tests,
        ] {
            if paths.len() > 1000 || paths.iter().any(|p| p.len() > 4096 || p.contains('\0')) {
                return Err(DomainError::LimitExceeded("task policy paths".into()));
            }
        }
        if self
            .verification
            .as_ref()
            .is_some_and(|v| v.len() > 4096 || v.contains('\0'))
        {
            return Err(DomainError::LimitExceeded("task policy verifier".into()));
        }
        Ok(())
    }
    pub fn format(&self) -> String {
        let tag = |v: serde_json::Value| v.as_str().unwrap().to_owned();
        let mut lines = vec![
            String::new(),
            "Controlled execution policy:".into(),
            format!("- mode: {}", tag(serde_json::json!(self.mode))),
            format!("- intent: {}", tag(serde_json::json!(self.intent))),
            format!(
                "- mutation expected: {}",
                if self.mutation_expected { "yes" } else { "no" }
            ),
        ];
        for (label, values) in [
            ("allowed paths", &self.allowed_paths),
            ("expected changed paths", &self.expected_changed_paths),
            ("likely tests", &self.likely_tests),
        ] {
            if !values.is_empty() {
                lines.push(format!("- {label}: {}", values.join(", ")));
            }
        }
        if let Some(v) = &self.verification {
            lines.push(format!("- verifier: {v}"));
        }
        lines.push(format!(
            "- budgets: reads={}, searches={}, patches={}",
            self.max_reads_before_patch, self.max_searches_before_patch, self.max_patch_attempts
        ));
        if self.stop_after_verifier_pass {
            lines.push("- stop after verifier pass: yes".into());
        }
        lines.join("\n")
    }
}
