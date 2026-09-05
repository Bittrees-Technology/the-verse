# SPDX-License-Identifier: AGPL-3.0-or-later
extends SceneTree

# Run with a real Compatibility renderer, not --headless's dummy renderer.
const CLIENT_SCRIPT: Script = preload("res://src/main.gd")
const ROCK_SHADER: Shader = preload("res://shaders/asteroid_surface.gdshader")
const ROCK_TEXTURE: Texture2D = preload("res://assets/materials/verse_asteroid_regolith_albedo.png")
var viewport: SubViewport
var stage: Node3D
var material: ShaderMaterial


func _initialize() -> void:
	call_deferred("_run")


func _capture() -> Image:
	for frame in range(4):
		await process_frame
	await RenderingServer.frame_post_draw
	return viewport.get_texture().get_image()


func _difference(a: Image, b: Image) -> float:
	var total := 0.0
	for y in range(a.get_height()):
		for x in range(a.get_width()):
			var ca := a.get_pixel(x, y)
			var cb := b.get_pixel(x, y)
			total += absf(ca.r - cb.r) + absf(ca.g - cb.g) + absf(ca.b - cb.b)
	return total / float(a.get_width() * a.get_height() * 3)


func _run() -> void:
	if DisplayServer.get_name() == "headless":
		printerr("VERSE_ASTEROID_RENDER_FAILED requires a real rendering device")
		quit(1)
		return
	viewport = SubViewport.new()
	viewport.size = Vector2i(640, 480)
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	viewport.msaa_3d = Viewport.MSAA_4X
	root.add_child(viewport)
	stage = Node3D.new()
	viewport.add_child(stage)
	var environment := WorldEnvironment.new()
	environment.environment = Environment.new()
	environment.environment.background_mode = Environment.BG_COLOR
	environment.environment.background_color = Color(0.018, 0.025, 0.04)
	environment.environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.environment.ambient_light_color = Color(0.3, 0.38, 0.5)
	environment.environment.ambient_light_energy = 0.35
	stage.add_child(environment)
	var light := DirectionalLight3D.new()
	light.rotation_degrees = Vector3(-25, -45, 0)
	light.light_energy = 1.6
	stage.add_child(light)
	var camera := Camera3D.new()
	stage.add_child(camera)
	camera.position = Vector3(0, 0.5, 4.6)
	camera.look_at(Vector3.ZERO)
	camera.current = true
	material = ShaderMaterial.new()
	material.shader = ROCK_SHADER
	material.set_shader_parameter("albedo_texture", ROCK_TEXTURE)
	# Exercise the actual mined-voxel mesher across eight chunk boundaries.
	var rock := Node3D.new()
	stage.add_child(rock)
	var builder := Node3D.new()
	builder.set_script(CLIENT_SCRIPT)
	builder.set("asteroid_root", rock)
	builder.set("rock_material", material)
	var voxels := {}
	for x in range(-3, 4):
		for y in range(-3, 4):
			for z in range(-3, 4):
				var coordinate := Vector3i(x, y, z)
				if Vector3(coordinate).length() > 2.7:
					continue
				voxels[builder.call("_coord_key", coordinate)] = {
					"material": "ferrite_ore" if x > 0 else "rock",
				}
	builder.set("voxel_lookup", voxels)
	for x in range(-1, 1):
		for y in range(-1, 1):
			for z in range(-1, 1):
				builder.call("_rebuild_voxel_chunk", Vector3i(x, y, z))
	builder.free()
	camera.position = Vector3(0.4, 0.7, 6.5)
	camera.look_at(Vector3.ZERO)
	var rendered := await _capture()
	if rendered == null or rendered.is_empty():
		printerr("VERSE_ASTEROID_RENDER_FAILED empty framebuffer")
		quit(1)
		return
	# Translate camera, geometry, and lighting together: the image must not swim.
	stage.position = Vector3(32, -16, 24)
	var translated := await _capture()
	var drift := _difference(rendered, translated)
	stage.position = Vector3.ZERO
	material.set_shader_parameter("relief_strength", 0.0)
	var flat := await _capture()
	var relief_delta := _difference(rendered, flat)
	material.set_shader_parameter("relief_strength", 0.7)
	var output := "user://asteroid-render-after.png"
	for argument in OS.get_cmdline_user_args():
		if argument.begins_with("--output="):
			output = argument.trim_prefix("--output=")
	if rendered.save_png(output) != OK:
		printerr("VERSE_ASTEROID_RENDER_FAILED cannot save preview")
		quit(1)
		return
	# Optional same-camera baseline capture for visual review, never required by tests.
	for argument in OS.get_cmdline_user_args():
		if argument.begins_with("--baseline="):
			var previous := Shader.new()
			previous.code = FileAccess.get_file_as_string(argument.trim_prefix("--baseline="))
			material.shader = previous
			var baseline := await _capture()
			baseline.save_png(output.get_basename() + "-before.png")
	if drift > 0.002 or relief_delta < 0.0002:
		printerr("VERSE_ASTEROID_RENDER_FAILED drift=%f relief=%f" % [drift, relief_delta])
		quit(1)
		return
	print("VERSE_ASTEROID_RENDER_OK drift=%f relief=%f preview=%s" % [drift, relief_delta, output])
	quit(0)
