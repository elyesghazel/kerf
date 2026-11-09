//! Playback engine: a video thread running the compositor and an audio thread
//! running the mixer, both slaved to one wall clock owned by the UI.
//!
//! While paused, the latest seek request wins (older ones are dropped), so
//! scrubbing never queues up work. While playing, the video thread renders
//! whatever frame the clock says is due and simply skips frames it cannot
//! make in time.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver, Sender};
use kerf_core::{Project, Time};
use kerf_media::{Frame, SAMPLE_RATE};
use kerf_render::{Compositor, Mixer};

pub struct Rendered {
    pub t: Time,
    pub frame: Frame,
    pub ms: f32,
}

enum VMsg {
    Project(Arc<Project>),
    Seek(Time),
    Play { t: Time, at: Instant },
    Pause(Time),
    Size(u32, u32),
    Proxies(bool),
}

enum AMsg {
    Project(Arc<Project>),
    Play { t: Time, at: Instant },
    Pause,
}

#[derive(Default)]
pub struct Meters {
    pub peak_l: AtomicU32,
    pub peak_r: AtomicU32,
}

impl Meters {
    pub fn get(&self) -> (f32, f32) {
        (
            f32::from_bits(self.peak_l.load(Ordering::Relaxed)),
            f32::from_bits(self.peak_r.load(Ordering::Relaxed)),
        )
    }
}

pub struct Engine {
    vtx: Sender<VMsg>,
    atx: Sender<AMsg>,
    pub frames: Receiver<Rendered>,
    playing: bool,
    anchor_t: Time,
    anchor: Instant,
    pub meters: Arc<Meters>,
    pub audio_ok: bool,
    _stream: Option<cpal::Stream>,
    pub size: (u32, u32),
    pub loop_range: Option<(Time, Time)>,
}

