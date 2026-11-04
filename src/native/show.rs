//! Bounded current-state presentation; storage and full history are unchanged.
use super::{cli::ShowArgs, ok, Outcome};
use crate::{backend::Backend, engine, error::Result, output};
use serde_json::json;

pub(super) fn run(
    a: &ShowArgs,
    b: &dyn Backend,
    at: Option<u64>,
    as_json: bool,
) -> Result<Outcome> {
    let mut views = engine::show(b, &a.ids, at)?;
    let totals: Vec<_> = views.iter().map(|v| v.comments.len()).collect();
    if !a.all_comments {
        for view in &mut views {
            let hidden = view.comments.len().saturating_sub(a.comments_limit);
            view.comments.drain(..hidden);
        }
    }
    let mut value = output::show_json(&views);
    let mut texts = Vec::new();
    for ((view, total), row) in views
        .iter()
        .zip(totals)
        .zip(value.as_array_mut().into_iter().flatten())
    {
        let folded = total - view.comments.len();
        row["comments_total"] = json!(total);
        row["comments_folded"] = json!(folded);
        let mut text = output::view_text(view);
        if folded > 0 {
            text.push_str(&format!(
                "\n  {folded} older comments folded; sd show {} --all-comments for full history",
                view.seed.id
            ));
        }
        if total > 20 {
            text.push_str("\n  Consolidate current state with sd rescope; historical comments remain available.");
        }
        let external = crate::blockers::display(b, &view.seed, at);
        if crate::blockers::has_metadata(&view.seed) {
            text.push_str(&format!("\nexternal blockers: {external}"));
        }
        row["external_blockers"] = external;
        texts.push(text);
    }
    Ok(ok(as_json, value, texts.join("\n\n"), vec![]))
}
