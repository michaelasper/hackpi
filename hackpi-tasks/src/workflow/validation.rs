use anyhow::{bail, Result};
use std::collections::HashSet;

use crate::workflow::profile::WorkflowProfile;

// ── WorkflowProfile: validation methods ────────────────────────────────────

impl WorkflowProfile {
    /// Check whether transitioning from `from_state` to `to_state` is allowed
    /// by this workflow's transition rules.
    pub fn validate_transition(&self, from_state: &str, to_state: &str) -> bool {
        // If from == to, allow (no-op transition)
        if from_state == to_state {
            return true;
        }

        self.transitions
            .iter()
            .any(|t| t.from == from_state && t.to.iter().any(|target| target == to_state))
    }

    /// Validate the structural integrity of this workflow profile.
    ///
    /// Checks:
    /// - No duplicate states
    /// - All transition source states exist in the states list
    /// - All transition target states exist in the states list
    /// - No orphan states (states that never appear in any transition)
    pub fn validate(&self) -> Result<()> {
        // Check for duplicate states
        let mut seen = HashSet::new();
        for state in &self.states {
            if !seen.insert(state.as_str()) {
                bail!("duplicate state: \"{state}\"");
            }
        }

        // Check transition sources and targets
        let state_set: HashSet<&str> = self.states.iter().map(|s| s.as_str()).collect();

        for (i, transition) in self.transitions.iter().enumerate() {
            // Source state must exist
            if !state_set.contains(transition.from.as_str()) {
                bail!(
                    "transition {}: source state \"{}\" not found in states list",
                    i,
                    transition.from
                );
            }

            // Target states must exist
            if transition.to.is_empty() {
                bail!(
                    "transition {}: from \"{}\" has no target states",
                    i,
                    transition.from
                );
            }

            for target in &transition.to {
                if !state_set.contains(target.as_str()) {
                    bail!(
                        "transition {}: target state \"{}\" not found in states list",
                        i,
                        target
                    );
                }
            }
        }

        // Check for orphan states (states not in any transition as source or target)
        let mut referenced_states: HashSet<&str> = HashSet::new();
        for transition in &self.transitions {
            referenced_states.insert(transition.from.as_str());
            for target in &transition.to {
                referenced_states.insert(target.as_str());
            }
        }

        for state in &self.states {
            if !referenced_states.contains(state.as_str()) {
                bail!("orphan state: \"{state}\" is not referenced in any transition");
            }
        }

        Ok(())
    }
}
