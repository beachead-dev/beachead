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
    let persona = state.persona_manager.create(req)?;
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
    let result = state.persona_manager.update(&PersonaId(id), req)?;
    Ok(Json(result))
}

/// DELETE /api/personas/{id} — delete a persona.
async fn delete_persona(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, OrchestratorError> {
    state.persona_manager.delete(&PersonaId(id))?;
    Ok(StatusCode::NO_CONTENT)
}
