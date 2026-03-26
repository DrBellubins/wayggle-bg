pub fn shader_toy_adaptor(fragment_shader: String) -> String {
    return format!(
        "
        #version 300 es
        precision highp float;
        
        uniform float u_time;
        uniform vec2 u_resolution;
        uniform vec2 u_mouse;

        in vec2 v_position;

        out vec4 fragColor;

        float iTime;
        vec3 iResolution;
        vec2 iMouse;

        {fragment_shader}

        void main() {{
            iResolution=vec3(u_resolution,u_resolution.x/u_resolution.y);
            iTime=u_time;
            iMouse=u_mouse;
            vec2 w=(v_position * 0.5 + 0.5) * u_resolution.xy;
            mainImage(fragColor,w);
        }}
        "
    );
}
