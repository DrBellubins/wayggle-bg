use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use super::AppConfiguration;
use super::render_thread::{run_render_thread, RenderCommand, RenderEvent};

use wayland_client::protocol::{wl_callback, wl_compositor, wl_display, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};
use wayland_client::globals::GlobalListContents;
use wayland_client::protocol::wl_registry;
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::State;

pub struct AppState {
    pub start_time: Instant,
    pub conf: AppConfiguration,
    pub closed: bool,

    pub configured_once: bool,

    pub render_tx: Option<Sender<RenderCommand>>,
    pub render_rx: Option<Receiver<RenderEvent>>,
    pub render_in_flight: bool,
    pub last_render_request_at: Instant,

    // Wayland objects we need on this thread
    pub display: wl_display::WlDisplay,
    pub compositor: wl_compositor::WlCompositor,
    pub surface: wl_surface::WlSurface,
    pub layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
}

impl AppState {
    pub fn new(
        display: wl_display::WlDisplay,
        compositor: wl_compositor::WlCompositor,
        surface: wl_surface::WlSurface,
        layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        conf: AppConfiguration,
    ) -> Self {
        let now = Instant::now();
        Self {
            start_time: now,
            conf,
            closed: false,
            configured_once: false,

            render_tx: None,
            render_rx: None,
            render_in_flight: false,
            last_render_request_at: now,

            display,
            compositor,
            surface,
            layer_surface,
        }
    }

    pub fn is_running(&self) -> bool {
        !self.closed
    }

    fn drain_render_events(&mut self) {
        if let Some(rx) = self.render_rx.as_ref() {
            while let Ok(RenderEvent::FrameComplete) = rx.try_recv() {
                self.render_in_flight = false;
            }
        }
    }

    fn ensure_render_thread_started(&mut self, width: u32, height: u32) {
        if self.render_tx.is_some() {
            return;
        }

        let (cmd_tx, cmd_rx) = mpsc::channel::<RenderCommand>();
        let (evt_tx, evt_rx) = mpsc::channel::<RenderEvent>();

        let display = self.display.clone();
        let surface = self.surface.clone();
        let conf = self.conf.clone();

        std::thread::spawn(move || {
            run_render_thread(display, surface, width, height, conf, cmd_rx, evt_tx);
        });

        self.render_tx = Some(cmd_tx);
        self.render_rx = Some(evt_rx);
        self.render_in_flight = false;
    }

    fn try_request_render(&mut self) {
        self.drain_render_events();

        let Some(tx) = self.render_tx.as_ref() else { return; };
        if self.render_in_flight {
            return; // drop frame request
        }

        // Reduce GPU contention (cursor smoothness)
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

// Layer-surface events
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
                layer_surface.ack_configure(serial);

                state.ensure_render_thread_started(width, height);

                if let Some(tx) = state.render_tx.as_ref() {
                    let _ = tx.send(RenderCommand::Resize { width, height });
                }

                // kick frame loop once
                if !state.configured_once {
                    state.configured_once = true;

                    state.try_request_render();

                    let _cb = state.surface.frame(qh, ());
                    state.surface.commit();
                }
            }
            zwlr_layer_surface_v1::Event::Closed => {
                state.closed = true;
                if let Some(tx) = state.render_tx.as_ref() {
                    let _ = tx.send(RenderCommand::Exit);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<AppState>,
    ) {
        // registry events are handled internally by registry_queue_init
    }
}

impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &zwlr_layer_shell_v1::ZwlrLayerShellV1,
        _event: zwlr_layer_shell_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<AppState>,
    ) {
        // No events to handle for zwlr_layer_shell_v1
    }
}

// Frame callback events
impl Dispatch<wl_callback::WlCallback, ()> for AppState {
    fn event(
        state: &mut Self,
        _callback: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.try_request_render();

            let _cb = state.surface.frame(qh, ());
            state.surface.commit();
        }
    }
}

// These interfaces don’t emit events we care about, but we must implement Dispatch.
impl Dispatch<wl_surface::WlSurface, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_surface::WlSurface,
        _event: wl_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wl_compositor::WlCompositor, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_compositor::WlCompositor,
        _event: wl_compositor::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}