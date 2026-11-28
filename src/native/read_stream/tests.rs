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
        let doc = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let key = Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap();
        let stream_key = quipu::signing::public_key_hex(&key);
        let identity = Arc::new(SigningIdentity::new(key, "fixture"));
        let signature = quipu::replica_manifest::sign_manifest(&cut.manifest, &identity).unwrap();
        let raw = encode(&cut.manifest).unwrap();
        let manifest = dir.path().join("manifest.json");
        fs::write(&manifest, &raw).unwrap();
        let bootstrap = dir.path().join("bootstrap.sse");
        fs::write(
            &bootstrap,
            format!(
                "event: quipu.asof\ndata: {}\n\n",
                json!({"manifest":cut.manifest,"records":records,"manifest_signature":signature})
            ),
        )
        .unwrap();
        let c = json!({"primary_url":"http://127.0.0.1:1","graph":GRAPH,"principal":"fixture-principal", "replication_credential_file":dir.path().join("replicate-token"), "store_id":cut.manifest.source.store_id,"log_epoch":cut.manifest.source.log_epoch,"scope_digest":cut.manifest.scope_digest,"policy_digest":cut.manifest.policy_digest,"stream_key":stream_key,"manifest":manifest,"manifest_sha256":super::super::attest::sha256_hex(&raw),"bootstrap":bootstrap,"socket":dir.path().join("read.sock"),"max_freshness_ms":5000});
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
        let (session, open) =
            Session::open(source.store(), &req, "fixture-session".into(), identity).unwrap();
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
            park_ms: None,
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
    // Quipu's barrier frames carry a standard SSE cursor id (exact shape of a
    // real /share/asof commit frame: id line first). It is discarded.
    let barrier = b"event: quipu.asof.commit\nid: 1311146\ndata: {\"source_tx\":1311146}\n\n";
    assert_eq!(
        Frames::new(&barrier[..]).next_frame().unwrap(),
        Some(("quipu.asof.commit".into(), "{\"source_tx\":1311146}".into()))
    );
    let id_first = b"id: 7\r\nevent: quipu.changes.commit\r\ndata: {}\r\n\r\nevent: quipu.replicate.coverage\ndata: {}\n\n";
    let mut frames = Frames::new(&id_first[..]);
    assert_eq!(
        frames.next_frame().unwrap(),
        Some(("quipu.changes.commit".into(), "{}".into()))
    );
    // The id is per envelope: the next frame may omit it.
    assert_eq!(
        frames.next_frame().unwrap(),
        Some(("quipu.replicate.coverage".into(), "{}".into()))
    );
    assert_eq!(frames.next_frame().unwrap(), None);
    for bad in [
        "event: x\ndata: {}",
        "event: x\nid: unsigned\ndata: {}\n\n",
        "event: x\nid: 1\nid: 2\ndata: {}\n\n",
        "event: x\nid: -1\ndata: {}\n\n",
        "event: x\nid:\ndata: {}\n\n",
        "event: x\nid: 99999999999999999999\ndata: {}\n\n",
        "event: x\nid: 1 \ndata: {}\n\n",
        "id: 5\n\n",
        "event: x\nretry: 10\ndata: {}\n\n",
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
        park_ms: None,
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
fn unsigned_bootstrap_refusal_names_the_applier_reason() {
    // quipu 0.18.1 served pooled cuts with `"manifest_signature": null`
    // (aegis-xdvg.4). The applier refuses the bootstrap frame; the operator must
    // see why, not only the later generic "replica bootstrap is incomplete".
    let fx = Fixture::new();
    let raw = fs::read_to_string(&fx.config.bootstrap).unwrap();
    let data = raw
        .strip_prefix("event: quipu.asof\ndata: ")
        .and_then(|s| s.strip_suffix("\n\n"))
        .unwrap();
    let mut frame: Value = serde_json::from_str(data).unwrap();
    frame["manifest_signature"] = Value::Null;
    fs::write(
        &fx.config.bootstrap,
        format!("event: quipu.asof\ndata: {frame}\n\n"),
    )
    .unwrap();
    let error = fx.config.bootstrap().err().unwrap().to_string();
    assert!(error.contains("bootstrap refused"), "{error}");
    assert!(error.contains("signed admitted descriptor"), "{error}");
}

#[test]
fn bootstrap_streams_up_to_the_cut_ceiling_and_still_refuses_bad_input() {
    // A measured full-board cut is ~0.8 GB; the old 256 MiB in-memory read
    // refused it outright. The ceiling now matches the server's cut spool quota.
    const { assert!(MAX_BOOTSTRAP >= 8 * 1024 * 1024 * 1024) };
    const { assert!(MAX_BOOTSTRAP > 256 * 1024 * 1024) };
    let fx = Fixture::new();
    let len = fs::metadata(&fx.config.bootstrap).unwrap().len();
    // Exactly at the ceiling streams to a complete, requestable board.
    let board = fx.config.bootstrap_bounded(len).unwrap();
    assert!(board.stream_request().is_ok());
    // One byte over the ceiling refuses before reading any frame.
    assert!(fx.config.bootstrap_bounded(len - 1).is_err());
    // A truncated bootstrap is never an empty success, streamed or not.
    let full = fs::read(&fx.config.bootstrap).unwrap();
    fs::write(&fx.config.bootstrap, &full[..full.len() / 2]).unwrap();
    assert!(fx.config.bootstrap_bounded(MAX_BOOTSTRAP).is_err());
    // A bootstrap with an extra trailing frame fragment is refused, not applied.
    let mut grown = full.clone();
    grown.extend_from_slice(b"event: quipu.asof\ndata: {}");
    fs::write(&fx.config.bootstrap, &grown).unwrap();
    assert!(fx.config.bootstrap_bounded(MAX_BOOTSTRAP).is_err());
    // Restored exact bytes stream again.
    fs::write(&fx.config.bootstrap, &full).unwrap();
    assert!(fx.config.bootstrap_bounded(MAX_BOOTSTRAP).is_ok());
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

#[test]
fn fresh_projection_preserves_not_found_exit_and_json_contract() {
    let mut f = Fixture::new();
    f.prove(1, 3);
    let known = f.request(&["sd", "--json", "show", &f.id]);
    let present = render(&mut f.board, &f.config, &f.cfg, known, 4).unwrap();
    assert!(present.result.stdout.contains("before"));
    let cli = Cli::try_parse_from(["sd", "--json", "show", "sd-missing"]).unwrap();
    let original = super::super::dispatch(&cli, &f.cfg, &context(), &mut f.source).unwrap_err();
    assert_eq!(original.exit_code(), 3);
    let expected = super::super::error_outcome(cli.wants_json(), &original, vec![]);
    let request = f.request(&["sd", "--json", "show", "sd-missing"]);
    let reply = render(&mut f.board, &f.config, &f.cfg, request, 4).unwrap();
    assert_eq!(reply.result.code, expected.code);
    assert_eq!(reply.result.stdout, expected.stdout);
    assert_eq!(reply.result.stderr, expected.stderr);
    f.board.disconnect();
    let request = f.request(&["sd", "--json", "show", "sd-missing"]);
    assert!(render(&mut f.board, &f.config, &f.cfg, request, 5).is_err());
}

// aegis-xdvg.4 ruling A' (4 s), the #292 review findings, and aegis-az3d.2
// (1.5 s: the scheduled peak must fit the read-headroom window).
#[test]
fn challenge_cadence_stays_inside_the_freshness_cap() {
    assert_eq!(super::CHALLENGE_INTERVAL_MS, 1_500);
    let mut live = super::Liveness::new();
    let mut count = 0;
    for t in (0..3_600_000u64).step_by(10) {
        if live.challenge_due(t) {
            live.challenge_sent(t);
            count += 1;
        }
    }
    assert_eq!(count, 2_400); // ~2400/h per healthy reader, not 4800
}

#[test]
fn a_failed_issue_does_not_burn_the_slot_and_reconnect_or_clock_jump_challenge_at_once() {
    let mut live = super::Liveness::new();
    assert!(live.challenge_due(0));
    // board had no session yet: nothing sent, still due next tick
    assert!(live.challenge_due(10));
    live.challenge_sent(10);
    assert!(!live.challenge_due(1_000));
    live.clock_jump();
    assert!(live.challenge_due(1_000)); // re-prove at once after a clock jump
    live.challenge_sent(3_000);
    live.disconnected(3_500, 0);
    assert!(!live.challenge_due(3_600)); // never while disconnected
    live.reconnected();
    assert!(live.challenge_due(3_700)); // first challenge immediately on reconnect
}

#[test]
fn reconnect_backoff_is_jittered_capped_and_resets_only_after_a_live_minute() {
    // jitter 0 gives backoff/2, jitter 999 just under backoff
    for (jitter, expect) in [
        (
            0u64,
            [500u64, 1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000],
        ),
        (
            999,
            [999, 1_999, 3_998, 7_996, 15_992, 31_984, 59_970, 59_970],
        ),
    ] {
        let mut live = super::Liveness::new();
        let mut now = 1_000;
        for want in expect {
            live.frame_applied(now); // live, but dropped quickly
            live.disconnected(now + 10, jitter);
            assert_eq!(live.next_reconnect - (now + 10), want);
            assert!(live.reconnect_due(live.next_reconnect));
            now = live.next_reconnect;
            live.reconnected();
        }
        assert_eq!(live.backoff, super::RECONNECT_MAX_MS);
    }
    // a connect that hangs > 60 s and fails without ever applying a frame
    let mut live = super::Liveness::new();
    live.disconnected(10, 0);
    live.reconnected();
    let before = live.backoff;
    live.disconnected(200_000, 0);
    assert_eq!(
        live.backoff,
        (before * 2).min(super::RECONNECT_MAX_MS),
        "no reset without a live frame"
    );
    // a session live for a minute resets
    live.reconnected();
    live.frame_applied(300_000);
    live.disconnected(300_000 + super::RECONNECT_HEALTHY_MS, 0);
    assert_eq!(
        live.next_reconnect - (300_000 + super::RECONNECT_HEALTHY_MS),
        super::RECONNECT_INITIAL_MS / 2
    );
    // a refused reconnect backs off too, never past the cap
    for _ in 0..20 {
        live.reconnect_failed(0, 999);
    }
    assert_eq!(live.backoff, super::RECONNECT_MAX_MS);
}

#[test]
fn every_stream_gets_a_new_generation_for_its_challenge_replies() {
    let mut live = super::Liveness::new();
    let first = live.generation();
    live.disconnected(5, 500);
    live.reconnected();
    assert_ne!(live.generation(), first); // a late reply tagged `first` is ignored
}

// aegis-ygz: a reader refusal kept its reason on neither side of the IPC
// boundary, so a freshness-budget refusal of a heavy owner-plate read surfaced
// as "consumer refused freshness or read binding" and, one layer up, as
// "SDK refused or response exceeded budget".
#[test]
fn a_reader_refusal_reaches_the_consumer_with_its_reason() {
    let budget = super::unknown("read exceeded proof budget");
    let relayed = super::relayed_refusal(&super::refusal_reason(&budget.message));
    assert_eq!(relayed.message, budget.message);
    assert!(relayed.message.contains("read exceeded proof budget"));

    // A pre-fix reader sends the generic protocol line; it passes through as-is.
    let old = "push read UNKNOWN; no primary polling fallback";
    assert_eq!(super::relayed_refusal(old).message, old);

    // Anything not in protocol form is still reported as an UNKNOWN push read.
    let foreign = super::relayed_refusal("engine said\nno\tthanks");
    assert!(foreign
        .message
        .starts_with("push read UNKNOWN: reader refused: engine said no thanks"));
    assert_eq!(foreign.kind, budget.kind);
}

#[test]
fn a_relayed_reason_is_one_bounded_line() {
    let long = "x".repeat(10_000);
    let reason = super::refusal_reason(&format!("a\r\nb\u{1b}[31m{long}"));
    assert!(!reason.chars().any(char::is_control));
    assert!(reason.starts_with("a b [31m"));
    assert_eq!(reason.chars().count(), super::MAX_REASON + 1); // + the ellipsis
    assert_eq!(super::refusal_reason("short"), "short");
}

#[test]
fn actual_private_ipc_relays_the_reader_refusal_reason() {
    let f = Fixture::new();
    let listener = UnixListener::bind(&f.config.socket).unwrap();
    fs::set_permissions(&f.config.socket, fs::Permissions::from_mode(0o600)).unwrap();
    let refusal = super::unknown("read exceeded proof budget").message;
    let sent = refusal.clone();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        timeout(&stream).unwrap();
        let _: Request = receive(&mut stream, MAX_REQUEST).unwrap();
        send(&mut stream, &Err::<Reply, _>(sent), MAX_REPLY).unwrap();
    });
    let cli = Cli::try_parse_from(["sd", "--json", "list"]).unwrap();
    let error = read(&cli, &f.cfg, &context(), &f.config_path).unwrap_err();
    assert_eq!(error.message, refusal);
    server.join().unwrap();
}

