//! `kerf-media`: everything that touches media files.
//!
//! kerf talks to the `ffmpeg` / `ffprobe` *binaries* over pipes instead of
//! linking libav*. That keeps the build free of C bindings (no bindgen, no
//! breakage when the system jumps from FFmpeg 8 to 9), lets a decode crash
//! take down a child process instead of the editor, and gives every decoder
//! its own OS thread pool for free.

pub mod assets;
pub mod detect;
pub mod ff;
pub mod probe;
pub mod proxy;
pub mod stream;

pub use assets::{cache_dir, media_key, Thumbs, Waveform};
pub use probe::{probe, Probe};
pub use stream::{grab_frame, AudioStream, Frame, VideoStream, SAMPLE_RATE};
