use super::prompts::SourceInfo;
use crate::{
    DomainError,
    task::units,
    tools::research::{has, replace, trim},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub file_path: String,
    pub base_dir: String,
    #[serde(default)]
    pub disable_model_invocation: bool,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_info: Option<SourceInfo>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collision: Option<serde_json::Value>,
}
pub struct LoadedSkill {
    pub skill: Option<Skill>,
    pub diagnostics: Vec<Diagnostic>,
}
pub fn validate_name(name: &str) -> Option<String> {
    if name.is_empty() {
        Some("Skill name is required.".into())
    } else if units(name) > 64 {
        Some("Skill name must be at most 64 characters.".into())
    } else if !has(r"^[a-z][a-z0-9]*(-[a-z0-9]+)*$", name) {
        Some("Skill name must be lowercase alphanumeric with hyphens (no leading/trailing hyphens, no consecutive hyphens).".into())
    } else {
        None
    }
}
pub fn load(file_path: &str, content: &str, source_info: Option<SourceInfo>) -> LoadedSkill {
    let mut diagnostics = vec![];
    let mut warning = |message: String| {
        diagnostics.push(Diagnostic {
            kind: "warning".into(),
            message,
            path: file_path.into(),
            collision: None,
        })
    };
    let parsed = match super::frontmatter::parse(content) {
        Ok(value) => value,
        Err(error) => {
            warning(format!("failed to parse skill frontmatter: {error:?}"));
            return LoadedSkill {
                skill: None,
                diagnostics,
            };
        }
    };
    let dir = file_path
        .rsplit_once('/')
        .map(|(p, _)| if p.is_empty() { "/" } else { p })
        .unwrap_or(".");
    let parent = dir.rsplit('/').next().unwrap_or(dir);
    let front = &parsed.frontmatter;
    for field in ["name", "description"] {
        if let Some(value) = front.get(field)
            && !value.is_null()
            && !value.is_string()
        {
            warning(format!("skill {field} must be a string"));
            return LoadedSkill {
                skill: None,
                diagnostics,
            };
        }
    }
    let description = front
        .get("description")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if description.is_empty() || trim(description).is_empty() {
        warning("description is required".into());
    } else if units(description) > 1024 {
        warning(format!(
            "description exceeds 1024 characters ({})",
            units(description)
        ));
    }
    let name = front
        .get("name")
        .and_then(serde_json::Value::as_str)
        .filter(|v| !v.is_empty())
        .unwrap_or(parent);
    if name != parent {
        warning(format!(
            "name \"{name}\" does not match parent directory \"{parent}\""
        ));
    }
    if units(name) > 64 {
        warning(format!("name exceeds 64 characters ({})", units(name)));
    }
    if !has(r"^[a-z0-9-]+$", name) {
        warning(
            "name contains invalid characters (must be lowercase a-z, 0-9, hyphens only)".into(),
        );
    }
    if name.starts_with('-') || name.ends_with('-') {
        warning("name must not start or end with a hyphen".into());
    }
    if name.contains("--") {
        warning("name must not contain consecutive hyphens".into());
    }
    if trim(description).is_empty() {
        return LoadedSkill {
            skill: None,
            diagnostics,
        };
    }
    let triggers = front
        .get("triggers")
        .and_then(serde_json::Value::as_array)
        .map(|v| {
            v.iter()
                .filter_map(serde_json::Value::as_str)
                .map(trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    LoadedSkill {
        skill: Some(Skill {
            name: name.into(),
            description: description.into(),
            file_path: file_path.into(),
            base_dir: dir.into(),
            disable_model_invocation: front["disable-model-invocation"] == true,
            triggers,
            source_info,
        }),
        diagnostics,
    }
}
fn normalized(text: &str) -> String {
    format!(
        " {} ",
        trim(&replace(&text.to_lowercase(), r"[^a-z0-9]+", " "))
    )
}
pub fn match_prompt<'a>(text: &str, skills: &'a [Skill]) -> Result<Option<&'a Skill>, DomainError> {
    if text.len() > 1024 * 1024 || skills.len() > 4096 {
        return Err(DomainError::LimitExceeded("skill activation".into()));
    }
    if trim(text).is_empty()
        || text
            .trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
            .starts_with("<skill ")
    {
        return Ok(None);
    }
    let prompt = normalized(text);
    let mut best = None;
    let mut best_score = 0;
    for skill in skills {
        if skill.disable_model_invocation {
            continue;
        }
        if skill.triggers.len() > 256
            || skill.name.len() > 65536
            || skill.triggers.iter().any(|t| t.len() > 65536)
        {
            return Err(DomainError::LimitExceeded("skill triggers".into()));
        }
        let name = normalized(&skill.name.replace('-', " ")).trim().to_string();
        let words = name
            .split(' ')
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>();
        let mut score =
            if words.len() > 1 && words.iter().all(|w| prompt.contains(&format!(" {w} "))) {
                words.len() * 10 + 3
            } else {
                0
            };
        for candidate in std::iter::once(&name).chain(skill.triggers.iter()) {
            let norm = normalized(candidate).trim().to_string();
            if norm.is_empty() {
                continue;
            }
            let words = norm
                .split(' ')
                .filter(|w| !w.is_empty())
                .collect::<Vec<_>>();
            let phrase = prompt.contains(&format!(" {norm} "));
            let word_set =
                words.len() > 1 && words.iter().all(|w| prompt.contains(&format!(" {w} ")));
            if !phrase && !word_set {
                continue;
            }
            let candidate_score = words.len() * 10 + if candidate == &name { 5 } else { 0 };
            let candidate_score = if candidate != &name && !phrase {
                candidate_score.saturating_sub(1)
            } else {
                candidate_score
            };
            score = score.max(candidate_score);
        }
        if score > 0 && score > best_score {
            best = Some(skill);
            best_score = score;
        }
    }
    Ok(best)
}
fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn format(skills: &[Skill]) -> Result<String, DomainError> {
    if skills.len() > 4096 {
        return Err(DomainError::LimitExceeded("skill prompt count".into()));
    }
    let visible = skills
        .iter()
        .filter(|s| !s.disable_model_invocation)
        .collect::<Vec<_>>();
    if visible.is_empty() {
        return Ok(String::new());
    }
    let mut lines=vec!["\n\nThe following skills provide specialized instructions for specific tasks.".into(),"AG1 deterministically embeds a skill when its activation trigger matches the user request; an embedded skill block is already loaded and must not be read again.".into(),"If no skill was embedded but a listed skill clearly applies, use the read tool to load its file before other task tools.".into(),"When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.".into(),String::new(),"<available_skills>".into()];
    let mut bytes =
        lines.iter().map(String::len).sum::<usize>() + lines.len() + "</available_skills>".len();
    for skill in visible {
        if skill.name.len() + skill.description.len() + skill.file_path.len() > 1024 * 1024 {
            return Err(DomainError::LimitExceeded("skill prompt fields".into()));
        }
        let values = [
            "  <skill>".into(),
            format!("    <name>{}</name>", xml(&skill.name)),
            format!("    <description>{}</description>", xml(&skill.description)),
            format!("    <location>{}</location>", xml(&skill.file_path)),
            "  </skill>".into(),
        ];
        bytes += values.iter().map(String::len).sum::<usize>() + 5;
        if bytes > 4 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("skill prompt bytes".into()));
        }
        lines.extend(values);
    }
    lines.push("</available_skills>".into());
    Ok(lines.join("\n"))
}

