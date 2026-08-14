use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use prost::Message;
use rand::RngCore;
use squaremap_protocol::wire::{
    backend_substitution, envelope, BackendResultCode, BackendSubstitution, BridgePolicyReplace,
    ConfigReplace, ControlKind, ControlRequest, ControlResult, Envelope, Hello, ProtocolError,
    ProtocolErrorCode, Shutdown,
};
use squaremap_protocol::{read_envelope, write_envelope, FrameClass, FrameError, FrameLimits};
use squaremap_render::{Limits, PngOptions, RenderSettings, TileStore};
use squaremap_server::config::ConfigStore;
use squaremap_server::control::ControlState;
use squaremap_server::scheduler::live::{LiveSnapshotBridge, OutboundSnapshotRequest, SnapshotDispatcher};
use squaremap_server::scheduler::{RenderTileInstaller, Scheduler, SchedulerConfig, WorldRenderConfig};
use squaremap_server::snapshot_client::SnapshotClient;
use crate::session::{Session, SessionError};
use squaremap_state::{Repository, World, WorldId};
use squaremap_server::http::{HttpConfig, HttpServer};
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
            Self::InvalidConnect(message) => write!(formatter, "invalid loopback connect address: {message}"),
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
        return Err(BootstrapError::InvalidConnect("address is not loopback".to_string()));
    }
    Ok(address)
}

pub fn read_token<R: BufRead>(reader: R) -> Result<Zeroizing<Vec<u8>>, BootstrapError> {
    let mut bounded = reader.take((MAX_TOKEN_LINE_BYTES + 1) as u64);
    let mut line = Zeroizing::new(Vec::with_capacity(MAX_TOKEN_LINE_BYTES + 1));
    bounded.read_to_end(&mut line)?;
    if !line.ends_with(b"\n") {
        return Err(BootstrapError::Token("expected one newline-terminated line of at most 88 characters".to_string()));
    }
    line.pop();
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    if line.len() > MAX_TOKEN_LINE_BYTES || !line.is_ascii() {
        return Err(BootstrapError::Token("token line is too long or non-ASCII".to_string()));
    }
    let decoded = Zeroizing::new(
        STANDARD
            .decode(&line)
            .map_err(|error| BootstrapError::Token(error.to_string()))?,
    );
    if decoded.len() != BOOTSTRAP_TOKEN_BYTES {
        return Err(BootstrapError::Token("token must decode to exactly 32 bytes".to_string()));
    }
    Ok(decoded)
}

async fn write_bootstrap_hello<W: AsyncWrite + Unpin>(
    writer: &mut W,
    plugin_version: &str,
    session_id: &[u8; SESSION_ID_BYTES],
    token: &mut Zeroizing<Vec<u8>>,
) -> Result<(), BootstrapError> {
    let result = write_bootstrap_hello_inner(writer, plugin_version, session_id, token.as_slice()).await;
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
    encoded.map_err(FrameError::ProtobufEncode).map_err(BootstrapError::Frame)?;
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

async fn queue_outbound(
    sender: &mpsc::Sender<Envelope>,
    envelope: Envelope,
) -> Result<(), BootstrapError> {
    sender
        .send(envelope)
        .await
        .map_err(|_| BootstrapError::Io(io::Error::new(io::ErrorKind::BrokenPipe, "bridge writer stopped")))
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

fn configure_installer(
    installer: &RenderTileInstaller,
    config: &ConfigReplace,
    policy: &BridgePolicyReplace,
) -> Result<(SchedulerConfig, bool), String> {
    let global = config.global.as_ref().ok_or("global settings are missing")?;
    let default_world = config.world.as_ref().ok_or("default world settings are missing")?;
    let render = config.render.as_ref().ok_or("render settings are missing")?;
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
        let identity = configured.identity.as_ref().ok_or("world identity is missing")?;
        let settings = configured.settings.as_ref().ok_or("world settings are missing")?;
        let max_zoom = u8::try_from(settings.zoom_max)
            .map_err(|_| format!("maximum zoom is outside platform bounds for {}:{}", identity.namespace, identity.value))?;
        if max_zoom > 9 {
            return Err(format!("maximum zoom exceeds the tile pyramid limit for {}:{}", identity.namespace, identity.value));
        }
        installer.configure_world(
            WorldId::new(identity.namespace.clone(), identity.value.clone(), identity.epoch),
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
                png_options: PngOptions { compression: global.compress_images },
                tile_prefix: PathBuf::from("tiles").join(format!("{}_{}", identity.namespace, identity.value)),
            },
        )?;
    }
    Ok((
        SchedulerConfig {
            max_active_snapshots,
            dirty_page_size,
            background_interval: Duration::from_secs(u64::from(default_world.background_render_interval_seconds)),
            ..SchedulerConfig::default()
        },
        render.background_enabled && default_world.background_render_enabled,
    ))
}

