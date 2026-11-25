#version 450
// A decoded picture sampled through its Y'CbCr conversion (the sampler does the matrix and
// range): target pixel (x, y) shows buffer pixel (left + x, top + y).
layout(set = 0, binding = 0) uniform sampler2D picture;
layout(push_constant) uniform Crop {
    vec2 origin;
    vec2 inv_size;
} crop;
layout(location = 0) out vec4 color;

void main() {
    vec2 at = (crop.origin + gl_FragCoord.xy) * crop.inv_size;
    color = vec4(texture(picture, at).rgb, 1.0);
}
