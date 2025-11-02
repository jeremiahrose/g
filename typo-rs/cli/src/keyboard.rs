//! Global keyboard listener for macOS

use crate::app::{App, ApprovalDecision};
use anyhow::Result;
use rdev::{listen, Event, EventType, Key};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Keyboard listener state
struct KeyboardState {
    shift_pressed: Arc<RwLock<bool>>,
}

pub struct KeyboardListener;

impl KeyboardListener {
    pub async fn start(app: Arc<App>) -> Result<()> {
        let state = Arc::new(KeyboardState {
            shift_pressed: Arc::new(RwLock::new(false)),
        });

        tracing::debug!(
            "Global keyboard listener started (Right Cmd=approve, Right Option=reject, Shift+Cmd=always allow)"
        );

        // Listen for keyboard events
        tokio::task::spawn_blocking(move || {
            if let Err(e) = listen(move |event: Event| {
                let app = Arc::clone(&app);
                let state = Arc::clone(&state);

                tokio::spawn(async move {
                    if let Err(e) = handle_key_event(event, app, state).await {
                        tracing::debug!("Keyboard event error: {}", e);
                    }
                });
            }) {
                tracing::error!("Keyboard listener error: {}", e);
            }
        });

        Ok(())
    }
}

async fn handle_key_event(event: Event, app: Arc<App>, state: Arc<KeyboardState>) -> Result<()> {
    match event.event_type {
        EventType::KeyPress(key) => {
            // Track shift state
            if key == Key::ShiftRight {
                let mut shift = state.shift_pressed.write().await;
                *shift = true;
            }

            // Check for pending approval
            let pending = app.pending_approval.read().await;
            if let Some((_, _, ref tx)) = *pending {
                let shift_pressed = *state.shift_pressed.read().await;

                let decision = match key {
                    Key::MetaRight => {
                        // Right Command
                        if shift_pressed {
                            Some(ApprovalDecision::AlwaysAllow)
                        } else {
                            Some(ApprovalDecision::ApproveOnce)
                        }
                    }
                    Key::AltRight => {
                        // Right Option
                        Some(ApprovalDecision::RejectOnce)
                    }
                    _ => None,
                };

                if let Some(decision) = decision {
                    let _ = tx.send(decision).await;
                }
            }
        }
        EventType::KeyRelease(key) => {
            // Track shift state
            if key == Key::ShiftRight {
                let mut shift = state.shift_pressed.write().await;
                *shift = false;
            }
        }
        _ => {}
    }

    Ok(())
}