// aegis-ygz, Sattler ruling A: an on-demand challenge for a read that lacks
// freshness headroom, capped IN CODE at one per interval.
#[test]
fn on_demand_challenges_are_capped_at_one_per_interval() {
    let mut live = super::Liveness::new();
    // Worst case: a headroom-starved read arrives every 10 ms for an hour.
    for t in (0..3_600_000u64).step_by(10) {
        if live.challenge_due(t) {
            live.challenge_sent(t);
        }
        if live.on_demand_due(t) {
            live.on_demand_sent(t);
        }
    }
    let per_hour = 3_600_000 / super::CHALLENGE_INTERVAL_MS;
    assert!(
        live.on_demand_sent <= per_hour,
        "on-demand {}",
        live.on_demand_sent
    );
    assert!(
        live.scheduled_sent + live.on_demand_sent <= 2 * per_hour,
        "total {}",
        live.scheduled_sent + live.on_demand_sent
    );
    assert!(live.on_demand_sent > 0);
}

#[test]
fn idle_reader_stays_at_the_scheduled_cadence() {
    let mut live = super::Liveness::new();
    for t in (0..3_600_000u64).step_by(10) {
        if live.challenge_due(t) {
            live.challenge_sent(t);
        }
    }
    let per_hour = 3_600_000 / super::CHALLENGE_INTERVAL_MS;
    assert_eq!((live.scheduled_sent, live.on_demand_sent), (per_hour, 0));
}

