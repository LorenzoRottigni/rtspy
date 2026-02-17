use opencv::{highgui, prelude::*, videoio};
use std::env;
use std::thread;
use std::time::Duration;

pub fn main() -> opencv::Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <rtsp_url>", args[0]);
        return Ok(());
    }

    let rtsp_url = &args[1];

    let mut cap = videoio::VideoCapture::from_file(rtsp_url, videoio::CAP_FFMPEG)?;
    if !cap.is_opened()? {
        panic!("Failed to open RTSP stream: {}", rtsp_url);
    }

    highgui::named_window(rtsp_url, highgui::WINDOW_AUTOSIZE)?;

    loop {
        let mut frame = Mat::default();
        if !cap.read(&mut frame)? || frame.empty() {
            // Retry on empty frame
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        highgui::imshow(rtsp_url, &frame)?;

        if highgui::wait_key(1)? == 27 {
            break;
        }
    }

    Ok(())
}
