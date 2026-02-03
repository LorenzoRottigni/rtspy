use crossbeam::channel::unbounded;
use gstreamer as gst;
use gstreamer_app::AppSrc;
use gstreamer_rtsp_server::prelude::*;
use gstreamer_rtsp_server::{RTSPMediaFactory, RTSPServer};
use gstreamer_video::{VideoFormat, VideoInfo};
use opencv::{prelude::*, videoio};
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    gst::init()?;

    // RTSP server
    let server = RTSPServer::new();
    server.set_service("8554");

    // Media factory
    let factory = RTSPMediaFactory::new();
    factory.set_launch(
        "appsrc name=src is-live=true block=true format=time \
         ! videoconvert ! x264enc tune=zerolatency bitrate=500 speed-preset=superfast \
         ! rtph264pay name=pay0 pt=96",
    );

    // Mount
    let mounts = server.mount_points().unwrap();
    mounts.add_factory("/live", factory.clone());
    server.attach(None)?;

    println!("RTSP server running at rtsp://127.0.0.1:8554/live");

    // Channel for sending frame bytes (Vec<u8>)
    let (tx, rx) = unbounded::<Vec<u8>>();

    // Camera thread
    thread::spawn(move || {
        let mut cam = videoio::VideoCapture::new(0, videoio::CAP_ANY).unwrap();
        cam.set(videoio::CAP_PROP_FRAME_WIDTH, 640.0).unwrap();
        cam.set(videoio::CAP_PROP_FRAME_HEIGHT, 480.0).unwrap();

        loop {
            let mut frame = Mat::default();
            if cam.read(&mut frame).unwrap() && !frame.empty() {
                // Convert to raw bytes for sending
                let bytes = frame.data_bytes().unwrap().to_vec();
                if tx.send(bytes).is_err() {
                    break;
                }
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }
    });

    // Media configure
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

        // Spawn thread to push bytes into appsrc
        let rx = rx.clone();
        thread::spawn(move || {
            for bytes in rx.iter() {
                let buffer = gst::Buffer::from_slice(bytes);
                let _ = appsrc.push_buffer(buffer);
                thread::sleep(Duration::from_millis(33));
            }
        });
    });

    // Run main loop
    let main_loop = glib::MainLoop::new(None, false);
    main_loop.run();

    Ok(())
}
