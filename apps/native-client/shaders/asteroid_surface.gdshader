// SPDX-License-Identifier: AGPL-3.0-or-later
shader_type spatial;
render_mode cull_disabled, diffuse_burley, specular_schlick_ggx;

uniform sampler2D albedo_texture : source_color, repeat_enable, filter_linear_mipmap_anisotropic;

varying vec3 world_position;
varying vec3 world_normal;

float hash31(vec3 point) {
	point = fract(point * 0.1031);
	point += dot(point, point.yzx + 33.33);
	return fract((point.x + point.y) * point.z);
}

float value_noise(vec3 point) {
	vec3 cell = floor(point);
	vec3 local = fract(point);
	local = local * local * (3.0 - 2.0 * local);
	float c000 = hash31(cell + vec3(0.0, 0.0, 0.0));
	float c100 = hash31(cell + vec3(1.0, 0.0, 0.0));
	float c010 = hash31(cell + vec3(0.0, 1.0, 0.0));
	float c110 = hash31(cell + vec3(1.0, 1.0, 0.0));
	float c001 = hash31(cell + vec3(0.0, 0.0, 1.0));
	float c101 = hash31(cell + vec3(1.0, 0.0, 1.0));
	float c011 = hash31(cell + vec3(0.0, 1.0, 1.0));
	float c111 = hash31(cell + vec3(1.0, 1.0, 1.0));
	float lower = mix(mix(c000, c100, local.x), mix(c010, c110, local.x), local.y);
	float upper = mix(mix(c001, c101, local.x), mix(c011, c111, local.x), local.y);
	return mix(lower, upper, local.z);
}

float fractal_noise(vec3 point) {
	float result = 0.0;
	float amplitude = 0.55;
	for (int octave = 0; octave < 4; octave++) {
		result += value_noise(point) * amplitude;
		point = point * 2.07 + vec3(9.2, 3.7, 5.1);
		amplitude *= 0.48;
	}
	return result;
}

void vertex() {
	float relief = (fractal_noise(VERTEX * 0.62) - 0.52) * 0.18;
	VERTEX += NORMAL * relief;
	world_position = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
	world_normal = normalize(MODEL_NORMAL_MATRIX * NORMAL);
}

vec3 triplanar_albedo(vec3 point, vec3 normal) {
	vec3 blend = pow(abs(normal), vec3(5.0));
	blend /= max(blend.x + blend.y + blend.z, 0.0001);
	float scale = 0.19;
	vec3 x_sample = texture(albedo_texture, point.yz * scale).rgb;
	vec3 y_sample = texture(albedo_texture, point.xz * scale).rgb;
	vec3 z_sample = texture(albedo_texture, point.xy * scale).rgb;
	return x_sample * blend.x + y_sample * blend.y + z_sample * blend.z;
}

void fragment() {
	float broad = fractal_noise(world_position * 0.42);
	float grain = value_noise(world_position * 5.8);
	float ore_mask = smoothstep(0.12, 0.34, COLOR.r - COLOR.b);
	vec3 scanned_rock = triplanar_albedo(world_position, world_normal);
	vec3 rock = mix(scanned_rock, COLOR.rgb, 0.26) * mix(0.72, 1.12, broad);
	rock *= mix(0.88, 1.08, grain);
	vec3 ferrite = scanned_rock * vec3(0.74, 0.46, 0.25) * mix(0.72, 1.20, broad);
	ALBEDO = mix(rock, ferrite, ore_mask);
	ROUGHNESS = mix(0.98, 0.72, ore_mask) - grain * 0.045;
	METALLIC = ore_mask * 0.26;
}
