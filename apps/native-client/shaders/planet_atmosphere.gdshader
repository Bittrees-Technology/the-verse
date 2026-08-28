shader_type spatial;
render_mode unshaded, blend_add, cull_front, depth_draw_never;

uniform vec3 horizon_color : source_color = vec3(0.16, 0.48, 0.82);
uniform vec3 outer_color : source_color = vec3(0.055, 0.20, 0.52);
uniform float atmosphere_strength = 0.52;

void fragment() {
	float view_facing = abs(dot(normalize(NORMAL), normalize(VIEW)));
	float limb = clamp(1.0 - view_facing, 0.0, 1.0);
	float dense_haze = smoothstep(0.20, 0.92, pow(limb, 1.8));
	float thin_air = smoothstep(0.015, 0.72, pow(limb, 3.4));
	ALBEDO = mix(outer_color, horizon_color, dense_haze);
	ALPHA = thin_air * atmosphere_strength;
}
