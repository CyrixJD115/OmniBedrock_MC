use crate::{api::AppState, auth::User};
use axum::{
    extract::{
        Query, State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use omnibedrock_core::error::{ApiError, Result};
use omnibedrock_server::server::Console;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Deserialize, Clone)]
pub struct WsQuery {
    #[serde(default)]
    pub token: String,
    pub after: Option<u64>,
    pub stream_id: Option<Uuid>,
}
async fn user(state: &AppState, q: &WsQuery, permission: &str) -> Result<User> {
    let user = state.auth.resolve_user(Some(&q.token)).await?;
    state.auth.require(&user, permission).await?;
    Ok(user)
}
pub async fn console_upgrade(
    State(state): State<AppState>,
    Query(q): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    user(&state, &q, "CONSOLE_VIEW").await?;
    Ok(ws
        .max_message_size(16 * 1024)
        .on_upgrade(move |ws| console_socket(state, q, ws)))
}
pub async fn metrics_upgrade(
    State(state): State<AppState>,
    Query(q): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    user(&state, &q, "SERVER_VIEW").await?;
    Ok(ws
        .max_message_size(16 * 1024)
        .on_upgrade(move |ws| metrics_socket(state, q, ws)))
}
pub async fn backups_upgrade(
    State(state): State<AppState>,
    Query(q): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    user(&state, &q, "BACKUPS_VIEW").await?;
    Ok(ws
        .max_message_size(16 * 1024)
        .on_upgrade(move |ws| backup_socket(state, q, ws)))
}
pub async fn logs_upgrade(
    State(state): State<AppState>,
    Query(q): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    user(&state, &q, "AUDIT_VIEW").await?;
    Ok(ws
        .max_message_size(16 * 1024)
        .on_upgrade(move |ws| logs_socket(state, q, ws)))
}
pub async fn broadcaster_upgrade(
    State(state): State<AppState>,
    Query(q): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    user(&state, &q, "BROADCASTER_VIEW").await?;
    Ok(ws
        .max_message_size(16 * 1024)
        .on_upgrade(move |ws| broadcaster_socket(state, q, ws)))
}
type Writer = futures_util::stream::SplitSink<WebSocket, Message>;
async fn send(writer: &mut Writer, value: &impl serde::Serialize) -> anyhow::Result<()> {
    let data = serde_json::to_string(value)?;
    tokio::time::timeout(
        Duration::from_secs(10),
        writer.send(Message::Text(data.into())),
    )
    .await??;
    Ok(())
}
async fn replay(
    writer: &mut Writer,
    console: &Console,
    cursor: &mut Option<u64>,
) -> anyhow::Result<bool> {
    let batch = console.replay_reliable(*cursor, 256).await?;
    if let Some((from, to)) = batch.gap {
        send(writer,&json!({"type":"gap","from":from,"to":to,"message":"Console retention window exceeded","stream_id":console.stream_id})).await?;
    }
    for line in &batch.lines {
        send(writer, line.as_ref()).await?;
        *cursor = Some(line.sequence);
    }
    if cursor.is_none() {
        *cursor = Some(batch.latest);
    }
    Ok(cursor.is_some_and(|n| n < batch.latest))
}
async fn console_socket(state: AppState, q: WsQuery, ws: WebSocket) {
    let (mut writer, mut reader) = ws.split();
    let console = &state.server.console;
    let mut lines = console.changed.subscribe();
    let mut status = state.server.state.clone();
    let mut errors = console.errors_changed.subscribe();
    let mut cursor = if q.stream_id == Some(console.stream_id)
        && q.after.is_none_or(|n| n <= console.replay(None, 0).latest)
    {
        q.after
    } else {
        None
    };
    // A restarted panel has a new stream id; tell the frontend to reset its cursor.
    if send(
        &mut writer,
        &json!({"type":"hello","stream_id":console.stream_id,"resumed":cursor.is_some()}),
    )
    .await
    .is_err()
    {
        return;
    }
    if send(
        &mut writer,
        &json!({"type":"status","status":state.server.snapshot().status}),
    )
    .await
    .is_err()
    {
        return;
    }
    if send(
        &mut writer,
        &json!({"type":"error_stats","errors":console.errors()}),
    )
    .await
    .is_err()
    {
        return;
    }
    let mut backlog = true;
    let mut heartbeat =
        tokio::time::interval(Duration::from_secs(state.config.ws_heartbeat_interval));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _=state.shutdown.cancelled()=>break,
            incoming=reader.next()=>{
                match incoming {
                    Some(Ok(Message::Text(text)))=>{
                        match serde_json::from_str::<Value>(&text) {
                            Ok(v) if v["command"].is_string()=>{
                                let outcome=async {let u=user(&state,&q,"CONSOLE_SEND").await?; let cmd=v["command"].as_str().unwrap_or_default(); state.server.send_command(cmd).await.map_err(|e|ApiError::bad(e.to_string()))?; state.audit(&u,"console.command",cmd,"console").await?; Ok::<_,ApiError>(())}.await;
                                if let Err(e)=outcome&& send(&mut writer,&json!({"type":"error","message":e.message})).await.is_err(){break;}
                            }, Ok(_)=>{}, Err(_)=>{if send(&mut writer,&json!({"type":"error","message":"Invalid JSON message"})).await.is_err(){break;}}
                        }
                    }
                    Some(Ok(Message::Ping(data)))=>{if writer.send(Message::Pong(data)).await.is_err(){break;}}
                    Some(Ok(Message::Pong(_)))=>{}, _=>break,
                }
            }
            _=async {if backlog {tokio::task::yield_now().await;}else{let _=lines.changed().await;}}=>{
                match replay(&mut writer,console,&mut cursor).await {Ok(more)=>backlog=more,Err(_)=>break}
            }
            change=status.changed()=>{
                if change.is_err(){break;}
                let current=status.borrow_and_update().status;
                if send(&mut writer,&json!({"type":"status","status":current})).await.is_err(){break;}
            }
            change=errors.changed()=>{
                if change.is_err(){break;} errors.borrow_and_update();
                if send(&mut writer,&json!({"type":"error_stats","errors":console.errors()})).await.is_err(){break;}
            }
            _=heartbeat.tick()=>{
                if user(&state,&q,"CONSOLE_VIEW").await.is_err(){let _=writer.send(Message::Close(Some(CloseFrame{code:1008,reason:"Authentication or permission expired".into()}))).await;break;}
                if send(&mut writer,&json!({"type":"ping"})).await.is_err(){break;}
            }
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(1), writer.close()).await;
}
async fn metrics_socket(state: AppState, q: WsQuery, ws: WebSocket) {
    let (mut writer, mut reader) = ws.split();
    let mut metrics = state.metrics.latest.clone();
    let initial = metrics.borrow_and_update().clone();
    if send(&mut writer, &json!({"type":"metrics","data":initial}))
        .await
        .is_err()
    {
        return;
    }
    let mut heartbeat =
        tokio::time::interval(Duration::from_secs(state.config.ws_heartbeat_interval));
    loop {
        tokio::select! {
            _=state.shutdown.cancelled()=>break,
            incoming=reader.next()=>{match incoming{Some(Ok(Message::Ping(v)))=>{if writer.send(Message::Pong(v)).await.is_err(){break;}},Some(Ok(Message::Text(_)|Message::Pong(_)))=>{},_=>break}},
            change=metrics.changed()=>{if change.is_err(){break;}let data=metrics.borrow_and_update().clone();if send(&mut writer,&json!({"type":"metrics","data":data})).await.is_err(){break;}},
            _=heartbeat.tick()=>{if user(&state,&q,"SERVER_VIEW").await.is_err() || send(&mut writer,&json!({"type":"ping"})).await.is_err(){break;}}
        }
    }
}
async fn backup_socket(state: AppState, q: WsQuery, ws: WebSocket) {
    let (mut writer, mut reader) = ws.split();
    let mut events = state.backups.events.subscribe();
    if send(
        &mut writer,
        &json!({"type":"hello","active":state.backups.active()}),
    )
    .await
    .is_err()
    {
        return;
    }
    let mut heartbeat =
        tokio::time::interval(Duration::from_secs(state.config.ws_heartbeat_interval));
    loop {
        tokio::select! {
            _=state.shutdown.cancelled()=>break,
            incoming=reader.next()=>{match incoming{Some(Ok(Message::Ping(v)))=>{if writer.send(Message::Pong(v)).await.is_err(){break;}},Some(Ok(Message::Text(_)|Message::Pong(_)))=>{},_=>break}},
            event=events.recv()=>{match event{Ok(v)=>{if send(&mut writer,&v).await.is_err(){break;}},Err(broadcast::error::RecvError::Lagged(n))=>{if send(&mut writer,&json!({"type":"gap","missed":n,"active":state.backups.active()})).await.is_err(){break;}},Err(_)=>break}},
            _=heartbeat.tick()=>{if user(&state,&q,"BACKUPS_VIEW").await.is_err() || send(&mut writer,&json!({"type":"ping"})).await.is_err(){break;}}
        }
    }
}
async fn logs_socket(state: AppState, q: WsQuery, ws: WebSocket) {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let (mut writer, mut reader) = ws.split();
    let path = state.config.logs_dir.join("omnibedrock.log");
    let Ok(mut file) = tokio::fs::File::open(&path).await else {
        return;
    };
    let Ok(mut position) = file.seek(std::io::SeekFrom::End(0)).await else {
        return;
    };
    let mut ticker = tokio::time::interval(Duration::from_millis(200));
    let mut last_auth = tokio::time::Instant::now();
    let mut pending = String::new();
    loop {
        tokio::select! {
            _=state.shutdown.cancelled()=>break,
            incoming=reader.next()=>{match incoming{Some(Ok(Message::Ping(v)))=>{if writer.send(Message::Pong(v)).await.is_err(){break;}},Some(Ok(Message::Text(_)|Message::Pong(_)))=>{},_=>break}},
            _=ticker.tick()=>{
                if last_auth.elapsed()>Duration::from_secs(state.config.ws_heartbeat_interval){if user(&state,&q,"AUDIT_VIEW").await.is_err() || send(&mut writer,&json!({"type":"ping"})).await.is_err(){break;}last_auth=tokio::time::Instant::now();}
                let Ok(metadata)=file.metadata().await else{break;};
                if metadata.len()<position{let _=file.seek(std::io::SeekFrom::Start(0)).await;position=0;pending.clear();}
                let mut bytes=[0u8;8192];let Ok(n)=file.read(&mut bytes).await else{break;};position+=n as u64;pending.push_str(&String::from_utf8_lossy(&bytes[..n]));
                while let Some(i)=pending.find('\n'){let line=pending[..i].to_owned();pending.drain(..=i);if send(&mut writer,&json!({"type":"log","line":line})).await.is_err(){return;}}
                if pending.len()>65536 {let line=std::mem::take(&mut pending);if send(&mut writer,&json!({"type":"log","line":line})).await.is_err(){break;}}
            }
        }
    }
}

