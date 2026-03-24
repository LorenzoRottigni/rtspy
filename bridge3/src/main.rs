use anyhow::{anyhow, Context, Result};
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    routing::get,
    Router,
};
use futures::{SinkExt, StreamExt};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_sdp as gst_sdp;
use gstreamer_webrtc as gst_webrtc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

const RTSP_URL: &str = "rtsp://127.0.0.1:8554/live";

struct Bridge {
    pipeline: gst::Pipeline,
    webrtcbin: gst::Element,
    bus_quit: Arc<AtomicBool>,
}

impl Bridge {
    fn new() -> Result<Self> {
        gst::init()?;

        let pipeline: gst::Pipeline = gst::Pipeline::new();
        let bus_quit = Arc::new(AtomicBool::new(false));

        // RTSP source
        let rtspsrc: gst::Element = gst::ElementFactory::make("rtspsrc")
            .build()
            .expect("Failed to create rtspsrc");
        rtspsrc.set_property("location", RTSP_URL);
        rtspsrc.set_property("latency", 200u32);
        rtspsrc.set_property_from_str("protocols", "tcp");

        // H264 depay + pay + webrtcbin
        let depay: gst::Element = gst::ElementFactory::make("rtph264depay")
            .build()
            .expect("Failed to create depay");
        let pay: gst::Element = gst::ElementFactory::make("rtph264pay")
            .build()
            .expect("Failed to create pay");
        pay.set_property("config-interval", -1i32);
        let webrtcbin: gst::Element = gst::ElementFactory::make("webrtcbin")
            .build()
            .expect("Failed to create webrtcbin");
        webrtcbin.set_property_from_str("bundle-policy", "max-bundle");

        // Add elements
        pipeline.add_many(&[&rtspsrc, &depay, &pay, &webrtcbin])?;
        gst::Element::link_many(&[&depay, &pay])?;

        // rtspsrc dynamic pad -> depay
        let depay_clone = depay.clone();
        rtspsrc.connect_pad_added(move |_: &gst::Element, src_pad: &gst::Pad| {
            let sink_pad = depay_clone.static_pad("sink").unwrap();
            if sink_pad.is_linked() {
                return;
            }

            let caps = match src_pad
                .current_caps()
                .or_else(|| Some(src_pad.query_caps(None)))
            {
                Some(caps) => caps,
                None => return,
            };

            let s = match caps.structure(0) {
                Some(s) => s,
                None => return,
            };

            let is_h264 = s.name() == "application/x-rtp"
                && s.get::<&str>("media").ok() == Some("video")
                && s.get::<&str>("encoding-name").ok() == Some("H264");

            if !is_h264 {
                return;
            }

            if let Err(err) = src_pad.link(&sink_pad) {
                eprintln!("Failed to link RTSP pad: {err:?}");
            }
        });

        // pay -> webrtcbin dynamic pad
        let webrtc_clone = webrtcbin.clone();
        pay.connect_pad_added(move |_, src_pad| {
            if let Some(sink_pad) = webrtc_clone.request_pad_simple("send_rtp_sink_0") {
                if let Err(err) = src_pad.link(&sink_pad) {
                    eprintln!("Failed to link pay to webrtcbin: {err:?}");
                }
            }
        });

        Ok(Self {
            pipeline,
            webrtcbin,
            bus_quit,
        })
    }

