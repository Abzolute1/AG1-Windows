//! Exact-ref network Git. The credential exists only in native memory and a
//! readonly namespace-only config bind created from an inherited stdin pipe.
use super::*;
pub(super) struct Sensitive(pub(super) Vec<u8>);
impl Drop for Sensitive {fn drop(&mut self){self.0.fill(0);}}

impl LocalGit {
    pub(super) fn reference(&self,name:&str,control:&ScanControl<'_>)->Result<String> {
        let (_,symbolic)=self.command(&["symbolic-ref".into(),"--quiet".into(),name.into()],None,None,BTreeMap::new(),control)?;
        if symbolic{return Err(DomainError::InvalidInput("A direct remote-tracking ref is required".into()));}
        let (bytes,ok)=self.command(&["show-ref".into(),"--verify".into(),"--hash".into(),name.into()],None,None,BTreeMap::new(),control)?;
        if !ok{return Ok("0".repeat(40));}
        let value=String::from_utf8(bytes).map_err(|_|DomainError::Conflict)?.trim().to_string();
        if value.len()!=40||!oid(&value){return Err(DomainError::Conflict);}Ok(value)
    }
    pub(super) fn execute_network(&self,plan:&LocalPlan,credential:&super::super::transport::GitCredential,control:&ScanControl<'_>)->Result<Value> {
        self.authority(true)?;self.storage_check(control)?;control.check()?;
        let branch=plan.args["branch"].as_str().ok_or(DomainError::PermissionDenied)?;
        let expected=plan.args["expected_remote_sha"].as_str().ok_or(DomainError::PermissionDenied)?;
        let repo=plan.args["repo"].as_str().ok_or(DomainError::PermissionDenied)?;
        let tracking=plan.preview["tracking_ref"].as_str().ok_or(DomainError::PermissionDenied)?;
        let before=plan.preview["tracking_before"].as_str().ok_or(DomainError::PermissionDenied)?;
        let remote_ref=plan.preview["remote_branch"].as_str().ok_or(DomainError::PermissionDenied)?;
        let fetching=plan.action!="push";
        if self.reference(tracking,control)?!=before{return Err(DomainError::Conflict);}
        let lock=IndexLock::acquire(&self.metadata()?.git)?;
        let recheck=||->Result<()> {
            self.authority(true)?;self.storage_check(control)?;control.check()?;lock.verify()?;
            if self.state(control)?!=(plan.head.clone(),plan.branch.clone()) || self.index()?.as_ref().map(|(_,r)|r)!=plan.index.as_ref().map(|(_,r)|r)
                ||Some(self.metadata_snapshot(".git/config",1024*1024)?.revision)!=plan.config_revision{return Err(DomainError::Conflict);}
            Ok(())
        };
        recheck()?;
        let url=format!("https://github.com/{repo}.git");
        #[cfg(test)]
        let url=self.network_fixture.as_ref().map(|(url,_)|url.clone()).unwrap_or(url);
        let config=Sensitive(credential.config());
        #[cfg(test)]
        let config={let mut config=config;if let Some((_,extra))=&self.network_fixture{config.0.extend_from_slice(extra);}config};
        // Git may skip its lease check for an up-to-date push. Independently
        // verify the approved advertised branch first, also binding a fetch's
        // reviewed SHA to the named branch rather than an arbitrary object.
        let advertised=self.network_command(&["ls-remote".into(),"--refs".into(),"--".into(),url.clone(),remote_ref.into()],&config.0,false,control)?;
        let text=std::str::from_utf8(&advertised.0).map_err(|_|DomainError::Conflict)?;
        let actual=if text.is_empty(){"0".repeat(40)}else{
            let lines=text.lines().collect::<Vec<_>>();
            if lines.len()!=1{return Err(DomainError::Conflict);}
            let (sha,name)=lines[0].split_once('\t').ok_or(DomainError::Conflict)?;
            if sha.len()!=40||!oid(sha)||name!=remote_ref{return Err(DomainError::Conflict);}
            sha.to_string()
        };
        if actual!=expected{return Err(DomainError::Conflict);}
        recheck()?;
        if plan.action=="pull" {
            // The separately confirmed fetch already acquired this exact tree.
            // Keep remote verification inside the same account/cancel custody
            // as the upcoming transition, without writing refs or the index.
            if before!=expected || self.reference(tracking,control)?!=expected {return Err(DomainError::Conflict);}
            return Ok(json!({"operation":"pull","outcome":"verified","working_files_changed":false}));
        }
        let args=if fetching {
            vec!["fetch".into(),"--quiet".into(),"--no-tags".into(),"--no-write-fetch-head".into(),"--no-auto-maintenance".into(),"--no-write-commit-graph".into(),"--no-recurse-submodules".into(),"--refmap=".into(),"--".into(),url,expected.into()]
        }else{
            if expected!="0".repeat(40){self.run(&["merge-base","--is-ancestor",expected,&plan.head],control)?;}
            vec!["push".into(),"--quiet".into(),"--porcelain".into(),"--no-verify".into(),"--no-signed".into(),"--recurse-submodules=no".into(),format!("--force-with-lease=refs/heads/{branch}:{}",if expected=="0".repeat(40){""}else{expected}),"--".into(),url,format!("{}:refs/heads/{branch}",plan.head)]
        };
        self.network_command(&args,&config.0,fetching,control)?;
        if fetching {
            // A fetch downloads objects only. Ref publication is a separate
            // exact compare-and-swap after successful commit verification.
            recheck().map_err(|_|DomainError::DurabilityUnknown)?;
            if self.run(&["cat-file","-t",expected],control).map_err(|_|DomainError::DurabilityUnknown)?.trim()!="commit"{return Err(DomainError::DurabilityUnknown);}
            if self.reference(tracking,control).map_err(|_|DomainError::DurabilityUnknown)?!=before{return Err(DomainError::DurabilityUnknown);}
            self.run(&["update-ref","--no-deref",tracking,expected,before],control).map_err(|_|DomainError::DurabilityUnknown)?;
        }
        recheck().map_err(|_|DomainError::DurabilityUnknown)?;
        Ok(json!({"operation":plan.action,"repository":repo,"remote_branch":remote_ref,"commit":if fetching{expected}else{&plan.head},"pull_number":plan.args.get("pull_number"),"expected_remote_sha":expected,"tracking_ref":if fetching{Some(tracking)}else{None},"working_files_changed":false,"outcome":"applied","automatic_retry":false}))
    }

