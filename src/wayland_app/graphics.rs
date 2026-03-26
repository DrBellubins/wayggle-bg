use glow::HasContext;
use khronos_egl as egl;

use std::sync::Arc;

use wayland_client::protocol::{wl_display, wl_surface};
use wayland_client::Proxy;
use wayland_egl as wegl;

use super::AppConfiguration;

/// Struct to manage EGL/OpenGL ES initialization and rendering using `glow`
pub struct Graphics {
    // Fix E0107: egl::Instance is generic in khronos-egl v6
    egl_instance: egl::Instance<egl::Static>,
    egl_display: egl::Display,
    egl_context: egl::Context,
    egl_surface: egl::Surface,
    wl_egl_surface: wegl::WlEglSurface,
    width: i32,
    height: i32,

    gl: glow::Context,

    shader_program: glow::Program,
    vbo: glow::Buffer,

    time_uniform_location: Option<glow::UniformLocation>,
    resolution_uniform_location: Option<glow::UniformLocation>,
    cursor_location_and_inspector:
        Option<(glow::UniformLocation, Arc<dyn Fn() -> (f32, f32) + Send + Sync>)>,
}

impl Graphics {
    pub fn render(&mut self, elapsed: f32) {
        self.egl_instance
            .make_current(
                self.egl_display,
                Some(self.egl_surface),
                Some(self.egl_surface),
                Some(self.egl_context),
            )
            .inspect_err(|e| tracing::error!("Failed to make EGL context current: {}", e))
            .unwrap();

        unsafe {
            self.gl.viewport(0, 0, self.width, self.height);
            self.gl.use_program(Some(self.shader_program));

            if let Some(location) = self.time_uniform_location.as_ref() {
                self.gl.uniform_1_f32(Some(location), elapsed);
            }
            if let Some(location) = self.resolution_uniform_location.as_ref() {
                self.gl
                    .uniform_2_f32(Some(location), self.width as f32, self.height as f32);
            }
            if let Some((cursor_location, get_cursor)) = self.cursor_location_and_inspector.as_ref()
            {
                let (x, y) = get_cursor();
                self.gl.uniform_2_f32(Some(cursor_location), x, y);
            }

            self.gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }

        self.egl_instance
            .swap_buffers(self.egl_display, self.egl_surface)
            .inspect_err(|e| tracing::error!("Failed to swap EGL buffers: {}", e))
            .unwrap();
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width as i32;
        self.height = height as i32;
        self.wl_egl_surface
            .resize(width as i32, height as i32, 0, 0);
        unsafe {
            self.gl.viewport(0, 0, width as i32, height as i32);
        }
    }

