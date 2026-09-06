//! Compatibility extraction for models that do not submit a structured plan.
//! The result is still a draft/interview; parsing never grants execution.
use super::*;
use regex::Regex;
mod questions;
use questions::{QuestionItem, questions};

fn rx(pattern: &str) -> Regex {
    Regex::new(pattern).expect("constant planning expression")
}
fn norm(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn clean(value: &str) -> String {
    rx(r"`([^`]+)`")
        .replace_all(&rx(r"\*{1,2}([^*]+)\*{1,2}").replace_all(value, "$1"), "$1")
        .trim()
        .into()
}
fn normalized_goal(value: &str) -> String {
    norm(&rx(r"\bplanningmode\b").replace_all(
        &rx(r"\bplanmode\b").replace_all(&value.trim().to_lowercase(), "plan mode"),
        "planning mode",
    ))
}
fn subject(value: &str) -> String {
    let value = normalized_goal(value);
    for pattern in [
        r"\b(?:enter|open|start|use|switch to|go into)\s+(?:planning|plan)\s+mode\s+(?:for|to plan)\s+(.+)$",
        r"\b(?:let(?:'s| us)?\s+)?plan(?:\s+out)?\s+(.+)$",
        r"\b(?:help me\s+)?plan(?:\s+out)?\s+(.+)$",
        r"\b(?:build|create|make|implement|design|scaffold|start)\s+(.+)$",
    ] {
        if let Some(c) = rx(pattern).captures(&value) {
            return c[1].trim().into();
        }
    }
    value
}
fn generic(goal: &str) -> bool {
    let text = normalized_goal(goal);
    let subject = subject(goal);
    !text.is_empty() && !rx(r"\b(codebase|repo|repository|workspace|session|current implementation|existing implementation|current code|file|files|path|module|function|class|test|tests|refactor|bug|regression|migration|runtime|bootstrap|tsconfig|package\.json)\b").is_match(&subject)
        && rx(r"\b(plan|planning|build|create|make|implement|design|scaffold|start|enter|open|switch)\b").is_match(&text)
        && rx(r"\b(game|app|website|site|tool|api|cli|dashboard|bot|agent|project|snake)\b").is_match(&subject)
}
pub fn should_interview(goal: &str) -> bool {
    let subject = subject(goal);
    generic(goal) && (!rx(r"\b(python|typescript|javascript|js|ts|c\+\+|c#|java|kotlin|swift|rust|go|golang|ruby|php|html|css|react|vue|svelte|angular|pygame|tkinter|godot|unity)\b").is_match(&subject)
        || !rx(r"\b(web|browser|terminal|cli|desktop|mobile|ios|android|canvas|electron|react native|pygame|tkinter|unity|godot)\b").is_match(&subject))
}
pub fn allow_inspection(goal: &str, previous: bool) -> bool {
    if previous {
        return false;
    }
    let text = goal.to_lowercase();
    !text.trim().is_empty()
        && [
            "codebase",
            "repo",
            "repository",
            "workspace",
            "session",
            "current implementation",
            "existing implementation",
            "current code",
            "file",
            "files",
            "path",
            "module",
            "function",
            "class",
            "test",
            "tests",
            "refactor",
            "bug",
            "regression",
            "migration",
            "runtime",
            "bootstrap",
            "cli",
            "tui",
            "provider",
            "artifact",
            "tsconfig",
            "package.json",
            ".ts",
            ".js",
            ".tsx",
            ".jsx",
        ]
        .iter()
        .any(|s| text.contains(s))
}
fn fallback_questions(goal: &str) -> Vec<Question> {
    let values = if rx(r"\bgame\b").is_match(&subject(goal)) {
        vec![
            (
                "Target language?",
                vec!["Python", "JavaScript/TypeScript", "C++", "Rust", "Other"],
            ),
            (
                "Target interface/platform?",
                vec!["Terminal", "Desktop GUI", "Web browser", "Mobile"],
            ),
            (
                "Feature scope?",
                vec![
                    "Basic gameplay only",
                    "Score + restart",
                    "Polished visuals",
                    "Extended mechanics",
                ],
            ),
        ]
    } else {
        vec![
            ("Target language or runtime?", vec![]),
            ("Target platform or interface?", vec![]),
            ("Scope, constraints, or must-have features?", vec![]),
        ]
    };
    values
        .into_iter()
        .map(|(q, options)| Question {
            question: q.into(),
            options: options.into_iter().map(str::to_owned).collect(),
        })
        .collect()
}
fn category(prompt: &str) -> Option<&'static str> {
    let text = prompt.trim_end_matches(['?', ':']).trim().to_lowercase();
    for (kind, pattern) in [
        (
            "language",
            r"\b(language|runtime|stack|framework|library)\b",
        ),
        (
            "platform",
            r"\b(platform|interface|ui|display|terminal|desktop|browser|web|mobile)\b",
        ),
        ("scope", r"\b(feature|features|scope|mechanics|gameplay)\b"),
    ] {
        if rx(pattern).is_match(&text) {
            return Some(kind);
        }
    }
    None
}
fn boolean(prompt: &str) -> bool {
    rx(r"(?i)^(would|do|does|did|is|are|can|could|should|will)\b")
        .is_match(prompt.trim_end_matches(['?', ':']).trim())
}
fn add_options(goal: &str, mut questions: Vec<Question>) -> Vec<Question> {
    let fallback = fallback_questions(goal);
    for q in &mut questions {
        if q.options.len() >= 2 {
            continue;
        }
        if boolean(&q.question) {
            q.options = vec!["Yes".into(), "No".into()];
            continue;
        }
        if let Some(category) = category(&q.question)
            && let Some(f) = fallback
                .iter()
                .find(|f| f.options.len() >= 2 && self::category(&f.question) == Some(category))
        {
            q.options = f.options.clone();
        }
    }
    questions
}
fn section<'a>(text: &'a str, names: &[&str]) -> Option<&'a str> {
    let names = names
        .iter()
        .map(|s| regex::escape(s))
        .collect::<Vec<_>>()
        .join("|");
    let start = rx(&format!(
        r"(?i)(?:^|\n)(?:#{{1,6}}\s*)?(?:{names})\s*:?[ \t]*\n"
    ))
    .find(text)?;
    let body = &text[start.end()..];
    let end=rx(r"(?i)\n(?:#{1,6}\s*)?(?:plan|steps|assumptions|constraints|risks|verification|questions|open questions|clarifying questions)\s*:?[ \t]*\n").find(body).map_or(body.len(),|m|m.start());
    let value = body[..end].trim();
    (!value.is_empty()).then_some(value)
}
fn items(text: &str, pattern: &str) -> Vec<String> {
    rx(pattern)
        .captures_iter(text)
        .map(|c| clean(&c[1]))
        .filter(|s| !s.is_empty())
        .collect()
}
const NUMBERED: &str = r"(?m)^\s*\d+[.)]\s+(.+)$";
const STEP: &str = r"(?im)^\s*step\s+\d+\s*[:.)-]\s+(.+)$";
const BULLET: &str = r"(?m)^\s*[-*]\s+(.+)$";
fn question_like(value: &str) -> bool {
    value.contains('?') || rx(r"(?i)^(what|which|who|where|when|why|how|do|does|did|is|are|can|could|should|would|will|any|target|interface|platform|feature|features|scope|language|runtime|framework|library)\b").is_match(value.trim())
        || rx(r":\s*.+(?:\?|,|\bor\b)").is_match(value)
}
impl Plan {
    pub fn fallback(
        plan_id: &str,
        goal: &str,
        previous: Option<&Plan>,
        feedback: Option<&str>,
        raw: &str,
        timestamp: &str,
    ) -> Result<Option<Self>, DomainError> {
        if previous.is_some() || !should_interview(goal) {
            return Ok(None);
        }
        let submission=Submission{summary:Some("A few core implementation choices need to be fixed before a concrete plan can be drafted.".into()),open_questions:fallback_questions(goal),..Default::default()};
        let mut plan =
            Self::from_submission(plan_id, goal, &submission, None, feedback, timestamp)?
                .ok_or(DomainError::Conflict)?;
        plan.raw_plan = raw.trim().into();
        plan.validate()?;
        Ok(Some(plan))
    }
    pub fn from_markdown(
        plan_id: &str,
        goal: &str,
        text: &str,
        previous: Option<&Plan>,
        feedback: Option<&str>,
        timestamp: &str,
    ) -> Result<Option<Self>, DomainError> {
        bounded_json(&Value::String(text.into()))?;
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let question_section = section(
            text,
            &["questions", "open questions", "clarifying questions"],
        );
        let mut open = questions(
            question_section.unwrap_or(""),
            &[QuestionItem::Bullet, QuestionItem::Numbered],
        );
        if open.is_empty() && question_section.is_some() {
            open = fallback_questions(goal);
        }
        open = add_options(goal, open);
        let clarifying_heading=rx(r"(?im)^(?:#{1,6}\s*)?(clarifying questions|questions|open questions)\b").is_match(text)
            || rx(r"(?im)^(?:before i propose a plan|before i can propose a plan|i need (?:a few )?answers|clarifying questions?|questions?)\b").is_match(text);
        let step_section = section(text, &["plan", "steps", "execution plan"]);
        let mut steps = if let Some(s) = step_section {
            [NUMBERED, STEP, BULLET]
                .into_iter()
                .map(|p| items(s, p))
                .find(|v| !v.is_empty())
                .unwrap_or_default()
        } else if clarifying_heading {
            items(text, STEP)
        } else {
            items(text, NUMBERED)
                .into_iter()
                .chain(items(text, STEP))
                .collect()
        };
        let assumptions = items(section(text, &["assumptions"]).unwrap_or(""), BULLET);
        let constraints = items(section(text, &["constraints"]).unwrap_or(""), BULLET);
        let risks = items(section(text, &["risks", "risk"]).unwrap_or(""), BULLET);
        let verification = items(
            section(text, &["verification", "verify", "validation"]).unwrap_or(""),
            BULLET,
        );
        let summary=section(text,&["summary"]).map(norm).or_else(|| {
            let boundary=rx(r"(?i)\n(?:(?:#{1,6}\s*)?(?:summary|plan|steps|assumptions|constraints|risks|verification|questions|open questions|clarifying questions)\b[:\s]*|\s*\d+[.)]\s+)");
            let prefix=text[..boundary.find(text).map_or(text.len(),|m|m.start())].trim();
            (!prefix.is_empty()).then(||norm(prefix))
        });
        let reclassify=open.is_empty() && !steps.is_empty() && steps.iter().all(|s|question_like(s))
            && [&assumptions,&constraints,&risks,&verification].iter().all(|v|v.is_empty())
            && rx(r"(?i)clarif|need (?:a few )?clarif|need (?:a few )?answers|need to know|before i can").is_match(text);
        if reclassify {
            open = questions(
                step_section.unwrap_or(text),
                &[QuestionItem::Numbered, QuestionItem::Step],
            );
            if open.is_empty() {
                open = fallback_questions(goal);
            }
            open = add_options(goal, open);
            steps.clear();
        }
        let force = !steps.is_empty()
            && !open.is_empty()
            && previous.is_none()
            && generic(goal)
            && open
                .iter()
                .any(|q| matches!(category(&q.question), Some("language" | "platform")));
        if force {
            steps.clear();
        }
        let plan_signal = steps.len() >= 2
            || rx(r"(?i)headings?:").is_match(text)
            || rx(r"(?im)^(?:#{1,6}\s*)?(plan|steps|execution plan)\b").is_match(text);
        let question_signal = !open.is_empty() && (clarifying_heading || reclassify || force);
        if (!plan_signal || steps.is_empty()) && !question_signal {
            return Ok(None);
        }
        let submission = Submission {
            summary,
            steps: steps.clone(),
            open_questions: open,
            assumptions: assumptions.clone(),
            constraints: constraints.clone(),
            risks: risks.clone(),
            verification: verification.clone(),
        };
        let mut plan =
            Self::from_submission(plan_id, goal, &submission, previous, feedback, timestamp)?
                .ok_or(DomainError::Conflict)?;
        plan.steps = steps;
        plan.assumptions = assumptions;
        plan.constraints = constraints;
        plan.risks = risks;
        plan.verification = verification;
        plan.raw_plan = text.into();
        plan.validate()?;
        Ok(Some(plan))
    }
}
