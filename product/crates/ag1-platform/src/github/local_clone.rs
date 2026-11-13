//! New-directory clone publication. No existing working tree is checked out or
//! merged. Interrupted staging folders remain recoverable, never auto-deleted.
use super::*;
use crate::workspace::PinnedDirectory;
use std::os::unix::fs::DirBuilderExt;

pub(super) struct CloneTarget {
    parent: PinnedDirectory,
    destination: String,
    staging: String,
}

fn branch_valid(branch: &str) -> bool {
    !branch.is_empty()
        && branch.len() <= 512
        && !branch.contains("..")
        && !branch.contains("@{")
        && !branch.ends_with('.')
        && !branch
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c))
        && !branch
            .split('/')
            .any(|s| s.is_empty() || s.starts_with('.') || s.ends_with(".lock"))
}
impl LocalGit {
    pub(super) fn prepare_clone(
        &self,
        args: &Value,
        scope: &str,
        control: &ScanControl<'_>,
    ) -> Result<LocalPlan> {
        self.authority(true)?;
        self.workspace.verify_root()?;
        control.check()?;
        if scope.is_empty() || scope.len() > 1024 {
            return Err(DomainError::PermissionDenied);
        }
        let repo = super::super::repo(args).map_err(DomainError::InvalidInput)?;
        let branch = args["branch"]
            .as_str()
            .filter(|s| branch_valid(s))
            .ok_or_else(|| {
                DomainError::InvalidInput("Choose an exact valid branch to clone".into())
            })?;
        let sha = args["expected_remote_sha"]
            .as_str()
            .filter(|s| s.len() == 40 && oid(s) && *s != "0".repeat(40))
            .ok_or_else(|| {
                DomainError::InvalidInput(
                    "Clone requires the reviewed 40-character branch head SHA".into(),
                )
            })?;
        let destination = args["destination"]
            .as_str()
            .filter(|s| {
                !s.is_empty()
                    && s.len() < 2048
                    && !s.contains('\0')
                    && s.split('/')
                        .all(|p| !p.is_empty() && p != "." && p != ".." && p != ".git")
            })
            .ok_or_else(|| {
                DomainError::InvalidInput(
                    "Choose a new relative clone folder inside the granted root".into(),
                )
            })?;
        let path = Path::new(destination);
        let parent = self
            .workspace
            .pin_directory(path.parent().and_then(Path::to_str).unwrap_or(""))?;
        if self.workspace.exists(destination)? {
            return Err(DomainError::Conflict);
        }
        let staging = format!(".ag1-clone-{}", crate::client::nonce()?);
        let admission = self
            .github
            .as_ref()
            .ok_or_else(|| DomainError::Unavailable("Connect GitHub before cloning".into()))?
            .prepare_git(scope, control)
            .map_err(super::super::tool::domain)?;
        parent.verify_identity()?;
        self.authority(true)?;
        let mut preview = json!({"action":"clone","repository":repo,"folder":self.root(),"destination":self.root().join(destination),"remote_branch":format!("refs/heads/{branch}"),"expected_remote_sha":sha,"account":admission.account(),"arguments":args,
            "recovery_path":self.root().join(parent.relative_path()).join(&staging),"network_warning":"Clone this exact reviewed branch into a new folder. Existing destinations are never replaced. Interrupted staging remains recoverable.",
            "limits":"Selected branch with its history, no implicit submodule/LFS downloads, hooks or filters. Exact Git bytes. Checkout bound: 10,000 entries / 256 MiB; network pack files: 256 MiB each. No automatic retries."});
        preview["storage"] = self.storage_admit("clone", control)?;
        let revision = format!(
            "local-git-{:x}",
            Sha256::digest(json!([scope, preview]).to_string())
        );
        Ok(LocalPlan {
            action: "clone".into(),
            args: args.clone(),
            scope: scope.into(),
            head: String::new(),
            branch: format!("refs/heads/{branch}"),
            index: None,
            sources: Vec::new(),
            config_revision: None,
            network: Some(admission),
            clone_target: Some(CloneTarget {
                parent,
                destination: destination.into(),
                staging,
            }),
            transition: None,
            revision,
            expires: Instant::now() + Duration::from_secs(300),
            preview,
        })
    }

