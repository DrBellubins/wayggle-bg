pub fn shader_toy_adaptor(fragment_shader: String) -> String {
    return format!(
        "
        #version 300 es
        precision highp float;
        
        uniform float u_time;          // legacy (kept for compatibility)
        uniform vec2  u_time_hi_lo;    // NEW: (hi, lo) parts of time in seconds
        uniform vec2 u_resolution;
        uniform vec2 u_mouse;

        in vec2 v_position;

        out vec4 fragColor;

        float iTime;
        vec3 iResolution;
        vec iMouse;

        {fragment_shader}

        void main() {{
            iResolution=vec3(u_resolution,u_resolution.x/u_resolution.y);

            // Reconstruct higher-precision time
            // (still a float in GLSL, but keeps fractional motion stable for very long uptimes)
            iTime = u_time_hi_lo.x + u_time_hi_lo.y;

            iMouse=u_mouse;
            vec2 w=(v_position * 0.5 + 0.5) * u_resolution.xy;
            mainImage(fragColor,w);
        }}
        "
    );
}
