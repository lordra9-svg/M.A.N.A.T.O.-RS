//! Minimal PulseAudio output using the stable `pa_simple` API.
//!
//! The library is loaded at runtime with `libloading`, so building the engine
//! does not require PulseAudio development headers. On a desktop with
//! PulseAudio or PipeWire's Pulse server this gives us real output with no
//! extra build dependencies; otherwise `start` returns an error and the engine
//! falls back to silence.

use std::ffi::{c_char, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use anyhow::{bail, Context};

use crate::Mixer;

const PA_SAMPLE_S16LE: c_int = 3;
const PA_STREAM_PLAYBACK: c_int = 1;
const CHUNK_FRAMES: usize = 512;

/// Raw PulseAudio stream handle. `pa_simple` is only used by the writer thread
/// after the handle is created, so moving the pointer there is sound.
struct SendStream(*mut c_void);
unsafe impl Send for SendStream {}

#[repr(C)]
struct PaSampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

type PaSimpleNew = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    c_int,
    *const c_char,
    *const c_char,
    *const PaSampleSpec,
    *const c_void,
    *const c_void,
    *mut c_int,
) -> *mut c_void;
type PaSimpleWrite = unsafe extern "C" fn(*mut c_void, *const c_void, usize, *mut c_int) -> c_int;
type PaSimpleDrain = unsafe extern "C" fn(*mut c_void, *mut c_int) -> c_int;
type PaSimpleFree = unsafe extern "C" fn(*mut c_void);
type PaSimpleGetLatency = unsafe extern "C" fn(*mut c_void, *mut c_int) -> u64;

/// Handle for the background writer thread. Dropping it stops the thread.
pub struct PulseSink {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for PulseSink {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Opens a PulseAudio playback stream and starts mixing into it.
pub fn start(mixer: Arc<Mutex<Mixer>>, rate: u32, channels: usize) -> anyhow::Result<PulseSink> {
    if channels == 0 || channels > u8::MAX as usize {
        bail!("audio channel count must be between 1 and 255");
    }

    // Keep the library loaded for as long as the thread uses the symbols.
    let library = unsafe { libloading::Library::new("libpulse-simple.so.0") }
        .or_else(|_| unsafe { libloading::Library::new("libpulse-simple.so") })
        .context("libpulse-simple is not available")?;

    let pa_new: PaSimpleNew = unsafe { *library.get(b"pa_simple_new\0")? };
    let pa_write: PaSimpleWrite = unsafe { *library.get(b"pa_simple_write\0")? };
    let pa_drain: PaSimpleDrain = unsafe { *library.get(b"pa_simple_drain\0")? };
    let pa_free: PaSimpleFree = unsafe { *library.get(b"pa_simple_free\0")? };
    let pa_latency: Option<PaSimpleGetLatency> = unsafe {
        library
            .get::<PaSimpleGetLatency>(b"pa_simple_get_latency\0")
            .ok()
            .map(|s| *s)
    };

    let name = b"manato\0";
    let stream_name = b"M.A.N.A.T.O.-RS\0";
    let spec = PaSampleSpec {
        format: PA_SAMPLE_S16LE,
        rate,
        channels: channels as u8,
    };
    let mut error = 0;
    let stream = unsafe {
        pa_new(
            ptr::null(),
            name.as_ptr() as *const c_char,
            PA_STREAM_PLAYBACK,
            ptr::null(),
            stream_name.as_ptr() as *const c_char,
            &spec,
            ptr::null(),
            ptr::null(),
            &mut error,
        )
    };
    if stream.is_null() {
        bail!("pa_simple_new failed (error {error})");
    }

    if let Some(latency) = pa_latency {
        let mut error = 0;
        let usec = unsafe { latency(stream, &mut error) };
        if error == 0 {
            log::info!(
                "pulse audio ready: {rate} Hz, {channels} ch, {} ms latency",
                usec / 1000
            );
        }
    }

    let stream = SendStream(stream);
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let handle = thread::Builder::new()
        .name("manato-audio".into())
        .spawn(move || {
            // Moving the library into the thread keeps the function pointers
            // valid until the writer loop exits.
            let _library = library;
            let stream = stream;
            let mut samples = vec![0.0f32; CHUNK_FRAMES * channels];
            let mut pcm = vec![0i16; samples.len()];
            loop {
                if thread_stop.load(Ordering::Relaxed) {
                    break;
                }
                match mixer.lock() {
                    Ok(mut mixer) => mixer.fill(&mut samples),
                    Err(_) => break,
                }
                for (dst, src) in pcm.iter_mut().zip(samples.iter()) {
                    *dst = (src.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                }
                let mut error = 0;
                let written = unsafe {
                    pa_write(
                        stream.0,
                        pcm.as_ptr() as *const c_void,
                        pcm.len() * std::mem::size_of::<i16>(),
                        &mut error,
                    )
                };
                if written < 0 {
                    log::warn!("pulse audio write failed (error {error})");
                    break;
                }
            }
            let mut error = 0;
            unsafe {
                pa_drain(stream.0, &mut error);
                pa_free(stream.0);
            }
        })
        .context("failed to spawn audio thread")?;

    Ok(PulseSink {
        stop,
        handle: Some(handle),
    })
}
