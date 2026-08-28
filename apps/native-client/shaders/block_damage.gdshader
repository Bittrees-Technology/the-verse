// SPDX-License-Identifier: AGPL-3.0-or-later
shader_type spatial;
render_mode blend_mix, depth_draw_never, cull_back, diffuse_burley, specular_schlick_ggx;

uniform float severity : hint_range(0.0, 1.0) = 0.35;
uniform float pattern_seed : hint_range(0.0, 1.0) = 0.0;

varying vec3 local_position;
varying vec3 local_normal;

float hash21(vec2 point) {
	return fract(sin(dot(point, vec2(127.1, 311.7))) * 43758.5453);
}

void vertex() {
	local_position = VERTEX;
	local_normal = NORMAL;
}

void fragment() {
	vec3 axis = abs(local_normal);
	float face_seed = dot(local_normal, vec3(0.173, 0.379, 0.617));
	float seed = fract(pattern_seed + face_seed);
	vec2 face_position;
	if (axis.x > axis.y && axis.x > axis.z) {
		face_position = local_position.zy;
	} else if (axis.y > axis.z) {
		face_position = local_position.xz;
	} else {
		face_position = local_position.xy;
	}
	vec2 uv = face_position + vec2(0.5);
	float bend = 0.50 + 0.16 * sin((uv.x + seed) * 9.0);
	float split = 0.48 - 0.24 * (uv.x - 0.5) + 0.07 * sin(uv.x * 21.0);
	float primary = 1.0 - smoothstep(0.008, 0.026, abs(uv.y - bend));
	float branch = 1.0 - smoothstep(0.006, 0.020, abs(uv.y - split));
	branch *= smoothstep(0.18, 0.35, uv.x) * (1.0 - smoothstep(0.68, 0.88, uv.x));
	float pitting = smoothstep(0.84, 0.96, hash21(floor(uv * 18.0) + seed));
	float crack = max(primary, branch) * smoothstep(0.12, 0.32, severity);
	float scorch = severity * (0.045 + 0.075 * hash21(uv * 7.0 + seed));
	float opacity = clamp(scorch + crack * (0.42 + severity * 0.42) + pitting * severity * 0.20, 0.0, 0.92);
	ALBEDO = mix(vec3(0.10, 0.045, 0.018), vec3(0.008), clamp(crack + pitting, 0.0, 1.0));
	ROUGHNESS = 0.98;
	METALLIC = 0.08;
	ALPHA = opacity;
}
