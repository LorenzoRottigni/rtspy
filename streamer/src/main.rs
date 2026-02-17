mod capture;
mod streamer;
mod types;

use capture::start_camera;
use crossbeam::channel::unbounded;
use glib::MainLoop;
use streamer::start_streamer;
use types::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (tx, rx) = unbounded::<FrameBytes>();

    start_camera(tx, 640.0, 480.0);

    start_streamer(rx, "8554", "/live")?;

    let main_loop = MainLoop::new(None, false);
    main_loop.run();

    Ok(())
}