    pub(super) fn execute_clone(
        &self,
        mut plan: LocalPlan,
        scope: &str,
        control: &ScanControl<'_>,
    ) -> Result<Value> {
        let target = plan
            .clone_target
            .take()
            .ok_or(DomainError::PermissionDenied)?;
        let admission = plan.network.take().ok_or(DomainError::PermissionDenied)?;
        self.authority(true)?;
        target.parent.verify_identity()?;
        if self.workspace.exists(&target.destination)? {
            return Err(DomainError::Conflict);
        }
        self.github.as_ref().ok_or(DomainError::PermissionDenied)?.execute_git(admission,scope,control,|credential,c| {
            let check=||->Result<()> {c.check()?;self.authority(true)?;self.storage_check(c)?;self.workspace.verify_root()?;target.parent.verify_identity()?;Ok(())};
            check()?;if self.workspace.exists(&target.destination)?{return Err(DomainError::Conflict);}
            // Pinned-parent creation avoids following a rediscovered user path.
            std::fs::DirBuilder::new().mode(0o700).create(target.parent.process_cwd().join(&target.staging)).map_err(io_error)?;
            let work=||->Result<Value> {
                check()?;
                let root=self.root().join(target.parent.relative_path()).join(&target.staging);
                self.workspace.pin_directory(&target.parent.relative_path().join(&target.staging).to_string_lossy())?.verify_identity()?;
                let workspace=WorkspaceService::new()?.register(&root)?;
                if workspace.root()!=root{return Err(DomainError::Conflict);}
                check()?;
                for path in [".git/objects/info",".git/objects/pack",".git/refs/heads",".git/refs/remotes/origin",".git/info"] {workspace.create_directories_checked(path,&check)?;}
                let repo=plan.args["repo"].as_str().unwrap();let branch=plan.args["branch"].as_str().unwrap();let sha=plan.args["expected_remote_sha"].as_str().unwrap();
                let persistent=format!("[core]\nrepositoryformatversion = 0\nbare = false\nfilemode = true\n[remote \"origin\"]\nurl = https://github.com/{repo}.git\nfetch = +refs/heads/*:refs/remotes/origin/*\n[branch {}]\nremote = origin\nmerge = {}\n",json!(branch),json!(format!("refs/heads/{branch}")));
                workspace.create_checked(".git/config",persistent.as_bytes(),false,&check)?;
                workspace.create_checked(".git/HEAD",format!("ref: refs/heads/{branch}\n").as_bytes(),false,&check)?;
                let local=LocalGit{storage_active:std::sync::atomic::AtomicU8::new(1),storage_polled:std::sync::Mutex::new(Instant::now()),#[cfg(test)] storage_limits:self.storage_limits,metadata:std::sync::Mutex::new(None),workspace:workspace.clone(),supervisor:self.supervisor.clone(),git:self.git.clone(),bwrap:self.bwrap.clone(),state:self.state.clone(),grant:self.grant.clone(),github:self.github.clone(),
                    #[cfg(test)] network_fixture:self.network_fixture.clone(),
                    #[cfg(test)] transition_hook:None};
                local.run(&["check-ref-format",&format!("refs/heads/{branch}")],c)?;
                let url=format!("https://github.com/{repo}.git");
                #[cfg(test)] let url=local.network_fixture.as_ref().map(|(url,_)|url.clone()).unwrap_or(url);
                let config=super::network::Sensitive(credential.config());
                #[cfg(test)] let config={let mut config=config;if let Some((_,extra))=&local.network_fixture {
                    let certificate=self.workspace.snapshot("fixture-cert.pem",1024*1024)?;
                    workspace.create_checked(".git/fixture-cert.pem",&certificate.bytes,false,&check)?;
                    config.0.extend_from_slice(String::from_utf8_lossy(extra).replace("/workspace/fixture-cert.pem","/workspace/.git/fixture-cert.pem").as_bytes());
                }config};
                let advertised=local.network_command(&["ls-remote".into(),"--refs".into(),"--".into(),url.clone(),format!("refs/heads/{branch}")],&config.0,false,c)?;
                if advertised.0!=format!("{sha}\trefs/heads/{branch}\n").as_bytes(){return Err(DomainError::Conflict);}
                check()?;
                local.network_command(&["fetch".into(),"--quiet".into(),"--no-tags".into(),"--no-write-fetch-head".into(),"--no-auto-maintenance".into(),"--no-write-commit-graph".into(),"--no-recurse-submodules".into(),"--refmap=".into(),"--".into(),url,sha.into()],&config.0,true,c)?;
                if local.run(&["cat-file","-t",sha],c)?.trim()!="commit"{return Err(DomainError::Conflict);}
                let (tree,ok)=local.command(&["ls-tree".into(),"-r".into(),"-l".into(),"-z".into(),sha.into()],None,None,BTreeMap::new(),c)?;
                if !ok{return Err(DomainError::Conflict);}
                let mut count=0;let mut bytes=0u64;
                for entry in tree.split(|b|*b==0).filter(|e|!e.is_empty()) {
                    count+=1;let header=entry.split(|b|*b==b'\t').next().unwrap();let header=std::str::from_utf8(header).map_err(|_|DomainError::Conflict)?;let fields=header.split_whitespace().collect::<Vec<_>>();
                    if fields.len()!=4{return Err(DomainError::Conflict);}
                    if fields[1]=="blob"{bytes=bytes.checked_add(fields[3].parse::<u64>().map_err(|_|DomainError::Conflict)?).ok_or(DomainError::Conflict)?;}
                    if count>10000||bytes>256*1024*1024{return Err(DomainError::LimitExceeded("clone checkout tree".into()));}
                }
                local.run(&["read-tree",sha],c)?;
                // No --force: a file appearing in staging is never overwritten.
                let (_,ok)=local.command_guarded(&["checkout-index".into(),"--all".into()],None,None,BTreeMap::new(),c,None,true)?;
                if !ok{return Err(DomainError::Conflict);}
                local.run(&["update-ref",&format!("refs/heads/{branch}"),sha,&"0".repeat(40)],c)?;
                local.run(&["update-ref",&format!("refs/remotes/origin/{branch}"),sha,&"0".repeat(40)],c)?;
                check()?;workspace.sync_owned_tree_checked(&check)?;
                self.workspace.publish_owned_directory_checked(&workspace,&target.destination,&target.parent,&check)?;
                Ok(json!({"operation":"clone","outcome":"applied","repository":repo,"path":self.root().join(&target.destination),"branch":branch,"commit":sha,"files":count,"checkout_bytes":bytes,"submodules_initialized":false,"automatic_retry":false}))
            };
            // Staging may contain objects/files even if publication never ran.
            // The preview already names its retained recovery location.
            work().map_err(|_|DomainError::DurabilityUnknown)
        })
    }
}