#[test]
fn on_demand_waits_for_an_in_flight_challenge_and_restarts_the_schedule() {
    let mut live = super::Liveness::new();
    live.challenge_sent(0);
    assert!(live.in_flight(500));
    assert!(!live.on_demand_due(500)); // a proof is already coming
    assert!(live.on_demand_due(super::CHALLENGE_RTT_HEADROOM_MS));
    live.on_demand_sent(2_500);
    assert!(!live.on_demand_due(2_500 + super::CHALLENGE_INTERVAL_MS - 1));
    assert!(!live.challenge_due(2_500 + super::CHALLENGE_INTERVAL_MS - 1));
    assert!(live.challenge_due(2_500 + super::CHALLENGE_INTERVAL_MS));
    // A dropped stream issues nothing on demand.
    live.disconnected(3_000, 0);
    assert!(!live.on_demand_due(100_000));
}

#[test]
fn headroom_is_the_read_budget_inside_the_freshness_cap() {
    let mut f = Fixture::new();
    assert!(!super::has_headroom(&f.board, 0)); // no proof yet: UNKNOWN
    f.prove(1, 1_000);
    let fresh = f.board.staleness_bound(1_000).unwrap();
    let limit = super::MAX_FRESH_MS - super::READ_HEADROOM_MS;
    assert!(super::has_headroom(&f.board, 1_000));
    assert!(super::has_headroom(&f.board, 1_000 + limit - fresh));
    assert!(!super::has_headroom(&f.board, 1_000 + limit - fresh + 1));
    // The measured refusal: a bound of ~4.2 s left no room for a 0.84 s read.
    assert!(!super::has_headroom(&f.board, 1_000 + 4_200));
}

