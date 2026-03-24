// bridge/src/main.rs
use anyhow::{Context, Result};
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    routing::get,
    Router,
};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_rtsp::RTSPLowerTrans;
use gstreamer_sdp as gst_sdp;
use gstreamer_webrtc as gst_webrtc;
use std::sync::Arc;
use tokio::sync::mpsc;

const RTSP_URL: &str = "rtsp://127.0.0.1:8554/live";

struct Bridge {
    pipeline: gst::Pipeline,
    webrtcbin: gst::Element,
    pay: gst::Element,
}

impl Bridge {
    fn new() -> Result<Self> {
        gst::init()?;

        // Create pipeline elements manually
        let pipeline = gst::Pipeline::new();

        let rtspsrc = gst::ElementFactory::make("rtspsrc").build()?;
        rtspsrc.set_property("location", RTSP_URL);
        rtspsrc.set_property("latency", 0u32);
        rtspsrc.set_property("protocols", RTSPLowerTrans::TCP);

        let depay = gst::ElementFactory::make("rtph264depay").build()?;
        let parse = gst::ElementFactory::make("h264parse").build()?;
        let pay = gst::ElementFactory::make("rtph264pay").build()?;
        pay.set_property("config-interval", -1i32); // send SPS/PPS periodically
        pay.set_property_from_str("aggregate-mode", "zero-latency");

        let webrtcbin = gst::ElementFactory::make("webrtcbin").build()?;
        webrtcbin.set_property_from_str("bundle-policy", "max-bundle");

        pipeline.add_many(&[&rtspsrc, &depay, &parse, &pay, &webrtcbin])?;
        gst::Element::link_many(&[&depay, &parse, &pay])?;

        // connect rtspsrc pad-added dynamically
        let depay_clone = depay.clone();
        rtspsrc.connect_pad_added(move |_, src_pad| {
            let sink_pad = depay_clone.static_pad("sink").unwrap();
            if sink_pad.is_linked() {
                return;
            }
            src_pad.link(&sink_pad).unwrap();
        });

        // link pay -> webrtcbin via request pad
        let webrtc_clone = webrtcbin.clone();
        pay.connect_pad_added(move |pay, src_pad| {
            // request a send_rtp_sink pad on webrtcbin
            let pad_name = "send_rtp_sink_0"; // first video stream
            let webrtc_sink = webrtc_clone
                .request_pad_simple(pad_name)
                .expect("failed to request webrtcbin pad");
            src_pad
                .link(&webrtc_sink)
                .expect("failed to link pay -> webrtcbin");
        });

        pipeline.add(&rtspsrc)?;

        Ok(Self {
            pipeline,
            webrtcbin,
            pay,
        })
    }

    async fn start(&self) -> Result<String> {
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let tx = Arc::new(std::sync::Mutex::new(Some(tx)));

        let wb = self.webrtcbin.clone();
        let tx2 = tx.clone();

        self.webrtcbin
            .connect("on-negotiation-needed", false, move |_| {
                let tx2 = tx2.clone();
                let wb_inner = wb.clone();
                let wb_offer = wb.clone();

                let promise = gst::Promise::with_change_func(move |reply| {
                    let reply = reply.unwrap().unwrap();
                    let offer = reply
                        .value("offer")
                        .unwrap()
                        .get::<gst_webrtc::WebRTCSessionDescription>()
                        .unwrap();
                    wb_inner.emit_by_name::<()>(
                        "set-local-description",
                        &[&offer, &None::<gst::Promise>],
                    );
                    let sdp = offer.sdp().as_text().unwrap();
                    if let Some(tx) = tx2.lock().unwrap().take() {
                        let _ = tx.send(sdp);
                    }
                });
                wb_offer.emit_by_name::<()>("create-offer", &[&None::<gst::Structure>, &promise]);
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
    let bridge = Arc::new(match Bridge::new() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Bridge init failed: {e}");
            return;
        }
    });

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

    let offer_sdp = match bridge.start().await {
        Ok(sdp) => sdp,
        Err(e) => {
            eprintln!("Bridge start failed: {e}");
            return;
        }
    };

    let offer_msg = serde_json::json!({ "type": "offer", "sdp": offer_sdp }).to_string();
    if socket.send(Message::Text(offer_msg)).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            Some(ice) = ice_rx.recv() => {
                if socket.send(Message::Text(ice)).await.is_err() {
                    break;
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(txt))) => {
                        let v: serde_json::Value = match serde_json::from_str(&txt) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        match v["type"].as_str() {
                            Some("answer") => {
                                if let Some(sdp) = v["sdp"].as_str() {
                                    bridge.set_answer(sdp).ok();
                                }
                            }
                            Some("ice") => {
                                if let (Some(mline), Some(candidate)) = (
                                    v["mlineIndex"].as_u64(),
                                    v["candidate"].as_str(),
                                ) {
                                    bridge.add_ice_candidate(mline as u32, candidate);
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => break,
                }
            }
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
