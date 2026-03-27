use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use super::AppConfiguration;
use super::render_thread::run_render_thread;
use super::render_thread::{RenderCommand, RenderEvent};

use wayland_client::protocol::wl_display;
use wayland_client::{
    protocol::{wl_callback, wl_compositor, wl_registry, wl_surface},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

pub struct AppState {
    pub render_tx: Option<Sender<RenderCommand>>,
    pub render_rx: Option<Receiver<RenderEvent>>,
    pub render_in_flight: bool,

    pub start_time: Instant,
    pub conf: AppConfiguration,
    pub closed: bool,

    // Track first configure so we can "kick" the frame loop once.
    pub configured_once: bool,

    // Optional: cap render request rate (reduces GPU contention -> smoother cursor)
    pub last_render_request_at: Instant,

    // Wayland objects
    pub display: wl_display::WlDisplay,
    pub compositor: Option<(wl_compositor::WlCompositor, u32)>,
    pub layer_shell: Option<(zwlr_layer_shell_v1::ZwlrLayerShellV1, u32)>,
    pub surface: Option<wl_surface::WlSurface>,
    pub layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
}

impl AppState {
    pub fn new(display: wl_display::WlDisplay, conf: AppConfiguration) -> Self {
        let now = Instant::now();
        AppState {
            render_tx: None,
            render_rx: None,
            render_in_flight: false,
            start_time: now,
            conf,
            closed: false,
            configured_once: false,
            last_render_request_at: now,
            display,
            compositor: None,
            layer_shell: None,
            surface: None,
            layer_surface: None,
        }
    }

    pub fn is_running(&self) -> bool {
        !self.closed
    }

    fn drain_render_events(&mut self) {
        if let Some(rx) = self.render_rx.as_ref() {
            while let Ok(_evt) = rx.try_recv() {
                self.render_in_flight = false;
            }
        }
    }

    fn ensure_render_thread_started(&mut self, surface: wl_surface::WlSurface, width: u32, height: u32) {
        if self.render_tx.is_some() {
            return;
        }

        let (cmd_tx, cmd_rx) = mpsc::channel::<RenderCommand>();
        let (evt_tx, evt_rx) = mpsc::channel::<RenderEvent>();

        let display = self.display.clone();
        let wl_surface = surface; // cloned by caller
        let conf = self.conf.clone();

        std::thread::spawn(move || {
            run_render_thread(display, wl_surface, width, height, conf, cmd_rx, evt_tx);
        });

        self.render_tx = Some(cmd_tx);
        self.render_rx = Some(evt_rx);
        self.render_in_flight = false;
    }

    fn try_request_render(&mut self) {
        self.drain_render_events();

        let Some(tx) = self.render_tx.as_ref() else { return; };

        // one frame in flight
        if self.render_in_flight {
            return;
        }

        // FPS cap (tune this; 30 helps cursor smoothness a lot if shader is heavy)
        const FPS_CAP: Option<f32> = Some(30.0);
        if let Some(fps) = FPS_CAP {
            let min_dt = Duration::from_secs_f32(1.0 / fps);
            if self.last_render_request_at.elapsed() < min_dt {
                return;
            }
        }
        self.last_render_request_at = Instant::now();

        let elapsed = self.start_time.elapsed().as_secs_f32();
        let _ = tx.send(RenderCommand::Render { elapsed });
        self.render_in_flight = true;
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for AppState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global { name, interface, version } => match interface.as_str() {
                "wl_compositor" => {
                    tracing::info!("Compositor found: {} (version {})", name, version);
                    state.compositor = Some((registry.bind(name, version, qh, ()), name));
                }
                "zwlr_layer_shell_v1" => {
                    tracing::info!("LayerShell found: {} (version {})", name, version);
                    state.layer_shell = Some((registry.bind(name, version, qh, ()), name));
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                if let Some((_, compositor_name)) = &state.compositor {
                    if *compositor_name == name {
                        tracing::warn!("Compositor {} removed", name);
                        state.compositor = None;
                    }
                }
                if let Some((_, layer_shell_name)) = &state.layer_shell {
                    if *layer_shell_name == name {
                        tracing::warn!("LayerShell {} removed", name);
                        state.layer_shell = None;
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _layer_shell: &zwlr_layer_shell_v1::ZwlrLayerShellV1,
        _event: zwlr_layer_shell_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // no-op
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for AppState {
    fn event(
        state: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                // Configure can happen several times; keep it debug to avoid spam.
                tracing::debug!(
                    "Layer surface configured: serial={}, width={}, height={}",
                    serial,
                    width,
                    height
                );

                layer_surface.ack_configure(serial);

                let Some(wl_surface) = state.surface.as_ref().cloned() else { return; };

                state.ensure_render_thread_started(wl_surface.clone(), width, height);

                if let Some(tx) = state.render_tx.as_ref() {
                    let _ = tx.send(RenderCommand::Resize { width, height });
                }

                // Kick the frame callback loop once after first configure.
                // This is required to start getting wl_callback::Done events.
                if !state.configured_once {
                    state.configured_once = true;

                    // Request an initial render immediately so you see something.
                    state.try_request_render();

                    let _cb = wl_surface.frame(qh, ());
                    wl_surface.commit();
                }
            }
            zwlr_layer_surface_v1::Event::Closed => {
                tracing::info!("Layer surface closed");
                state.closed = true;
                if let Some(tx) = state.render_tx.as_ref() {
                    let _ = tx.send(RenderCommand::Exit);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for AppState {
    fn event(
        state: &mut Self,
        _callback: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_callback::Event::Done { .. } => {
                // Throttled render request (drops frames when busy)
                state.try_request_render();

                // Keep frame loop alive
                if let Some(surface) = state.surface.as_ref() {
                    let _cb = surface.frame(qh, ());
                    surface.commit();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for AppState {
    fn event(
        _state: &mut Self,
        _surface: &wl_surface::WlSurface,
        _event: wl_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // no-op
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for AppState {
    fn event(
        _state: &mut Self,
        _compositor: &wl_compositor::WlCompositor,
        _event: wl_compositor::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // no-op
    }
}