#[test]
fn replication_http_budget_covers_stream_and_challenge_dispatch() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};

    const CHILD: &str = "SEEDS_REPLICATION_BUDGET_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        for limit in ["0", "1", "invalid"] {
            let cache = tempfile::tempdir().unwrap();
            // libtest names omit the crate prefix from module_path!(). Require
            // one actual executed test so an incorrect filter cannot pass.
            let name = format!(
                "{}::replication_http_budget_covers_stream_and_challenge_dispatch",
                module_path!().split_once("::").unwrap().1,
            );
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &name, "--nocapture"])
                .env(CHILD, "1")
                .env("ST_QUIPU_READS_PER_MINUTE", limit)
                .env("XDG_CACHE_HOME", cache.path())
                .output()
                .unwrap();
            assert!(
                result.status.success()
                    && String::from_utf8_lossy(&result.stdout).contains("1 passed"),
                "replication budget control {limit}: {} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr),
            );
        }
        return;
    }
    let mut f = Fixture::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    f.config.primary_url = format!("http://{}", listener.local_addr().unwrap());
    fs::write(
        &f.config.replication_credential_file,
        "fixture-only-token\n",
    )
    .unwrap();
    fs::set_permissions(
        &f.config.replication_credential_file,
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let transport = transport::Transport::new(&f.cfg, &f.config);
    let limit = std::env::var("ST_QUIPU_READS_PER_MINUTE").unwrap();
    if limit == "invalid" {
        assert!(transport.is_err());
        assert!(matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock));
        return;
    }
    let transport = transport.unwrap();
    assert!(transport.request("/query").is_err());
    let done = Arc::new(AtomicBool::new(false));
    let stop = done.clone();
    let worker = std::thread::spawn(move || {
        let mut paths = Vec::new();
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut bytes = Vec::new();
                    let mut byte = [0];
                    while !bytes.ends_with(b"\r\n\r\n") {
                        assert_eq!(stream.read(&mut byte).unwrap(), 1);
                        bytes.push(byte[0]);
                        assert!(bytes.len() < 16384);
                    }
                    let header = String::from_utf8(bytes).unwrap();
                    paths.push(header.lines().next().unwrap().to_owned());
                    assert!(header.contains("Bearer fixture-only-token"));
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .unwrap();
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if stop.load(Ordering::Acquire) {
                        return paths;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(e) => panic!("loopback accept failed: {e}"),
            }
        }
    });
    assert!(transport
        .request("/replicate/stream")
        .unwrap()
        .call()
        .is_ok());
    let challenge = transport.request("/replicate/challenge").unwrap().call();
    done.store(true, Ordering::Release);
    let paths = worker.join().unwrap();
    if limit == "1" {
        assert!(challenge
            .unwrap_err()
            .to_string()
            .contains("local Quipu read budget"));
        assert_eq!(paths, ["POST /replicate/stream HTTP/1.1"]);
    } else {
        assert!(challenge.is_ok());
        assert_eq!(
            paths,
            [
                "POST /replicate/stream HTTP/1.1",
                "POST /replicate/challenge HTTP/1.1"
            ]
        );
    }
}

