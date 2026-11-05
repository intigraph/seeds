//! GitHub issue mapping; dependencies use a visible portable body section.
use super::{http::Client, required, Adapter, Config, Item, Note, Remote};
use crate::error::{Result, SdError};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(super) struct Github {
    client: Client,
    repo: String,
}
impl Github {
    pub fn new(config: Config, client: Client) -> Result<Self> {
        let repo = config
            .repository
            .ok_or_else(|| SdError::usage("github requires repository = owner/repo"))?;
        let parts = repo.split('/').collect::<Vec<_>>();
        if parts.len() != 2
            || parts.iter().any(|s| {
                s.is_empty()
                    || matches!(*s, "." | "..")
                    || !s
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            })
        {
            return Err(SdError::usage("invalid GitHub repository"));
        }
        Ok(Self { client, repo })
    }
    fn path(&self, id: &str) -> Result<String> {
        if !id.bytes().all(|c| c.is_ascii_digit()) || id.is_empty() {
            return Err(SdError::usage("GitHub IDs must be issue numbers"));
        }
        Ok(format!("repos/{}/issues/{id}", self.repo))
    }
    fn pages(&self, path: &str, query: &[(&str, String)]) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        for page in 1..=100 {
            let mut q = query.to_vec();
            q.extend([("per_page", "100".into()), ("page", page.to_string())]);
            let data = self.client.request("GET", path, &q, None)?;
            let rows = data
                .as_array()
                .ok_or_else(|| SdError::refused("expected GitHub array"))?;
            out.extend(rows.iter().cloned());
            if rows.len() < 100 {
                return Ok(out);
            }
        }
        Err(SdError::refused(
            "GitHub pagination bound reached; completeness unknown",
        ))
    }
    fn decode(&self, value: &Value) -> Result<Remote> {
        if value.get("pull_request").is_some() {
            return Err(SdError::refused(
                "pull requests are not issue transfer targets",
            ));
        }
        let number = value["number"]
            .as_u64()
            .ok_or_else(|| SdError::refused("GitHub issue number missing"))?
            .to_string();
        let origin = super::decode_origin(
            value["body"].as_str().unwrap_or_default(),
            "<!-- seeds-origin:",
            " -->",
        );
        let mut description = value["body"].as_str().unwrap_or_default().to_string();
        let mut blocked_on = Vec::new();
        if let Some((body, tail)) = description.split_once("\n\n## Seeds dependencies\n") {
            blocked_on = tail
                .lines()
                .filter_map(|l| l.strip_prefix("- blocked by: "))
                .map(str::to_string)
                .collect();
            description = body.into();
        }
        if let Some((body, _)) = description.split_once("\n\n<!-- seeds-origin:") {
            description = body.into();
        }
        let labels = value["labels"]
            .as_array()
            .ok_or_else(|| SdError::refused("GitHub labels missing"))?
            .iter()
            .map(|v| {
                v["name"]
                    .as_str()
                    .or_else(|| v.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| SdError::refused("invalid GitHub label"))
            })
            .collect::<Result<Vec<_>>>()?;
        let managed = |prefix: &str| -> Result<Option<String>> {
            let found = labels
                .iter()
                .filter_map(|l| l.strip_prefix(prefix))
                .collect::<Vec<_>>();
            if found.len() > 1 {
                return Err(SdError::conflict(
                    "multiple conflicting Seeds mapping labels",
                ));
            }
            Ok(found.first().map(|s| s.to_string()))
        };
        if origin.is_some()
            && ["seeds:status:", "seeds:priority:", "seeds:type:"]
                .iter()
                .any(|p| !labels.iter().any(|l| l.starts_with(p)))
        {
            return Err(SdError::refused(
                "GitHub did not preserve required mapping labels",
            ));
        }
        let assignees = value["assignees"]
            .as_array()
            .ok_or_else(|| SdError::refused("GitHub assignees array missing"))?;
        if assignees.len() > 1 {
            return Err(SdError::refused(
                "multiple GitHub assignees require explicit reconciliation",
            ));
        }
        let mapped_status = managed("seeds:status:")?;
        if value["state"] == "open" && mapped_status.as_deref() == Some("closed") {
            return Err(SdError::conflict(
                "open GitHub issue has a closed mapping label; reconcile status",
            ));
        }
        let status = if value["state"] == "closed" {
            "closed".into()
        } else {
            mapped_status.unwrap_or_else(|| "open".into())
        };
        let priority = managed("seeds:priority:")?
            .map(|s| crate::model::parse_priority(&s))
            .transpose()?
            .unwrap_or(2);
        let issue_type = managed("seeds:type:")?.unwrap_or_else(|| "task".into());
        let comments = self
            .pages(&format!("{}/comments", self.path(&number)?), &[])?
            .iter()
            .map(|c| {
                let body = required(c, "body")?;
                if let Some(metadata) = super::decode_origin(&body, "<!-- seeds-note:", " -->") {
                    let mut note: Note = serde_json::from_str(&metadata)
                        .map_err(|_| SdError::refused("malformed Seeds comment provenance"))?;
                    let visible = body
                        .split("\n\n<!-- seeds-note:")
                        .next()
                        .unwrap_or_default();
                    let prefix = format!("{} ({})\n", note.author, note.at);
                    note.text = visible
                        .strip_prefix(&prefix)
                        .ok_or_else(|| {
                            SdError::conflict("exported comment attribution was edited")
                        })?
                        .into();
                    return Ok(note);
                }
                Ok(Note {
                    key: format!(
                        "github:{}:{}",
                        self.repo,
                        c["id"]
                            .as_u64()
                            .ok_or_else(|| SdError::refused("comment ID missing"))?
                    ),
                    author: required(&c["user"], "login")?,
                    at: required(c, "created_at")?,
                    text: body,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let dependency_path = format!("{}/dependencies/blocked_by", self.path(&number)?);
        match self.pages(&dependency_path, &[]) {
            Ok(deps) => {
                blocked_on.extend(
                    deps.iter()
                        .map(|d| required(d, "html_url"))
                        .collect::<Result<Vec<_>>>()?,
                );
                blocked_on.sort();
                blocked_on.dedup();
            }
            Err(e) if e.message.contains("HTTP 404") || e.message.contains("HTTP 410") => {}
            Err(e) => return Err(e),
        }
        // Sub-issues are read from the documented parent relationship.
        let parent =
            match self
                .client
                .request("GET", &format!("{}/parent", self.path(&number)?), &[], None)
            {
                Ok(p) => Some(required(&p, "html_url")?),
                Err(e) if e.message.contains("HTTP 404") => None,
                Err(e) => return Err(e),
            };
        Ok(Remote {
            id: number,
            origin,
            link: required(value, "html_url")?,
            item: Item {
                title: required(value, "title")?,
                description,
                status,
                priority,
                issue_type,
                assignee: assignees
                    .first()
                    .map(|a| required(a, "login"))
                    .transpose()?,
                labels: labels
                    .into_iter()
                    .filter(|l| !l.starts_with("seeds:"))
                    .collect(),
                parent,
                blocked_on,
                comments,
            },
        })
    }
}
fn marker(origin: &str) -> String {
    format!("<!-- seeds-origin:{} -->", hex(origin.as_bytes()))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
impl Adapter for Github {
    fn find(&self, origin: &str) -> Result<Option<Remote>> {
        let marker = marker(origin);
        let mut hits = Vec::new();
        for row in self.pages(
            &format!("repos/{}/issues", self.repo),
            &[("state", "all".into())],
        )? {
            if row.get("pull_request").is_none()
                && row["body"].as_str().is_some_and(|b| b.contains(&marker))
            {
                hits.push(row);
            }
        }
        if hits.len() > 1 {
            return Err(SdError::conflict(
                "multiple GitHub issues claim this Seeds origin",
            ));
        }
        hits.first().map(|v| self.decode(v)).transpose()
    }
    fn get(&self, id: &str) -> Result<Remote> {
        self.decode(&self.client.request("GET", &self.path(id)?, &[], None)?)
    }
    fn get_link(&self, link: &str) -> Result<Remote> {
        let remote = self.get(link.rsplit('/').next().unwrap_or_default())?;
        if remote.link != link {
            return Err(SdError::refused(
                "GitHub link is outside selected repository",
            ));
        }
        Ok(remote)
    }
    fn put(&self, origin: &str, item: &Item, current: Option<&Remote>) -> Result<Remote> {
        let mut labels = item.labels.clone();
        labels.extend([
            format!("seeds:status:{}", item.status),
            format!("seeds:priority:{}", item.priority),
            format!("seeds:type:{}", item.issue_type),
        ]);
        if item.labels.iter().any(|l| l.starts_with("seeds:")) {
            return Err(SdError::refused(
                "seeds: labels are reserved for transfer mappings",
            ));
        }
        let body = format!("{}\n\n{}", item.description, marker(origin));
        let payload = json!({"title":item.title,"body":body,"state":if item.status=="closed"{"closed"}else{"open"},"labels":labels,"assignees":item.assignee.iter().collect::<Vec<_>>()});
        let (method, path) = match current {
            Some(r) => ("PATCH", self.path(&r.id)?),
            None => ("POST", format!("repos/{}/issues", self.repo)),
        };
        let value = self.client.request(method, &path, &[], Some(&payload))?;
        let number = value["number"]
            .as_u64()
            .ok_or_else(|| SdError::refused("GitHub write receipt lacks number"))?
            .to_string();
        if current.is_none() && item.status == "closed" {
            self.client.request(
                "PATCH",
                &self.path(&number)?,
                &[],
                Some(&json!({"state":"closed"})),
            )?;
        }
        // Native comments have idempotent source markers; never replace a human comment.
        let existing = self.pages(&format!("{}/comments", self.path(&number)?), &[])?;
        for note in &item.comments {
            let mut metadata = note.clone();
            metadata.text.clear();
            let tag = format!(
                "<!-- seeds-note:{} -->",
                hex(serde_json::to_string(&metadata)
                    .map_err(|_| SdError::failed("comment encoding failed"))?
                    .as_bytes())
            );
            let body = format!("{} ({})\n{}\n\n{}", note.author, note.at, note.text, tag);
            if let Some(old) = existing
                .iter()
                .find(|c| c["body"].as_str().is_some_and(|s| s.contains(&tag)))
            {
                if old["body"].as_str() != Some(body.as_str()) {
                    return Err(SdError::conflict(
                        "exported comment was edited; reconcile immutable history",
                    ));
                }
                continue;
            }
            self.client.request(
                "POST",
                &format!("{}/comments", self.path(&number)?),
                &[],
                Some(&json!({"body":body})),
            )?;
        }
        let remote = self.get(&number)?;
        let mut actual_labels = remote.item.labels.clone();
        let mut desired_labels = item.labels.clone();
        actual_labels.sort();
        desired_labels.sort();
        if actual_labels != desired_labels
            || item
                .comments
                .iter()
                .any(|note| !remote.item.comments.contains(note))
            || remote.item.title != item.title
            || remote.item.description != item.description
            || remote.item.status != item.status
            || remote.item.priority != item.priority
            || remote.item.issue_type != item.issue_type
            || remote.item.assignee != item.assignee
        {
            return Err(SdError::refused(
                "GitHub write read-back differs from requested mapping; preserve partial result",
            ));
        }
        Ok(remote)
    }
    fn relate(
        &self,
        remote: &Remote,
        item: &Item,
        targets: &BTreeMap<String, Remote>,
        origin: &str,
    ) -> Result<()> {
        let path = self.path(&remote.id)?;
        let dependency_path = format!("{path}/dependencies/blocked_by");
        let desired = item
            .blocked_on
            .iter()
            .map(|id| {
                targets
                    .get(id)
                    .ok_or_else(|| SdError::refused("dependency target not selected"))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut body = format!("{}\n\n{}", item.description, marker(origin));
        match self.pages(&dependency_path, &[]) {
            Ok(existing) => {
                let wanted = desired
                    .iter()
                    .map(|d| d.link.clone())
                    .collect::<std::collections::BTreeSet<_>>();
                for old in &existing {
                    if !wanted.contains(&required(old, "html_url")?) {
                        let id = old["id"]
                            .as_u64()
                            .ok_or_else(|| SdError::refused("dependency id missing"))?;
                        self.client.request(
                            "DELETE",
                            &format!("{dependency_path}/{id}"),
                            &[],
                            None,
                        )?;
                    }
                }
                for target in &desired {
                    if !existing
                        .iter()
                        .any(|v| v["html_url"].as_str() == Some(&target.link))
                    {
                        let issue =
                            self.client
                                .request("GET", &self.path(&target.id)?, &[], None)?;
                        self.client.request(
                            "POST",
                            &dependency_path,
                            &[],
                            Some(&json!({"issue_id":issue["id"]})),
                        )?;
                    }
                }
            }
            Err(e) if e.message.contains("HTTP 404") || e.message.contains("HTTP 410") => {
                if !desired.is_empty() {
                    body.push_str("\n\n## Seeds dependencies\n");
                    for d in &desired {
                        body.push_str(&format!("- blocked by: {}\n", d.link));
                    }
                }
            }
            Err(e) => return Err(e),
        }
        self.client
            .request("PATCH", &path, &[], Some(&json!({"body":body})))?;
        let actual = self.get(&remote.id)?;
        let wanted_parent = item
            .parent
            .as_ref()
            .map(|id| {
                targets
                    .get(id)
                    .ok_or_else(|| SdError::refused("parent target not selected"))
            })
            .transpose()?;
        if actual.item.parent.as_deref() != wanted_parent.map(|p| p.link.as_str()) {
            let child = self.client.request("GET", &path, &[], None)?;
            if let Some(old) = &actual.item.parent {
                let old = self.get_link(old)?;
                self.client.request(
                    "DELETE",
                    &format!("{}/sub_issue", self.path(&old.id)?),
                    &[],
                    Some(&json!({"sub_issue_id":child["id"]})),
                )?;
            }
            if let Some(parent) = wanted_parent {
                self.client.request(
                    "POST",
                    &format!("{}/sub_issues", self.path(&parent.id)?),
                    &[],
                    Some(&json!({"sub_issue_id":child["id"]})),
                )?;
            }
        }
        let confirmed = self.get(&remote.id)?;
        let mut wanted_dependencies = desired.iter().map(|r| r.link.clone()).collect::<Vec<_>>();
        wanted_dependencies.sort();
        wanted_dependencies.dedup();
        if confirmed.item.parent.as_deref() != wanted_parent.map(|p| p.link.as_str())
            || confirmed.item.blocked_on != wanted_dependencies
        {
            return Err(SdError::refused(
                "GitHub relationship read-back differs; preserve partial result",
            ));
        }
        Ok(())
    }
}
