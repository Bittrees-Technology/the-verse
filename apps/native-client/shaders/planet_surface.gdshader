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
	vec3 mapped_surface = texture(planet_albedo, UV).rgb;
	float mapped_luma = dot(mapped_surface, vec3(0.2126, 0.7152, 0.0722));
	float ocean_blue = mapped_surface.b - max(mapped_surface.r, mapped_surface.g * 0.82);
	float water = smoothstep(0.035, 0.13, ocean_blue);
	water *= 1.0 - smoothstep(0.68, 0.86, mapped_luma);

	float macro_relief = terrain_noise(radial * 17.0 + vec3(9.0, 1.5, 6.0));
	float surface_detail = terrain_noise(radial * 94.0 + vec3(3.0, 7.0, 2.0));
	float outpost_alignment = dot(radial, normalize(outpost_direction));
	float outpost_region = smoothstep(0.9915, 0.9988, outpost_alignment);
	float outpost_clearing = smoothstep(0.9988, 0.99975, outpost_alignment);

	// The development outpost starts on the authoritative spherical surface.
	// Give that small area a temperate visual biome without inventing relief
	// that would disagree with collision.
	water *= 1.0 - outpost_region;
	vec3 temperate_ground = mix(
		vec3(0.085, 0.19, 0.065),
		vec3(0.24, 0.31, 0.12),
		surface_detail
	);
	mapped_surface = mix(mapped_surface, temperate_ground, outpost_region * 0.88);
	mapped_surface = mix(mapped_surface, vec3(0.22, 0.24, 0.19), outpost_clearing * 0.32);

	float land = 1.0 - water;
	float polar_ice = smoothstep(0.76, 0.96, abs(radial.y));
	float mountain_snow = smoothstep(0.64, 0.86, mapped_luma) * land;
	float terrain_variation = mix(0.94, 1.065, surface_detail);
	terrain_variation *= mix(0.97, 1.035, macro_relief);
	vec3 deep_ocean = vec3(0.008, 0.045, 0.12);
	vec3 physical_ocean = mix(deep_ocean, mapped_surface, 0.82);
	vec3 finished_surface = mix(mapped_surface, physical_ocean, water);
	finished_surface = mix(
		finished_surface,
		vec3(0.78, 0.84, 0.86),
		polar_ice * mountain_snow * (1.0 - outpost_region)
	);

	ALBEDO = finished_surface * mix(1.0, terrain_variation, land);
	ROUGHNESS = mix(0.10, mix(0.70, 0.94, surface_detail), land);
	ROUGHNESS = mix(ROUGHNESS, 0.62, mountain_snow * land);
	METALLIC = 0.0;
	SPECULAR = mix(0.92, 0.28, land);
}
