use super::*;

impl FakeRoute {
    pub(super) fn fork_parent(
        &self,
        state: &State,
        request: &AccountForRequest,
        catalog: &Catalog,
        now: u64,
    ) -> Result<Option<usize>, Refusal> {
        let Some(fork) = &request.fork_of else {
            return Ok(None);
        };
        let invalid = || Refusal::invalid_request("fork_of", "fork parent is not available");
        let key = (self.principal().to_owned(), fork.admission_id.clone());
        let Some((parent_request, parent)) = state.admissions.get(&key) else {
            let mut refusal = invalid();
            if self.fault("fork_of_private_parent")
                && state
                    .admissions
                    .keys()
                    .any(|(_, id)| id == &fork.admission_id)
            {
                refusal
                    .detail
                    .insert("reason".into(), json!("another_principal"));
            }
            return Err(refusal);
        };
        let lapse = if state.started_admissions.contains(&key) {
            parent.pin.lease_valid_until_ms
        } else {
            parent.pin.admission_valid_until_ms
        };
        if parent_request.provider != request.provider || now > lapse.saturating_add(3_600_000) {
            return Err(invalid());
        }
        let index = catalog
            .accounts
            .iter()
            .position(|account| {
                account.account_id == parent.account.account_id
                    && account.credential_id == parent.account.credential_id
            })
            .ok_or_else(invalid)?;
        let account = &catalog.accounts[index];
        let block = state
            .blocks
            .get(&account.account_id)
            .filter(|retry| retry.is_none_or(|time| time > now));
        let pair = (account.account_id.clone(), account.credential_id.clone());
        let unavailable = block.is_some()
            || state
                .signed_out_credentials
                .contains(&account.credential_id)
            || state.observe_block(&pair, &request.pool).is_some();
        let move_blocked_parent =
            self.fault("fork_of_blocked_parent") && block.is_some_and(|retry| retry.is_some());
        if unavailable && !move_blocked_parent {
            let recheck = now + 900_000;
            let retry = block.copied().flatten().unwrap_or(recheck).min(recheck);
            let retry = if self.fault("fork_of_unknown_retry")
                && block.is_some_and(|retry| retry.is_none())
            {
                now + 1_800_000
            } else {
                retry
            };
            let mut refusal = Refusal::account_hold(retry);
            refusal
                .detail
                .insert("reason".into(), json!("fork_parent_account_unavailable"));
            return Err(refusal);
        }
        Ok(Some(index))
    }
}
