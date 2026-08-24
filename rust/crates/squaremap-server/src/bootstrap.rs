use crate::session::{Session, SessionError};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use prost::Message;
use rand::RngCore;
use squaremap_protocol::wire::{
    BackendResultCode, BackendSubstitution, BridgePolicyReplace, ChunkCoordinate, ConfigReplace,
    ControlKind, ControlRequest, ControlResult, DirtyReplayRequest, DirtyResyncComplete, Envelope,
    Hello, ProtocolError, ProtocolErrorCode, Shutdown, backend_substitution, envelope,
};
use squaremap_protocol::{FrameClass, FrameError, FrameLimits, read_envelope, write_envelope};
use squaremap_render::{Limits, PngOptions, RenderSettings, TileStore};
use squaremap_server::config::ConfigStore;
use squaremap_server::control::ControlState;
use squaremap_server::dirty_resync::{self, ReplayState};
use squaremap_server::http::{HttpConfig, HttpServer};
use squaremap_server::scheduler::live::{
    EnumerationDispatcher, LiveSnapshotBridge, OutboundEnumerationRequest, OutboundSnapshotRequest,
    SnapshotDispatcher,
};
use squaremap_server::scheduler::{
    BridgeError, RenderTileInstaller, Scheduler, SchedulerConfig, WorldRenderConfig,
};
use squaremap_server::snapshot_client::SnapshotClient;
use squaremap_state::{Repository, World, WorldId};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{self, BufRead, Read};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use zeroize::{Zeroize, Zeroizing};

const PROTOCOL_MAJOR: u32 = 1;
const PROTOCOL_MINOR: u32 = 0;
const SESSION_ID_BYTES: usize = 16;
const BOOTSTRAP_TOKEN_BYTES: usize = 32;
const MAX_TOKEN_LINE_BYTES: usize = 88;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub enum BootstrapError {
    InvalidConnect(String),
    Token(String),
    Io(io::Error),
    Frame(FrameError),
    Rejected(String),
    ConnectTimeout,
}

impl fmt::Display for BootstrapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConnect(message) => {
                write!(formatter, "invalid loopback connect address: {message}")
            }
            Self::Token(message) => write!(formatter, "invalid bootstrap token: {message}"),
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Frame(error) => write!(formatter, "bridge frame error: {error}"),
            Self::Rejected(message) => write!(formatter, "bridge rejected handshake: {message}"),
            Self::ConnectTimeout => formatter.write_str("bridge connection timed out"),
        }
    }
}

impl std::error::Error for BootstrapError {}

impl From<io::Error> for BootstrapError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<FrameError> for BootstrapError {
    fn from(error: FrameError) -> Self {
        Self::Frame(error)
    }
}

pub fn parse_connect(value: &str) -> Result<SocketAddr, BootstrapError> {
    let address = SocketAddr::from_str(value)
        .map_err(|error| BootstrapError::InvalidConnect(error.to_string()))?;
    if !address.ip().is_loopback() {
        return Err(BootstrapError::InvalidConnect(
            "address is not loopback".to_string(),
        ));
    }
    Ok(address)
}

pub fn read_token<R: BufRead>(reader: R) -> Result<Zeroizing<Vec<u8>>, BootstrapError> {
    let mut bounded = reader.take((MAX_TOKEN_LINE_BYTES + 1) as u64);
    let mut line = Zeroizing::new(Vec::with_capacity(MAX_TOKEN_LINE_BYTES + 1));
    bounded.read_to_end(&mut line)?;
    if !line.ends_with(b"\n") {
        return Err(BootstrapError::Token(
            "expected one newline-terminated line of at most 88 characters".to_string(),
        ));
    }
    line.pop();
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    if line.len() > MAX_TOKEN_LINE_BYTES || !line.is_ascii() {
        return Err(BootstrapError::Token(
            "token line is too long or non-ASCII".to_string(),
        ));
    }
    let decoded = Zeroizing::new(
        STANDARD
            .decode(&line)
            .map_err(|error| BootstrapError::Token(error.to_string()))?,
    );
    if decoded.len() != BOOTSTRAP_TOKEN_BYTES {
        return Err(BootstrapError::Token(
            "token must decode to exactly 32 bytes".to_string(),
        ));
    }
    Ok(decoded)
}

async fn write_bootstrap_hello<W: AsyncWrite + Unpin>(
    writer: &mut W,
    plugin_version: &str,
    session_id: &[u8; SESSION_ID_BYTES],
    token: &mut Zeroizing<Vec<u8>>,
) -> Result<(), BootstrapError> {
    let result =
        write_bootstrap_hello_inner(writer, plugin_version, session_id, token.as_slice()).await;
    token.zeroize();
    result
}

async fn write_bootstrap_hello_inner<W: AsyncWrite + Unpin>(
    writer: &mut W,
    plugin_version: &str,
    session_id: &[u8; SESSION_ID_BYTES],
    token: &[u8],
) -> Result<(), BootstrapError> {
    let mut hello = Envelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        session_id: session_id.to_vec(),
        sequence: 1,
        correlation_id: 0,
        payload: Some(envelope::Payload::Hello(Hello {
            plugin_version: plugin_version.to_string(),
            bootstrap_token: token.to_vec(),
        })),
    };
    let mut encoded_payload = Vec::with_capacity(hello.encoded_len());
    let encoded = hello.encode(&mut encoded_payload);
    zeroize_hello_payload(&mut hello);
    let payload = Zeroizing::new(encoded_payload);
    encoded
        .map_err(FrameError::ProtobufEncode)
        .map_err(BootstrapError::Frame)?;
    if payload.is_empty() {
        return Err(BootstrapError::Frame(FrameError::ZeroLength));
    }
    if payload.len() > FrameLimits::MAX_CONTROL_BYTES as usize {
        return Err(BootstrapError::Frame(FrameError::DeclaredLength {
            class: FrameClass::Control,
            length: payload.len() as u32,
            max: FrameLimits::MAX_CONTROL_BYTES,
        }));
    }
    let mut prefix = [0_u8; 5];
    prefix[0] = FrameClass::Control as u8;
    prefix[1..].copy_from_slice(&(payload.len() as u32).to_be_bytes());
    writer.write_all(&prefix).await?;
    writer.write_all(payload.as_slice()).await?;
    Ok(())
}

fn zeroize_hello_payload(envelope: &mut Envelope) {
    if let Some(envelope::Payload::Hello(hello)) = envelope.payload.as_mut() {
        hello.bootstrap_token.zeroize();
        hello.bootstrap_token.clear();
    }
}
fn make_outbound(session_id: &[u8], correlation_id: u64, payload: envelope::Payload) -> Envelope {
    Envelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        session_id: session_id.to_vec(),
        sequence: 0,
        correlation_id,
        payload: Some(payload),
    }
}
fn replay_error(error: dirty_resync::DirtyResyncError) -> ProtocolError {
    ProtocolError {
        code: ProtocolErrorCode::InvalidMessage as i32,
        message: format!("{error:?}"),
        fatal: true,
        offending_sequence: 0,
        config_revision: 0,
    }
}

async fn dispatch_replay_request(
    repository: &Repository,
    request: DirtyReplayRequest,
    bridge_id: &[u8],
    session_id: &[u8],
    config_revision: u64,
    outbound: &mpsc::Sender<Envelope>,
) -> Result<(), BootstrapError> {
    let mut replay = match ReplayState::new(request.clone(), bridge_id, session_id, config_revision)
    {
        Ok(replay) => replay,
        // A stale replay from the previous config revision must not tear down HTTP.
        Err(squaremap_server::dirty_resync::DirtyResyncError::RevisionMismatch) => return Ok(()),
        Err(error) => {
            return Err(BootstrapError::Rejected(format!("{error:?}")));
        }
    };
    let request = replay.request().clone();
    let world = request.world.clone().expect("validated replay world");
    let cursor = match request.cursor.as_ref() {
        Some(squaremap_protocol::wire::dirty_replay_request::Cursor::Continuation(coord)) => {
            Some(squaremap_state::ChunkCoordinate {
                x: coord.x,
                z: coord.z,
            })
        }
        _ => None,
    };
    let mut rows = repository
        .dirty_replay_page(
            bridge_id,
            session_id,
            &squaremap_state::WorldId::new(&world.namespace, &world.value, world.epoch),
            cursor.as_ref(),
            request.max_items as usize,
        )
        .await
        .map_err(|error| BootstrapError::Rejected(error.to_string()))?;
    let has_more = rows.len() > request.max_items as usize;
    if has_more {
        rows.pop();
    }
    let last_coordinate = if has_more {
        rows.last().map(|last| ChunkCoordinate {
            x: last.coordinate.x,
            z: last.coordinate.z,
        })
    } else {
        None
    };
    let mut count = 0_u32;
    for row in rows {
        let item = dirty_resync::replay_item(
            &squaremap_state::DirtyChunk {
                world: squaremap_state::WorldId::new(
                    &row.world.namespace,
                    &row.world.value,
                    row.world.epoch,
                ),
                coordinate: row.coordinate,
                revision: row.revision,
                owner_bridge_id: bridge_id.to_vec(),
                lease_expires_epoch_seconds: 0,
            },
            bridge_id,
            session_id,
            config_revision,
            request.replay_id,
            count,
        );
        replay
            .accept_item(&item)
            .map_err(|error| BootstrapError::Rejected(format!("{error:?}")))?;
        queue_outbound(
            outbound,
            make_outbound(session_id, 0, envelope::Payload::DirtyReplayItem(item)),
        )
        .await?;
        count += 1;
    }
    let completion = DirtyResyncComplete {
        bridge_id: bridge_id.to_vec(),
        session_id: session_id.to_vec(),
        config_revision,
        replay_id: request.replay_id,
        item_count: count,
        status: squaremap_protocol::wire::DirtyResyncStatus::Complete as i32,
        failure_reason: String::new(),
        has_more,
        last_coordinate,
    };
    replay
        .complete(&completion)
        .map_err(|error| BootstrapError::Rejected(format!("{error:?}")))?;
    queue_outbound(
        outbound,
        make_outbound(
            session_id,
            0,
            envelope::Payload::DirtyResyncComplete(completion),
        ),
    )
    .await
}

async fn queue_outbound(
    sender: &mpsc::Sender<Envelope>,
    envelope: Envelope,
) -> Result<(), BootstrapError> {
    sender.send(envelope).await.map_err(|_| {
        BootstrapError::Io(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "bridge writer stopped",
        ))
    })
}

async fn write_outbound<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut receiver: mpsc::Receiver<Envelope>,
    next_sequence: Arc<AtomicU64>,
    failure: oneshot::Sender<io::Error>,
) {
    while let Some(mut envelope) = receiver.recv().await {
        envelope.sequence = next_sequence.fetch_add(1, Ordering::Relaxed);
        if let Err(error) = write_envelope(&mut writer, &envelope, FrameLimits::default()).await {
            let _ = failure.send(io::Error::other(error.to_string()));
            return;
        }
    }
}

async fn dispatch_snapshot_requests(
    mut requests: mpsc::Receiver<OutboundSnapshotRequest>,
    dispatcher: Arc<SnapshotDispatcher>,
    outbound: mpsc::Sender<Envelope>,
) {
    while let Some(request) = requests.recv().await {
        match dispatcher.begin(request).await {
            Ok(envelope) => {
                if outbound.send(envelope).await.is_err() {
                    return;
                }
            }
            Err(error) => tracing::warn!(error = %error, "could not dispatch snapshot request"),
        }
    }
}

async fn dispatch_enumeration_requests(
    mut requests: mpsc::Receiver<OutboundEnumerationRequest>,
    dispatcher: Arc<EnumerationDispatcher>,
    outbound: mpsc::Sender<Envelope>,
    session_id: Vec<u8>,
    activation: Arc<tokio::sync::RwLock<Option<(Vec<u8>, Vec<u8>, u64)>>>,
) {
    dispatcher.set_outbound(outbound.clone()).await;
    let mut next_id = 1_u64;
    while let Some(request) = requests.recv().await {
        let Some((bridge_id, active_session, revision)) = activation.read().await.clone() else {
            tracing::warn!("cannot dispatch enumeration before activation");
            let OutboundEnumerationRequest {
                response,
                cancellation,
                ..
            } = request;
            drop(cancellation);
            let _ = response.send(Err(BridgeError::Transient("bridge is not active".into())));
            continue;
        };
        if active_session != session_id {
            tracing::warn!("cannot dispatch enumeration for inactive session");
            let OutboundEnumerationRequest {
                response,
                cancellation,
                ..
            } = request;
            drop(cancellation);
            let _ = response.send(Err(BridgeError::Transient(
                "bridge session mismatch".into(),
            )));
            continue;
        }
        match dispatcher
            .begin(request, &session_id, &bridge_id, revision, next_id)
            .await
        {
            Ok(envelope) => {
                let correlation_id = envelope.correlation_id;
                next_id = next_id.saturating_add(1);
                if outbound.send(envelope).await.is_err() {
                    dispatcher
                        .abort_request(correlation_id, "enumeration outbound closed")
                        .await;
                    return;
                }
            }
            Err(error) => tracing::warn!(?error, "could not dispatch enumeration request"),
        }
    }
}
fn configure_installer(
    installer: &RenderTileInstaller,
    config: &ConfigReplace,
    policy: &BridgePolicyReplace,
) -> Result<(SchedulerConfig, bool), String> {
    let global = config
        .global
        .as_ref()
        .ok_or("global settings are missing")?;
    let default_world = config
        .world
        .as_ref()
        .ok_or("default world settings are missing")?;
    let render = config
        .render
        .as_ref()
        .ok_or("render settings are missing")?;
    let max_active_snapshots = usize::try_from(policy.snapshot_credits)
        .map_err(|_| "snapshot credit count is outside platform bounds")?;
    let dirty_page_size = usize::try_from(default_world.background_render_max_chunks_per_interval)
        .map_err(|_| "background page size is outside platform bounds")?;
    let max_zoom = u8::try_from(default_world.zoom_max)
        .map_err(|_| "maximum zoom is outside platform bounds")?;
    if max_zoom > 9 {
        return Err("maximum zoom exceeds the tile pyramid limit".to_string());
    }
    for configured in &config.worlds {
        let identity = configured
            .identity
            .as_ref()
            .ok_or("world identity is missing")?;
        let settings = configured
            .settings
            .as_ref()
            .ok_or("world settings are missing")?;
        let max_zoom = u8::try_from(settings.zoom_max).map_err(|_| {
            format!(
                "maximum zoom is outside platform bounds for {}:{}",
                identity.namespace, identity.value
            )
        })?;
        if max_zoom > 9 {
            return Err(format!(
                "maximum zoom exceeds the tile pyramid limit for {}:{}",
                identity.namespace, identity.value
            ));
        }
        installer.configure_world(
            WorldId::new(
                identity.namespace.clone(),
                identity.value.clone(),
                identity.epoch,
            ),
            WorldRenderConfig {
                settings: RenderSettings {
                    iterate_up: settings.map_iterate_up,
                    map_max_height: settings.map_max_height,
                    biomes_enabled: settings.map_biomes_enabled,
                    biome_blend: settings.map_biomes_blend,
                    glass_clear: settings.map_glass_clear,
                    water_clear: settings.map_water_clear,
                    water_checkerboard: settings.map_water_checkerboard,
                    lava_checkerboard: settings.map_lava_checkerboard,
                },
                // Invisible and iterate-up-base membership is carried by each
                // world-specific registry descriptor.
                invisible_ids: Vec::<u32>::new().into(),
                iterate_up_base_ids: Vec::<u32>::new().into(),
                biome_zoom_seed: settings.biome_zoom_seed as i64,
                max_zoom,
                png_options: PngOptions {
                    compression: global.compress_images,
                },
                tile_prefix: PathBuf::from("tiles")
                    .join(format!("{}_{}", identity.namespace, identity.value)),
            },
        )?;
    }
    Ok((
        SchedulerConfig {
            max_active_snapshots,
            dirty_page_size,
            background_interval: Duration::from_secs(u64::from(
                default_world.background_render_interval_seconds,
            )),
            ..SchedulerConfig::default()
        },
        render.background_enabled && default_world.background_render_enabled,
    ))
}