fn spawn_background_scheduler(
    scheduler: Arc<Scheduler>,
    enabled: bool,
) -> Option<JoinHandle<()>> {
    if !enabled {
        return None;
    }
    let interval = scheduler.config().background_interval;
    Some(tokio::spawn(async move {
        if let Err(error) = scheduler.resume_jobs().await {
            tracing::warn!(error = %error, "could not resume durable render jobs");
        }
        loop {
            if let Err(error) = scheduler.run_dirty_page().await {
                tracing::warn!(error = %error, "background render page failed");
            }
            tokio::time::sleep(interval).await;
        }
    }))
}
fn control_response(code: BackendResultCode, identity: Option<&squaremap_protocol::wire::WorldIdentity>) -> ControlResult {
    let substitutions = identity
        .into_iter()
        .map(|world| BackendSubstitution {
            key: "world".to_string(),
            value: Some(backend_substitution::Value::WorldIdentity(world.clone())),
        })
        .collect();
    ControlResult { code: code as i32, substitutions, rendered_chunks: 0 }
}

async fn handle_control_request(
    state: &mut ControlState,
    request: &ControlRequest,
    scheduler: Option<Arc<Scheduler>>,
    active_jobs: Arc<Mutex<HashMap<WorldId, Vec<u8>>>>,
    revision: u64,
    root: &squaremap_server::output::OutputRoot,
) -> ControlResult {
    let baseline = state.handle(request);
    if baseline.code != BackendResultCode::BackendUnavailable as i32 {
        return baseline;
    }
    let Ok(kind) = ControlKind::try_from(request.kind) else {
        return control_response(BackendResultCode::InvalidRequest, request.world.as_ref());
    };
    let Some(identity) = request.world.as_ref() else {
        return control_response(BackendResultCode::InvalidRequest, None);
    };
    let Some(scheduler) = scheduler else {
        return baseline;
    };
    let world = WorldId::new(identity.namespace.clone(), identity.value.clone(), identity.epoch);
    match kind {
        ControlKind::FullRender | ControlKind::RadiusRender => {
            let Ok(mut active) = active_jobs.lock() else {
                return control_response(BackendResultCode::BackendUnavailable, Some(identity));
            };
            if active.contains_key(&world) {
                return control_response(BackendResultCode::RenderInProgress, Some(identity));
            }
            let coordinates = if !request.coordinates.is_empty() {
                request.coordinates.iter().map(|coordinate| squaremap_state::ChunkCoordinate {
                    x: coordinate.x,
                    z: coordinate.z,
                }).collect::<Vec<_>>()
            } else if kind == ControlKind::FullRender {
                Vec::new()
            } else {
                if request.radius == 0 || request.radius > 256 {
                    return control_response(BackendResultCode::InvalidRequest, Some(identity));
                }
                let radius = i64::from(request.radius);
                let center_x = i64::from(request.center_x);
                let center_z = i64::from(request.center_z);
                let mut coordinates = Vec::with_capacity(((radius * 2 + 1) * (radius * 2 + 1)) as usize);
                for x in (center_x - radius)..=(center_x + radius) {
                    for z in (center_z - radius)..=(center_z + radius) {
                        let (Ok(x), Ok(z)) = (i32::try_from(x), i32::try_from(z)) else {
                            return control_response(BackendResultCode::InvalidRequest, Some(identity));
                        };
                        coordinates.push(squaremap_state::ChunkCoordinate { x, z });
                    }
                }
                coordinates
            };
            if coordinates.is_empty() {
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
                    return control_response(BackendResultCode::Failed, Some(identity));
                }
            };
            let job_id = job.id.clone();
            active.insert(world.clone(), job_id.clone());
            drop(active);
            let jobs = active_jobs.clone();
            tokio::spawn(async move {
                if let Err(error) = scheduler.run_job(&job_id).await {
                    tracing::warn!(error = %error, "render failed");
                }
                if let Ok(mut active) = jobs.lock() {
                    active.remove(&world);
                }
            });
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
            if scheduler.is_paused() {
                scheduler.resume();
                control_response(BackendResultCode::RendersResumed, Some(identity))
            } else {
                scheduler.pause();
                control_response(BackendResultCode::RendersPaused, Some(identity))
            }
        }
        ControlKind::CancelRender => {
            let job = active_jobs.lock().ok().and_then(|active| active.get(&world).cloned());
            let Some(job) = job else {
                return control_response(BackendResultCode::RenderNotInProgress, Some(identity));
            };
            match scheduler.cancel_job(&job).await {
                Ok(()) => {
                    if let Ok(mut active) = active_jobs.lock() {
                        active.remove(&world);
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
            if active_jobs.lock().ok().is_some_and(|active| active.contains_key(&world)) {
                return control_response(BackendResultCode::RenderInProgress, Some(identity));
            }
            let tile_prefix = PathBuf::from("tiles").join(format!("{}_{}", identity.namespace, identity.value));
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

    let configured_root = std::env::var("SQUAREMAP_OUTPUT_ROOT")
        .map_err(|_| BootstrapError::Rejected("SQUAREMAP_OUTPUT_ROOT is required for bridge mode".to_string()))?;
    let mut http_server: Option<HttpServer> = None;
    let root = squaremap_server::output::OutputRoot::new(configured_root)?;
    let repository = Arc::new(
        Repository::open(root.path().join(".squaremap-state.sqlite"))
            .await
            .map_err(|error| BootstrapError::Rejected(format!("could not open durable render state: {error}")))?,
    );
    let mut session = Session::from_session_id(&session_id)
        .map_err(|error| BootstrapError::Rejected(error.to_string()))?;
    let mut config_store = ConfigStore::default();
    let mut control_state = ControlState::default();
    let snapshot_client = SnapshotClient::new(session_id, Limits::default())
        .map_err(|error| BootstrapError::Rejected(error.to_string()))?;
    let (snapshot_bridge, snapshot_requests) = LiveSnapshotBridge::channel(256);
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

    let mut scheduler: Option<Arc<Scheduler>> = None;
    let active_jobs = Arc::new(Mutex::new(HashMap::<WorldId, Vec<u8>>::new()));
    let mut active_config_revision = 0_u64;
    let mut background_task: Option<JoinHandle<()>> = None;
    let mut loop_error = None;
    loop {
        tokio::select! {
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
                                    "bridge envelope failed protocol/session validation".to_string(),
                                ));
                            }
                            break;
                        }
                        if matches!(envelope.payload, Some(envelope::Payload::Shutdown(Shutdown { .. }))) {
                            break;
                        }
                        let sequence = envelope.sequence;
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
                            session.process(envelope.clone(), move |_| {
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
                                        offending_sequence: sequence,
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
                            } else {
                                loop_error = Some(BootstrapError::Rejected(protocol_error.message.clone()));
                            }
                            break;
                        }
                        let accepted = outcome
                            .ack()
                            .is_some_and(|ack| ack.status == squaremap_protocol::wire::AckStatus::Accepted as i32);
                        if accepted && is_snapshot {
                            if let Err(error) = dispatcher.accept(&envelope).await {
                                let response = make_outbound(
                                    &session_id,
                                    envelope.correlation_id,
                                    envelope::Payload::ProtocolError(ProtocolError {
                                        code: ProtocolErrorCode::Internal as i32,
                                        message: error.to_string(),
                                        fatal: true,
                                        offending_sequence: sequence,
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
                                            let scheduler = Scheduler::new(
                                                repository.clone(),
                                                snapshot_bridge.clone(),
                                                installer.clone(),
                                                scheduler_config,
                                            )
                                            .map_err(|error| error.to_string())?;
                                            let next_http = match config.global.as_ref() {
                                                Some(global) if global.http_enabled => {
                                                    let bind = format!("{}:{}", global.http_bind, global.http_port)
                                                        .parse()
                                                        .map_err(|error| format!("invalid HTTP bind address: {error}"))?;
                                                    Some(
                                                        HttpServer::bind(
                                                            HttpConfig { bind, enabled: true, dev_frontend: None },
                                                            root.clone(),
                                                        )
                                                        .await
                                                        .map_err(|error| format!("could not start HTTP server: {error}"))?,
                                                    )
                                                }
                                                _ => None,
                                            };
                                            config_store
                                                .stage_and_swap(config.clone())
                                                .map_err(|error| error.to_string())?;
                                            Ok::<_, String>((policy, Arc::new(scheduler), background_enabled, next_http))
                                        }
                                        .await;
                                        match configured {
                                            Ok((policy, next_scheduler, background_enabled, mut next_http)) => {
                                                if let Some(task) = background_task.take() {
                                                    task.abort();
                                                }
                                                let previous = http_server.take();
                                                if let Some(mut previous) = previous {
                                                    if next_http.is_none() {
                                                        if let Err(error) = previous.shutdown().await {
                                                            config_error = Some(format!("could not stop previous HTTP server: {error}"));
                                                        }
                                                    } else if let Err(error) = previous.shutdown().await {
                                                        if let Some(mut next) = next_http.take() {
                                                            let _ = next.shutdown().await;
                                                        }
                                                        config_error = Some(format!("could not stop previous HTTP server: {error}"));
                                                    }
                                                }
                                                if config_error.is_some() {
                                                    if let Some(mut next) = next_http {
                                                        let _ = next.shutdown().await;
                                                    }
                                                    break;
                                                }
                                                background_task =
                                                    spawn_background_scheduler(next_scheduler.clone(), background_enabled);
                                                scheduler = Some(next_scheduler);
                                                active_config_revision = config.revision;
                                                control_state.replace_worlds(
                                                    config.worlds
                                                        .iter()
                                                        .filter_map(|world| world.identity.clone())
                                                        .collect(),
                                                );
                                                policy_result = Some(policy);
                                                http_server = next_http;
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
                                control_result = Some(
                                    handle_control_request(
                                        &mut control_state,
                                        request,
                                        scheduler.clone(),
                                        active_jobs.clone(),
                                        active_config_revision,
                                        &root,
                                    )
                                    .await,
                                );
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
                                    offending_sequence: sequence,
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

    dispatcher.abort("bridge connection closed").await;
    if let Some(task) = background_task {
        task.abort();
    }
    snapshot_task.abort();
    if let Some(mut server) = http_server {
        if let Err(error) = server.shutdown().await {
            tracing::warn!(%error, "HTTP server shutdown failed");
        }
    }
    drop(outbound);
    let _ = writer_task.await;
    if let Some(error) = loop_error {
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
        let Some(identity) = configured.identity.as_ref() else { continue };
        current.insert((identity.namespace.clone(), identity.value.clone(), identity.epoch));
        repository.apply_world(World::new(
            identity.namespace.clone(),
            identity.value.clone(),
            identity.epoch,
            configured.encode_to_vec(),
        )).await?;
    }
    for existing in repository.recover().await?.worlds {
        if !current.contains(&(existing.id.namespace.clone(), existing.id.value.clone(), existing.id.epoch)) {
            repository.remove_world(&WorldId::new(
                existing.id.namespace,
                existing.id.value,
                existing.id.epoch,
            )).await?;
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
        assert_eq!(parse_connect("127.0.0.1:1234").unwrap(), "127.0.0.1:1234".parse::<SocketAddr>().unwrap());
        assert!(parse_connect("192.0.2.1:1234").is_err());
        assert!(parse_connect("0.0.0.0:1234").is_err());
    }

    #[test]
    fn accepts_ipv6_loopback() {
        assert_eq!(parse_connect("[::1]:1234").unwrap(), "[::1]:1234".parse::<SocketAddr>().unwrap());
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
    use super::{run_bridge, write_bootstrap_hello, BootstrapError};
    use squaremap_protocol::wire::{envelope, Envelope, HelloAck, Shutdown, ShutdownReason};
    use squaremap_protocol::{read_envelope, write_envelope, FrameLimits};
    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::AsyncWrite;
    use tokio::net::TcpListener;
    use zeroize::Zeroizing;

    #[tokio::test]
    async fn accepted_handshake_stays_alive_until_shutdown() {
        let output_root = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("SQUAREMAP_OUTPUT_ROOT", output_root.path()); }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let hello = read_envelope(&mut socket, FrameLimits::default()).await.unwrap();
            let ack = Envelope {
                protocol_major: 1,
                protocol_minor: 0,
                session_id: hello.session_id.clone(),
                sequence: 2,
                correlation_id: 0,
                payload: Some(envelope::Payload::HelloAck(HelloAck {
                    protocol_major: 1,
                    protocol_minor: 0,
                    backend_version: "fixture".to_string(),
                    accepted: true,
                    rejection_reason: String::new(),
                })),
            };
            write_envelope(&mut socket, &ack, FrameLimits::default()).await.unwrap();
            let shutdown = Envelope {
                protocol_major: 1,
                protocol_minor: 0,
                session_id: hello.session_id,
                sequence: 3,
                correlation_id: 0,
                payload: Some(envelope::Payload::Shutdown(Shutdown {
                    reason: ShutdownReason::Requested as i32,
                    message: String::new(),
                })),
            };
            write_envelope(&mut socket, &shutdown, FrameLimits::default()).await.unwrap();
        });
        let result = run_bridge(address, "fixture", Zeroizing::new(vec![0; 32])).await;
        server.await.unwrap();
        unsafe { std::env::remove_var("SQUAREMAP_OUTPUT_ROOT"); }
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn rejected_ack_fails_bootstrap() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let hello = read_envelope(&mut socket, FrameLimits::default()).await.unwrap();
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
            write_envelope(&mut socket, &ack, FrameLimits::default()).await.unwrap();
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
            Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, "fixture failure")))
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn secret_hello_writer_zeroizes_token_on_write_error() {
        let mut token = Zeroizing::new(vec![0xa5; 32]);
        let mut writer = FailingWriter;
        let session_id = [0_u8; 16];
        let result = write_bootstrap_hello(&mut writer, "fixture", &session_id, &mut token).await;
        assert!(result.is_err());
        assert!(token.iter().all(|byte| *byte == 0));
    }
}
#[cfg(test)]
mod control_tests {
    use super::handle_control_request;
    use async_trait::async_trait;
    use squaremap_protocol::wire::{BackendResultCode, ControlKind, ControlRequest, WorldIdentity};
    use squaremap_server::control::ControlState;
    use squaremap_server::output::OutputRoot;
    use squaremap_server::scheduler::{
        BridgeError, InstallRequest, Scheduler, SchedulerConfig, SnapshotBridge, SnapshotReply, SnapshotRequest,
        TileInstaller,
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
    }

    struct NoopInstaller;

    #[async_trait]
    impl TileInstaller for NoopInstaller {
        async fn install(&self, _request: InstallRequest) -> Result<(), String> {
            Ok(())
        }
    }

    fn identity() -> WorldIdentity {
        WorldIdentity {
            namespace: "minecraft".to_string(),
            value: "overworld".to_string(),
            epoch: 7,
        }
    }

    fn request(kind: ControlKind, coordinates: Vec<ChunkCoordinate>) -> ControlRequest {
        ControlRequest {
            kind: kind as i32,
            world: Some(identity()),
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

    async fn fixture() -> (
        tempfile::TempDir,
        Arc<Repository>,
        Arc<Scheduler>,
        OutputRoot,
        ControlState,
        Arc<Mutex<HashMap<WorldId, Vec<u8>>>>,
    ) {
        let directory = tempdir().unwrap();
        let repository = Arc::new(Repository::open(directory.path().join("state.sqlite")).await.unwrap());
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
        state.replace_worlds(vec![identity()]);
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
    async fn full_render_requires_coordinates_and_starts_durable_job() {
        let (_directory, repository, scheduler, root, mut state, active_jobs) = fixture().await;
        let result = handle_control_request(
            &mut state,
            &request(ControlKind::FullRender, vec![ChunkCoordinate { x: 3, z: 4 }]),
            Some(scheduler.clone()),
            active_jobs.clone(),
            11,
            &root,
        )
        .await;
        assert_eq!(result.code, BackendResultCode::FullRenderStarted as i32);
        assert_eq!(repository.recover().await.unwrap().jobs.len(), 1);
        drop(scheduler);
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
        assert!(scheduler.is_paused());

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
        assert!(!scheduler.is_paused());
    }

    #[tokio::test]
    async fn reset_clears_durable_work_and_tiles_without_removing_world() {
        let (_directory, repository, scheduler, root, mut state, active_jobs) = fixture().await;
        let world = WorldId::new("minecraft", "overworld", 7);
        repository
            .mark_dirty(&world, ChunkCoordinate { x: 1, z: 2 }, 3, &[1; 16], 1)
            .await
            .unwrap();
        scheduler
            .start_job(
                world.clone(),
                squaremap_state::JobKind::Full,
                vec![ChunkCoordinate { x: 3, z: 4 }],
            )
            .await
            .unwrap();
        root.atomic_write("tiles/minecraft_overworld/0/0.png", b"tile").unwrap();

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
        assert!(root.existing_files("tiles/minecraft_overworld").unwrap().is_empty());
        assert!(repository.world_is_current(&world).await.unwrap());
    }
}
