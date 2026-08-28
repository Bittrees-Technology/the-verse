shader_type spatial;
render_mode diffuse_burley, specular_schlick_ggx, cull_back, depth_draw_opaque;

uniform sampler2D planet_albedo : source_color, repeat_enable, filter_linear_mipmap_anisotropic;
uniform vec3 outpost_direction = vec3(0.0, 1.0, 0.0);

varying vec3 planet_direction;

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
	// The authoritative body is a sphere. Keeping presentation on that same
	// surface prevents feet, vehicles, and the outpost from disappearing into
	// decorative terrain that has no matching collider.
	planet_direction = normalize(VERTEX);
}

void fragment() {
	vec3 radial = normalize(planet_direction);
	float continental = terrain_noise(radial * 3.4 + vec3(2.7, 8.1, 4.3));
	float regional = terrain_noise(radial * 11.0 + vec3(9.0, 1.5, 6.0));
	float detail = terrain_noise(radial * 46.0 + vec3(3.0, 7.0, 2.0));
	vec3 mapped_surface = texture(planet_albedo, UV).rgb;
	float texture_value = dot(mapped_surface, vec3(0.30, 0.55, 0.15));
	float polar_land = smoothstep(0.70, 0.96, abs(radial.y)) * 0.10;
	float outpost_patch = smoothstep(0.990, 0.9985, dot(radial, normalize(outpost_direction)));
	float land_field = continental * 0.72 + regional * 0.20 + texture_value * 0.08 + polar_land + outpost_patch * 0.14;
	float land = smoothstep(0.49, 0.535, land_field);
	float coast = smoothstep(0.49, 0.57, land_field);

	vec3 deep_ocean = vec3(0.012, 0.085, 0.19);
	vec3 shelf_ocean = vec3(0.025, 0.30, 0.43);
	vec3 ocean = mix(deep_ocean, shelf_ocean, smoothstep(0.36, 0.52, land_field));
	vec3 dry_land = vec3(0.43, 0.34, 0.16);
	vec3 grassland = vec3(0.12, 0.34, 0.12);
	vec3 forest = vec3(0.035, 0.19, 0.085);
	vec3 vegetation = mix(grassland, forest, smoothstep(0.45, 0.78, regional));
	vec3 ground = mix(dry_land, vegetation, smoothstep(0.34, 0.66, detail));
	ground = mix(ground, vec3(0.08, 0.30, 0.10), outpost_patch * 0.82);
	float mountain = smoothstep(0.61, 0.76, land_field + detail * 0.10);
	mountain *= 1.0 - outpost_patch * 0.92;
	ground = mix(ground, vec3(0.36, 0.38, 0.36), mountain);
	float snow = smoothstep(0.76, 0.94, abs(radial.y) + mountain * 0.18);
	snow *= 1.0 - outpost_patch;
	ground = mix(ground, vec3(0.88, 0.93, 0.95), snow * mountain);
	vec3 coast_color = vec3(0.63, 0.58, 0.34);
	ground = mix(coast_color, ground, coast);

	vec3 finished_surface = mix(ocean, ground, land);
	ALBEDO = finished_surface * mix(0.90, 1.10, detail);
	EMISSION = finished_surface * 0.018;
	ROUGHNESS = mix(0.24, 0.88, land);
	METALLIC = 0.0;
	SPECULAR = mix(0.78, 0.30, land);
}
