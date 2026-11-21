// Render a BGRA or RGBA CEF texture into Slint-compatible RGBA8.
// Reading a BGRA8 texture returns normalized logical RGBA components.
// The viewport and scissor are set to match the image's physical pixels.
@group(0) @binding(0) var source_image: texture_2d<f32>;

@vertex
fn vertex_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let vertices = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0)
    );
    return vec4<f32>(vertices[index], 0.0, 1.0);
}

@fragment
fn fragment_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(source_image, vec2<i32>(position.xy), 0);
}
