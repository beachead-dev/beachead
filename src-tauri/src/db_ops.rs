//! Database CRUD operations for all domain types.
//! These functions provide the data access layer used by managers.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};

use crate::error::OrchestratorError;
use crate::types::{
    AgentMetadata, AgentType, AgentTypeId, Persona, PersonaId, PersonaMcpServer, Session,
    SessionId, SessionStatus,
};

// --- Agent Type Operations ---

pub fn insert_agent_type(conn: &Connection, agent: &AgentType) -> Result<(), OrchestratorError> {
    let metadata_json =
        serde_json::to_string(&agent.metadata).map_err(|e| OrchestratorError::Internal(e.to_string()))?;

    conn.execute(
        "INSERT INTO agent_types (id, name, sbx_agent, kit_ref, is_builtin, metadata, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            agent.id.0,
            agent.name,
            agent.sbx_agent,
            agent.kit_ref,
            agent.is_builtin as i32,
            metadata_json,
            agent.created_at.to_rfc3339(),
            agent.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

pub fn get_agent_type(conn: &Connection, id: &AgentTypeId) -> Result<AgentType, OrchestratorError> {
    conn.query_row(
        "SELECT id, name, sbx_agent, kit_ref, is_builtin, metadata, created_at, updated_at
         FROM agent_types WHERE id = ?1",
        params![id.0],
        |row| {
            let metadata_str: String = row.get(5)?;
            let created_str: String = row.get(6)?;
            let updated_str: String = row.get(7)?;

            Ok(AgentType {
                id: AgentTypeId(row.get(0)?),
                name: row.get(1)?,
                sbx_agent: row.get(2)?,
                kit_ref: row.get(3)?,
                is_builtin: row.get::<_, i32>(4)? != 0,
                metadata: serde_json::from_str(&metadata_str).unwrap_or(AgentMetadata {
                    required_secrets: vec![],
                    auth_methods: vec![],
                    description: String::new(),
                    supports_interactive_auth: false,
                    mcp_config_path: None,
                }),
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .unwrap()
                    .with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            OrchestratorError::NotFound(format!("Agent type not found: {}", id.0))
        }
        other => OrchestratorError::Database(other.to_string()),
    })
}

pub fn update_agent_type(
    conn: &Connection,
    id: &AgentTypeId,
    name: &str,
    kit_ref: Option<&str>,
    metadata: &AgentMetadata,
    updated_at: &DateTime<Utc>,
) -> Result<(), OrchestratorError> {
    let metadata_json =
        serde_json::to_string(metadata).map_err(|e| OrchestratorError::Internal(e.to_string()))?;

    let rows = conn.execute(
        "UPDATE agent_types SET name = ?1, kit_ref = ?2, metadata = ?3, updated_at = ?4
         WHERE id = ?5",
        params![name, kit_ref, metadata_json, updated_at.to_rfc3339(), id.0],
    )?;

    if rows == 0 {
        return Err(OrchestratorError::NotFound(format!(
            "Agent type not found: {}",
            id.0
        )));
    }
    Ok(())
}

pub fn delete_agent_type(conn: &Connection, id: &AgentTypeId) -> Result<(), OrchestratorError> {
    let rows = conn.execute("DELETE FROM agent_types WHERE id = ?1", params![id.0])?;
    if rows == 0 {
        return Err(OrchestratorError::NotFound(format!(
            "Agent type not found: {}",
            id.0
        )));
    }
    Ok(())
}

pub fn count_personas_by_agent_type(
    conn: &Connection,
    agent_type_id: &AgentTypeId,
) -> Result<i64, OrchestratorError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM personas WHERE agent_type_id = ?1",
            params![agent_type_id.0],
            |row| row.get(0),
        )
        .map_err(|e| OrchestratorError::Database(e.to_string()))?;
    Ok(count)
}

pub fn agent_type_name_exists(
    conn: &Connection,
    name: &str,
    exclude_id: Option<&AgentTypeId>,
) -> Result<bool, OrchestratorError> {
    let exists: bool = match exclude_id {
        Some(id) => conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM agent_types WHERE name = ?1 AND id != ?2",
                params![name, id.0],
                |row| row.get(0),
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?,
        None => conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM agent_types WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?,
    };
    Ok(exists)
}

pub fn list_agent_types(conn: &Connection) -> Result<Vec<AgentType>, OrchestratorError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, sbx_agent, kit_ref, is_builtin, metadata, created_at, updated_at
         FROM agent_types ORDER BY name",
    )?;

    let agents = stmt
        .query_map([], |row| {
            let metadata_str: String = row.get(5)?;
            let created_str: String = row.get(6)?;
            let updated_str: String = row.get(7)?;

            Ok(AgentType {
                id: AgentTypeId(row.get(0)?),
                name: row.get(1)?,
                sbx_agent: row.get(2)?,
                kit_ref: row.get(3)?,
                is_builtin: row.get::<_, i32>(4)? != 0,
                metadata: serde_json::from_str(&metadata_str).unwrap_or(AgentMetadata {
                    required_secrets: vec![],
                    auth_methods: vec![],
                    description: String::new(),
                    supports_interactive_auth: false,
                    mcp_config_path: None,
                }),
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .unwrap()
                    .with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(agents)
}

// --- Persona Operations ---

pub fn insert_persona(conn: &Connection, persona: &Persona) -> Result<(), OrchestratorError> {
    let cli_args_json = serde_json::to_string(&persona.agent_cli_args)
        .map_err(|e| OrchestratorError::Internal(e.to_string()))?;

    conn.execute(
        "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, agent_cli_args, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            persona.id.0,
            persona.name,
            persona.agent_type_id.0,
            persona.workspace_path.to_string_lossy().to_string(),
            persona.memory_enabled as i32,
            cli_args_json,
            persona.created_at.to_rfc3339(),
            persona.updated_at.to_rfc3339(),
        ],
    )?;

    // Insert MCP server entries
    for mcp in &persona.mcp_servers {
        insert_persona_mcp_server(conn, mcp)?;
    }

    Ok(())
}

pub fn get_persona(conn: &Connection, id: &PersonaId) -> Result<Persona, OrchestratorError> {
    let persona = conn.query_row(
        "SELECT id, name, agent_type_id, workspace_path, memory_enabled, agent_cli_args, created_at, updated_at
         FROM personas WHERE id = ?1",
        params![id.0],
        |row| {
            let cli_args_str: Option<String> = row.get(5)?;
            let created_str: String = row.get(6)?;
            let updated_str: String = row.get(7)?;

            Ok(Persona {
                id: PersonaId(row.get(0)?),
                name: row.get(1)?,
                agent_type_id: AgentTypeId(row.get(2)?),
                workspace_path: std::path::PathBuf::from(row.get::<_, String>(3)?),
                memory_enabled: row.get::<_, i32>(4)? != 0,
                agent_cli_args: cli_args_str
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default(),
                mcp_servers: vec![], // filled below
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .unwrap()
                    .with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            OrchestratorError::NotFound(format!("Persona not found: {}", id.0))
        }
        other => OrchestratorError::Database(other.to_string()),
    })?;

    // Load MCP servers
    let mcp_servers = list_persona_mcp_servers(conn, &persona.id)?;

    Ok(Persona {
        mcp_servers,
        ..persona
    })
}

pub fn list_personas(conn: &Connection) -> Result<Vec<Persona>, OrchestratorError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, agent_type_id, workspace_path, memory_enabled, agent_cli_args, created_at, updated_at
         FROM personas ORDER BY name",
    )?;

    let personas = stmt
        .query_map([], |row| {
            let cli_args_str: Option<String> = row.get(5)?;
            let created_str: String = row.get(6)?;
            let updated_str: String = row.get(7)?;

            Ok(Persona {
                id: PersonaId(row.get(0)?),
                name: row.get(1)?,
                agent_type_id: AgentTypeId(row.get(2)?),
                workspace_path: std::path::PathBuf::from(row.get::<_, String>(3)?),
                memory_enabled: row.get::<_, i32>(4)? != 0,
                agent_cli_args: cli_args_str
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default(),
                mcp_servers: vec![],
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .unwrap()
                    .with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // Load MCP servers for each persona
    let mut result = Vec::with_capacity(personas.len());
    for persona in personas {
        let mcp_servers = list_persona_mcp_servers(conn, &persona.id)?;
        result.push(Persona {
            mcp_servers,
            ..persona
        });
    }

    Ok(result)
}

// --- Persona MCP Server Operations ---

pub fn insert_persona_mcp_server(
    conn: &Connection,
    mcp: &PersonaMcpServer,
) -> Result<(), OrchestratorError> {
    let auth_headers_json = mcp
        .auth_headers
        .as_ref()
        .map(|h| serde_json::to_string(h).unwrap_or_default());

    conn.execute(
        "INSERT INTO persona_mcp_servers (id, persona_id, name, url, description, auth_headers, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            mcp.id,
            mcp.persona_id.0,
            mcp.name,
            mcp.url,
            mcp.description,
            auth_headers_json,
            mcp.created_at.to_rfc3339(),
            mcp.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

pub fn get_persona_mcp_server(
    conn: &Connection,
    id: &str,
) -> Result<PersonaMcpServer, OrchestratorError> {
    conn.query_row(
        "SELECT id, persona_id, name, url, description, auth_headers, created_at, updated_at
         FROM persona_mcp_servers WHERE id = ?1",
        params![id],
        |row| {
            let auth_str: Option<String> = row.get(5)?;
            let created_str: String = row.get(6)?;
            let updated_str: String = row.get(7)?;

            Ok(PersonaMcpServer {
                id: row.get(0)?,
                persona_id: PersonaId(row.get(1)?),
                name: row.get(2)?,
                url: row.get(3)?,
                description: row.get(4)?,
                auth_headers: auth_str.and_then(|s| serde_json::from_str(&s).ok()),
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .unwrap()
                    .with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            OrchestratorError::NotFound(format!("MCP server entry not found: {}", id))
        }
        other => OrchestratorError::Database(other.to_string()),
    })
}

pub fn update_persona_mcp_server(
    conn: &Connection,
    id: &str,
    name: &str,
    url: &str,
    description: Option<&str>,
    auth_headers: Option<&serde_json::Value>,
    updated_at: &DateTime<Utc>,
) -> Result<(), OrchestratorError> {
    let auth_headers_json = auth_headers.map(|h| serde_json::to_string(h).unwrap_or_default());

    let rows = conn.execute(
        "UPDATE persona_mcp_servers SET name = ?1, url = ?2, description = ?3, auth_headers = ?4, updated_at = ?5
         WHERE id = ?6",
        params![
            name,
            url,
            description,
            auth_headers_json,
            updated_at.to_rfc3339(),
            id,
        ],
    )?;

    if rows == 0 {
        return Err(OrchestratorError::NotFound(format!(
            "MCP server entry not found: {}",
            id
        )));
    }
    Ok(())
}

pub fn delete_persona_mcp_server(conn: &Connection, id: &str) -> Result<(), OrchestratorError> {
    let rows = conn.execute("DELETE FROM persona_mcp_servers WHERE id = ?1", params![id])?;
    if rows == 0 {
        return Err(OrchestratorError::NotFound(format!(
            "MCP server entry not found: {}",
            id
        )));
    }
    Ok(())
}

pub fn list_persona_mcp_servers(
    conn: &Connection,
    persona_id: &PersonaId,
) -> Result<Vec<PersonaMcpServer>, OrchestratorError> {
    let mut stmt = conn.prepare(
        "SELECT id, persona_id, name, url, description, auth_headers, created_at, updated_at
         FROM persona_mcp_servers WHERE persona_id = ?1 ORDER BY name",
    )?;

    let servers = stmt
        .query_map(params![persona_id.0], |row| {
            let auth_str: Option<String> = row.get(5)?;
            let created_str: String = row.get(6)?;
            let updated_str: String = row.get(7)?;

            Ok(PersonaMcpServer {
                id: row.get(0)?,
                persona_id: PersonaId(row.get(1)?),
                name: row.get(2)?,
                url: row.get(3)?,
                description: row.get(4)?,
                auth_headers: auth_str.and_then(|s| serde_json::from_str(&s).ok()),
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .unwrap()
                    .with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(servers)
}

// --- Persona Helper Operations ---

pub fn persona_name_exists(
    conn: &Connection,
    name: &str,
    exclude_id: Option<&PersonaId>,
) -> Result<bool, OrchestratorError> {
    let exists: bool = match exclude_id {
        Some(id) => conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM personas WHERE name = ?1 AND id != ?2",
                params![name, id.0],
                |row| row.get(0),
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?,
        None => conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM personas WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?,
    };
    Ok(exists)
}

pub fn count_active_sessions_for_persona(
    conn: &Connection,
    persona_id: &PersonaId,
) -> Result<i64, OrchestratorError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE persona_id = ?1 AND status IN ('running', 'starting')",
            params![persona_id.0],
            |row| row.get(0),
        )
        .map_err(|e| OrchestratorError::Database(e.to_string()))?;
    Ok(count)
}

// --- Session Operations ---

pub fn insert_session(conn: &Connection, session: &Session) -> Result<(), OrchestratorError> {
    conn.execute(
        "INSERT INTO sessions (id, persona_id, sandbox_id, kit_path, status, error_message, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            session.id.0,
            session.persona_id.0,
            session.sandbox_id,
            session.kit_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            session.status.to_string(),
            session.error_message,
            session.created_at.to_rfc3339(),
            session.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

pub fn get_session(conn: &Connection, id: &SessionId) -> Result<Session, OrchestratorError> {
    conn.query_row(
        "SELECT id, persona_id, sandbox_id, kit_path, status, error_message, created_at, updated_at
         FROM sessions WHERE id = ?1",
        params![id.0],
        |row| {
            let kit_path_str: Option<String> = row.get(3)?;
            let status_str: String = row.get(4)?;
            let created_str: String = row.get(6)?;
            let updated_str: String = row.get(7)?;

            Ok(Session {
                id: SessionId(row.get(0)?),
                persona_id: PersonaId(row.get(1)?),
                sandbox_id: row.get(2)?,
                kit_path: kit_path_str.map(std::path::PathBuf::from),
                status: status_str.parse().unwrap_or(SessionStatus::Failed),
                error_message: row.get(5)?,
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .unwrap()
                    .with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            OrchestratorError::NotFound(format!("Session not found: {}", id.0))
        }
        other => OrchestratorError::Database(other.to_string()),
    })
}

pub fn update_session_status(
    conn: &Connection,
    id: &SessionId,
    status: &SessionStatus,
    error_message: Option<&str>,
) -> Result<(), OrchestratorError> {
    let now = Utc::now();
    let rows = conn.execute(
        "UPDATE sessions SET status = ?1, error_message = ?2, updated_at = ?3 WHERE id = ?4",
        params![status.to_string(), error_message, now.to_rfc3339(), id.0],
    )?;
    if rows == 0 {
        return Err(OrchestratorError::NotFound(format!(
            "Session not found: {}",
            id.0
        )));
    }
    Ok(())
}

pub fn update_session_sandbox_id(
    conn: &Connection,
    id: &SessionId,
    sandbox_id: &str,
) -> Result<(), OrchestratorError> {
    let now = Utc::now();
    let rows = conn.execute(
        "UPDATE sessions SET sandbox_id = ?1, updated_at = ?2 WHERE id = ?3",
        params![sandbox_id, now.to_rfc3339(), id.0],
    )?;
    if rows == 0 {
        return Err(OrchestratorError::NotFound(format!(
            "Session not found: {}",
            id.0
        )));
    }
    Ok(())
}

/// Query sessions with status "running" or "starting" (used for recovery on startup).
pub fn list_active_sessions(conn: &Connection) -> Result<Vec<Session>, OrchestratorError> {
    let mut stmt = conn.prepare(
        "SELECT id, persona_id, sandbox_id, kit_path, status, error_message, created_at, updated_at
         FROM sessions WHERE status IN ('running', 'starting') ORDER BY created_at DESC",
    )?;

    let sessions = stmt
        .query_map([], |row| {
            let kit_path_str: Option<String> = row.get(3)?;
            let status_str: String = row.get(4)?;
            let created_str: String = row.get(6)?;
            let updated_str: String = row.get(7)?;

            Ok(Session {
                id: SessionId(row.get(0)?),
                persona_id: PersonaId(row.get(1)?),
                sandbox_id: row.get(2)?,
                kit_path: kit_path_str.map(std::path::PathBuf::from),
                status: status_str.parse().unwrap_or(SessionStatus::Failed),
                error_message: row.get(5)?,
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .unwrap()
                    .with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(sessions)
}

pub fn list_sessions(conn: &Connection) -> Result<Vec<Session>, OrchestratorError> {
    let mut stmt = conn.prepare(
        "SELECT id, persona_id, sandbox_id, kit_path, status, error_message, created_at, updated_at
         FROM sessions ORDER BY created_at DESC",
    )?;

    let sessions = stmt
        .query_map([], |row| {
            let kit_path_str: Option<String> = row.get(3)?;
            let status_str: String = row.get(4)?;
            let created_str: String = row.get(6)?;
            let updated_str: String = row.get(7)?;

            Ok(Session {
                id: SessionId(row.get(0)?),
                persona_id: PersonaId(row.get(1)?),
                sandbox_id: row.get(2)?,
                kit_path: kit_path_str.map(std::path::PathBuf::from),
                status: status_str.parse().unwrap_or(SessionStatus::Failed),
                error_message: row.get(5)?,
                created_at: DateTime::parse_from_rfc3339(&created_str)
                    .unwrap()
                    .with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&updated_str)
                    .unwrap()
                    .with_timezone(&Utc),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(sessions)
}
