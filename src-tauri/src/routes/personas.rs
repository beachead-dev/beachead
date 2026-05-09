use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};

use crate::error::OrchestratorError;
use crate::server::AppState;
use crate::types::{
    CreatePersonaRequest, Persona, PersonaId, UpdatePersonaRequest,
    UpdateResult,
};

/// Build the persona routes sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/personas", get(list_personas).post(create_persona))
        .route(
            "/api/personas/{id}",
            get(get_persona).put(update_persona).delete(delete_persona),
        )
}

/// GET /api/personas — list all personas.
async fn list_personas(
    State(state): State<AppState>,
) -> Result<Json<Vec<Persona>>, OrchestratorError> {
    let personas = state.persona_manager.list()?;
    Ok(Json(personas))
}

/// POST /api/personas — create a new persona.
async fn create_persona(
    State(state): State<AppState>,
    Json(req): Json<CreatePersonaRequest>,
) -> Result<(StatusCode, Json<Persona>), OrchestratorError> {
    let memory_enabled = req.memory_enabled.unwrap_or(false);
    let persona = state.persona_manager.create(req)?;

    // If memory is enabled, create and start an MCP container for this persona
    if memory_enabled {
        if let Some(ref mgr) = state.mcp_container_manager {
            if let Err(e) = mgr.create_container(persona.id.clone()).await {
                // Container creation failed — delete the persona and return error
                let _ = state.persona_manager.delete(&persona.id);
                return Err(OrchestratorError::DockerError(format!(
                    "Failed to create memory container for persona '{}': {}",
                    persona.name, e
                )));
            }
        }
    }

    Ok((StatusCode::CREATED, Json(persona)))
}

/// GET /api/personas/{id} — get a persona by ID.
async fn get_persona(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Persona>, OrchestratorError> {
    let persona = state.persona_manager.get(&PersonaId(id))?;
    Ok(Json(persona))
}

/// PUT /api/personas/{id} — update a persona.
async fn update_persona(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdatePersonaRequest>,
) -> Result<Json<UpdateResult>, OrchestratorError> {
    let persona_id = PersonaId(id);

    // Check if memory_enabled is changing
    let existing = state.persona_manager.get(&persona_id)?;
    let new_memory_enabled = req.memory_enabled.unwrap_or(existing.memory_enabled);
    let memory_was_enabled = existing.memory_enabled;

    let result = state.persona_manager.update(&persona_id, req)?;

    // Handle MCP container lifecycle based on memory_enabled changes
    if let Some(ref mgr) = state.mcp_container_manager {
        if new_memory_enabled && !memory_was_enabled {
            // Memory just enabled — create and start container
            if let Err(e) = mgr.create_container(persona_id.clone()).await {
                // Revert memory_enabled back to false
                let revert_req = UpdatePersonaRequest {
                    name: None,
                    agent_type_id: None,
                    workspace_path: None,
                    memory_enabled: Some(false),
                    agent_cli_args: None,
                    mcp_servers: None,
                };
                let _ = state.persona_manager.update(&persona_id, revert_req);
                return Err(OrchestratorError::DockerError(format!(
                    "Failed to create memory container: {}. Memory has not been enabled.",
                    e
                )));
            }
        } else if !new_memory_enabled && memory_was_enabled {
            // Memory just disabled — remove container
            if let Err(e) = mgr.remove_container(persona_id.clone()).await {
                eprintln!(
                    "Warning: failed to remove MCP container for persona: {}",
                    e
                );
            }
        }
    }

    Ok(Json(result))
}

/// DELETE /api/personas/{id} — delete a persona.
async fn delete_persona(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, OrchestratorError> {
    let persona_id = PersonaId(id);
    let existing = state.persona_manager.get(&persona_id)?;

    state.persona_manager.delete(&persona_id)?;

    // If persona had memory enabled, clean up the MCP container
    if existing.memory_enabled {
        if let Some(ref mgr) = state.mcp_container_manager {
            if let Err(e) = mgr.remove_container(persona_id).await {
                eprintln!("Warning: failed to remove MCP container on persona delete: {}", e);
            }
        }
    }

    Ok(StatusCode::NO_CONTENT)
}
