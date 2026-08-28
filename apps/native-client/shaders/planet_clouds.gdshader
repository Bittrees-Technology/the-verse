shader_type spatial;
render_mode blend_mix, diffuse_burley, specular_schlick_ggx, cull_disabled, depth_prepass_alpha;

float hash21(vec2 point) {
	point = fract(point * vec2(123.34, 456.21));
	point += dot(point, point + 45.32);
	return fract(point.x * point.y);
}

float value_noise(vec2 point) {
	vec2 cell = floor(point);
	vec2 local = fract(point);
	local = local * local * (3.0 - 2.0 * local);
	return mix(
		mix(hash21(cell), hash21(cell + vec2(1.0, 0.0)), local.x),
		mix(hash21(cell + vec2(0.0, 1.0)), hash21(cell + vec2(1.0)), local.x),
		local.y
	);
}

float cloud_noise(vec2 point) {
	float result = 0.0;
	float amplitude = 0.55;
	for (int octave = 0; octave < 5; octave++) {
		result += value_noise(point) * amplitude;
		point = point * 2.04 + vec2(7.3, 2.9);
		amplitude *= 0.48;
	}
	return result;
}

void fragment() {
	float broad = cloud_noise(UV * vec2(7.0, 3.5));
	float detail = cloud_noise(UV * vec2(31.0, 15.5) + vec2(11.0, 3.0));
	float cloud = smoothstep(0.61, 0.79, broad + detail * 0.18);
	ALBEDO = mix(vec3(0.56, 0.65, 0.68), vec3(0.90, 0.94, 0.95), detail);
	ROUGHNESS = 0.92;
	METALLIC = 0.0;
	ALPHA = cloud * 0.38;
}
