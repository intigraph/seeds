//! Repeat one keyed delivery across an ISO-week boundary, using an in-memory store.
use seeds::backend::{Backend, Claims, Ctx};
use seeds::message::{message_id, window_graph, Message};
use seeds::quipu_backend::QuipuBackend;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let graph = "https://example.org/project/mail";
    let mut backend = QuipuBackend::in_memory(graph)?;
    let mut windows = Vec::new();
    let mut writes = 0;
    for at in ["2026-11-01T23:59:59Z", "2026-11-02T00:00:01Z"] {
        let ctx = Ctx {
            now: at.into(),
            actor: "sender".into(),
            prefix: "demo".into(),
            claims: Claims::default(),
        };
        let message = Message {
            id: message_id(Some("delivery-1"), "reader", at, "same message"),
            recipient: "reader".into(),
            sender: Some("sender".into()),
            sent_at: at.into(),
            text: "same message".into(),
            kind: "note".into(),
            delivery_key: Some("delivery-1".into()),
            about: vec![],
            read_at: None,
        };
        let window = window_graph(graph, at).ok_or("invalid fixture time")?;
        let (written, _) = backend.message_send(&ctx, &window, &message, "demo")?;
        writes += usize::from(written);
        println!("same key at {at}: written={written}");
        windows.push(window);
    }
    let messages = backend.messages(&windows, None)?;
    println!("writes={writes}; messages={}", messages.len());
    for message in messages {
        println!("stored send time: {}", message.sent_at);
    }
    println!("work items={}", backend.snapshot(None)?.seeds.len());
    Ok(())
}