#[test]
fn memory_line_reads_the_proc_status_fields_and_marks_missing_ones() {
    let status = "Name:\tsd\nVmData:\t 3980332 kB\nVmRSS:\t 3963540 kB\nThreads:\t3\n";
    assert_eq!(
        memory_line(status),
        "rss_kb=3963540 data_kb=3980332 swap_kb=?"
    );
    assert_eq!(memory_line(""), "rss_kb=? data_kb=? swap_kb=?");
    // A prefix of another field name is not that field.
    assert_eq!(
        memory_line("VmRSSx:\t1 kB\nVmSwap:\tbad kB\n"),
        "rss_kb=? data_kb=? swap_kb=?"
    );
}

/// aegis-az3d.2: a bounded closed-inclusive list goes to the primary; an
/// open-scope read, a show, or an unbounded page stays on the reader.
#[test]
fn bounded_closed_inclusive_lists_are_primary_scope() {
    let scope = |args: &[&str]| primary_scope(&Cli::try_parse_from(args).unwrap().command);
    assert!(scope(&["sd", "list", "--all"]));
    assert!(scope(&["sd", "list", "--all", "--limit", "200"]));
    assert!(scope(&["sd", "list", "-a"]));
    assert!(scope(&["sd", "list", "--status", "closed"]));
    assert!(scope(&["sd", "list", "--status", "open,Closed"]));
    assert!(!scope(&["sd", "list"]));
    assert!(!scope(&["sd", "list", "--limit", "200"]));
    assert!(!scope(&["sd", "list", "--status", "open"]));
    assert!(!scope(&["sd", "list", "--all", "--limit", "0"]));
    assert!(!scope(&["sd", "ready"]));
    assert!(!scope(&["sd", "show", "sd-1"]));
}