#[cfg(test)]
fn spawn_background_scheduler_observed(
    scheduler: Arc<Scheduler>,
    enabled: bool,
    live: Arc<std::sync::atomic::AtomicU64>,
) -> Option<JoinHandle<()>> {
    if !enabled {
        return None;
    }
    let interval = scheduler.config().background_interval;
    live.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    Some(tokio::spawn(async move {
        struct LiveGuard(Arc<std::sync::atomic::AtomicU64>);
        impl Drop for LiveGuard {
            fn drop(&mut self) {
                self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let _guard = LiveGuard(live);
        let _ = scheduler.resume_jobs().await;
        loop {
            let _ = scheduler.run_dirty_page().await;
            tokio::time::sleep(interval).await;
        }
    }))
}

fn spawn_background_scheduler(
    scheduler: Arc<Scheduler>,
    enabled: bool,
    identity: Arc<tokio::sync::RwLock<Option<(Vec<u8>, Vec<u8>, u64)>>>,
) -> Option<JoinHandle<()>> {
    if !enabled {
        return None;
    }
    let interval = scheduler.config().background_interval;
    Some(tokio::spawn(async move {
        let _ = scheduler.resume_jobs().await;
        loop {
            if let Some((bridge_id, _, _)) = identity.read().await.clone() {
                let _ = scheduler.run_live_dirty_page(&bridge_id).await;
            }
            tokio::time::sleep(interval).await;
        }
    }))
}
fn control_response(
    code: BackendResultCode,
    identity: Option<&squaremap_protocol::wire::WorldIdentity>,
) -> ControlResult {
    let substitutions = identity
        .into_iter()
        .map(|world| BackendSubstitution {
            key: "world".to_string(),
            value: Some(backend_substitution::Value::WorldIdentity(world.clone())),
        })
        .collect();
    ControlResult {
        code: code as i32,
        substitutions,
        rendered_chunks: 0,
    }
}
#[derive(Debug)]
enum RenderState {
    Discovering(tokio_util::sync::CancellationToken),
    Running {
        job_id: Vec<u8>,
        task: Option<JoinHandle<()>>,
    },
}
async fn shutdown_render_tasks(
    active_jobs: &Arc<Mutex<HashMap<WorldId, RenderState>>>,
    scheduler: Option<&Arc<Scheduler>>,
) {
    let entries = match active_jobs.lock() {
        Ok(mut active) => active.drain().map(|(_, state)| state).collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    let mut handles = Vec::new();
    for state in entries {
        match state {
            RenderState::Discovering(token) => token.cancel(),
            RenderState::Running { job_id, task } => {
                if let Some(scheduler) = scheduler {
                    let _ = scheduler.cancel_job(&job_id).await;
                }
                if let Some(task) = task {
                    handles.push(task);
                }
            }
        }
    }
    for task in handles {
        task.abort();
        let _ = task.await;
    }
}

#[allow(dead_code)]
async fn handle_control_request(
    state: &mut ControlState,
    request: &ControlRequest,
    scheduler: Option<Arc<Scheduler>>,
    active_jobs: Arc<Mutex<HashMap<WorldId, RenderState>>>,
    revision: u64,
    root: &squaremap_server::output::OutputRoot,
) -> ControlResult {
    let baseline = state.handle(request);
    if baseline.code != BackendResultCode::BackendUnavailable as i32 {
        return baseline;
    }
    execute_control_request(request, baseline, scheduler, active_jobs, revision, root).await
}
async fn execute_control_request(
    request: &ControlRequest,
    baseline: ControlResult,
    scheduler: Option<Arc<Scheduler>>,
    active_jobs: Arc<Mutex<HashMap<WorldId, RenderState>>>,
    revision: u64,
    root: &squaremap_server::output::OutputRoot,
) -> ControlResult {
    let Ok(kind) = ControlKind::try_from(request.kind) else {
        return control_response(BackendResultCode::InvalidRequest, request.world.as_ref());
    };
    let Some(identity) = request.world.as_ref() else {
        return control_response(BackendResultCode::InvalidRequest, None);
    };
    let world = WorldId::new(
        identity.namespace.clone(),
        identity.value.clone(),
        identity.epoch,
    );
    let Some(scheduler) = scheduler else {
        return baseline;
    };
    match kind {
        ControlKind::FullRender | ControlKind::RadiusRender => {
            let reservation = {
                let mut active = match active_jobs.lock() {
                    Ok(active) => active,
                    Err(_) => {
                        return control_response(
                            BackendResultCode::BackendUnavailable,
                            Some(identity),
                        );
                    }
                };
                if active.contains_key(&world) {
                    return control_response(BackendResultCode::RenderInProgress, Some(identity));
                }
                let token = tokio_util::sync::CancellationToken::new();
                active.insert(world.clone(), RenderState::Discovering(token.clone()));
                token
            };
            let coordinates = if kind == ControlKind::FullRender {
                match tokio::select! {
                    result = scheduler.discover_full_render_coordinates(&world) => result,
                    _ = reservation.cancelled() => {
                        if let Ok(mut active) = active_jobs.lock() { active.remove(&world); }
                        return control_response(BackendResultCode::RenderCancelled, Some(identity));
                    }
                } {
                    Ok(coordinates) if coordinates.len() <= 16_384 => coordinates,
                    Ok(_) | Err(_) => {
                        if let Ok(mut active) = active_jobs.lock() {
                            active.remove(&world);
                        }
                        return control_response(BackendResultCode::Failed, Some(identity));
                    }
                }
            } else if !request.coordinates.is_empty() {
                if request.coordinates.len() > 16_384 {
                    if let Ok(mut active) = active_jobs.lock() {
                        active.remove(&world);
                    }
                    return control_response(BackendResultCode::InvalidRequest, Some(identity));
                }
                scheduler.filter_visible(
                    &world,
                    request
                        .coordinates
                        .iter()
                        .map(|coordinate| squaremap_state::ChunkCoordinate {
                            x: coordinate.x,
                            z: coordinate.z,
                        })
                        .collect(),
                )
            } else {
                let Ok(radius_blocks) = i32::try_from(request.radius) else {
                    if let Ok(mut active) = active_jobs.lock() {
                        active.remove(&world);
                    }
                    return control_response(BackendResultCode::InvalidRequest, Some(identity));
                };
                let visibility = scheduler.visibility_for(&world);
                match squaremap_server::radius::chunks_from_blocks(
                    request.center_x,
                    request.center_z,
                    radius_blocks,
                    visibility.as_ref(),
                ) {
                    Ok(coordinates) if !coordinates.is_empty() && coordinates.len() <= 16_384 => {
                        coordinates
                    }
                    Ok(_) | Err(_) => {
                        if let Ok(mut active) = active_jobs.lock() {
                            active.remove(&world);
                        }
                        return control_response(BackendResultCode::InvalidRequest, Some(identity));
                    }
                }
            };
            if coordinates.is_empty() {
                if let Ok(mut active) = active_jobs.lock() {
                    active.remove(&world);
                }
                return control_response(BackendResultCode::InvalidRequest, Some(identity));
            }
            let job_kind = if kind == ControlKind::FullRender {
                squaremap_state::JobKind::Full
            } else {
                squaremap_state::JobKind::Radius
            };
            let job = match scheduler
                .start_job_at_revision(world.clone(), job_kind, coordinates, revision)
                .await
            {
                Ok(job) => job,
                Err(error) => {
                    tracing::warn!(error = %error, "could not start render");
                    if let Ok(mut active) = active_jobs.lock() {
                        active.remove(&world);
                    }
                    return control_response(BackendResultCode::Failed, Some(identity));
                }
            };
            let job_id = job.id.clone();
            let cancelled = {
                let mut active = match active_jobs.lock() {
                    Ok(active) => active,
                    Err(_) => {
                        return control_response(
                            BackendResultCode::BackendUnavailable,
                            Some(identity),
                        );
                    }
                };
                match active.get(&world) {
                    Some(RenderState::Discovering(token)) if token.is_cancelled() => {
                        active.remove(&world);
                        true
                    }
                    Some(RenderState::Discovering(_)) => {
                        active.insert(
                            world.clone(),
                            RenderState::Running {
                                job_id: job_id.clone(),
                                task: None,
                            },
                        );
                        false
                    }
                    _ => true,
                }
            };
            if cancelled {
                let _ = scheduler.cancel_job(&job_id).await;
                return control_response(BackendResultCode::RenderCancelled, Some(identity));
            }
            let jobs = active_jobs.clone();
            let world_for_task = world.clone();
            let scheduler_for_task = scheduler.clone();
            let task_job_id = job_id.clone();
            let task = tokio::spawn(async move {
                if let Err(error) = scheduler_for_task.run_job(&task_job_id).await {
                    tracing::warn!(error = %error, "render failed");
                }
                if let Ok(mut active) = jobs.lock() {
                    if matches!(active.get(&world_for_task), Some(RenderState::Running { job_id: active_id, .. }) if active_id == &task_job_id)
                    {
                        active.remove(&world_for_task);
                    }
                }
            });
            if let Ok(mut active) = active_jobs.lock() {
                if let Some(RenderState::Running {
                    task: task_slot, ..
                }) = active.get_mut(&world)
                {
                    *task_slot = Some(task);
                }
            }
            control_response(
                if kind == ControlKind::FullRender {
                    BackendResultCode::FullRenderStarted
                } else {
                    BackendResultCode::RadiusRenderStarted
                },
                Some(identity),
            )
        }
        ControlKind::PauseRenders => {
            if scheduler.is_paused(&world) {
                scheduler.resume(&world);
                control_response(BackendResultCode::RendersResumed, Some(identity))
            } else {
                scheduler.pause(&world);
                control_response(BackendResultCode::RendersPaused, Some(identity))
            }
        }
        ControlKind::CancelRender => {
            let state = active_jobs
                .lock()
                .ok()
                .and_then(|active| match active.get(&world) {
                    Some(RenderState::Discovering(token)) => Some((None, Some(token.clone()))),
                    Some(RenderState::Running { job_id, .. }) => Some((Some(job_id.clone()), None)),
                    None => None,
                });
            let Some((job, token)) = state else {
                return control_response(BackendResultCode::RenderNotInProgress, Some(identity));
            };
            if let Some(token) = token {
                token.cancel();
                if let Ok(mut active) = active_jobs.lock() {
                    active.remove(&world);
                }
                return control_response(BackendResultCode::RenderCancelled, Some(identity));
            }
            let job = job.expect("running state has job id");
            match scheduler.cancel_job(&job).await {
                Ok(()) => {
                    if let Ok(mut active) = active_jobs.lock() {
                        if matches!(active.get(&world), Some(RenderState::Running { job_id, .. }) if job_id == &job)
                        {
                            active.remove(&world);
                        }
                    }
                    control_response(BackendResultCode::RenderCancelled, Some(identity))
                }
                Err(error) => {
                    tracing::warn!(error = %error, "could not cancel render");
                    control_response(BackendResultCode::Failed, Some(identity))
                }
            }
        }
        ControlKind::ResetMap => {
            let render_in_progress = match active_jobs.lock() {
                Ok(active) => active.contains_key(&world),
                Err(_) => {
                    return control_response(BackendResultCode::BackendUnavailable, Some(identity));
                }
            };
            if render_in_progress {
                return control_response(BackendResultCode::RenderInProgress, Some(identity));
            }
            let tile_prefix =
                PathBuf::from("tiles").join(format!("{}_{}", identity.namespace, identity.value));
            if let Err(error) = scheduler.reset_world(&world).await {
                tracing::warn!(error = %error, "could not reset durable render state");
                return control_response(BackendResultCode::Failed, Some(identity));
            }
            if let Err(error) = root.remove_tree(&tile_prefix) {
                tracing::warn!(error = %error, "could not reset rendered tiles");
                return control_response(BackendResultCode::Failed, Some(identity));
            }
            control_response(BackendResultCode::MapReset, Some(identity))
        }
        ControlKind::Health | ControlKind::Reload | ControlKind::Unspecified => baseline,
    }
}

pub async fn run_bridge(
    connect: SocketAddr,
    plugin_version: &str,
    mut token: Zeroizing<Vec<u8>>,
) -> Result<(), BootstrapError> {
    let mut stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(connect))
        .await
        .map_err(|_| BootstrapError::ConnectTimeout)??;
    let mut session_id = [0_u8; SESSION_ID_BYTES];
    rand::rng().fill_bytes(&mut session_id);
    session_id[6] = (session_id[6] & 0x0f) | 0x40;
    session_id[8] = (session_id[8] & 0x3f) | 0x80;
    tracing::info!(
        session_id = %hex::encode(session_id),
        protocol_major = PROTOCOL_MAJOR,
        plugin_version,
        "bridge hello sent"
    );
    write_bootstrap_hello(&mut stream, plugin_version, &session_id, &mut token).await?;
    let ack = read_envelope(&mut stream, FrameLimits::default()).await?;
    let ack_payload = match ack.payload {
        Some(envelope::Payload::HelloAck(payload)) => payload,
        _ => return Err(BootstrapError::Rejected("expected HelloAck".to_string())),
    };
    if ack.protocol_major != PROTOCOL_MAJOR
        || ack.session_id.as_slice() != session_id
        || ack_payload.protocol_major != PROTOCOL_MAJOR
        || !ack_payload.accepted
    {
        return Err(BootstrapError::Rejected(ack_payload.rejection_reason));
    }

    let mut session = Session::from_ids(&[0; SESSION_ID_BYTES], &session_id)
        .map_err(|error| BootstrapError::Rejected(error.to_string()))?;
    let configured_root = std::env::var("SQUAREMAP_OUTPUT_ROOT").map_err(|_| {
        BootstrapError::Rejected("SQUAREMAP_OUTPUT_ROOT is required for bridge mode".to_string())
    })?;
    let mut http_server: Option<HttpServer> = None;
    let root = squaremap_server::output::OutputRoot::new(configured_root)?;
    let repository = Arc::new(
        Repository::open(root.path().join(".squaremap-state.sqlite"))
            .await
            .map_err(|error| {
                BootstrapError::Rejected(format!("could not open durable render state: {error}"))
            })?,
    );
    let (control_result_sender, mut control_result_receiver) =
        mpsc::channel::<(u64, ControlResult)>(16);
    let mut config_store = ConfigStore::default();
    let mut control_state = ControlState::default();
    let snapshot_client = SnapshotClient::new(session_id, Limits::default())
        .map_err(|error| BootstrapError::Rejected(error.to_string()))?;
    let (snapshot_bridge, snapshot_requests, enumeration_requests) =
        LiveSnapshotBridge::channel_with_enumeration(256);
    let dispatcher = Arc::new(SnapshotDispatcher::new(snapshot_client));
    let tile_store: Arc<dyn TileStore> = Arc::new(root.clone());
    let installer = Arc::new(RenderTileInstaller::new(tile_store));

    let (outbound, outbound_receiver) = mpsc::channel(256);
    let (writer_failure, mut writer_failure_receiver) = oneshot::channel();
    let next_outbound_sequence = Arc::new(AtomicU64::new(ack.sequence.saturating_add(1)));
    let (mut reader, writer) = stream.into_split();
    let writer_task = tokio::spawn(write_outbound(
        writer,
        outbound_receiver,
        next_outbound_sequence,
        writer_failure,
    ));
    let snapshot_task = tokio::spawn(dispatch_snapshot_requests(
        snapshot_requests,
        dispatcher.clone(),
        outbound.clone(),
    ));
    let mut control_tasks: HashMap<u64, JoinHandle<()>> = HashMap::new();
    let mut scheduler: Option<Arc<Scheduler>> = None;
    let active_jobs = Arc::new(Mutex::new(HashMap::<WorldId, RenderState>::new()));
    let mut active_config_revision = 0_u64;
    let mut stable_bridge_id: Option<Vec<u8>> = None;
    let activation = Arc::new(tokio::sync::RwLock::new(None));
    let mut recovery_ready = false;
    let mut background_task: Option<JoinHandle<()>> = None;
    let enumeration_dispatcher = Arc::new(EnumerationDispatcher::new());
    let enumeration_task = tokio::spawn(dispatch_enumeration_requests(
        enumeration_requests,
        enumeration_dispatcher.clone(),
        outbound.clone(),
        session_id.to_vec(),
        activation.clone(),
    ));
    let mut shutdown_received = false;
    let mut loop_error = None;
    loop {
        tokio::select! {
            Some((correlation_id, result)) = control_result_receiver.recv() => {
                control_tasks.remove(&correlation_id);
                let response = make_outbound(&session_id, correlation_id, envelope::Payload::ControlResult(result));
                if let Err(queue_error) = queue_outbound(&outbound, response).await {
                    loop_error = Some(queue_error);
                    break;
                }
            }
            frame = read_envelope(&mut reader, FrameLimits::default()) => {
                match frame {
                    Ok(envelope) => {
                        if envelope.protocol_major != PROTOCOL_MAJOR
                            || envelope.session_id.as_slice() != session.session_id()
                        {
                            let error = make_outbound(
                                &session_id,
                                envelope.correlation_id,
                                envelope::Payload::ProtocolError(ProtocolError {
                                    code: if envelope.protocol_major != PROTOCOL_MAJOR {
                                        ProtocolErrorCode::UnsupportedVersion as i32
                                    } else {
                                        ProtocolErrorCode::InvalidMessage as i32
                                    },
                                    message: "bridge envelope failed authenticated protocol/session validation".to_string(),
                                    fatal: true,
                                    offending_sequence: envelope.sequence,
                                    config_revision: 0,
                                }),
                            );
                            if let Err(queue_error) = queue_outbound(&outbound, error).await {
                                loop_error = Some(queue_error);
                            } else {
                                loop_error = Some(BootstrapError::Rejected(
                                    "bridge envelope failed authenticated protocol/session validation".to_string(),
                                ));
                            }
                            break;
                        }
                        if matches!(envelope.payload, Some(envelope::Payload::Shutdown(Shutdown { .. }))) {
                            shutdown_received = true;
                            break;
                        }
                        if let Some(envelope::Payload::BridgeIdentityReplace(identity)) = envelope.payload.as_ref() {
                            let payload = identity.clone();
                            let repository_for_identity = repository.clone();
                            let outcome = session.process(envelope.clone(), move |_| {
                                let repository = repository_for_identity.clone();
                                let payload = payload.clone();
                                async move {
                                    if payload.bridge_id.len() != SESSION_ID_BYTES || payload.bridge_id.iter().all(|byte| *byte == 0) {
                                        return Err(SessionError::HandlerFailed("invalid bridge identity".to_string()));
                                    }
                                    repository
                                        .persist_bridge_identity(&payload.bridge_id, &envelope.session_id, envelope.sequence)
                                        .await
                                        .map_err(|error| SessionError::HandlerFailed(error.to_string()))
                                }
                            }).await.map_err(|error| BootstrapError::Rejected(error.to_string()))?;
                            if let Some(protocol_error) = outcome.protocol_error() {
                                queue_outbound(
                                    &outbound,
                                    make_outbound(&session_id, envelope.correlation_id, envelope::Payload::ProtocolError(protocol_error.clone())),
                                ).await?;
                                loop_error = Some(BootstrapError::Rejected(protocol_error.message.clone()));
                                break;
                            }
                            if outcome.ack().is_some_and(|ack| ack.status == squaremap_protocol::wire::AckStatus::Accepted as i32) {
                                let bridge_id: [u8; SESSION_ID_BYTES] = identity.bridge_id.as_slice().try_into().expect("validated bridge identity length");
                                session.replace_bridge_id(bridge_id).map_err(|error| BootstrapError::Rejected(error.to_string()))?;
                                repository
                                    .reassign_bridge_lease(&identity.bridge_id, &session_id)
                                    .await
                                    .map_err(|error| BootstrapError::Rejected(error.to_string()))?;
                                stable_bridge_id = Some(identity.bridge_id.clone());
                                *activation.write().await = Some((identity.bridge_id.clone(), session_id.to_vec(), active_config_revision));
                                enumeration_dispatcher.activate(identity.bridge_id.clone(), session_id.to_vec(), active_config_revision).await;
                            }
                            if let Some(ack) = outcome.ack() {
                                queue_outbound(&outbound, make_outbound(&session_id, envelope.correlation_id, envelope::Payload::Ack(ack.clone()))).await?;
                            }
                            continue;
                        }
                        if let Some(envelope::Payload::DirtyReplayRequest(request)) = envelope.payload.as_ref() {
                            if stable_bridge_id.is_none() || !recovery_ready {
                                let response = make_outbound(
                                    &session_id,
                                    envelope.correlation_id,
                                    envelope::Payload::ProtocolError(ProtocolError {
                                        code: ProtocolErrorCode::InvalidMessage as i32,
                                        message: "DirtyReplayRequest before authenticated recovery".to_string(),
                                        fatal: true,
                                        offending_sequence: envelope.sequence,
                                        config_revision: active_config_revision,
                                    }),
                                );
                                if let Err(queue_error) = queue_outbound(&outbound, response).await {
                                    loop_error = Some(queue_error);
                                } else {
                                    loop_error = Some(BootstrapError::Rejected("DirtyReplayRequest before authenticated recovery".to_string()));
                                }
                                break;
                            }
                            let bridge_id = stable_bridge_id.as_ref().unwrap().clone();
                            let request = request.clone();
                            let repository_for_replay = repository.clone();
                            let outbound_for_replay = outbound.clone();
                            let session_id_for_replay = session_id.to_vec();
                            let outcome = session.process(envelope.clone(), move |_| {
                                let repository = repository_for_replay.clone();
                                let outbound = outbound_for_replay.clone();
                                let bridge_id = bridge_id.clone();
                                let session_id = session_id_for_replay.clone();
                                let request = request.clone();
                                async move {
                                    dispatch_replay_request(
                                        &repository,
                                        request,
                                        &bridge_id,
                                        &session_id,
                                        active_config_revision,
                                        &outbound,
                                    )
                                    .await
                                    .map_err(|error| SessionError::HandlerFailed(error.to_string()))
                                }
                            }).await;
                            let outcome = match outcome {
                                Ok(outcome) => outcome,
                                Err(error) => {
                                    let response = make_outbound(
                                        &session_id,
                                        envelope.correlation_id,
                                        envelope::Payload::ProtocolError(ProtocolError {
                                            code: ProtocolErrorCode::Internal as i32,
                                            message: error.to_string(),
                                            fatal: true,
                                            offending_sequence: envelope.sequence,
                                            config_revision: active_config_revision,
                                        }),
                                    );
                                    if let Err(queue_error) = queue_outbound(&outbound, response).await {
                                        loop_error = Some(queue_error);
                                    } else {
                                        loop_error = Some(BootstrapError::Rejected(error.to_string()));
                                    }
                                    break;
                                }
                            };
                            if let Some(protocol_error) = outcome.protocol_error() {
                                let response = make_outbound(
                                    &session_id,
                                    envelope.correlation_id,
                                    envelope::Payload::ProtocolError(protocol_error.clone()),
                                );
                                if let Err(queue_error) = queue_outbound(&outbound, response).await {
                                    loop_error = Some(queue_error);
                                } else {
                                    loop_error = Some(BootstrapError::Rejected(protocol_error.message.clone()));
                                }
                                break;
                            }
                            if let Some(ack) = outcome.ack() {
                                let response = make_outbound(
                                    &session_id,
                                    envelope.correlation_id,
                                    envelope::Payload::Ack(ack.clone()),
                                );
                                if let Err(queue_error) = queue_outbound(&outbound, response).await {
                                    loop_error = Some(queue_error);
                                    break;
                                }
                            }
                            continue;
                        }
                        if matches!(
                            envelope.payload,
                            Some(envelope::Payload::DirtyReplayItem(_) | envelope::Payload::DirtyResyncComplete(_))
                        ) {
                            let response = make_outbound(
                                &session_id,
                                envelope.correlation_id,
                                envelope::Payload::ProtocolError(replay_error(
                                    squaremap_server::dirty_resync::DirtyResyncError::ReplayMismatch,
                                )),
                            );
                            if let Err(queue_error) = queue_outbound(&outbound, response).await {
                                loop_error = Some(queue_error);
                                break;
                            }
                            loop_error = Some(BootstrapError::Rejected("inbound replay item or completion is a protocol violation".to_string()));
                            break;
                        }
                        if let Some(envelope::Payload::WorldResyncRequired(required)) = envelope.payload.as_ref() {
                            if recovery_ready {
                                let _ = repository.reset_world(&WorldId::new(
                                    required.world.as_ref().map_or("", |world| world.namespace.as_str()),
                                    required.world.as_ref().map_or("", |world| world.value.as_str()),
                                    required.world.as_ref().map_or(0, |world| world.epoch),
                                )).await;
                            }
                        }
                        let is_enumeration = matches!(envelope.payload, Some(envelope::Payload::WorldEnumerationItem(_) | envelope::Payload::WorldEnumerationComplete(_)));
                        if is_enumeration {
                            let dispatcher_for_enumeration = enumeration_dispatcher.clone();
                            let outcome = session.process(envelope.clone(), move |frame| {
                                let dispatcher = dispatcher_for_enumeration.clone();
                                let frame = frame.clone();
                                async move {
                                    dispatcher.accept(&frame).await.map(|_| ()).map_err(|error| SessionError::HandlerFailed(format!("{error:?}")))
                                }
                            }).await;
                            let outcome = match outcome {
                                Ok(outcome) => outcome,
                                Err(error) => {
                                    let response = make_outbound(
                                        &session_id,
                                        envelope.correlation_id,
                                        envelope::Payload::ProtocolError(ProtocolError {
                                            code: ProtocolErrorCode::Internal as i32,
                                            message: error.to_string(),
                                            fatal: true,
                                            offending_sequence: envelope.sequence,
                                            config_revision: 0,
                                        }),
                                    );
                                    loop_error = match queue_outbound(&outbound, response).await {
                                        Ok(()) => Some(BootstrapError::Rejected(error.to_string())),
                                        Err(queue_error) => Some(queue_error),
                                    };
                                    break;
                                }
                            };
                            if let Some(protocol_error) = outcome.protocol_error() {
                                let response = make_outbound(
                                    &session_id,
                                    envelope.correlation_id,
                                    envelope::Payload::ProtocolError(protocol_error.clone()),
                                );
                                loop_error = match queue_outbound(&outbound, response).await {
                                    Ok(()) => Some(BootstrapError::Rejected(protocol_error.message.clone())),
                                    Err(queue_error) => Some(queue_error),
                                };
                                break;
                            }
                            if let Some(ack) = outcome.ack() {
                                let response = make_outbound(
                                    &session_id,
                                    envelope.correlation_id,
                                    envelope::Payload::Ack(ack.clone()),
                                );
                                if let Err(queue_error) = queue_outbound(&outbound, response).await {
                                    loop_error = Some(queue_error);
                                    break;
                                }
                            }
                            continue;
                        }
                        let is_snapshot = matches!(
                            envelope.payload,
                            Some(envelope::Payload::RegistryReplace(_)
                                | envelope::Payload::ChunkSnapshot(_)
                                | envelope::Payload::ChunkMissing(_))
                        );
                        let outcome = if matches!(envelope.payload, Some(envelope::Payload::ChunkDirty(_))) {
                            session.process_with_repository(envelope.clone(), repository.clone()).await
                        } else {
                            let handler_root = root.clone();
                            let handler_envelope = envelope.clone();
                            session.process_with_disposition(envelope.clone(), move |_| {
                                let handler_root = handler_root.clone();
                                let handler_envelope = handler_envelope.clone();
                                async move {
                                    if matches!(
                                        handler_envelope.payload,
                                        Some(
                                            envelope::Payload::ControlRequest(_)
                                                | envelope::Payload::ConfigReplace(_)
                                                | envelope::Payload::RegistryReplace(_)
                                                | envelope::Payload::ChunkSnapshot(_)
                                                | envelope::Payload::ChunkMissing(_)
                                        )
                                    ) {
                                        return Ok(());
                                    }
                                    squaremap_server::views::apply_replacement(&handler_root, &handler_envelope)
                                        .map(|_| ())
                                        .map_err(|error| SessionError::HandlerFailed(error.to_string()))
                                }
                            }, |candidate| {
                                let Some(envelope::Payload::ConfigReplace(config)) = candidate.payload.as_ref() else { return None; };
                                squaremap_server::config::validate_and_stage(config.clone()).err().map(|error| ProtocolError {
                                    code: ProtocolErrorCode::InvalidMessage as i32,
                                    message: error.to_string(),
                                    fatal: false,
                                    offending_sequence: candidate.sequence,
                                    config_revision: config.revision,
                                })
                            }).await
                        };
                        let outcome = match outcome {
                            Ok(outcome) => outcome,
                            Err(error) => {
                                let response = make_outbound(
                                    &session_id,
                                    envelope.correlation_id,
                                    envelope::Payload::ProtocolError(ProtocolError {
                                        code: ProtocolErrorCode::Internal as i32,
                                        message: error.to_string(),
                                        fatal: true,
                                        offending_sequence: envelope.sequence,
                                        config_revision: 0,
                                    }),
                                );
                                if let Err(queue_error) = queue_outbound(&outbound, response).await {
                                    loop_error = Some(queue_error);
                                } else {
                                    loop_error = Some(BootstrapError::Rejected(error.to_string()));
                                }
                                break;
                            }
                        };
                        if let Some(protocol_error) = outcome.protocol_error() {
                            let response = make_outbound(
                                &session_id,
                                envelope.correlation_id,
                                envelope::Payload::ProtocolError(protocol_error.clone()),
                            );
                            if let Err(queue_error) = queue_outbound(&outbound, response).await {
                                loop_error = Some(queue_error);
                                break;
                            }
                            if protocol_error.fatal {
                                loop_error = Some(BootstrapError::Rejected(protocol_error.message.clone()));
                                break;
                            }
                        }
                        let accepted = outcome
                            .ack()
                            .is_some_and(|ack| ack.status == squaremap_protocol::wire::AckStatus::Accepted as i32)
                            && outcome.protocol_error().is_none();
                        if accepted && is_snapshot {
                            if let Err(error) = dispatcher.accept(&envelope).await {
                                let response = make_outbound(
                                    &session_id,
                                    envelope.correlation_id,
                                    envelope::Payload::ProtocolError(ProtocolError {
                                        code: ProtocolErrorCode::Internal as i32,
                                        message: error.to_string(),
                                        fatal: true,
                                        offending_sequence: envelope.sequence,
                                        config_revision: 0,
                                    }),
                                );
                                if let Err(queue_error) = queue_outbound(&outbound, response).await {
                                    loop_error = Some(queue_error);
                                } else {
                                    loop_error = Some(BootstrapError::Rejected(error.to_string()));
                                }
                                break;
                            }
                        }

                        let mut control_result = None;
                        let mut policy_result = None;
                        let mut config_error = None;
                        if accepted {
                            if let Some(envelope::Payload::ConfigReplace(config)) = envelope.payload.as_ref() {
                                match squaremap_server::config::validate_and_stage(config.clone()) {
                                    Ok((_, policy)) => {
                                        let configured = async {
                                            sync_repository_worlds(repository.as_ref(), config)
                                                .await
                                                .map_err(|error| error.to_string())?;
                                            let (scheduler_config, background_enabled) =
                                                configure_installer(&installer, config, &policy)?;
                                            let next_scheduler = Arc::new(Scheduler::new(
                                                repository.clone(),
                                                snapshot_bridge.clone(),
                                                installer.clone(),
                                                scheduler_config,
                                            ).map_err(|error| error.to_string())?);
                                            next_scheduler.apply_visibility_from_config(config).map_err(|error| error.to_string())?;
                                            let desired_bind = match config.global.as_ref() {
                                                Some(global) if global.http_enabled => Some(
                                                    format!("{}:{}", global.http_bind, global.http_port)
                                                        .parse()
                                                        .map_err(|error| format!("invalid HTTP bind address: {error}"))?,
                                                ),
                                                _ => None,
                                            };
                                            let reuse_http = desired_bind.is_some()
                                                && http_server.as_ref().and_then(HttpServer::local_addr) == desired_bind;
                                            let next_http = if reuse_http {
                                                None
                                            } else {
                                                match desired_bind {
                                                    Some(bind) => Some(
                                                        HttpServer::bind(
                                                            HttpConfig::enabled_at(bind),
                                                            root.clone(),
                                                        )
                                                        .await
                                                        .map_err(|error| format!("could not start HTTP server: {error}"))?,
                                                    ),
                                                    None => None,
                                                }
                                            };
                                            config_store
                                                .stage_and_swap(config.clone())
                                                .map_err(|error| error.to_string())?;
                                            Ok::<_, String>((policy, next_scheduler, background_enabled, desired_bind, reuse_http, next_http))
                                        }
                                        .await;
                                        match configured {
                                            Ok((policy, next_scheduler, background_enabled, _desired_bind, reuse_http, mut next_http)) => {
                                                if !reuse_http {
                                                    if let Some(mut previous) = http_server.take() {
                                                        if let Err(error) = previous.shutdown().await {
                                                            if let Some(mut next) = next_http.take() {
                                                                let _ = next.shutdown().await;
                                                            }
                                                            config_error = Some(format!("could not stop previous HTTP server: {error}"));
                                                        }
                                                    }
                                                }
                                                if config_error.is_some() {
                                                    if let Some(mut next) = next_http {
                                                        let _ = next.shutdown().await;
                                                    }
                                                    break;
                                                }
                                                if !reuse_http {
                                                    http_server = next_http;
                                                }
                                                if let Some(task) = background_task.take() {
                                                    task.abort();
                                                }
                                                background_task = spawn_background_scheduler(next_scheduler.clone(), background_enabled, activation.clone());
                                                scheduler = Some(next_scheduler);
                                                active_config_revision = config.revision;
                                                if let Some(bridge_id) = stable_bridge_id.as_ref() {
                                                    *activation.write().await = Some((bridge_id.clone(), session_id.to_vec(), active_config_revision));
                                                    enumeration_dispatcher.activate(bridge_id.clone(), session_id.to_vec(), active_config_revision).await;
                                                }
                                                recovery_ready = stable_bridge_id.is_some();
                                                if recovery_ready {
                                                    if let Some(bridge_id) = stable_bridge_id.as_ref() {
                                                        if let Ok(watermark) = dirty_resync::resume_watermark(&repository, bridge_id, &session_id, active_config_revision).await {
                                                            let _ = queue_outbound(&outbound, make_outbound(&session_id, envelope.correlation_id, envelope::Payload::ResumeWatermark(watermark))).await;
                                                        }
                                                    }
                                                }
                                                control_state.replace_worlds(
                                                    config.worlds
                                                        .iter()
                                                        .filter_map(|world| world.identity.clone())
                                                        .collect(),
                                                );
                                                policy_result = Some(policy);
                                                let (http_port, http_is_enabled) = http_server
                                                    .as_ref()
                                                    .and_then(HttpServer::local_addr)
                                                    .map(|address| {
                                                        tracing::info!(http_addr = %address, "Rust HTTP server ready");
                                                        (address.port() as u32, true)
                                                    })
                                                    .unwrap_or((0, false));
                                                let ready = make_outbound(
                                                    &session_id,
                                                    envelope.correlation_id,
                                                    envelope::Payload::Ready(squaremap_protocol::wire::Ready {
                                                        state_revision: config.revision,
                                                        http_port,
                                                        http_enabled: http_is_enabled,
                                                    }),
                                                );
                                                if let Err(error) = queue_outbound(&outbound, ready).await {
                                                    loop_error = Some(error);
                                                    break;
                                                }
                                            }
                                            Err(error) => config_error = Some(error),
                                        }
                                    }
                                    Err(error) => config_error = Some(error.to_string()),
                                }
                            }
                            if let Some(envelope::Payload::ControlRequest(request)) = envelope.payload.as_ref() {
                                let baseline = control_state.handle(request);
                                if baseline.code == BackendResultCode::BackendUnavailable as i32
                                    && matches!(ControlKind::try_from(request.kind), Ok(ControlKind::FullRender | ControlKind::RadiusRender))
                                {
                                    if control_tasks.len() >= 16 {
                                        control_result = Some(control_response(BackendResultCode::BackendUnavailable, request.world.as_ref()));
                                    } else {
                                        let request = request.clone();
                                        let scheduler = scheduler.clone();
                                        let active_jobs = active_jobs.clone();
                                        let root = root.clone();
                                        let revision = active_config_revision;
                                        let correlation_id = envelope.correlation_id;
                                        let result_sender = control_result_sender.clone();
                                        let task = tokio::spawn(async move {
                                            let result = execute_control_request(&request, baseline, scheduler, active_jobs, revision, &root).await;
                                            let _ = result_sender.send((correlation_id, result)).await;
                                        });
                                        control_tasks.insert(correlation_id, task);
                                    }
                                } else {
                                    control_result = Some(execute_control_request(request, baseline, scheduler.clone(), active_jobs.clone(), active_config_revision, &root).await);
                                }
                            }
                        }
                        if let Some(message) = config_error {
                            let error = make_outbound(
                                &session_id,
                                envelope.correlation_id,
                                envelope::Payload::ProtocolError(ProtocolError {
                                    code: ProtocolErrorCode::InvalidMessage as i32,
                                    message,
                                    fatal: false,
                                    offending_sequence: envelope.sequence,
                                    config_revision: match envelope.payload.as_ref() {
                                        Some(envelope::Payload::ConfigReplace(config)) => config.revision,
                                        _ => 0,
                                    },
                                }),
                            );
                            if let Err(queue_error) = queue_outbound(&outbound, error).await {
                                loop_error = Some(queue_error);
                                break;
                            }
                        }
                        if let Some(ack_payload) = outcome.ack() {
                            let ack = make_outbound(
                                &session_id,
                                envelope.correlation_id,
                                envelope::Payload::Ack(ack_payload.clone()),
                            );
                            if let Err(queue_error) = queue_outbound(&outbound, ack).await {
                                loop_error = Some(queue_error);
                                break;
                            }
                        }
                        if let Some(result) = control_result {
                            let response = make_outbound(
                                &session_id,
                                envelope.correlation_id,
                                envelope::Payload::ControlResult(result),
                            );
                            if let Err(queue_error) = queue_outbound(&outbound, response).await {
                                loop_error = Some(queue_error);
                                break;
                            }
                        }
                        if let Some(policy) = policy_result {
                            let response = make_outbound(
                                &session_id,
                                envelope.correlation_id,
                                envelope::Payload::BridgePolicyReplace(policy),
                            );
                            if let Err(queue_error) = queue_outbound(&outbound, response).await {
                                loop_error = Some(queue_error);
                                break;
                            }
                        }
                    }
                    Err(FrameError::EarlyEof { actual: 0, .. }) => break,
                    Err(error) => {
                        loop_error = Some(error.into());
                        break;
                    }
                }
            }
            failure = &mut writer_failure_receiver => {
                loop_error = Some(match failure {
                    Ok(error) => BootstrapError::Io(error),
                    Err(_) => BootstrapError::Io(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "bridge writer stopped",
                    )),
                });
                break;
            }
        }
    }
    for (_, task) in control_tasks.drain() {
        task.abort();
        let _ = task.await;
    }
    shutdown_render_tasks(&active_jobs, scheduler.as_ref()).await;
    enumeration_dispatcher.deactivate().await;
    enumeration_task.abort();
    let _ = enumeration_task.await;
    dispatcher.abort("bridge connection closed").await;
    snapshot_task.abort();
    let _ = snapshot_task.await;
    if let Some(task) = background_task {
        task.abort();
        let _ = task.await;
    }
    drop(snapshot_bridge);
    if let Some(mut server) = http_server.take() {
        let _ = server.shutdown().await;
    }
    drop(dispatcher);
    drop(outbound);
    writer_task.abort();
    let _ = writer_task.await;
    if shutdown_received && matches!(loop_error, Some(BootstrapError::Frame(FrameError::Io(_)))) {
        Ok(())
    } else if let Some(error) = loop_error {
        Err(error)
    } else {
        Ok(())
    }
}

async fn sync_repository_worlds(
    repository: &Repository,
    config: &squaremap_protocol::wire::ConfigReplace,
) -> Result<(), squaremap_state::RepositoryError> {
    let mut current = HashSet::with_capacity(config.worlds.len());
    for configured in &config.worlds {
        let Some(identity) = configured.identity.as_ref() else {
            continue;
        };
        current.insert((
            identity.namespace.clone(),
            identity.value.clone(),
            identity.epoch,
        ));
        repository
            .apply_world(World::new(
                identity.namespace.clone(),
                identity.value.clone(),
                identity.epoch,
                configured.encode_to_vec(),
            ))
            .await?;
    }
    for existing in repository.recover().await?.worlds {
        if !current.contains(&(
            existing.id.namespace.clone(),
            existing.id.value.clone(),
            existing.id.epoch,
        )) {
            repository
                .remove_world(&WorldId::new(
                    existing.id.namespace,
                    existing.id.value,
                    existing.id.epoch,
                ))
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_connect, read_token};
    use std::io::Cursor;
    use std::net::SocketAddr;

    #[test]
    fn accepts_ipv4_loopback_only() {
        assert_eq!(
            parse_connect("127.0.0.1:1234").unwrap(),
            "127.0.0.1:1234".parse::<SocketAddr>().unwrap()
        );
        assert!(parse_connect("192.0.2.1:1234").is_err());
        assert!(parse_connect("0.0.0.0:1234").is_err());
    }

    #[test]
    fn accepts_ipv6_loopback() {
        assert_eq!(
            parse_connect("[::1]:1234").unwrap(),
            "[::1]:1234".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn requires_one_base64_line_of_exactly_32_bytes() {
        let encoded = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let token = read_token(Cursor::new(format!("{encoded}\n"))).unwrap();
        assert_eq!(token.len(), 32);
        assert!(read_token(Cursor::new("AAAA\n".to_string())).is_err());
    }

    #[test]
    fn rejects_overlong_token_line() {
        let line = format!("{}\n", "A".repeat(89));
        assert!(read_token(Cursor::new(line)).is_err());
    }
}
#[cfg(test)]
mod protocol_tests {
    use super::{BootstrapError, EnumerationDispatcher, OutboundEnumerationRequest, run_bridge};
    use serde::Deserialize;
    use squaremap_protocol::wire::{
        AdvancedSettings, BackendResultCode, BridgeIdentityReplace, ChunkCoordinate, ConfigReplace,
        ControlKind, ControlRequest, DirtyReplayRequest, DirtyResyncStatus, Envelope,
        GlobalSettings, HelloAck, LocaleSettings, RenderSettings, ReplayStart, Shutdown,
        ShutdownReason, UiSettings, WorldConfig, WorldEnumerationComplete, WorldEnumerationItem,
        WorldEnumerationRequest, WorldEnumerationStatus, WorldIdentity, WorldSettings,
        dirty_replay_request::Cursor as DirtyReplayCursor, envelope,
    };
    use squaremap_protocol::{FrameLimits, read_envelope, write_envelope};
    use std::io;
    use std::pin::Pin;
    use std::sync::{Arc, LazyLock};
    use std::task::{Context, Poll};
    use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio::sync::{Mutex, OwnedMutexGuard};
    use tokio::time::{Duration, timeout};

    static OUTPUT_ROOT_LOCK: LazyLock<Arc<Mutex<()>>> = LazyLock::new(|| Arc::new(Mutex::new(())));

    struct OutputRootGuard {
        _lock: OwnedMutexGuard<()>,
        previous: Option<std::ffi::OsString>,
    }

    impl OutputRootGuard {
        async fn acquire(path: &std::path::Path) -> Self {
            let lock = OUTPUT_ROOT_LOCK.clone().lock_owned().await;
            let previous = std::env::var_os("SQUAREMAP_OUTPUT_ROOT");
            unsafe {
                std::env::set_var("SQUAREMAP_OUTPUT_ROOT", path);
            }
            Self {
                _lock: lock,
                previous,
            }
        }
    }

    impl Drop for OutputRootGuard {
        fn drop(&mut self) {
            unsafe {
                match self.previous.take() {
                    Some(value) => std::env::set_var("SQUAREMAP_OUTPUT_ROOT", value),
                    None => std::env::remove_var("SQUAREMAP_OUTPUT_ROOT"),
                }
            }
        }
    }
    use zeroize::Zeroizing;

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    struct JobCursor {
        coordinates: Vec<squaremap_state::ChunkCoordinate>,
        next: usize,
        #[serde(default)]
        revision: u64,
    }
    fn config() -> ConfigReplace {
        ConfigReplace {
            revision: 7,
            global: Some(GlobalSettings {
                http_enabled: false,
                http_port: 8080,
                compression_ratio: 1.0,
                ..Default::default()
            }),
            advanced: Some(AdvancedSettings::default()),
            world: Some(WorldSettings {
                zoom_max: 3,
                zoom_default: 3,
                background_render_interval_seconds: 1,
                background_render_max_chunks_per_interval: 1,
                player_tracker_update_interval: 1,
                marker_api_update_interval_seconds: 1,
                ..Default::default()
            }),
            locale: Some(LocaleSettings {
                language: "lang-en.yml".into(),
                ..Default::default()
            }),
            render: Some(RenderSettings {
                progress_logging_interval_seconds: 1,
                background_interval_seconds: 1,
                background_max_chunks_per_interval: 1,
                ..Default::default()
            }),
            ui: Some(UiSettings {
                sidebar_pinned: "unpinned".into(),
                ..Default::default()
            }),
            player_privacy_enabled: Some(false),
            event_capture_enabled: Some(true),
            worlds: vec![WorldConfig {
                identity: Some(WorldIdentity {
                    namespace: "minecraft".to_string(),
                    value: "overworld".to_string(),
                    epoch: 1,
                }),
                settings: Some(WorldSettings {
                    zoom_max: 3,
                    zoom_default: 3,
                    background_render_interval_seconds: 1,
                    background_render_max_chunks_per_interval: 1,
                    player_tracker_update_interval: 1,
                    marker_api_update_interval_seconds: 1,
                    ..Default::default()
                }),
            }],
            ..Default::default()
        }
    }

    fn client_envelope_with_correlation(
        session_id: &[u8],
        sequence: u64,
        correlation_id: u64,
        payload: envelope::Payload,
    ) -> Envelope {
        let mut envelope = client_envelope(session_id, sequence, payload);
        envelope.correlation_id = correlation_id;
        envelope
    }

    fn client_envelope(session_id: &[u8], sequence: u64, payload: envelope::Payload) -> Envelope {
        Envelope {
            protocol_major: 1,
            protocol_minor: 0,
            session_id: session_id.to_vec(),
            sequence,
            correlation_id: sequence,
            payload: Some(payload),
        }
    }

    async fn handshake(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
        let hello = read_envelope(socket, FrameLimits::default()).await.unwrap();
        let session_id = hello.session_id.clone();
        let ack = Envelope {
            protocol_major: 1,
            protocol_minor: 0,
            session_id: session_id.clone(),
            sequence: 1,
            correlation_id: 0,
            payload: Some(envelope::Payload::HelloAck(HelloAck {
                protocol_major: 1,
                protocol_minor: 0,
                backend_version: "fixture".to_string(),
                accepted: true,
                rejection_reason: String::new(),
            })),
        };
        write_envelope(socket, &ack, FrameLimits::default())
            .await
            .unwrap();
        session_id
    }
    #[tokio::test]
    async fn loopback_replay_bootstrap_emits_watermark_items_and_completion() {
        let output_root = tempfile::tempdir().unwrap();
        let _output_root_guard = OutputRootGuard::acquire(output_root.path()).await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let session_id = handshake(&mut socket).await;
            let bridge_id = vec![0x42; 16];
            write_envelope(
                &mut socket,
                &client_envelope(
                    &session_id,
                    1,
                    envelope::Payload::BridgeIdentityReplace(BridgeIdentityReplace {
                        bridge_id: bridge_id.clone(),
                    }),
                ),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            loop {
                let frame = match timeout(
                    Duration::from_secs(5),
                    read_envelope(&mut socket, FrameLimits::default()),
                )
                .await
                {
                    Ok(Ok(frame)) => frame,
                    _ => break,
                };
                if matches!(frame.payload, Some(envelope::Payload::Ack(_))) {
                    break;
                }
            }
            write_envelope(
                &mut socket,
                &client_envelope(&session_id, 2, envelope::Payload::ConfigReplace(config())),
                FrameLimits::default(),
            )
            .await
            .map_err(|_| ())
            .ok();

            let mut saw_watermark = false;
            let mut saw_ready = false;
            loop {
                let frame = match timeout(
                    Duration::from_secs(5),
                    read_envelope(&mut socket, FrameLimits::default()),
                )
                .await
                {
                    Ok(Ok(frame)) => frame,
                    _ => break,
                };
                match frame.payload {
                    Some(envelope::Payload::ResumeWatermark(watermark)) => {
                        assert_eq!(watermark.bridge_id, bridge_id);
                        assert_eq!(watermark.config_revision, 7);
                        saw_watermark = true;
                    }
                    Some(envelope::Payload::Ready(ready)) => {
                        assert_eq!(ready.state_revision, 7);
                        saw_ready = true;
                    }
                    Some(envelope::Payload::Ack(ack)) if ack.acknowledged_sequence == 2 => {
                        break;
                    }
                    _ => {}
                }
            }
            assert!(saw_watermark);
            assert!(saw_ready);

            write_envelope(
                &mut socket,
                &client_envelope(
                    &session_id,
                    3,
                    envelope::Payload::DirtyReplayRequest(DirtyReplayRequest {
                        session_id: session_id.clone(),
                        config_revision: 7,
                        replay_id: 19,
                        world: Some(WorldIdentity {
                            namespace: "minecraft".to_string(),
                            value: "overworld".to_string(),
                            epoch: 1,
                        }),
                        max_items: 2,
                        bridge_id,
                        cursor: Some(DirtyReplayCursor::Start(ReplayStart {})),
                    }),
                ),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            let mut item_indices = Vec::new();
            let mut complete = None;
            let mut ack_seen = false;
            while complete.is_none() || !ack_seen {
                let frame = timeout(
                    Duration::from_secs(5),
                    read_envelope(&mut socket, FrameLimits::default()),
                )
                .await
                .unwrap()
                .unwrap();
                match frame.payload {
                    Some(envelope::Payload::DirtyReplayItem(item)) => {
                        assert_eq!(item.replay_id, 19);
                        item_indices.push(item.item_index);
                    }
                    Some(envelope::Payload::DirtyResyncComplete(done)) => {
                        assert_eq!(done.replay_id, 19);
                        assert_eq!(done.status, DirtyResyncStatus::Complete as i32);
                        complete = Some(done.item_count);
                    }
                    Some(envelope::Payload::Ack(ack)) if ack.acknowledged_sequence == 3 => {
                        ack_seen = true;
                    }
                    _ => {}
                }
            }
            assert!(item_indices.windows(2).all(|window| window[0] < window[1]));
            assert!(item_indices.len() <= 2);
            assert_eq!(complete, Some(item_indices.len() as u32));

            write_envelope(
                &mut socket,
                &client_envelope(
                    &session_id,
                    4,
                    envelope::Payload::DirtyReplayItem(Default::default()),
                ),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            let late = timeout(
                Duration::from_secs(5),
                read_envelope(&mut socket, FrameLimits::default()),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(
                matches!(&late.payload, Some(envelope::Payload::ProtocolError(error)) if error.fatal),
                "inbound DirtyReplayItem must be rejected with a fatal protocol error"
            );
            let _ = socket.shutdown().await;
        });
        let result = run_bridge(address, "fixture", Zeroizing::new(vec![0; 32])).await;
        assert!(
            matches!(result, Err(BootstrapError::Rejected(_))),
            "server must close with a rejected fatal error after an inbound replay payload: {result:?}"
        );
        server.await.unwrap();
    }
    #[tokio::test]
    async fn framed_identity_duplicate_and_gap_are_fail_closed() {
        let output_root = tempfile::tempdir().unwrap();
        let _output_root_guard = OutputRootGuard::acquire(output_root.path()).await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let _server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let session_id = handshake(&mut socket).await;
            let bridge_id = vec![0x31; 16];
            let identity = client_envelope(
                &session_id,
                1,
                envelope::Payload::BridgeIdentityReplace(BridgeIdentityReplace {
                    bridge_id: bridge_id.clone(),
                }),
            );
            write_envelope(&mut socket, &identity, FrameLimits::default())
                .await
                .unwrap();
            let accepted = read_envelope(&mut socket, FrameLimits::default())
                .await
                .unwrap();
            assert!(
                matches!(&accepted.payload, Some(envelope::Payload::Ack(ack)) if ack.acknowledged_sequence == 1 && ack.status == squaremap_protocol::wire::AckStatus::Accepted as i32)
            );
            write_envelope(&mut socket, &identity, FrameLimits::default())
                .await
                .unwrap();
            let duplicate = read_envelope(&mut socket, FrameLimits::default())
                .await
                .unwrap();
            assert!(
                matches!(&duplicate.payload, Some(envelope::Payload::Ack(ack)) if ack.acknowledged_sequence == 1 && ack.status == squaremap_protocol::wire::AckStatus::Duplicate as i32)
            );
            write_envelope(
                &mut socket,
                &client_envelope(
                    &session_id,
                    3,
                    envelope::Payload::BridgeIdentityReplace(BridgeIdentityReplace {
                        bridge_id: vec![0x32; 16],
                    }),
                ),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            let gap = read_envelope(&mut socket, FrameLimits::default())
                .await
                .unwrap();
            assert!(
                matches!(&gap.payload, Some(envelope::Payload::ProtocolError(error)) if error.fatal && error.offending_sequence == 3)
            );
            let _ = socket.shutdown().await;
        });
        let result = run_bridge(address, "fixture", Zeroizing::new(vec![0; 32])).await;
        assert!(result.is_err());
        let repository =
            squaremap_state::Repository::open(output_root.path().join(".squaremap-state.sqlite"))
                .await
                .unwrap();
        assert_eq!(
            repository.bridge_identity().await.unwrap(),
            Some(vec![0x31; 16])
        );
        let checkpoint = repository
            .bridge_checkpoint(&[0x31; 16])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(checkpoint.durable_sequence, 1);
        assert!(
            repository
                .bridge_checkpoint(&[0x32; 16])
                .await
                .unwrap()
                .is_none()
        );
    }
    #[tokio::test]
    async fn framed_enumeration_duplicate_and_gap_are_fail_closed() {
        let dispatcher = EnumerationDispatcher::new();
        let (outbound, _receiver) = tokio::sync::mpsc::channel(4);
        dispatcher.set_outbound(outbound).await;
        let session_id = vec![0x41; 16];
        let bridge_id = vec![0x42; 16];
        dispatcher
            .activate(bridge_id.clone(), session_id.clone(), 7)
            .await;
        let (_response, _cancel) = {
            let (response_tx, response_rx) = tokio::sync::oneshot::channel();
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
            let request = OutboundEnumerationRequest {
                request: WorldEnumerationRequest {
                    world: Some(WorldIdentity {
                        namespace: "minecraft".into(),
                        value: "overworld".into(),
                        epoch: 1,
                    }),
                    max_items: 1024,
                    ..Default::default()
                },
                response: response_tx,
                cancellation: cancel_rx,
            };
            let _outbound = dispatcher
                .begin(request, &session_id, &bridge_id, 7, 1)
                .await
                .unwrap();
            (response_rx, cancel_tx)
        };
        let item = Envelope {
            correlation_id: 1,
            payload: Some(envelope::Payload::WorldEnumerationItem(
                WorldEnumerationItem {
                    session_id: session_id.clone(),
                    bridge_id: bridge_id.clone(),
                    config_revision: 7,
                    enumeration_id: 1,
                    item_index: 0,
                    page_index: 0,
                    world: Some(WorldIdentity {
                        namespace: "minecraft".into(),
                        value: "overworld".into(),
                        epoch: 1,
                    }),
                    coordinate: Some(ChunkCoordinate { x: 1, z: 2 }),
                },
            )),
            ..Default::default()
        };
        assert!(dispatcher.accept(&item).await.unwrap());
        assert!(dispatcher.accept(&item).await.is_err());
    }
    #[tokio::test]
    async fn enumeration_dispatch_rejects_inactive_and_mismatched_sessions_without_hanging() {
        let dispatcher = Arc::new(EnumerationDispatcher::new());
        let (requests, request_rx) = tokio::sync::mpsc::channel(4);
        let (outbound, mut outbound_rx) = tokio::sync::mpsc::channel(4);
        let activation = Arc::new(tokio::sync::RwLock::new(None));
        let task = tokio::spawn(super::dispatch_enumeration_requests(
            request_rx,
            dispatcher.clone(),
            outbound,
            vec![1; 16],
            activation.clone(),
        ));
        let (response, result) = oneshot::channel();
        let (cancel_sender, cancellation) = oneshot::channel();
        requests
            .send(OutboundEnumerationRequest {
                request: WorldEnumerationRequest {
                    world: Some(WorldIdentity {
                        namespace: "minecraft".into(),
                        value: "overworld".into(),
                        epoch: 1,
                    }),
                    max_items: 1024,
                    ..Default::default()
                },
                response,
                cancellation,
            })
            .await
            .unwrap();
        assert!(
            matches!(timeout(Duration::from_secs(1), result).await.unwrap().unwrap(), Err(super::BridgeError::Transient(message)) if message == "bridge is not active")
        );
        let _ = cancel_sender.send(());

        *activation.write().await = Some((vec![2; 16], vec![9; 16], 7));
        let (response, result) = oneshot::channel();
        requests
            .send(OutboundEnumerationRequest {
                request: WorldEnumerationRequest {
                    world: Some(WorldIdentity {
                        namespace: "minecraft".into(),
                        value: "overworld".into(),
                        epoch: 1,
                    }),
                    max_items: 1024,
                    ..Default::default()
                },
                response,
                cancellation: oneshot::channel().1,
            })
            .await
            .unwrap();
        assert!(
            matches!(timeout(Duration::from_secs(1), result).await.unwrap().unwrap(), Err(super::BridgeError::Transient(message)) if message == "bridge session mismatch")
        );
        let (response, result) = oneshot::channel();
        let (_cancel_sender, cancellation) = oneshot::channel();
        requests
            .send(OutboundEnumerationRequest {
                request: WorldEnumerationRequest {
                    world: Some(WorldIdentity {
                        namespace: "minecraft".into(),
                        value: "overworld".into(),
                        epoch: 1,
                    }),
                    max_items: 1024,
                    ..Default::default()
                },
                response,
                cancellation,
            })
            .await
            .unwrap();
        *activation.write().await = Some((vec![2; 16], vec![1; 16], 7));
        let envelope = timeout(Duration::from_secs(1), outbound_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            envelope.payload,
            Some(envelope::Payload::WorldEnumerationRequest(_))
        ));
        dispatcher.abort("test cleanup").await;
        assert!(
            matches!(timeout(Duration::from_secs(1), result).await.unwrap().unwrap(), Err(super::BridgeError::Transient(message)) if message == "test cleanup")
        );
        task.abort();
    }
    #[tokio::test]
    async fn enumeration_dispatch_closed_outbound_resolves_pending_request() {
        let dispatcher = Arc::new(EnumerationDispatcher::new());
        let (requests, request_rx) = tokio::sync::mpsc::channel(1);
        let (outbound, outbound_rx) = tokio::sync::mpsc::channel(1);
        drop(outbound_rx);
        let activation = Arc::new(tokio::sync::RwLock::new(Some((
            vec![2; 16],
            vec![1; 16],
            7,
        ))));
        let task = tokio::spawn(super::dispatch_enumeration_requests(
            request_rx,
            dispatcher.clone(),
            outbound,
            vec![1; 16],
            activation,
        ));
        let (response, result) = oneshot::channel();
        requests
            .send(OutboundEnumerationRequest {
                request: WorldEnumerationRequest {
                    world: Some(WorldIdentity {
                        namespace: "minecraft".into(),
                        value: "overworld".into(),
                        epoch: 1,
                    }),
                    max_items: 1024,
                    ..Default::default()
                },
                response,
                cancellation: oneshot::channel().1,
            })
            .await
            .unwrap();
        assert!(
            matches!(timeout(Duration::from_secs(1), result).await.unwrap().unwrap(), Err(super::BridgeError::Transient(message)) if message == "enumeration outbound closed")
        );
        assert_eq!(dispatcher.in_flight().await, 0);
        task.abort();
    }
    #[tokio::test]
    async fn malformed_enumeration_item_tears_down_http_background_and_render_tasks() {
        let output_root = tempfile::tempdir().unwrap();
        let output_path = output_root.path().to_path_buf();
        let _output_root_guard = OutputRootGuard::acquire(&output_path).await;
        let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_address = http_listener.local_addr().unwrap();
        drop(http_listener);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let session_id = handshake(&mut socket).await;
            let bridge_id = vec![0x42; 16];
            write_envelope(
                &mut socket,
                &client_envelope(
                    &session_id,
                    1,
                    envelope::Payload::BridgeIdentityReplace(BridgeIdentityReplace {
                        bridge_id: bridge_id.clone(),
                    }),
                ),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            loop {
                let frame = timeout(
                    Duration::from_secs(5),
                    read_envelope(&mut socket, FrameLimits::default()),
                )
                .await
                .unwrap()
                .unwrap();
                if matches!(frame.payload, Some(envelope::Payload::Ack(_))) {
                    break;
                }
            }
            let mut configured = config();
            let global = configured.global.as_mut().unwrap();
            global.http_enabled = true;
            global.http_bind = "127.0.0.1".into();
            global.http_port = http_address.port() as u32;
            write_envelope(
                &mut socket,
                &client_envelope(&session_id, 2, envelope::Payload::ConfigReplace(configured)),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            let mut ready = None;
            while ready.is_none() {
                let frame = timeout(
                    Duration::from_secs(10),
                    read_envelope(&mut socket, FrameLimits::default()),
                )
                .await
                .unwrap()
                .unwrap();
                if let Some(payload) = frame.payload {
                    match payload {
                        envelope::Payload::Ready(payload) => ready = Some(payload),
                        envelope::Payload::ProtocolError(error) => {
                            panic!("config protocol error: {error:?}")
                        }
                        _ => {}
                    }
                }
            }
            let ready = ready.unwrap();
            assert_eq!(ready.http_port, http_address.port() as u32);
            write_envelope(
                &mut socket,
                &client_envelope(
                    &session_id,
                    3,
                    envelope::Payload::ControlRequest(ControlRequest {
                        kind: ControlKind::FullRender as i32,
                        world: Some(WorldIdentity {
                            namespace: "minecraft".into(),
                            value: "overworld".into(),
                            epoch: 1,
                        }),
                        ..Default::default()
                    }),
                ),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            let request = loop {
                let frame = timeout(
                    Duration::from_secs(10),
                    read_envelope(&mut socket, FrameLimits::default()),
                )
                .await
                .unwrap()
                .unwrap();
                match frame.payload {
                    Some(envelope::Payload::WorldEnumerationRequest(request)) => {
                        break (frame.correlation_id, request);
                    }
                    Some(envelope::Payload::ProtocolError(error)) => {
                        panic!("render protocol error: {error:?}")
                    }
                    _ => {}
                }
            };
            let (correlation, request) = request;
            let malformed = WorldEnumerationItem {
                session_id: session_id.clone(),
                bridge_id,
                config_revision: 7,
                enumeration_id: request.enumeration_id,
                item_index: 1,
                page_index: request.page_index,
                world: request.world,
                coordinate: Some(ChunkCoordinate { x: 1, z: 2 }),
            };
            write_envelope(
                &mut socket,
                &client_envelope_with_correlation(
                    &session_id,
                    4,
                    correlation,
                    envelope::Payload::WorldEnumerationItem(malformed.clone()),
                ),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            let frame = timeout(
                Duration::from_secs(10),
                read_envelope(&mut socket, FrameLimits::default()),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(
                matches!(frame.payload, Some(envelope::Payload::ProtocolError(error)) if error.fatal && error.code == squaremap_protocol::wire::ProtocolErrorCode::Internal as i32)
            );
            socket.shutdown().await.unwrap();
        });
        let bridge = timeout(
            Duration::from_secs(20),
            run_bridge(address, "fixture", Zeroizing::new(vec![0; 32])),
        )
        .await
        .expect("run_bridge did not terminate after malformed enumeration");
        let client_result = timeout(Duration::from_secs(10), client)
            .await
            .expect("client did not finish")
            .unwrap();
        let _ = client_result;
        assert!(matches!(bridge, Err(BootstrapError::Rejected(_))));
        assert!(std::net::TcpListener::bind(http_address).is_ok());
    }
    #[tokio::test]
    async fn accepted_handshake_stays_alive_until_shutdown() {
        let output_root = tempfile::tempdir().unwrap();
        let _output_root_guard = OutputRootGuard::acquire(output_root.path()).await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let hello = read_envelope(&mut socket, FrameLimits::default())
                .await
                .unwrap();
            let ack = Envelope {
                protocol_major: 1,
                protocol_minor: 0,
                session_id: hello.session_id.clone(),
                sequence: 1,
                correlation_id: 0,
                payload: Some(envelope::Payload::HelloAck(HelloAck {
                    protocol_major: 1,
                    protocol_minor: 0,
                    backend_version: "fixture".to_string(),
                    accepted: true,
                    rejection_reason: String::new(),
                })),
            };
            write_envelope(&mut socket, &ack, FrameLimits::default())
                .await
                .unwrap();
            let shutdown = Envelope {
                protocol_major: 1,
                protocol_minor: 0,
                session_id: hello.session_id,
                sequence: 2,
                correlation_id: 0,
                payload: Some(envelope::Payload::Shutdown(Shutdown {
                    reason: ShutdownReason::Requested as i32,
                    message: String::new(),
                })),
            };
            write_envelope(&mut socket, &shutdown, FrameLimits::default())
                .await
                .unwrap();
            socket.shutdown().await.unwrap();
            let mut buffer = [0_u8; 1];
            loop {
                match timeout(
                    Duration::from_secs(10),
                    tokio::io::AsyncReadExt::read(&mut socket, &mut buffer),
                )
                .await
                {
                    Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
                    Ok(Ok(_)) => {}
                }
            }
        });
        let result = run_bridge(address, "fixture", Zeroizing::new(vec![0; 32])).await;
        server.await.unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn rejected_ack_fails_bootstrap() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let hello = read_envelope(&mut socket, FrameLimits::default())
                .await
                .unwrap();
            let ack = Envelope {
                protocol_major: 1,
                protocol_minor: 0,
                session_id: hello.session_id,
                sequence: 2,
                correlation_id: 0,
                payload: Some(envelope::Payload::HelloAck(HelloAck {
                    protocol_major: 1,
                    protocol_minor: 0,
                    backend_version: String::new(),
                    accepted: false,
                    rejection_reason: "rejected by fixture".to_string(),
                })),
            };
            write_envelope(&mut socket, &ack, FrameLimits::default())
                .await
                .unwrap();
        });
        let result = run_bridge(address, "fixture", Zeroizing::new(vec![0; 32])).await;
        server.await.unwrap();
        assert!(matches!(result, Err(BootstrapError::Rejected(_))));
    }
    struct FailingWriter;

    impl AsyncWrite for FailingWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            _buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "fixture failure",
            )))
        }

        fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn post_config_paginated_full_render_bootstrap_persists_complete_job() {
        let output_root = tempfile::tempdir().unwrap();
        let output_path = output_root.path().to_path_buf();
        let _output_root_guard = OutputRootGuard::acquire(&output_path).await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (control_started_tx, mut control_started_rx) = oneshot::channel();
        let (assertion_complete_tx, assertion_complete_rx) = oneshot::channel();
        let mut client = tokio::spawn(async move {
            let (mut socket, _) = listener
                .accept()
                .await
                .map_err(|error| format!("client accept failed: {error}"))?;
            let session_id = handshake(&mut socket).await;
            let bridge_id = vec![0x42; 16];
            write_envelope(
                &mut socket,
                &client_envelope(
                    &session_id,
                    1,
                    envelope::Payload::BridgeIdentityReplace(BridgeIdentityReplace {
                        bridge_id: bridge_id.clone(),
                    }),
                ),
                FrameLimits::default(),
            )
            .await
            .map_err(|error| format!("identity write failed: {error}"))?;
            loop {
                let frame = timeout(
                    Duration::from_secs(5),
                    read_envelope(&mut socket, FrameLimits::default()),
                )
                .await
                .map_err(|_| "identity ack timed out".to_string())?
                .map_err(|error| format!("identity ack read failed: {error}"))?;
                if let Some(envelope::Payload::ProtocolError(error)) = frame.payload {
                    return Err(format!("identity protocol error: {error:?}"));
                }
                if let Some(envelope::Payload::Ack(ack)) = frame.payload {
                    assert_eq!(ack.acknowledged_sequence, 1);
                    break;
                }
            }
            write_envelope(
                &mut socket,
                &client_envelope(&session_id, 2, envelope::Payload::ConfigReplace(config())),
                FrameLimits::default(),
            )
            .await
            .map_err(|error| format!("config write failed: {error}"))?;
            let mut ready = false;
            while !ready {
                let frame = timeout(
                    Duration::from_secs(15),
                    read_envelope(&mut socket, FrameLimits::default()),
                )
                .await
                .map_err(|_| "ready timed out".to_string())?
                .map_err(|error| format!("ready read failed: {error}"))?;
                match frame.payload.as_ref() {
                    Some(envelope::Payload::ProtocolError(error)) => {
                        return Err(format!("config protocol error: {error:?}"));
                    }
                    Some(envelope::Payload::Ack(ack)) => assert_eq!(ack.acknowledged_sequence, 2),
                    _ => {}
                }
                if let Some(envelope::Payload::Ready(payload)) = frame.payload {
                    assert_eq!(payload.state_revision, 7);
                    ready = true;
                }
            }
            let control_id = 3;
            write_envelope(
                &mut socket,
                &client_envelope(
                    &session_id,
                    control_id,
                    envelope::Payload::ControlRequest(ControlRequest {
                        kind: ControlKind::FullRender as i32,
                        world: Some(WorldIdentity {
                            namespace: "minecraft".into(),
                            value: "overworld".into(),
                            epoch: 1,
                        }),
                        ..Default::default()
                    }),
                ),
                FrameLimits::default(),
            )
            .await
            .map_err(|error| format!("control write failed: {error}"))?;
            let mut request = None;
            while request.is_none() {
                let frame = timeout(
                    Duration::from_secs(5),
                    read_envelope(&mut socket, FrameLimits::default()),
                )
                .await
                .map_err(|_| "enumeration request timed out".to_string())?
                .map_err(|error| format!("enumeration request read failed: {error}"))?;
                if let Some(envelope::Payload::WorldEnumerationRequest(payload)) = frame.payload {
                    request = Some((frame.correlation_id, payload));
                } else if let Some(envelope::Payload::ProtocolError(error)) = frame.payload {
                    return Err(format!("enumeration protocol error: {error:?}"));
                }
            }
            let (correlation, first) = request.unwrap();
            assert_eq!(first.config_revision, 7);
            assert_eq!(first.page_index, 0);
            assert_eq!(first.max_items, 1024);
            for index in 0..1024_u32 {
                if index % 128 == 0 {
                    tokio::task::yield_now().await;
                }
                write_envelope(
                    &mut socket,
                    &client_envelope_with_correlation(
                        &session_id,
                        4 + index as u64,
                        first.enumeration_id,
                        envelope::Payload::WorldEnumerationItem(WorldEnumerationItem {
                            session_id: session_id.clone(),
                            bridge_id: bridge_id.clone(),
                            config_revision: 7,
                            enumeration_id: first.enumeration_id,
                            item_index: index,
                            page_index: 0,
                            world: first.world.clone(),
                            coordinate: Some(ChunkCoordinate {
                                x: index as i32,
                                z: 0,
                            }),
                        }),
                    ),
                    FrameLimits::default(),
                )
                .await
                .map_err(|error| format!("page 0 item write failed: {error}"))?;
            }
            write_envelope(
                &mut socket,
                &client_envelope_with_correlation(
                    &session_id,
                    1028,
                    first.enumeration_id,
                    envelope::Payload::WorldEnumerationComplete(WorldEnumerationComplete {
                        session_id: session_id.clone(),
                        bridge_id: bridge_id.clone(),
                        config_revision: 7,
                        enumeration_id: first.enumeration_id,
                        item_count: 1024,
                        status: WorldEnumerationStatus::Complete as i32,
                        page_index: 0,
                        has_more: true,
                        failure_reason: String::new(),
                    }),
                ),
                FrameLimits::default(),
            )
            .await
            .map_err(|error| format!("page 0 complete write failed: {error}"))?;
            let second = loop {
                let frame = timeout(
                    Duration::from_secs(5),
                    read_envelope(&mut socket, FrameLimits::default()),
                )
                .await
                .map_err(|_| "page 1 request timed out".to_string())?
                .map_err(|error| format!("page 1 request read failed: {error}"))?;
                if let Some(envelope::Payload::ProtocolError(error)) = frame.payload.as_ref() {
                    return Err(format!("page 1 protocol error: {error:?}"));
                }
                if matches!(frame.payload, Some(envelope::Payload::Ack(_))) {
                    continue;
                }
                if let Some(envelope::Payload::WorldEnumerationRequest(payload)) = frame.payload {
                    break payload;
                }
            };
            assert_eq!(second.enumeration_id, correlation);
            assert_eq!(second.page_index, 1);
            for index in 0..2_u32 {
                write_envelope(
                    &mut socket,
                    &client_envelope_with_correlation(
                        &session_id,
                        1029 + index as u64,
                        second.enumeration_id,
                        envelope::Payload::WorldEnumerationItem(WorldEnumerationItem {
                            session_id: session_id.clone(),
                            bridge_id: bridge_id.clone(),
                            config_revision: 7,
                            enumeration_id: second.enumeration_id,
                            item_index: index,
                            page_index: 1,
                            world: second.world.clone(),
                            coordinate: Some(ChunkCoordinate {
                                x: 1024 + index as i32,
                                z: 0,
                            }),
                        }),
                    ),
                    FrameLimits::default(),
                )
                .await
                .map_err(|error| format!("page 1 item write failed: {error}"))?;
            }
            write_envelope(
                &mut socket,
                &client_envelope_with_correlation(
                    &session_id,
                    1031,
                    second.enumeration_id,
                    envelope::Payload::WorldEnumerationComplete(WorldEnumerationComplete {
                        session_id: session_id.clone(),
                        bridge_id: bridge_id.clone(),
                        config_revision: 7,
                        enumeration_id: second.enumeration_id,
                        item_count: 2,
                        status: WorldEnumerationStatus::Complete as i32,
                        page_index: 1,
                        has_more: false,
                        failure_reason: String::new(),
                    }),
                ),
                FrameLimits::default(),
            )
            .await
            .map_err(|error| format!("page 1 complete write failed: {error}"))?;
            loop {
                let frame = timeout(
                    Duration::from_secs(5),
                    read_envelope(&mut socket, FrameLimits::default()),
                )
                .await
                .map_err(|_| "control result timed out".to_string())?
                .map_err(|error| format!("control result read failed: {error}"))?;
                if frame.correlation_id == control_id {
                    if let Some(envelope::Payload::ControlResult(result)) = frame.payload {
                        assert_eq!(result.code, BackendResultCode::FullRenderStarted as i32);
                        control_started_tx
                            .send(())
                            .map_err(|_| "parent dropped control_started".to_string())?;
                        assertion_complete_rx
                            .await
                            .map_err(|_| "parent dropped assertion_complete".to_string())?;
                        let shutdown = client_envelope(
                            &session_id,
                            1032,
                            envelope::Payload::Shutdown(Shutdown {
                                reason: ShutdownReason::Requested as i32,
                                message: String::new(),
                            }),
                        );
                        write_envelope(&mut socket, &shutdown, FrameLimits::default())
                            .await
                            .map_err(|error| format!("shutdown write failed: {error}"))?;
                        socket
                            .shutdown()
                            .await
                            .map_err(|error| format!("shutdown half-close failed: {error}"))?;
                        let mut buffer = [0_u8; 1];
                        let _ = timeout(
                            Duration::from_secs(10),
                            tokio::io::AsyncReadExt::read(&mut socket, &mut buffer),
                        )
                        .await;
                        break;
                    }
                }
            }
            Ok::<(), String>(())
        });
        let mut bridge = tokio::spawn(run_bridge(address, "fixture", Zeroizing::new(vec![0; 32])));
        let started = timeout(Duration::from_secs(30), async {
            tokio::select! {
                result = &mut control_started_rx => result.map_err(|_| "client dropped control_started".to_string()),
                result = &mut bridge => Err(format!("bridge exited before ControlResult: {result:?}")),
                result = &mut client => Err(format!("client exited before ControlResult: {result:?}")),
            }
        }).await;
        started
            .expect("control_started wait timed out")
            .expect("control path failed");
        let repository =
            squaremap_state::Repository::open(output_root.path().join(".squaremap-state.sqlite"))
                .await
                .unwrap();
        let recovered = repository.recover().await.unwrap();
        assert_eq!(recovered.jobs.len(), 1);
        assert_eq!(recovered.jobs[0].kind, squaremap_state::JobKind::Full);
        let payload: JobCursor = serde_json::from_slice(&recovered.jobs[0].payload).unwrap();
        assert_eq!(
            payload.coordinates,
            (0..1026)
                .map(|x| squaremap_state::ChunkCoordinate { x, z: 0 })
                .collect::<Vec<_>>()
        );
        assert_eq!(payload.revision, 7);
        assert!(payload.next <= payload.coordinates.len());
        assertion_complete_tx.send(()).unwrap();
        let client_result = timeout(Duration::from_secs(10), client)
            .await
            .expect("client shutdown timed out")
            .expect("client task panicked");
        client_result.expect("client failed during protocol test");
        let bridge_result = timeout(Duration::from_secs(30), bridge)
            .await
            .expect("bridge shutdown timed out")
            .expect("bridge task panicked");
        bridge_result.expect("run_bridge failed");
    }
    #[tokio::test]
    async fn framed_invalid_config_emits_error_ack_and_accepts_next_sequence() {
        let output_root = tempfile::tempdir().unwrap();
        let _output_root_guard = OutputRootGuard::acquire(output_root.path()).await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let session_id = handshake(&mut socket).await;

            let mut initial = config();
            initial.revision = 1;
            write_envelope(
                &mut socket,
                &client_envelope(
                    &session_id,
                    1,
                    envelope::Payload::ConfigReplace(initial.clone()),
                ),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            let mut initial_ready = None;
            let mut initial_ack = false;
            while initial_ready.is_none() || !initial_ack {
                let frame = timeout(
                    Duration::from_secs(10),
                    read_envelope(&mut socket, FrameLimits::default()),
                )
                .await
                .unwrap()
                .unwrap();
                match frame.payload {
                    Some(envelope::Payload::Ready(ready)) if ready.state_revision == 1 => {
                        initial_ready = Some(ready)
                    }
                    Some(envelope::Payload::Ack(ack)) if ack.acknowledged_sequence == 1 => {
                        initial_ack = true
                    }
                    _ => {}
                }
            }

            let mut invalid = initial.clone();
            invalid.revision = 2;
            invalid.worlds[0].identity = None;
            write_envelope(
                &mut socket,
                &client_envelope(&session_id, 2, envelope::Payload::ConfigReplace(invalid)),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            let mut invalid_error = None;
            let mut invalid_ack = false;
            while invalid_error.is_none() || !invalid_ack {
                let frame = timeout(
                    Duration::from_secs(10),
                    read_envelope(&mut socket, FrameLimits::default()),
                )
                .await
                .unwrap()
                .unwrap();
                match frame.payload {
                    Some(envelope::Payload::ProtocolError(error)) if error.config_revision == 2 => {
                        assert!(!error.fatal);
                        invalid_error = Some(error);
                    }
                    Some(envelope::Payload::Ack(ack)) if ack.acknowledged_sequence == 2 => {
                        assert_eq!(
                            ack.status,
                            squaremap_protocol::wire::AckStatus::Accepted as i32
                        );
                        invalid_ack = true;
                    }
                    _ => {}
                }
            }
            assert!(invalid_error.is_some());

            write_envelope(
                &mut socket,
                &client_envelope(
                    &session_id,
                    3,
                    envelope::Payload::Shutdown(Shutdown {
                        reason: ShutdownReason::Requested as i32,
                        message: String::new(),
                    }),
                ),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            socket.shutdown().await.unwrap();
        });
        let result = run_bridge(address, "fixture", Zeroizing::new(vec![0; 32])).await;
        assert!(result.is_ok());
        server.await.unwrap();
    }
    #[tokio::test]
    async fn framed_http_config_reload_reuses_and_handoffs_listener() {
        let output_root = tempfile::tempdir().unwrap();
        let _output_root_guard = OutputRootGuard::acquire(output_root.path()).await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let session_id = handshake(&mut socket).await;
            let bridge_id = vec![0x52; 16];
            write_envelope(
                &mut socket,
                &client_envelope(
                    &session_id,
                    1,
                    envelope::Payload::BridgeIdentityReplace(BridgeIdentityReplace { bridge_id }),
                ),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            loop {
                let frame = timeout(
                    Duration::from_secs(5),
                    read_envelope(&mut socket, FrameLimits::default()),
                )
                .await
                .unwrap()
                .unwrap();
                if matches!(frame.payload, Some(envelope::Payload::Ack(ack)) if ack.acknowledged_sequence == 1)
                {
                    break;
                }
            }
            let mut initial = config();
            let initial_port = TcpListener::bind("127.0.0.1:0")
                .await
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            initial.global.as_mut().unwrap().http_enabled = true;
            initial.global.as_mut().unwrap().http_bind = "127.0.0.1".into();
            initial.global.as_mut().unwrap().http_port = initial_port as u32;
            write_envelope(
                &mut socket,
                &client_envelope(
                    &session_id,
                    2,
                    envelope::Payload::ConfigReplace(initial.clone()),
                ),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            let ready = read_ready(&mut socket, 7).await;
            assert!(ready.http_enabled);
            assert_eq!(ready.http_port, initial_port as u32);
            assert_http_serves(initial_port).await;

            let mut same = initial.clone();
            same.revision = 8;
            write_envelope(
                &mut socket,
                &client_envelope(&session_id, 3, envelope::Payload::ConfigReplace(same)),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            let ready = read_ready(&mut socket, 8).await;
            assert!(ready.http_enabled);
            assert_eq!(ready.http_port, initial_port as u32);
            assert_http_serves(initial_port).await;

            let next_port = TcpListener::bind("127.0.0.1:0")
                .await
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            let mut changed = initial.clone();
            changed.revision = 9;
            changed.global.as_mut().unwrap().http_port = next_port as u32;
            write_envelope(
                &mut socket,
                &client_envelope(&session_id, 4, envelope::Payload::ConfigReplace(changed)),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            let ready = read_ready(&mut socket, 9).await;
            assert!(ready.http_enabled);
            assert_eq!(ready.http_port, next_port as u32);
            assert!(std::net::TcpListener::bind(("127.0.0.1", initial_port)).is_ok());
            assert_http_serves(next_port).await;

            let mut disabled = config();
            disabled.revision = 10;
            disabled.global.as_mut().unwrap().http_bind = "127.0.0.1".into();
            disabled.global.as_mut().unwrap().http_port = next_port as u32;
            write_envelope(
                &mut socket,
                &client_envelope(&session_id, 5, envelope::Payload::ConfigReplace(disabled)),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            let ready = read_ready(&mut socket, 10).await;
            assert!(!ready.http_enabled);
            assert_eq!(ready.http_port, 0);
            assert!(std::net::TcpListener::bind(("127.0.0.1", next_port)).is_ok());

            write_envelope(
                &mut socket,
                &client_envelope(
                    &session_id,
                    6,
                    envelope::Payload::Shutdown(Shutdown {
                        reason: ShutdownReason::Requested as i32,
                        message: String::new(),
                    }),
                ),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            socket.shutdown().await.unwrap();
        });
        let result = run_bridge(address, "fixture", Zeroizing::new(vec![0; 32])).await;
        assert!(result.is_ok());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn framed_http_config_reload_bind_failure_preserves_previous_server() {
        let output_root = tempfile::tempdir().unwrap();
        let _output_root_guard = OutputRootGuard::acquire(output_root.path()).await;
        let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let occupied_port = occupied.local_addr().unwrap().port();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let session_id = handshake(&mut socket).await;
            let bridge_id = vec![0x53; 16];
            write_envelope(
                &mut socket,
                &client_envelope(
                    &session_id,
                    1,
                    envelope::Payload::BridgeIdentityReplace(BridgeIdentityReplace { bridge_id }),
                ),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            loop {
                let frame = timeout(
                    Duration::from_secs(5),
                    read_envelope(&mut socket, FrameLimits::default()),
                )
                .await
                .unwrap()
                .unwrap();
                if matches!(frame.payload, Some(envelope::Payload::Ack(ack)) if ack.acknowledged_sequence == 1)
                {
                    break;
                }
            }
            let mut initial = config();
            let initial_port = TcpListener::bind("127.0.0.1:0")
                .await
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            initial.global.as_mut().unwrap().http_enabled = true;
            initial.global.as_mut().unwrap().http_bind = "127.0.0.1".into();
            initial.global.as_mut().unwrap().http_port = initial_port as u32;
            write_envelope(
                &mut socket,
                &client_envelope(
                    &session_id,
                    2,
                    envelope::Payload::ConfigReplace(initial.clone()),
                ),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            let ready = read_ready(&mut socket, 7).await;
            assert_http_serves(ready.http_port as u16).await;

            let mut failed = initial.clone();
            failed.revision = 8;
            failed.global.as_mut().unwrap().http_port = occupied_port as u32;
            write_envelope(
                &mut socket,
                &client_envelope(&session_id, 3, envelope::Payload::ConfigReplace(failed)),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            let error = read_protocol_error(&mut socket, 8).await;
            assert!(!error.fatal);
            assert_eq!(error.config_revision, 8);
            assert_http_serves(initial_port).await;

            let mut valid = initial;
            valid.revision = 9;
            write_envelope(
                &mut socket,
                &client_envelope(&session_id, 4, envelope::Payload::ConfigReplace(valid)),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            let ready = read_ready(&mut socket, 9).await;
            assert_eq!(ready.http_port, initial_port as u32);
            assert_http_serves(initial_port).await;

            write_envelope(
                &mut socket,
                &client_envelope(
                    &session_id,
                    5,
                    envelope::Payload::Shutdown(Shutdown {
                        reason: ShutdownReason::Requested as i32,
                        message: String::new(),
                    }),
                ),
                FrameLimits::default(),
            )
            .await
            .unwrap();
            socket.shutdown().await.unwrap();
        });
        let result = run_bridge(address, "fixture", Zeroizing::new(vec![0; 32])).await;
        assert!(result.is_ok());
        server.await.unwrap();
        drop(occupied);
    }

    async fn read_ready(
        socket: &mut tokio::net::TcpStream,
        revision: u64,
    ) -> squaremap_protocol::wire::Ready {
        loop {
            let frame = timeout(
                Duration::from_secs(10),
                read_envelope(socket, FrameLimits::default()),
            )
            .await
            .unwrap()
            .unwrap();
            match frame.payload {
                Some(envelope::Payload::Ready(ready)) => {
                    assert_eq!(ready.state_revision, revision);
                    return ready;
                }
                Some(envelope::Payload::ProtocolError(error)) => {
                    panic!("unexpected protocol error: {error:?}")
                }
                _ => {}
            }
        }
    }

    async fn read_protocol_error(
        socket: &mut tokio::net::TcpStream,
        revision: u64,
    ) -> squaremap_protocol::wire::ProtocolError {
        loop {
            let frame = timeout(
                Duration::from_secs(10),
                read_envelope(socket, FrameLimits::default()),
            )
            .await
            .unwrap()
            .unwrap();
            if let Some(envelope::Payload::ProtocolError(error)) = frame.payload {
                assert_eq!(error.config_revision, revision);
                return error;
            }
        }
    }

    async fn assert_http_serves(port: u16) {
        let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        socket
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        socket.read_to_end(&mut response).await.unwrap();
        assert!(response.starts_with(b"HTTP/1.1 "));
    }
}
#[cfg(test)]
mod control_tests {
    use super::{RenderState, handle_control_request, spawn_background_scheduler_observed};
    use async_trait::async_trait;
    use squaremap_protocol::wire::{BackendResultCode, ControlKind, ControlRequest, WorldIdentity};
    use squaremap_server::control::ControlState;
    use squaremap_server::output::OutputRoot;
    use squaremap_server::scheduler::{
        BridgeError, InstallRequest, Scheduler, SchedulerConfig, SnapshotBridge, SnapshotReply,
        SnapshotRequest, StagedInstall, TileInstaller,
    };
    use squaremap_state::{ChunkCoordinate, Repository, World, WorldId};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    #[derive(Default)]
    struct MissingBridge;

    #[async_trait]
    impl SnapshotBridge for MissingBridge {
        async fn request(&self, _request: SnapshotRequest) -> Result<SnapshotReply, BridgeError> {
            unreachable!("control fixture never runs a snapshot request")
        }
        async fn enumerate_world(
            &self,
            _world: &WorldId,
        ) -> Result<Vec<ChunkCoordinate>, BridgeError> {
            Ok(vec![ChunkCoordinate { x: 3, z: 4 }])
        }
    }
    #[derive(Default)]
    struct BlockingBridge {
        started: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }

    #[async_trait]
    impl SnapshotBridge for BlockingBridge {
        async fn request(&self, _request: SnapshotRequest) -> Result<SnapshotReply, BridgeError> {
            unreachable!("blocking fixture never runs a snapshot request")
        }
        async fn enumerate_world(
            &self,
            _world: &WorldId,
        ) -> Result<Vec<ChunkCoordinate>, BridgeError> {
            self.started.notify_one();
            self.release.notified().await;
            Ok(vec![ChunkCoordinate { x: 3, z: 4 }])
        }
    }

    struct NoopInstaller;
    struct NoopStagedInstall;

    #[async_trait]
    impl StagedInstall for NoopStagedInstall {
        async fn publish(self: Box<Self>) -> Result<(), String> {
            Ok(())
        }
    }

    #[async_trait]
    impl TileInstaller for NoopInstaller {
        async fn stage(&self, _request: InstallRequest) -> Result<Box<dyn StagedInstall>, String> {
            Ok(Box::new(NoopStagedInstall))
        }
    }

    fn identity(value: &str) -> WorldIdentity {
        WorldIdentity {
            namespace: "minecraft".to_string(),
            value: value.to_string(),
            epoch: 7,
        }
    }

    fn world(value: &str) -> WorldId {
        WorldId::new("minecraft", value, 7)
    }

    fn request_for(
        kind: ControlKind,
        identity: WorldIdentity,
        coordinates: Vec<ChunkCoordinate>,
    ) -> ControlRequest {
        ControlRequest {
            kind: kind as i32,
            world: Some(identity),
            center_x: 0,
            center_z: 0,
            radius: 2,
            coordinates: coordinates
                .into_iter()
                .map(|coordinate| squaremap_protocol::wire::ChunkCoordinate {
                    x: coordinate.x,
                    z: coordinate.z,
                })
                .collect(),
        }
    }

    fn request(kind: ControlKind, coordinates: Vec<ChunkCoordinate>) -> ControlRequest {
        request_for(kind, identity("overworld"), coordinates)
    }

    async fn fixture() -> (
        tempfile::TempDir,
        Arc<Repository>,
        Arc<Scheduler>,
        OutputRoot,
        ControlState,
        Arc<Mutex<HashMap<WorldId, RenderState>>>,
    ) {
        let directory = tempdir().unwrap();
        let repository = Arc::new(
            Repository::open(directory.path().join("state.sqlite"))
                .await
                .unwrap(),
        );
        repository
            .apply_world(World::new("minecraft", "overworld", 7, Vec::new()))
            .await
            .unwrap();
        let scheduler = Arc::new(
            Scheduler::new(
                repository.clone(),
                Arc::new(MissingBridge),
                Arc::new(NoopInstaller),
                SchedulerConfig::default(),
            )
            .unwrap(),
        );
        let root = OutputRoot::new(directory.path().join("output")).unwrap();
        let mut state = ControlState::default();
        state.replace_worlds(vec![identity("overworld")]);
        (
            directory,
            repository,
            scheduler,
            root,
            state,
            Arc::new(Mutex::new(HashMap::new())),
        )
    }
    #[tokio::test]
    async fn background_scheduler_replacement_and_shutdown_drop_old_generation() {
        let (_directory, _repository, scheduler, _root, _state, _active_jobs) = fixture().await;
        let live = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let first =
            spawn_background_scheduler_observed(scheduler.clone(), true, live.clone()).unwrap();
        tokio::task::yield_now().await;
        assert_eq!(live.load(std::sync::atomic::Ordering::SeqCst), 1);
        first.abort();
        let _ = first.await;
        assert_eq!(live.load(std::sync::atomic::Ordering::SeqCst), 0);
        let second = spawn_background_scheduler_observed(scheduler, true, live.clone()).unwrap();
        tokio::task::yield_now().await;
        assert_eq!(live.load(std::sync::atomic::Ordering::SeqCst), 1);
        second.abort();
        let _ = second.await;
        assert_eq!(live.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cancel_render_during_blocked_enumeration_leaves_no_job_or_state() {
        let directory = tempdir().unwrap();
        let repository = Arc::new(
            Repository::open(directory.path().join("state.sqlite"))
                .await
                .unwrap(),
        );
        repository
            .apply_world(World::new("minecraft", "overworld", 7, Vec::new()))
            .await
            .unwrap();
        let bridge = Arc::new(BlockingBridge::default());
        let scheduler = Arc::new(
            Scheduler::new(
                repository.clone(),
                bridge.clone(),
                Arc::new(NoopInstaller),
                SchedulerConfig::default(),
            )
            .unwrap(),
        );
        let root = OutputRoot::new(directory.path().join("output")).unwrap();
        let mut state = ControlState::default();
        state.replace_worlds(vec![identity("overworld")]);
        let active_jobs = Arc::new(Mutex::new(HashMap::new()));
        let render_active = active_jobs.clone();
        let render_scheduler = scheduler.clone();
        let render_root = root.clone();
        let mut render_state = ControlState::default();
        render_state.replace_worlds(vec![identity("overworld")]);
        let render = tokio::spawn(async move {
            handle_control_request(
                &mut render_state,
                &request(ControlKind::FullRender, Vec::new()),
                Some(render_scheduler),
                render_active,
                11,
                &render_root,
            )
            .await
        });
        bridge.started.notified().await;
        assert!(matches!(
            active_jobs.lock().unwrap().get(&world("overworld")),
            Some(RenderState::Discovering(_))
        ));
        let cancel = handle_control_request(
            &mut state,
            &request(ControlKind::CancelRender, Vec::new()),
            Some(scheduler.clone()),
            active_jobs.clone(),
            11,
            &root,
        )
        .await;
        assert_eq!(cancel.code, BackendResultCode::RenderCancelled as i32);
        bridge.release.notify_one();
        let result = render.await.unwrap();
        assert_eq!(result.code, BackendResultCode::RenderCancelled as i32);
        assert!(active_jobs.lock().unwrap().is_empty());
        assert!(repository.recover().await.unwrap().jobs.is_empty());
    }

    #[tokio::test]
    async fn full_render_uses_authoritative_enumeration_and_starts_durable_job() {
        let (_directory, repository, scheduler, root, mut state, active_jobs) = fixture().await;
        let result = handle_control_request(
            &mut state,
            &request(
                ControlKind::FullRender,
                vec![ChunkCoordinate { x: 99, z: 99 }],
            ),
            Some(scheduler.clone()),
            active_jobs.clone(),
            11,
            &root,
        )
        .await;
        assert_eq!(result.code, BackendResultCode::FullRenderStarted as i32);
        let jobs = repository.recover().await.unwrap().jobs;
        assert_eq!(jobs.len(), 1);
        assert!(String::from_utf8_lossy(&jobs[0].payload).contains("\"x\":3"));
        assert!(!String::from_utf8_lossy(&jobs[0].payload).contains("\"x\":99"));
        assert!(!jobs[0].id.is_empty());
        let cancel = handle_control_request(
            &mut state,
            &request(ControlKind::CancelRender, Vec::new()),
            Some(scheduler),
            active_jobs.clone(),
            11,
            &root,
        )
        .await;
        assert_eq!(cancel.code, BackendResultCode::RenderCancelled as i32);
        assert!(active_jobs.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn shutdown_render_tasks_aborts_nonterminating_runner_and_clears_state() {
        let active_jobs = Arc::new(Mutex::new(HashMap::new()));
        let marker = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task_marker = marker.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::task::yield_now().await;
                task_marker.store(true, std::sync::atomic::Ordering::Release);
            }
        });
        active_jobs.lock().unwrap().insert(
            world("overworld"),
            RenderState::Running {
                job_id: vec![1],
                task: Some(task),
            },
        );
        super::shutdown_render_tasks(&active_jobs, None).await;
        assert!(active_jobs.lock().unwrap().is_empty());
        assert!(!marker.load(std::sync::atomic::Ordering::Acquire));
    }

    #[tokio::test]
    async fn pause_renders_only_toggles_requested_world() {
        let (_directory, repository, scheduler, root, mut state, active_jobs) = fixture().await;
        repository
            .apply_world(World::new("minecraft", "nether", 7, Vec::new()))
            .await
            .unwrap();
        state.replace_worlds(vec![identity("overworld"), identity("nether")]);

        let first = handle_control_request(
            &mut state,
            &request_for(ControlKind::PauseRenders, identity("overworld"), Vec::new()),
            Some(scheduler.clone()),
            active_jobs.clone(),
            11,
            &root,
        )
        .await;
        assert_eq!(first.code, BackendResultCode::RendersPaused as i32);
        assert!(scheduler.is_paused(&world("overworld")));
        assert!(!scheduler.is_paused(&world("nether")));

        let second = handle_control_request(
            &mut state,
            &request_for(ControlKind::PauseRenders, identity("nether"), Vec::new()),
            Some(scheduler.clone()),
            active_jobs,
            11,
            &root,
        )
        .await;
        assert_eq!(second.code, BackendResultCode::RendersPaused as i32);
        assert!(scheduler.is_paused(&world("overworld")));
        assert!(scheduler.is_paused(&world("nether")));
    }

    #[tokio::test]
    async fn pause_renders_toggles_scheduler_state() {
        let (_directory, _repository, scheduler, root, mut state, active_jobs) = fixture().await;
        let first = handle_control_request(
            &mut state,
            &request(ControlKind::PauseRenders, Vec::new()),
            Some(scheduler.clone()),
            active_jobs.clone(),
            11,
            &root,
        )
        .await;
        assert_eq!(first.code, BackendResultCode::RendersPaused as i32);
        assert!(scheduler.is_paused(&world("overworld")));

        let second = handle_control_request(
            &mut state,
            &request(ControlKind::PauseRenders, Vec::new()),
            Some(scheduler.clone()),
            active_jobs,
            11,
            &root,
        )
        .await;
        assert_eq!(second.code, BackendResultCode::RendersResumed as i32);
        assert!(!scheduler.is_paused(&world("overworld")));
    }

    #[tokio::test]
    async fn reset_clears_durable_work_and_tiles_without_removing_world() {
        let output_root = tempdir().unwrap();
        let root = OutputRoot::new(output_root.path()).unwrap();
        let repository = Arc::new(
            Repository::open(output_root.path().join(".sqlite"))
                .await
                .unwrap(),
        );
        let world = WorldId::new("minecraft", "overworld", 7);
        repository
            .apply_world(World::new("minecraft", "overworld", 7, Vec::new()))
            .await
            .unwrap();
        repository
            .mark_dirty(
                &world,
                ChunkCoordinate { x: 1, z: 2 },
                3,
                &[1; 16],
                &[2; 16],
                1,
            )
            .await
            .unwrap();
        let scheduler = Arc::new(
            Scheduler::new(
                repository.clone(),
                Arc::new(MissingBridge),
                Arc::new(NoopInstaller),
                SchedulerConfig::default(),
            )
            .unwrap(),
        );
        let mut state = ControlState::default();
        state.replace_worlds(vec![WorldIdentity {
            namespace: "minecraft".into(),
            value: "overworld".into(),
            epoch: 7,
        }]);
        let active_jobs = Arc::new(Mutex::new(HashMap::new()));
        scheduler
            .start_job(
                world.clone(),
                squaremap_state::JobKind::Full,
                vec![ChunkCoordinate { x: 3, z: 4 }],
            )
            .await
            .unwrap();
        root.atomic_write("tiles/minecraft_overworld/0/0.png", b"tile")
            .unwrap();

        let result = handle_control_request(
            &mut state,
            &request(ControlKind::ResetMap, Vec::new()),
            Some(scheduler),
            active_jobs,
            12,
            &root,
        )
        .await;
        assert_eq!(result.code, BackendResultCode::MapReset as i32);
        let recovered = repository.recover().await.unwrap();
        assert!(recovered.dirty.is_empty());
        assert!(recovered.jobs.is_empty());
        assert!(
            root.existing_files("tiles/minecraft_overworld")
                .unwrap()
                .is_empty()
        );
        assert!(repository.world_is_current(&world).await.unwrap());
    }

    #[tokio::test]
    async fn reset_during_blocked_discovery_does_not_reset_or_delete_tiles() {
        let directory = tempdir().unwrap();
        let repository = Arc::new(
            Repository::open(directory.path().join("state.sqlite"))
                .await
                .unwrap(),
        );
        let world = world("overworld");
        repository
            .apply_world(World::new("minecraft", "overworld", 7, Vec::new()))
            .await
            .unwrap();
        repository
            .mark_dirty(
                &world,
                ChunkCoordinate { x: 1, z: 2 },
                3,
                &[1; 16],
                &[2; 16],
                1,
            )
            .await
            .unwrap();
        let bridge = Arc::new(BlockingBridge::default());
        let scheduler = Arc::new(
            Scheduler::new(
                repository.clone(),
                bridge.clone(),
                Arc::new(NoopInstaller),
                SchedulerConfig::default(),
            )
            .unwrap(),
        );
        let root = OutputRoot::new(directory.path().join("output")).unwrap();
        root.atomic_write("tiles/minecraft_overworld/0/0.png", b"tile")
            .unwrap();
        let active_jobs = Arc::new(Mutex::new(HashMap::new()));
        let mut render_state = ControlState::default();
        render_state.replace_worlds(vec![identity("overworld")]);
        let render_active = active_jobs.clone();
        let render_scheduler = scheduler.clone();
        let render_root = root.clone();
        let render = tokio::spawn(async move {
            handle_control_request(
                &mut render_state,
                &request(ControlKind::FullRender, Vec::new()),
                Some(render_scheduler),
                render_active,
                11,
                &render_root,
            )
            .await
        });
        bridge.started.notified().await;
        assert!(matches!(
            active_jobs.lock().unwrap().get(&world),
            Some(RenderState::Discovering(_))
        ));

        let mut state = ControlState::default();
        state.replace_worlds(vec![identity("overworld")]);
        let reset = handle_control_request(
            &mut state,
            &request(ControlKind::ResetMap, Vec::new()),
            Some(scheduler.clone()),
            active_jobs.clone(),
            11,
            &root,
        )
        .await;
        assert_eq!(reset.code, BackendResultCode::RenderInProgress as i32);
        assert!(!repository.recover().await.unwrap().dirty.is_empty());
        assert_eq!(repository.recover().await.unwrap().jobs.len(), 0);
        assert_eq!(
            root.existing_files("tiles/minecraft_overworld")
                .unwrap()
                .len(),
            1
        );
        assert!(matches!(
            active_jobs.lock().unwrap().get(&world),
            Some(RenderState::Discovering(_))
        ));

        let cancel = handle_control_request(
            &mut state,
            &request(ControlKind::CancelRender, Vec::new()),
            Some(scheduler),
            active_jobs.clone(),
            11,
            &root,
        )
        .await;
        assert_eq!(cancel.code, BackendResultCode::RenderCancelled as i32);
        bridge.release.notify_one();
        assert_eq!(
            render.await.unwrap().code,
            BackendResultCode::RenderCancelled as i32
        );
    }
}
