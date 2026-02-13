//! Session manager implementation
//!
//! This module provides the core session management functionality including
//! creation, deletion, querying, and network port associations.

use bevy::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

use super::types::*;

/// Resource that manages all active sessions
#[derive(Resource, Default)]
pub struct SessionManager {
    /// All sessions indexed by ID
    sessions: HashMap<Uuid, Session>,

    /// Mapping from network port to session ID
    port_to_session: HashMap<u16, Uuid>,

    /// Currently active network receivers
    active_receivers: HashMap<u16, bool>,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new network session
    pub fn create_network_session(&mut self, name: &str, port: u16) -> Result<Uuid, SessionError> {
        // Check if port is already in use
        if self.port_to_session.contains_key(&port) {
            return Err(SessionError::PortInUse(port));
        }

        let session = Session::new(
            name.to_string(),
            SessionSource::Network {
                port,
                source_address: None,
            },
        );

        let id = session.id;
        self.sessions.insert(id, session);
        self.port_to_session.insert(port, id);

        debug!(
            "Created network session '{}' (ID: {}) on port {}",
            name, id, port
        );
        Ok(id)
    }

    /// Create a new file session
    pub fn create_file_session(&mut self, name: &str, path: PathBuf) -> Result<Uuid, SessionError> {
        let session = Session::new(name.to_string(), SessionSource::File { path: path.clone() });

        let id = session.id;
        self.sessions.insert(id, session);

        debug!(
            "Created file session '{}' (ID: {}) for path {:?}",
            name, id, path
        );
        Ok(id)
    }

    /// Delete a session
    pub fn delete_session(&mut self, id: Uuid) -> Result<(), SessionError> {
        let session = self
            .sessions
            .remove(&id)
            .ok_or(SessionError::SessionNotFound(id))?;

        // Clean up port mapping if it's a network session
        if let SessionSource::Network { port, .. } = &session.source {
            self.port_to_session.remove(port);
            self.active_receivers.remove(port);
        }

        debug!("Deleted session '{}' (ID: {})", session.name, id);
        Ok(())
    }

    /// Get a session by ID
    pub fn get_session(&self, id: Uuid) -> Option<&Session> {
        self.sessions.get(&id)
    }

    /// Get a mutable session by ID
    pub fn get_session_mut(&mut self, id: Uuid) -> Option<&mut Session> {
        self.sessions.get_mut(&id)
    }

    /// Find session ID by network port
    pub fn find_session_by_port(&self, port: u16) -> Option<Uuid> {
        self.port_to_session.get(&port).copied()
    }

    /// Add a mesh to a session
    pub fn add_mesh_to_session(&mut self, id: Uuid, mesh: Mesh) -> Result<usize, SessionError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(SessionError::SessionNotFound(id))?;

        let frame_index = session.add_frame(mesh);
        Ok(frame_index)
    }

    /// Get all sessions
    pub fn get_all_sessions(&self) -> Vec<&Session> {
        self.sessions.values().collect()
    }

    /// Get sessions matching a filter
    pub fn get_sessions_filtered(&self, filter: &SessionFilter) -> Vec<&Session> {
        self.sessions
            .values()
            .filter(|session| {
                // Filter by source type
                if let Some(source_type) = &filter.source_type {
                    if !source_type.matches(&session.source) {
                        return false;
                    }
                }

                // Filter by name
                if let Some(name_contains) = &filter.name_contains {
                    if !session
                        .name
                        .to_lowercase()
                        .contains(&name_contains.to_lowercase())
                    {
                        return false;
                    }
                }

                // Filter by frame count
                if let Some(min_frames) = filter.min_frames {
                    if session.frame_count() < min_frames {
                        return false;
                    }
                }

                true
            })
            .collect()
    }

    /// Mark a network receiver as active
    pub fn set_receiver_active(&mut self, port: u16, active: bool) {
        self.active_receivers.insert(port, active);
    }

    /// Check if a network receiver is active
    pub fn is_receiver_active(&self, port: u16) -> bool {
        self.active_receivers.get(&port).copied().unwrap_or(false)
    }

    /// Get statistics for a session
    pub fn get_session_stats(&self, id: Uuid) -> Option<SessionStats> {
        self.sessions.get(&id).map(|session| SessionStats {
            id: session.id,
            name: session.name.clone(),
            frame_count: session.frame_count(),
            created_at: session.created_at,
            last_accessed: session.last_accessed,
            source: session.source.display_string(),
            is_active: match &session.source {
                SessionSource::Network { port, .. } => self.is_receiver_active(*port),
                _ => false,
            },
        })
    }

    /// Clear all frames from a session
    pub fn clear_session_frames(&mut self, id: Uuid) -> Result<(), SessionError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(SessionError::SessionNotFound(id))?;

        session.frames.clear();
        session.metadata.frames_received = 0;

        debug!("Cleared all frames from session '{}'", session.name);
        Ok(())
    }
}

/// Statistics for a session
#[derive(Debug, Clone)]
pub struct SessionStats {
    pub id: Uuid,
    pub name: String,
    pub frame_count: usize,
    pub created_at: chrono::DateTime<chrono::Local>,
    pub last_accessed: chrono::DateTime<chrono::Local>,
    pub source: String,
    pub is_active: bool,
}

/// Errors that can occur during session operations
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("Session not found: {0}")]
    SessionNotFound(Uuid),

    #[error("Port {0} is already in use by another session")]
    PortInUse(u16),

    #[error("Invalid session configuration: {0}")]
    InvalidConfiguration(String),

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
}
