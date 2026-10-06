use std::{cell::Cell, rc::Rc, sync::{Arc, mpsc}, time::{Duration, Instant}};

use cpal::{BufferSize, FromSample, I24, SizedSample, Stream, StreamConfig, traits::{DeviceTrait, StreamTrait}};
use midir::MidiInputConnection;
use raw_window_handle::HasWindowHandle;
use winit::{application::ApplicationHandler, dpi::{LogicalSize, PhysicalSize, Size}, event::WindowEvent, event_loop::{ActiveEventLoop, ControlFlow, EventLoop}, window::{Window, WindowAttributes, WindowId}};

use super::{parameters::StandaloneParameterEventMap, audio::AudioState, config::{AudioOutputConfig, MidiInputConfig}, host::StandaloneHost, midi, plugin::StandalonePlugin};

use crate::{Editor, Event, Host, HostInfo, ProcessMode, Processor, ProcessorConfig, formats::PluginFormat};

struct StandaloneRunner<P: StandalonePlugin> {
    _plugin: Rc<P>, // Just keep the plugin alive (alongside the editor)
    editor: P::Editor,
    title: &'static str,
    window: Option<Window>,
    requested_window_size: Rc<Cell<Option<(f64, f64)>>>,
    last_frame: Instant,
    audio_stream: Stream,
    midi_connections: Vec<MidiInputConnection<()>>,
}

impl<P: StandalonePlugin> StandaloneRunner<P> {
    fn new(
        plugin: Rc<P>,
        editor: P::Editor,
        requested_window_size: Rc<Cell<Option<(f64, f64)>>>,
        audio_stream: Stream,
        midi_connections: Vec<MidiInputConnection<()>>,
    ) -> Self {
        Self {
            _plugin: plugin,
            editor,
            title: P::NAME,
            window: None,
            requested_window_size,
            last_frame: Instant::now(),
            audio_stream,
            midi_connections,
        }
    }

    fn editor_size(window: &Window, size: PhysicalSize<u32>) -> (f64, f64) {
        // Editor sizes are physical pixels, except on macOS, where the plugin view applies the
        // system's DPI scale and sizes are logical points.
        if cfg!(target_os = "macos") {
            size.to_logical::<f64>(window.scale_factor()).into()
        } else {
            size.cast::<f64>().into()
        }
    }

    fn request_window_size(window: &Window, editor_size: (f64, f64)) {
        let new_size: Size = if cfg!(target_os = "macos") {
            LogicalSize::<f64>::from(editor_size).into()
        } else {
            PhysicalSize::<u32>::from(editor_size).into()
        };
        let new_physical_size = new_size.to_physical::<u32>(window.scale_factor());
        if window.request_inner_size(new_size).is_some_and(|applied_size| applied_size != new_physical_size) {
            tracing::warn!("Failed to apply new standalone editor window size");
        }
    }
}

impl<P: StandalonePlugin> Drop for StandaloneRunner<P> {
    fn drop(&mut self) {
        let _ = self.audio_stream.pause();
        self.midi_connections.clear();
    }
}

