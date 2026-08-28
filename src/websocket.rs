use std::net::SocketAddr;
use std::sync::mpsc;

use anyhow::{bail, Context};
use esp_idf_svc::io::EspIOError;
use esp_idf_svc::ws::client::{
    EspWebSocketClient, EspWebSocketClientConfig, WebSocketEvent, WebSocketEventType,
};
use esp_idf_svc::ws::FrameType;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};

use crate::config::env::TOBOGGAN_PRESENTER_TOKEN;
use crate::config::websocket::{
    COMMAND_FRAME_CAPACITY, CONNECTION_TIMEOUT, EVENT_POLL_INTERVAL, POLL_INTERVAL, RECEIVE_BUFFER,
    TASK_STACK,
};
use crate::{bounded, error_diff, AppStateDiff, ErrorMessage, StateMode};

/// The role the server granted this connection.
///
/// Deserialized through `String` so that a role this firmware predates maps to
/// `Other` instead of failing the frame - losing the `client_id` that arrives
/// alongside it would cost us the ability to unregister cleanly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(from = "String")]
pub enum Role {
    /// May move the deck.
    Presenter,
    /// Receives every broadcast; any command that would move the deck is
    /// refused with an `Error` notification.
    Audience,
    /// Unrecognized, or absent from the frame.
    #[default]
    Other,
}

impl From<String> for Role {
    fn from(raw: String) -> Self {
        match raw.as_str() {
            "Presenter" => Self::Presenter,
            "Audience" => Self::Audience,
            _ => Self::Other,
        }
    }
}

/// The id the server assigned this connection.
///
/// A `slotmap` key on the server, so it crosses the wire as an object rather
/// than a string — and has to go back the same way in `Unregister`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientId {
    idx: u32,
    version: u32,
}

/// The commands this client sends.
///
/// Mirrors `toboggan_core::Command`, which is `#[serde(tag = "command")]` - only
/// the variants this firmware actually uses. Serialized rather than written out
/// by hand so the shape lives in one place; the old string literals were a
/// second copy of the protocol waiting to drift.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "command")]
pub enum Command {
    /// Join the presentation. Must be the first frame sent.
    Register {
        name: &'static str,
        /// Omitted entirely when absent, matching the server's
        /// `skip_serializing_if`, so an untokened box registers exactly as it
        /// did before this existed.
        #[serde(skip_serializing_if = "Option::is_none")]
        token: Option<&'static str>,
    },
    /// Leave cleanly, so the server drops our slot rather than timing it out.
    Unregister { client: ClientId },
    /// Advance one step within the current slide.
    NextStep,
    /// Go back one step within the current slide.
    PreviousStep,
}

/// Serializes one command and writes it to the socket.
fn send(client: &mut EspWebSocketClient, command: &Command) -> anyhow::Result<()> {
    // Into a stack buffer rather than a fresh `String` per command: every
    // variant is `&'static str` or `Copy`, so the frame size is known and
    // small. `impl Write for &mut [u8]` errors rather than truncating, so an
    // oversized token fails loudly at registration instead of sending a
    // half-written frame.
    let mut buffer = [0_u8; COMMAND_FRAME_CAPACITY];
    let mut remaining = buffer.as_mut_slice();
    serde_json::to_writer(&mut remaining, command).context("serialize command")?;
    let written = COMMAND_FRAME_CAPACITY - remaining.len();
    let frame = &buffer[..written];

    // Borrowed for valid UTF-8, which serde_json always emits.
    info!("🦋 WS - sending {}", String::from_utf8_lossy(frame));
    client
        .send(FrameType::Text(false), frame)
        .context("send command")?;
    Ok(())
}

#[derive(Debug, Clone)]
enum WsMessage {
    Connected,
    Registered {
        client_id: ClientId,
        role: Role,
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
        message: ErrorMessage,
    },
    Closed,
}

