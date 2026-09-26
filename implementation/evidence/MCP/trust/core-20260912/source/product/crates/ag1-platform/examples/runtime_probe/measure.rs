use ag1_platform::session::{
    commands::{RunSettings, SessionCommands},
    registry::{PreparedRuntime, Selection},
};
use ag1_protocol::wire as w;
struct TimedResolver {
    inner: Resolver,
    start: Instant,
    requests: Mutex<Vec<u64>>,
}
impl ProviderResolver for TimedResolver {
    fn resolve<'a>(
        &'a self,
        input: ProviderInput<'a>,
        cancel: &'a AtomicBool,
    ) -> PortFuture<'a, ResolvedRequest> {
        self.requests
            .lock()
            .unwrap()
            .push(self.start.elapsed().as_nanos().try_into().unwrap());
        self.inner.resolve(input, cancel)
    }
}
fn envelope(commands: &SessionCommands, id: &str, command: w::Command) -> w::CommandEnvelope {
    w::CommandEnvelope {
        version: Default::default(),
        id: id.to_owned().try_into().unwrap(),
        scope: commands.scope().clone(),
        expected_revision: commands.snapshot().unwrap().revision,
        command,
        extension: None,
    }
}
fn heap_bytes() -> u64 {
    // glibc allocator observation includes Rust's system allocator and SQLite.
    let info = unsafe { libc::mallinfo2() };
    (info.uordblks + info.hblkhd) as u64
}
pub fn measure() {
    let destination = std::env::args().nth(1).expect("report path");
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap().block_on(async {
        let mut samples = vec![];
        let content = "Owned evidence β\n".repeat(256);
        for _ in 0..25 {
            let (endpoint, server) = server(vec![calls(vec![("read-1","read",json!({"path":"input.txt"}))]),stop(),stop()]);
            let f = fixture(&endpoint,"disabled");
            std::fs::write(f.root.join("input.txt"), &content).unwrap();
            let resolver = Arc::new(TimedResolver{inner:Resolver::new(true),start:Instant::now(),requests:Mutex::new(vec![])});
            let commands = SessionCommands::new(Selection{epoch:1,services:Arc::new(PreparedRuntime::from_shared(f.host.clone(),resolver.clone(),f.files.clone()))},"measurement".to_owned().try_into().unwrap(),Arc::new(FixedClock),RunSettings{compaction:ag1_core::context::compaction::Settings{enabled:false,..Default::default()},..Default::default()}).unwrap();
            let start = Instant::now();
            let origin_ns = resolver.start.elapsed().as_nanos() as u64;
            let prompt = w::Command::Prompt{input:w::PromptInput::Inline{text:"Read input.txt".into()}};
            assert!(matches!(commands.dispatch(envelope(&commands,"prompt",prompt)).await.result,w::CommandResult::Accepted{..}));
            let driver = commands.clone();
            let running = tokio::spawn(async move { driver.run_next(&ignore).await });
            resolver.inner.entered.notified().await;
            let follow = w::Command::Session{action:"follow_up".into(),args:json!({"input":{"kind":"inline","text":"Continue"}})};
            assert!(matches!(commands.dispatch(envelope(&commands,"follow",follow)).await.result,w::CommandResult::Accepted{..}));
            resolver.inner.release.notify_one();
            running.await.unwrap().unwrap().unwrap();
            while commands.run_next(&ignore).await.unwrap().is_some() {}
            let total = start.elapsed().as_nanos() as u64;
            assert!(f.host.settled().unwrap());
            assert!(commands.snapshot().unwrap().queued.is_empty());
            let requests = server.join().unwrap();
            assert_eq!(requests.len(),3);
            let results = requests[1]["messages"].as_array().unwrap().iter().filter(|m|m["role"]=="tool").collect::<Vec<_>>();
            assert_eq!(results.len(),1);
            assert!(results[0]["content"].as_str().unwrap().contains(content.trim_end()));
            let journal = f.store.journal_snapshot(&SessionId::new("session").unwrap()).unwrap();
            let branch = ag1_core::task::branch_values(&journal).unwrap();
            let result = branch.iter().find(|e|e["message"]["role"]=="toolResult").unwrap();
            assert_ne!(result["message"]["isError"],true);
            assert!(requests[2]["messages"].to_string().contains("Continue"));
            let times = resolver.requests.lock().unwrap();
            assert_eq!(times.len(),3);
            samples.push(json!({"total_ns":total,"first_request_ns":times[0]-origin_ns,"tool_context_ns":times[1]-times[0],"followup_ns":times[2]-times[1],"requests":3,"tool_results":1,"full_read_evidence":true,"users":["Read input.txt","Continue"]}));
        }
        let mut replies = vec![];
        for _ in 0..32 { replies.extend([stop(),stop(),stop()]); }
        let (endpoint, server) = server(replies);
        let history = vec![json!({"role":"user","content":"Original task and evidence ".repeat(200)}),json!({"role":"assistant","content":[{"type":"text","text":"Earlier findings are recorded."}],"stopReason":"stop"})];
        let mut f = fixture_seeded(&endpoint,"disabled",history);
        let mut cycles = vec![];
        for i in 0..32 {
            let start = Instant::now();
            let mut turn = input();turn.command=format!("cycle-{i}");turn.message=json!({"role":"user","content":format!("Continue cycle {i}"),"timestamp":0});
            f.host.run(turn,&Resolver::new(false),f.files.clone(),&ignore).await.unwrap();
            let summary = f.host.compact(ag1_platform::session::compaction::CompactInput{command:format!("compact-{i}"),timestamp:NOW.into(),settings:ag1_core::context::compaction::Settings{enabled:true,reserve_tokens:8192,keep_recent_tokens:1},focus:None,reason:ag1_platform::session::compaction::Reason::Manual,llm_timeout_ms:5000},&Resolver::new(false),&ignore).await.unwrap().unwrap();
            let journal = f.store.journal_snapshot(&SessionId::new("session").unwrap()).unwrap();
            let state = SessionRuntime::resume(SessionId::new("session").unwrap(),1,&journal,f.host.configuration().unwrap()).unwrap();
            let projected_messages = state.messages().values().len();
            assert_eq!(state.stamp().epoch(),summary.context_epoch);
            let entries = journal.entries.len();drop(journal);
            f.host = Arc::new(NativeSession::new(state,f.store.clone(),"session".into(),f.workspace.clone(),Arc::new(BlockingPool::new(1,32).unwrap()),f.scheduler.clone(),ProviderHttp::new(std::slice::from_ref(&endpoint),Duration::from_secs(1)).unwrap()).unwrap());
            assert!(f.host.settled().unwrap());
            cycles.push(json!({"cycle":i,"ns":start.elapsed().as_nanos() as u64,"heap_bytes":heap_bytes(),"journal_entries":entries,"projected_messages":projected_messages}));
        }
        assert_eq!(server.join().unwrap().len(),96);
        let report=json!({"status":"measured","runtime":"rust","samples":samples,"cycles":cycles,"input_sha256":ag1_storage::digest(content.as_bytes()),"model_actions":0,"external_network_actions":0,"scope":"Release native SessionCommands/NativeSession, real SQL FULL durability, context custody, native read and owned loopback SSE; resolver-entry intervals include prior response/tool/queue processing. 32 cumulative journal resume/compaction cycles use the same store; glibc allocated bytes are observations, not allocation counts or a proof of constant-space physical history."});
        std::fs::write(destination,serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    });
}
