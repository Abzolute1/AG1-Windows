use crate::{
    DomainError,
    tools::research::{rx, slice},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
const MAX: usize = 4 * 1024 * 1024;
fn bounded(text: &str) -> Result<(), DomainError> {
    if text.len() > MAX {
        Err(DomainError::LimitExceeded("prompt resource text".into()))
    } else {
        Ok(())
    }
}
fn push(output: &mut String, text: &str) -> Result<(), DomainError> {
    if text.len() > MAX.saturating_sub(output.len()) {
        return Err(DomainError::LimitExceeded("template expansion".into()));
    }
    output.push_str(text);
    Ok(())
}
pub fn command_args(input: &str) -> Result<Vec<String>, DomainError> {
    bounded(input)?;
    let mut args = vec![];
    let mut current = String::new();
    let mut quote = None;
    for c in input.chars() {
        if let Some(q) = quote {
            if c == q {
                quote = None
            } else {
                current.push(c)
            }
        } else if c == '\"' || c == '\'' {
            quote = Some(c)
        } else if c == ' ' || c == '\t' {
            if !current.is_empty() {
                args.push(std::mem::take(&mut current));
                if args.len() > 256 {
                    return Err(DomainError::LimitExceeded("template arguments".into()));
                }
            }
        } else {
            current.push(c)
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    if args.len() > 256 {
        return Err(DomainError::LimitExceeded("template arguments".into()));
    }
    Ok(args)
}
fn replace_callback(
    input: &str,
    pattern: &'static str,
    mut replacement: impl FnMut(&regex::Captures<'_>) -> String,
) -> Result<String, DomainError> {
    let mut output = String::new();
    let mut pos = 0;
    for captures in rx(pattern).captures_iter(input) {
        let matched = captures.get(0).unwrap();
        push(&mut output, &input[pos..matched.start()])?;
        push(&mut output, &replacement(&captures))?;
        pos = matched.end();
    }
    push(&mut output, &input[pos..])?;
    Ok(output)
}
fn replace_js(input: &str, needle: &str, replacement: &str) -> Result<String, DomainError> {
    let mut output = String::new();
    let mut pos = 0;
    for (start, _) in input.match_indices(needle) {
        push(&mut output, &input[pos..start])?;
        let mut from = 0;
        let bytes = replacement.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'$' && i + 1 < bytes.len() {
                let value = match bytes[i + 1] {
                    b'$' => Some("$"),
                    b'&' => Some(needle),
                    b'`' => Some(&input[..start]),
                    b'\'' => Some(&input[start + needle.len()..]),
                    _ => None,
                };
                if let Some(value) = value {
                    push(&mut output, &replacement[from..i])?;
                    push(&mut output, value)?;
                    i += 2;
                    from = i;
                    continue;
                }
            }
            i += 1;
        }
        push(&mut output, &replacement[from..])?;
        pos = start + needle.len();
    }
    push(&mut output, &input[pos..])?;
    Ok(output)
}
pub fn substitute(content: &str, args: &[String]) -> Result<String, DomainError> {
    bounded(content)?;
    if args.len() > 256 || args.iter().map(String::len).sum::<usize>() > MAX {
        return Err(DomainError::LimitExceeded("template arguments".into()));
    }
    let positional = replace_callback(content, r"\$([0-9]+)", |c| {
        c[1].parse::<usize>()
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| args.get(i))
            .cloned()
            .unwrap_or_default()
    })?;
    let sliced = replace_callback(&positional, r"\$\{@:([0-9]+)(?::([0-9]+))?\}", |c| {
        let start = c[1]
            .parse::<usize>()
            .unwrap_or(usize::MAX)
            .saturating_sub(1)
            .min(args.len());
        let end = c
            .get(2)
            .map(|v| {
                start
                    .saturating_add(v.as_str().parse::<usize>().unwrap_or(usize::MAX))
                    .min(args.len())
            })
            .unwrap_or(args.len());
        args[start..end].join(" ")
    })?;
    let all = args.join(" ");
    let arguments = replace_js(&sliced, "$ARGUMENTS", &all)?;
    replace_js(&arguments, "$@", &all)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub path: String,
    pub source: String,
    pub scope: String,
    pub origin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_dir: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub name: String,
    pub description: String,
    pub content: String,
    pub file_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_info: Option<SourceInfo>,
}
pub fn template(
    file_path: &str,
    content: &str,
    source_info: Option<SourceInfo>,
) -> Result<Template, DomainError> {
    let parsed = super::frontmatter::parse(content)?;
    if parsed
        .frontmatter
        .get("description")
        .is_some_and(|v| !v.is_null() && !v.is_string())
    {
        return Err(DomainError::InvalidInput(
            "prompt description must be a string".into(),
        ));
    }
    let name = file_path
        .rsplit('/')
        .next()
        .unwrap_or(file_path)
        .strip_suffix(".md")
        .unwrap_or_else(|| file_path.rsplit('/').next().unwrap_or(file_path))
        .to_string();
    let description = if let Some(description) = parsed
        .frontmatter
        .get("description")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
    {
        description.to_string()
    } else if let Some(first) = parsed.body.split('\n').find(|l| !l.trim().is_empty()) {
        format!(
            "{}{}",
            slice(first, 60),
            if crate::task::units(first) > 60 {
                "..."
            } else {
                ""
            }
        )
    } else {
        String::new()
    };
    Ok(Template {
        name,
        description,
        content: parsed.body,
        file_path: file_path.into(),
        source_info,
    })
}
pub fn expand(input: &str, templates: &[Template]) -> Result<String, DomainError> {
    bounded(input)?;
    if templates.len() > 4096 {
        return Err(DomainError::LimitExceeded("prompt templates".into()));
    }
    if !input.starts_with('/') {
        return Ok(input.into());
    }
    let (name, arguments) = input[1..].split_once(' ').unwrap_or((&input[1..], ""));
    if let Some(template) = templates.iter().find(|v| v.name == name) {
        substitute(&template.content, &command_args(arguments)?)
    } else {
        Ok(input.into())
    }
}
