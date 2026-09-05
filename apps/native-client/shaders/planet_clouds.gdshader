shader_type spatial;
render_mode blend_mix, diffuse_burley, specular_schlick_ggx, cull_disabled, depth_prepass_alpha;

uniform float cloud_scale = 2.35;
uniform float coverage = 0.63;
uniform float opacity = 0.50;
uniform vec3 noise_offset = vec3(5.4, 11.7, 2.9);

varying vec3 cloud_direction;
varying vec3 weather_samples;

float hash31(vec3 point) {
	point = fract(point * 0.1031);
	point += dot(point, point.yzx + 33.33);
	return fract((point.x + point.y) * point.z);
}

float value_noise(vec3 point) {
	vec3 cell = floor(point);
	vec3 local = fract(point);
	local = local * local * (3.0 - 2.0 * local);
	float lower_front = mix(hash31(cell), hash31(cell + vec3(1.0, 0.0, 0.0)), local.x);
	float upper_front = mix(hash31(cell + vec3(0.0, 1.0, 0.0)), hash31(cell + vec3(1.0, 1.0, 0.0)), local.x);
	float lower_back = mix(hash31(cell + vec3(0.0, 0.0, 1.0)), hash31(cell + vec3(1.0, 0.0, 1.0)), local.x);
	float upper_back = mix(hash31(cell + vec3(0.0, 1.0, 1.0)), hash31(cell + vec3(1.0)), local.x);
	return mix(mix(lower_front, upper_front, local.y), mix(lower_back, upper_back, local.y), local.z);
}

float cloud_noise(vec3 point) {
	float result = 0.0;
	float amplitude = 0.55;
	for (int octave = 0; octave < 5; octave++) {
		result += value_noise(point) * amplitude;
		point = point * 2.04 + vec3(7.3, 2.9, 5.1);
		amplitude *= 0.48;
	}
	return result;
}

void vertex() {
	cloud_direction = normalize(VERTEX);
	// Sample the smooth weather field on the dense cloud shell once per vertex.
	// Interpolation avoids repeating fifteen noise octaves for every lit pixel.
	weather_samples = vec3(
		cloud_noise(cloud_direction * cloud_scale + noise_offset),
		cloud_noise(cloud_direction * cloud_scale * 2.8 + noise_offset.zxy * 1.7),
		cloud_noise(cloud_direction * cloud_scale * 11.0 + noise_offset.yzx * 2.3)
	);
}

void fragment() {
	vec3 radial = normalize(cloud_direction);
	float broad = weather_samples.x;
	float fronts = weather_samples.y;
	float detail = weather_samples.z;
	float latitude_breakup = smoothstep(0.72, 0.98, abs(radial.y));
	float weather = broad * 0.77 + fronts * 0.23 - latitude_breakup * 0.055;
	float cloud = smoothstep(coverage, coverage + 0.17, weather);
	cloud *= mix(0.62, 1.08, smoothstep(0.34, 0.80, detail));
	float cloud_light = smoothstep(0.28, 0.86, detail);
	ALBEDO = mix(vec3(0.46, 0.53, 0.57), vec3(0.94, 0.97, 0.98), cloud_light);
	ROUGHNESS = mix(0.86, 0.98, cloud_light);
	METALLIC = 0.0;
	SPECULAR = 0.18;
	ALPHA = clamp(cloud * opacity, 0.0, 0.70);
}