    async fn start(&self) -> Result<String> {
        self.start_bus_logger();
        let (tx, rx) = oneshot::channel::<String>();
        let tx = Arc::new(Mutex::new(Some(tx)));
        let wb = Arc::new(self.webrtcbin.clone());

        let tx_clone = tx.clone();
        let wb_clone = wb.clone();

        self.webrtcbin
            .connect("on-negotiation-needed", false, move |_| {
                let wb_inner = wb_clone.clone();
                let tx_inner = tx_clone.clone();

                let wb_for_promise = wb_inner.clone();
                let promise = gst::Promise::with_change_func(move |reply| {
                    let reply = reply.unwrap().unwrap();
                    let offer = reply
                        .value("offer")
                        .unwrap()
                        .get::<gst_webrtc::WebRTCSessionDescription>()
                        .unwrap();

                    wb_for_promise.emit_by_name::<()>(
                        "set-local-description",
                        &[&offer, &None::<gst::Promise>],
                    );

                    let sdp_text = offer.sdp().as_text().unwrap();
                    if let Some(sender) = tx_inner.blocking_lock().take() {
                        let _ = sender.send(sdp_text.to_string());
                    }
                });

                wb_inner.emit_by_name::<()>("create-offer", &[&None::<gst::Structure>, &promise]);
                None
            });

        let set_state_res = self
            .pipeline
            .set_state(gst::State::Playing)
            .context("Failed to set pipeline to Playing")?;
        let (state_res, current, pending) = self.pipeline.state(gst::ClockTime::from_seconds(3));
        if state_res.is_err() {
            let details = self
                .pipeline_error()
                .unwrap_or_else(|| "unknown".to_string());
            return Err(anyhow!(
                "Pipeline failed to reach Playing. set_state={set_state_res:?}, current={current:?}, pending={pending:?}, details={details}"
            ));
        }

        Ok(rx.await.context("Never got SDP offer")?)
    }