impl<P: StandalonePlugin> ApplicationHandler for StandaloneRunner<P> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Create new window
        let (default_width, default_height) = P::Editor::DEFAULT_SIZE;
        let attrs = WindowAttributes::default()
            .with_title(self.title)
            .with_inner_size(LogicalSize::new(default_width, default_height))
            .with_resizable(self.editor.can_resize());

        let window = match event_loop.create_window(attrs) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("failed to create window: {e}");
                event_loop.exit();
                return;
            }
        };

        // Set initial scale and get initial size
        if !cfg!(target_os = "macos") {
            // On macOS the system's DPI scale already is applied in the plugin view
            self.editor.set_scale(window.scale_factor());
        }
        // Resize window, in case editor uses a custom scaling factor
        Self::request_window_size(&window, self.editor.window_size());

        // Attach editor to the window
        let handle = window
            .window_handle()
            .expect("Failed to get window's platform handle")
            .as_raw();
        self.editor.open(handle);
        self.window = Some(window);
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.editor.close();
        self.window = None;
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.editor.close();
                event_loop.exit();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, mut inner_size_writer } => {
                if !cfg!(target_os = "macos") {
                    // see `resumed`impl
                    self.editor.set_scale(scale_factor);
                    // apply new window size, if needed
                    let new_size = PhysicalSize::<u32>::from(self.editor.window_size());
                    if inner_size_writer.request_inner_size(new_size).is_err() {
                        tracing::warn!("Failed to apply new standalone editor window size");
                    }
                }
            }
            WindowEvent::Resized(_) => {
                let Some(window) = &self.window else {
                    return;
                };
                let inner_size = window.inner_size();
                // Minimized windows report a zero size on Windows: ignore those.
                if inner_size.width == 0 || inner_size.height == 0 {
                    return;
                }
                let size = Self::editor_size(window, inner_size);
                let Some(supported_size) = self.editor.check_window_size(size) else {
                    return;
                };
                // Requesting a size un-maximizes the window on Windows, so let the editor fit
                // itself into maximized and fullscreen windows instead.
                let needs_correction = (supported_size.0 - size.0).abs() > 1.0
                    || (supported_size.1 - size.1).abs() > 1.0;
                let new_size = if needs_correction && !window.is_maximized() && window.fullscreen().is_none() {
                    Self::request_window_size(window, supported_size);
                    supported_size
                } else {
                    size
                };
                self.editor.set_window_size(new_size.0, new_size.1);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            return;
        }

        let now = Instant::now();
        let frame_interval = Duration::from_millis(16);

        if now >= self.last_frame + frame_interval {
            self.editor.on_frame();
            self.last_frame = now;
        }

        // Apply pending requests from `StandaloneHost::resize_view`
        if let Some(size) = self.requested_window_size.take()
            && let Some(window) = &self.window
        {
            Self::request_window_size(window, size);
        }

        event_loop.set_control_flow(ControlFlow::WaitUntil(self.last_frame + frame_interval));
    }
}

/// Runs the given plugin as a standalone application using the default audio output device and all available
/// MIDI input ports (if the plugin has `HAS_NOTE_INPUT` set).
///
/// # Example
///
/// ```rust,ignore
/// use plinth_plugin::standalone::run_standalone;
///
/// fn main() {
///     run_standalone::<MyPlugin>();
/// }
/// ```
pub fn run_standalone<P: StandalonePlugin + 'static>() {
    run_standalone_with_config::<P>(AudioOutputConfig::default(), MidiInputConfig::default());
}

