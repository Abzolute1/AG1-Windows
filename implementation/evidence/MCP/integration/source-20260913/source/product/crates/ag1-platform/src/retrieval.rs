//! Source retrieval over a root capability, captured branch/Git signals, and an owned rg process.
use crate::{
    io_error,
    process::{ProcessSpec, ProcessSupervisor},
    source::SourceService,
    workspace::{MAX_READ, Workspace},
};
use ag1_core::{
    DomainError,
    context::retrieval::{RecentAccess, SourceRetrievalPort, WorkingTree},
    evidence::SourceSnapshot,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
static SERIAL: AtomicU64 = AtomicU64::new(1);
pub struct SourceHost {
    pub workspace: Arc<Workspace>,
    pub source: SourceService,
    pub recent: Vec<RecentAccess>,
    pub tree: WorkingTree,
    pub cancelled: Arc<AtomicBool>,
    pub supervisor: ProcessSupervisor,
    pub rg: Option<PathBuf>,
    pub spools: PathBuf,
}
impl SourceHost {
    fn cpu_search(
        &self,
        terms: &[String],
        check: &impl Fn() -> Result<(), DomainError>,
    ) -> Result<BTreeMap<String, BTreeSet<usize>>, DomainError> {
        let mut out: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        let mut count = 0;
        let terms: Vec<_> = terms.iter().map(|s| s.to_lowercase()).collect();
        let mut bytes = 0;
        for path in self.workspace.scan(1000)? {
            check()?;
            let Ok(snapshot) = self.workspace.snapshot(&path, 2 * 1024 * 1024) else {
                continue;
            };
            bytes += snapshot.bytes.len();
            if bytes > 16 * 1024 * 1024 {
                return Err(DomainError::LimitExceeded("CPU lexical input".into()));
            }
            let Ok(text) = std::str::from_utf8(&snapshot.bytes) else {
                continue;
            };
            for (line, text) in text.lines().enumerate() {
                if terms.iter().any(|term| text.to_lowercase().contains(term)) {
                    out.entry(path.clone()).or_default().insert(line + 1);
                    count += 1;
                    if count == 100 {
                        return Ok(out);
                    }
                }
            }
        }
        Ok(out)
    }
}
impl SourceRetrievalPort for SourceHost {
    fn capture(&self, path: &str) -> Result<SourceSnapshot, DomainError> {
        self.source.capture(path)
    }
    fn lexical(&self, terms: &[String]) -> Result<BTreeMap<String, BTreeSet<usize>>, DomainError> {
        self.lexical_checked(terms, &|| {
            if self.cancelled() {
                Err(DomainError::Cancelled)
            } else {
                Ok(())
            }
        })
    }
    fn working_tree(&self) -> Result<WorkingTree, DomainError> {
        Ok(self.tree.clone())
    }
    fn recent_accesses(&self) -> Result<Vec<RecentAccess>, DomainError> {
        Ok(self.recent.clone())
    }
    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl SourceHost {
    pub fn lexical_checked(
        &self,
        terms: &[String],
        check: &impl Fn() -> Result<(), DomainError>,
    ) -> Result<BTreeMap<String, BTreeSet<usize>>, DomainError> {
        if terms.is_empty() {
            return Ok(BTreeMap::new());
        }
        check()?;
        if terms.len() > 12 || terms.iter().any(|s| s.len() > 800) {
            return Err(DomainError::LimitExceeded("lexical query".into()));
        }
        let Some(executable) = &self.rg else {
            return self.cpu_search(terms, check);
        };
        self.workspace.verify_root()?;
        let pattern = terms
            .iter()
            .map(|s| regex::escape(s))
            .collect::<Vec<_>>()
            .join("|");
        let mut argv: Vec<_> = [
            "--json",
            "--line-number",
            "--smart-case",
            "--color=never",
            "--hidden",
            "--sort=path",
            "--max-filesize",
            "2M",
            "--glob",
            "!.git/**",
            "--glob",
            "!node_modules/**",
            "--glob",
            "!dist/**",
            "--glob",
            "!build/**",
            "--",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        argv.push(pattern);
        argv.push(".".into());
        let spool = self.spools.join(format!(
            "source-rg-{}-{}.log",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let mut child = self.supervisor.spawn(
            ProcessSpec {
                resident: false,
                executable: executable.clone(),
                argv,
                cwd: self.workspace.root().into(),
                env: BTreeMap::from([
                    ("LC_ALL".into(), "C".into()),
                    ("RIPGREP_CONFIG_PATH".into(), "/dev/null".into()),
                ]),
                timeout_ms: 10000,
                grace_ms: 100,
            },
            &spool,
        )?;
        let started = Instant::now();
        let status = loop {
            if let Err(error) = check() {
                child.stop()?;
                return Err(error);
            }
            if started.elapsed() > Duration::from_secs(12) {
                child.stop()?;
                return Err(DomainError::TimedOut);
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        if !matches!(status.code(), Some(0 | 1)) {
            return Err(DomainError::Unavailable(
                "owned rg search failed; diagnostic spool retained".into(),
            ));
        }
        let view = child.tail(0)?;
        if view.truncated || view.total_seen > MAX_READ as u64 {
            return Err(DomainError::LimitExceeded("rg response".into()));
        }
        let mut bytes = vec![];
        while bytes.len() < view.disk_bytes {
            bytes.extend(child.log_page(bytes.len() as u64, 32768)?);
        }
        drop(child);
        std::fs::remove_file(spool).map_err(io_error)?;
        self.workspace.verify_root()?;
        let mut out: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        let mut count = 0;
        for line in bytes.split(|b| *b == b'\n') {
            if line.is_empty() {
                continue;
            }
            let event: Value = serde_json::from_slice(line)
                .map_err(|_| DomainError::InvalidInput("rg JSON event".into()))?;
            if event["type"] != "match" {
                continue;
            }
            let path = event["data"]["path"]["text"]
                .as_str()
                .ok_or_else(|| DomainError::InvalidInput("rg UTF-8 path".into()))?
                .trim_start_matches("./");
            let n = event["data"]["line_number"]
                .as_u64()
                .ok_or_else(|| DomainError::InvalidInput("rg line".into()))?
                as usize;
            ag1_core::evidence::SourceReference::new(path, &"0".repeat(64))?;
            out.entry(path.into()).or_default().insert(n);
            count += 1;
            if count == 100 {
                break;
            }
        }
        Ok(out)
    }
}
