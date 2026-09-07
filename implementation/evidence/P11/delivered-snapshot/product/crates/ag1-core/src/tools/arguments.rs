//! Reference-compatible legacy edit argument preparation, before schema validation and policy.
use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;
fn string<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| v[*k].as_str())
}
fn path(raw: &str) -> String {
    let absolute = raw.starts_with('/');
    let mut parts = vec![];
    for p in raw.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push(p);
                }
            }
            _ => parts.push(p),
        }
    }
    let mut out = format!("{}{}", if absolute { "/" } else { "" }, parts.join("/"));
    if out.is_empty() {
        out = ".".into();
    }
    if raw.ends_with('/') && !out.ends_with('/') {
        out.push('/');
    }
    out.replace('\\', "/")
}
fn replacement(v: &Value) -> Option<Value> {
    Some(
        json!({"oldText":string(v,&["oldText","old_text","search","find","before","old"] )?,"newText":string(v,&["newText","new_text","replace","replacement","after","new"] )?}),
    )
}
fn array(v: &Value) -> Option<Vec<Value>> {
    let parsed = if let Some(s) = v.as_str() {
        serde_json::from_str(s.trim()).ok()?
    } else {
        v.clone()
    };
    let rows = if let Some(a) = parsed.as_array() {
        a.clone()
    } else if parsed.is_object() {
        vec![parsed]
    } else {
        return None;
    };
    rows.iter().map(replacement).collect()
}
static SEARCH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<{5,}\s*SEARCH\s*\n(.*?)\n={5,}\s*\n(.*?)\n>{5,}\s*REPLACE").unwrap()
});
pub fn patch(input: &Value) -> Option<Value> {
    if !input.is_object() {
        return None;
    }
    let target = string(
        input,
        &["path", "filePath", "file_path", "filename", "file"],
    )
    .filter(|s| !s.is_empty())
    .map(path);
    if let (Some(path), Some(edit)) = (&target, replacement(input)) {
        return Some(json!({"path":path,"replacements":[edit]}));
    }
    for key in ["edits", "replacements", "changes"] {
        if let (Some(path), Some(rows)) = (&target, array(&input[key]))
            && !rows.is_empty()
        {
            return Some(json!({"path":path,"replacements":rows}));
        }
    }
    let text = string(input, &["patch", "diff"])?;
    if text.is_empty() {
        return None;
    }
    let rows: Vec<_> = SEARCH
        .captures_iter(text)
        .map(|c| json!({"oldText":c[1].to_string(),"newText":c[2].to_string()}))
        .collect();
    if let Some(path) = &target
        && !rows.is_empty()
    {
        return Some(json!({"path":path,"replacements":rows}));
    }
    let lines: Vec<_> = text
        .split('\n')
        .map(|s| s.strip_suffix('\r').unwrap_or(s))
        .collect();
    let plus = lines
        .iter()
        .find_map(|s| s.strip_prefix("+++ "))
        .map(str::trim);
    let target = if let Some(p) = plus.filter(|p| *p != "/dev/null") {
        Some(path(
            p.strip_prefix("a/")
                .or_else(|| p.strip_prefix("b/"))
                .unwrap_or(p)
                .trim(),
        ))
    } else {
        target
    }?;
    let mut old = vec![];
    let mut new = vec![];
    let mut started = false;
    let mut edits = vec![];
    fn flush(old: &mut Vec<&str>, new: &mut Vec<&str>, edits: &mut Vec<Value>) {
        let a = old.join("\n");
        let b = new.join("\n");
        if a != b && !a.is_empty() {
            edits.push(json!({"oldText":a,"newText":b}));
        }
        old.clear();
        new.clear();
    }
    for line in lines {
        if line.starts_with("@@") {
            if started {
                flush(&mut old, &mut new, &mut edits);
            }
            started = true;
            continue;
        }
        if !started || line.starts_with("\\ No newline") {
            continue;
        }
        if let Some(s) = line.strip_prefix(' ') {
            old.push(s);
            new.push(s);
        } else if line.starts_with('-') && !line.starts_with("---") {
            old.push(&line[1..]);
        } else if line.starts_with('+') && !line.starts_with("+++") {
            new.push(&line[1..]);
        }
    }
    if started {
        flush(&mut old, &mut new, &mut edits);
    }
    if edits.is_empty() {
        None
    } else {
        Some(json!({"path":target,"replacements":edits}))
    }
}
fn canonical_edit(v: &Value) -> bool {
    v.is_object() && v["oldText"].is_string() && v["newText"].is_string()
}
pub fn prepare(name: &str, input: &Value) -> Value {
    if name != "edit" || !input.is_object() {
        return input.clone();
    }
    if let Some(intent) = patch(input)
        && !input["edits"].is_array()
        && !input["oldText"].is_string()
        && !input["newText"].is_string()
    {
        return json!({"path":intent["path"],"edits":intent["replacements"]});
    }
    let mut out = input.clone();
    if let Some(text) = input["edits"].as_str() {
        let text = text.trim();
        if (text.starts_with('[') || text.starts_with('{'))
            && let Ok(value) = serde_json::from_str::<Value>(text)
        {
            if value
                .as_array()
                .is_some_and(|a| a.iter().all(canonical_edit))
            {
                out["edits"] = value;
            } else if canonical_edit(&value) {
                out["edits"] = json!([value]);
            }
        }
    }
    if input["oldText"].is_string() && input["newText"].is_string() {
        let mut rows = out["edits"].as_array().cloned().unwrap_or_default();
        rows.push(json!({"oldText":input["oldText"],"newText":input["newText"]}));
        out["edits"] = json!(rows);
        out.as_object_mut().unwrap().remove("oldText");
        out.as_object_mut().unwrap().remove("newText");
    }
    out
}
