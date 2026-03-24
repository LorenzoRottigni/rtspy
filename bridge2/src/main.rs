use anyhow::{Context, Result};
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    routing::get,
    Router,
};
use futures::StreamExt;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_sdp as gst_sdp;
use gstreamer_webrtc as gst_webrtc;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

const RTSP_URL: &str = "rtsp://127.0.0.1:8554/live";

struct Bridge {
    pipeline: gst::Pipeline,
    webrtcbin: gst::Element,
    pay: gst::Element,
}

impl Bridge {
    fn new() -> Result<Self> {
        gst::init()?;
        let pipeline = gst::Pipeline::new();

        // RTSP source
        let rtspsrc = gst::ElementFactory::make("rtspsrc").build()?;
        rtspsrc.set_property("location", RTSP_URL);
        rtspsrc.set_property("latency", 200u32);
        rtspsrc.set_property_from_str("protocols", "tcp");

        // H264 depay + pay
        let depay = gst::ElementFactory::make("rtph264depay").build()?;
        let pay = gst::ElementFactory::make("rtph264pay").build()?;
        pay.set_property("config-interval", -1i32);

        // WebRTC
        let webrtcbin = gst::ElementFactory::make("webrtcbin").build()?;
        webrtcbin.set_property_from_str("bundle-policy", "max-bundle");

        // Add elements
        pipeline.add_many(&[&rtspsrc, &depay, &pay, &webrtcbin])?;
        gst::Element::link_many(&[&depay, &pay])?;

        // rtspsrc dynamic pad -> depay
        let depay_clone = depay.clone();
        rtspsrc.connect_pad_added(move |_, src_pad| {
            let sink_pad = depay_clone.static_pad("sink").unwrap();
            if !sink_pad.is_linked() {
                src_pad.link(&sink_pad).unwrap();
            }
        });

        // pay -> webrtcbin dynamic pad
        let webrtc_clone = webrtcbin.clone();
        pay.connect_pad_added(move |_, src_pad| {
            if let Some(sink_pad) = webrtc_clone.request_pad_simple("send_rtp_sink_0") {
                src_pad.link(&sink_pad).unwrap();
            }
        });

        Ok(Self {
            pipeline,
            webrtcbin,
            pay,
        })
    }

    async fn start(&self) -> Result<String> {
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let tx = Arc::new(Mutex::new(Some(tx)));
        let wb_arc = Arc::new(self.webrtcbin.clone());
        let tx_arc = tx.clone();

        self.webrtcbin
            .connect("on-negotiation-needed", false, move |_| {
                let wb_clone = wb_arc.clone();
                let wb_clone_2 = wb_clone.clone();
                let tx_clone = tx_arc.clone();

                let promise = gst::Promise::with_change_func(move |reply| {
                    let reply = reply.unwrap().unwrap();
                    let offer = reply
                        .value("offer")
                        .unwrap()
                        .get::<gst_webrtc::WebRTCSessionDescription>()
                        .unwrap();

                    wb_clone.clone().emit_by_name::<()>(
                        "set-local-description",
                        &[&offer, &None::<gst::Promise>],
                    );

                    let sdp = offer.sdp().as_text().unwrap();
                    if let Some(sender) = tx_clone.blocking_lock().take() {
                        let _ = sender.send(sdp);
                    }
                });

                wb_clone_2.emit_by_name::<()>("create-offer", &[&None::<gst::Structure>, &promise]);
                None
            });

        self.pipeline.set_state(gst::State::Playing)?;
        let sdp = rx.await.context("never got SDP offer")?;
        Ok(sdp)
    }

    fn set_answer(&self, sdp: &str) -> Result<()> {
        let sdp = gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes()).context("bad SDP")?;
        let answer =
            gst_webrtc::WebRTCSessionDescription::new(gst_webrtc::WebRTCSDPType::Answer, sdp);
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
        self.pipeline.set_state(gst::State::Null).ok();
    }
}

async fn handle_ws(mut socket: WebSocket) {
    let bridge = Arc::new(Bridge::new().unwrap());
    let (ice_tx, mut ice_rx) = mpsc::channel::<String>(32);

    bridge.on_ice_candidate({
        let tx = ice_tx.clone();
        move |mline, candidate| {
            let msg = serde_json::json!({
                "type": "ice",
                "mlineIndex": mline,
                "candidate": candidate,
            })
            .to_string();
            let _ = tx.blocking_send(msg);
        }
    });

    let offer_sdp = bridge.start().await.unwrap();
    let offer_msg = serde_json::json!({ "type": "offer", "sdp": offer_sdp }).to_string();
    socket.send(Message::Text(offer_msg)).await.unwrap();

    while let Some(msg) = socket.recv().await {
        match msg {
            Ok(Message::Text(txt)) => {
                let v: serde_json::Value = serde_json::from_str(&txt).unwrap_or_default();
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
            _ => break,
        }
    }

    bridge.stop();
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route(
            "/ws",
            get(|ws: WebSocketUpgrade| async { ws.on_upgrade(handle_ws) }),
        )
        .route("/", get(|| async { axum::response::Html(INDEX_HTML) }));

    println!("Listening on http://127.0.0.1:3000");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();
    axum::serve(listener, app).await.unwrap();
}

const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html>
<body style="margin:0;background:#000">
  <video id="v" autoplay playsinline muted style="width:100%;height:100vh;object-fit:contain"></video>
  <script>
    const pc  = new RTCPeerConnection();
    const ws  = new WebSocket("ws://127.0.0.1:3000/ws");

    pc.ontrack = e => document.getElementById("v").srcObject = e.streams[0];

    pc.onicecandidate = ({ candidate }) => {
      if (candidate) ws.send(JSON.stringify({
        type: "ice",
        mlineIndex: candidate.sdpMLineIndex,
        candidate: candidate.candidate,
      }));
    };

    ws.onmessage = async ({ data }) => {
      const msg = JSON.parse(data);
      if (msg.type === "offer") {
        await pc.setRemoteDescription({ type: "offer", sdp: msg.sdp });
        const answer = await pc.createAnswer();
        await pc.setLocalDescription(answer);
        ws.send(JSON.stringify({ type: "answer", sdp: answer.sdp }));
      } else if (msg.type === "ice") {
        await pc.addIceCandidate({ sdpMLineIndex: msg.mlineIndex, candidate: msg.candidate });
      }
    };
  </script>
</body>
</html>"#;