/// aegis-az3d.2: time parked is not staleness of the cut answered after it.
/// The client charges the reader's bound plus only the round trip the reader
/// did not hold; an older reader (held 0) is charged the whole trip.
#[test]
fn end_to_end_freshness_charges_transit_not_the_park() {
    // Parked 3 s, a fresh 400 ms cut rendered, 20 ms of IPC on top.
    assert_eq!(end_to_end_ms(400, 3_000, 3_020), 420);
    // Older reader: no held time, the whole round trip is charged.
    assert_eq!(end_to_end_ms(400, 0, 3_020), 3_420);
    assert_eq!(end_to_end_ms(u64::MAX, 0, 1), u64::MAX);
}

/// aegis-az3d.2: the reader honours a shorter caller wait but never extends
/// its own; the client waits for the park plus the ordinary IO timeout.
#[test]
fn park_deadline_is_the_shorter_of_caller_and_reader() {
    let f = Fixture::new();
    let mut r = f.request(&["sd", "list"]);
    assert_eq!(
        f.cfg.read_park_ms,
        super::super::config::DEFAULT_READ_PARK_MS
    );
    assert_eq!(park_ms(&r, &f.cfg), 5_000);
    r.park_ms = Some(1_000);
    assert_eq!(park_ms(&r, &f.cfg), 1_000);
    r.park_ms = Some(9_000);
    assert_eq!(park_ms(&r, &f.cfg), 5_000);
    assert_eq!(reply_timeout(5_000), Duration::from_millis(7_000));
    // A reader configured longer than the default never parks a default
    // caller (which sends no park_ms) past that caller's 7 s reply timeout.
    let mut long = f.cfg.clone();
    long.read_park_ms = super::super::config::MAX_READ_PARK_MS;
    r.park_ms = None;
    assert_eq!(park_ms(&r, &long), 5_000);
    r.park_ms = Some(9_000);
    assert_eq!(park_ms(&r, &long), 9_000);
    // The default wait is not sent, so an older reader still decodes it.
    let r = f.request(&["sd", "list"]);
    assert!(!String::from_utf8(encode(&r).unwrap())
        .unwrap()
        .contains("park_ms"));
    // An older reader's reply has no held_ms; it decodes as 0.
    let old: Reply = serde_json::from_value(json!({
        "version":1,"primary_url":"u","graph":"g","principal":"p","store_id":"s",
        "log_epoch":"e","scope_digest":"d","policy_digest":"q","source_tx":1,
        "local_tx":1,"bound_ms":10,"result":{"stdout":"","stderr":"","code":0}
    }))
    .unwrap();
    assert_eq!(old.held_ms, 0);
}

