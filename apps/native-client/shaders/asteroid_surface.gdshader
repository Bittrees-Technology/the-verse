// SPDX-License-Identifier: AGPL-3.0-or-later
shader_type spatial;
render_mode cull_disabled, diffuse_burley, specular_schlick_ggx;

uniform sampler2D albedo_texture : source_color, repeat_enable, filter_linear_mipmap_anisotropic;

uniform float relief_strength : hint_range(0.0, 1.0) = 0.7;

varying vec3 surface_position;
varying vec3 surface_normal;

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

float fractal_noise(vec3 point, float footprint) {
	float result = 0.0;
	float amplitude = 0.55;
	for (int octave = 0; octave < 4; octave++) {
		// Suppress unresolved octaves instead of letting them sparkle at distance.
		float resolved = 1.0 - smoothstep(0.35, 1.0, footprint);
		result += mix(0.5, value_noise(point), resolved) * amplitude;
		point = point * 2.07 + vec3(9.2, 3.7, 5.1);
		amplitude *= 0.48;
		footprint *= 2.07;
	}
	return result;
}

void vertex() {
	// Chunk vertices already share asteroid-local coordinates. Keep the material
	// fixed to that frame and leave the mined surface silhouette unchanged.
	surface_position = VERTEX;
	surface_normal = NORMAL;
}

// Screen derivatives reconstruct a tangent-plane height gradient without
// requiring UV tangents on the marching-tetrahedra mesh. VERTEX and NORMAL in
// fragment() are both view-space; mirrored/back faces retain a bounded slope.
vec3 relief_normal(vec3 position, vec3 normal, float height) {
	vec3 dpdx = dFdx(position);
	vec3 dpdy = dFdy(position);
	vec3 tangent_x = cross(dpdy, normal);
	vec3 tangent_y = cross(normal, dpdx);
	float determinant = dot(dpdx, tangent_x);
	vec3 gradient = (dFdx(height) * tangent_x + dFdy(height) * tangent_y);
	gradient *= sign(determinant) / max(abs(determinant), 0.00000001);
	gradient /= max(1.0, length(gradient) / 0.35);
	return normalize(normal - relief_strength * gradient);
}

vec3 triplanar_albedo(vec3 point, vec3 normal) {
	vec3 blend = pow(abs(normal), vec3(5.0));
	blend /= max(blend.x + blend.y + blend.z, 0.0001);
	float scale = 0.34;
	vec3 x_sample = texture(albedo_texture, point.yz * scale).rgb;
	vec3 y_sample = texture(albedo_texture, point.xz * scale).rgb;
	vec3 z_sample = texture(albedo_texture, point.xy * scale).rgb;
	return x_sample * blend.x + y_sample * blend.y + z_sample * blend.z;
}

void fragment() {
	float footprint = max(length(dFdx(surface_position)), length(dFdy(surface_position)));
	float broad = fractal_noise(surface_position * 0.42, footprint * 0.42);
	float grain_visibility = 1.0 - smoothstep(0.35, 1.0, footprint * 5.8);
	float grain = mix(0.5, value_noise(surface_position * 5.8), grain_visibility);
	float ore_mask = smoothstep(0.12, 0.34, COLOR.r - COLOR.b);
	vec3 scanned_rock = triplanar_albedo(surface_position, normalize(surface_normal));
	float scanned_luma = dot(scanned_rock, vec3(0.2126, 0.7152, 0.0722));
	// Geological scans are deliberately restrained so generated source-map
	// color cannot turn a close surface into saturated cyan bands.
	scanned_rock = mix(vec3(scanned_luma), scanned_rock, 0.14) * vec3(0.80, 0.75, 0.68);
	vec3 rock = mix(scanned_rock, COLOR.rgb, 0.26) * mix(0.72, 1.12, broad);
	rock *= mix(0.88, 1.08, grain);
	vec3 ferrite = scanned_rock * vec3(0.74, 0.46, 0.25) * mix(0.72, 1.20, broad);
	ALBEDO = mix(rock, ferrite, ore_mask);
	ROUGHNESS = mix(0.98, 0.72, ore_mask) - grain * 0.045;
	METALLIC = ore_mask * 0.26;
	float height = broad * 0.32 + scanned_luma * 0.015 + grain * 0.002;
	NORMAL = relief_normal(VERTEX, normalize(NORMAL), height);
}