/// Runs the given plugin as a standalone application with explicit audio and MIDI configuration.
///
/// # Example
///
/// ```rust,ignore
/// fn main() {
///     // Enumerate available audio devices for the default driver
///     let audio_devices = AudioOutputConfig::available_devices(AudioDeviceDriver::Default)
///         .expect("Failed to enumerate audio devices");
///     // Enumerate available MIDI input ports
///     let midi_ports = MidiInputConfig::available_ports()
///         .expect("Failed to enumerate MIDI ports");
///
///     let audio_config = AudioOutputConfig {
///         driver: AudioDeviceDriver::Default,
///         device_id: audio_devices.first().map(|(id, _)| id.clone()),
///         sample_rate: Some(48000),
///         buffer_size: Some(512),
///     };
///     let midi_config = MidiInputConfig {
///         port_names: Some(vec!["My MIDI Keyboard".to_string()]),
///     };
///
///     run_standalone_with_config::<MyPlugin>(audio_config, midi_config);
/// }
/// ```
pub fn run_standalone_with_config<P: StandalonePlugin + 'static>(
    audio_config: AudioOutputConfig,
    midi_config: MidiInputConfig,
) {
    let host_info = HostInfo {
        name: Some("Standalone".to_string()),
        format: PluginFormat::Standalone,
    };

    let plugin = Rc::new(P::new(host_info));

    // Parameter event map (shared between host and audio thread)
    let parameter_event_map =
        plugin.with_parameters(|params| Arc::new(StandaloneParameterEventMap::new(params)));

    // Channels
    let (midi_sender, midi_receiver) = mpsc::channel::<Event>();

    // Open MIDI connections if plugin accepts note inputs
    let midi_connections = if P::HAS_NOTE_INPUT {
        midi::connect_inputs(&midi_config, midi_sender, P::MIDI_CAPABILITIES)
    } else {
        vec![]
    };

    // Open audio device
    let mut audio_host = audio_config
        .open_host()
        .expect("Failed to open audio driver");
    let audio_device = audio_config
        .open_device(&mut audio_host)
        .expect("Failed to open audio device");
    let audio_stream_config = audio_config
        .select_stream_config(&audio_device)
        .expect("Failed to select audio stream config");

    // Create processor
    // NB: CPAL unfortunately has no getter for the real applied block size, so we need to ensure that the processor never gets called with more frames
    let processor_config = ProcessorConfig {
        sample_rate: audio_stream_config.sample_rate() as f64,
        min_block_size: 1,
        max_block_size: P::MAX_BLOCK_SIZE,
        process_mode: ProcessMode::Realtime,
    };
    let mut processor = plugin.create_processor(processor_config);
    processor.reset();

    // Create audio state
    let audio_state = AudioState::<P>::new(
        processor,
        audio_stream_config.channels() as usize,
        midi_receiver,
        parameter_event_map.clone(),
    );

    // Create and start the CPAL stream
    fn run_audio_stream<P, T>(
        device: &cpal::Device,
        config: cpal::StreamConfig,
        mut audio_state: AudioState<P>,
    ) -> Result<Stream, Box<dyn std::error::Error>>
    where
        P: StandalonePlugin + 'static,
        T: SizedSample + FromSample<f32>,
        f32: FromSample<T>,
    {
        let channels = config.channels as usize;

        let stream = device.build_output_stream(
            &config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                audio_state.process(data, channels);
            },
            |err| {
                tracing::error!("An audio stream error occurred: {err}");
            },
            None,
        )?;
        stream.play()?;

        Ok(stream)
    }

    let stream_format = audio_stream_config.sample_format();
    let stream_config = StreamConfig {
        channels: audio_stream_config.channels(),
        sample_rate: audio_stream_config.sample_rate(),
        buffer_size: audio_config
            .buffer_size
            .map(BufferSize::Fixed)
            .unwrap_or(BufferSize::Default),
    };

    let audio_stream = match stream_format {
        cpal::SampleFormat::I8 => run_audio_stream::<P, i8>(&audio_device, stream_config, audio_state),
        cpal::SampleFormat::I16 => run_audio_stream::<P, i16>(&audio_device, stream_config, audio_state),
        cpal::SampleFormat::I24 => run_audio_stream::<P, I24>(&audio_device, stream_config, audio_state),
        cpal::SampleFormat::I32 => run_audio_stream::<P, i32>(&audio_device, stream_config, audio_state),
        cpal::SampleFormat::I64 => run_audio_stream::<P, i64>(&audio_device, stream_config, audio_state),
        cpal::SampleFormat::U8 => run_audio_stream::<P, u8>(&audio_device, stream_config, audio_state),
        cpal::SampleFormat::U16 => run_audio_stream::<P, u16>(&audio_device, stream_config, audio_state),
        cpal::SampleFormat::U32 => run_audio_stream::<P, u32>(&audio_device, stream_config, audio_state),
        cpal::SampleFormat::U64 => run_audio_stream::<P, u64>(&audio_device, stream_config, audio_state),
        cpal::SampleFormat::F32 => run_audio_stream::<P, f32>(&audio_device, stream_config, audio_state),
        cpal::SampleFormat::F64 => run_audio_stream::<P, f64>(&audio_device, stream_config, audio_state),
        sample_format => panic!("Unsupported sample format '{sample_format}'"),
    }
    .expect("Failed to build audio output stream");

    // Create host and editor
    let requested_window_size = Rc::new(Cell::new(None));
    let host = Rc::new(StandaloneHost::new(plugin.clone(), parameter_event_map, requested_window_size.clone()));
    let editor = plugin.create_editor(host as Rc<dyn Host>);

    // Create winit event loop
    let event_loop = EventLoop::new().expect("Failed to create event loop");

    // Run winit event loop (blocks until window is closed)
    let mut runner = StandaloneRunner::new(plugin, editor, requested_window_size, audio_stream, midi_connections);

    event_loop.run_app(&mut runner).expect("Event loop error");
}