    pub(super) fn network_command(&self,args:&[String],config:&[u8],fetch:bool,control:&ScanControl<'_>)->Result<Sensitive> {
        control.check()?;self.authority(true)?;self.storage_check(control)?;self.layout()?;
        let root=self.workspace.pin_directory("")?;
        let binding=self.metadata()?;
        let metadata=binding.git.pin_directory("")?;
        let common=binding.common.pin_directory("")?;
        let objects=binding.common.pin_directory("objects")?;
        let original_info=binding.common.pin_directory("info")?;
        let original_object_info=binding.common.pin_directory("objects/info")?;
        let info_temp=tempfile::Builder::new().prefix("git-network-info-").tempdir_in(&self.state).map_err(io_error)?;
        let info_workspace=WorkspaceService::new()?.register(info_temp.path())?;
        let empty_info=info_workspace.pin_directory("")?;
        // Config must already be an ordinary in-folder file. bwrap binds its
        // native pipe data over it, without truncating/writing the host file.
        let original=self.metadata_snapshot(".git/config",1024*1024)?.revision;
        let mut argv=["--die-with-parent","--unshare-user","--unshare-pid","--unshare-ipc","--unshare-uts","--cap-drop","ALL","--ro-bind","/usr","/usr","--symlink","usr/bin","/bin","--proc","/proc","--dev","/dev","--tmpfs","/tmp","--dir","/tmp/ag1-home","--ro-bind-fd",PINNED_CWD_FD,"/workspace"].into_iter().map(str::to_string).collect::<Vec<_>>();
        for (pin,target) in [(&common,"/gitcommon"),(&metadata,binding.namespace_git_dir.as_str())] {
            argv.extend(["--ro-bind-fd".into(),format!("{PINNED_DIRECTORY_PREFIX}{}",pin.process_cwd().display()),target.into()]);
        }
        if fetch{argv.extend(["--bind-fd".into(),format!("{PINNED_DIRECTORY_PREFIX}{}",objects.process_cwd().display()),"/gitcommon/objects".into()]);}
        for target in ["/gitcommon/info","/gitcommon/objects/info"] {
            argv.extend(["--ro-bind-fd".into(),format!("{PINNED_DIRECTORY_PREFIX}{}",empty_info.process_cwd().display()),target.into()]);
        }
        for path in ["/lib","/lib64","/etc/ssl","/etc/pki","/etc/resolv.conf","/etc/hosts","/etc/nsswitch.conf","/etc/gai.conf"] {
            if Path::new(path).exists(){argv.extend(["--ro-bind".into(),path.into(),path.into()]);}
        }
        argv.extend(["--ro-bind-data".into(),"0".into(),"/gitcommon/config".into(),"--chdir".into(),"/workspace".into(),"--".into(),self.git.to_str().ok_or(DomainError::PermissionDenied)?.into(),"--no-pager".into(),"--literal-pathspecs".into()]);
        argv.extend_from_slice(args);
        let environment=BTreeMap::from([
            ("PATH".into(),"/usr/bin:/bin".into()),("HOME".into(),"/tmp/ag1-home".into()),("GIT_DIR".into(),binding.namespace_git_dir.clone()),("GIT_COMMON_DIR".into(),"/gitcommon".into()),("GIT_WORK_TREE".into(),"/workspace".into()),("GIT_NO_REPLACE_OBJECTS".into(),"1".into()),
            ("GIT_CONFIG_NOSYSTEM".into(),"1".into()),("GIT_CONFIG_GLOBAL".into(),"/dev/null".into()),("GIT_TERMINAL_PROMPT".into(),"0".into()),("GIT_OPTIONAL_LOCKS".into(),"0".into()),("GIT_NO_LAZY_FETCH".into(),"1".into()),("LC_ALL".into(),"C".into()),
        ]);
        let spool=self.state.join(format!("git-network-{}.log",crate::client::nonce()?));
        struct Cleanup(PathBuf);impl Drop for Cleanup{fn drop(&mut self){let _=std::fs::remove_file(&self.0);}}
        let _cleanup=Cleanup(spool.clone());
        let mut child=self.supervisor.spawn_private_git(ProcessSpec{executable:self.bwrap.clone(),argv,cwd:root.process_cwd(),env:environment,timeout_ms:control.deadline.saturating_duration_since(Instant::now()).as_millis().clamp(1,300000) as u64,grace_ms:100,resident:false},&spool)?;
        // A later error is conservatively uncertain: the child may already have
        // downloaded objects or submitted its exact remote reference update.
        let mut execute=||->Result<Sensitive> {
            let mut offset=0;
            while offset<config.len(){control.check()?;self.authority(true)?;self.storage_check(control)?;match child.try_write_input(&config[offset..])?{Some(0)=>return Err(DomainError::Conflict),Some(n)=>offset+=n,None=>std::thread::sleep(Duration::from_millis(2)),}}
            child.close_input();
            let mut output=Sensitive(Vec::new());
            loop{
                control.check()?;self.authority(true)?;self.storage_check(control)?;
                let (mut bytes,done)=child.poll_stdout(32768)?;output.0.extend_from_slice(&bytes);bytes.fill(0);
                if output.0.len()>2*1024*1024{return Err(DomainError::LimitExceeded("network Git output".into()));}
                if let Some(status)=done{if !status.success(){return Err(DomainError::Unavailable("Git rejected or interrupted the exact network request. Check the remote branch and permissions before retrying.".into()));}break;}
                std::thread::sleep(Duration::from_millis(2));
            }
            root.verify_identity()?;metadata.verify_identity()?;common.verify_identity()?;objects.verify_identity()?;original_info.verify_identity()?;original_object_info.verify_identity()?;empty_info.verify_identity()?;binding.verify(self,true)?;
            if self.metadata_snapshot(".git/config",1024*1024)?.revision!=original{return Err(DomainError::Conflict);}
            Ok(output)
        };
        execute().map_err(|_|DomainError::DurabilityUnknown)
    }
}

#[cfg(test)]
#[path = "local_network_fixture.rs"]
mod fixture;
