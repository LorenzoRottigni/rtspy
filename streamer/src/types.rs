use crossbeam::channel::Sender;

pub type FrameBytes = Vec<u8>;
pub type FrameSender = Sender<FrameBytes>;
