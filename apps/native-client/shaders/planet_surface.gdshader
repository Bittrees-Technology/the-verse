shader_type spatial;
render_mode diffuse_burley, specular_schlick_ggx, cull_back, depth_draw_opaque;

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
	float displacement = (broad - 0.50) * 1.45 + ridges * 0.26;
	VERTEX += radial * displacement;
	terrain_position = VERTEX;
}

void fragment() {
	vec3 radial = normalize(terrain_position);
	float broad = terrain_noise(radial * 5.2);
	float detail = terrain_noise(radial * 34.0);
	float mineral = terrain_noise(radial * 76.0 + vec3(8.0, 2.0, 11.0));
	vec3 basalt = vec3(0.075, 0.105, 0.115);
	vec3 iron_sand = vec3(0.25, 0.16, 0.105);
	vec3 lichen = vec3(0.12, 0.22, 0.18);
	vec3 ice_dust = vec3(0.34, 0.42, 0.43);
	vec3 surface = mix(basalt, iron_sand, smoothstep(0.43, 0.72, broad));
	surface = mix(surface, lichen, smoothstep(0.63, 0.86, detail) * 0.48);
	surface = mix(surface, ice_dust, smoothstep(0.82, 0.96, mineral) * 0.42);
	ALBEDO = surface * mix(0.78, 1.18, detail);
	ROUGHNESS = mix(0.96, 0.72, mineral);
	METALLIC = smoothstep(0.88, 0.98, mineral) * 0.16;
}
