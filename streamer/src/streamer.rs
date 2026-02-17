use crate::types::*;
use crossbeam::channel::Receiver;
use gstreamer as gst;
use gstreamer_app::AppSrc;
use gstreamer_rtsp_server::prelude::*;
use gstreamer_rtsp_server::{RTSPMediaFactory, RTSPServer};
use gstreamer_video::{VideoFormat, VideoInfo};
use std::thread;
use std::time::Duration;

pub fn start_streamer(
    rx: Receiver<FrameBytes>,
    port: &str,
    mount_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    gst::init()?;

    let server = RTSPServer::new();
    server.set_service(port);

    let factory = RTSPMediaFactory::new();
    factory.set_launch(
        "appsrc name=src is-live=true block=true format=time \
         ! videoconvert ! x264enc tune=zerolatency bitrate=500 speed-preset=superfast \
         ! rtph264pay name=pay0 pt=96",
    );

    let mounts = server.mount_points().unwrap();
    mounts.add_factory(mount_path, factory.clone());
    server.attach(None)?;

    println!(
        "RTSP server running at rtsp://127.0.0.1:{}/{}",
        port, mount_path
    );

    factory.connect_media_configure(move |_, media| {
        let pipeline = media
            .element()
            .dynamic_cast::<gst::Bin>()
            .expect("Failed to cast to Bin");

        let appsrc = pipeline
            .by_name("src")
            .expect("Failed to get appsrc")
            .dynamic_cast::<AppSrc>()
            .expect("Failed to cast to AppSrc");

        let info = VideoInfo::builder(VideoFormat::Bgr, 640, 480)
            .fps(gst::Fraction::new(30, 1))
            .build()
            .unwrap();
        appsrc.set_caps(Some(&info.to_caps().unwrap()));

        let rx = rx.clone();
        thread::spawn(move || {
            for bytes in rx.iter() {
                let buffer = gst::Buffer::from_slice(bytes);
                let _ = appsrc.push_buffer(buffer);
                thread::sleep(Duration::from_millis(33));
            }
        });
    });

    Ok(())
}
