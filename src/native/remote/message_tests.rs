//! Message dedup against the real shared applier, including concurrent windows.
use super::*;
use crate::message::{message_id, window_graph, Message};

fn candidate(at: &str, key: &str) -> (Ctx, Message) {
    (
        Ctx {
            now: at.into(),
            actor: "sender".into(),
            prefix: "test".into(),
            claims: Default::default(),
        },
        Message {
            id: message_id(Some(key), "reader", at, "same message"),
            recipient: "reader".into(),
            sender: Some("sender".into()),
            sent_at: at.into(),
            text: "same message".into(),
            kind: "note".into(),
            delivery_key: Some(key.into()),
            about: vec![],
            read_at: None,
        },
    )
}

// Older supported appliers return 204 without affected counts. Keep that
// acknowledgement UNKNOWN; prove the stored effect separately by its binding.
fn send_checked(
    b: &mut RemoteBackend,
    ctx: &Ctx,
    window: &str,
    m: &Message,
    source: &str,
) -> Option<bool> {
    match b.message_send(ctx, window, m, source) {
        Ok((written, _)) => Some(written),
        Err(error) => {
            assert_eq!(error.kind, ErrorKind::Indeterminate, "{error:?}");
            assert!(
                error.message.contains("did not report what it committed"),
                "{error:?}"
            );
            let (_, original) = b.message_receipt(&m.id).unwrap().unwrap();
            assert!(original.same_delivery(m));
            None
        }
    }
}

#[test]
fn remote_key_binding_is_atomic_across_week_windows() {
    let Some(server) = super::tests::quipu_server() else {
        eprintln!("SKIPPED: set SEEDS_TEST_QUIPU_SERVER for the real atomic message control");
        return;
    };
    let graph = "https://example.org/project/messages";
    let mut b = RemoteBackend::connect(&server.base, graph, None, None, &[]).unwrap();
    let (first_ctx, first) = candidate("2026-11-01T23:59:59Z", "sequential");
    let (retry_ctx, retry) = candidate("2026-11-02T00:00:01Z", "sequential");
    let first_w = window_graph(graph, &first.sent_at).unwrap();
    let retry_w = window_graph(graph, &retry.sent_at).unwrap();
    assert_ne!(
        send_checked(&mut b, &first_ctx, &first_w, &first, "test"),
        Some(false)
    );
    assert_ne!(
        send_checked(&mut b, &retry_ctx, &retry_w, &retry, "test"),
        Some(true)
    );
    assert_eq!(
        b.message_receipt(&first.id).unwrap().unwrap(),
        (first_w.clone(), first.clone())
    );
    let mut changed = retry.clone();
    changed.recipient = "someone-else".into();
    assert!(b
        .message_send(&retry_ctx, &retry_w, &changed, "test")
        .is_err());
    assert_eq!(
        b.messages(&[first_w.clone(), retry_w.clone()], None)
            .unwrap(),
        vec![first]
    );

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = ["2026-11-01T23:59:59Z", "2026-11-02T00:00:01Z"]
        .into_iter()
        .map(|at| {
            let base = server.base.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut b = RemoteBackend::connect(&base, graph, None, None, &[]).unwrap();
                let (ctx, m) = candidate(at, "concurrent");
                let window = window_graph(graph, at).unwrap();
                barrier.wait();
                send_checked(&mut b, &ctx, &window, &m, "race")
            })
        })
        .collect();
    let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let known_writes = outcomes.iter().filter(|r| **r == Some(true)).count();
    assert!(known_writes <= 1);
    if outcomes.iter().all(Option::is_some) {
        assert_eq!(known_writes, 1);
    }
    let messages = b.messages(&[first_w, retry_w], None).unwrap();
    assert_eq!(messages.len(), 2, "one sequential and one concurrent key");
    assert!(b.snapshot(None).unwrap().seeds.is_empty());
}

#[test]
fn cli_retry_reports_original_window_and_time() {
    use crate::native::{cli::MsgCommand, msg};
    use crate::quipu_backend::QuipuBackend;
    let graph = "https://example.org/project/receipt";
    let mut b = QuipuBackend::in_memory(graph).unwrap();
    let command = MsgCommand::Send {
        to: "reader".into(),
        from: Some("sender".into()),
        kind: "note".into(),
        key: Some("cli-key".into()),
        about: vec![],
        text: vec!["same message".into()],
        message: None,
        file: None,
    };
    let (first, _) = candidate("2026-11-01T23:59:59Z", "cli-key");
    let (retry, _) = candidate("2027-01-04T00:00:01Z", "cli-key");
    let original = msg(true, &first, &mut b, &command).unwrap();
    let receipt = msg(true, &retry, &mut b, &command).unwrap();
    assert!(original.stdout.contains("2026-W44"));
    assert!(receipt.stdout.contains("2026-W44"));
    assert!(receipt.stdout.contains("2026-11-01T23:59:59Z"));
    assert!(!receipt.stdout.contains("2027-01-04"));
}

