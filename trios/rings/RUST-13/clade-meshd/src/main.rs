//! HTTP control daemon for trios-mesh.
//!
//! Exposes node status, ETX routing, and crypto diagnostics to the Trios UI
//! via a small warp REST API. Runs entirely in host-sim mode (no radio/TUN).
//!
//! phi^2 + phi^-2 = 3

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use warp::{reply::json, Filter};

use trios_mesh::crypto::StaticKey;
use trios_mesh::daemon::Node;
use trios_mesh::discovery::Hello;
use trios_mesh::NodeId;

const DEFAULT_PORT: u16 = 9505;
const ETX_WINDOW: usize = 16;

/// Global node state protected by a read-write lock.
struct MeshState {
    node: Node,
    /// Derived public keys for any peers we have seeded.
    peer_keys: HashMap<NodeId, Vec<u8>>,
}

impl MeshState {
    fn new(id: NodeId) -> Self {
        Self {
            node: Node::new(id, ETX_WINDOW),
            peer_keys: HashMap::new(),
        }
    }
}

#[derive(Serialize, Debug, Clone)]
struct HealthResponse {
    status: String,
    node_id: NodeId,
}

#[derive(Serialize, Debug, Clone)]
struct StatusResponse {
    node_id: NodeId,
    neighbors: Vec<NeighborStatus>,
    routes: Vec<RouteStatus>,
    sessions: Vec<SessionStatus>,
    metrics: MetricSnapshot,
}

#[derive(Serialize, Debug, Clone)]
struct NeighborStatus {
    id: NodeId,
    etx: f32,
    etx_label: String,
}

#[derive(Serialize, Debug, Clone)]
struct RouteStatus {
    destination: NodeId,
    next_hop: Option<NodeId>,
    path_etx: Option<f32>,
}

#[derive(Serialize, Debug, Clone)]
struct SessionStatus {
    peer: NodeId,
    has_session: bool,
}

#[derive(Serialize, Debug, Clone)]
struct MetricSnapshot {
    link_loss_to_reroute_ms: Option<f32>,
    node_off_to_reroute_ms: Option<f32>,
}

#[derive(Deserialize, Debug, Clone)]
struct ObserveRequest {
    peer: NodeId,
    we_heard: bool,
    they_heard: bool,
}

#[derive(Deserialize, Debug, Clone)]
struct HelloRequest {
    peer: NodeId,
    seq: u32,
    heard: Vec<NodeId>,
}

#[derive(Deserialize, Debug, Clone)]
struct SendRequest {
    dst: NodeId,
    payload: String, // base64
}

#[derive(Serialize, Debug, Clone)]
struct SendResponse {
    frame: String, // base64
}

#[derive(Deserialize, Debug, Clone)]
struct OpenRequest {
    src: NodeId,
    frame: String, // base64
}

#[derive(Serialize, Debug, Clone)]
struct OpenResponse {
    payload: String, // base64
}

#[derive(Deserialize, Debug, Clone)]
struct PeerRequest {
    peer: NodeId,
}