/// A refusal after a wait names the replica's state, never a bare UNKNOWN.
#[test]
fn lag_line_names_bound_frame_age_and_cursors() {
    let mut f = Fixture::new();
    let line = f.board.lag(1);
    assert!(line.contains("staleness bound UNKNOWN"), "{line}");
    f.prove(1, 10);
    let line = f.board.lag(11);
    assert!(
        line.contains("staleness bound ") && !line.contains("bound UNKNOWN"),
        "{line}"
    );
    assert!(
        line.contains("source tx ") && line.contains("local tx "),
        "{line}"
    );
}

/// The `Reply` an older client decodes: no `held_ms`, unknown fields denied.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct OldReply {
    version: u32,
    primary_url: String,
    graph: String,
    principal: String,
    store_id: String,
    log_epoch: String,
    scope_digest: String,
    policy_digest: String,
    source_tx: u64,
    local_tx: u64,
    bound_ms: u64,
    result: Outcome,
}

/// aegis-az3d.2 review: a read that did not park sends no `held_ms`, so an
/// older client still decodes the reply; a parked read reports its hold from
/// the `since` it was accepted at, never longer than the time since then.
#[test]
fn unparked_reply_decodes_with_an_older_client_and_parked_hold_starts_at_accept() {
    let mut f = Fixture::new();
    let mut clock = Clock::new();
    let (now, _) = clock.sample();
    f.prove(1, now);
    let (mut server, mut client) = UnixStream::pair().unwrap();
    let request = f.request(&["sd", "list"]);
    answer(
        &mut server,
        request,
        &mut f.board,
        &f.config,
        &f.cfg,
        &mut clock,
        now,
        0,
        &mut refusals::Refusals::default(),
    );
    let mut header = [0; 4];
    client.read_exact(&mut header).unwrap();
    let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
    client.read_exact(&mut bytes).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("held_ms"));
    let old: std::result::Result<OldReply, String> = decode(&bytes).unwrap();
    assert!(old.is_ok());

    // Parked: accepted at `since`, released `waited` ms later.
    let started = Instant::now();
    let (since, _) = clock.sample();
    thread::sleep(Duration::from_millis(30));
    let (released, _) = clock.sample();
    f.prove(2, released);
    let request = f.request(&["sd", "list"]);
    answer(
        &mut server,
        request,
        &mut f.board,
        &f.config,
        &f.cfg,
        &mut clock,
        since,
        released - since,
        &mut refusals::Refusals::default(),
    );
    let reply: std::result::Result<Reply, String> = receive(&mut client, MAX_REPLY).unwrap();
    let held = reply.unwrap().held_ms;
    let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap();
    assert!(held >= 30, "held {held} ms");
    assert!(held <= elapsed + 2, "held {held} ms past {elapsed} ms");
}

/// One loop pass as `run` does it: apply a frame, observe, then release at
/// most `RELEASE_PER_PASS` reads. Returns the reads released per pass.
fn passes(q: &mut ParkQueue<u32>, frames: &mut u32, mut now: u64, connected: bool) -> Vec<usize> {
    let mut out = Vec::new();
    while !q.is_empty() && out.len() < 100 {
        *frames += 1; // the pass applies its frames before any render
        q.observe(true, true);
        let mut n = 0;
        while n < RELEASE_PER_PASS {
            if q.next(now, connected).is_none() {
                break;
            }
            n += 1;
        }
        out.push(n);
        now += 10;
    }
    out
}

