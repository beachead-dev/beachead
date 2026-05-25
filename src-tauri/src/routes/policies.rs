use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, put},
    Json, Router,
};
use serde::Deserialize;

use crate::error::OrchestratorError;
use crate::sbx::{PolicyDefault, PolicyLogEntry, PolicyState};
use crate::server::AppState;
use crate::types::{AddPolicyRuleRequest, SetDefaultPolicyRequest};

/// Build the policy routes sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/policies", get(get_policies))
        .route("/api/policies/default", put(set_default_policy))
        .route("/api/policies/rules", axum::routing::post(add_rule))
        .route(
            "/api/policies/rules/{id}",
            axum::routing::delete(remove_rule),
        )
        .route("/api/policies/log", get(get_policy_log))
}

/// GET /api/policies — get current policy state.
async fn get_policies(
    State(state): State<AppState>,
) -> Result<Json<PolicyState>, OrchestratorError> {
    let mgr = state.require_policy_manager()?;
    let policy_state = mgr.get_state().await?;
    Ok(Json(policy_state))
}

/// PUT /api/policies/default — set the default policy mode.
async fn set_default_policy(
    State(state): State<AppState>,
    Json(req): Json<SetDefaultPolicyRequest>,
) -> Result<StatusCode, OrchestratorError> {
    let mgr = state.require_policy_manager()?;
    let mode = match req.mode.as_str() {
        "allow" => PolicyDefault::Allow,
        "deny" => PolicyDefault::Deny,
        "balanced" => PolicyDefault::Balanced,
        other => {
            return Err(OrchestratorError::Validation(format!(
                "Invalid policy mode '{}': must be 'allow', 'deny', or 'balanced'",
                other
            )));
        }
    };
    mgr.set_default(mode).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/policies/rules — add a network policy rule.
async fn add_rule(
    State(state): State<AppState>,
    Json(req): Json<AddPolicyRuleRequest>,
) -> Result<StatusCode, OrchestratorError> {
    let mgr = state.require_policy_manager()?;
    mgr.add_rule(&req.action, &req.target).await?;
    Ok(StatusCode::CREATED)
}

/// DELETE /api/policies/rules/{id} — remove a policy rule by ID.
async fn remove_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, OrchestratorError> {
    let mgr = state.require_policy_manager()?;
    mgr.remove_rule(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Query parameters for the policy log endpoint.
#[derive(Debug, Deserialize)]
struct PolicyLogQuery {
    sandbox_id: Option<String>,
    limit: Option<u32>,
}

/// GET /api/policies/log — get the policy traffic log.
async fn get_policy_log(
    State(state): State<AppState>,
    Query(query): Query<PolicyLogQuery>,
) -> Result<Json<Vec<PolicyLogEntry>>, OrchestratorError> {
    let mgr = state.require_policy_manager()?;
    let entries = mgr
        .get_log(query.sandbox_id.as_deref(), query.limit)
        .await?;
    Ok(Json(entries))
}
