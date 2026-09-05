// SPDX-License-Identifier: AGPL-3.0-or-later
shader_type spatial;
render_mode diffuse_burley, specular_schlick_ggx;
uniform vec4 stone_color : source_color = vec4(0.35, 0.34, 0.30, 1.0);
varying vec3 stone_point;
void vertex() {
    stone_point = VERTEX;
}
void fragment() {
    float flow = stone_point.x * 8.0 + stone_point.y * 3.0 + sin(stone_point.z * 5.0) * 1.6;
    float vein = pow(abs(sin(flow + sin(stone_point.y * 11.0) * 0.35)), 48.0);
    ALBEDO = stone_color.rgb * mix(1.0, 0.94, vein);
    ROUGHNESS = 0.43;
    METALLIC = 0.04;
    SPECULAR = 0.35;
}
