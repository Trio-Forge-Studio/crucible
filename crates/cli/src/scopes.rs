//! The scope check before the first call of `crucible provision` and `crucible deploy`. It is advisory: when Roblox
//! does not answer the introspection within `LIMIT`, the run goes on, and a call that lacks a scope fails as it would
//! have. `--no-scope-check` skips it.

use crate::output::Output;
use crucible_oc::Scope;
use crucible_oc::auth::Introspection;
use crucible_oc::ops::api_keys;
use crucible_oc::outcome::Failure;
use crucible_oc::transport::Transport;
use serde_json::json;
use std::time::Duration;

/// How long the introspection may take, retries and waits included, before the run goes on without it.
pub const LIMIT: Duration = Duration::from_secs(10);

/// What the key can do, or `None` when Roblox did not say. A disabled or expired key fails with exit 3.
pub fn introspect(transport: &Transport<'_>, output: &Output) -> Result<Option<Introspection>, Failure> {
    match transport.run(&api_keys::introspect().deadline(transport.clock().now() + LIMIT)) {
        Ok(key) if !key.enabled || key.expired => Err(Failure::new(3, "auth", "the key is disabled or expired")),
        Ok(key) => Ok(Some(key)),
        Err(failure) => {
            let message = failure.error["message"].as_str().unwrap_or_default();
            output.note(&format!(
                "Could not check the key's scopes within {} s: {message}. Going on; a call that lacks a scope fails \
                 by itself.",
                LIMIT.as_secs()
            ));
            Ok(None)
        }
    }
}

/// Fails with exit 3 and `error.missingScopes` when the key lacks a scope in `needed`.
pub fn require(key: Option<&Introspection>, needed: &[Scope]) -> Result<(), Failure> {
    let Some(key) = key else { return Ok(()) };
    let missing = key.missing(needed);
    if missing.is_empty() {
        return Ok(());
    }
    Err(Failure::new(
        3,
        "auth",
        format!(
            "the key lacks {}, which this run needs. Add them to the key on the Creator Dashboard, or run with a key \
             that has them. If you know the key can make these calls, rerun with --no-scope-check",
            missing.join(", ")
        ),
    )
    .with("missingScopes", json!(missing)))
}
