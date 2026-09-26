//! Local dependency capture for the native repetition guard. All methods that
//! inspect paths run on the existing blocking tool worker, never the async host.
use ag1_core::{
    DomainError,
    runtime::repetition::{self, Inspection},
};
use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Component, Path, PathBuf},
    sync::{LazyLock, Mutex},
};
mod recovery;

pub struct Guard {
    root: PathBuf,
    state: Mutex<repetition::Guard>,
    recovery: Mutex<recovery::Guard>,
}
impl Guard {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            state: Mutex::default(),
            recovery: Mutex::default(),
        }
    }
    pub fn reset(&self) -> Result<(), DomainError> {
        *self.recovery.lock().map_err(|_| DomainError::Conflict)? = Default::default();
        self.state
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .reset();
        Ok(())
    }
    pub fn start_turn(&self) -> Result<(), DomainError> {
        self.state
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .start_turn();
        Ok(())
    }
    pub fn stop_reason(&self) -> Result<Option<String>, DomainError> {
        if let Some(reason) = &self.recovery.lock().map_err(|_| DomainError::Conflict)?.stop {
            return Ok(Some(reason.clone()));
        }
        Ok(self
            .state
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .stop_reason()
            .map(str::to_owned))
    }
    /// Observe only acknowledged native outcomes, never extension projections
    /// or replayed receipts. This does not authorize or execute a retry.
    pub(super) fn recovery_outcome(
        &self, tool: &str, call: &str, args: &Value, result: &Value,
        todo: Option<&Value>,
    ) -> Result<Vec<repetition::Event>, DomainError> {
        let candidate = recovery::candidate(&self.root, tool, args, result, todo);
        Ok(self.recovery.lock().map_err(|_| DomainError::Conflict)?
            .observe(tool, call, candidate))
    }
    pub fn before(
        &self,
        tool: &str,
        call: &str,
        args: &Value,
        now: u64,
    ) -> Result<(Option<String>, Vec<repetition::Event>), DomainError> {
        let candidate = inspection(&self.root, tool, args);
        let revision = candidate
            .as_ref()
            .and_then(|c| revision(&self.root, Path::new(&c.root)));
        Ok(self
            .state
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .before(tool, call, candidate, revision, now))
    }
    pub fn after(
        &self,
        tool: &str,
        call: &str,
        args: &Value,
        result: &Value,
        now: u64,
    ) -> Result<(), DomainError> {
        let mut state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        let revision = state
            .pending_root(call)
            .and_then(|root| revision(&self.root, Path::new(root)));
        let content = result.get("content").unwrap_or(result);
        // Hash the complete output; no prefix or whitespace normalization can
        // justify discarding a changed observation. Details are not SDK content.
        let mut digest = HashWriter(Sha256::new());
        serde_json::to_writer(&mut digest, &(result["isError"] == true, content))
            .map_err(|_| DomainError::InvalidInput("repetition result".into()))?;
        let digest = format!("{:x}", digest.0.finalize());
        let mut preview = String::new();
        let mut units = 0;
        if let Some(blocks) = content.as_array() {
            for text in blocks.iter().filter_map(|b| b["text"].as_str()) {
                if !preview.is_empty() && units < 1000 {
                    preview.push('\n');
                    units += 1;
                }
                for ch in text.chars() {
                    if units + ch.len_utf16() > 1000 {
                        break;
                    }
                    preview.push(ch);
                    units += ch.len_utf16();
                }
                if units >= 1000 {
                    break;
                }
            }
        }
        state.after(
            call,
            repetition::Completed {
                tool,
                recognized: inspection(&self.root, tool, args).is_some(),
                revision: revision.as_deref(),
                result: digest,
                preview,
                now,
            },
        );
        Ok(())
    }
}
struct HashWriter(Sha256);
impl std::io::Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn resolve(base: &Path, path: &str) -> PathBuf {
    let joined = base.join(path);
    let mut output = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                output.pop();
            }
            Component::CurDir => (),
            other => output.push(other),
        }
    }
    output
}
fn inspection(cwd: &Path, tool: &str, args: &Value) -> Option<Inspection> {
    let args = args.as_object()?;
    let (identity, root) = if tool == "bash" {
        shell(cwd, args.get("command")?.as_str()?)?
    } else if matches!(tool, "read" | "grep" | "find" | "ls") {
        let root = resolve(cwd, args.get("path").and_then(Value::as_str).unwrap_or("."));
        let mut identity = args.clone();
        identity.insert("path".into(), json!(root.to_str()?));
        (Value::Object(identity), root)
    } else {
        return None;
    };
    Some(Inspection {
        signature: ag1_storage::digest(json!([tool, identity]).to_string().as_bytes()),
        root: root.to_str()?.into(),
    })
}
fn shell(cwd: &Path, command: &str) -> Option<(Value, PathBuf)> {
    static CD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^cd ([A-Za-z0-9_./-]+) && (.+)$").unwrap());
    static INVENTORY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^ls -la ([A-Za-z0-9_./-]+) && wc -l ([A-Za-z0-9_./-]+)/\*\.ts$").unwrap()
    });
    static LIST: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^ls((?: -[a-zA-Z]+)*) ([A-Za-z0-9_./-]+)$").unwrap());
    static RANGE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"^sed -n (['"])([0-9]+)(?:,([0-9]+))?p(['"]) ([A-Za-z0-9_./-]+)$"#).unwrap()
    });
    static SEARCH: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"^(rg|grep) (-[nFiE]+) (['"])([A-Za-z0-9_ .:^*+?|(){}\[\]\\/-]+)(['"]) ([A-Za-z0-9_./-]+)$"#,
        )
        .unwrap()
    });
    let mut base = cwd.to_owned();
    let mut remaining = command.trim();
    let cd = CD.captures(remaining);
    if let Some(cd) = &cd {
        if cd[1].starts_with('-') {
            return None;
        }
        base = resolve(cwd, &cd[1]);
        remaining = cd.get(2)?.as_str();
    }
    if let Some(c) = INVENTORY.captures(remaining) {
        if c[1].starts_with('-') || c[2].starts_with('-') {
            return None;
        }
        let root = resolve(&base, &c[1]);
        if root != resolve(&base, &c[2]) {
            return None;
        }
        return Some((json!(["inventory", root.to_str()?, "*.ts"]), root));
    }
    if let Some(c) = LIST.captures(remaining) {
        if c[2].starts_with('-') {
            return None;
        }
        let root = resolve(&base, &c[2]);
        return Some((json!(["ls", &c[1], root.to_str()?]), root));
    }
    if let Some(c) = RANGE.captures(remaining) {
        if c[1] != c[4] || c[5].starts_with('-') {
            return None;
        }
        let start: u64 = c[2].parse().ok()?;
        let end: u64 = c.get(3).map_or(&c[2], |v| v.as_str()).parse().ok()?;
        if start == 0 || end < start || end > 9_007_199_254_740_991 {
            return None;
        }
        let root = resolve(&base, &c[5]);
        return Some((json!(["sed-range", root.to_str()?, start, end]), root));
    }
    if let Some(c) = SEARCH.captures(remaining) {
        if c[3] != c[5] || c[6].starts_with('-') {
            return None;
        }
        let root = resolve(&base, &c[6]);
        return Some((json!([&c[1], &c[2], &c[3], &c[4], root.to_str()?]), root));
    }
    None
}
#[cfg(unix)]
fn revision(cwd: &Path, root: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    if !root.starts_with(cwd) || std::fs::canonicalize(root).ok()? != root {
        return None;
    }
    let mut pending = vec![root.to_owned()];
    let mut entries = Vec::new();
    while let Some(path) = pending.pop() {
        if entries.len() + pending.len() + 1 > 256 {
            return None;
        }
        let stat = std::fs::symlink_metadata(&path).ok()?;
        if !stat.is_file() && !stat.is_dir() {
            return None;
        }
        entries.push(json!([
            path.to_str()?,
            stat.dev().to_string(),
            stat.ino().to_string(),
            stat.size().to_string(),
            format!("{}:{}", stat.mtime(), stat.mtime_nsec()),
            format!("{}:{}", stat.ctime(), stat.ctime_nsec())
        ]));
        if stat.is_dir() {
            let mut children = Vec::new();
            for child in std::fs::read_dir(&path).ok()? {
                children.push(child.ok()?.path());
                if entries.len() + pending.len() + children.len() > 256 {
                    return None;
                }
            }
            children.sort();
            pending.extend(children);
        }
    }
    Some(ag1_storage::digest(json!(entries).to_string().as_bytes()))
}
#[cfg(not(unix))]
fn revision(_: &Path, _: &Path) -> Option<String> {
    None
}
