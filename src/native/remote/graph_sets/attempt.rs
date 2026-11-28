//! One send, then status/seal recovery. A status read never replays bytes.
use super::*;
use crate::backend::WriteBatch;

pub(super) struct Recovery {
    pub same: Option<Value>,
    pub prior: Vec<Value>,
}
impl Mode {
    fn bound(&self, a: &Attempt) -> Result<()> {
        if a.base != self.profile.base
            || a.project != self.profile.project
            || a.store_id != self.profile.store_id
            || a.log_epoch != self.profile.log_epoch
            || a.principal != self.profile.principal
            || a.credential_id != self.profile.credential_id
            || a.credential_sha256 != self.credential_sha256
        {
            return Err(invalid(
                "pending attempt belongs to a different profile or credential; retained",
            ));
        }
        Ok(())
    }
    fn resolve(&self, remote: &RemoteBackend, a: &mut Attempt) -> Result<Value> {
        self.bound(a)?;
        let mut status = self
            .status(remote, a, false)
            .map_err(|_| a.unknown("status_unavailable", None))?;
        if status["state"] == "admitted" {
            // The server serializes sealing with commit; it may find a committed attempt instead.
            status = match self.status(remote, a, true) {
                Ok(s) => s,
                Err(_) => self
                    .status(remote, a, false)
                    .map_err(|_| a.unknown("seal_unconfirmed", None))?,
            };
        }
        let state = status["state"]
            .as_str()
            .ok_or_else(|| invalid("status has no state"))?;
        let tx = status["commit"]["tx_id"].as_u64();
        match state {
            "finished" => {
                let receipt = &status["receipt"];
                let owner = &receipt["owner"];
                if owner["store_id"] != a.store_id
                    || owner["log_epoch"] != a.log_epoch
                    || owner["local_tx"].as_u64().is_none()
                    || owner["local_tx"].as_u64() != tx
                {
                    return Err(a.unknown("invalid_receipt", tx));
                }
                let cut = owner["cut"]
                    .as_str()
                    .ok_or_else(|| a.unknown("receipt_cut_unavailable", tx))?;
                let selected: Vec<String> = self
                    .profile
                    .members()
                    .into_iter()
                    .filter(|g| a.update.contains(&format!("GRAPH <{g}>")))
                    .collect();
                self.check_cut_for(cut, &selected, tx)
                    .map_err(|_| a.unknown("receipt_cut_invalid", tx))?;
                let (changed, _) = super::super::update_report(&receipt.to_string())
                    .ok_or_else(|| a.unknown("receipt_counts_invalid", tx))?;
                // A guarded no-op is not a successful new intent.
                if changed == 0 {
                    return Err(SdError::conflict(
                        "graph-set guarded write did not apply; terminal envelope retained",
                    ));
                }
                a.phase = "finished".into();
                self.journal.persist(a, false)?;
                Ok(
                    json!({"recovered":true,"state":"finished","write_id":a.write_id,
                    "item":a.item_ids.first(),"item_ids":a.item_ids,"owner":owner}),
                )
            }
            "sealed" => {
                if tx.is_some() || status.get("receipt").is_some() {
                    return Err(invalid("sealed status carries commit evidence"));
                }
                a.phase = "sealed".into();
                self.journal.persist(a, false)?;
                Ok(
                    json!({"recovered":true,"state":"sealed","applied":false,"write_id":a.write_id,
                    "item":a.item_ids.first(),"item_ids":a.item_ids,"owner":{"store_id":a.store_id,"log_epoch":a.log_epoch}}),
                )
            }
            "committed" => {
                a.phase = "committed".into();
                self.journal.persist(a, false)?;
                let tx = tx.ok_or_else(|| a.unknown("committed_witness_only", None))?;
                let cut = self
                    .prove(remote, a, tx)
                    .map_err(|_| a.unknown("committed_witness_only", Some(tx)))?;
                Ok(
                    json!({"recovered":true,"state":"committed_proven","write_id":a.write_id,
                    "item":a.item_ids.first(),"item_ids":a.item_ids,
                    "owner":{"store_id":a.store_id,"log_epoch":a.log_epoch,"local_tx":tx,"cut":cut},
                    "custody_retained":true}),
                )
            }
            "unknown" | "admitted" | "stopped" => Err(a.unknown(state, tx)),
            _ => Err(invalid("unrecognized completion state")),
        }
    }
    pub(super) fn recover(
        &self,
        remote: &RemoteBackend,
        intent: &intent::Intent,
    ) -> Result<Recovery> {
        let mut result = Recovery {
            same: None,
            prior: vec![],
        };
        let mut refusal = None;
        for mut a in self.journal.pending()? {
            let same = a.intent_digest == intent.digest;
            let overlaps = intent.targets.iter().any(|id| {
                a.item_ids
                    .iter()
                    .any(|old| old.starts_with(id) || id.starts_with(old))
            });
            match self.resolve(remote, &mut a) {
                Ok(record) => {
                    if same {
                        if result.same.is_some() {
                            return Err(invalid(
                                "multiple completed attempts share one intent; retained",
                            ));
                        }
                        result.same = Some(record);
                    } else {
                        result.prior.push(record);
                    }
                    if matches!(a.phase.as_str(), "finished" | "sealed") {
                        self.journal.remove_terminal(&a)?;
                    }
                }
                Err(error) if same || overlaps => {
                    refusal = Some(error);
                }
                Err(error) => {
                    // Different items may proceed, but custody stays visible and on disk.
                    result.prior.push(json!({"write_id":a.write_id,"item_ids":a.item_ids,"state":"unresolved","details":error.details}));
                }
            }
        }
        if let Some(error) = refusal {
            return Err(error);
        }
        Ok(result)
    }
    pub(in crate::native::remote) fn commit(
        &self,
        remote: &RemoteBackend,
        batch: &WriteBatch,
        update: &str,
        form: &str,
        record: &[(String, Vec<Fact>)],
    ) -> Result<u64> {
        let intent = self.intent.borrow();
        let intent = intent
            .as_ref()
            .ok_or_else(|| invalid("command intent was not prepared"))?;
        intent.check_files()?;
        let item_ids: std::collections::BTreeSet<String> = batch
            .seeds
            .iter()
            .map(|w| w.seed.id.clone())
            .chain(batch.comments.iter().map(|c| c.seed.clone()))
            .chain(batch.delete_seeds.iter().map(|(id, _)| id.clone()))
            .chain(batch.delete_comments.iter().map(|(id, _)| id.clone()))
            .collect();
        let mut a = Attempt {
            schema: ATTEMPT_CONTRACT.into(),
            write_id: new_id()?,
            base: self.profile.base.clone(),
            project: self.profile.project.clone(),
            store_id: self.profile.store_id.clone(),
            log_epoch: self.profile.log_epoch.clone(),
            principal: self.profile.principal.clone(),
            credential_id: self.profile.credential_id.clone(),
            credential_sha256: self.credential_sha256.clone(),
            intent_digest: intent.digest.clone(),
            item_ids: item_ids.into_iter().collect(),
            update: update.into(),
            form: form.into(),
            body_sha256: crate::pendant::sha256(form.as_bytes()),
            phase: "prepared".into(),
            expected: Expected {
                seeds: batch.seeds.iter().map(|w| w.seed.facts()).collect(),
                comments: batch.comments.iter().map(|c| c.facts()).collect(),
                delete_seeds: batch.delete_seeds.clone(),
                delete_comments: batch.delete_comments.clone(),
                record: record.to_vec(),
            },
        };
        self.journal.persist(&a, true)?;
        // Always resolve through authenticated status, including after a lost response.
        let _sent = self.send_attempt(remote, &a);
        let resolved = self.resolve(remote, &mut a)?;
        if resolved["state"] != "finished" && resolved["state"] != "committed_proven" {
            return Err(a.unknown("sealed_not_applied", None));
        }
        let tx = resolved["owner"]["local_tx"]
            .as_u64()
            .ok_or_else(|| a.unknown("missing_owner_tx", None))?;
        if a.phase == "finished" {
            self.journal.remove_terminal(&a)?;
        }
        *self.completed.borrow_mut() = Some(resolved);
        // The Backend scalar is a legacy main-store transaction slot, not an owner tx.
        let _ = tx;
        Ok(0)
    }
}
