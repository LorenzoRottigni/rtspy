use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::{SinkExt, StreamExt};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_sdp as gst_sdp;
use gstreamer_webrtc as gst_webrtc;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Clone)]
struct AppState {
    webrtc: Arc<Mutex<Option<gst::Element>>>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum SignalMessage {
    Offer {
        sdp: String,
    },
    Answer {
        sdp: String,
    },
    Ice {
        candidate: String,
        #[serde(rename = "sdpMLineIndex")]
        sdp_mline_index: u32,
    },
}

#[tokio::main]
async fn main() {
    gst::init().unwrap();

    let pipeline = create_pipeline();

    let webrtc = pipeline.by_name("webrtc").expect("webrtcbin not found");

    let state = AppState {
        webrtc: Arc::new(Mutex::new(Some(webrtc.clone()))),
    };

    pipeline.set_state(gst::State::Playing).unwrap();

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state);

    println!("WebSocket signaling at ws://127.0.0.1:3000/ws");

    axum::serve(
        tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap(),
        app,
    )
    .await
    .unwrap();
}

fn create_pipeline() -> gst::Pipeline {
    let pipeline_str = "\
        videotestsrc ! videoconvert ! x264enc tune=zerolatency ! \
        h264parse config-interval=1 ! rtph264pay config-interval=1 pt=96 ! \
        webrtcbin name=webrtc";

    match gst::parse::launch(pipeline_str) {
        Ok(p) => p.downcast::<gst::Pipeline>().unwrap(),
        Err(e) => {
            eprintln!("Pipeline creation failed: {:?}", e);
            panic!();
        }
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    println!("Client connected");

    let (mut sender, mut receiver) = socket.split();

    let webrtc = state.webrtc.lock().unwrap().clone().unwrap();

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    // Send messages to client
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let _ = sender.send(Message::Text(msg)).await;
        }
    });

    // ICE candidates from GStreamer → client
    let tx_clone = tx.clone();
    webrtc.connect("on-ice-candidate", false, move |values| {
        let sdp_mline_index = values[1].get::<u32>().unwrap();
        let candidate = values[2].get::<String>().unwrap();

        let msg = SignalMessage::Ice {
            candidate,
            sdp_mline_index,
        };

        let json = serde_json::to_string(&msg).unwrap();
        tx_clone.send(json).unwrap();

        None
    });

    // Negotiation needed → create offer
    let tx_clone = tx.clone();
    let webrtc_clone = webrtc.clone();

    webrtc.connect("on-negotiation-needed", false, move |_| {
        let webrtc = webrtc_clone.clone();
        let tx = tx_clone.clone();

        gst::glib::MainContext::default().spawn_local(async move {
            let offer =
                webrtc.emit_by_name::<gst_webrtc::WebRTCSessionDescription>("create-offer", &[]);

            webrtc.emit_by_name::<()>("set-local-description", &[&offer]);

            let sdp = offer.sdp().as_text().unwrap();

            let msg = SignalMessage::Offer { sdp };
            let json = serde_json::to_string(&msg).unwrap();

            tx.send(json).unwrap();
        });

        None
    });

    // Receive messages from client
    while let Some(Ok(msg)) = receiver.next().await {
        if let Message::Text(text) = msg {
            println!("Received: {}", text);

            if let Ok(signal) = serde_json::from_str::<SignalMessage>(&text) {
                match signal {
                    SignalMessage::Answer { sdp } => {
                        let sdp = gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes()).unwrap();

                        let answer = gst_webrtc::WebRTCSessionDescription::new(
                            gst_webrtc::WebRTCSDPType::Answer,
                            sdp,
                        );

                        webrtc.emit_by_name::<()>("set-remote-description", &[&answer]);
                    }
                    SignalMessage::Ice {
                        candidate,
                        sdp_mline_index,
                    } => {
                        webrtc.emit_by_name::<()>(
                            "add-ice-candidate",
                            &[&sdp_mline_index, &candidate.as_str()],
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    println!("Client disconnected");
}