/// Broadcaster stream: console replay plus status and device-code events.
/// Commands require BROADCASTER_MANAGE and are revalidated on each send.
async fn broadcaster_socket(state: AppState, q: WsQuery, ws: WebSocket) {
    let (mut writer, mut reader) = ws.split();
    let bc = &state.broadcaster;
    let console = &bc.console;
    let mut lines = console.changed.subscribe();
    let mut status = bc.status.clone();
    let mut codes = bc.device_codes.clone();
    let mut cursor = if q.stream_id == Some(console.stream_id)
        && q.after.is_none_or(|n| n <= console.replay(None, 0).latest)
    {
        q.after
    } else {
        None
    };
    if send(
        &mut writer,
        &json!({"type":"hello","stream_id":console.stream_id,"resumed":cursor.is_some()}),
    )
    .await
    .is_err()
    {
        return;
    }
    if send(
        &mut writer,
        &json!({"type":"status","status":bc.snapshot()}),
    )
    .await
    .is_err()
    {
        return;
    }
    if send(
        &mut writer,
        &json!({"type":"device_codes","codes":bc.device_codes()}),
    )
    .await
    .is_err()
    {
        return;
    }
    let mut backlog = true;
    let mut heartbeat =
        tokio::time::interval(Duration::from_secs(state.config.ws_heartbeat_interval));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _=state.shutdown.cancelled()=>break,
            incoming=reader.next()=>{
                match incoming {
                    Some(Ok(Message::Text(text)))=>{
                        match serde_json::from_str::<Value>(&text) {
                            Ok(v) if v["command"].is_string()=>{
                                let outcome=async {let u=user(&state,&q,"BROADCASTER_MANAGE").await?; let cmd=v["command"].as_str().unwrap_or_default(); state.broadcaster.send_command(cmd).await.map_err(|e|ApiError::bad(e.to_string()))?; state.audit(&u,"broadcaster.command",cmd,"broadcaster").await?; Ok::<_,ApiError>(())}.await;
                                if let Err(e)=outcome&& send(&mut writer,&json!({"type":"error","message":e.message})).await.is_err(){break;}
                            }, Ok(_)=>{}, Err(_)=>{if send(&mut writer,&json!({"type":"error","message":"Invalid JSON message"})).await.is_err(){break;}}
                        }
                    }
                    Some(Ok(Message::Ping(data)))=>{if writer.send(Message::Pong(data)).await.is_err(){break;}}
                    Some(Ok(Message::Pong(_)))=>{}, _=>break,
                }
            }
            _=async {if backlog {tokio::task::yield_now().await;}else{let _=lines.changed().await;}}=>{
                match replay(&mut writer,console,&mut cursor).await {Ok(more)=>backlog=more,Err(_)=>break}
            }
            change=status.changed()=>{
                if change.is_err(){break;}
                status.borrow_and_update();
                if send(&mut writer,&json!({"type":"status","status":state.broadcaster.snapshot()})).await.is_err(){break;}
            }
            change=codes.changed()=>{
                if change.is_err(){break;}
                codes.borrow_and_update();
                if send(&mut writer,&json!({"type":"device_codes","codes":state.broadcaster.device_codes()})).await.is_err(){break;}
            }
            _=heartbeat.tick()=>{
                if user(&state,&q,"BROADCASTER_VIEW").await.is_err(){let _=writer.send(Message::Close(Some(CloseFrame{code:1008,reason:"Authentication or permission expired".into()}))).await;break;}
                if send(&mut writer,&json!({"type":"ping"})).await.is_err(){break;}
            }
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(1), writer.close()).await;
}
