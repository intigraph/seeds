// This native-only replay tool intentionally reads a retained JSON fixture.
#![allow(clippy::disallowed_methods)]
//! Offline canonical-name replay; never contacts a server.
#[cfg(feature = "native")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use serde::Deserialize;
    #[derive(Deserialize)]
    struct Row {
        id: String,
        title: String,
        created: String,
        closed: Option<String>,
        status: String,
    }
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: intake_replay INPUT.json SINCE_RFC3339 UNTIL_RFC3339".into());
    }
    let since: jiff::Timestamp = args[2].parse()?;
    let until: jiff::Timestamp = args[3].parse()?;
    if until <= since {
        return Err("until must follow since".into());
    }
    let mut rows: Vec<Row> = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let times: std::collections::BTreeMap<_, _> = rows
        .iter()
        .map(|r| {
            Ok((
                r.id.clone(),
                (
                    r.created.parse::<jiff::Timestamp>()?,
                    r.closed
                        .as_deref()
                        .map(str::parse::<jiff::Timestamp>)
                        .transpose()?,
                ),
            ))
        })
        .collect::<Result<_, jiff::Error>>()?;
    if times.len() != rows.len() {
        return Err("duplicate corpus IDs".into());
    }
    rows.sort_by(|a, b| times[&a.id].0.cmp(&times[&b.id].0).then(a.id.cmp(&b.id)));
    let mut refused = Vec::new();
    let mut proposals = 0;
    let mut same_time_unknown = 0;
    let mut closed_time_unknown = 0;
    for row in rows
        .iter()
        .filter(|r| times[&r.id].0 >= since && times[&r.id].0 < until)
    {
        proposals += 1;
        let corpus: Vec<_> = rows
            .iter()
            .filter(|r| times[&r.id].0 < times[&row.id].0)
            .filter(|r| {
                if matches!(r.status.as_str(), "closed" | "tombstone") && r.closed.is_none() {
                    closed_time_unknown += 1;
                    return false;
                }
                !times[&r.id].1.is_some_and(|time| time <= times[&row.id].0)
            })
            .map(|r| seeds::model::Seed {
                id: r.id.clone(),
                title: r.title.clone(),
                status: "open".into(),
                ..Default::default()
            })
            .collect();
        same_time_unknown += rows
            .iter()
            .filter(|r| r.id != row.id && times[&r.id].0 == times[&row.id].0)
            .count();
        let candidates = seeds::intake::name_candidates(&corpus, &row.title, None)?;
        if !candidates.is_empty() {
            refused.push(serde_json::json!({"id":row.id,"candidates":candidates}));
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "method":"quipu canonical-name resolver; no embeddings; current text and recorded close times",
            "threshold":seeds::intake::NAME_THRESHOLD,"since":args[2],"until":args[3],
            "corpus_rows":rows.len(),"proposals":proposals,"would_refuse":refused.len(),
            "same_timestamp_order_unknown":same_time_unknown,"closed_without_time_comparisons_unknown":closed_time_unknown,
            "limitations":"not exact historical text/status replay; unrecorded edits/reopens/deletions unknown; source corpus coverage supplied separately",
            "refusals":refused
        }))?
    );
    Ok(())
}
#[cfg(not(feature = "native"))]
fn main() {}
