use std::net::SocketAddr;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{bail, Context};
use esp_idf_svc::io::EspIOError;
use esp_idf_svc::tls::X509;
use esp_idf_svc::ws::client::{
    EspWebSocketClient, EspWebSocketClientConfig, WebSocketEvent, WebSocketEventType,
};
use esp_idf_svc::ws::FrameType;
use heapless::spsc::{Producer, Queue};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};

use crate::config::websocket::{CONNECTION_TIMEOUT, MESSAGE_QUEUE_SIZE, POLL_INTERVAL};
use crate::{error_diff, AppStateDiff, StateMode};

/// The id the server assigned this connection.
///
/// A `slotmap` key on the server, so it crosses the wire as an object rather
/// than a string — and has to go back the same way in `Unregister`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct ClientId {
    idx: u32,
    version: u32,
}

/// The PEM-encoded ISRG Root X1 certificate at the end of the cert chain
/// for the websocket server at echo.websocket.org.
const SERVER_ROOT_CERT: &[u8] = b"
-----BEGIN CERTIFICATE-----
MIIFazCCA1OgAwIBAgIRAIIQz7DSQONZRGPgu2OCiwAwDQYJKoZIhvcNAQELBQAw
TzELMAkGA1UEBhMCVVMxKTAnBgNVBAoTIEludGVybmV0IFNlY3VyaXR5IFJlc2Vh
cmNoIEdyb3VwMRUwEwYDVQQDEwxJU1JHIFJvb3QgWDEwHhcNMTUwNjA0MTEwNDM4
WhcNMzUwNjA0MTEwNDM4WjBPMQswCQYDVQQGEwJVUzEpMCcGA1UEChMgSW50ZXJu
ZXQgU2VjdXJpdHkgUmVzZWFyY2ggR3JvdXAxFTATBgNVBAMTDElTUkcgUm9vdCBY
MTCCAiIwDQYJKoZIhvcNAQEBBQADggIPADCCAgoCggIBAK3oJHP0FDfzm54rVygc
h77ct984kIxuPOZXoHj3dcKi/vVqbvYATyjb3miGbESTtrFj/RQSa78f0uoxmyF+
0TM8ukj13Xnfs7j/EvEhmkvBioZxaUpmZmyPfjxwv60pIgbz5MDmgK7iS4+3mX6U
A5/TR5d8mUgjU+g4rk8Kb4Mu0UlXjIB0ttov0DiNewNwIRt18jA8+o+u3dpjq+sW
T8KOEUt+zwvo/7V3LvSye0rgTBIlDHCNAymg4VMk7BPZ7hm/ELNKjD+Jo2FR3qyH
B5T0Y3HsLuJvW5iB4YlcNHlsdu87kGJ55tukmi8mxdAQ4Q7e2RCOFvu396j3x+UC
B5iPNgiV5+I3lg02dZ77DnKxHZu8A/lJBdiB3QW0KtZB6awBdpUKD9jf1b0SHzUv
KBds0pjBqAlkd25HN7rOrFleaJ1/ctaJxQZBKT5ZPt0m9STJEadao0xAH0ahmbWn
OlFuhjuefXKnEgV4We0+UXgVCwOPjdAvBbI+e0ocS3MFEvzG6uBQE3xDk3SzynTn
jh8BCNAw1FtxNrQHusEwMFxIt4I7mKZ9YIqioymCzLq9gwQbooMDQaHWBfEbwrbw
qHyGO0aoSCqI3Haadr8faqU9GY/rOPNk3sgrDQoo//fb4hVC1CLQJ13hef4Y53CI
rU7m2Ys6xt0nUW7/vGT1M0NPAgMBAAGjQjBAMA4GA1UdDwEB/wQEAwIBBjAPBgNV
HRMBAf8EBTADAQH/MB0GA1UdDgQWBBR5tFnme7bl5AFzgAiIyBpY9umbbjANBgkq
hkiG9w0BAQsFAAOCAgEAVR9YqbyyqFDQDLHYGmkgJykIrGF1XIpu+ILlaS/V9lZL
ubhzEFnTIZd+50xx+7LSYK05qAvqFyFWhfFQDlnrzuBZ6brJFe+GnY+EgPbk6ZGQ
3BebYhtF8GaV0nxvwuo77x/Py9auJ/GpsMiu/X1+mvoiBOv/2X/qkSsisRcOj/KK
NFtY2PwByVS5uCbMiogziUwthDyC3+6WVwW6LLv3xLfHTjuCvjHIInNzktHCgKQ5
ORAzI4JMPJ+GslWYHb4phowim57iaztXOoJwTdwJx4nLCgdNbOhdjsnvzqvHu7Ur
TkXWStAmzOVyyghqpZXjFaH3pO3JLF+l+/+sKAIuvtd7u+Nxe5AW0wdeRlN8NwdC
jNPElpzVmbUq4JUagEiuTDkHzsxHpFKVK7q4+63SM1N95R1NbdWhscdCb+ZAJzVc
oyi3B43njTOQ5yOf+1CceWxG1bQVs5ZufpsMljq4Ui0/1lvh+wjChP4kqKOJ2qxq
4RgqsahDYVvTH9w7jXbyLeiNdd8XM2w9U/t7y0Ff/9yi0GE44Za4rF2LN9d11TPA
mRGunUHBcnWEvgJBQl9nJEiU0Zsnvgc/ubhPgXRR4Xq37Z0j4r7g1SgEEzwxA57d
emyPxgcYxn/eR44/KJ4EBs+lVDR3veyJm+kXQ99b21/+jh5Xos1AnX5iItreGCc=
-----END CERTIFICATE-----\0";

