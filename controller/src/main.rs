mod types;
mod viewer;

use types::StreamerUrl;
use viewer::start_viewer;

fn main() -> opencv::Result<()> {
    let rtsp_url: StreamerUrl = "rtsp://127.0.0.1:8554/live".to_string();
    start_viewer(&rtsp_url, "Controller View")?;
    Ok(())
}
