shader_type spatial;
render_mode unshaded, blend_add, cull_front, depth_draw_never;

uniform vec3 atmosphere_color : source_color = vec3(0.18, 0.52, 0.78);
uniform float atmosphere_strength = 0.46;

void fragment() {
	float view_facing = abs(dot(normalize(NORMAL), normalize(VIEW)));
	float limb = pow(1.0 - view_facing, 2.7);
	float outer_fade = smoothstep(0.02, 0.72, limb);
	ALBEDO = atmosphere_color * mix(0.42, 1.0, limb);
	ALPHA = outer_fade * atmosphere_strength;
}