/// Message received from the server (skip if does not match)
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Message {
    Registered {
        client_id: ClientId,
        /// An unfamiliar role deserializes to `Role::Other` rather than
        /// failing, so it cannot cost us the `client_id` in the same frame.
        #[serde(default)]
        role: Role,
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
pub fn connect_to_ws(
    address: SocketAddr,
    tx: &mpsc::Sender<AppStateDiff>,
    commands: &mpsc::Receiver<Command>,
) -> anyhow::Result<()> {
    let uri = format!("ws://{address}/api/ws");
    info!("🦋 WS using URI {uri}");

    let config = EspWebSocketClientConfig {
        reconnect_timeout_ms: CONNECTION_TIMEOUT,
        network_timeout_ms: CONNECTION_TIMEOUT,
        // Both default to the component's own values - a 4 KB task stack and a
        // 1 KB receive buffer. `handle_event` runs on that stack and parses
        // JSON there, and a frame over the buffer size arrives split.
        task_stack: TASK_STACK,
        buffer_size: RECEIVE_BUFFER,
        ..Default::default()
    };

    // The client requires a `'static` closure, so the sending half has to be
    // owned outright. An `mpsc::Sender` is; a `heapless::spsc::Producer`
    // borrows its queue, which is why this used to leak one queue per call -
    // including per *failed* call, from a retry loop that never gives up.
    let (ws_sender, ws_receiver) = mpsc::channel::<WsMessage>();

    info!("🦋 WS connecting...");

    let mut client = EspWebSocketClient::new(&uri, &config, CONNECTION_TIMEOUT, move |event| {
        handle_event(&ws_sender, event);
    })
    .context("creating WS client")?;

    // Wait for connection
    let mut connected = false;
    let poll_count = CONNECTION_TIMEOUT.as_millis() / POLL_INTERVAL.as_millis();
    for _ in 0..poll_count {
        match ws_receiver.recv_timeout(POLL_INTERVAL) {
            Ok(WsMessage::Connected) => {
                connected = true;
                break;
            }
            Ok(other) => bail!("Expected connected event, got {other:?}"),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bail!("WebSocket event channel closed before connecting")
            }
        }
    }

    if !connected {
        bail!("WebSocket connection timeout");
    }

    info!("🦋 WS connnected");

    // Register client
    send(
        &mut client,
        &Command::Register {
            name: "ESP32",
            token: TOBOGGAN_PRESENTER_TOKEN,
        },
    )?;

    // Anything tapped while the socket was down is stale. Replaying it now
    // would jump the deck by however many times the screen was prodded while
    // it was not listening.
    let mut dropped = 0_usize;
    while commands.try_recv().is_ok() {
        dropped += 1;
    }
    if dropped > 0 {
        info!("🦋 WS - dropped {dropped} command(s) queued while disconnected");
    }

    // Track client_id for clean unregistration
    let mut client_id: Option<ClientId> = None;

    // Main message processing loop
    loop {
        // Drained before the inbound check: the `continue` below runs on almost
        // every pass, so anything after it would only be reached when a message
        // happened to be waiting.
        while let Ok(command) = commands.try_recv() {
            if let Err(error) = send(&mut client, &command) {
                warn!("🦋 WS - could not send {command:?}: {error}");
            }
        }

        // Bounded wait rather than sleep-then-poll: it wakes the moment a frame
        // lands, and still returns often enough to keep draining `commands`.
        let msg = match ws_receiver.recv_timeout(EVENT_POLL_INTERVAL) {
            Ok(msg) => msg,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                warn!("🦋 WS - event channel closed");
                break;
            }
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
                info!("🦋 WS - registered as {role:?} with client_id {id:?}");
                if role != Role::Presenter {
                    warn!(
                        "🦋 WS - registered as {role:?}, so taps cannot move the deck. \
                         Set TOBOGGAN_PRESENTER_TOKEN and start the server with \
                         --presenter-token to match."
                    );
                }
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
                // Once registered, this is very likely a refused command - the
                // server answers "This client is watching, not presenting" and
                // carries on. Taking the whole screen for that would blank the
                // deck mid-talk every time a tap was not allowed. Before
                // registration there is no such benign case, so it still shows.
                if client_id.is_some() {
                    warn!("🦋 WS - server refused: {message}");
                    continue;
                }
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
    if let Err(error) = send(client, &Command::Unregister { client: id }) {
        warn!("Failed to send Unregister: {error}");
    }
}

fn handle_event(sender: &mpsc::Sender<WsMessage>, event: &Result<WebSocketEvent, EspIOError>) {
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
                // Bounded here rather than in `Message`: the wire struct is
                // transient, and it is the stored copy - cloned on every state
                // change - that must not carry an unbounded server string.
                Message::Error { message } => WsMessage::Error {
                    message: bounded(&message),
                },
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
                    enqueue(sender, WsMessage::TalkChanged);
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

    enqueue(sender, msg);
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

/// Hands one message to `connect_to_ws`. The receiver outlives the client that
/// owns this sender, so a failure here means the loop has already stopped.
fn enqueue(sender: &mpsc::Sender<WsMessage>, msg: WsMessage) {
    if sender.send(msg).is_err() {
        warn!("WebSocket receiver gone, dropping message");
    }
}
