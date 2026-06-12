//! Feed audio into wisp's `@audio`/`@audio_fft` textures.
//!
//! Captures the default input device via `nannou_audio` on its own thread (cpal
//! streams are not `Send`), handing sample chunks to the main thread where they
//! are pushed into the `WispAudio` resource. When no input device is available,
//! a wandering test tone is synthesized instead.
//!
//! Pass another shader's asset path as an argument, e.g.
//! `-- wisp/test_audio.wgsl` for the oscilloscope view.

use nannou::prelude::*;
use nannou_audio as audio;
use std::sync::mpsc;

const SYNTH_SAMPLE_RATE: f32 = 44_100.0;

fn main() {
    nannou::app(model).update(update).exit(exit).run();
}

struct Model {
    /// Wrapped in a `Mutex` only because the model must be `Sync`.
    samples_rx: std::sync::Mutex<mpsc::Receiver<(Vec<f32>, usize)>>,
    audio_thread: Option<std::thread::JoinHandle<()>>,
    exit_tx: mpsc::Sender<()>,
    /// Oscillator phase for the fallback tone, once capture is known dead.
    synth_phase: f32,
    silent_frames: u32,
}

fn model(app: &App) -> Model {
    let camera = app.new_camera().build();
    app.new_window()
        .camera(camera)
        .primary()
        .size_pixels(1024, 512)
        .view(view)
        .build();

    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| String::from("wisp/test_audio_fft.wgsl"));
    let wisp: Handle<Wisp> = app.asset_server().load(path);
    app.command_scope(move |mut commands| {
        commands.entity(camera).insert(WispHandle(wisp));
    });

    let (samples_tx, samples_rx) = mpsc::channel();
    let (exit_tx, exit_rx) = mpsc::channel();
    let audio_thread = std::thread::spawn(move || {
        let host = audio::Host::new();
        let stream = match host
            .new_input_stream(samples_tx)
            .capture(capture)
            .build()
        {
            Ok(stream) => stream,
            Err(err) => {
                eprintln!("no audio input ({err}); synthesizing a tone instead");
                return;
            }
        };
        if let Err(err) = stream.play() {
            eprintln!("failed to start the input stream ({err})");
            return;
        }
        // Keep the stream alive until exit.
        let _ = exit_rx.recv();
    });

    Model {
        samples_rx: std::sync::Mutex::new(samples_rx),
        audio_thread: Some(audio_thread),
        exit_tx,
        synth_phase: 0.0,
        silent_frames: 0,
    }
}

fn capture(samples_tx: &mut mpsc::Sender<(Vec<f32>, usize)>, buffer: &audio::Buffer) {
    let _ = samples_tx.send((buffer.to_vec(), buffer.channels()));
}

fn update(app: &App, model: &mut Model) {
    let mut chunks: Vec<(Vec<f32>, usize)> = Vec::new();
    if let Ok(samples_rx) = model.samples_rx.lock() {
        while let Ok(chunk) = samples_rx.try_recv() {
            chunks.push(chunk);
        }
    }

    // Fall back to a wandering tone when capture never delivers (no device).
    match chunks.is_empty() {
        false => model.silent_frames = 0,
        true => {
            model.silent_frames += 1;
            if model.silent_frames > 30 {
                let hz = 330.0 + 220.0 * (app.time() * 0.4).sin();
                let frames = (app.time_delta() * SYNTH_SAMPLE_RATE).clamp(64.0, 4096.0) as usize;
                let samples: Vec<f32> = (0..frames)
                    .map(|_| {
                        model.synth_phase = (model.synth_phase + hz / SYNTH_SAMPLE_RATE).fract();
                        (model.synth_phase * TAU).sin() * 0.8
                    })
                    .collect();
                chunks.push((samples, 1));
            }
        }
    }

    if chunks.is_empty() {
        return;
    }
    app.command_scope(move |mut commands| {
        commands.queue(move |world: &mut World| {
            let mut audio = world.resource_mut::<WispAudio>();
            for (samples, channels) in &chunks {
                audio.push_frames(samples, *channels);
            }
        });
    });
}

fn view(app: &App, _model: &Model) {
    let _draw = app.draw();
}

fn exit(_app: &App, mut model: Model) {
    let _ = model.exit_tx.send(());
    if let Some(handle) = model.audio_thread.take() {
        let _ = handle.join();
    }
}
