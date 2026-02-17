use crate::types::*;
use chrono::Local;
use opencv::{core::Point, core::Scalar, imgproc, prelude::*, videoio};
use std::thread;
use std::time::Duration;

pub fn start_camera(tx: FrameSender, width: f64, height: f64) {
    thread::spawn(move || {
        let mut cam = videoio::VideoCapture::new(0, videoio::CAP_ANY).unwrap();
        cam.set(videoio::CAP_PROP_FRAME_WIDTH, width).unwrap();
        cam.set(videoio::CAP_PROP_FRAME_HEIGHT, height).unwrap();

        loop {
            let mut frame = Mat::default();
            if cam.read(&mut frame).unwrap() && !frame.empty() {
                // Overlay timestamp
                let now = Local::now();
                let timestamp = now.format("%Y-%m-%d %H:%M:%S").to_string();
                imgproc::put_text(
                    &mut frame,
                    &timestamp,
                    Point::new(10, 30),
                    imgproc::FONT_HERSHEY_SIMPLEX,
                    1.0,
                    Scalar::new(0.0, 255.0, 0.0, 0.0),
                    2,
                    imgproc::LINE_AA,
                    false,
                )
                .unwrap();

                // Send bytes
                let bytes = frame.data_bytes().unwrap().to_vec();
                if tx.send(bytes).is_err() {
                    break;
                }
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }
    });
}
