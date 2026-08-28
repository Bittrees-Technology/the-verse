# SPDX-License-Identifier: AGPL-3.0-or-later
extends SceneTree

const PLANET_TEXTURE: Texture2D = preload(
	"res://assets/materials/khepri_prime_earthlike_albedo_v2.png"
)
const PLANET_SHADER: Shader = preload("res://shaders/planet_surface.gdshader")
const ATMOSPHERE_SHADER: Shader = preload("res://shaders/planet_atmosphere.gdshader")
const CLOUD_SHADER: Shader = preload("res://shaders/planet_clouds.gdshader")

var failures: Array[String] = []


func _initialize() -> void:
	call_deferred("_run")


func _run() -> void:
	_check(
		PLANET_TEXTURE.get_width() == 1774 and PLANET_TEXTURE.get_height() == 887,
		"Earthlike albedo stays at its exact 2:1 source dimensions",
	)
	var albedo_image := PLANET_TEXTURE.get_image()
	_check(albedo_image != null and not albedo_image.is_empty(), "Earthlike albedo decodes")
	if albedo_image != null and not albedo_image.is_empty():
		_check(albedo_image.has_mipmaps(), "Earthlike albedo retains distance-stable mipmaps")
		var seam_error := 0.0
		for y in albedo_image.get_height():
			var left := albedo_image.get_pixel(0, y)
			var right := albedo_image.get_pixel(albedo_image.get_width() - 1, y)
			seam_error += (
				absf(left.r - right.r) + absf(left.g - right.g) + absf(left.b - right.b)
			) / 3.0
		seam_error /= float(albedo_image.get_height())
		_check(seam_error < 0.06, "Earthlike albedo horizontal seam stays visually bounded")
	_check(_has_uniform(PLANET_SHADER, "planet_albedo"), "surface exposes planet albedo")
	_check(_has_uniform(PLANET_SHADER, "outpost_direction"), "surface exposes outpost biome")
	_check(_has_uniform(ATMOSPHERE_SHADER, "horizon_color"), "atmosphere exposes horizon color")
	_check(_has_uniform(ATMOSPHERE_SHADER, "outer_color"), "atmosphere exposes outer color")
	for uniform_name in ["cloud_scale", "coverage", "opacity", "noise_offset"]:
		_check(_has_uniform(CLOUD_SHADER, uniform_name), "cloud exposes %s" % uniform_name)

	var sphere := SphereMesh.new()
	sphere.radius = 1.0
	sphere.height = 2.0
	sphere.radial_segments = 64
	sphere.rings = 32
	var surface_material := ShaderMaterial.new()
	surface_material.shader = PLANET_SHADER
	surface_material.set_shader_parameter("planet_albedo", PLANET_TEXTURE)
	surface_material.set_shader_parameter("outpost_direction", Vector3.UP)
	sphere.material = surface_material
	var instance := MeshInstance3D.new()
	instance.mesh = sphere
	root.add_child(instance)
	await process_frame
	await process_frame

	if not failures.is_empty():
		for failure in failures:
			printerr("VERSE_PLANET_VISUAL_FAILED %s" % failure)
		quit(1)
		return
	print(
		"VERSE_PLANET_VISUAL_OK albedo=1774x887 surface=biome-aware "
		+ "clouds=spherical-layered atmosphere=compressed-limb"
	)
	quit(0)


func _has_uniform(shader: Shader, uniform_name: String) -> bool:
	for uniform_data in shader.get_shader_uniform_list():
		if String(uniform_data.get("name", "")) == uniform_name:
			return true
	return false


func _check(condition: bool, label: String) -> void:
	if not condition:
		failures.append(label)
