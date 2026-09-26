//! Explicit Git metadata authority, retained across preview/confirmation.
//! Logical `.git/` names here are native constants/validated journal lock names;
//! ordinary workspace file operations never resolve through this mapping.
use super::*;
use crate::{change_receipts::Revision, workspace::{GitEntry, GitSnapshot, PinnedDirectory}};
use ag1_core::ByteSnapshot;

pub(super) struct Binding {
    pub(super) git: Arc<Workspace>,
    pub(super) common: Arc<Workspace>,
    links: Vec<(Arc<Workspace>, String, ByteSnapshot)>,
    pins: Vec<PinnedDirectory>,
    worktree_config: Option<FileRevision>,
    pub(super) namespace_git_dir: String,
}
impl Binding {
    fn resolve(local: &LocalGit) -> Result<Self> {
        let metadata = crate::git::readonly::metadata(&local.workspace)?;
        let namespace_git_dir = metadata.namespace_git_dir();
        let mut pins = vec![local.workspace.pin_directory("")?, metadata.git.pin_directory("")?, metadata.common.pin_directory("")?];
        if metadata.links.is_empty() {
            pins.push(local.workspace.pin_directory(".git")?);
        } else {
            let (authority, revision, _) = local.grant.as_ref().ok_or_else(|| DomainError::Unavailable("Linked worktree writes need an explicit Chat folders grant to the common repository metadata".into()))?;
            authority.check(*revision)?;
            let grants = authority.snapshot()?;
            if grants.selection.revision != *revision { return Err(DomainError::Conflict); }
            for workspace in [&metadata.git, &metadata.common] {
                let root = grants.resolve(workspace.root().to_str().ok_or(DomainError::PermissionDenied)?, false)?;
                let relative = workspace.root().strip_prefix(root.root()).map_err(|_| DomainError::PermissionDenied)?;
                // A grant to an ancestor permits this exact directory only. Pin
                // traversal rejects symlinks and retains the granted ancestor.
                pins.push(root.pin_directory(relative.to_str().ok_or(DomainError::PermissionDenied)?)?);
            }
            for ((owner, name, link), expected) in metadata.links.iter().zip([metadata.git.root().to_path_buf(), metadata.common.root().to_path_buf(), local.root().join(".git")]) {
                let text = std::str::from_utf8(&link.bytes).map_err(|_| DomainError::PermissionDenied)?.trim_end();
                let target = if name == ".git" { text.strip_prefix("gitdir: ").ok_or(DomainError::PermissionDenied)? } else { text };
                if name == "commondir" && !["../..", "../../"].contains(&target) { return Err(DomainError::Unavailable("Writable linked worktrees require the standard Git-created relative commondir layout".into())); }
                if normalize(&owner.root().join(target))? != expected { return Err(DomainError::PermissionDenied); }
            }
        }
        let worktree_config = optional_revision(&metadata.git, "config.worktree")?;
        let binding = Self {git:metadata.git, common:metadata.common, links:metadata.links, pins, worktree_config, namespace_git_dir};
        binding.verify(local, false)?;
        Ok(binding)
    }
    pub(super) fn verify(&self, local: &LocalGit, write: bool) -> Result<()> {
        local.workspace.verify_root()?;
        for pin in &self.pins { pin.verify_identity()?; }
        if optional_revision(&self.git, "config.worktree")? != self.worktree_config { return Err(DomainError::Conflict); }
        for (workspace, name, snapshot) in &self.links {
            if workspace.snapshot(name, 4096)?.revision != snapshot.revision { return Err(DomainError::Conflict); }
        }
        if write {
            for name in ["objects/info/alternates","objects/info/http-alternates"] {
                if self.common.snapshot(name,1024*1024).map(|s|!s.bytes.is_empty()).or_else(|e|if e==DomainError::NotFound{Ok(false)}else{Err(e)})? {
                    return Err(DomainError::Unavailable("Native Git writes require a self-contained object store; alternate object stores are not granted through repository metadata".into()));
                }
            }
        }
        if !self.links.is_empty() {
            let (authority, revision, _) = local.grant.as_ref().ok_or(DomainError::PermissionDenied)?;
            authority.check(*revision)?;
            let grants = authority.snapshot()?;
            if grants.selection.revision != *revision { return Err(DomainError::Conflict); }
            for root in [&self.git, &self.common] {
                grants.resolve(root.root().to_str().ok_or(DomainError::PermissionDenied)?, write)?;
            }
            if write && (self.git.directory_identity().0 != local.workspace.directory_identity().0 || self.common.directory_identity().0 != local.workspace.directory_identity().0) {
                return Err(DomainError::Unavailable("Linked worktree writes require metadata and working files on the same filesystem for exact retained-inode recovery".into()));
            }
        }
        Ok(())
    }
    pub(super) fn identity(&self) -> Value {
        let root = |w:&Workspace| { let (device,inode)=w.directory_identity(); json!({"path":w.root(),"device":device,"inode":inode}) };
        json!({"worktree_config":self.worktree_config.as_ref().map(Revision::from),"git":root(&self.git),"common":root(&self.common),"linked":!self.links.is_empty(),"links":self.links.iter().map(|(owner,path,s)|json!({"root":owner.root(),"path":path,"revision":Revision::from(&s.revision)})).collect::<Vec<_>>()})
    }
}
fn optional_revision(root:&Workspace, name:&str)->Result<Option<FileRevision>> {
    match root.snapshot(name,1024*1024) {Ok(s)=>Ok(Some(s.revision)),Err(DomainError::NotFound)=>Ok(None),Err(e)=>Err(e)}
}
fn normalize(path:&Path)->Result<PathBuf> {
    let mut out=PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::RootDir => out.push("/"),
            std::path::Component::Normal(n) => out.push(n),
            std::path::Component::CurDir => {},
            std::path::Component::ParentDir => if !out.pop() {return Err(DomainError::PermissionDenied)},
            _=>return Err(DomainError::PermissionDenied),
        }
    }
    Ok(out)
}
impl LocalGit {
    pub(super) fn metadata(&self)->Result<Arc<Binding>> {
        let mut binding=self.metadata.lock().map_err(|_|DomainError::Conflict)?;
        if let Some(value)=binding.as_ref() { value.verify(self,false)?;return Ok(value.clone()); }
        let value=Arc::new(Binding::resolve(self)?);
        *binding=Some(value.clone());Ok(value)
    }
    fn metadata_target<'a>(&self,path:&'a str)->Result<(Arc<Workspace>, &'a str)> {
        let path=path.strip_prefix(".git/").ok_or(DomainError::PermissionDenied)?;
        if path.is_empty() || path.split('/').any(|p|p.is_empty()||p=="."||p=="..") {return Err(DomainError::PermissionDenied);}
        let binding=self.metadata()?;
        let common=path=="config" || path.starts_with("info/") || path.starts_with("objects/") || path.starts_with("refs/heads/") || path.starts_with("refs/remotes/") || path.starts_with("refs/tags/");
        Ok((if common{binding.common.clone()}else{binding.git.clone()},path))
    }
    pub(super) fn metadata_snapshot(&self,path:&str,limit:usize)->Result<ByteSnapshot> {
        let (root,name)=self.metadata_target(path)?;root.snapshot(name,limit)
    }
    pub(super) fn metadata_git_snapshot(&self,path:&str)->Result<Option<GitSnapshot>> {
        let (root,name)=self.metadata_target(path)?;root.git_snapshot(name)
    }
    pub(super) fn publish_metadata(&self,path:&str,expected:Option<&GitSnapshot>,replacement:Option<&GitEntry>,recovery:&str,check:impl Fn(crate::workspace::UndoStage)->Result<()>)->Result<Option<GitSnapshot>> {
        let (root,name)=self.metadata_target(path)?;
        root.publish_git_entry_to_checked(name,expected,replacement,&self.workspace,recovery,check)
    }
    pub(super) fn verify_record_metadata(&self,value:&Value)->Result<()> {
        let binding=self.metadata()?;
        if let Some(record)=value.get("metadata") {
            if record!=&binding.identity(){return Err(DomainError::Conflict);}
        } else if !binding.links.is_empty() {
            // Legacy records did not authorize external worktree metadata.
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folder_access::{AccessMode, Change, FolderAccess};
    use std::sync::atomic::{AtomicBool, Ordering};
    fn git(root:&Path,args:&[&str])->String {
        let output=std::process::Command::new("/usr/bin/git").args(args).current_dir(root).env_clear().env("PATH","/usr/bin:/bin").env("GIT_CONFIG_NOSYSTEM","1").env("GIT_CONFIG_GLOBAL","/dev/null").env("GIT_AUTHOR_NAME","Owned").env("GIT_COMMITTER_NAME","Owned").env("GIT_AUTHOR_EMAIL","owned@example.invalid").env("GIT_COMMITTER_EMAIL","owned@example.invalid").output().unwrap();
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));String::from_utf8(output.stdout).unwrap().trim().into()
    }
    struct Fixture {temp:tempfile::TempDir, workspace:Arc<Workspace>,grants:Arc<FolderAccess>,cancel:Arc<AtomicBool>,target:String}
    impl Fixture {
        fn new()->Self {
            let temp=tempfile::tempdir().unwrap();let main=temp.path().join("main");let linked=temp.path().join("linked");std::fs::create_dir(&main).unwrap();
            git(&main,&["init","--quiet","-b","main"]);git(&main,&["remote","add","origin","https://github.com/owned/repo.git"]);
            std::fs::write(main.join("file"),b"original\n").unwrap();git(&main,&["add","file"]);git(&main,&["commit","--quiet","-m","baseline"]);
            git(&main,&["checkout","--quiet","-b","target"]);std::fs::write(main.join("file"),b"target\0\xff\n").unwrap();git(&main,&["add","file"]);git(&main,&["commit","--quiet","-m","target"]);let target=git(&main,&["rev-parse","HEAD"]);git(&main,&["checkout","--quiet","main"]);
            git(&main,&["worktree","add","--quiet","-b","linked",linked.to_str().unwrap()]);
            let workspace=WorkspaceService::new().unwrap().register(&linked).unwrap();let grants=FolderAccess::open(workspace.clone(),&temp.path().join("grants"),"owned").unwrap();
            Self{temp,workspace,grants,cancel:Arc::new(AtomicBool::new(false)),target}
        }
        fn control(&self)->ScanControl<'_>{ScanControl{cancel:&self.cancel,deadline:Instant::now()+Duration::from_secs(45)}}
        fn grant(&self,mode:AccessMode){let snapshot=self.grants.snapshot().unwrap();self.grants.change(Change{revision:snapshot.selection.revision,action:if snapshot.selection.folders.len()==1{"add"}else{"mode"}.into(),path:self.temp.path().join("main"),mode:Some(mode)}).unwrap();}
        fn local(&self)->LocalGit {
            let helper=std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("ag1-process-host");
            LocalGit::new(self.workspace.clone(),&helper,Path::new("/usr/bin/git"),Path::new("/usr/bin/bwrap"),&self.temp.path().join("state")).unwrap().with_grant(self.grants.clone(),self.grants.snapshot().unwrap().selection.revision,true)
        }
        fn apply(&self,local:&LocalGit,action:&str,args:Value)->Value {let plan=local.prepare(action,&args,"scope",&self.control()).unwrap();local.execute(plan,"scope",&self.control()).unwrap()}
    }
    #[test]
    fn linked_metadata_requires_explicit_writable_grant_and_preserves_other_worktrees() {
        let f=Fixture::new();let args=json!({"repo":"owned/repo","paths":["file"]});
        assert!(f.local().prepare("stage",&args,"scope",&f.control()).is_err());
        f.grant(AccessMode::ReadOnly);let readonly=f.local();assert!(readonly.prepare("status",&json!({"repo":"owned/repo"}),"scope",&f.control()).is_ok());assert!(readonly.prepare("stage",&args,"scope",&f.control()).is_err());
        f.grant(AccessMode::ReadWrite);let local=f.local();let main=f.temp.path().join("main");let index=std::fs::read(main.join(".git/index")).unwrap();let head=git(&main,&["rev-parse","HEAD"]);
        std::fs::write(f.workspace.root().join("file"),b"linked stage\0\xff").unwrap();f.apply(&local,"stage",args.clone());assert_eq!(local.metadata_snapshot(".git/index",8*1024*1024).unwrap().revision,local.index().unwrap().unwrap().1);
        f.apply(&local,"unstage",args.clone());assert_eq!(git(f.workspace.root(),&["show",":file"]),"original");f.apply(&local,"stage",args);
        f.apply(&local,"commit",json!({"repo":"owned/repo","message":"linked commit","author_name":"Owned","author_email":"owned@example.invalid"}));
        f.apply(&local,"branch_create",json!({"repo":"owned/repo","branch":"linked-created"}));
        let checkout=json!({"repo":"owned/repo","branch":"target","target_sha":f.target});
        f.apply(&local,"checkout",checkout);assert_eq!(std::fs::read(f.workspace.root().join("file")).unwrap(),b"target\0\xff\n");
        assert_eq!(std::fs::read(main.join(".git/index")).unwrap(),index);assert_eq!(git(&main,&["rev-parse","HEAD"]),head);assert_eq!(std::fs::read(main.join("file")).unwrap(),b"original\n");
        assert!(local.prepare("checkout",&json!({"repo":"owned/repo","branch":"main","target_sha":head}),"scope",&f.control()).is_err());
    }
    #[test]
    fn exact_git_objects_ignore_replacement_refs_and_alternate_stores_cannot_gain_write_authority() {
        let f=Fixture::new();f.grant(AccessMode::ReadWrite);let local=f.local();let main=f.temp.path().join("main");let original=git(&main,&["rev-parse","HEAD"]);
        git(&main,&["replace",&original,&f.target]);
        assert_eq!(local.run(&["show",&format!("{original}:file")],&f.control()).unwrap(),"original\n");
        let plan=local.prepare("status",&json!({"repo":"owned/repo"}),"scope",&f.control()).unwrap();assert_eq!(plan.preview["head"],original);assert_eq!(plan.preview["staged_diff"],"");
        let args=json!({"repo":"owned/repo","paths":["file"]});let plan=local.prepare("stage",&args,"scope",&f.control()).unwrap();let index=local.index().unwrap().unwrap().0;
        let alternate=main.join(".git/objects/info/http-alternates");std::fs::write(&alternate,b"https://ungranted.invalid/objects\n").unwrap();
        assert!(matches!(local.execute(plan,"scope",&f.control()),Err(DomainError::Unavailable(_))));assert_eq!(local.index().unwrap().unwrap().0,index);assert!(alternate.exists());
        assert_eq!(local.run(&["show",&format!("{original}:file")],&f.control()).unwrap(),"original\n");
    }
    #[test]
    fn linked_metadata_late_grant_link_config_and_directory_changes_refuse_without_index_write() {
        for change in ["grant","link","config","directory","cancel"] {
            let f=Fixture::new();f.grant(AccessMode::ReadWrite);let local=f.local();std::fs::write(f.workspace.root().join("file"),b"new").unwrap();let args=json!({"repo":"owned/repo","paths":["file"]});let plan=local.prepare("stage",&args,"scope",&f.control()).unwrap();let metadata=local.metadata().unwrap();let index=std::fs::read(metadata.git.root().join("index")).unwrap();
            match change {
                "grant"=>f.grant(AccessMode::ReadOnly),
                "link"=>{let p=f.workspace.root().join(".git");let old=std::fs::read(&p).unwrap();std::fs::write(p,old).unwrap();},
                "config"=>std::fs::write(metadata.git.root().join("config.worktree"),b"[core]\nfilemode=false\n").unwrap(),
                "directory"=>{let p=f.temp.path().join("main/.git");std::fs::rename(&p,f.temp.path().join("retained-metadata")).unwrap();std::fs::create_dir(p).unwrap();},
                "cancel"=>f.cancel.store(true,Ordering::Release),_=>unreachable!(),
            }
            assert!(local.execute(plan,"scope",&f.control()).is_err(),"{change}");
            let index_path=if change=="directory"{f.temp.path().join("retained-metadata/worktrees/linked/index")}else{metadata.git.root().join("index")};assert_eq!(std::fs::read(index_path).unwrap(),index,"{change}");
        }
    }
    #[test]
    fn linked_recovery_survives_reopen_and_rejects_changed_pointer_identity() {
        for changed in [false,true] {
            let f=Fixture::new();f.grant(AccessMode::ReadWrite);let mut local=f.local();let cancel=f.cancel.clone();local.transition_hook=Some(Arc::new(move |phase,_|if phase=="after_index"{cancel.store(true,Ordering::Release)}));
            let plan=local.prepare("checkout",&json!({"repo":"owned/repo","branch":"target","target_sha":f.target}),"scope",&f.control()).unwrap();let id=Path::new(plan.preview["recovery_path"].as_str().unwrap()).file_name().unwrap().to_str().unwrap().to_string();
            assert_eq!(local.execute(plan,"scope",&f.control()),Err(DomainError::DurabilityUnknown));drop(local);f.cancel.store(false,Ordering::Release);
            if changed {let p=f.workspace.root().join(".git");let old=std::fs::read(&p).unwrap();std::fs::write(p,old).unwrap();}
            let local=f.local();let plan=local.prepare("recover",&json!({"repo":"owned/repo","recovery_id":id,"choice":"restore"}),"fresh",&f.control());
            if changed {assert!(plan.is_err());assert_eq!(std::fs::read(f.workspace.root().join("file")).unwrap(),b"target\0\xff\n");}
            else {local.execute(plan.unwrap(),"fresh",&f.control()).unwrap();assert_eq!(std::fs::read(f.workspace.root().join("file")).unwrap(),b"original\n");assert_eq!(git(f.workspace.root(),&["branch","--show-current"]),"linked");}
        }
    }
}
