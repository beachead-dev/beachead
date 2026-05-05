use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};

use crate::error::OrchestratorError;
use crate::server::AppState;
use crate::types::{AgentType, AgentTypeId, CreateAgentRequest, UpdateAgentRequest};

/// Build the agent routes sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/agents", get(list_agents).post(create_agent))
        .route(
            "/api/agents/{id}",
            get(get_agent).put(update_agent).delete(delete_agent),
        )
}

/// GET /api/agents — list all agent types.
async fn list_agents(
    State(state): State<AppState>,
) -> Result<Json<Vec<AgentType>>, OrchestratorError> {
    let agents = state.agent_manager.list()?;
    Ok(Json(agents))
}

/// POST /api/agents — create a custom agent type.
async fn create_agent(
    State(state): State<AppState>,
    Json(req): Json<CreateAgentRequest>,
) -> Result<(StatusCode, Json<AgentType>), OrchestratorError> {
    let agent = state.agent_manager.create(req).await?;
    Ok((StatusCode::CREATED, Json(agent)))
}

/// GET /api/agents/{id} — get an agent type by ID.
async fn get_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AgentType>, OrchestratorError> {
    let agent = state.agent_manager.get(&AgentTypeId(id))?;
    Ok(Json(agent))
}

/// PUT /api/agents/{id} — update a custom agent type.
async fn update_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAgentRequest>,
) -> Result<Json<AgentType>, OrchestratorError> {
    let agent = state.agent_manager.update(&AgentTypeId(id), req).await?;
    Ok(Json(agent))
}

/// DELETE /api/agents/{id} — delete a custom agent type.
async fn delete_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, OrchestratorError> {
    state.agent_manager.delete(&AgentTypeId(id))?;
    Ok(StatusCode::NO_CONTENT)
}
