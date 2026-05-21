use anyhow::{bail, Result};

use crate::store::json_store::JsonTaskStore;

impl JsonTaskStore {
    /// Validate a state transition for a task with the given workflow.
    /// Returns an error if the transition is not allowed.
    pub(crate) async fn validate_state_transition(
        &self,
        current_state: &str,
        new_state: &str,
        workflow_name: &str,
    ) -> Result<()> {
        let workflow = self.get_workflow(workflow_name).await;
        if !workflow.validate_transition(current_state, new_state) {
            bail!(
                "Invalid transition: \"{}\" → \"{}\" is not allowed in workflow \"{}\"",
                current_state,
                new_state,
                workflow.name
            );
        }
        Ok(())
    }
}
