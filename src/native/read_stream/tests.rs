use super::*;
use crate::{
    backend::Backend,
    engine::{self, CreateReq, UpdateReq},
    quipu_backend::QuipuBackend,
    vocab,
};
use quipu::{
    asof_share::{self, Limits},
    replicate_source::Session,
    signing::SigningIdentity,
};
use ring::signature::Ed25519KeyPair;
use std::sync::Arc;

const GRAPH: &str = "urn:test:ipc-board";
struct Fixture {
    _dir: tempfile::TempDir,
    config_path: PathBuf,
    config: Config,
    cfg: Resolved,
    board: StreamBoard,
    source: QuipuBackend,
    session: Session,
    id: String,
}
fn context() -> Ctx {
    Ctx {
        now: "2026-10-29T00:00:00Z".into(),
        actor: "reader".into(),
        prefix: "sd".into(),
        claims: Claims::default(),
    }
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let mut source = QuipuBackend::in_memory(GRAPH).unwrap();
        source
            .store()
            .graph_create(&vocab::ephemeral_graph(GRAPH))
            .unwrap();
        let id = engine::create(
            &mut source,
            &context(),
            &CreateReq {
                title: "before".into(),
                ..Default::default()
            },
        )
        .unwrap()
        .0
        .id;
        let mut graphs = vec![GRAPH.to_string(), vocab::ephemeral_graph(GRAPH)];
        graphs.sort();
        let cut = asof_share::cut(
            source.store(),
            &asof_share::Request {
                graphs: graphs.clone(),
                context_graphs: vec![],
                tx: None,
                expected_store_id: source.store().store_id().unwrap(),
            },
            dir.path(),
            &Limits::default(),
        )
        .unwrap();
        let records: Vec<Value> = fs::read_to_string(&cut.spool)
            .unwrap()
            .lines()
            .flat_map(|s| {
                serde_json::from_str::<Value>(s).unwrap()["records"]
                    .as_array()
                    .unwrap()
                    .clone()
            })
            .collect();
        let raw = encode(&cut.manifest).unwrap();
        let manifest = dir.path().join("manifest.json");
        fs::write(&manifest, &raw).unwrap();
        let bootstrap = dir.path().join("bootstrap.sse");
        fs::write(
            &bootstrap,
            format!(
                "event: quipu.asof\ndata: {}\n\n",
                json!({"manifest":cut.manifest,"records":records})
            ),
        )
        .unwrap();
        let doc = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let key = Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap();
        let c = json!({"primary_url":"http://127.0.0.1:1","graph":GRAPH,"principal":"fixture-principal", "replication_credential_file":dir.path().join("replicate-token"), "store_id":cut.manifest.source.store_id,"log_epoch":cut.manifest.source.log_epoch,"scope_digest":cut.manifest.scope_digest,"policy_digest":cut.manifest.policy_digest,"stream_key":quipu::signing::public_key_hex(&key),"manifest":manifest,"manifest_sha256":super::super::attest::sha256_hex(&raw),"bootstrap":bootstrap,"socket":dir.path().join("read.sock"),"max_freshness_ms":5000});
        let config_path = dir.path().join("consumer.json");
        fs::write(&config_path, encode(&c).unwrap()).unwrap();
        for path in [&manifest, &bootstrap, &config_path] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let cfg = super::super::config::resolve(&super::super::config::Inputs {
            cwd: dir.path().into(),
            flag_url: Some("http://127.0.0.1:1".into()),
            flag_graph: Some(GRAPH.into()),
            ..Default::default()
        })
        .unwrap();
        let config = Config::load(&config_path, &cfg).unwrap();
        let mut board = config.bootstrap().unwrap();
        let req = board.stream_request().unwrap();
        let (session, open) = Session::open(
            source.store(),
            &req,
            "fixture-session".into(),
            Arc::new(SigningIdentity::new(key, "fixture")),
        )
        .unwrap();
        board.apply_frame(open.event, &open.data, 1).unwrap();
        Self {
            _dir: dir,
            config_path,
            config,
            cfg,
            board,
            source,
            session,
            id,
        }
    }
    fn prove(&mut self, nonce: u8, now: u64) {
        self.board.challenge([nonce; 16], now).unwrap();
        let (frames, answered) = self
            .session
            .poll(self.source.store(), 100, Some([nonce; 16]))
            .unwrap();
        assert!(answered);
        for frame in frames {
            self.board
                .apply_frame(frame.event, &frame.data, now + 1)
                .unwrap();
        }
    }
    fn request(&self, args: &[&str]) -> Request {
        let cli = Cli::try_parse_from(args).unwrap();
        let values = match &cli.command {
            Command::List(a) => serde_json::to_value(a),
            Command::Ready(a) => serde_json::to_value(a),
            Command::Show(a) => serde_json::to_value(a),
            Command::Blocked(a) => serde_json::to_value(a),
            _ => panic!("read only"),
        }
        .unwrap();
        Request {
            version: 1,
            primary_url: self.config.primary_url.clone(),
            graph: GRAPH.into(),
            principal: self.config.principal.clone(),
            command: cli.command.name().into(),
            args: values,
            json: true,
            format: None,
            actor: context().actor,
            now: context().now,
            max_ms: 5000,
        }
    }
}

