//! Repository verifier selection from a host-captured bounded workspace index.
//! Commands are guidance; neither filenames nor scripts grant execution authority.
use crate::{DomainError, tools::list::compare};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};

pub const MARKERS: &[&str] = &[
    ".defects4j.config",
    "package.json",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lockb",
    "deno.json",
    "deno.jsonc",
    "pyproject.toml",
    "requirements.txt",
    "Cargo.toml",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "gradlew",
    "mvnw",
];
pub const SOURCE_EXTENSIONS: &[&str] = &["js", "jsx", "ts", "tsx", "mjs", "cjs"];
static TEST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(^|/)(test|tests|__tests__)/.+\.[cm]?[jt]sx?$|\.(test|spec)\.[cm]?[jt]sx?$|(^|/)[^/]+[-_.]test\.[cm]?[jt]sx?$").unwrap()
});
static IMPORTS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r#"(?-u:\b)import\s+(?:[^'"]+\s+from\s+)?["']([^"']+)["']"#,
        r#"(?-u:\b)export\s+[^'"]+\s+from\s+["']([^"']+)["']"#,
        r#"(?-u:\b)require\(\s*["']([^"']+)["']\s*\)"#,
        r#"(?-u:\b)import\(\s*["']([^"']+)["']\s*\)"#,
    ]
    .into_iter()
    .map(|pattern| Regex::new(pattern).unwrap())
    .collect()
});
static EXPLICIT: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)intended verification command is:\s*`?([^`\n]+)`?",
        r"(?i)verification command is:\s*`?([^`\n]+)`?",
        r"(?i)run the targeted verification command:\s*`?([^`\n]+)`?",
        r"(?i)run\s+`([^`]+)`\s+before",
    ]
    .into_iter()
    .map(|pattern| Regex::new(pattern).unwrap())
    .collect()
});
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub specifier: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Index {
    pub files: Vec<String>,
    pub test_files: Vec<String>,
    pub package_scripts: BTreeMap<String, String>,
    pub import_edges: Vec<Edge>,
}
pub fn is_test(path: &str) -> bool {
    TEST.is_match(path)
}
fn node_test(path: &str) -> bool {
    [".js", ".mjs", ".cjs"]
        .iter()
        .any(|suffix| path.ends_with(suffix))
}
fn shell_path(path: &str) -> String {
    let path = if path.starts_with('-') {
        format!("./{path}")
    } else {
        path.to_owned()
    };
    if path
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"_./-".contains(&byte))
    {
        path
    } else {
        format!("'{}'", path.replace('\'', "'\"'\"'"))
    }
}
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r' | ' ' | '\u{00A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}
pub(super) fn trim(text: &str) -> &str {
    text.trim_matches(js_space)
}
fn explicit(prompt: &str) -> Option<String> {
    EXPLICIT
        .iter()
        .find_map(|pattern| {
            pattern.captures(prompt).map(|found| {
                let value = trim(found.get(1).unwrap().as_str());
                value.strip_suffix('.').unwrap_or(value).to_owned()
            })
        })
        .filter(|command| !command.is_empty())
}
fn import_path(from: &str, specifier: &str, files: &BTreeSet<&str>) -> Option<String> {
    if !specifier.starts_with('.') {
        return None;
    }
    let directory = from.rsplit_once('/').map_or("", |(dir, _)| dir);
    let joined = format!("{directory}/{specifier}");
    let mut parts = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    let base = parts.join("/");
    let candidates = std::iter::once(base.clone())
        .chain(
            [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"]
                .into_iter()
                .map(|ext| format!("{base}{ext}")),
        )
        .chain(
            [
                "index.ts",
                "index.tsx",
                "index.js",
                "index.jsx",
                "index.mjs",
                "index.cjs",
            ]
            .into_iter()
            .map(|file| format!("{base}/{file}")),
        );
    candidates
        .into_iter()
        .find(|path| files.contains(path.as_str()))
}
impl Index {
    pub fn captured(
        mut files: Vec<String>,
        package_scripts: BTreeMap<String, String>,
        sources: &BTreeMap<String, String>,
    ) -> Result<Self, DomainError> {
        if files.len() > 1000
            || files
                .iter()
                .any(|path| path.len() > 4096 || path.contains('\0'))
            || sources.values().map(String::len).sum::<usize>() > 16 * 1024 * 1024
            || package_scripts.len() > 4096
            || package_scripts
                .iter()
                .map(|(k, v)| k.len() + v.len())
                .sum::<usize>()
                > 1024 * 1024
        {
            return Err(DomainError::LimitExceeded("verifier index".into()));
        }
        files.sort_by(|a, b| compare(a, b));
        let names: BTreeSet<&str> = files.iter().map(String::as_str).collect();
        let mut import_edges = Vec::new();
        for file in &files {
            let Some(source) = sources.get(file) else {
                continue;
            };
            let mut seen = BTreeSet::new();
            for pattern in IMPORTS.iter() {
                for found in pattern.captures_iter(source) {
                    let specifier = found.get(1).unwrap().as_str();
                    if seen.insert(specifier)
                        && let Some(to) = import_path(file, specifier, &names)
                    {
                        if import_edges.len() == 8192 {
                            return Err(DomainError::LimitExceeded("verifier import edges".into()));
                        }
                        import_edges.push(Edge {
                            from: file.clone(),
                            to,
                            specifier: specifier.into(),
                        });
                    }
                }
            }
        }
        import_edges.sort_by(|a, b| {
            compare(
                &format!("{}:{}", a.from, a.to),
                &format!("{}:{}", b.from, b.to),
            )
        });
        let test_files = files.iter().filter(|path| is_test(path)).cloned().collect();
        Ok(Self {
            files,
            test_files,
            package_scripts,
            import_edges,
        })
    }
    fn marker(&self, name: &str, command: &str) -> Option<String> {
        let path = self
            .files
            .iter()
            .filter(|path| path.rsplit('/').next() == Some(name))
            .min_by(|a, b| {
                a.split('/')
                    .count()
                    .cmp(&b.split('/').count())
                    .then_with(|| compare(a, b))
            })?;
        Some(match path.rsplit_once('/') {
            Some((dir, _)) => format!("cd '{}' && {command}", dir.replace('\'', "'\"'\"'")),
            None => command.into(),
        })
    }
    fn likely(&self) -> Option<String> {
        if let Some(command) = self.marker(".defects4j.config", "defects4j test") {
            return Some(command);
        }
        for (name, command) in [("test", "npm test"), ("check", "npm run check")] {
            if self
                .package_scripts
                .get(name)
                .is_some_and(|script| !script.is_empty())
            {
                return Some(command.into());
            }
        }
        for (name, command) in [
            ("mvnw", "./mvnw test"),
            ("pom.xml", "mvn test"),
            ("gradlew", "./gradlew test"),
            ("build.gradle", "gradle test"),
            ("build.gradle.kts", "gradle test"),
            ("Cargo.toml", "cargo test"),
            ("go.mod", "go test ./..."),
            ("pyproject.toml", "pytest"),
            ("requirements.txt", "pytest"),
        ] {
            if let Some(command) = self.marker(name, command) {
                return Some(command);
            }
        }
        self.test_files
            .iter()
            .any(|path| node_test(path))
            .then(|| "node --test".into())
    }
    pub(super) fn selected_files(&self, prompt: &str) -> Vec<&str> {
        let lower = prompt.to_lowercase();
        let terms: BTreeSet<&str> = lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|term| term.len() >= 3)
            .collect();
        let mut scored: Vec<_> = self
            .files
            .iter()
            .filter_map(|path| {
                let lower = path.to_lowercase();
                let tokens: BTreeSet<&str> = lower
                    .split(|c: char| !c.is_ascii_alphanumeric())
                    .filter(|s| !s.is_empty())
                    .collect();
                let mut score: i32 = terms
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
                if is_test(path) {
                    score += 2;
                }
                let parts: Vec<&str> = lower.split('/').collect();
                if parts[..parts.len() - 1].contains(&"src") {
                    score += 1;
                }
                if parts[..parts.len() - 1]
                    .iter()
                    .any(|part| ["generated", "snapshot", "fixture", "fixtures"].contains(part))
                {
                    score -= 4;
                }
                (score > 0).then_some((path.as_str(), score))
            })
            .collect();
        scored.sort_by(|(a, sa), (b, sb)| sb.cmp(sa).then_with(|| compare(a, b)));
        let mut selected: BTreeSet<&str> = if scored.is_empty() {
            self.files.iter().take(24).map(String::as_str).collect()
        } else {
            scored.iter().take(24).map(|(path, _)| *path).collect()
        };
        loop {
            let before = selected.len();
            for edge in &self.import_edges {
                if selected.contains(edge.from.as_str()) {
                    selected.insert(&edge.to);
                }
                if selected.contains(edge.to.as_str()) && is_test(&edge.from) {
                    selected.insert(&edge.from);
                }
            }
            if selected.len() == before {
                break;
            }
        }
        // JavaScript's default sort orders UTF-16 code units at this step.
        let mut selected: Vec<&str> = selected.into_iter().collect();
        selected.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        selected.truncate(24);
        selected
    }
    pub fn verifier(&self, prompt: &str) -> Result<Option<String>, DomainError> {
        if prompt.len() > 4 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("verifier prompt".into()));
        }
        let selected = self.selected_files(prompt);
        let tests: Vec<_> = self
            .test_files
            .iter()
            .filter(|path| selected.contains(&path.as_str()))
            .map(String::as_str)
            .collect();
        self.verifier_with_tests(prompt, &tests)
    }
    pub(super) fn verifier_with_tests(
        &self,
        prompt: &str,
        selected_tests: &[&str],
    ) -> Result<Option<String>, DomainError> {
        let command = explicit(prompt).or_else(|| {
            self.likely().map(|command| {
                // These commands are constructed here, not free-form shell
                // text. Collapsing whitespace would corrupt quoted directories.
                let normalized = command;
                let tests: Vec<_> = selected_tests
                    .iter()
                    .copied()
                    .filter(|path| node_test(&path.to_lowercase()))
                    .collect();
                if normalized == "node --test" && tests.len() == 1 {
                    format!("node --test {}", shell_path(tests[0]))
                } else {
                    normalized
                }
            })
        });
        if command
            .as_ref()
            .is_some_and(|command| command.len() > 4096 || command.contains('\0'))
        {
            return Err(DomainError::LimitExceeded("verifier command".into()));
        }
        Ok(command)
    }
}
