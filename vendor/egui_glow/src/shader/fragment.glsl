#ifdef GL_ES
    precision mediump float;
#endif

uniform sampler2D u_sampler;
// Subpixel text (Purgeon): 0 = normal drawing. For the font texture, 1 = darken pass
// (multiplies what is behind by 1 - coverage, per channel), 2 = add pass (adds colour x coverage).
uniform int u_pass;
// 1 / font texture width, to step one texel left and right
uniform float u_texel;

#if NEW_SHADER_INTERFACE
    in vec4 v_rgba_in_gamma;
    in vec2 v_tc;
    out vec4 f_color;
    // a dirty hack applied to support webGL2
    #define gl_FragColor f_color
    #define texture2D texture
#else
    varying vec4 v_rgba_in_gamma;
    varying vec2 v_tc;
#endif

// -----------------------------------------------
// Adapted from
// https://www.shadertoy.com/view/llVGzG
// Originally presented in:
// Jimenez 2014, "Next Generation Post-Processing in Call of Duty"
//
// A good overview can be found in
// https://blog.demofox.org/2022/01/01/interleaved-gradient-noise-a-different-kind-of-low-discrepancy-sequence/
// via https://github.com/rerun-io/rerun/
float interleaved_gradient_noise(vec2 n) {
    float f = 0.06711056 * n.x + 0.00583715 * n.y;
    return fract(52.9829189 * fract(f));
}

vec3 dither_interleaved(vec3 rgb, float levels) {
    float noise = interleaved_gradient_noise(gl_FragCoord.xy);
    // scale down the noise slightly to ensure flat colors aren't getting dithered
    noise = (noise - 0.5) * 0.95;
    return rgb + noise / (levels - 1.0);
}

// 0-1 sRGB gamma  from  0-1 linear
vec3 srgb_gamma_from_linear(vec3 rgb) {
    bvec3 cutoff = lessThan(rgb, vec3(0.0031308));
    vec3 lower = rgb * vec3(12.92);
    vec3 higher = vec3(1.055) * pow(rgb, vec3(1.0 / 2.4)) - vec3(0.055);
    return mix(higher, lower, vec3(cutoff));
}

// 0-1 sRGBA gamma  from  0-1 linear
vec4 srgba_gamma_from_linear(vec4 rgba) {
    return vec4(srgb_gamma_from_linear(rgba.rgb), rgba.a);
}

void main() {
    if (u_pass != 0) {
        // glyphs drawn for subpixels have u moved right by 2 and three texels per pixel
        vec2 tc = v_tc;
        vec3 cov;
        if (tc.x >= 2.0) {
            tc.x -= 2.0;
            cov = vec3(texture2D(u_sampler, tc - vec2(u_texel, 0.0)).a,
                       texture2D(u_sampler, tc).a,
                       texture2D(u_sampler, tc + vec2(u_texel, 0.0)).a);
        } else {
            cov = vec3(texture2D(u_sampler, tc).a);
        }
        if (u_pass == 1) {
            gl_FragColor = vec4(cov * v_rgba_in_gamma.a, 0.0);
        } else {
            vec3 rgb = v_rgba_in_gamma.rgb * cov;
#if DITHERING
            rgb = dither_interleaved(rgb, 256.);
#endif
            gl_FragColor = vec4(rgb, v_rgba_in_gamma.a * (cov.r + cov.g + cov.b) / 3.0);
        }
        return;
    }
#if SRGB_TEXTURES
    vec4 texture_in_gamma = srgba_gamma_from_linear(texture2D(u_sampler, v_tc));
#else
    vec4 texture_in_gamma = texture2D(u_sampler, v_tc);
#endif

    // We multiply the colors in gamma space, because that's the only way to get text to look right.
    vec4 frag_color_gamma = v_rgba_in_gamma * texture_in_gamma;

    // Dither the float color down to eight bits to reduce banding.
    // This step is optional for egui backends.
#if DITHERING
    frag_color_gamma.rgb = dither_interleaved(frag_color_gamma.rgb, 256.);
#endif
    gl_FragColor = frag_color_gamma;
}
