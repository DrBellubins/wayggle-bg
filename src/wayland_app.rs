mod app_state;
mod graphics;
mod render_thread;

use std::sync::Arc;

use wayland_client::{
    globals::registry_queue_init,
    protocol::wl_compositor,
    Connection, QueueHandle,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

#[derive(Clone)]
pub struct AppConfiguration {
    pub vertex_shader: String,
    pub fragment_shader: String,
    pub get_cursor: Option<Arc<dyn Fn() -> (f32, f32) + Send + Sync>>,
}

pub fn run(conf: AppConfiguration) {
    let conn = Connection::connect_to_env().unwrap();

    // Robust global init (don’t rely on manual wl_registry Dispatch)
    let (globals, mut event_queue) = registry_queue_init::<app_state::AppState>(&conn).unwrap();
    let qh: QueueHandle<app_state::AppState> = event_queue.handle();

    // Bind required globals
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=6, ()).unwrap();
    let layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1 = globals.bind(&qh, 1..=5, ()).unwrap();

    let surface = compositor.create_surface(&qh, ());
    let layer_surface = layer_shell.get_layer_surface(
        &surface,
        None,
        zwlr_layer_shell_v1::Layer::Bottom,
        "egl_background".to_string(),
        &qh,
        (),
    );

    layer_surface.set_exclusive_zone(-1);
    layer_surface.set_anchor(
        zwlr_layer_surface_v1::Anchor::Top
            | zwlr_layer_surface_v1::Anchor::Bottom
            | zwlr_layer_surface_v1::Anchor::Left
            | zwlr_layer_surface_v1::Anchor::Right,
    );
    layer_surface.set_size(0, 0);

    // Build state (now we already have all required objects)
    let display = conn.display();
    let mut state = app_state::AppState::new(display, compositor, surface, layer_surface, conf);

    // This commit is what triggers the first configure
    state.surface.commit();
    tracing::info!("Initial commit done. Waiting for configure event...");

    while state.is_running() {
        event_queue.blocking_dispatch(&mut state).unwrap();
    }
}