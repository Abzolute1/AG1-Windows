use super::*;
use crate::session::adapters::NativeAdapters;
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tooling {
    #[serde(default)]
    pub browser: Option<Browser>,
    pub process_host: PathBuf,
    pub git: PathBuf,
    pub rg: PathBuf,
    pub fd: PathBuf,
    pub bubblewrap: PathBuf,
    pub targets: crate::projects::Toolchains,
}
pub(super) fn adapters(
    workspace: Arc<Workspace>,
    store: Arc<Store>,
    state: &Path,
    session: &str,
    epoch: u64,
    clock: Arc<dyn Clock>,
    tooling: Option<&Tooling>,
) -> Result<NativeAdapters> {
    let scope = ag1_core::tools::registry::Scope {
        workspace: workspace.id().as_str().into(),
        session: session.into(),
        epoch,
        revision: 0,
    };
    let fetch = Arc::new(crate::web_tools::WebService::new(
        crate::web_http::WebHttp::new(Default::default()),
        scope,
        &nonce()?,
        BTreeMap::new(),
    )?);
    let search = Arc::new(crate::web_tools::SearchService::new(
        fetch.clone(),
        Default::default(),
        &nonce()?,
    )?);
    let mut result = NativeAdapters {
        planning: true,
        web: Some(Arc::new(crate::session::adapters::NativeWeb {
            fetch,
            search: Some(search),
            runtime: tokio::runtime::Handle::current(),
        })),
        memory: Some(Arc::new(crate::memory_tools::MemoryTools::new(
            workspace.clone(),
            store.clone(),
        ))),
        artifact: Some(Arc::new(crate::artifact_tools::ArtifactTools::new(
            workspace.clone(),
            store.clone(),
        ))),
        task: Some(Arc::new(crate::task_tools::TaskTools::new(
            workspace.clone(),
        ))),
        todo: Some(Arc::new(crate::todo_tools::TodoTools::new(
            workspace.clone(),
        ))),
        observation: Some(Arc::new(crate::observation_tools::ObservationTools::new(
            workspace.clone(),
        ))),
        episode: Some(Arc::new(crate::episode_tools::EpisodeTools::new(
            workspace.clone(),
            store.clone(),
            session.into(),
        ))),
        ..Default::default()
    };
    if let Some(t) = tooling {
        let spools = state.join("tools");
        fs::create_dir_all(&spools).map_err(crate::io_error)?;
        let supervisor = Arc::new(crate::process::ProcessSupervisor::new(&t.process_host)?);
        let git = || crate::git::GitService::new(&t.process_host, &t.git, &spools);
        result.git = Some(Arc::new(crate::git_tools::GitTools::new(
            workspace.clone(),
            git()?,
        )));
        result.source = Some(Arc::new(crate::source_tools::SourceTools::new(
            crate::retrieval::SourceHost {
                workspace: workspace.clone(),
                source: crate::source::SourceService::new(workspace.clone()),
                recent: vec![],
                tree: Default::default(),
                cancelled: Arc::new(AtomicBool::new(false)),
                supervisor: crate::process::ProcessSupervisor::new(&t.process_host)?,
                rg: Some(t.rg.clone()),
                spools: spools.clone(),
            },
            git()?,
            store.clone(),
            session.into(),
        )));
        result.repo = Some(Arc::new(crate::repo_tools::RepoTools::new(
            workspace.clone(),
            git()?,
            store.clone(),
        )));
        result.search = Some(Arc::new(crate::search_tools::SearchService::new(
            workspace.clone(),
            supervisor.clone(),
            &t.rg,
            &t.fd,
            &spools,
        )?));
        let shell = Arc::new(crate::shell_tools::ShellService::new(
            workspace.clone(),
            supervisor,
            &spools,
            crate::shell_tools::ShellConfig {
                executable: "/bin/bash".into(),
                env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
                command_prefix: None,
                command_template: None,
                sandbox: crate::shell_tools::Sandbox::Workspace,
                bubblewrap: Some(t.bubblewrap.clone()),
                output_bytes: 8 * 1024 * 1024,
                address_space: None,
            },
        )?);
        result.jobs = Some(Arc::new(crate::job_tools::JobService::new(
            store.clone(),
            shell.clone(),
            clock,
            ag1_core::tools::registry::Scope {
                workspace: workspace.id().as_str().into(),
                session: session.into(),
                epoch,
                revision: 0,
            },
            &nonce()?,
        )?));
        result.shell = Some(shell);
    }
    Ok(result)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Browser {
    pub node: PathBuf,
    pub driver: PathBuf,
    pub playwright: PathBuf,
    pub executable: PathBuf,
}
impl Browser {
    pub(super) fn bind(&self, helper: &Path, state: &Path) -> crate::projects::preview::Browser {
        crate::projects::preview::Browser {
            node: self.node.clone(),
            driver: self.driver.clone(),
            playwright: self.playwright.clone(),
            executable: self.executable.clone(),
            helper: helper.into(),
            state: state.join("browser"),
        }
    }
}
