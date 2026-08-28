shader_type spatial;
render_mode diffuse_burley, specular_schlick_ggx, cull_back, depth_draw_opaque;

uniform sampler2D planet_albedo : source_color, repeat_enable, filter_linear_mipmap_anisotropic;

varying vec3 terrain_position;

float hash31(vec3 point) {
	point = fract(point * 0.1031);
	point += dot(point, point.yzx + 33.33);
	return fract((point.x + point.y) * point.z);
}

float value_noise(vec3 point) {
	vec3 cell = floor(point);
	vec3 fraction = fract(point);
	fraction = fraction * fraction * (3.0 - 2.0 * fraction);
	float lower_front = mix(hash31(cell), hash31(cell + vec3(1.0, 0.0, 0.0)), fraction.x);
	float upper_front = mix(hash31(cell + vec3(0.0, 1.0, 0.0)), hash31(cell + vec3(1.0, 1.0, 0.0)), fraction.x);
	float lower_back = mix(hash31(cell + vec3(0.0, 0.0, 1.0)), hash31(cell + vec3(1.0, 0.0, 1.0)), fraction.x);
	float upper_back = mix(hash31(cell + vec3(0.0, 1.0, 1.0)), hash31(cell + vec3(1.0)), fraction.x);
	return mix(mix(lower_front, upper_front, fraction.y), mix(lower_back, upper_back, fraction.y), fraction.z);
}

float terrain_noise(vec3 point) {
	float result = 0.0;
	float amplitude = 0.56;
	for (int octave = 0; octave < 5; octave++) {
		result += value_noise(point) * amplitude;
		point = point * 2.03 + vec3(7.1, 3.7, 5.9);
		amplitude *= 0.48;
	}
	return result;
}

void vertex() {
	vec3 radial = normalize(VERTEX);
	float broad = terrain_noise(radial * 5.2);
	float ridges = abs(terrain_noise(radial * 19.0) - 0.5) * 2.0;
	float displacement = (broad - 0.50) * 18.0 + ridges * 3.2;
	VERTEX += radial * displacement;
	terrain_position = VERTEX;
}

void fragment() {
	vec3 radial = normalize(terrain_position);
	float detail = terrain_noise(radial * 34.0);
	float mineral = terrain_noise(radial * 76.0 + vec3(8.0, 2.0, 11.0));
	vec3 mapped_surface = texture(planet_albedo, UV).rgb;
	vec3 mineral_tint = mix(vec3(0.78, 0.88, 0.90), vec3(0.72, 0.48, 0.31), mineral);
	vec3 finished_surface = mapped_surface * mineral_tint * mix(0.92, 1.20, detail);
	ALBEDO = finished_surface * 1.18;
	EMISSION = finished_surface * 0.055;
	ROUGHNESS = mix(0.96, 0.72, mineral);
	METALLIC = smoothstep(0.90, 0.99, mineral) * 0.10;
}
