use opencv::{highgui, prelude::*, videoio};
use std::thread;
use std::time::Duration;

pub fn start_viewer(rtsp_url: &str, window_name: &str) -> opencv::Result<()> {
    // Try to open with FFmpeg backend
    let mut cap = videoio::VideoCapture::from_file(rtsp_url, videoio::CAP_FFMPEG)?;
    if !cap.is_opened()? {
        eprintln!("❌ Failed to open RTSP stream: {}", rtsp_url);
        return Ok(());
    }

    highgui::named_window(window_name, highgui::WINDOW_AUTOSIZE)?;

    loop {
        let mut frame = opencv::prelude::Mat::default();
        if !cap.read(&mut frame)? || frame.empty() {
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        highgui::imshow(window_name, &frame)?;
        if highgui::wait_key(1)? == 27 {
            break;
        }
    }

    Ok(())
}
