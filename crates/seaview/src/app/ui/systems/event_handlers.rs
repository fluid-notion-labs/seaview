//! Event handler systems for UI events
//!
//! This module contains systems that handle UI events and update the application state accordingly.

use bevy::prelude::*;

use crate::app::ui::state::{DeleteSessionEvent, SwitchSessionEvent, UiState};

/// System that handles session switch events
pub fn handle_switch_session_events(
    mut events: MessageReader<SwitchSessionEvent>,
    mut ui_state: ResMut<UiState>,
) {
    for event in events.read() {
        debug!("Switching to session: {:?}", event.session_id);
        ui_state.set_active_session(Some(event.session_id));
    }
}

/// System that handles session deletion events
pub fn handle_delete_session_events(
    mut events: MessageReader<DeleteSessionEvent>,
    mut ui_state: ResMut<UiState>,
) {
    for event in events.read() {
        debug!("Deleting session: {:?}", event.session_id);

        // If we're deleting the active session, clear it
        if ui_state.active_session == Some(event.session_id) {
            ui_state.set_active_session(None);
        }

        // TODO: Actually delete the session from SessionManager when implemented
    }
}

/// System that handles session creation events
pub fn handle_create_session_events(
    mut events: MessageReader<crate::app::ui::state::CreateSessionEvent>,
) {
    for event in events.read() {
        debug!(
            "Creating new session '{}' with source: {:?}",
            event.name, event.source_type
        );

        // TODO: Actually create the session in SessionManager when implemented
    }
}
