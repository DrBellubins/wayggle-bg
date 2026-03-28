mod adaptors;
mod cli;
mod cursor_support;
mod wayland_app;
use clap::Parser as _;
use std::rc::Rc;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::FULL)
        .init();

    let default_vertex_shader = include_str!("../shaders/default-vert.glsl").to_string();

    let cli_configuration = cli::Cli::parse();

    let default_shaders = vec![(
        "box".to_string(),
        include_str!("../shaders/box.glsl").to_string(),
    )]
        .into_iter()
        .collect::<std::collections::HashMap<String, String>>();

    let (vertex_shader, fragment_shader) = match cli_configuration.command {
        cli::Command::ShaderToy { fragment_shader } => {
            let fragment_src = std::fs::read_to_string(&fragment_shader).unwrap_or_else(|e| {
                tracing::error!(
                    "Failed to read fragment shader file '{}': {}",
                    fragment_shader,
                    e
                );
                std::process::exit(1);
            });

            let fragment_shader = adaptors::shader_toy_adaptor(fragment_src);
            (default_vertex_shader.clone(), fragment_shader)
        }
        cli::Command::TheBookOfShaders {
            fragment_shader,
            vertex_shader,
        } => {
            let fragment_shader = fragment_shader;
            let vertex_shader = vertex_shader.unwrap_or(default_vertex_shader);
            (vertex_shader, fragment_shader)
        }
        cli::Command::Default { name } => {
            let fragment_shader = default_shaders
                .get(&name)
                .unwrap_or_else(|| {
                    tracing::error!("Shader '{}' not found in default shaders", name);
                    std::process::exit(1);
                })
                .clone();
            let fragment_shader = adaptors::shader_toy_adaptor(fragment_shader);
            (default_vertex_shader.clone(), fragment_shader)
        }
    };

    let get_cursor = match cli_configuration.cursor_support {
        cli::CursorSupportKind::Hyprland => Some(Rc::new(
            cursor_support::hyprland_get_cursor as fn() -> (f32, f32),
        )),
        cli::CursorSupportKind::Disabled => None,
    };

    let conf = wayland_app::AppConfiguration {
        vertex_shader,
        fragment_shader,
        get_cursor,
        fps: cli_configuration.fps,
        initial_time_offset: cli_configuration.initial_time_offset,
    };
    wayland_app::run(conf);
}
