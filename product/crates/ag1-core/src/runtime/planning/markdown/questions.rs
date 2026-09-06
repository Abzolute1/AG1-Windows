use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum QuestionItem {
    Bullet,
    Numbered,
    Step,
}
struct Item {
    head: String,
    continuations: Vec<String>,
}
pub(super) fn questions(text: &str, modes: &[QuestionItem]) -> Vec<Question> {
    let start = rx(r"(?i)^(\s*)(?:(\d+[.)])|([-*])|(step\s+\d+\s*[:.)-]))\s+(.+)$");
    let mut items = Vec::new();
    let mut current: Option<Item> = None;
    let mut indent = 0;
    for line in text.lines() {
        let line = line.replace('\t', "    ");
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(c) = start.captures(&line) {
            let kind = if c.get(2).is_some() {
                QuestionItem::Numbered
            } else if c.get(3).is_some() {
                QuestionItem::Bullet
            } else {
                QuestionItem::Step
            };
            if modes.contains(&kind) {
                if let Some(previous) = current.take() {
                    items.push(previous);
                }
                indent = c[1].chars().count();
                current = Some(Item {
                    head: c[5].trim().into(),
                    continuations: vec![],
                });
                continue;
            }
        }
        if let Some(current) = &mut current
            && (line.chars().take_while(|c| c.is_whitespace()).count() > indent
                || trimmed.starts_with(['(', '[', '{'])
                || rx(r"^[-*]\s+").is_match(trimmed)
                || rx(r"(?i)^or\b").is_match(trimmed))
        {
            current
                .continuations
                .push(rx(r"^[-*]\s+").replace(trimmed, "").into());
        }
    }
    if let Some(current) = current {
        items.push(current);
    }
    items
        .into_iter()
        .filter_map(|item| {
            let head = clean(&item.head);
            if head.is_empty() {
                return None;
            }
            let continuations = item
                .continuations
                .iter()
                .map(|s| clean(s))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            let question = prompt(&head, &continuations)?;
            let options = options(&head, &continuations, &question);
            Some(Question { question, options })
        })
        .collect()
}
fn likely(value: &str) -> bool {
    rx(r"(?i)^(target|what|which|who|where|when|why|how|do|does|did|is|are|can|could|should|would|will|any|interface|type|platform|feature|features|key features|scope|language|runtime|framework|library|display|terminal|desktop|web|mobile|game loop|input|grid|speed|specific requirements|requirements|constraints)\b").is_match(value.trim())
}
fn standalone(value: &str) -> bool {
    let value = value.trim();
    value.is_empty() || (!value.contains(['?',':']) && !rx(r"(?i)^(target|what|which|who|where|when|why|how|do|does|did|is|are|can|could|should|would|will|any|interface|type|platform|feature|features|key features|scope|language|runtime|framework|library|display|specific requirements|requirements|constraints)\b").is_match(value) && value.split_whitespace().count()<=6)
}
fn delimited(text: &str) -> Vec<(usize, usize)> {
    let mut stack = Vec::new();
    let mut sections = Vec::new();
    for (i, c) in text.char_indices() {
        if ['(', '['].contains(&c) {
            stack.push((c, i));
            continue;
        }
        if [')', ']'].contains(&c)
            && let Some(&(open, start)) = stack.last()
            && matches!((open, c), ('(', ')') | ('[', ']'))
        {
            stack.pop();
            if stack.is_empty() {
                sections.push((start, i));
            }
        }
    }
    sections
}
fn fragments(text: &str) -> Vec<String> {
    rx(r"\?\s+")
        .split(text)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
fn prompt(head: &str, continuations: &[String]) -> Option<String> {
    let mut prompt = head.trim().to_owned();
    if let Some(segment) = rx(r"\s+-\s+")
        .split(&prompt)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .find(|s| s.ends_with('?') || likely(s))
        .map(str::to_owned)
    {
        prompt = segment;
    }
    let parts = fragments(&prompt);
    if parts.len() >= 2 {
        prompt = format!("{}?", parts[0].trim_end_matches('?').trim());
    }
    if let Some((before, after)) = prompt.split_once(':')
        && !before.trim().is_empty()
        && (!after.trim().is_empty() || !continuations.is_empty())
    {
        prompt = before.trim().into();
    }
    if let Some((start, end)) = delimited(&prompt)
        .into_iter()
        .rev()
        .find(|&(_, end)| matches!(prompt[end + 1..].trim(), "" | "?"))
        && start > 0
    {
        let suffix = if prompt[end + 1..].trim() == "?" {
            "?"
        } else {
            ""
        };
        prompt = format!("{}{suffix}", prompt[..start].trim());
    }
    prompt = norm(
        prompt
            .trim_end_matches(|c: char| c == ':' || c.is_whitespace())
            .trim(),
    );
    if prompt.is_empty() || standalone(&prompt) {
        return None;
    }
    if !prompt.ends_with('?') && likely(&prompt) {
        prompt.push('?');
    }
    Some(prompt)
}
fn split_choices(text: &str) -> Vec<String> {
    let text = rx(r"\?+\s*$").replace(text, "").trim().to_owned();
    if text.is_empty() {
        return vec![];
    }
    if !rx(r"(?i)[;,]|\bor\b").is_match(&text) {
        let parts = fragments(&text);
        if parts.len() >= 2 {
            return parts;
        }
    }
    let mut options = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut position = 0;
    while position < text.len() {
        let remaining = &text[position..];
        let c = remaining.chars().next().unwrap();
        position += c.len_utf8();
        if ['(', '['].contains(&c) {
            depth += 1;
            current.push(c);
            continue;
        }
        if [')', ']'].contains(&c) {
            depth = depth.saturating_sub(1);
            current.push(c);
            continue;
        }
        if depth == 0 && [',', ';'].contains(&c) {
            if !current.trim().is_empty() {
                options.push(current.trim().into());
            }
            current.clear();
            continue;
        }
        if depth == 0
            && remaining
                .get(..4)
                .is_some_and(|s| s.eq_ignore_ascii_case(" or "))
        {
            if !current.trim().is_empty() {
                options.push(current.trim().into());
            }
            current.clear();
            position += 3;
            continue;
        }
        current.push(c);
    }
    if !current.trim().is_empty() {
        options.push(current.trim().into());
    }
    options
}
fn low_quality(option: &str, prompt: &str) -> bool {
    let normalized = option.trim_end_matches(['?', ':']).trim().to_lowercase();
    let prompt = prompt.trim_end_matches(['?', ':']).trim().to_lowercase();
    normalized.is_empty() || normalized==prompt
        || rx(r"^(?:/|\.\.?/|~/)").is_match(option.trim())
        || rx(r"(?i)(?:^|\s)(?:/home/|/tmp/|/var/|/etc/)").is_match(option)
        || rx(r"(?i)^(?:e\.g\.|eg\.?|example:?)\s+").is_match(option)
        || rx(r":\s*$").is_match(option)
        || rx(r"(?i)^(for example|example|for instance|details|more details|more info|other|another)$").is_match(&normalized)
}
fn options(head: &str, continuations: &[String], prompt: &str) -> Vec<String> {
    if rx(r"^(any )?(specific )?(requirements|constraints|notes|preferences|trade-?offs)$")
        .is_match(&prompt.trim_end_matches(['?', ':']).trim().to_lowercase())
    {
        return vec![];
    }
    let mut options = Vec::new();
    let push = |options: &mut Vec<String>, values: Vec<String>| {
        for value in values {
            let value = clean(&value);
            let value = if (value.starts_with('(') && value.ends_with(')'))
                || (value.starts_with('[') && value.ends_with(']'))
            {
                value[1..value.len() - 1].trim()
            } else {
                value.trim()
            };
            let value = rx(r"(?i)\betc\.?$")
                .replace(
                    &rx(r"(?i)^and\s+").replace(&rx(r"(?i)^or\s+").replace(value, ""), ""),
                    "",
                )
                .trim()
                .to_owned();
            if value.is_empty() {
                continue;
            }
            options.push(if standalone(&value) || value.ends_with('?') {
                value.trim_end_matches('?').trim().into()
            } else {
                value
            });
        }
    };
    let colon = head.find(':');
    let parts = fragments(head);
    let chain = parts.len() >= 3 || (parts.len() >= 2 && colon.is_some());
    if chain {
        if colon.is_some()
            && let Some((_, first)) = parts[0].split_once(':')
            && !first.trim().is_empty()
        {
            push(&mut options, vec![first.trim().into()]);
        }
        push(&mut options, parts.into_iter().skip(1).collect());
    }
    if let Some(colon) = colon
        && !chain
    {
        push(&mut options, split_choices(head[colon + 1..].trim()));
    }
    if options.is_empty() {
        for (start, end) in delimited(head) {
            push(&mut options, split_choices(&head[start + 1..end]));
        }
    }
    if options.len() < 2 {
        push(
            &mut options,
            continuations
                .iter()
                .filter(|s| !rx(r"(?i)^e\.g\.").is_match(s.trim()))
                .cloned()
                .collect(),
        );
    }
    let mut seen = std::collections::BTreeSet::new();
    options
        .into_iter()
        .filter(|s| !low_quality(s, prompt) && seen.insert(s.clone()))
        .take(8)
        .collect()
}