/// Forward the real applier's committed update, then discard only its response.
#[test]
fn committed_message_with_lost_response_is_not_written_again() {
    use std::io::{BufRead, BufReader, Read, Write};
    let Some(server) = super::tests::quipu_server() else {
        eprintln!("SKIPPED: set SEEDS_TEST_QUIPU_SERVER for the real lost-response control");
        return;
    };
    let graph = "https://example.org/project/lost-message";
    let mut direct = RemoteBackend::connect(&server.base, graph, None, None, &[]).unwrap();
    let (ctx, first) = candidate("2026-11-01T23:59:59Z", "warm");
    let window = window_graph(graph, &ctx.now).unwrap();
    send_checked(&mut direct, &ctx, &window, &first, "warm");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy_url = format!("http://{}", listener.local_addr().unwrap());
    let base = server.base.clone();
    let proxy = std::thread::spawn(move || {
        let mut updates = 0;
        for _ in 0..5 {
            let (stream, _) = listener.accept().unwrap();
            let mut r = BufReader::new(stream);
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let words: Vec<_> = line.split_whitespace().collect();
            let (method, path) = (words[0].to_string(), words[1].to_string());
            let mut length = 0;
            let mut content_type = String::new();
            let mut accept = String::new();
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                if h == "\r\n" {
                    break;
                }
                if let Some((k, v)) = h.split_once(':') {
                    match k.to_ascii_lowercase().as_str() {
                        "content-length" => length = v.trim().parse().unwrap(),
                        "content-type" => content_type = v.trim().into(),
                        "accept" => accept = v.trim().into(),
                        _ => (),
                    }
                }
            }
            let mut body = vec![0; length];
            r.read_exact(&mut body).unwrap();
            let request = ureq::request(&method, &format!("{base}{path}"))
                .set("Content-Type", &content_type)
                .set("Accept", &accept);
            let response = if method == "GET" {
                request.call()
            } else {
                request.send_bytes(&body)
            }
            .unwrap();
            let status = response.status();
            let mut body = response.into_string().unwrap();
            let status = if path == "/update" {
                updates += 1;
                body = "lost after commit".into();
                502
            } else {
                status
            };
            write!(
                r.get_mut(),
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
        updates
    });
    let mut b = RemoteBackend::connect(&proxy_url, graph, None, None, &[]).unwrap();
    let (_, original) = candidate(&ctx.now, "lost-real");
    let lost = b.message_send(&ctx, &window, &original, "lost");
    assert!(lost.is_ok(), "{lost:?}");
    assert_eq!(proxy.join().unwrap(), 1);
    let (retry_ctx, retry) = candidate("2026-11-02T00:00:01Z", "lost-real");
    let retry_window = window_graph(graph, &retry_ctx.now).unwrap();
    assert_ne!(
        send_checked(&mut direct, &retry_ctx, &retry_window, &retry, "retry"),
        Some(true)
    );
    assert_eq!(
        direct.message_receipt(&retry.id).unwrap().unwrap(),
        (window.clone(), original)
    );
    assert_eq!(
        direct
            .messages(&[window, retry_window], None)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn live_window_registry_refuses_unknown_coverage_and_keeps_old_windows() {
    let project = "https://example.org/project/messages";
    let old = format!("{project}/messages/2025-W52");
    let row = json!({"iri": old, "source": null, "lifecycle": null});
    assert_eq!(
        live_message_windows(project, &json!({"graphs": [row.clone()], "count": 1})).unwrap(),
        vec![old.clone()]
    );
    for value in [
        json!({}),
        json!({"graphs": [] ,"truncated": true}),
        json!({"graphs": [], "count": 1}),
        json!({"graphs": [], "has_more": true}),
        json!({"graphs": [{"iri": old}]}),
        json!({"graphs": [row.clone(), row.clone()]}),
        json!({"graphs": [{"iri": old, "source": null, "lifecycle": "unknown"}]}),
    ] {
        assert!(live_message_windows(project, &value).is_err(), "{value}");
    }
    let registry = json!({"graphs": [row,
        {"iri": format!("{project}/messages/2026-W01"), "source": null, "lifecycle": "frozen"},
        {"iri": format!("{project}/messages/2026-W02"), "source": "attachment", "lifecycle": null},
        {"iri": "https://example.org/foreign/messages/2026-W03", "source": null, "lifecycle": null}]});
    assert_eq!(live_message_windows(project, &registry).unwrap(), vec![old]);
}