    pub fn new(
        display: &wl_display::WlDisplay,
        surface: &wl_surface::WlSurface,
        width: u32,
        height: u32,
        conf: &AppConfiguration,
    ) -> Self {
        // Fix E0107 + typo from your attached file:
        // egl::Instance::<egl::Static>::new(egl::Static)
        let egl_instance = egl::Instance::<egl::Static>::new(egl::Static);

        let egl_display = unsafe {
            egl_instance
                .get_display(display.id().as_ptr() as egl::NativeDisplayType)
                .ok_or("Failed to get EGL display")
                .unwrap()
        };

        egl_instance.initialize(egl_display).unwrap();
        egl_instance.bind_api(egl::OPENGL_ES_API).unwrap();

        let attributes = [
            egl::RED_SIZE,
            8,
            egl::GREEN_SIZE,
            8,
            egl::BLUE_SIZE,
            8,
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT,
            egl::RENDERABLE_TYPE,
            egl::OPENGL_ES3_BIT,
            egl::NONE,
        ];
        let config = egl_instance
            .choose_first_config(egl_display, &attributes)
            .unwrap()
            .expect("Failed to find suitable EGL config");

        let context_attributes = [egl::CONTEXT_CLIENT_VERSION, 3, egl::NONE];
        let egl_context = egl_instance
            .create_context(egl_display, config, None, &context_attributes)
            .unwrap();

        let wl_egl_surface =
            wegl::WlEglSurface::new(surface.id(), width as i32, height as i32).unwrap();

        let egl_surface = unsafe {
            egl_instance
                .create_window_surface(
                    egl_display,
                    config,
                    wl_egl_surface.ptr() as egl::NativeWindowType,
                    None,
                )
                .unwrap()
        };

        egl_instance
            .make_current(
                egl_display,
                Some(egl_surface),
                Some(egl_surface),
                Some(egl_context),
            )
            .unwrap();

        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                egl_instance.get_proc_address(s).unwrap() as *const _
            })
        };

        let shader_program = unsafe {
            let program = gl.create_program().expect("Cannot create program");

            let vs = gl.create_shader(glow::VERTEX_SHADER).unwrap();
            gl.shader_source(vs, &conf.vertex_shader);
            gl.compile_shader(vs);
            if !gl.get_shader_compile_status(vs) {
                tracing::error!(
                    "Vertex shader compilation failed: {}",
                    gl.get_shader_info_log(vs)
                );
                std::process::exit(1);
            }
            gl.attach_shader(program, vs);

            let fs = gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
            gl.shader_source(fs, &conf.fragment_shader);
            gl.compile_shader(fs);
            if !gl.get_shader_compile_status(fs) {
                tracing::error!(
                    "Fragment shader compilation failed: {}",
                    gl.get_shader_info_log(fs)
                );
                std::process::exit(1);
            }
            gl.attach_shader(program, fs);

            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                panic!("{}", gl.get_program_info_log(program));
            }

            gl.detach_shader(program, fs);
            gl.delete_shader(fs);
            gl.detach_shader(program, vs);
            gl.delete_shader(vs);

            gl.use_program(Some(program));
            program
        };

        let time_uniform_location = unsafe { gl.get_uniform_location(shader_program, "u_time") };
        let resolution_uniform_location =
            unsafe { gl.get_uniform_location(shader_program, "u_resolution") };

        let cursor_location_and_inspector = conf.get_cursor.as_ref().and_then(|get_cursor| {
            let cursor_location = unsafe { gl.get_uniform_location(shader_program, "u_mouse") };
            cursor_location.map(|loc| (loc, get_cursor.clone()))
        });

        let vbo = unsafe {
            let vertices: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
            let vertices_u8: &[u8] = core::slice::from_raw_parts(
                vertices.as_ptr() as *const u8,
                vertices.len() * std::mem::size_of::<f32>(),
            );

            let vbo = gl.create_buffer().unwrap();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, vertices_u8, glow::STATIC_DRAW);

            let pos_attr_loc = gl
                .get_attrib_location(shader_program, "a_position")
                .expect("Failed to get attribute location for a_position");
            gl.enable_vertex_attrib_array(pos_attr_loc);
            gl.vertex_attrib_pointer_f32(pos_attr_loc, 2, glow::FLOAT, false, 0, 0);

            vbo
        };

        Graphics {
            egl_instance,
            egl_display,
            egl_context,
            egl_surface,
            wl_egl_surface,
            width: width as i32,
            height: height as i32,
            gl,
            shader_program,
            vbo,
            time_uniform_location,
            resolution_uniform_location,
            cursor_location_and_inspector,
        }
    }
}

impl Drop for Graphics {
    fn drop(&mut self) {
        unsafe {
            let _ = self
                .egl_instance
                .make_current(self.egl_display, None, None, None);

            self.gl.delete_program(self.shader_program);
            self.gl.delete_buffer(self.vbo);

            let _ = self
                .egl_instance
                .destroy_surface(self.egl_display, self.egl_surface);

            let _ = self
                .egl_instance
                .destroy_context(self.egl_display, self.egl_context);

            let _ = self.egl_instance.terminate(self.egl_display);
        }
    }
}