fn port() -> u16 {
    std::env::var("TRIOS_MESH_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

fn with_state(
    state: Arc<RwLock<MeshState>>,
) -> impl Filter<Extract = (Arc<RwLock<MeshState>>,), Error = Infallible> + Clone {
    warp::any().map(move || state.clone())
}

fn format_etx(etx: f32) -> String {
    if etx.is_infinite() {
        "dead".to_string()
    } else if etx <= 1.0 {
        "perfect".to_string()
    } else if etx <= 2.0 {
        "good".to_string()
    } else if etx <= 4.0 {
        "fair".to_string()
    } else {
        "poor".to_string()
    }
}

async fn health_handler(state: Arc<RwLock<MeshState>>) -> Result<impl warp::Reply, Infallible> {
    let state = state.read().await;
    Ok(json(&HealthResponse {
        status: "ok".to_string(),
        node_id: state.node.id,
    }))
}

async fn status_handler(state: Arc<RwLock<MeshState>>) -> Result<impl warp::Reply, Infallible> {
    let state = state.read().await;
    let node = &state.node;

    let neighbors = node
        .etx
        .neighbors()
        .iter()
        .map(|(id, etx)| NeighborStatus {
            id: *id,
            etx: *etx,
            etx_label: format_etx(*etx),
        })
        .collect();

    let routes = node
        .etx
        .path_routes()
        .iter()
        .map(|(dst, nh, etx)| RouteStatus {
            destination: *dst,
            next_hop: Some(*nh),
            path_etx: Some(*etx),
        })
        .collect();

    let sessions = state
        .peer_keys
        .keys()
        .map(|peer| SessionStatus {
            peer: *peer,
            has_session: node.has_session(*peer),
        })
        .collect();

    let metrics = MetricSnapshot {
        link_loss_to_reroute_ms: node.metrics.link_loss_to_reroute_ms,
        node_off_to_reroute_ms: node.metrics.node_off_to_reroute_ms,
    };

    Ok(json(&StatusResponse {
        node_id: node.id,
        neighbors,
        routes,
        sessions,
        metrics,
    }))
}

async fn observe_handler(
    req: ObserveRequest,
    state: Arc<RwLock<MeshState>>,
) -> Result<impl warp::Reply, Infallible> {
    let mut state = state.write().await;
    state.node.etx.record(req.peer, req.we_heard, req.they_heard);
    Ok(json(&serde_json::json!({ "ok": true })))
}

async fn hello_handler(
    req: HelloRequest,
    state: Arc<RwLock<MeshState>>,
) -> Result<impl warp::Reply, Infallible> {
    let mut state = state.write().await;
    let my_id = state.node.id;
    let hello = Hello::new(req.peer, req.seq, Hello::now_ms(), req.heard, [0u8; 16]);
    let heard_us = hello.reports_hearing(my_id);
    // Update ETX as if we heard the peer and the peer does not hear us yet.
    // A real implementation would parse the HELLO MAC and reverse-link quality.
    state.node.etx.record(req.peer, true, heard_us);
    Ok(json(&serde_json::json!({
        "ok": true,
        "peer": req.peer,
        "heard_us": heard_us
    })))
}

async fn send_handler(
    req: SendRequest,
    state: Arc<RwLock<MeshState>>,
) -> Result<impl warp::Reply, Infallible> {
    let payload = match BASE64.decode(&req.payload) {
        Ok(p) => p,
        Err(_) => {
            return Ok(warp::reply::with_status(
                json(&serde_json::json!({"error": "invalid base64 payload"})),
                warp::http::StatusCode::BAD_REQUEST,
            ))
        }
    };

    let mut state = state.write().await;
    let ttl = 8; // DEFAULT_TTL is private in router.rs; mirror it here.
    match state.node.seal_data(req.dst, ttl, &payload) {
        Some(frame) => Ok(warp::reply::with_status(
            json(&SendResponse {
                frame: BASE64.encode(&frame),
            }),
            warp::http::StatusCode::OK,
        )),
        None => Ok(warp::reply::with_status(
            json(&serde_json::json!({
                "error": "no session or seal failed"
            })),
            warp::http::StatusCode::SERVICE_UNAVAILABLE,
        )),
    }
}

async fn open_handler(
    req: OpenRequest,
    state: Arc<RwLock<MeshState>>,
) -> Result<impl warp::Reply, Infallible> {
    let frame = match BASE64.decode(&req.frame) {
        Ok(f) => f,
        Err(_) => {
            return Ok(warp::reply::with_status(
                json(&serde_json::json!({"error": "invalid base64 frame"})),
                warp::http::StatusCode::BAD_REQUEST,
            ))
        }
    };

    let mut state = state.write().await;
    match state.node.open_data(req.src, &frame) {
        Ok(payload) => Ok(warp::reply::with_status(
            json(&OpenResponse {
                payload: BASE64.encode(&payload),
            }),
            warp::http::StatusCode::OK,
        )),
        Err(e) => Ok(warp::reply::with_status(
            json(&serde_json::json!({ "error": format!("{:?}", e) })),
            warp::http::StatusCode::UNAUTHORIZED,
        )),
    }
}

async fn force_dead_handler(
    req: PeerRequest,
    state: Arc<RwLock<MeshState>>,
) -> Result<impl warp::Reply, Infallible> {
    let mut state = state.write().await;
    state.node.etx.force_dead(req.peer);
    state.node.on_link_loss_detected();
    state.node.on_reroute_completed();
    Ok(json(&serde_json::json!({ "ok": true, "peer": req.peer })))
}

async fn seed_peer_handler(
    req: PeerRequest,
    state: Arc<RwLock<MeshState>>,
) -> Result<impl warp::Reply, Infallible> {
    let mut state = state.write().await;
    let seed = deterministic_seed(req.peer);
    let peer_key = StaticKey::from_seed(seed).public();
    let peer_bytes = peer_key.to_bytes();
    let my_seed = deterministic_seed(state.node.id);
    let my_key = StaticKey::from_seed(my_seed);
    let session = my_key.session_with(&peer_key, state.node.id < req.peer);
    state.node.add_session(req.peer, session);
    state.peer_keys.insert(req.peer, peer_bytes.to_vec());
    Ok(json(&serde_json::json!({
        "ok": true,
        "peer": req.peer,
        "public_key": BASE64.encode(peer_bytes)
    })))
}

/// Deterministic 32-byte seed from a node id.
fn deterministic_seed(id: NodeId) -> [u8; 32] {
    let mut seed = [0u8; 32];
    seed[0..4].copy_from_slice(&id.to_be_bytes());
    // Fill remainder with a simple pattern; not for production keys.
    for (i, slot) in seed.iter_mut().enumerate().skip(4) {
        *slot = ((i * 7 + (id as usize)) % 251) as u8;
    }
    seed
}

async fn link_loss_handler(
    state: Arc<RwLock<MeshState>>,
) -> Result<impl warp::Reply, Infallible> {
    let mut state = state.write().await;
    state.node.on_link_loss_detected();
    Ok(json(&serde_json::json!({ "ok": true })))
}

async fn reroute_handler(
    state: Arc<RwLock<MeshState>>,
) -> Result<impl warp::Reply, Infallible> {
    let mut state = state.write().await;
    state.node.on_reroute_completed();
    Ok(json(&serde_json::json!({ "ok": true })))
}

fn routes(
    state: Arc<RwLock<MeshState>>,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let health = warp::path("health")
        .and(warp::get())
        .and(with_state(state.clone()))
        .and_then(health_handler);

    let status = warp::path("status")
        .and(warp::get())
        .and(with_state(state.clone()))
        .and_then(status_handler);

    let observe = warp::path("observe")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_state(state.clone()))
        .and_then(observe_handler);

    let hello = warp::path("hello")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_state(state.clone()))
        .and_then(hello_handler);

    let send = warp::path("send")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_state(state.clone()))
        .and_then(send_handler);

    let open = warp::path("open")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_state(state.clone()))
        .and_then(open_handler);

    let force_dead = warp::path("force-dead")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_state(state.clone()))
        .and_then(force_dead_handler);

    let seed_peer = warp::path("seed-peer")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_state(state.clone()))
        .and_then(seed_peer_handler);

    let link_loss = warp::path("link-loss")
        .and(warp::post())
        .and(with_state(state.clone()))
        .and_then(link_loss_handler);

    let reroute = warp::path("reroute")
        .and(warp::post())
        .and(with_state(state.clone()))
        .and_then(reroute_handler);

    let cors = warp::cors()
        .allow_any_origin()
        .allow_methods(vec!["GET", "POST", "OPTIONS"])
        .allow_headers(vec!["content-type"]);

    health
        .or(status)
        .or(observe)
        .or(hello)
        .or(send)
        .or(open)
        .or(force_dead)
        .or(seed_peer)
        .or(link_loss)
        .or(reroute)
        .with(cors)
}

#[tokio::main]
async fn main() {
    let node_id: NodeId = std::env::var("TRIOS_MESH_NODE_ID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    let state = Arc::new(RwLock::new(MeshState::new(node_id)));
    let port = port();
    println!("[clade-meshd] node_id={node_id} port={port}");
    warp::serve(routes(state)).run(([127, 0, 0, 1], port)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_seed_is_stable() {
        let a = deterministic_seed(42);
        let b = deterministic_seed(42);
        assert_eq!(a, b);
        assert_eq!(a[0..4], 42u32.to_be_bytes());
    }

    #[test]
    fn format_etx_labels() {
        assert_eq!(format_etx(1.0), "perfect");
        assert_eq!(format_etx(1.5), "good");
        assert_eq!(format_etx(3.0), "fair");
        assert_eq!(format_etx(10.0), "poor");
        assert_eq!(format_etx(f32::INFINITY), "dead");
    }
}