#[derive(Debug, Clone)]
enum WsMessage {
    Connected,
    Registered {
        client_id: ClientId,
        role: Option<String>,
    },
    State {
        current: usize,
        current_step: usize,
        mode: StateMode,
    },
    Blink,
    /// The deck was rebuilt, so the talk data is stale.
    TalkChanged,
    Error {
        message: String,
    },
    Closed,
}

/// Message received from the server (skip if does not match)
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Message {
    Registered {
        client_id: ClientId,
        /// Logged, never branched on — so a role the server adds later cannot
        /// cost us the `client_id` that arrived in the same frame.
        #[serde(default)]
        role: Option<String>,
    },
    Blink,
    State {
        state: InnerState,
    },
    TalkChange {
        state: InnerState,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "state")]
enum InnerState {
    Init,
    Running {
        current: usize,
        #[serde(default)]
        current_step: usize,
    },
    Done {
        current: usize,
        #[serde(default)]
        current_step: usize,
    },
}

/// Connect to WebSocket server for real-time presentation control
///
/// # Errors
/// Returns error if WebSocket connection fails, registration fails, or message parsing fails
pub fn connect_to_ws(address: SocketAddr, tx: &mpsc::Sender<AppStateDiff>) -> anyhow::Result<()> {
    let uri = format!("ws://{address}/api/ws");
    info!("🦋 WS using URI {uri}");

    let config = EspWebSocketClientConfig {
        server_cert: Some(X509::pem_until_nul(SERVER_ROOT_CERT)),
        reconnect_timeout_ms: CONNECTION_TIMEOUT,
        network_timeout_ms: CONNECTION_TIMEOUT,
        ..Default::default()
    };

    // Use heapless queue for lock-free communication
    // Leak the queue to make it effectively static for the closure
    let ws_queue = Box::leak(Box::new(Queue::<WsMessage, MESSAGE_QUEUE_SIZE>::new()));
    let (mut ws_producer, mut ws_consumer) = ws_queue.split();

    info!("🦋 WS connecting...");

    let mut client = EspWebSocketClient::new(&uri, &config, CONNECTION_TIMEOUT, move |event| {
        handle_event(&mut ws_producer, event);
    })
    .context("creating WS client")?;

    // Wait for connection - poll the queue
    let mut connected = false;
    let poll_count = CONNECTION_TIMEOUT.as_millis() / POLL_INTERVAL.as_millis();
    for _ in 0..poll_count {
        if let Some(first_event) = ws_consumer.dequeue() {
            match first_event {
                WsMessage::Connected => {
                    connected = true;
                    break;
                }
                other => bail!("Expected connected event, got {other:?}"),
            }
        }
        std::thread::sleep(POLL_INTERVAL);
    }

    if !connected {
        bail!("WebSocket connection timeout");
    }

    info!("🦋 WS connnected");

    // Register client
    let message = r#"{"command":"Register","name":"ESP32"}"#;
    info!("Websocket send, text: {message}");
    client.send(FrameType::Text(false), message.as_bytes())?;

    // Track client_id for clean unregistration
    let mut client_id: Option<ClientId> = None;

    // Main message processing loop
    loop {
        let Some(msg) = ws_consumer.dequeue() else {
            // No message available, sleep briefly to avoid busy waiting
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        info!("🦋 WS incoming {msg:?}");

        // Every arm either forwards one diff or leaves the loop, so the send
        // and its failure handling live in one place below rather than once
        // per message kind.
        let diff = match msg {
            WsMessage::Connected => bail!("🦋 WS unexpected connected message"),
            WsMessage::Registered {
                client_id: id,
                role,
            } => {
                let role = role.as_deref().unwrap_or("unknown");
                info!("🦋 WS - registered as {role} with client_id {id:?}");
                client_id = Some(id);
                continue;
            }
            WsMessage::Closed => {
                info!("🦋 WS - closing");
                send_unregister(&mut client, client_id);
                break;
            }
            WsMessage::State {
                current,
                current_step,
                mode,
            } => AppStateDiff::UpdateSlide {
                current,
                current_step,
                mode,
            },
            WsMessage::Blink => AppStateDiff::Blink,
            WsMessage::TalkChanged => AppStateDiff::TalkReload,
            WsMessage::Error { message } => {
                info!("🦋 WS - error {message}");
                error_diff!("{message}")
            }
        };

        if let Err(error) = tx.send(diff) {
            error!("Failed to forward WebSocket message: {error}, stopping WebSocket");
            send_unregister(&mut client, client_id);
            break;
        }
    }

    Ok(())
}

/// Send Unregister command to cleanly disconnect from server
fn send_unregister(client: &mut EspWebSocketClient, client_id: Option<ClientId>) {
    let Some(id) = client_id else {
        debug!("No client_id to unregister");
        return;
    };
    // Serialized rather than formatted by hand: the id is an object, and
    // spelling its shape out here is a second place for it to drift.
    let field = match serde_json::to_string(&id) {
        Ok(field) => field,
        Err(error) => {
            warn!("Failed to serialize client_id: {error}");
            return;
        }
    };
    let msg = format!(r#"{{"command":"Unregister","client":{field}}}"#);
    info!("🦋 WS - sending unregister: {msg}");
    if let Err(e) = client.send(FrameType::Text(false), msg.as_bytes()) {
        warn!("Failed to send Unregister: {e}");
    }
}

fn handle_event(
    producer: &mut Producer<'_, WsMessage>,
    event: &Result<WebSocketEvent, EspIOError>,
) {
    let event = match event {
        Ok(event) => event,
        Err(err) => {
            warn!("📥 WS connection failure {err:?}");
            return;
        }
    };

    let msg = match event.event_type {
        WebSocketEventType::BeforeConnect => {
            info!("📥 WS - before connect");
            return;
        }
        WebSocketEventType::Connected => {
            info!("📥 WS - connected");
            WsMessage::Connected
        }
        WebSocketEventType::Disconnected => {
            info!("📥 WS - disconnected");
            WsMessage::Closed
        }
        WebSocketEventType::Close(reason) => {
            info!("📥 WS - closed: {reason:?}");
            WsMessage::Closed
        }
        WebSocketEventType::Closed => {
            info!("📥 WS - closed");
            WsMessage::Closed
        }

        WebSocketEventType::Text(txt) => {
            info!("📥 WS - text: {txt}");

            let Ok(msg) = serde_json::from_str::<Message>(txt) else {
                debug!("📥 WS - skip the message");
                return;
            };

            match msg {
                Message::Registered { client_id, role } => {
                    WsMessage::Registered { client_id, role }
                }
                Message::Blink => {
                    info!("📥 WS - ⚡️ blink event received");
                    WsMessage::Blink
                }
                Message::Error { message } => WsMessage::Error { message },
                Message::State { state } => {
                    let Some(msg) = slide_update(state) else {
                        return;
                    };
                    msg
                }
                Message::TalkChange { state } => {
                    info!("📥 WS - 📚 the deck was rebuilt");
                    // Queued ahead of the state so the talk is re-fetched even
                    // when the deck has not started and the state below is
                    // dropped.
                    enqueue(producer, WsMessage::TalkChanged);
                    let Some(msg) = slide_update(state) else {
                        return;
                    };
                    msg
                }
            }
        }
        WebSocketEventType::Binary(items) => {
            warn!("📥 WS - skip binary payload {items:?}");
            return;
        }
        WebSocketEventType::Ping => {
            debug!("📥 WS - ping");
            return;
        }
        WebSocketEventType::Pong => {
            debug!("📥 WS - pong");
            return;
        }
    };

    enqueue(producer, msg);
}

/// The slide update an `InnerState` describes, or `None` before the deck has
/// started — there is no slide to show yet.
fn slide_update(state: InnerState) -> Option<WsMessage> {
    let (current, current_step, mode) = match state {
        InnerState::Init => {
            warn!("📥 WS - the deck has not started");
            return None;
        }
        InnerState::Running {
            current,
            current_step,
        } => (current, current_step, StateMode::Running),
        InnerState::Done {
            current,
            current_step,
        } => (current, current_step, StateMode::Done),
    };

    Some(WsMessage::State {
        current,
        current_step,
        mode,
    })
}

/// Hands one message to the main loop. Lock-free; a full queue drops it.
fn enqueue(producer: &mut Producer<'_, WsMessage>, msg: WsMessage) {
    if producer.enqueue(msg).is_err() {
        warn!("WebSocket queue full, dropping message");
    }
}
