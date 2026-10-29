//! Online player listing for the players endpoints.
use omnibedrock_core::error::{ApiError, Result};
use omnibedrock_server::{instances::Instance, server::Status};
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) async fn list_players(svc: &Arc<Instance>) -> Result<Value> {
    if svc.server.snapshot().status != Status::Running {
        return Ok(json!({"players":[],"count":0}));
    }
    let console = &svc.server.console;
    let mut changes = console.changed.subscribe();
    let mut cursor = Some(console.replay(None, 0).latest);
    svc.server.send_command("list").await?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let batch = console.replay(cursor, 256);
        if batch.gap.is_some() {
            return Err(ApiError::conflict(
                "Player response exceeded console retention",
            ));
        }
        for line in batch.lines {
            cursor = Some(line.sequence);
            if line.line.to_lowercase().contains("players online") {
                let names = line
                    .line
                    .rsplit_once(':')
                    .map(|(_, names)| {
                        names
                            .split(',')
                            .map(str::trim)
                            .filter(|n| !n.is_empty())
                            .map(|name| json!({"name":name}))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                return Ok(json!({"count":names.len(),"players":names}));
            }
        }
        if tokio::time::timeout_at(deadline, changes.changed())
            .await
            .is_err()
        {
            return Ok(json!({"players":[],"count":0}));
        }
    }
}