#[test]
fn actual_shared_applier_renders_existing_board_plate_ready_and_show_outputs() {
    let mut f = Fixture::new();
    let r = f.request(&["sd", "list"]);
    assert!(render(&mut f.board, &f.config, &f.cfg, r, 1).is_err());
    let heartbeat = f.session.heartbeat();
    f.board
        .apply_frame(heartbeat.event, &heartbeat.data, 2)
        .unwrap();
    let r = f.request(&["sd", "ready"]);
    assert!(render(&mut f.board, &f.config, &f.cfg, r, 2).is_err());
    f.prove(1, 3);
    for args in [
        vec!["sd", "--json", "list"],
        vec!["sd", "--json", "ready"],
        vec!["sd", "--json", "blocked"],
        vec!["sd", "--json", "show", f.id.as_str()],
        vec!["sd", "--json", "list", "--assignee", "reader"],
    ] {
        let r = f.request(&args);
        let got = render(&mut f.board, &f.config, &f.cfg, r, 4).unwrap();
        let cli = Cli::try_parse_from(args).unwrap();
        let expected = super::super::dispatch(&cli, &f.cfg, &context(), &mut f.source).unwrap();
        assert_eq!(got.result.stdout, expected.stdout);
        assert_eq!(got.result.code, expected.code);
        assert_eq!(got.source_tx, f.source.snapshot(None).unwrap().tx);
    }
    engine::update(
        &mut f.source,
        &context(),
        std::slice::from_ref(&f.id),
        &UpdateReq {
            title: Some("after push".into()),
            assignee: Some("reader".into()),
            ..Default::default()
        },
    )
    .unwrap();
    f.prove(2, 5);
    for _ in 0..20 {
        let r = f.request(&["sd", "list", "--assignee", "reader"]);
        let got = render(&mut f.board, &f.config, &f.cfg, r, 6).unwrap();
        assert!(got.result.stdout.contains("after push"));
    }
}

#[test]
fn missing_expired_disconnected_and_changed_bindings_never_return_an_empty_board() {
    let mut f = Fixture::new();
    f.prove(1, 3);
    for field in [
        "source",
        "graph",
        "principal",
        "command",
        "version",
        "budget",
    ] {
        let mut r = f.request(&["sd", "list"]);
        match field {
            "source" => r.primary_url.push_str("/foreign"),
            "graph" => r.graph = "urn:foreign".into(),
            "principal" => r.principal = "other".into(),
            "command" => r.command = "create".into(),
            "version" => r.version = 2,
            _ => r.max_ms = 5001,
        }
        assert!(
            render(&mut f.board, &f.config, &f.cfg, r, 4).is_err(),
            "{field}"
        );
    }
    let r = f.request(&["sd", "list"]);
    assert!(render(&mut f.board, &f.config, &f.cfg, r, 5004).is_err());
    f.board.disconnect();
    let r = f.request(&["sd", "ready"]);
    assert!(render(&mut f.board, &f.config, &f.cfg, r, 5).is_err());
    f.board.clock_discontinuity();
    let r = f.request(&["sd", "list"]);
    assert!(render(&mut f.board, &f.config, &f.cfg, r, 5).is_err());
}