/// A validated helper draft. Publication belongs to the capability-bound host.
pub struct Draft {
    name: String,
    content: String,
}
impl Draft {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn content(&self) -> &str {
        &self.content
    }
}
pub fn draft(name: &str, description: &str, disable: bool) -> Result<Draft, DomainError> {
    if let Some(error) = validate_name(name) {
        return Err(DomainError::InvalidInput(error));
    }
    if description.is_empty() {
        return Err(DomainError::InvalidInput("Description is required.".into()));
    }
    if units(description) > 1024 {
        return Err(DomainError::InvalidInput(
            "Description must be at most 1024 characters.".into(),
        ));
    }
    // Preserve ordinary reference bytes. Quote whenever a plain YAML scalar would
    // change the description or introduce fields through colons/newlines.
    let yaml = format!("description: {description}");
    let plain = super::frontmatter::yaml(&yaml).ok().is_some_and(|v| {
        v.as_object().is_some_and(|m| m.len() == 1) && v["description"] == description
    });
    let encoded = if plain {
        description.to_owned()
    } else {
        serde_json::to_string(description)
            .map_err(|_| DomainError::InvalidInput("skill description".into()))?
    };
    let option = if disable {
        "\ndisable-model-invocation: true"
    } else {
        ""
    };
    let content = format!(
        "---\nname: {name}\ndescription: {encoded}{option}\n---\n\n# {name}\n\n{description}\n\n## Instructions\n\n<!-- Write your skill instructions here. These are injected into the model's context when the skill is invoked. -->\n\n## Examples\n\n<!-- Add example invocations or use cases. -->\n"
    );
    Ok(Draft {
        name: name.into(),
        content,
    })
}