#[test]
fn a_release_of_all_parked_reads_is_bounded_per_pass_and_apply_runs_between() {
    let mut q = ParkQueue::default();
    for i in 0..MAX_PARKED as u32 {
        assert!(!q.full());
        q.park(i, 0, 5_000);
    }
    assert!(q.full());
    let mut frames = 0;
    let per_pass = passes(&mut q, &mut frames, 100, true);
    assert!(
        per_pass.iter().all(|&n| n <= RELEASE_PER_PASS),
        "{per_pass:?}"
    );
    assert_eq!(per_pass.iter().sum::<usize>(), MAX_PARKED);
    assert_eq!(per_pass.len(), MAX_PARKED.div_ceil(RELEASE_PER_PASS));
    // A frame was applied before every batch: apply kept pace with release.
    assert_eq!(frames as usize, per_pass.len());
}

#[test]
fn released_reads_keep_going_without_a_new_frame_until_headroom_is_lost() {
    let mut q = ParkQueue::default();
    for i in 0..20u32 {
        q.park(i, 0, 5_000);
    }
    q.observe(true, true);
    assert_eq!(q.next(10, true).map(|r| r.0), Some(0)); // oldest first
    q.observe(false, true); // no frame this pass, still fresh
    assert_eq!(q.next(20, true).map(|r| r.0), Some(1));
    q.observe(false, false); // headroom gone: wait again
    assert!(q.next(30, true).is_none());
    q.observe(false, true); // fresh again but nothing moved: still waiting
    assert!(q.next(40, true).is_none());
    q.observe(true, true);
    assert_eq!(q.next(50, true).map(|r| r.0), Some(2));
}

#[test]
fn a_parked_read_goes_at_its_deadline_or_when_the_stream_drops() {
    let mut q = ParkQueue::default();
    q.park(1u32, 0, 5_000);
    q.park(2u32, 100, 300);
    q.observe(false, false);
    assert!(q.next(299, true).is_none());
    assert_eq!(q.next(300, true), Some((2, 100))); // deadline, out of order
    assert!(q.next(301, true).is_none());
    assert_eq!(q.next(302, false), Some((1, 0))); // stream down
    assert!(q.is_empty());
}

#[test]
fn a_zero_park_never_waits() {
    let f = Fixture::new();
    let mut r = f.request(&["sd", "list"]);
    let mut off = f.cfg.clone();
    off.read_park_ms = 0;
    assert_eq!(park_ms(&r, &off), 0);
    r.park_ms = Some(0);
    assert_eq!(park_ms(&r, &f.cfg), 0);
}

/// Exercise the real server answer and the client relay, without a live service.
#[test]
fn refused_answer_sends_stable_code_and_counts_reason() {
    let mut f = Fixture::new();
    let mut clock = Clock::new();
    let mut refusals = refusals::Refusals::default();
    let (mut server, mut client) = UnixStream::pair().unwrap();
    let request = f.request(&["sd", "list"]);
    answer(
        &mut server,
        request,
        &mut f.board,
        &f.config,
        &f.cfg,
        &mut clock,
        0,
        5006,
        &mut refusals,
    );
    let reply: std::result::Result<Reply, String> = receive(&mut client, MAX_REPLY).unwrap();
    let error = relayed_refusal(&reply.err().expect("unproved replica must refuse"));
    assert!(
        error.message.contains("[reason=freshness_unknown]"),
        "{}",
        error.message
    );
    assert!(error.message.contains("after waiting 5006 ms"));
    assert!(refusals
        .counters()
        .contains("refusals_total{reason=freshness_unknown}=1"));
}