#[test]
fn actual_private_ipc_preserves_json_and_charges_client_roundtrip() {
    let mut f = Fixture::new();
    f.prove(1, 3);
    let request = f.request(&["sd", "list"]);
    let reply = render(&mut f.board, &f.config, &f.cfg, request, 4).unwrap();
    let expected = reply.result.stdout.clone();
    let listener = UnixListener::bind(&f.config.socket).unwrap();
    fs::set_permissions(&f.config.socket, fs::Permissions::from_mode(0o600)).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        timeout(&stream).unwrap();
        let r: Request = receive(&mut stream, MAX_REQUEST).unwrap();
        assert_eq!(r.command, "list");
        send(&mut stream, &Ok::<_, String>(reply), MAX_REPLY).unwrap();
    });
    let cli = Cli::try_parse_from(["sd", "--json", "list"]).unwrap();
    let got = read(&cli, &f.cfg, &context(), &f.config_path).unwrap();
    assert_eq!(got.stdout, expected);
    assert!(got.stderr.contains("push read source tx"));
    server.join().unwrap();
}

#[test]
fn cli_configured_read_has_no_primary_fallback_and_mutations_are_not_ipc_operations() {
    let f = Fixture::new();
    let mut cfg = f.cfg.clone();
    cfg.read_stream_config = Some(f.config_path);
    let cli = Cli::try_parse_from(["sd", "list"]).unwrap();
    let error = super::super::run_with(&cli, &cfg).unwrap_err();
    assert!(
        error.message.contains("push read UNKNOWN")
            && error.message.contains("no primary polling fallback")
    );
    for args in [
        vec!["sd", "create", "x"],
        vec!["sd", "update", "sd-a", "--claim"],
        vec!["sd", "close", "sd-a"],
    ] {
        let cli = Cli::try_parse_from(args).unwrap();
        assert!(cli.command.writes());
        assert!(!eligible(&cli.command));
    }
}

#[test]
fn pinned_input_checks_refuse_modified_manifest_and_unsafe_socket_or_packet() {
    let mut f = Fixture::new();
    assert!(run(&f.cfg, &f.config_path, true).is_ok());
    fs::write(&f.config.manifest, b"{}").unwrap();
    assert!(run(&f.cfg, &f.config_path, true).is_err());
    fs::set_permissions(&f.config_path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Config::load(&f.config_path, &f.cfg).is_err());
    fs::set_permissions(&f.config_path, fs::Permissions::from_mode(0o600)).unwrap();
    f.cfg.location = Location::Url("https://other.invalid".into());
    assert!(Config::load(&f.config_path, &f.cfg).is_err());
}

#[test]
fn sse_envelope_preserves_payload_and_rejects_truncation_or_extra_fields() {
    let good = b": heartbeat\r\nevent: quipu.replicate.coverage\r\ndata: {\r\ndata: }\r\n\r\n";
    assert_eq!(
        Frames::new(&good[..]).next_frame().unwrap(),
        Some(("quipu.replicate.coverage".into(), "{\n}".into()))
    );
    for bad in [
        "event: x\ndata: {}",
        "event: x\nid: unsigned\ndata: {}\n\n",
        "data: {}\n\n",
        "event: x\nevent: y\ndata: {}\n\n",
    ] {
        assert!(Frames::new(bad.as_bytes()).next_frame().is_err(), "{bad}");
    }
}
#[test]
fn projection_query_preserves_rows_and_refuses_uncovered_scope() {
    let mut f = Fixture::new();
    f.prove(1, 10);
    let query = format!("SELECT ?title WHERE {{ GRAPH <{GRAPH}> {{ ?s <https://schema.org/name> ?title }} }} LIMIT 10");
    let request = Request {
        version: 1,
        primary_url: f.config.primary_url.clone(),
        graph: GRAPH.into(),
        principal: f.config.principal.clone(),
        command: "read-stream-query".into(),
        args: json!({"query":query}),
        json: true,
        format: None,
        actor: "reader".into(),
        now: context().now,
        max_ms: 5000,
    };
    let result = render(&mut f.board, &f.config, &f.cfg, request, 11).unwrap();
    let value: Value = serde_json::from_str(&result.result.stdout).unwrap();
    assert_eq!(value["count"], 1);
    assert_eq!(value["rows"][0]["title"], "before");
    for text in [
        "SELECT * WHERE { ?s ?p ?o }",
        "SELECT * WHERE { GRAPH <urn:foreign> { ?s ?p ?o } }",
        "SELECT * WHERE { GRAPH ?g { ?s ?p ?o } }",
        "SELECT * FROM <urn:test:ipc-board> WHERE { ?s ?p ?o }",
        "ASK { SERVICE <https://remote.example> { ?s ?p ?o } }",
        "ASK { GRAPH <urn:test:ipc-board> { ?s ?p ?o FILTER EXISTS { GRAPH <urn:foreign> { ?a ?b ?c } } } }",
        "SELECT (EXISTS { GRAPH <urn:foreign> { ?a ?b ?c } } AS ?x) WHERE {}",
        "INSERT DATA { <urn:a> <urn:b> <urn:c> }",
        "CONSTRUCT { ?s ?p ?o } WHERE { GRAPH <urn:test:ipc-board> { ?s ?p ?o } }",
    ] {
        assert!(query_scope::evaluate(&f.source,GRAPH,0,text).is_err(),"admitted {text}");
    }
}