    fn set_answer(&self, sdp: &str) -> Result<()> {
        let sdp_msg =
            gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes()).context("Failed to parse SDP")?;
        let answer =
            gst_webrtc::WebRTCSessionDescription::new(gst_webrtc::WebRTCSDPType::Answer, sdp_msg);
        self.webrtcbin
            .emit_by_name::<()>("set-remote-description", &[&answer, &None::<gst::Promise>]);
        Ok(())
    }

    fn on_ice_candidate<F: Fn(u32, String) + Send + Sync + 'static>(&self, f: F) {
        self.webrtcbin
            .connect("on-ice-candidate", false, move |values| {
                let mline: u32 = values[1].get().unwrap();
                let candidate: String = values[2].get().unwrap();
                f(mline, candidate);
                None
            });
    }

    fn add_ice_candidate(&self, mline: u32, candidate: &str) {
        self.webrtcbin
            .emit_by_name::<()>("add-ice-candidate", &[&mline, &candidate]);
    }

    fn stop(&self) {
        self.bus_quit.store(true, Ordering::Relaxed);
        self.pipeline.set_state(gst::State::Null).ok();
    }

    fn pipeline_error(&self) -> Option<String> {
        let bus = self.pipeline.bus()?;
        for _ in 0..10 {
            if let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(100)) {
                if let gst::MessageView::Error(err) = msg.view() {
                    let debug = err.debug().unwrap_or_default();
                    return Some(format!("{} {debug}", err.error()));
                }
            }
        }
        None
    }

    fn start_bus_logger(&self) {
        let bus = match self.pipeline.bus() {
            Some(bus) => bus,
            None => return,
        };
        let quit = self.bus_quit.clone();

        std::thread::spawn(move || {
            while !quit.load(Ordering::Relaxed) {
                if let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(200)) {
                    use gst::MessageView;
                    match msg.view() {
                        MessageView::Error(err) => {
                            let debug = err.debug().unwrap_or_default();
                            eprintln!(
                                "GST error from {:?}: {} {}",
                                err.src().map(|s| s.path_string()),
                                err.error(),
                                debug
                            );
                        }
                        MessageView::Warning(warn) => {
                            let debug = warn.debug().unwrap_or_default();
                            eprintln!(
                                "GST warning from {:?}: {} {}",
                                warn.src().map(|s| s.path_string()),
                                warn.error(),
                                debug
                            );
                        }
                        MessageView::StateChanged(state) => {
                            if let Some(src) = msg.src() {
                                let name = src.path_string();
                                eprintln!(
                                    "State changed ({name}): {:?} -> {:?}",
                                    state.old(),
                                    state.current()
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
        });
    }
}

async fn handle_ws(socket: WebSocket) {
    let bridge = Arc::new(Bridge::new().unwrap());
    let (ice_tx, mut ice_rx) = mpsc::channel::<String>(32);

    let (mut ws_tx, mut ws_rx) = socket.split();

    bridge.on_ice_candidate({
        let tx = ice_tx.clone();
        move |mline, candidate| {
            let msg = serde_json::json!({
                "type": "ice",
                "mlineIndex": mline,
                "candidate": candidate
            })
            .to_string();
            let _ = tx.blocking_send(msg);
        }
    });

    let offer_sdp = match bridge.start().await {
        Ok(sdp) => sdp,
        Err(err) => {
            let _ = ws_tx
                .send(Message::Text(
                    serde_json::json!({
                        "type": "error",
                        "message": format!("{err:?}")
                    })
                    .to_string(),
                ))
                .await;
            bridge.stop();
            return;
        }
    };
    let offer_msg = serde_json::json!({ "type": "offer", "sdp": offer_sdp }).to_string();
    if ws_tx.send(Message::Text(offer_msg)).await.is_err() {
        bridge.stop();
        return;
    }

    loop {
        tokio::select! {
            Some(msg) = ws_rx.next() => {
                if let Ok(Message::Text(txt)) = msg {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
                        match v["type"].as_str() {
                            Some("answer") => {
                                if let Some(sdp) = v["sdp"].as_str() {
                                    bridge.set_answer(sdp).ok();
                                }
                            }
                            Some("ice") => {
                                if let (Some(mline), Some(candidate)) =
                                    (v["mlineIndex"].as_u64(), v["candidate"].as_str())
                                {
                                    bridge.add_ice_candidate(mline as u32, candidate);
                                }
                            }
                            _ => {}
                        }
                    }
                } else {
                    break;
                }
            }
            Some(msg) = ice_rx.recv() => {
                if ws_tx.send(Message::Text(msg)).await.is_err() {
                    break;
                }
            }
            else => break,
        }
    }

    bridge.stop();
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("Fatal error: {err:?}");
    }
}

async fn run() -> Result<()> {
    let app = Router::new()
        .route(
            "/ws",
            get(|ws: WebSocketUpgrade| async { ws.on_upgrade(handle_ws) }),
        )
        .route("/", get(|| async { axum::response::Html(INDEX_HTML) }));

    let bind_addr = std::env::var("BIND").unwrap_or_else(|_| "127.0.0.1:3000".to_string());
    println!("Listening on http://{bind_addr}");

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("Failed to bind {bind_addr} (is another instance running?)"))?;
    axum::serve(listener, app).await?;
    Ok(())
}

const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html>
<body style="margin:0;background:#000">
<video id="v" autoplay playsinline muted style="width:100%;height:100vh;object-fit:contain"></video>
<script>
const pc = new RTCPeerConnection();
const ws = new WebSocket("ws://127.0.0.1:3000/ws");

pc.ontrack = e => document.getElementById("v").srcObject = e.streams[0];

pc.onicecandidate = ({ candidate }) => {
  if(candidate) ws.send(JSON.stringify({
    type:"ice",
    mlineIndex:candidate.sdpMLineIndex,
    candidate:candidate.candidate
  }));
};

ws.onmessage = async ({ data }) => {
  const msg = JSON.parse(data);
  if(msg.type === "offer") {
    await pc.setRemoteDescription({ type:"offer", sdp:msg.sdp });
    const answer = await pc.createAnswer();
    await pc.setLocalDescription(answer);
    ws.send(JSON.stringify({ type:"answer", sdp:answer.sdp }));
  } else if(msg.type==="ice") {
    await pc.addIceCandidate({ sdpMLineIndex:msg.mlineIndex, candidate:msg.candidate });
  }
};
</script>
</body>
</html>"#;
