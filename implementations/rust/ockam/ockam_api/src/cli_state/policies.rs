use crate::cli_state::{AutoRetry, CliState};
use ockam_abac::{Policies, ResourcePolicySqlxDatabase, ResourceTypePolicySqlxDatabase};
use std::sync::Arc;

impl CliState {
    pub fn policies(&self, node_name: &str) -> Policies {
        Policies::new(
            Arc::new(AutoRetry::new(ResourcePolicySqlxDatabase::new(
                self.database(),
                node_name,
            ))),
            Arc::new(AutoRetry::new(ResourceTypePolicySqlxDatabase::new(
                self.database(),
                node_name,
            ))),
        )
    }
}