#[test]
fn private_inputs_refuse_fifo_symlink_and_growth_before_reading() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let start = Instant::now();
    assert!(private_file(&fifo, 1024).is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
    let file = dir.path().join("file");
    fs::write(&file, b"1234").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    assert!(private_file(&link, 1024).is_err());
    assert!(private_file(&file, 3).is_err());
    assert_eq!(private_file(&file, 4).unwrap(), b"1234");
}

#[test]
fn replication_credential_is_separate_rotated_and_never_falls_back() {
    let f = Fixture::new();
    let file = &f.config.replication_credential_file;
    fs::write(file, b"replication-fixture-one\n").unwrap();
    fs::set_permissions(file, fs::Permissions::from_mode(0o600)).unwrap();
    let transport = transport::Transport::new(&f.cfg, &f.config).unwrap();
    let first = transport.request("/replicate/stream").unwrap();
    assert_eq!(
        first.header("Authorization"),
        Some("Bearer replication-fixture-one")
    );
    assert!(first.header(super::super::attest::HEADER).is_none());
    assert!(transport.request("/query").is_err());
    assert!(transport.request("/update").is_err());
    assert!(transport.request("/identity").is_err());
    fs::write(file, b"replication-fixture-two\r\n").unwrap();
    assert_eq!(
        transport
            .request("/replicate/challenge")
            .unwrap()
            .header("Authorization"),
        Some("Bearer replication-fixture-two")
    );
    for bytes in [
        b"".as_slice(),
        b"one\ntwo",
        b"bad\r\nHeader: value",
        b"\xff",
    ] {
        fs::write(file, bytes).unwrap();
        assert!(transport.request("/replicate/challenge").is_err());
    }
    fs::write(file, b"valid-fixture").unwrap();
    fs::set_permissions(file, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(transport.request("/replicate/challenge").is_err());
    fs::remove_file(file).unwrap();
    assert!(transport.request("/replicate/challenge").is_err());
}

#[test]
fn replication_wire_uses_only_dedicated_bearer_and_does_not_follow_redirects() {
    use std::net::TcpListener;
    let mut f = Fixture::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    f.config.primary_url = format!("http://{}", listener.local_addr().unwrap());
    f.cfg.location = Location::Url(f.config.primary_url.clone());
    fs::write(&f.config.replication_credential_file, b"wire-fixture").unwrap();
    fs::set_permissions(
        &f.config.replication_credential_file,
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let destination = TcpListener::bind("127.0.0.1:0").unwrap();
    destination.set_nonblocking(true).unwrap();
    let redirect = format!("http://{}/query", destination.local_addr().unwrap());
    let server = thread::spawn(move || {
        let mut paths = Vec::new();
        for response in ["HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
            format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {redirect}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")] {
            let (mut peer,_)=listener.accept().unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut request=BufReader::new(peer.try_clone().unwrap());
            let mut headers=String::new();
            loop { let mut line=String::new(); request.read_line(&mut line).unwrap();
                headers.push_str(&line); if line=="\r\n" {break;} }
            assert!(headers.to_ascii_lowercase().contains("authorization: bearer wire-fixture\r\n"));
            assert!(!headers.to_ascii_lowercase().contains("x-quipu-attest:"));
            paths.push(headers.lines().next().unwrap().to_string());
            let mut body=[0;2];request.read_exact(&mut body).unwrap();assert_eq!(&body,b"{}");
            peer.write_all(response.as_bytes()).unwrap();
        }
        paths
    });
    let transport = transport::Transport::new(&f.cfg, &f.config).unwrap();
    assert!(transport
        .request("/replicate/stream")
        .unwrap()
        .send_bytes(b"{}")
        .is_err());
    let redirected = transport
        .request("/replicate/challenge")
        .unwrap()
        .send_bytes(b"{}");
    assert!(redirected.is_err() || redirected.unwrap().status() == 307);
    assert_eq!(
        server.join().unwrap(),
        vec![
            "POST /replicate/stream HTTP/1.1",
            "POST /replicate/challenge HTTP/1.1"
        ]
    );
    assert!(matches!(destination.accept(),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock));
}

#[test]
fn demo_cli_push_projection_without_primary_queries() {
    use std::net::TcpListener;
    let mut f = Fixture::new();
    let primary = TcpListener::bind("127.0.0.1:0").unwrap();
    primary.set_nonblocking(true).unwrap();
    // Positive control: this listener really observes the configured destination.
    let control = std::net::TcpStream::connect(primary.local_addr().unwrap()).unwrap();
    let (observed, _) = primary.accept().unwrap();
    drop(observed);
    drop(control);
    f.config.primary_url = format!("http://{}", primary.local_addr().unwrap());
    f.cfg.location = Location::Url(f.config.primary_url.clone());
    f.cfg.read_stream_config = Some(f.config_path.clone());
    let mut packet: Value = decode(&fs::read(&f.config_path).unwrap()).unwrap();
    packet["primary_url"] = json!(f.config.primary_url);
    fs::write(&f.config_path, encode(&packet).unwrap()).unwrap();
    f.prove(1, 3);
    let listener = UnixListener::bind(&f.config.socket).unwrap();
    fs::set_permissions(&f.config.socket, fs::Permissions::from_mode(0o600)).unwrap();
    let (replies, rx) = mpsc::sync_channel::<Reply>(1);
    let server = thread::spawn(move || {
        for reply in rx {
            let (mut stream, _) = listener.accept().unwrap();
            timeout(&stream).unwrap();
            let request: Request = receive(&mut stream, MAX_REQUEST).unwrap();
            assert_eq!(request.command, "list");
            send(&mut stream, &Ok::<_, String>(reply), MAX_REPLY).unwrap();
        }
    });
    let cli = Cli::try_parse_from(["sd", "--json", "list"]).unwrap();
    for (nonce, now, title) in [(2, 5, "before"), (3, 8, "after push")] {
        engine::update(
            &mut f.source,
            &context(),
            std::slice::from_ref(&f.id),
            &UpdateReq {
                title: Some(title.into()),
                ..Default::default()
            },
        )
        .unwrap();
        f.prove(nonce, now);
        let request = f.request(&["sd", "--json", "list"]);
        let reply = render(&mut f.board, &f.config, &f.cfg, request, now + 1).unwrap();
        replies.send(reply).unwrap();
        let result = super::super::run_with(&cli, &f.cfg).unwrap();
        assert!(result.stdout.contains(title));
        let value: Value = serde_json::from_str(&result.stdout).unwrap();
        println!(
            "sd --json list: .issues[0].title => {}",
            value["issues"][0]["title"]
        );
        println!("{}", result.stderr);
    }
    drop(replies);
    server.join().unwrap();
    f.board.disconnect();
    let request = f.request(&["sd", "--json", "list"]);
    assert!(render(&mut f.board, &f.config, &f.cfg, request, 10).is_err());
    let error = super::super::run_with(&cli, &f.cfg).unwrap_err();
    assert!(error.message.contains("UNKNOWN"));
    println!("disconnected consumer => {}", error.message);
    assert!(matches!(primary.accept(),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock));
    println!("observed primary HTTP connections: 0 (local fixture, not deployed traffic)");
}
