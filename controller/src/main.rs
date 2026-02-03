use opencv::{highgui, prelude::*, videoio};
use std::thread;
use std::time::Duration;

fn main() -> opencv::Result<()> {
    // Open the RTSP stream from your streamer
    let rtsp_url = "rtsp://127.0.0.1:8554/live";
    let mut cap = videoio::VideoCapture::from_file(rtsp_url, videoio::CAP_FFMPEG)?;
    if !cap.is_opened()? {
        panic!("Failed to open RTSP stream: {}", rtsp_url);
    }

    // Create OpenCV window
    highgui::named_window("Controller View", highgui::WINDOW_AUTOSIZE)?;

    loop {
        let mut frame = Mat::default();
        if !cap.read(&mut frame)? {
            // Stream ended or error
            println!("Failed to read frame, retrying...");
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        if frame.empty() {
            // No data yet
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        // Show the frame
        highgui::imshow("Controller View", &frame)?;

        // Exit on ESC key
        if highgui::wait_key(1)? == 27 {
            break;
        }
    }

    Ok(())
}
