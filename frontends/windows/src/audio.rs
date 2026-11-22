//! Plays the core's audio ring through the default output device
//! (frontends/macos/src/Audio.m, spec §6.6, §11.2): WASAPI in shared mode,
//! driven by its buffer event, on a thread of its own registered with MMCSS.
//!
//! The thread does nothing but move samples from the ring to the device: no
//! locks, and the machine is never touched. It follows the default device as
//! the Mac's default output unit does: when the user picks another one, or
//! the device goes away, the stream is opened again on the new default.

use std::sync::Arc;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use juku::audio::AudioRing;
use windows::Win32::Foundation::{CloseHandle, HANDLE, PROPERTYKEY, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW, SetEvent, WaitForMultipleObjects,
};
use windows::core::{PCWSTR, Result, implement, w};


pub const SAMPLE_RATE: u32 = 48_000;

/// Samples the ring holds after an underrun before playing again: about
/// 43 ms, the latency (as juku_audio_new on the Mac).
pub const PRIME: usize = 2048;

// The device buffer asked for: 20 ms.
const BUFFER_DURATION: i64 = 200_000; // 100 ns units

// Without a device, look for one again this often.
const RETRY_MS: u32 = 1000;

const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;

/// An event handle that may be passed between threads.
#[derive(Clone, Copy)]
struct Event(isize);

impl Event {
    fn new() -> Result<Event> {
        Ok(Event(unsafe { CreateEventW(None, false, false, None)? }.0 as isize))
    }

    fn handle(self) -> HANDLE {
        HANDLE(self.0 as *mut _)
    }

    fn set(self) {
        let _ = unsafe { SetEvent(self.handle()) };
    }

    fn close(self) {
        let _ = unsafe { CloseHandle(self.handle()) };
    }
}

/// Tells the audio thread when the default output device changes.
#[implement(IMMNotificationClient)]
struct DeviceWatcher {
    changed: Event,
}

impl IMMNotificationClient_Impl for DeviceWatcher_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
        Ok(())
    }

    fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> Result<()> {
        if flow == eRender && role == eConsole {
            self.changed.set();
        }

        Ok(())
    }

    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

/// An open stream on the default output device.
struct Stream {
    client: IAudioClient,
    render: IAudioRenderClient,
    frames: u32,
}

impl Stream {
    /// Opens the default device: 48 kHz stereo float, which Windows converts
    /// to the device's own format if it has to.
    fn open(enumerator: &IMMDeviceEnumerator, ready: Event) -> Result<Stream> {
        unsafe {
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            let format = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
                nChannels: 2,
                nSamplesPerSec: SAMPLE_RATE,
                nAvgBytesPerSec: SAMPLE_RATE * 8,
                nBlockAlign: 8,
                wBitsPerSample: 32,
                cbSize: 0,
            };
            let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
            client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, BUFFER_DURATION, 0, &format, None)?;
            client.SetEventHandle(ready.handle())?;
            let frames = client.GetBufferSize()?;
            let render: IAudioRenderClient = client.GetService()?;
            client.Start()?;
            Ok(Stream { client, render, frames })
        }
    }

    /// Fills the free part of the device buffer from the ring, interleaving
    /// its left and right channels through `buf`.
    fn fill(&self, ring: &AudioRing, buf: &mut (Vec<f32>, Vec<f32>)) -> Result<()> {
        unsafe {
            let free = self.frames - self.client.GetCurrentPadding()?;

            if free == 0 {
                return Ok(());
            }

            let (left, right) = buf;
            left.resize(free as usize, 0.0);
            right.resize(free as usize, 0.0);
            let real = ring.pull(left, right);
            let data = self.render.GetBuffer(free)?.cast::<f32>();
            let stereo = std::slice::from_raw_parts_mut(data, free as usize * 2);

            for (frame, (&l, &r)) in stereo.chunks_exact_mut(2).zip(left.iter().zip(right.iter())) {
                frame[0] = l;
                frame[1] = r;
            }

            let flags = if real == 0 { AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 } else { 0 };
            self.render.ReleaseBuffer(free, flags)
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = unsafe { self.client.Stop() };
    }
}

/// The audio thread: opens the stream, reports whether it could, then keeps
/// the device fed until `stop` is set.
fn run(ring: Arc<AudioRing>, stop: Event, opened: mpsc::Sender<bool>) {
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let mut task = 0;
    let mmcss = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task) };

    let (Ok(ready), Ok(changed)) = (Event::new(), Event::new()) else {
        let _ = opened.send(false);
        return;
    };
    let enumerator: Result<IMMDeviceEnumerator> = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) };
    let Ok(enumerator) = enumerator else {
        let _ = opened.send(false);
        ready.close();
        changed.close();
        return;
    };
    let watcher: IMMNotificationClient = DeviceWatcher { changed }.into();
    let _ = unsafe { enumerator.RegisterEndpointNotificationCallback(&watcher) };

    let mut stream = Stream::open(&enumerator, ready).ok();
    let _ = opened.send(stream.is_some());

    if stream.is_some() {
        let mut buf = (Vec::new(), Vec::new());

        loop {
            let handles = [stop.handle(), changed.handle(), ready.handle()];
            let timeout = if stream.is_some() { 2000 } else { RETRY_MS };
            let woken = unsafe { WaitForMultipleObjects(&handles, false, timeout) };

            if woken == WAIT_OBJECT_0 {
                break;
            }

            let reopen = woken.0 == WAIT_OBJECT_0.0 + 1
                || stream.as_ref().is_none_or(|s| s.fill(&ring, &mut buf).is_err());

            if reopen {
                // The default device changed or went away: the new default, or
                // none until there is one.
                drop(stream.take());
                stream = Stream::open(&enumerator, ready).ok();
                log!("audio output {}", if stream.is_some() { "reopened" } else { "unavailable" });
            }
        }
    }

    drop(stream);
    let _ = unsafe { enumerator.UnregisterEndpointNotificationCallback(&watcher) };
    ready.close();
    changed.close();

    if let Ok(mmcss) = mmcss {
        let _ = unsafe { AvRevertMmThreadCharacteristics(mmcss) };
    }

    unsafe { CoUninitialize() };
}

/// The output, while it plays. Dropping it stops the thread.
pub struct AudioOutput {
    stop: Event,
    thread: Option<JoinHandle<()>>,
}

impl AudioOutput {
    /// Starts playing `ring`, or None if no output device can be opened; the
    /// machine then runs on the host clock alone (spec §6.6).
    pub fn start(ring: Arc<AudioRing>) -> Option<AudioOutput> {
        let stop = Event::new().ok()?;
        let (opened, result) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || run(ring, stop, opened))
            .ok()?;
        let output = AudioOutput { stop, thread: Some(thread) };

        if result.recv_timeout(Duration::from_secs(5)) == Ok(true) {
            Some(output)
        } else {
            log!("no audio output");
            None
        }
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        self.stop.set();

        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }

        self.stop.close();
    }
}
