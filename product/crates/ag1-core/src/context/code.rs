//! Bounded lexical symbols and local relationships; these are navigation hints, not AST proof.
use super::slice_utf16;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    pub line: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Relation {
    pub from: String,
    pub to: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}
fn extension(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase()
}
fn pattern(kind: &str, pattern: &str) -> (String, Regex) {
    (
        kind.into(),
        Regex::new(
            &pattern
                .replace("ID", r"([A-Za-z_$][A-Za-z0-9_$]*)")
                .replace(r"\b", r"(?-u:\b)"),
        )
        .unwrap(),
    )
}
pub fn symbols(path: &str, lines: &[String]) -> Vec<Symbol> {
    let ext = extension(path);
    let mut patterns = vec![];
    if !["py", "go", "rs"].contains(&ext.as_str()) {
        for (kind, p) in [
            ("class", r"\bclass\s+ID"),
            ("interface", r"\binterface\s+ID"),
            ("enum", r"\benum\s+ID"),
            ("type", r"\btype\s+ID\b"),
        ] {
            patterns.push(pattern(kind, p));
        }
    }
    let extra: &[(&str, &str)] = match ext.as_str() {
        "js" | "jsx" | "mjs" | "cjs" => &[
            ("function", r"\bfunction\s+ID\s*\("),
            ("binding", r"\b(?:const|let|var)\s+ID\b"),
            (
                "method",
                r"^\s*(?:public\s+|private\s+|protected\s+|static\s+|async\s+)*ID\s*\(",
            ),
        ],
        "ts" | "tsx" | "mts" | "cts" => &[
            ("function", r"\bfunction\s+ID\s*(?:<[^>]+>)?\s*\("),
            ("binding", r"\b(?:const|let|var)\s+ID\b"),
            (
                "method",
                r"^\s*(?:public\s+|private\s+|protected\s+|static\s+|readonly\s+|async\s+)*(?:get\s+|set\s+)?ID\s*(?:<[^>]+>)?\s*\(",
            ),
        ],
        "py" => &[
            ("function", r"^\s*(?:async\s+)?def\s+ID\s*\("),
            ("class", r"^\s*class\s+ID\b"),
        ],
        "go" => &[
            ("function", r"^\s*func\s+(?:\([^)]*\)\s*)?ID\s*\("),
            ("type", r"^\s*type\s+ID\b"),
        ],
        "rs" => &[
            ("function", r"\bfn\s+ID\s*(?:<[^>]+>)?\s*\("),
            ("struct", r"\bstruct\s+ID\b"),
            ("enum", r"\benum\s+ID\b"),
            ("trait", r"\btrait\s+ID\b"),
            ("module", r"\bmod\s+ID\b"),
        ],
        _ => &[],
    };
    for (kind, p) in extra {
        patterns.push(pattern(kind, p));
    }
    let mut out = vec![];
    let mut seen = BTreeSet::new();
    for (i, line) in lines.iter().enumerate() {
        if out.len() >= 200 {
            break;
        }
        let trim = line.trim_start();
        if ["//", "#", "/*", "*"].iter().any(|s| trim.starts_with(s)) {
            continue;
        }
        for (kind, p) in &patterns {
            if let Some(c) = p.captures(line) {
                let name = &c[1];
                if kind == "method"
                    && ["catch", "for", "if", "switch", "while", "with"].contains(&name)
                {
                    continue;
                }
                if seen.insert((kind.clone(), name.to_owned(), i)) {
                    out.push(Symbol {
                        name: slice_utf16(name, 200),
                        kind: kind.clone(),
                        line: i + 1,
                    });
                }
            }
        }
    }
    out
}
pub struct CodeFile<'a> {
    pub path: &'a str,
    pub lines: &'a [String],
    pub symbols: &'a [Symbol],
}
fn resolve(from: &str, spec: &str, known: &BTreeSet<&str>) -> Option<String> {
    if !spec.starts_with('.') {
        return None;
    }
    let parent = from.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
    let joined = format!("{parent}/{spec}");
    let mut parts = vec![];
    for p in joined.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(p),
        }
    }
    if parts.is_empty() {
        return None;
    }
    let base = parts.join("/");
    let extensions = [
        "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "py", "go", "rs",
    ];
    let mut candidates = vec![base.clone()];
    if Path::new(&base).extension().is_some() {
        let stem = base.rsplit_once('.').unwrap().0;
        for ext in extensions {
            candidates.push(format!("{stem}.{ext}"));
        }
    } else {
        for ext in extensions {
            candidates.push(format!("{base}.{ext}"));
        }
        for ext in extensions {
            candidates.push(format!("{base}/index.{ext}"));
        }
        candidates.push(format!("{base}/__init__.py"));
    }
    candidates.into_iter().find(|p| known.contains(p.as_str()))
}
pub fn relations(files: &[CodeFile<'_>]) -> Vec<Relation> {
    let known: BTreeSet<_> = files.iter().map(|f| f.path).collect();
    let mut out = vec![];
    let mut seen = BTreeSet::new();
    let mut add = |from: &str, to: &str, kind: &str, label: Option<String>| {
        let key = (
            from.to_owned(),
            to.to_owned(),
            kind.to_owned(),
            label.clone(),
        );
        if out.len() < 200 && seen.insert(key) {
            out.push(Relation {
                from: from.into(),
                to: to.into(),
                kind: kind.into(),
                label,
            });
        }
    };
    let js: Vec<_> = [
        r#"\b(?:import|export)\b[^"']*\bfrom\s*["']([^"']+)["']"#,
        r#"\bimport\s*["']([^"']+)["']"#,
        r#"\brequire\s*\(\s*["']([^"']+)["']\s*\)"#,
        r#"\bimport\s*\(\s*["']([^"']+)["']\s*\)"#,
    ]
    .into_iter()
    .map(|p| Regex::new(p).unwrap())
    .collect();
    let python = Regex::new(r"^\s*from\s+(\.+)([A-Za-z0-9_.]*)\s+import\s+").unwrap();
    let rust = Regex::new(r"^\s*(?:pub\s+)?mod\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*;").unwrap();
    for f in files {
        let ext = extension(f.path);
        for line in f.lines {
            let mut imports = vec![];
            if ["js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts"].contains(&ext.as_str()) {
                for p in &js {
                    for c in p.captures_iter(line) {
                        imports.push((c[1].into(), None));
                    }
                }
            } else if ext == "py" {
                if let Some(c) = python.captures(line) {
                    imports.push((
                        format!(
                            "{}{}",
                            if c[1].len() == 1 {
                                "./".into()
                            } else {
                                "../".repeat(c[1].len() - 1)
                            },
                            c[2].replace('.', "/")
                        ),
                        None,
                    ));
                }
            } else if ext == "rs"
                && let Some(c) = rust.captures(line)
            {
                imports.push((format!("./{}", &c[1]), Some("mod".into())));
            }
            for (spec, label) in imports {
                if let Some(target) = resolve(f.path, &spec, &known)
                    && target != f.path
                {
                    add(f.path, &target, "imports", label);
                }
            }
        }
    }
    for target in files {
        for s in target.symbols {
            if s.name.len() < 3 {
                continue;
            }
            let identifier =
                Regex::new(&format!(r"(?-u:\b){}(?-u:\b)", regex::escape(&s.name))).unwrap();
            for source in files {
                if source.path != target.path && source.lines.iter().any(|l| identifier.is_match(l))
                {
                    add(source.path, target.path, "references", Some(s.name.clone()));
                }
            }
        }
    }
    out
}