impl Engine {
    pub fn new(ctx: egui::Context, project: Arc<Project>) -> Self {
        let (vtx, vrx) = unbounded::<VMsg>();
        let (ftx, frx) = unbounded::<Rendered>();
        let p0 = project.clone();
        std::thread::Builder::new()
            .name("kerf-video".into())
            .spawn(move || video_thread(vrx, ftx, ctx, p0))
            .unwrap();

        let (atx, arx) = unbounded::<AMsg>();
        let meters = Arc::new(Meters::default());
        let (stream, audio_ok) = match start_audio(arx, project, meters.clone()) {
            Ok(s) => (Some(s), true),
            Err(e) => {
                log::warn!("audio output unavailable: {e}");
                (None, false)
            }
        };
        Engine {
            vtx,
            atx,
            frames: frx,
            playing: false,
            anchor_t: Time::ZERO,
            anchor: Instant::now(),
            meters,
            audio_ok,
            _stream: stream,
            size: (0, 0),
            loop_range: None,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// The current playback time (only advances while playing).
    pub fn now(&self) -> Time {
        if self.playing {
            self.anchor_t + Time::from_secs_f64(self.anchor.elapsed().as_secs_f64())
        } else {
            self.anchor_t
        }
    }

    pub fn set_project(&self, p: Arc<Project>) {
        let _ = self.vtx.send(VMsg::Project(p.clone()));
        let _ = self.atx.send(AMsg::Project(p));
    }

    pub fn set_size(&mut self, w: u32, h: u32) {
        if (w, h) != self.size {
            self.size = (w, h);
            let _ = self.vtx.send(VMsg::Size(w, h));
        }
    }

    pub fn set_proxies(&self, on: bool) {
        let _ = self.vtx.send(VMsg::Proxies(on));
    }

    pub fn seek(&mut self, t: Time) {
        let t = t.max(Time::ZERO);
        self.anchor_t = t;
        self.anchor = Instant::now();
        if self.playing {
            self.play_from(t);
        } else {
            let _ = self.vtx.send(VMsg::Seek(t));
        }
    }

    pub fn play_from(&mut self, t: Time) {
        let at = Instant::now();
        self.playing = true;
        self.anchor_t = t;
        self.anchor = at;
        let _ = self.vtx.send(VMsg::Play { t, at });
        let _ = self.atx.send(AMsg::Play { t, at });
    }

    pub fn pause(&mut self) {
        let t = self.now();
        self.playing = false;
        self.anchor_t = t;
        let _ = self.vtx.send(VMsg::Pause(t));
        let _ = self.atx.send(AMsg::Pause);
    }

    pub fn toggle(&mut self, end: Time) {
        if self.playing {
            self.pause();
        } else {
            let t = self.now();
            // Pressing play at the end starts over.
            let t = if t >= end && end > Time::ZERO { Time::ZERO } else { t };
            self.play_from(t);
        }
    }
}

fn video_thread(rx: Receiver<VMsg>, tx: Sender<Rendered>, ctx: egui::Context, mut project: Arc<Project>) {
    let mut comp = Compositor::default();
    let mut size = (640u32, 360u32);
    let mut playing: Option<(Time, Instant)> = None;
    let mut pending: Option<Time> = Some(Time::ZERO);
    let mut last_frame: i64 = -1;
    let mut dirty = true;
    loop {
        // Drain messages; block only when idle and paused.
        let msg = if playing.is_none() && pending.is_none() {
            rx.recv().ok()
        } else {
            rx.try_recv().ok()
        };
        let mut got = msg;
        while let Some(m) = got.take() {
            match m {
                VMsg::Project(p) => {
                    project = p;
                    dirty = true;
                    if playing.is_none() && pending.is_none() {
                        pending = Some(Time::from_frame(last_frame.max(0), project.timeline.fps));
                    }
                }
                VMsg::Seek(t) => pending = Some(t),
                VMsg::Play { t, at } => {
                    playing = Some((t, at));
                    last_frame = -1;
                }
                VMsg::Pause(t) => {
                    playing = None;
                    pending = Some(t);
                }
                VMsg::Size(w, h) => {
                    size = (w.max(2) & !1, h.max(2) & !1);
                    comp.reset();
                    dirty = true;
                    pending = pending.or(Some(Time::from_frame(last_frame.max(0), project.timeline.fps)));
                }
                VMsg::Proxies(on) => {
                    comp.use_proxies = on;
                    comp.reset();
                    dirty = true;
                }
            }
            got = rx.try_recv().ok();
        }
        let fps = project.timeline.fps;
        let t = if let Some((t0, at)) = playing {
            let t = t0 + Time::from_secs_f64(at.elapsed().as_secs_f64());
            let f = t.frame_at(fps);
            if f == last_frame && !dirty {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            t
        } else if let Some(t) = pending.take() {
            t
        } else {
            continue;
        };
        dirty = false;
        last_frame = t.frame_at(fps);
        let start = Instant::now();
        let mut frame = Frame::new(size.0, size.1);
        // Render at the exact frame boundary so preview == export.
        let tf = Time::from_frame(last_frame, fps);
        comp.render(&project, tf, &mut frame);
        let ms = start.elapsed().as_secs_f32() * 1000.0;
        let _ = tx.send(Rendered { t: tf, frame, ms });
        ctx.request_repaint();
    }
}

fn start_audio(rx: Receiver<AMsg>, project: Arc<Project>, meters: Arc<Meters>) -> anyhow::Result<cpal::Stream> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let dev = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no output device"))?;
    let def = dev.default_output_config()?;
    let channels = def.channels().max(1);
    let config = cpal::StreamConfig {
        channels,
        sample_rate: SAMPLE_RATE,
        buffer_size: cpal::BufferSize::Default,
    };
    // ~0.5 s of stereo audio between the mixer thread and the device.
    let (mut prod, mut cons) = rtrb::RingBuffer::<f32>::new(SAMPLE_RATE as usize);
    let flush = Arc::new(AtomicU64::new(0));
    let flush_cb = flush.clone();
    let mut seen_flush = 0u64;
    let ch = channels as usize;
    let stream = dev.build_output_stream::<f32, _, _>(
        config,
        move |data: &mut [f32], _| {
            let f = flush_cb.load(Ordering::Relaxed);
            if f != seen_flush {
                seen_flush = f;
                while cons.pop().is_ok() {}
            }
            for frame in data.chunks_mut(ch) {
                let l = cons.pop().unwrap_or(0.0);
                let r = cons.pop().unwrap_or(l);
                match frame.len() {
                    1 => frame[0] = (l + r) * 0.5,
                    _ => {
                        frame[0] = l;
                        frame[1] = r;
                        for x in frame.iter_mut().skip(2) {
                            *x = 0.0;
                        }
                    }
                }
            }
        },
        |e| log::warn!("audio stream error: {e}"),
        None,
    )?;
    stream.play()?;

    std::thread::Builder::new().name("kerf-audio".into()).spawn(move || {
        let mut mixer = Mixer::default();
        let mut project = project;
        let mut playing: Option<(i64, Instant)> = None; // (start sample, wall anchor)
        let mut produced: i64 = 0;
        let chunk = 1024usize;
        let mut buf = vec![0f32; chunk * 2];
        let lead = (SAMPLE_RATE as f64 * 0.12) as i64;
        loop {
            let msg = if playing.is_none() { rx.recv().ok() } else { rx.try_recv().ok() };
            let mut got = msg;
            while let Some(m) = got.take() {
                match m {
                    AMsg::Project(p) => {
                        project = p;
                    }
                    AMsg::Play { t, at } => {
                        flush.fetch_add(1, Ordering::Relaxed);
                        let s = (t.as_secs_f64() * SAMPLE_RATE as f64) as i64;
                        playing = Some((s, at));
                        produced = s;
                        mixer.reset();
                    }
                    AMsg::Pause => {
                        flush.fetch_add(1, Ordering::Relaxed);
                        playing = None;
                        meters.peak_l.store(0f32.to_bits(), Ordering::Relaxed);
                        meters.peak_r.store(0f32.to_bits(), Ordering::Relaxed);
                    }
                }
                got = rx.try_recv().ok();
            }
            let Some((s0, at)) = playing else { continue };
            let now = s0 + (at.elapsed().as_secs_f64() * SAMPLE_RATE as f64) as i64;
            // Fell behind (underrun): jump to where the clock is.
            if produced < now {
                produced = now;
            }
            if produced < now + lead && prod.slots() >= chunk * 2 {
                mixer.mix(&project, produced, &mut buf);
                let (mut pl, mut pr) = (0f32, 0f32);
                for fr in buf.chunks_exact(2) {
                    pl = pl.max(fr[0].abs());
                    pr = pr.max(fr[1].abs());
                    let _ = prod.push(fr[0]);
                    let _ = prod.push(fr[1]);
                }
                let decay = |old: u32, new: f32| f32::from_bits(old) * 0.85 + 0.0 * new;
                let l = pl.max(decay(meters.peak_l.load(Ordering::Relaxed), pl));
                let r = pr.max(decay(meters.peak_r.load(Ordering::Relaxed), pr));
                meters.peak_l.store(l.to_bits(), Ordering::Relaxed);
                meters.peak_r.store(r.to_bits(), Ordering::Relaxed);
                produced += chunk as i64;
            } else {
                std::thread::sleep(Duration::from_millis(4));
            }
        }
    })?;
    Ok(stream)
}
