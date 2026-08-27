# SPDX-License-Identifier: AGPL-3.0-or-later
extends Node3D

const ARMOR_TEXTURE: Texture2D = preload("res://assets/materials/verse_armor_albedo.png")
const ASTEROID_TEXTURE: Texture2D = preload(
	"res://assets/materials/verse_asteroid_regolith_albedo.png"
)
const PLANET_TEXTURE: Texture2D = preload("res://assets/materials/khepri_prime_albedo.png")
const ASTEROID_SHADER: Shader = preload("res://shaders/asteroid_surface.gdshader")
const PLANET_SHADER: Shader = preload("res://shaders/planet_surface.gdshader")
const ATMOSPHERE_SHADER: Shader = preload("res://shaders/planet_atmosphere.gdshader")
const CLOUD_SHADER: Shader = preload("res://shaders/planet_clouds.gdshader")
const BLOCK_DAMAGE_SHADER: Shader = preload("res://shaders/block_damage.gdshader")
const PROTOCOL_VERSION := 6
const DEFAULT_SERVER := "ws://127.0.0.1:7777/ws"
const PLAYER_INVENTORY := "inventory-player-local"
const STARTER_GRID := "grid-starter"
const MOVE_SPEED := 7.0
const BOOST_MULTIPLIER := 2.1
const MOVE_ACCELERATION := 18.0
const MOVE_DAMPING := 7.5
const MOUSE_SENSITIVITY := 0.0022
const MOVE_SEND_INTERVAL := 0.10
const TARGET_RANGE := 9.0
const MINE_DURATION := 0.72
const WELD_DURATION := 0.52
const DAMAGE_DURATION := 0.46
const WALK_SPEED := 4.4
const WALK_ACCELERATION := 15.0
const JUMP_SPEED := 4.2
const ROLL_SPEED := 1.65
const PLAYER_SURFACE_CLEARANCE := 1.05
const PLANET_VISUAL_CENTER := Vector3(900.0, -2200.0, -3800.0)
const PLANET_VISUAL_RADIUS := 1200.0
const PLANET_ATMOSPHERE_RADIUS := 1242.0
const VOXEL_CHUNK_SIZE := 8
const ISO_LEVEL := 0.5
const MARCHING_CORNERS: Array[Vector3i] = [
	Vector3i(0, 0, 0), Vector3i(1, 0, 0),
	Vector3i(1, 1, 0), Vector3i(0, 1, 0),
	Vector3i(0, 0, 1), Vector3i(1, 0, 1),
	Vector3i(1, 1, 1), Vector3i(0, 1, 1),
]
const MARCHING_TETRAHEDRA: Array[Vector4i] = [
	Vector4i(0, 5, 1, 6), Vector4i(0, 1, 2, 6),
	Vector4i(0, 2, 3, 6), Vector4i(0, 3, 7, 6),
	Vector4i(0, 7, 4, 6), Vector4i(0, 4, 5, 6),
]
const DENSITY_NEIGHBORS: Array[Vector3i] = [
	Vector3i(1, 0, 0), Vector3i(-1, 0, 0),
	Vector3i(0, 1, 0), Vector3i(0, -1, 0),
	Vector3i(0, 0, 1), Vector3i(0, 0, -1),
]

var socket := WebSocketPeer.new()
var server_url := DEFAULT_SERVER
var connected := false
var handshake_sent := false
var operation_counter := 0
var move_send_elapsed := 0.0
var last_sent_position := Vector3.ZERO
var first_snapshot := true
var snapshot: Dictionary = {}
var voxel_lookup: Dictionary = {}
var voxel_coordinate_lookup: Dictionary = {}
var voxel_chunk_nodes: Dictionary = {}
var grid_lookup: Dictionary = {}
var rendered_voxel_count := -1
var selected_block_kind := "structural"
var target_voxel: Variant = null
var target_block: Dictionary = {}
var recent_message := "Starting local universe connection…"
var recent_message_color := Color(0.56, 0.87, 1.0)
var smoke_test := false
var smoke_operation := ""
var last_socket_state := -1
var closed_reported := false
var player_velocity := Vector3.ZERO
var dampeners_enabled := true
var suit_light_enabled := true
var action_charge := 0.0
var action_target_key := ""
var action_cooldown := 0.0
var tool_kick := 0.0
var elapsed_time := 0.0
var build_mode := false
var build_rotation_quarters := 0
var last_level := 1
var pending_mine_position: Variant = null
var inventory_open := false
var grid_control_active := false
var inventory_item_labels: Dictionary = {}
var inventory_capacity_labels: Dictionary = {}
var inventory_capacity_bars: Dictionary = {}
var inventory_rows: Dictionary = {}
var inventory_selected_resource := "component"
var inventory_selected_side := "suit"
var inventory_filters := {"suit": "all", "cargo": "all"}
var inventory_search_queries := {"suit": "", "cargo": ""}

var camera: Camera3D
var asteroid_root: Node3D
var grids_root: Node3D
var stars_root: Node3D
var planet_root: Node3D
var target_highlight: MeshInstance3D
var status_label: Label
var inventory_label: Label
var target_label: Label
var message_label: Label
var connection_label: Label
var selected_label: Label
var mission_label: Label
var level_label: Label
var telemetry_label: Label
var interaction_label: Label
var action_progress: ProgressBar
var hotbar_label: Label
var mode_label: Label
var tool_root: Node3D
var tool_tip: Node3D
var tool_light: OmniLight3D
var build_preview: Node3D
var action_beam: MeshInstance3D
var action_flare: MeshInstance3D
var action_sparks: GPUParticles3D
var mining_fragments: GPUParticles3D
var suit_light: SpotLight3D
var inventory_overlay: Control
var planet_cloud_layer: MeshInstance3D

var rock_material: Material
var block_materials: Dictionary = {}
var detail_materials: Dictionary = {}


func _ready() -> void:
	_parse_command_line()
	_register_inputs()
	_build_environment()
	_build_viewmodel()
	_build_interface()
	Input.mouse_mode = Input.MOUSE_MODE_CAPTURED
	_connect_to_server()


func _exit_tree() -> void:
	socket.close()


func _process(delta: float) -> void:
	elapsed_time += delta
	if planet_cloud_layer != null:
		planet_cloud_layer.rotation.y += delta * 0.0025
	_poll_socket()
	_update_movement(delta)
	_update_target()
	_update_tool_action(delta)
	_update_viewmodel(delta)
	_update_interface()


func _input(event: InputEvent) -> void:
	if event is InputEventMouseMotion and Input.mouse_mode == Input.MOUSE_MODE_CAPTURED:
		camera.rotate_object_local(Vector3.UP, -event.relative.x * MOUSE_SENSITIVITY)
		camera.rotate_object_local(Vector3.RIGHT, -event.relative.y * MOUSE_SENSITIVITY)
		camera.transform.basis = camera.transform.basis.orthonormalized()
		return

	# Inventory text entry owns keyboard input. A held grid-control key still gets
	# its release so opening the terminal cannot leave thrust latched.
	if inventory_open and event is InputEventKey:
		if event.keycode == KEY_M and not event.pressed and grid_control_active:
			_stop_target_grid()
		var focus_owner := get_viewport().gui_get_focus_owner()
		var text_entry_focused := focus_owner is LineEdit or focus_owner is TextEdit
		if _inventory_close_shortcut(event, text_entry_focused):
			_set_inventory_open(false)
		return

	if event is InputEventKey and event.keycode == KEY_M and not event.echo:
		if event.pressed and Input.mouse_mode == Input.MOUSE_MODE_CAPTURED:
			_move_target_grid()
		elif grid_control_active:
			_stop_target_grid()
		return

	if event is InputEventKey and event.pressed and not event.echo:
		match event.keycode:
			KEY_ESCAPE:
				if inventory_open:
					_set_inventory_open(false)
				else:
					Input.mouse_mode = (
						Input.MOUSE_MODE_VISIBLE
						if Input.mouse_mode == Input.MOUSE_MODE_CAPTURED
						else Input.MOUSE_MODE_CAPTURED
					)
			KEY_I:
				_set_inventory_open(not inventory_open)
			KEY_J:
				_toggle_jetpack()
			KEY_H:
				_toggle_helmet()
			KEY_1:
				selected_block_kind = "structural"
				build_mode = true
			KEY_2:
				selected_block_kind = "anchor"
				build_mode = true
			KEY_3:
				selected_block_kind = "cargo"
				build_mode = true
			KEY_4:
				selected_block_kind = "power_source"
				build_mode = true
			KEY_5:
				selected_block_kind = "damage_test"
				build_mode = true
			KEY_B:
				build_mode = not build_mode
				action_charge = 0.0
			KEY_Q:
				if not inventory_open and Input.mouse_mode == Input.MOUSE_MODE_CAPTURED:
					camera.rotate_object_local(Vector3.FORWARD, -deg_to_rad(8.0))
					camera.transform.basis = camera.transform.basis.orthonormalized()
			KEY_E:
				if not inventory_open and Input.mouse_mode == Input.MOUSE_MODE_CAPTURED:
					camera.rotate_object_local(Vector3.FORWARD, deg_to_rad(8.0))
					camera.transform.basis = camera.transform.basis.orthonormalized()
			KEY_BRACKETLEFT:
				if build_mode:
					build_rotation_quarters = posmod(build_rotation_quarters - 1, 4)
					_set_message("Block orientation %d°" % (build_rotation_quarters * 90))
			KEY_BRACKETRIGHT:
				if build_mode:
					build_rotation_quarters = posmod(build_rotation_quarters + 1, 4)
					_set_message("Block orientation %d°" % (build_rotation_quarters * 90))
			KEY_F:
				_toggle_anchor()
			KEY_X:
				_stop_target_grid()
			KEY_R:
				_refine_ore()
			KEY_T:
				_craft_component()
			KEY_V:
				_transfer_to_or_from_cargo(event.shift_pressed)
			KEY_P:
				_send({"type": "request_snapshot"})
			KEY_Z:
				dampeners_enabled = not dampeners_enabled
				_set_message(
					"Inertial dampeners %s" % ("online" if dampeners_enabled else "offline")
				)
			KEY_L:
				suit_light_enabled = not suit_light_enabled
				suit_light.visible = suit_light_enabled
				_set_message("Helmet light %s" % ("online" if suit_light_enabled else "offline"))
			KEY_F5:
				_connect_to_server()

	if event is InputEventMouseButton and event.pressed:
		if Input.mouse_mode != Input.MOUSE_MODE_CAPTURED:
			Input.mouse_mode = Input.MOUSE_MODE_CAPTURED
			return
		if event.button_index == MOUSE_BUTTON_RIGHT:
			build_mode = false
			action_charge = 0.0


func _inventory_close_shortcut(event: InputEventKey, text_entry_focused: bool) -> bool:
	if not event.pressed or event.echo:
		return false
	return event.keycode == KEY_ESCAPE or (event.keycode == KEY_I and not text_entry_focused)


func _parse_command_line() -> void:
	for argument in OS.get_cmdline_user_args():
		if argument.begins_with("--server="):
			server_url = argument.trim_prefix("--server=")
		elif argument == "--smoke-test":
			smoke_test = true


func _register_inputs() -> void:
	_add_key_action("move_forward", KEY_W)
	_add_key_action("move_backward", KEY_S)
	_add_key_action("move_left", KEY_A)
	_add_key_action("move_right", KEY_D)
	_add_key_action("move_up", KEY_SPACE)
	_add_key_action("move_down", KEY_C)
	_add_key_action("move_boost", KEY_SHIFT)
	_add_key_action("roll_left", KEY_Q)
	_add_key_action("roll_right", KEY_E)


func _add_key_action(action: StringName, keycode: Key) -> void:
	if not InputMap.has_action(action):
		InputMap.add_action(action)
	var key := InputEventKey.new()
	key.physical_keycode = keycode
	InputMap.action_add_event(action, key)


func _build_environment() -> void:
	var world_environment := WorldEnvironment.new()
	var environment := Environment.new()
	environment.background_mode = Environment.BG_COLOR
	environment.background_color = Color(0.006, 0.016, 0.032)
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.ambient_light_color = Color(0.20, 0.28, 0.42)
	environment.ambient_light_energy = 0.38
	environment.tonemap_mode = Environment.TONE_MAPPER_FILMIC
	environment.fog_enabled = true
	environment.fog_light_color = Color(0.055, 0.11, 0.16)
	environment.fog_light_energy = 0.34
	environment.fog_density = 0.00004
	environment.fog_sky_affect = 0.18
	world_environment.environment = environment
	add_child(world_environment)

	var key_light := DirectionalLight3D.new()
	key_light.rotation_degrees = Vector3(-38.0, -52.0, 0.0)
	key_light.light_color = Color(0.82, 0.89, 1.0)
	key_light.light_energy = 1.20
	key_light.shadow_enabled = true
	add_child(key_light)

	var rim_light := DirectionalLight3D.new()
	rim_light.rotation_degrees = Vector3(28.0, 132.0, 0.0)
	rim_light.light_color = Color(1.0, 0.58, 0.36)
	rim_light.light_energy = 0.32
	add_child(rim_light)

	camera = Camera3D.new()
	camera.current = true
	camera.fov = 74.0
	camera.near = 0.05
	camera.far = 12_000.0
	camera.position = Vector3(12.0, 4.5, 10.0)
	add_child(camera)
	camera.look_at(Vector3.ZERO, Vector3.UP)
	suit_light = SpotLight3D.new()
	suit_light.name = "HelmetWorkLight"
	suit_light.light_color = Color(0.86, 0.92, 1.0)
	suit_light.light_energy = 3.2
	suit_light.spot_range = 18.0
	suit_light.spot_angle = 33.0
	suit_light.spot_angle_attenuation = 1.35
	suit_light.shadow_enabled = true
	camera.add_child(suit_light)

	asteroid_root = Node3D.new()
	asteroid_root.name = "AuthoritativeAsteroid"
	add_child(asteroid_root)
	grids_root = Node3D.new()
	grids_root.name = "AuthoritativeGrids"
	add_child(grids_root)
	stars_root = Node3D.new()
	stars_root.name = "Starfield"
	add_child(stars_root)
	planet_root = Node3D.new()
	planet_root.name = "KhepriPrime"
	add_child(planet_root)

	var asteroid_material := ShaderMaterial.new()
	asteroid_material.shader = ASTEROID_SHADER
	asteroid_material.set_shader_parameter("albedo_texture", ASTEROID_TEXTURE)
	rock_material = asteroid_material
	block_materials = {
		"structural": _armored_material(Color(0.72, 0.78, 0.80), 0.72, 0.62),
		"control_core": _armored_material(Color(0.34, 0.72, 0.79), 0.46, 0.55),
		"power_source": _armored_material(Color(0.93, 0.57, 0.20), 0.40, 0.48),
		"battery": _armored_material(Color(0.86, 0.78, 0.24), 0.48, 0.38),
		"cargo": _armored_material(Color(0.36, 0.75, 0.56), 0.70, 0.28),
		"drill": _armored_material(Color(0.82, 0.38, 0.23), 0.50, 0.60),
		"anchor": _armored_material(Color(0.65, 0.37, 0.77), 0.54, 0.48),
		"damage_test": _armored_material(Color(0.92, 0.22, 0.16), 0.64, 0.32),
	}
	detail_materials = {
		"steel": _material(Color(0.50, 0.58, 0.61), 0.38, 0.82),
		"dark": _material(Color(0.025, 0.035, 0.045), 0.50, 0.68),
		"construction": _material(Color(0.20, 0.25, 0.27), 0.58, 0.84),
		"cyan": _emissive_material(Color(0.10, 0.72, 1.0), 2.8),
		"amber": _emissive_material(Color(1.0, 0.37, 0.055), 3.0),
		"green": _emissive_material(Color(0.12, 0.95, 0.53), 2.2),
		"red": _emissive_material(Color(1.0, 0.10, 0.045), 2.8),
		"glass": _glass_material(),
		"hologram": _hologram_material(),
	}
	_build_starfield()
	_build_distant_world()
	_build_orbital_dust()
	_build_target_highlight()
	_build_action_feedback()


func _material(color: Color, roughness: float, metallic: float) -> StandardMaterial3D:
	var material := StandardMaterial3D.new()
	material.albedo_color = color
	material.roughness = roughness
	material.metallic = metallic
	return material


func _armored_material(color: Color, roughness: float, metallic: float) -> StandardMaterial3D:
	var material := _material(color, roughness, metallic)
	material.albedo_texture = ARMOR_TEXTURE
	material.texture_filter = BaseMaterial3D.TEXTURE_FILTER_LINEAR_WITH_MIPMAPS_ANISOTROPIC
	material.texture_repeat = true
	material.uv1_triplanar = true
	material.uv1_world_triplanar = false
	material.uv1_scale = Vector3.ONE * 0.72
	return material


func _emissive_material(color: Color, energy: float) -> StandardMaterial3D:
	var material := _material(color * 0.28, 0.28, 0.36)
	material.emission_enabled = true
	material.emission = color
	material.emission_energy_multiplier = energy
	return material


func _hologram_material() -> StandardMaterial3D:
	var material := StandardMaterial3D.new()
	material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.albedo_color = Color(0.12, 0.78, 1.0, 0.22)
	material.emission_enabled = true
	material.emission = Color(0.08, 0.56, 1.0)
	material.emission_energy_multiplier = 1.8
	return material


func _glass_material() -> StandardMaterial3D:
	var material := StandardMaterial3D.new()
	material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	material.albedo_color = Color(0.025, 0.16, 0.22, 0.58)
	material.metallic = 0.72
	material.roughness = 0.12
	material.cull_mode = BaseMaterial3D.CULL_DISABLED
	return material


func _build_distant_world() -> void:
	var planet := MeshInstance3D.new()
	planet.name = "KhepriPrimeSurface"
	var planet_mesh := SphereMesh.new()
	planet_mesh.radius = PLANET_VISUAL_RADIUS
	planet_mesh.height = PLANET_VISUAL_RADIUS * 2.0
	planet_mesh.radial_segments = 192
	planet_mesh.rings = 96
	var surface_material := ShaderMaterial.new()
	surface_material.shader = PLANET_SHADER
	surface_material.set_shader_parameter("planet_albedo", PLANET_TEXTURE)
	planet_mesh.material = surface_material
	planet.mesh = planet_mesh
	planet.position = PLANET_VISUAL_CENTER
	planet.rotation_degrees = Vector3(0.0, -32.0, -11.0)
	planet_root.add_child(planet)

	planet_cloud_layer = MeshInstance3D.new()
	planet_cloud_layer.name = "KhepriCloudLayer"
	var cloud_mesh := SphereMesh.new()
	cloud_mesh.radius = PLANET_VISUAL_RADIUS + 12.0
	cloud_mesh.height = (PLANET_VISUAL_RADIUS + 12.0) * 2.0
	cloud_mesh.radial_segments = 160
	cloud_mesh.rings = 80
	var cloud_material := ShaderMaterial.new()
	cloud_material.shader = CLOUD_SHADER
	cloud_mesh.material = cloud_material
	planet_cloud_layer.mesh = cloud_mesh
	planet_cloud_layer.position = PLANET_VISUAL_CENTER
	planet_cloud_layer.rotation_degrees = Vector3(0.0, -20.0, -11.0)
	planet_root.add_child(planet_cloud_layer)

	var atmosphere := MeshInstance3D.new()
	atmosphere.name = "KhepriAtmosphere"
	var atmosphere_mesh := SphereMesh.new()
	atmosphere_mesh.radius = PLANET_ATMOSPHERE_RADIUS
	atmosphere_mesh.height = PLANET_ATMOSPHERE_RADIUS * 2.0
	atmosphere_mesh.radial_segments = 128
	atmosphere_mesh.rings = 64
	var atmosphere_material := ShaderMaterial.new()
	atmosphere_material.shader = ATMOSPHERE_SHADER
	atmosphere_mesh.material = atmosphere_material
	atmosphere.mesh = atmosphere_mesh
	atmosphere.position = PLANET_VISUAL_CENTER
	planet_root.add_child(atmosphere)

	var moon := MeshInstance3D.new()
	var moon_mesh := SphereMesh.new()
	moon_mesh.radius = 84.0
	moon_mesh.height = 168.0
	moon_mesh.radial_segments = 48
	moon_mesh.rings = 24
	moon_mesh.material = _material(Color(0.22, 0.18, 0.16), 0.98, 0.01)
	moon.mesh = moon_mesh
	moon.position = Vector3(2250.0, -730.0, -5100.0)
	planet_root.add_child(moon)


func _build_orbital_dust() -> void:
	var mesh := SphereMesh.new()
	mesh.radius = 0.025
	mesh.height = 0.05
	mesh.radial_segments = 4
	mesh.rings = 2
	mesh.material = _emissive_material(Color(0.40, 0.61, 0.72), 0.55)
	var dust := MultiMesh.new()
	dust.transform_format = MultiMesh.TRANSFORM_3D
	dust.mesh = mesh
	dust.instance_count = 180
	var random := RandomNumberGenerator.new()
	random.seed = 918_220
	for index in dust.instance_count:
		var point := Vector3(
			random.randf_range(-42.0, 42.0),
			random.randf_range(-24.0, 24.0),
			random.randf_range(-42.0, 42.0)
		)
		dust.set_instance_transform(index, Transform3D(Basis(), point))
	var instance := MultiMeshInstance3D.new()
	instance.multimesh = dust
	add_child(instance)


func _build_action_feedback() -> void:
	var beam_mesh := CylinderMesh.new()
	beam_mesh.top_radius = 0.018
	beam_mesh.bottom_radius = 0.035
	beam_mesh.height = 1.0
	beam_mesh.radial_segments = 8
	beam_mesh.material = _emissive_material(Color(0.08, 0.78, 1.0), 5.0)
	action_beam = MeshInstance3D.new()
	action_beam.name = "ToolBeam"
	action_beam.mesh = beam_mesh
	action_beam.visible = false
	add_child(action_beam)

	var flare_mesh := SphereMesh.new()
	flare_mesh.radius = 0.11
	flare_mesh.height = 0.22
	flare_mesh.radial_segments = 10
	flare_mesh.rings = 5
	flare_mesh.material = _emissive_material(Color(1.0, 0.29, 0.055), 7.0)
	action_flare = MeshInstance3D.new()
	action_flare.name = "ToolImpact"
	action_flare.mesh = flare_mesh
	action_flare.visible = false
	add_child(action_flare)

	var spark_process := ParticleProcessMaterial.new()
	spark_process.emission_shape = ParticleProcessMaterial.EMISSION_SHAPE_SPHERE
	spark_process.emission_sphere_radius = 0.10
	spark_process.direction = Vector3(0.0, 1.0, 0.0)
	spark_process.spread = 180.0
	spark_process.initial_velocity_min = 0.8
	spark_process.initial_velocity_max = 3.4
	spark_process.damping_min = 1.2
	spark_process.damping_max = 2.4
	spark_process.scale_min = 0.45
	spark_process.scale_max = 1.15
	var spark_mesh := SphereMesh.new()
	spark_mesh.radius = 0.018
	spark_mesh.height = 0.036
	spark_mesh.radial_segments = 5
	spark_mesh.rings = 3
	spark_mesh.material = detail_materials["amber"]
	action_sparks = GPUParticles3D.new()
	action_sparks.name = "ToolSparks"
	action_sparks.amount = 46
	action_sparks.lifetime = 0.42
	action_sparks.randomness = 0.72
	action_sparks.explosiveness = 0.18
	action_sparks.local_coords = false
	action_sparks.process_material = spark_process
	action_sparks.draw_pass_1 = spark_mesh
	action_sparks.emitting = false
	add_child(action_sparks)

	var fragment_process := ParticleProcessMaterial.new()
	fragment_process.emission_shape = ParticleProcessMaterial.EMISSION_SHAPE_SPHERE
	fragment_process.emission_sphere_radius = 0.28
	fragment_process.direction = Vector3(0.0, 0.0, 1.0)
	fragment_process.spread = 180.0
	fragment_process.initial_velocity_min = 0.9
	fragment_process.initial_velocity_max = 3.6
	fragment_process.damping_min = 0.10
	fragment_process.damping_max = 0.42
	fragment_process.scale_min = 0.45
	fragment_process.scale_max = 1.35
	var fragment_mesh := BoxMesh.new()
	fragment_mesh.size = Vector3(0.065, 0.045, 0.085)
	fragment_mesh.material = _material(Color(0.16, 0.19, 0.20), 0.97, 0.04)
	mining_fragments = GPUParticles3D.new()
	mining_fragments.name = "MiningFragments"
	mining_fragments.amount = 74
	mining_fragments.lifetime = 1.15
	mining_fragments.randomness = 0.78
	mining_fragments.explosiveness = 0.94
	mining_fragments.one_shot = true
	mining_fragments.local_coords = false
	mining_fragments.process_material = fragment_process
	mining_fragments.draw_pass_1 = fragment_mesh
	mining_fragments.emitting = false
	add_child(mining_fragments)


func _build_starfield() -> void:
	var mesh := SphereMesh.new()
	mesh.radius = 0.11
	mesh.height = 0.22
	mesh.radial_segments = 5
	mesh.rings = 3
	var material := StandardMaterial3D.new()
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.albedo_color = Color(0.70, 0.82, 1.0)
	material.emission_enabled = true
	material.emission = Color(0.38, 0.55, 1.0)
	material.emission_energy_multiplier = 1.7
	mesh.material = material
	var multimesh := MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_3D
	multimesh.mesh = mesh
	multimesh.instance_count = 420
	var random := RandomNumberGenerator.new()
	random.seed = 44019
	for index in multimesh.instance_count:
		var direction := Vector3(
			random.randf_range(-1.0, 1.0),
			random.randf_range(-1.0, 1.0),
			random.randf_range(-1.0, 1.0)
		).normalized()
		var distance := random.randf_range(180.0, 420.0)
		var scale := random.randf_range(0.35, 1.7)
		multimesh.set_instance_transform(
			index,
			Transform3D(Basis().scaled(Vector3.ONE * scale), direction * distance)
		)
	var instance := MultiMeshInstance3D.new()
	instance.multimesh = multimesh
	stars_root.add_child(instance)


func _build_target_highlight() -> void:
	target_highlight = MeshInstance3D.new()
	var mesh := BoxMesh.new()
	mesh.size = Vector3.ONE * 1.045
	var material := StandardMaterial3D.new()
	material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.albedo_color = Color(0.20, 0.85, 1.0, 0.24)
	material.cull_mode = BaseMaterial3D.CULL_FRONT
	mesh.material = material
	target_highlight.mesh = mesh
	target_highlight.visible = false
	add_child(target_highlight)
	build_preview = Node3D.new()
	build_preview.name = "ConstructionHologram"
	var preview_mesh := BoxMesh.new()
	preview_mesh.size = Vector3.ONE * 1.0
	preview_mesh.material = detail_materials["hologram"]
	var preview_body := MeshInstance3D.new()
	preview_body.mesh = preview_mesh
	build_preview.add_child(preview_body)
	var preview_front := _box_visual(Vector3(0.46, 0.10, 0.12), detail_materials["hologram"])
	preview_front.position = Vector3(0.0, 0.0, -0.54)
	build_preview.add_child(preview_front)
	for x in [-0.34, 0.34]:
		var preview_rail := _box_visual(
			Vector3(0.045, 0.72, 0.045), detail_materials["hologram"]
		)
		preview_rail.position = Vector3(x, 0.0, -0.50)
		build_preview.add_child(preview_rail)
	build_preview.visible = false
	add_child(build_preview)


func _build_viewmodel() -> void:
	tool_root = Node3D.new()
	tool_root.name = "SalvageToolViewmodel"
	tool_root.position = Vector3(0.42, -0.34, -0.74)
	tool_root.rotation_degrees = Vector3(-7.0, -10.0, 2.0)
	camera.add_child(tool_root)

	var body := _box_visual(Vector3(0.24, 0.22, 0.54), detail_materials["dark"])
	body.position = Vector3(0.0, 0.0, -0.02)
	tool_root.add_child(body)
	var housing := _box_visual(Vector3(0.29, 0.16, 0.28), detail_materials["steel"])
	housing.position = Vector3(0.0, 0.035, -0.27)
	tool_root.add_child(housing)
	var grip := _box_visual(Vector3(0.12, 0.29, 0.13), detail_materials["dark"])
	grip.position = Vector3(0.0, -0.21, 0.08)
	grip.rotation_degrees.x = -12.0
	tool_root.add_child(grip)

	tool_tip = Node3D.new()
	tool_tip.position = Vector3(0.0, 0.035, -0.50)
	tool_root.add_child(tool_tip)
	for offset in [-0.095, 0.095]:
		var rail := _cylinder_visual(0.034, 0.26, detail_materials["steel"])
		rail.rotation_degrees.x = 90.0
		rail.position = Vector3(offset, 0.0, -0.08)
		tool_tip.add_child(rail)
	var emitter := _cylinder_visual(0.074, 0.12, detail_materials["cyan"])
	emitter.rotation_degrees.x = 90.0
	emitter.position.z = -0.19
	tool_tip.add_child(emitter)
	tool_light = OmniLight3D.new()
	tool_light.light_color = Color(0.14, 0.72, 1.0)
	tool_light.light_energy = 0.0
	tool_light.omni_range = 4.0
	tool_light.position.z = -0.28
	tool_tip.add_child(tool_light)


func _box_visual(size: Vector3, material: Material) -> MeshInstance3D:
	var instance := MeshInstance3D.new()
	var mesh := BoxMesh.new()
	mesh.size = size
	mesh.material = material
	instance.mesh = mesh
	return instance


func _cylinder_visual(radius: float, height: float, material: Material) -> MeshInstance3D:
	var instance := MeshInstance3D.new()
	var mesh := CylinderMesh.new()
	mesh.top_radius = radius
	mesh.bottom_radius = radius
	mesh.height = height
	mesh.radial_segments = 12
	mesh.material = material
	instance.mesh = mesh
	return instance


func _build_interface() -> void:
	var canvas := CanvasLayer.new()
	add_child(canvas)

	var top_bar := ColorRect.new()
	top_bar.color = Color(0.012, 0.022, 0.034, 0.94)
	top_bar.set_anchors_and_offsets_preset(Control.PRESET_TOP_WIDE)
	top_bar.custom_minimum_size.y = 52.0
	canvas.add_child(top_bar)
	var accent := ColorRect.new()
	accent.color = Color(0.10, 0.67, 0.94, 0.9)
	accent.position = Vector2(0.0, 50.0)
	accent.size = Vector2(430.0, 2.0)
	top_bar.add_child(accent)

	var title := Label.new()
	title.text = "THE VERSE  //  ORBITAL OPERATIONS"
	title.position = Vector2(24.0, 13.0)
	title.add_theme_font_size_override("font_size", 19)
	title.add_theme_color_override("font_color", Color(0.78, 0.91, 0.98))
	top_bar.add_child(title)

	connection_label = Label.new()
	connection_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	connection_label.set_anchors_preset(Control.PRESET_TOP_RIGHT)
	connection_label.position = Vector2(-455.0, 16.0)
	connection_label.size = Vector2(430.0, 24.0)
	connection_label.add_theme_font_size_override("font_size", 12)
	top_bar.add_child(connection_label)

	var left_panel := _hud_panel(Vector2(20.0, 74.0), Vector2(354.0, 252.0))
	canvas.add_child(left_panel)
	var suit_heading := _hud_label("EVA SUIT // ORPHEUS-7", Vector2(16.0, 13.0), 12)
	suit_heading.add_theme_color_override("font_color", Color(0.42, 0.78, 0.96))
	left_panel.add_child(suit_heading)
	level_label = _hud_label("SALVAGER // LEVEL 1", Vector2(16.0, 39.0), 18)
	level_label.add_theme_color_override("font_color", Color(0.94, 0.72, 0.28))
	left_panel.add_child(level_label)
	telemetry_label = _hud_label("SUIT 100  O₂ 100  POWER 100", Vector2(16.0, 70.0), 13)
	telemetry_label.add_theme_color_override("font_color", Color(0.64, 0.90, 0.94))
	left_panel.add_child(telemetry_label)

	status_label = Label.new()
	status_label.position = Vector2(16.0, 170.0)
	status_label.size = Vector2(322.0, 65.0)
	status_label.add_theme_font_size_override("font_size", 11)
	status_label.add_theme_color_override("font_color", Color(0.42, 0.52, 0.59))
	left_panel.add_child(status_label)

	inventory_label = Label.new()
	inventory_label.position = Vector2(16.0, 102.0)
	inventory_label.size = Vector2(322.0, 58.0)
	inventory_label.add_theme_font_size_override("font_size", 14)
	inventory_label.add_theme_color_override("font_color", Color(0.92, 0.72, 0.32))
	left_panel.add_child(inventory_label)

	var mission_panel := _hud_panel(Vector2(-404.0, 74.0), Vector2(384.0, 238.0))
	mission_panel.set_anchors_preset(Control.PRESET_TOP_RIGHT)
	canvas.add_child(mission_panel)
	var mission_heading := _hud_label("ACTIVE CONTRACT // PRIORITY", Vector2(17.0, 14.0), 11)
	mission_heading.add_theme_color_override("font_color", Color(1.0, 0.35, 0.12))
	mission_panel.add_child(mission_heading)
	var contract_name := _hud_label("WAKE THE KHEPRI RELAY", Vector2(17.0, 39.0), 18)
	contract_name.add_theme_color_override("font_color", Color(0.90, 0.94, 0.96))
	mission_panel.add_child(contract_name)
	mission_label = _hud_label("Awaiting authoritative career record…", Vector2(17.0, 76.0), 14)
	mission_label.size = Vector2(350.0, 145.0)
	mission_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	mission_label.add_theme_color_override("font_color", Color(0.69, 0.78, 0.83))
	mission_panel.add_child(mission_label)

	var crosshair := Label.new()
	crosshair.text = "◇"
	crosshair.set_anchors_and_offsets_preset(Control.PRESET_CENTER)
	crosshair.position -= Vector2(11.0, 19.0)
	crosshair.add_theme_font_size_override("font_size", 26)
	crosshair.add_theme_color_override("font_color", Color(0.54, 0.91, 1.0, 0.86))
	canvas.add_child(crosshair)

	interaction_label = Label.new()
	interaction_label.set_anchors_preset(Control.PRESET_CENTER)
	interaction_label.position = Vector2(-210.0, 42.0)
	interaction_label.size = Vector2(420.0, 56.0)
	interaction_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	interaction_label.add_theme_font_size_override("font_size", 14)
	interaction_label.add_theme_color_override("font_color", Color(0.68, 0.91, 1.0))
	canvas.add_child(interaction_label)
	target_label = interaction_label

	action_progress = ProgressBar.new()
	action_progress.set_anchors_preset(Control.PRESET_CENTER)
	action_progress.position = Vector2(-122.0, 94.0)
	action_progress.size = Vector2(244.0, 7.0)
	action_progress.min_value = 0.0
	action_progress.max_value = 1.0
	action_progress.show_percentage = false
	action_progress.add_theme_stylebox_override("background", _bar_style(Color(0.02, 0.05, 0.07, 0.92)))
	action_progress.add_theme_stylebox_override("fill", _bar_style(Color(0.10, 0.73, 1.0, 0.94)))
	canvas.add_child(action_progress)

	mode_label = Label.new()
	mode_label.set_anchors_preset(Control.PRESET_BOTTOM_WIDE)
	mode_label.offset_top = -142.0
	mode_label.offset_bottom = -117.0
	mode_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	mode_label.add_theme_font_size_override("font_size", 12)
	mode_label.add_theme_color_override("font_color", Color(0.46, 0.73, 0.86))
	canvas.add_child(mode_label)

	var hotbar := _hud_panel(Vector2(-380.0, -112.0), Vector2(760.0, 62.0))
	hotbar.set_anchors_preset(Control.PRESET_CENTER_BOTTOM)
	canvas.add_child(hotbar)
	hotbar_label = _hud_label("", Vector2(14.0, 12.0), 13)
	hotbar_label.size = Vector2(732.0, 38.0)
	hotbar_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	hotbar.add_child(hotbar_label)
	selected_label = hotbar_label

	var controls := _hud_label(
		"WASD / SPACE / C  MOVE    SHIFT  BOOST    Q/E  ROLL    HOLD M  GRID THRUST    X  STOP    I  INVENTORY    B  BUILD",
		Vector2(20.0, -40.0),
		11
	)
	controls.set_anchors_preset(Control.PRESET_BOTTOM_LEFT)
	controls.add_theme_color_override("font_color", Color(0.43, 0.53, 0.60))
	canvas.add_child(controls)

	var bottom_bar := ColorRect.new()
	bottom_bar.color = Color(0.012, 0.022, 0.034, 0.94)
	bottom_bar.set_anchors_and_offsets_preset(Control.PRESET_BOTTOM_WIDE)
	bottom_bar.position.y -= 34.0
	bottom_bar.custom_minimum_size.y = 34.0
	canvas.add_child(bottom_bar)
	message_label = Label.new()
	message_label.position = Vector2(22.0, 8.0)
	message_label.size = Vector2(1350.0, 28.0)
	message_label.add_theme_font_size_override("font_size", 12)
	bottom_bar.add_child(message_label)
	_build_inventory_terminal(canvas)


func _build_inventory_terminal(canvas: CanvasLayer) -> void:
	inventory_overlay = Control.new()
	inventory_overlay.name = "EngineeringInventoryTerminal"
	inventory_overlay.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	inventory_overlay.mouse_filter = Control.MOUSE_FILTER_STOP
	inventory_overlay.visible = false
	canvas.add_child(inventory_overlay)

	var blackout := ColorRect.new()
	blackout.color = Color(0.001, 0.004, 0.007, 0.84)
	blackout.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	inventory_overlay.add_child(blackout)

	var terminal := ColorRect.new()
	terminal.color = Color(0.036, 0.045, 0.051, 0.99)
	terminal.anchor_left = 0.035
	terminal.anchor_top = 0.045
	terminal.anchor_right = 0.965
	terminal.anchor_bottom = 0.95
	inventory_overlay.add_child(terminal)

	var accent := ColorRect.new()
	accent.color = Color(0.34, 0.73, 0.84)
	accent.anchor_right = 1.0
	accent.offset_bottom = 2.0
	terminal.add_child(accent)

	var tabs := ["INVENTORY", "CONTROL PANEL", "PRODUCTION", "INFO"]
	for tab_index in tabs.size():
		var tab := Button.new()
		tab.text = tabs[tab_index]
		tab.position = Vector2(20.0 + float(tab_index) * 142.0, 12.0)
		tab.size = Vector2(136.0, 34.0)
		tab.disabled = tab_index != 0
		if tab_index == 0:
			tab.add_theme_color_override("font_color", Color(0.87, 0.96, 1.0))
			tab.add_theme_stylebox_override(
				"normal", _terminal_style(Color(0.10, 0.22, 0.27), Color(0.31, 0.70, 0.82), 1)
			)
		terminal.add_child(tab)

	var heading := _hud_label("INVENTORY", Vector2(22.0, 58.0), 23)
	heading.add_theme_color_override("font_color", Color(0.78, 0.90, 0.94))
	terminal.add_child(heading)
	var subheading := _hud_label(
		"CONNECTED INVENTORIES  //  TWO-PANE LOGISTICS  //  SERVER AUTHORITATIVE",
		Vector2(160.0, 66.0),
		11
	)
	subheading.add_theme_color_override("font_color", Color(0.42, 0.62, 0.68))
	terminal.add_child(subheading)

	var close_button := Button.new()
	close_button.text = "CLOSE  [I]"
	close_button.set_anchors_preset(Control.PRESET_TOP_RIGHT)
	close_button.position = Vector2(-138.0, 14.0)
	close_button.size = Vector2(112.0, 34.0)
	close_button.pressed.connect(_set_inventory_open.bind(false))
	terminal.add_child(close_button)

	var suit_panel := _inventory_column(
		terminal, 0.018, 0.462, "ORPHEUS-7", "SUIT INVENTORY", "suit"
	)
	var cargo_panel := _inventory_column(
		terminal, 0.538, 0.982, "KHEPRI RELAY", "LARGE CARGO CONTAINER", "cargo"
	)
	for resource_data in [
		["ore", "Ferrite Ore", "Ore", Color(0.62, 0.34, 0.18), 37, 3.5],
		["refined_material", "Registered Alloy", "Ingot", Color(0.42, 0.66, 0.73), 15, 2.4],
		["component", "Construction Part", "Component", Color(0.71, 0.58, 0.24), 22, 4.8],
	]:
		var resource := String(resource_data[0])
		var index := ["ore", "refined_material", "component"].find(resource)
		_inventory_resource_row(
			suit_panel, "suit", resource, String(resource_data[1]), String(resource_data[2]),
			resource_data[3], int(resource_data[4]), float(resource_data[5]), 190.0 + float(index) * 58.0
		)
		_inventory_resource_row(
			cargo_panel, "cargo", resource, String(resource_data[1]), String(resource_data[2]),
			resource_data[3], int(resource_data[4]), float(resource_data[5]), 190.0 + float(index) * 58.0
		)
	_add_transfer_controls(terminal)
	_update_inventory_row_styles()

	var hint := _hud_label(
		"SELECT AN ITEM, THEN USE THE CENTER ARROWS  //  SINGLE = ONE UNIT  //  DOUBLE = FULL STACK  //  V = QUICK TRANSFER",
		Vector2(24.0, -34.0),
		11
	)
	hint.set_anchors_preset(Control.PRESET_BOTTOM_LEFT)
	hint.add_theme_color_override("font_color", Color(0.45, 0.66, 0.73))
	terminal.add_child(hint)


func _inventory_column(
	parent: Control, left: float, right: float, title: String, subtitle: String, side: String
) -> ColorRect:
	var panel := ColorRect.new()
	panel.color = Color(0.052, 0.063, 0.069, 0.98)
	panel.anchor_left = left
	panel.anchor_top = 0.12
	panel.anchor_right = right
	panel.anchor_bottom = 0.92
	parent.add_child(panel)
	var title_label := _hud_label(title, Vector2(16.0, 12.0), 16)
	title_label.add_theme_color_override("font_color", Color(0.80, 0.89, 0.91))
	panel.add_child(title_label)
	var subtitle_label := _hud_label(subtitle, Vector2(16.0, 36.0), 10)
	subtitle_label.add_theme_color_override("font_color", Color(0.38, 0.67, 0.74))
	panel.add_child(subtitle_label)

	var selector := OptionButton.new()
	selector.position = Vector2(16.0, 60.0)
	selector.size = Vector2(438.0, 32.0)
	selector.add_item("%s  /  %s" % [title, subtitle])
	panel.add_child(selector)
	var search := LineEdit.new()
	search.placeholder_text = "Search inventory"
	search.position = Vector2(16.0, 99.0)
	search.size = Vector2(438.0, 30.0)
	search.text_changed.connect(_inventory_search_changed.bind(side))
	panel.add_child(search)

	for filter_data in [["ALL", "all"], ["ORE", "ore"], ["INGOT", "refined_material"], ["COMP", "component"]]:
		var filter_button := Button.new()
		filter_button.text = String(filter_data[0])
		filter_button.position = Vector2(16.0 + float(inventory_rows.size() % 4) * 77.0, 136.0)
		filter_button.size = Vector2(72.0, 26.0)
		filter_button.pressed.connect(_set_inventory_filter.bind(side, String(filter_data[1])))
		panel.add_child(filter_button)
		inventory_rows["filter-slot-%s-%s" % [side, filter_data[1]]] = filter_button

	for header_data in [["ITEM", 17.0], ["AMOUNT", 266.0], ["VOL.", 335.0], ["MASS", 391.0]]:
		var header := _hud_label(String(header_data[0]), Vector2(float(header_data[1]), 169.0), 9)
		header.add_theme_color_override("font_color", Color(0.46, 0.57, 0.60))
		panel.add_child(header)

	var capacity_bar := ProgressBar.new()
	capacity_bar.set_anchors_preset(Control.PRESET_BOTTOM_LEFT)
	capacity_bar.position = Vector2(16.0, -41.0)
	capacity_bar.size = Vector2(438.0, 9.0)
	capacity_bar.max_value = 1.0
	capacity_bar.show_percentage = false
	capacity_bar.add_theme_stylebox_override("background", _bar_style(Color(0.018, 0.023, 0.026)))
	capacity_bar.add_theme_stylebox_override("fill", _bar_style(Color(0.27, 0.68, 0.77)))
	panel.add_child(capacity_bar)
	var capacity_label := _hud_label("0 / 0 L", Vector2(16.0, -66.0), 11)
	capacity_label.set_anchors_preset(Control.PRESET_BOTTOM_LEFT)
	capacity_label.size = Vector2(438.0, 22.0)
	capacity_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	capacity_label.add_theme_color_override("font_color", Color(0.57, 0.72, 0.75))
	panel.add_child(capacity_label)
	inventory_capacity_bars[side] = capacity_bar
	inventory_capacity_labels[side] = capacity_label
	return panel


func _inventory_resource_row(
	parent: Control,
	side: String,
	resource: String,
	name: String,
	code: String,
	color: Color,
	unit_liters: int,
	unit_mass_kg: float,
	y: float
) -> void:
	var row := Button.new()
	row.text = ""
	row.position = Vector2(16.0, y)
	row.size = Vector2(438.0, 52.0)
	row.focus_mode = Control.FOCUS_NONE
	row.pressed.connect(_select_inventory_resource.bind(side, resource))
	parent.add_child(row)
	var icon := ColorRect.new()
	icon.color = color
	icon.position = Vector2(7.0, 7.0)
	icon.size = Vector2(38.0, 38.0)
	icon.mouse_filter = Control.MOUSE_FILTER_IGNORE
	row.add_child(icon)
	var icon_inner := ColorRect.new()
	icon_inner.color = Color(color, 0.55)
	icon_inner.position = Vector2(5.0, 5.0)
	icon_inner.size = Vector2(28.0, 28.0)
	icon_inner.mouse_filter = Control.MOUSE_FILTER_IGNORE
	icon.add_child(icon_inner)
	var name_label := _hud_label(name, Vector2(54.0, 5.0), 13)
	name_label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	name_label.add_theme_color_override("font_color", Color(0.82, 0.87, 0.88))
	row.add_child(name_label)
	var code_label := _hud_label(code, Vector2(54.0, 27.0), 9)
	code_label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	code_label.add_theme_color_override("font_color", Color(0.43, 0.57, 0.60))
	row.add_child(code_label)
	var quantity_label := _hud_label("0", Vector2(248.0, 13.0), 13)
	quantity_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	quantity_label.size = Vector2(62.0, 25.0)
	quantity_label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	quantity_label.add_theme_color_override("font_color", Color(0.91, 0.91, 0.82))
	row.add_child(quantity_label)
	var volume_label := _hud_label("0 L", Vector2(318.0, 14.0), 11)
	volume_label.size = Vector2(54.0, 23.0)
	volume_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	volume_label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	row.add_child(volume_label)
	var mass_label := _hud_label("0 kg", Vector2(373.0, 14.0), 11)
	mass_label.size = Vector2(57.0, 23.0)
	mass_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	mass_label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	row.add_child(mass_label)
	inventory_item_labels["%s:%s:amount" % [side, resource]] = quantity_label
	inventory_item_labels["%s:%s:volume" % [side, resource]] = [volume_label, unit_liters]
	inventory_item_labels["%s:%s:mass" % [side, resource]] = [mass_label, unit_mass_kg]
	inventory_rows["%s:%s" % [side, resource]] = row


func _add_transfer_controls(parent: Control) -> void:
	var selected_hint := _hud_label("SELECT\nITEM", Vector2(-33.0, 170.0), 10)
	selected_hint.set_anchors_preset(Control.PRESET_CENTER_TOP)
	selected_hint.size = Vector2(66.0, 42.0)
	selected_hint.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	selected_hint.add_theme_color_override("font_color", Color(0.42, 0.60, 0.64))
	parent.add_child(selected_hint)
	for data in [
		["→", Vector2(-46.0, 226.0), false, false],
		["⇒", Vector2(2.0, 226.0), false, true],
		["←", Vector2(-46.0, 266.0), true, false],
		["⇐", Vector2(2.0, 266.0), true, true],
	]:
		var button := Button.new()
		button.text = String(data[0])
		button.set_anchors_preset(Control.PRESET_CENTER_TOP)
		button.position = data[1]
		button.size = Vector2(44.0, 34.0)
		button.pressed.connect(_transfer_selected_inventory.bind(bool(data[2]), bool(data[3])))
		parent.add_child(button)


func _transfer_selected_inventory(reverse: bool, all: bool) -> void:
	_transfer_inventory_resource(inventory_selected_resource, reverse, all)


func _select_inventory_resource(side: String, resource: String) -> void:
	inventory_selected_side = side
	inventory_selected_resource = resource
	_update_inventory_row_styles()


func _set_inventory_filter(side: String, filter: String) -> void:
	inventory_filters[side] = filter
	_apply_inventory_visibility(side)


func _inventory_search_changed(query: String, side: String) -> void:
	inventory_search_queries[side] = query.strip_edges().to_lower()
	_apply_inventory_visibility(side)


func _apply_inventory_visibility(side: String) -> void:
	var normalized := String(inventory_search_queries.get(side, ""))
	var filter := String(inventory_filters.get(side, "all"))
	for resource_data in [
		["ore", "ferrite ore"],
		["refined_material", "registered alloy ingot"],
		["component", "construction part component"],
	]:
		var resource := String(resource_data[0])
		var row: Button = inventory_rows.get("%s:%s" % [side, resource], null)
		if row != null:
			row.visible = (
				(normalized.is_empty() or String(resource_data[1]).contains(normalized))
				and (filter == "all" or filter == resource)
			)


func _update_inventory_row_styles() -> void:
	for side in ["suit", "cargo"]:
		for resource in ["ore", "refined_material", "component"]:
			var row: Button = inventory_rows.get("%s:%s" % [side, resource], null)
			if row == null:
				continue
			var selected: bool = (
				side == inventory_selected_side and resource == inventory_selected_resource
			)
			row.add_theme_stylebox_override(
				"normal",
				_terminal_style(
					Color(0.10, 0.15, 0.17) if selected else Color(0.035, 0.043, 0.047),
					Color(0.34, 0.73, 0.84) if selected else Color(0.10, 0.13, 0.14),
					1
				)
			)
			row.add_theme_stylebox_override(
				"hover", _terminal_style(Color(0.075, 0.11, 0.12), Color(0.28, 0.55, 0.61), 1)
			)


func _hud_panel(position: Vector2, size: Vector2) -> ColorRect:
	var panel := ColorRect.new()
	panel.color = Color(0.012, 0.025, 0.037, 0.88)
	panel.position = position
	panel.size = size
	return panel


func _hud_label(text: String, position: Vector2, font_size: int) -> Label:
	var label := Label.new()
	label.text = text
	label.position = position
	label.add_theme_font_size_override("font_size", font_size)
	return label


func _bar_style(color: Color) -> StyleBoxFlat:
	var style := StyleBoxFlat.new()
	style.bg_color = color
	style.corner_radius_top_left = 2
	style.corner_radius_top_right = 2
	style.corner_radius_bottom_left = 2
	style.corner_radius_bottom_right = 2
	return style


func _terminal_style(background: Color, border: Color, width: int) -> StyleBoxFlat:
	var style := StyleBoxFlat.new()
	style.bg_color = background
	style.border_color = border
	style.set_border_width_all(width)
	style.corner_radius_top_left = 1
	style.corner_radius_top_right = 1
	style.corner_radius_bottom_left = 1
	style.corner_radius_bottom_right = 1
	return style


func _connect_to_server() -> void:
	socket.close()
	socket = WebSocketPeer.new()
	socket.inbound_buffer_size = 8 * 1024 * 1024
	socket.outbound_buffer_size = 1024 * 1024
	connected = false
	handshake_sent = false
	closed_reported = false
	last_socket_state = -1
	var result := socket.connect_to_url(server_url)
	if smoke_test:
		print("VERSE_SMOKE_CONNECT url=%s result=%d" % [server_url, result])
	if result != OK:
		_set_message("Unable to begin connection: %s" % error_string(result), true)
	else:
		_set_message("Connecting to %s" % server_url)


func _poll_socket() -> void:
	socket.poll()
	var state := socket.get_ready_state()
	if smoke_test and state != last_socket_state:
		last_socket_state = state
		print("VERSE_SMOKE_SOCKET_STATE %d" % state)
	if state == WebSocketPeer.STATE_OPEN:
		connected = true
		if not handshake_sent:
			handshake_sent = true
			_send({
				"type": "hello",
				"protocol_version": PROTOCOL_VERSION,
				"client_name": "godot-native-p0.7",
			})
		while socket.get_available_packet_count() > 0:
			var text := socket.get_packet().get_string_from_utf8()
			var parsed: Variant = JSON.parse_string(text)
			if parsed is Dictionary:
				_handle_server_message(parsed)
	elif state == WebSocketPeer.STATE_CLOSED:
		if smoke_test and not connected and not closed_reported:
			closed_reported = true
			print(
				"VERSE_SMOKE_CLOSED code=%d reason=%s"
				% [socket.get_close_code(), socket.get_close_reason()]
			)
		if connected:
			_set_message(
				"Disconnected (%d): %s — press F5 to reconnect"
				% [socket.get_close_code(), socket.get_close_reason()],
				true
			)
		connected = false
		handshake_sent = false


func _handle_server_message(message: Dictionary) -> void:
	match message.get("type", ""):
		"welcome":
			_set_message("Connected to %s" % message.get("server_name", "The Verse"))
		"snapshot":
			_apply_snapshot(message.get("snapshot", {}))
		"intent_accepted":
			var receipt: Dictionary = message.get("receipt", {})
			_set_message(receipt.get("message", "Intent accepted"))
			if smoke_test and receipt.get("operation_id", "") == smoke_operation:
				print("VERSE_SMOKE_OK event=%d" % int(receipt.get("event_sequence", 0)))
				get_tree().quit(0)
		"intent_rejected":
			pending_mine_position = null
			_set_message(
				"%s — %s" % [message.get("code", "rejected"), message.get("message", "")],
				true
			)
		"fatal":
			_set_message(
				"FATAL %s — %s" % [message.get("code", ""), message.get("message", "")],
				true
			)


func _apply_snapshot(authoritative: Dictionary) -> void:
	if authoritative.is_empty():
		return
	snapshot = authoritative
	var player: Dictionary = snapshot.get("player", {})
	var position := _vec3(player.get("position", {}))
	if first_snapshot or camera.position.distance_to(position) > 2.8:
		camera.position = position
		if first_snapshot:
			var grids: Array = snapshot.get("grids", [])
			var focus := Vector3.ZERO
			if not grids.is_empty():
				focus = _vec3(grids[0].get("position", {}))
			camera.look_at(focus, Vector3.UP)
	last_sent_position = position
	first_snapshot = false
	var level := int(player.get("level", 1))
	if level > last_level:
		_set_message("CLEARANCE ADVANCED // SALVAGER LEVEL %d" % level)
		tool_kick = 1.0
	last_level = level

	var voxels: Array = snapshot.get("voxels", [])
	if voxels.size() != rendered_voxel_count:
		rendered_voxel_count = voxels.size()
		_rebuild_voxels(voxels)
		if pending_mine_position != null:
			var mined_coordinate: Vector3i = pending_mine_position
			if not voxel_lookup.has(_coord_key(mined_coordinate)):
				_emit_mining_fragments(Vector3(mined_coordinate))
				pending_mine_position = null
	_rebuild_grids(snapshot.get("grids", []))
	if smoke_test and smoke_operation.is_empty():
		if not _run_visual_smoke_assertions():
			get_tree().quit(1)
			return
		smoke_operation = _operation_id("godot-smoke")
		_send({
			"type": "move_player",
			"operation_id": smoke_operation,
			"position": _protocol_vec3(position + Vector3(0.01, 0.0, 0.0)),
		})


func _rebuild_voxels(voxels: Array) -> void:
	var started_usec := Time.get_ticks_usec()
	var previous_lookup := voxel_lookup
	var previous_coordinates := voxel_coordinate_lookup
	var next_lookup: Dictionary = {}
	var next_coordinates: Dictionary = {}
	for voxel in voxels:
		var coordinate: Dictionary = voxel.get("coordinate", {})
		var grid_position := Vector3i(
			int(coordinate.get("x", 0)),
			int(coordinate.get("y", 0)),
			int(coordinate.get("z", 0))
		)
		var key := _coord_key(grid_position)
		next_lookup[key] = voxel
		next_coordinates[key] = grid_position
	voxel_lookup = next_lookup
	voxel_coordinate_lookup = next_coordinates

	var dirty_chunks: Dictionary = {}
	if previous_lookup.is_empty():
		for coordinate in next_coordinates.values():
			_mark_chunks_influenced_by_voxel(dirty_chunks, coordinate)
	else:
		var changed_keys: Dictionary = {}
		for key in previous_lookup:
			changed_keys[key] = true
		for key in next_lookup:
			changed_keys[key] = true
		for key in changed_keys:
			var previous: Dictionary = previous_lookup.get(key, {})
			var current: Dictionary = next_lookup.get(key, {})
			if previous == current:
				continue
			var changed_coordinate: Vector3i = next_coordinates.get(
				key, previous_coordinates.get(key, Vector3i.ZERO)
			)
			_mark_chunks_influenced_by_voxel(dirty_chunks, changed_coordinate)

	for chunk in dirty_chunks.values():
		_rebuild_voxel_chunk(chunk)
	var elapsed_ms := float(Time.get_ticks_usec() - started_usec) / 1000.0
	print(
		"VERSE_VOXEL_REMESH chunks=%d total_chunks=%d voxels=%d elapsed_ms=%.3f"
		% [dirty_chunks.size(), voxel_chunk_nodes.size(), voxels.size(), elapsed_ms]
	)


func _emit_mining_fragments(position: Vector3) -> void:
	mining_fragments.global_position = position
	mining_fragments.restart()
	mining_fragments.emitting = true


func _voxel_chunk_coordinate(coordinate: Vector3i) -> Vector3i:
	return Vector3i(
		floori(float(coordinate.x) / float(VOXEL_CHUNK_SIZE)),
		floori(float(coordinate.y) / float(VOXEL_CHUNK_SIZE)),
		floori(float(coordinate.z) / float(VOXEL_CHUNK_SIZE))
	)


func _mark_chunks_influenced_by_voxel(chunks: Dictionary, coordinate: Vector3i) -> void:
	var minimum := _voxel_chunk_coordinate(coordinate - Vector3i(2, 2, 2))
	var maximum := _voxel_chunk_coordinate(coordinate + Vector3i.ONE)
	for x in range(minimum.x, maximum.x + 1):
		for y in range(minimum.y, maximum.y + 1):
			for z in range(minimum.z, maximum.z + 1):
				var chunk := Vector3i(x, y, z)
				chunks[_coord_key(chunk)] = chunk


func _rebuild_voxel_chunk(chunk: Vector3i) -> void:
	var chunk_key := _coord_key(chunk)
	var previous: MeshInstance3D = voxel_chunk_nodes.get(chunk_key, null)
	if previous != null:
		previous.queue_free()
		voxel_chunk_nodes.erase(chunk_key)
	var origin := chunk * VOXEL_CHUNK_SIZE
	var surface := SurfaceTool.new()
	surface.begin(Mesh.PRIMITIVE_TRIANGLES)
	for x in range(origin.x, origin.x + VOXEL_CHUNK_SIZE):
		for y in range(origin.y, origin.y + VOXEL_CHUNK_SIZE):
			for z in range(origin.z, origin.z + VOXEL_CHUNK_SIZE):
				var cell_origin := Vector3i(x, y, z)
				var corner_points: Array[Vector3] = []
				var corner_values: Array[float] = []
				var filled_count := 0
				for offset in MARCHING_CORNERS:
					var coordinate := cell_origin + offset
					var density := _voxel_density(coordinate)
					corner_points.append(Vector3(coordinate))
					corner_values.append(density)
					if density >= ISO_LEVEL:
						filled_count += 1
				if filled_count == 0 or filled_count == MARCHING_CORNERS.size():
					continue
				for tetrahedron in MARCHING_TETRAHEDRA:
					var indices: Array[int] = [
						tetrahedron.x, tetrahedron.y, tetrahedron.z, tetrahedron.w,
					]
					var tetra_points: Array[Vector3] = []
					var tetra_values: Array[float] = []
					for index in indices:
						tetra_points.append(corner_points[index])
						tetra_values.append(corner_values[index])
					_polygonize_tetrahedron(surface, tetra_points, tetra_values)
	surface.index()
	# SurfaceTool's indexed normal generation keeps tetrahedron winding and the
	# displacement shader consistent at chunk boundaries. Supplying only radial
	# normals caused near-camera chunks to shade as disconnected black shards.
	surface.generate_normals()
	var mesh := surface.commit()
	if mesh == null or mesh.get_surface_count() == 0:
		return
	var instance := MeshInstance3D.new()
	instance.name = "VoxelChunk_%d_%d_%d" % [chunk.x, chunk.y, chunk.z]
	instance.mesh = mesh
	instance.material_override = rock_material
	asteroid_root.add_child(instance)
	voxel_chunk_nodes[chunk_key] = instance


func _voxel_density(coordinate: Vector3i) -> float:
	var density := 0.46 if voxel_lookup.has(_coord_key(coordinate)) else 0.0
	for offset in DENSITY_NEIGHBORS:
		if voxel_lookup.has(_coord_key(coordinate + offset)):
			density += 0.09
	return density


func _polygonize_tetrahedron(
	surface: SurfaceTool,
	points: Array[Vector3],
	values: Array[float]
) -> void:
	var inside: Array[int] = []
	var outside: Array[int] = []
	for index in 4:
		if values[index] >= ISO_LEVEL:
			inside.append(index)
		else:
			outside.append(index)
	if inside.is_empty() or outside.is_empty():
		return
	var material_samples: Array[Vector3] = []
	for index in inside:
		material_samples.append(points[index])
	if inside.size() == 1:
		_add_surface_triangle(
			surface,
			_iso_intersection(
				points[inside[0]], values[inside[0]], points[outside[0]], values[outside[0]]
			),
			_iso_intersection(
				points[inside[0]], values[inside[0]], points[outside[1]], values[outside[1]]
			),
			_iso_intersection(
				points[inside[0]], values[inside[0]], points[outside[2]], values[outside[2]]
			),
			material_samples
		)
	elif inside.size() == 3:
		_add_surface_triangle(
			surface,
			_iso_intersection(
				points[outside[0]], values[outside[0]], points[inside[0]], values[inside[0]]
			),
			_iso_intersection(
				points[outside[0]], values[outside[0]], points[inside[1]], values[inside[1]]
			),
			_iso_intersection(
				points[outside[0]], values[outside[0]], points[inside[2]], values[inside[2]]
			),
			material_samples
		)
	else:
		var edge_a := _iso_intersection(
			points[inside[0]], values[inside[0]], points[outside[0]], values[outside[0]]
		)
		var edge_b := _iso_intersection(
			points[inside[0]], values[inside[0]], points[outside[1]], values[outside[1]]
		)
		var edge_c := _iso_intersection(
			points[inside[1]], values[inside[1]], points[outside[0]], values[outside[0]]
		)
		var edge_d := _iso_intersection(
			points[inside[1]], values[inside[1]], points[outside[1]], values[outside[1]]
		)
		_add_surface_triangle(surface, edge_a, edge_b, edge_d, material_samples)
		_add_surface_triangle(surface, edge_a, edge_d, edge_c, material_samples)


func _iso_intersection(
	first_point: Vector3,
	first_value: float,
	second_point: Vector3,
	second_value: float
) -> Vector3:
	var difference := second_value - first_value
	if absf(difference) < 0.0001:
		return first_point.lerp(second_point, 0.5)
	var fraction := clampf((ISO_LEVEL - first_value) / difference, 0.0, 1.0)
	return first_point.lerp(second_point, fraction)


func _add_surface_triangle(
	surface: SurfaceTool,
	first: Vector3,
	second: Vector3,
	third: Vector3,
	material_samples: Array[Vector3]
) -> void:
	var centroid := (first + second + third) / 3.0
	var face_normal := (second - first).cross(third - first).normalized()
	if face_normal.dot(centroid) < 0.0:
		var swap := second
		second = third
		third = swap
		face_normal = -face_normal
	var triangle_points: Array[Vector3] = [first, second, third]
	for point in triangle_points:
		var radial_normal: Vector3 = point.normalized()
		surface.set_normal(radial_normal)
		surface.set_color(_voxel_surface_color(point, material_samples))
		surface.add_vertex(point)


func _voxel_surface_color(point: Vector3, material_samples: Array[Vector3]) -> Color:
	var variation := _position_variation(point * 1.37)
	var ferrite := false
	for sample in material_samples:
		var coordinate := Vector3i(roundi(sample.x), roundi(sample.y), roundi(sample.z))
		var voxel: Dictionary = voxel_lookup.get(_coord_key(coordinate), {})
		if voxel.get("material", "rock") == "ferrite_ore":
			ferrite = true
	if ferrite:
		return Color(0.68 + variation * 0.14, 0.19, 0.055, 1.0)
	var shade := 0.27 + variation * 0.11
	return Color(shade * 0.84, shade * 0.94, shade, 1.0)


func _position_variation(position: Vector3) -> float:
	return fposmod(
		abs(sin(position.dot(Vector3(12.9898, 78.233, 37.719))) * 43758.5453),
		1.0
	)


func _rebuild_grids(grids: Array) -> void:
	for child in grids_root.get_children():
		child.queue_free()
	grid_lookup.clear()
	for grid in grids:
		var grid_id: String = grid.get("grid_id", "")
		grid_lookup[grid_id] = grid
		var grid_node := Node3D.new()
		grid_node.name = grid_id
		grid_node.position = _vec3(grid.get("position", {}))
		grid_node.quaternion = _grid_quaternion(grid)
		for block in grid.get("blocks", []):
			var block_visual := _build_block_visual(block)
			var coordinate: Dictionary = block.get("coordinate", {})
			block_visual.position = Vector3(
				float(coordinate.get("x", 0)),
				float(coordinate.get("y", 0)),
				float(coordinate.get("z", 0))
			)
			block_visual.rotation.y = deg_to_rad(float(int(block.get("orientation", 0)) * 90))
			grid_node.add_child(block_visual)
		if grid.get("power", {}).get("online", false):
			var work_light := OmniLight3D.new()
			work_light.light_color = Color(0.24, 0.72, 1.0)
			work_light.light_energy = 1.35
			work_light.omni_range = 10.0
			work_light.shadow_enabled = true
			work_light.position = Vector3(0.0, 2.2, 0.0)
			grid_node.add_child(work_light)
		grids_root.add_child(grid_node)


func _build_block_visual(block: Dictionary) -> Node3D:
	var root := Node3D.new()
	root.name = block.get("block_id", "block")
	var kind: String = block.get("kind", "structural")
	var health := int(block.get("health", 1))
	var max_health := maxi(int(block.get("max_health", health)), 1)
	var integrity := clampf(float(health) / float(max_health), 0.0, 1.0)
	var construction_complete := bool(block.get("construction_complete", integrity >= 1.0))
	if not construction_complete:
		root.set_meta("verse_visual_state", "frame")
		var frame_core := _box_visual(
			Vector3.ONE * lerpf(0.38, 0.72, integrity), detail_materials["construction"]
		)
		root.add_child(frame_core)
		_add_construction_frame(root, integrity)
		return root
	root.set_meta("verse_visual_state", "armor_damaged" if integrity < 1.0 else "armor_complete")
	var base := _box_visual(
		Vector3.ONE * 1.01,
		block_materials.get(kind, block_materials["structural"])
	)
	root.add_child(base)
	var front_panel := _box_visual(Vector3(0.70, 0.66, 0.026), detail_materials["dark"])
	front_panel.position.z = -0.518
	root.add_child(front_panel)

	match kind:
		"structural":
			for z in [-0.43, 0.43]:
				var horizontal_rail := _box_visual(
					Vector3(0.78, 0.045, 0.04), detail_materials["steel"]
				)
				horizontal_rail.position = Vector3(0.0, 0.47, z)
				root.add_child(horizontal_rail)
			for x in [-0.43, 0.43]:
				var vertical_rail := _box_visual(
					Vector3(0.04, 0.045, 0.78), detail_materials["steel"]
				)
				vertical_rail.position = Vector3(x, 0.47, 0.0)
				root.add_child(vertical_rail)
			for x in [-0.34, 0.34]:
				for y in [-0.31, 0.31]:
					var fastener := _cylinder_visual(0.035, 0.025, detail_materials["steel"])
					fastener.rotation_degrees.x = 90.0
					fastener.position = Vector3(x, y, -0.50)
					root.add_child(fastener)
		"control_core":
			var canopy := _box_visual(Vector3(0.70, 0.34, 0.58), detail_materials["glass"])
			canopy.position = Vector3(0.0, 0.52, 0.03)
			root.add_child(canopy)
			for x in [-0.37, 0.37]:
				var canopy_frame := _box_visual(
					Vector3(0.045, 0.42, 0.64), detail_materials["steel"]
				)
				canopy_frame.position = Vector3(x, 0.50, 0.03)
				root.add_child(canopy_frame)
			var display := _box_visual(Vector3(0.44, 0.30, 0.045), detail_materials["cyan"])
			display.position = Vector3(0.0, 0.05, -0.505)
			root.add_child(display)
			var mast := _cylinder_visual(0.035, 0.72, detail_materials["steel"])
			mast.position = Vector3(0.0, 0.76, 0.0)
			root.add_child(mast)
			var beacon := _cylinder_visual(0.075, 0.11, detail_materials["cyan"])
			beacon.position = Vector3(0.0, 1.14, 0.0)
			root.add_child(beacon)
		"power_source":
			var reactor_core := _cylinder_visual(0.22, 0.18, detail_materials["amber"])
			reactor_core.rotation_degrees.x = 90.0
			reactor_core.position.z = -0.56
			root.add_child(reactor_core)
			for y in [-0.25, 0.0, 0.25]:
				var vent := _box_visual(Vector3(0.72, 0.055, 0.045), detail_materials["steel"])
				vent.position = Vector3(0.0, y, -0.535)
				root.add_child(vent)
			root.add_child(_block_face_label("REACTOR", Color(1.0, 0.52, 0.12)))
		"battery":
			for x in [-0.22, 0.0, 0.22]:
				var cell := _cylinder_visual(0.085, 0.56, detail_materials["amber"])
				cell.position = Vector3(x, 0.0, -0.48)
				root.add_child(cell)
			var battery_bus := _box_visual(Vector3(0.66, 0.08, 0.055), detail_materials["cyan"])
			battery_bus.position = Vector3(0.0, -0.35, -0.54)
			root.add_child(battery_bus)
		"cargo":
			var door := _box_visual(Vector3(0.68, 0.62, 0.045), detail_materials["steel"])
			door.position.z = -0.535
			root.add_child(door)
			for offset in [-0.25, 0.25]:
				var latch := _box_visual(Vector3(0.075, 0.48, 0.04), detail_materials["dark"])
				latch.position = Vector3(offset, -0.02, -0.565)
				root.add_child(latch)
			var cargo_light := _box_visual(Vector3(0.33, 0.055, 0.065), detail_materials["green"])
			cargo_light.position = Vector3(0.0, 0.36, -0.57)
			root.add_child(cargo_light)
			root.add_child(_block_face_label("CARGO", Color(0.26, 1.0, 0.61)))
		"drill":
			var shaft := _cylinder_visual(0.16, 0.82, detail_materials["steel"])
			shaft.rotation_degrees.x = 90.0
			shaft.position.z = -0.68
			root.add_child(shaft)
			var bit := MeshInstance3D.new()
			var cone := CylinderMesh.new()
			cone.top_radius = 0.0
			cone.bottom_radius = 0.25
			cone.height = 0.52
			cone.radial_segments = 8
			cone.material = detail_materials["steel"]
			bit.mesh = cone
			bit.rotation_degrees.x = 90.0
			bit.position.z = -1.30
			root.add_child(bit)
		"anchor":
			var piston := _cylinder_visual(0.18, 0.58, detail_materials["dark"])
			piston.rotation_degrees.x = 90.0
			piston.position.z = -0.66
			root.add_child(piston)
			for x in [-0.22, 0.22]:
				var prong := _box_visual(Vector3(0.13, 0.18, 0.65), detail_materials["steel"])
				prong.position = Vector3(x, 0.0, -0.65)
				root.add_child(prong)
			var anchor_light := _box_visual(Vector3(0.36, 0.07, 0.05), detail_materials["cyan"])
			anchor_light.position = Vector3(0.0, 0.30, -0.51)
			root.add_child(anchor_light)
		"damage_test":
			for offset in [-0.22, 0.22]:
				var warning := _box_visual(Vector3(0.11, 0.62, 0.055), detail_materials["red"])
				warning.position = Vector3(offset, 0.0, -0.51)
				warning.rotation_degrees.z = 22.0
				root.add_child(warning)
	if integrity < 1.0:
		_add_damage_overlay(root, integrity, String(block.get("block_id", "block")))
	return root


func _block_face_label(text: String, color: Color) -> Label3D:
	var label := Label3D.new()
	label.text = text
	label.font_size = 28
	label.pixel_size = 0.0032
	label.modulate = color
	label.outline_modulate = Color(0.0, 0.0, 0.0, 0.9)
	label.outline_size = 5
	label.position = Vector3(0.0, -0.35, -0.574)
	return label


func _add_construction_frame(root: Node3D, integrity: float) -> void:
	var frame_material: Material = detail_materials["construction"]
	for x in [-0.44, 0.44]:
		for y in [-0.44, 0.44]:
			var z_rail := _box_visual(Vector3(0.055, 0.055, 1.0), frame_material)
			z_rail.position = Vector3(x, y, 0.0)
			root.add_child(z_rail)
	for x in [-0.44, 0.44]:
		for z in [-0.44, 0.44]:
			var y_rail := _box_visual(Vector3(0.055, 1.0, 0.055), frame_material)
			y_rail.position = Vector3(x, 0.0, z)
			root.add_child(y_rail)
	for y in [-0.44, 0.44]:
		for z in [-0.44, 0.44]:
			var x_rail := _box_visual(Vector3(1.0, 0.055, 0.055), frame_material)
			x_rail.position = Vector3(0.0, y, z)
			root.add_child(x_rail)
	var completed_quarters := clampi(ceili(integrity * 4.0), 1, 3)
	for index in completed_quarters:
		var plate := _box_visual(Vector3(0.22, 0.22, 0.035), detail_materials["amber"])
		plate.position = Vector3(-0.27 + float(index) * 0.27, -0.25, -0.49)
		root.add_child(plate)


func _add_damage_overlay(root: Node3D, integrity: float, block_id: String) -> void:
	var overlay := MeshInstance3D.new()
	overlay.name = "DamageOverlay"
	var shell := BoxMesh.new()
	shell.size = Vector3.ONE * 1.018
	var damage_material := ShaderMaterial.new()
	damage_material.shader = BLOCK_DAMAGE_SHADER
	damage_material.set_shader_parameter("severity", clampf(1.0 - integrity, 0.0, 1.0))
	damage_material.set_shader_parameter("pattern_seed", _stable_unit_seed(block_id))
	shell.material = damage_material
	overlay.mesh = shell
	overlay.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	root.add_child(overlay)


func _stable_unit_seed(text: String) -> float:
	var value := 0
	for index in text.length():
		value = (value * 131 + text.unicode_at(index)) % 104729
	return float(value) / 104729.0


func _update_movement(delta: float) -> void:
	if Input.mouse_mode != Input.MOUSE_MODE_CAPTURED or inventory_open:
		return
	var roll_input := (
		Input.get_action_strength("roll_right") - Input.get_action_strength("roll_left")
	)
	if absf(roll_input) > 0.001:
		camera.rotate_object_local(Vector3.FORWARD, roll_input * ROLL_SPEED * delta)
		camera.transform.basis = camera.transform.basis.orthonormalized()
	var player_state: Dictionary = snapshot.get("player", {})
	var environment: Dictionary = snapshot.get("environment", {})
	var jetpack_enabled := bool(player_state.get("jetpack_enabled", true))
	var gravity := _vec3(environment.get("gravity", {}))
	var planet_center := _vec3(environment.get("planet_center", {}))
	var surface_radius := float(environment.get("surface_radius_m", 0.0))
	var radial := camera.position - planet_center
	var world_up := radial.normalized() if radial.length_squared() > 0.001 else Vector3.UP
	var ground_radius := surface_radius + PLAYER_SURFACE_CLEARANCE
	var grounded := surface_radius > 0.0 and radial.length() <= ground_radius + 0.12
	var movement_input := Vector3(
		Input.get_action_strength("move_right") - Input.get_action_strength("move_left"),
		Input.get_action_strength("move_up") - Input.get_action_strength("move_down"),
		Input.get_action_strength("move_backward") - Input.get_action_strength("move_forward")
	)
	if jetpack_enabled:
		var desired_velocity := player_velocity
		var has_movement_input := movement_input.length_squared() > 0.0
		if has_movement_input:
			movement_input = movement_input.normalized()
			var speed := MOVE_SPEED
			if Input.is_action_pressed("move_boost"):
				speed *= BOOST_MULTIPLIER
			desired_velocity = camera.basis * movement_input * speed
		player_velocity = _integrate_jetpack_velocity(
			player_velocity,
			desired_velocity,
			gravity,
			delta,
			has_movement_input,
			dampeners_enabled
		)
	else:
		var tangent_input := Vector3(movement_input.x, 0.0, movement_input.z)
		var tangent_velocity := camera.basis * tangent_input
		tangent_velocity -= world_up * tangent_velocity.dot(world_up)
		if tangent_velocity.length_squared() > 0.0:
			tangent_velocity = tangent_velocity.normalized() * WALK_SPEED
			var vertical_velocity := world_up * player_velocity.dot(world_up)
			player_velocity = player_velocity.move_toward(
				tangent_velocity + vertical_velocity, WALK_ACCELERATION * delta
			)
		else:
			var vertical_velocity := world_up * player_velocity.dot(world_up)
			player_velocity = player_velocity.move_toward(
				vertical_velocity, MOVE_DAMPING * delta
			)
		player_velocity += gravity * delta
		if grounded and Input.is_action_just_pressed("move_up"):
			player_velocity += world_up * JUMP_SPEED

	var proposed_position := camera.position + player_velocity * delta
	if surface_radius > 0.0:
		var proposed_radial := proposed_position - planet_center
		if proposed_radial.length() < ground_radius:
			var surface_up := proposed_radial.normalized()
			proposed_position = planet_center + surface_up * ground_radius
			var inward_speed := player_velocity.dot(surface_up)
			if inward_speed < 0.0:
				player_velocity -= surface_up * inward_speed
	if _position_is_clear(proposed_position):
		camera.position = proposed_position
	else:
		player_velocity *= 0.18
		tool_kick = max(tool_kick, 0.22)

	var boost_amount: float = clampf(
		player_velocity.length() / (MOVE_SPEED * BOOST_MULTIPLIER),
		0.0,
		1.0
	)
	camera.fov = lerpf(camera.fov, 74.0 + boost_amount * 8.0, minf(delta * 5.0, 1.0))

	move_send_elapsed += delta
	if (
		connected
		and move_send_elapsed >= MOVE_SEND_INTERVAL
		and camera.position.distance_squared_to(last_sent_position) > 0.0001
	):
		move_send_elapsed = 0.0
		last_sent_position = camera.position
		_send({
			"type": "move_player",
			"operation_id": _operation_id("move"),
			"position": _protocol_vec3(camera.position),
		})


func _integrate_jetpack_velocity(
	current_velocity: Vector3,
	desired_velocity: Vector3,
	gravity: Vector3,
	delta: float,
	has_movement_input: bool,
	dampeners: bool
) -> Vector3:
	var integrated := current_velocity + gravity * delta
	if has_movement_input:
		return integrated.move_toward(desired_velocity, MOVE_ACCELERATION * delta)
	if dampeners:
		return integrated.move_toward(
			Vector3.ZERO, (MOVE_DAMPING + gravity.length()) * delta
		)
	return integrated


func _position_is_clear(position: Vector3) -> bool:
	var collision_offsets: Array[Vector3] = [
		Vector3.ZERO,
		Vector3(0.32, 0.0, 0.0), Vector3(-0.32, 0.0, 0.0),
		Vector3(0.0, 0.32, 0.0), Vector3(0.0, -0.32, 0.0),
		Vector3(0.0, 0.0, 0.32), Vector3(0.0, 0.0, -0.32),
	]
	for offset in collision_offsets:
		var sample: Vector3 = position + offset
		var coordinate := Vector3i(roundi(sample.x), roundi(sample.y), roundi(sample.z))
		if voxel_lookup.has(_coord_key(coordinate)):
			return false
	return true


func _update_target() -> void:
	target_voxel = _raymarch_voxel()
	target_block = _ray_target_block()
	build_preview.visible = false
	if target_voxel != null:
		target_highlight.visible = true
		target_highlight.global_position = Vector3(target_voxel)
	elif not target_block.is_empty():
		target_highlight.visible = true
		target_highlight.global_position = target_block.get("world_position", Vector3.ZERO)
	else:
		target_highlight.visible = false
	if build_mode and not target_block.is_empty() and not _block_needs_weld(target_block["block"]):
		var grid: Dictionary = target_block["grid"]
		var grid_position := _vec3(grid.get("position", {}))
		var grid_basis := _grid_basis(grid)
		build_preview.global_position = grid_position + grid_basis * Vector3(_build_coordinate())
		build_preview.global_transform = Transform3D(
			grid_basis * Basis(Vector3.UP, deg_to_rad(float(build_rotation_quarters * 90))),
			build_preview.global_position
		)
		build_preview.visible = true


func _block_needs_weld(block: Dictionary) -> bool:
	var health := int(block.get("health", 0))
	var max_health := maxi(int(block.get("max_health", health)), 1)
	return health < max_health


func _block_condition_label(block: Dictionary) -> String:
	if not bool(block.get("construction_complete", false)):
		return "FRAME"
	if _block_needs_weld(block):
		return "DAMAGED"
	return ""


func _weld_action_name(block: Dictionary) -> String:
	var integrity := (
		int(block.get("health", 0)) * 100 / maxi(int(block.get("max_health", 1)), 1)
	)
	return (
		"REPAIRING ARMOR // %d%%" % integrity
		if bool(block.get("construction_complete", false))
		else "WELDING FRAME // %d%%" % integrity
	)


func _damage_material(root: Node3D) -> ShaderMaterial:
	var overlay := root.get_node_or_null("DamageOverlay") as MeshInstance3D
	if overlay == null:
		return null
	var shell := overlay.mesh as BoxMesh
	if shell == null:
		return null
	return shell.material as ShaderMaterial


func _run_visual_smoke_assertions() -> bool:
	var inventory_key := InputEventKey.new()
	inventory_key.keycode = KEY_I
	inventory_key.pressed = true
	var escape_key := InputEventKey.new()
	escape_key.keycode = KEY_ESCAPE
	escape_key.pressed = true
	var frame := {
		"block_id": "smoke-frame",
		"kind": "structural",
		"health": 25,
		"max_health": 100,
		"construction_complete": false,
	}
	var damaged := {
		"block_id": "smoke-damaged",
		"kind": "structural",
		"health": 65,
		"max_health": 100,
		"construction_complete": true,
	}
	var repaired := {
		"block_id": "smoke-repaired",
		"kind": "structural",
		"health": 100,
		"max_health": 100,
		"construction_complete": true,
	}
	var frame_visual := _build_block_visual(frame)
	var damaged_visual := _build_block_visual(damaged)
	var repaired_visual := _build_block_visual(repaired)
	var damage_material: ShaderMaterial = _damage_material(damaged_visual)
	var expected_seed := _stable_unit_seed("smoke-damaged")
	var gravity_probe := Vector3(0.0, -0.5, 0.0)
	var drift_velocity := _integrate_jetpack_velocity(
		Vector3.ZERO, Vector3.ZERO, gravity_probe, 0.25, false, false
	)
	var dampened_velocity := _integrate_jetpack_velocity(
		Vector3.ZERO, Vector3.ZERO, gravity_probe, 0.25, false, true
	)
	var zero_gravity_velocity := _integrate_jetpack_velocity(
		Vector3.ZERO, Vector3.ZERO, Vector3.ZERO, 0.25, false, false
	)
	var valid: bool = (
		frame_visual.get_meta("verse_visual_state", "") == "frame"
		and frame_visual.get_node_or_null("DamageOverlay") == null
		and damaged_visual.get_meta("verse_visual_state", "") == "armor_damaged"
		and damaged_visual.get_node_or_null("DamageOverlay") != null
		and damage_material != null
		and damage_material.shader == BLOCK_DAMAGE_SHADER
		and is_equal_approx(float(damage_material.get_shader_parameter("severity")), 0.35)
		and is_equal_approx(
			float(damage_material.get_shader_parameter("pattern_seed")), expected_seed
		)
		and repaired_visual.get_meta("verse_visual_state", "") == "armor_complete"
		and repaired_visual.get_node_or_null("DamageOverlay") == null
		and _block_condition_label(frame) == "FRAME"
		and _block_condition_label(damaged) == "DAMAGED"
		and _weld_action_name(frame).begins_with("WELDING FRAME")
		and _weld_action_name(damaged).begins_with("REPAIRING ARMOR")
		and not _inventory_close_shortcut(inventory_key, true)
		and _inventory_close_shortcut(inventory_key, false)
		and _inventory_close_shortcut(escape_key, true)
		and drift_velocity.is_equal_approx(gravity_probe * 0.25)
		and dampened_velocity.is_zero_approx()
		and zero_gravity_velocity.is_zero_approx()
	)
	frame_visual.free()
	damaged_visual.free()
	repaired_visual.free()
	if not valid:
		printerr("VERSE_VISUAL_STATE_FAILED")
		return false
	print(
		"VERSE_VISUAL_STATE_OK frame=frame damaged=armor_damaged repaired=armor_complete inventory_focus=owned"
	)
	print("VERSE_EVA_GRAVITY_OK drift=gravity dampeners=compensating")
	return true


func _raymarch_voxel() -> Variant:
	var origin := camera.global_position
	var direction := -camera.global_transform.basis.z.normalized()
	var last_coordinate := Vector3i(999999, 999999, 999999)
	for index in 80:
		var sample := origin + direction * (float(index) * TARGET_RANGE / 80.0)
		var coordinate := Vector3i(
			roundi(sample.x),
			roundi(sample.y),
			roundi(sample.z)
		)
		if coordinate == last_coordinate:
			continue
		last_coordinate = coordinate
		if voxel_lookup.has(_coord_key(coordinate)):
			return coordinate
	return null


func _ray_target_block() -> Dictionary:
	var origin := camera.global_position
	var direction := -camera.global_transform.basis.z.normalized()
	var best: Dictionary = {}
	var best_t := TARGET_RANGE + 1.0
	for grid_id in grid_lookup:
		var grid: Dictionary = grid_lookup[grid_id]
		var grid_position := _vec3(grid.get("position", {}))
		var grid_basis := _grid_basis(grid)
		for block in grid.get("blocks", []):
			var local := _coord_vector(block.get("coordinate", {}))
			var world := grid_position + grid_basis * local
			var to_block := world - origin
			var t := to_block.dot(direction)
			if t <= 0.0 or t > TARGET_RANGE or t >= best_t:
				continue
			var perpendicular := (to_block - direction * t).length()
			if perpendicular <= 0.68:
				best_t = t
				best = {
					"grid_id": grid_id,
					"grid": grid,
					"block": block,
					"world_position": world,
				}
	return best


func _update_tool_action(delta: float) -> void:
	if inventory_open:
		action_charge = 0.0
		action_target_key = ""
		action_progress.value = 0.0
		return
	action_cooldown = maxf(0.0, action_cooldown - delta)
	var holding_primary := Input.is_mouse_button_pressed(MOUSE_BUTTON_LEFT)
	var holding_secondary := Input.is_mouse_button_pressed(MOUSE_BUTTON_RIGHT)
	var action_key := ""
	var duration := MINE_DURATION
	var action_name := ""

	if holding_secondary and not target_block.is_empty():
		action_key = "damage:%s" % target_block["block"].get("block_id", "")
		duration = DAMAGE_DURATION
		action_name = "CUTTING ARMOR"
	elif holding_primary and build_mode and not target_block.is_empty():
		var construction_target: Dictionary = target_block["block"]
		if _block_needs_weld(construction_target):
			action_key = "weld:%s" % construction_target.get("block_id", "")
			duration = WELD_DURATION
			action_name = _weld_action_name(construction_target)
		else:
			action_key = "build:%s:%s:%d" % [
				target_block.get("grid_id", ""),
				str(_build_coordinate()),
				build_rotation_quarters,
			]
			duration = WELD_DURATION
			action_name = "PLACING %s FRAME" % selected_block_kind.to_upper()
	elif holding_primary and not build_mode and target_voxel != null:
		action_key = "mine:%s" % _coord_key(target_voxel)
		duration = MINE_DURATION
		action_name = "EXTRACTING ORE"

	if action_key.is_empty() or action_cooldown > 0.0:
		action_charge = maxf(0.0, action_charge - delta * 3.5)
		action_target_key = ""
		action_progress.value = action_charge
		return
	if action_key != action_target_key:
		action_target_key = action_key
		action_charge = 0.0
	action_charge += delta / duration
	action_progress.value = clampf(action_charge, 0.0, 1.0)
	mode_label.text = action_name
	tool_kick = maxf(tool_kick, minf(action_charge, 0.62))
	if action_charge < 1.0:
		return

	var completed_action := action_target_key
	action_charge = 0.0
	action_target_key = ""
	action_cooldown = 0.42
	tool_kick = 1.0
	if holding_secondary:
		_damage_target_block()
	elif completed_action.begins_with("weld:"):
		_weld_target_block()
	elif build_mode:
		_build_selected_block()
	else:
		_mine_target_voxel()


func _update_viewmodel(delta: float) -> void:
	tool_kick = move_toward(tool_kick, 0.0, delta * 4.2)
	var motion := clampf(player_velocity.length() / MOVE_SPEED, 0.0, 1.5)
	var bob := Vector3(
		sin(elapsed_time * 3.7) * 0.008 * motion,
		cos(elapsed_time * 6.2) * 0.007 * motion,
		tool_kick * 0.055
	)
	tool_root.position = Vector3(0.42, -0.34, -0.74) + bob
	tool_root.rotation_degrees = Vector3(
		-7.0 - tool_kick * 4.0,
		-10.0,
		2.0 + sin(elapsed_time * 3.2) * motion
	)
	tool_tip.rotation.z += delta * (18.0 if action_charge > 0.0 else 1.2)
	tool_light.light_energy = lerpf(
		tool_light.light_energy,
		5.0 if action_charge > 0.0 else 0.0,
		minf(delta * 14.0, 1.0)
	)
	if build_preview.visible:
		build_preview.scale = Vector3.ONE * (1.0 + sin(elapsed_time * 4.5) * 0.012)
	_update_action_feedback()


func _update_action_feedback() -> void:
	var active := action_charge > 0.0 and not action_target_key.is_empty()
	action_beam.visible = active
	action_flare.visible = active
	action_sparks.emitting = active
	if not active:
		return
	var target_position := _active_action_position()
	var beam_origin := tool_tip.global_position
	var beam_vector := target_position - beam_origin
	if beam_vector.length_squared() < 0.001:
		action_beam.visible = false
		return
	action_beam.global_position = (beam_origin + target_position) * 0.5
	action_beam.global_basis = Basis(Quaternion(Vector3.UP, beam_vector.normalized())).scaled(
		Vector3(1.0, beam_vector.length(), 1.0)
	)
	action_flare.global_position = target_position
	action_sparks.global_position = target_position
	var pulse := 0.72 + sin(elapsed_time * 38.0) * 0.22 + action_charge * 0.42
	action_flare.scale = Vector3.ONE * pulse


func _active_action_position() -> Vector3:
	if action_target_key.begins_with("mine:") and target_voxel != null:
		return Vector3(target_voxel)
	if action_target_key.begins_with("weld:") and not target_block.is_empty():
		return target_block.get("world_position", camera.global_position - camera.basis.z * 2.0)
	if action_target_key.begins_with("build:"):
		return build_preview.global_position
	if not target_block.is_empty():
		return target_block.get("world_position", camera.global_position - camera.basis.z * 2.0)
	return camera.global_position - camera.basis.z * 2.0


func _mine_target_voxel() -> void:
	if target_voxel == null:
		_set_message("Aim at an asteroid voxel within mining range", true)
		return
	pending_mine_position = target_voxel
	_send({
		"type": "mine_voxel",
		"operation_id": _operation_id("mine"),
		"coordinate": _protocol_ivec3(target_voxel),
	})


func _damage_target_block() -> void:
	if target_block.is_empty():
		_set_message("Aim at a grid block to apply test damage", true)
		return
	var block: Dictionary = target_block["block"]
	_send({
		"type": "damage_block",
		"operation_id": _operation_id("damage"),
		"grid_id": target_block["grid_id"],
		"block_id": block.get("block_id", ""),
	})


func _build_selected_block() -> void:
	if target_block.is_empty():
		_set_message("Aim at a grid block before building", true)
		return
	var coordinate := _build_coordinate()
	_send({
		"type": "build_block",
		"operation_id": _operation_id("build"),
		"grid_id": target_block["grid_id"],
		"coordinate": _protocol_ivec3(coordinate),
		"kind": selected_block_kind,
		"orientation": build_rotation_quarters,
	})


func _weld_target_block() -> void:
	if target_block.is_empty():
		_set_message("Aim at an unfinished frame or damaged block", true)
		return
	var block: Dictionary = target_block["block"]
	if not _block_needs_weld(block):
		_set_message("Target block is already at full integrity", true)
		return
	_send({
		"type": "weld_block",
		"operation_id": _operation_id("weld"),
		"grid_id": target_block["grid_id"],
		"block_id": block.get("block_id", ""),
	})


func _build_coordinate() -> Vector3i:
	if target_block.is_empty():
		return Vector3i.ZERO
	var grid: Dictionary = target_block["grid"]
	var block: Dictionary = target_block["block"]
	var current := _coord_i(block.get("coordinate", {}))
	var offset: Vector3i
	if selected_block_kind == "anchor":
		var grid_position := _vec3(grid.get("position", {}))
		var basis := _grid_basis(grid)
		var toward_asteroid := basis.inverse() * (-grid_position)
		offset = _dominant_axis(toward_asteroid)
	else:
		var basis := _grid_basis(grid)
		var toward_camera: Vector3 = basis.inverse() * (
			camera.global_position - target_block.get("world_position", Vector3.ZERO)
		)
		offset = _dominant_axis(toward_camera)
	return current + offset


func _toggle_anchor() -> void:
	var grid_id := _target_or_starter_grid()
	_send({
		"type": "toggle_grid_anchor",
		"operation_id": _operation_id("anchor"),
		"grid_id": grid_id,
	})


func _move_target_grid() -> void:
	var grid_id := _target_or_starter_grid()
	var direction := -camera.global_transform.basis.z.normalized()
	var grid: Dictionary = grid_lookup.get(grid_id, {})
	var local_direction := (_grid_basis(grid).inverse() * direction).limit_length(0.999)
	grid_control_active = true
	_send({
		"type": "set_grid_control",
		"operation_id": _operation_id("grid-control"),
		"grid_id": grid_id,
		"linear_input": _protocol_vec3(local_direction),
		"angular_input": _protocol_vec3(Vector3(0.0, 0.24, 0.0)),
		"dampeners": true,
	})


func _stop_target_grid() -> void:
	grid_control_active = false
	_send({
		"type": "set_grid_control",
		"operation_id": _operation_id("grid-stop"),
		"grid_id": _target_or_starter_grid(),
		"linear_input": _protocol_vec3(Vector3.ZERO),
		"angular_input": _protocol_vec3(Vector3.ZERO),
		"dampeners": true,
	})


func _refine_ore() -> void:
	_send({
		"type": "refine_ore",
		"operation_id": _operation_id("refine"),
		"inventory_id": PLAYER_INVENTORY,
		"batches": 1,
	})


func _craft_component() -> void:
	_send({
		"type": "craft_component",
		"operation_id": _operation_id("craft"),
		"inventory_id": PLAYER_INVENTORY,
		"quantity": 1,
	})


func _transfer_to_or_from_cargo(reverse: bool) -> void:
	var cargo_id := _first_cargo_inventory()
	if cargo_id.is_empty():
		_set_message("No live cargo inventory is available", true)
		return
	_send({
		"type": "transfer_inventory",
		"operation_id": _operation_id("transfer"),
		"source_inventory_id": cargo_id if reverse else PLAYER_INVENTORY,
		"destination_inventory_id": PLAYER_INVENTORY if reverse else cargo_id,
		"resource": "ore",
		"quantity": 1,
	})


func _set_inventory_open(open: bool) -> void:
	inventory_open = open
	inventory_overlay.visible = open
	build_mode = false if open else build_mode
	action_charge = 0.0
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE if open else Input.MOUSE_MODE_CAPTURED
	_set_message(
		"Engineering inventory terminal online" if open else "Engineering terminal closed"
	)


func _toggle_jetpack() -> void:
	var player: Dictionary = snapshot.get("player", {})
	_send({
		"type": "set_suit_mode",
		"operation_id": _operation_id("suit"),
		"helmet_closed": bool(player.get("helmet_closed", true)),
		"jetpack_enabled": not bool(player.get("jetpack_enabled", true)),
	})


func _toggle_helmet() -> void:
	var player: Dictionary = snapshot.get("player", {})
	_send({
		"type": "set_suit_mode",
		"operation_id": _operation_id("suit"),
		"helmet_closed": not bool(player.get("helmet_closed", true)),
		"jetpack_enabled": bool(player.get("jetpack_enabled", true)),
	})


func _transfer_inventory_resource(resource: String, reverse: bool, all: bool) -> void:
	var cargo_id := _first_cargo_inventory()
	if cargo_id.is_empty():
		_set_message("No live cargo inventory is available", true)
		return
	var source_id := cargo_id if reverse else PLAYER_INVENTORY
	var destination_id := PLAYER_INVENTORY if reverse else cargo_id
	var quantity := 1
	if all:
		quantity = _resource_amount(_inventory(source_id).get("contents", {}), resource)
	if quantity <= 0:
		_set_message("The selected source stack is empty", true)
		return
	_send({
		"type": "transfer_inventory",
		"operation_id": _operation_id("terminal-transfer"),
		"source_inventory_id": source_id,
		"destination_inventory_id": destination_id,
		"resource": resource,
		"quantity": quantity,
	})


func _resource_amount(contents: Dictionary, resource: String) -> int:
	match resource:
		"ore":
			return int(contents.get("ore", 0))
		"refined_material":
			return int(contents.get("refined_material", 0))
		"component":
			return int(contents.get("components", 0))
	return 0


func _first_cargo_inventory() -> String:
	for inventory in snapshot.get("inventories", []):
		var domain: Dictionary = inventory.get("domain", {})
		if domain.get("kind", "") == "cargo":
			return inventory.get("inventory_id", "")
	return ""


func _target_or_starter_grid() -> String:
	if not target_block.is_empty():
		return target_block.get("grid_id", STARTER_GRID)
	if grid_lookup.has(STARTER_GRID):
		return STARTER_GRID
	if not grid_lookup.is_empty():
		return grid_lookup.keys()[0]
	return STARTER_GRID


func _send(message: Dictionary) -> void:
	if socket.get_ready_state() != WebSocketPeer.STATE_OPEN:
		_set_message("No authoritative server connection — press F5", true)
		return
	var error := socket.send_text(JSON.stringify(message))
	if error != OK:
		_set_message("Network send failed: %s" % error_string(error), true)


func _operation_id(prefix: String) -> String:
	operation_counter += 1
	return "%s-%d-%d" % [prefix, Time.get_ticks_usec(), operation_counter]


func _update_interface() -> void:
	connection_label.text = (
		"● LINKED // ORIGIN CELL"
		if connected
		else "○ RELAY OFFLINE // F5 TO RETRY"
	)
	connection_label.add_theme_color_override(
		"font_color",
		Color(0.35, 0.95, 0.62) if connected else Color(1.0, 0.38, 0.25)
	)
	var player: Dictionary = snapshot.get("player", {})
	var level := int(player.get("level", 1))
	var experience := int(player.get("experience", 0))
	var next_level := int(player.get("next_level_experience", 100))
	level_label.text = "SALVAGER // LEVEL %d     REP %d / %d" % [level, experience, next_level]
	var suit_power := clampi(100 - roundi(player_velocity.length() * 1.4), 72, 100)
	var oxygen_percent := int(player.get("suit_oxygen_milli", 1000)) / 10
	var helmet_state := "SEALED" if player.get("helmet_closed", true) else "OPEN"
	var jetpack_state := "JET" if player.get("jetpack_enabled", true) else "WALK"
	var dampener_state := "DAMP" if dampeners_enabled else "DRIFT"
	telemetry_label.text = "O₂ %03d%%   PWR %03d%%   %s   %s   %s" % [
		oxygen_percent, suit_power, helmet_state, jetpack_state, dampener_state
	]
	telemetry_label.add_theme_color_override(
		"font_color", Color(1.0, 0.32, 0.18) if oxygen_percent < 20 else Color(0.64, 0.90, 0.94)
	)
	var environment: Dictionary = snapshot.get("environment", {})
	var gravity_g := float(environment.get("gravity_m_s2", 0.0)) / 9.80665
	var atmosphere_percent := roundi(float(environment.get("atmosphere_density", 0.0)) * 100.0)
	status_label.text = (
		"%s  //  ALT %.1f m\nGRAV %.2f g    ATM %03d%%    %s\nEVENT %d    LEDGER %s    %s"
		% [
			String(environment.get("celestial_body_name", "DEEP SPACE")).to_upper(),
			float(environment.get("altitude_m", 0.0)),
			gravity_g,
			atmosphere_percent,
			"BREATHABLE" if environment.get("breathable", false) else "VACUUM",
			int(snapshot.get("event_sequence", 0)),
			"CONSERVED" if snapshot.get("conservation", {}).get("valid", false) else "FAULT",
			String(snapshot.get("world_hash", "—")).left(8),
		]
	)
	var player_inventory := _inventory(PLAYER_INVENTORY)
	var contents: Dictionary = player_inventory.get("contents", {})
	var conserved: Dictionary = snapshot.get("conservation", {})
	inventory_label.text = (
		"CARGO HARNESS  //  %d / %d L\nORE  %03d     ALLOY  %03d     PARTS  %03d\n[I] OPEN LOGISTICS TERMINAL"
	) % [
		int(player_inventory.get("used_liters", 0)),
		int(player_inventory.get("capacity_liters", 0)),
		int(contents.get("ore", 0)),
		int(contents.get("refined_material", 0)),
		int(contents.get("components", 0)),
	]
	inventory_label.add_theme_color_override(
		"font_color",
		Color(0.95, 0.71, 0.27)
		if conserved.get("valid", false)
		else Color(1.0, 0.18, 0.18)
	)
	_update_inventory_terminal()
	var career: Dictionary = player.get("career", {})
	mission_label.text = _mission_text(career)
	if build_mode:
		var choices := [
			["1", "FRAME", "structural"],
			["2", "ANCHOR", "anchor"],
			["3", "CARGO", "cargo"],
			["4", "POWER", "power_source"],
			["5", "BREACH", "damage_test"],
		]
		var hotbar_parts: Array[String] = []
		for choice in choices:
			var text := "[%s] %s" % [choice[0], choice[1]]
			if choice[2] == selected_block_kind:
				text = "▶ " + text + " ◀"
			hotbar_parts.append(text)
		hotbar_label.text = "%s     ROT %03d°  [ / ]" % [
			"     ".join(hotbar_parts), build_rotation_quarters * 90
		]
	else:
		hotbar_label.text = "HAND DRILL     [Q/E] ROLL     [B] CONSTRUCTION     [R] REFINE     [T] FABRICATE     [V] CARGO"
	if target_voxel != null:
		var voxel: Dictionary = voxel_lookup.get(_coord_key(target_voxel), {})
		var deposit := (
			"FERRITE DEPOSIT // HIGH YIELD"
			if voxel.get("material", "rock") == "ferrite_ore"
			else "CARBONACEOUS ROCK // LOW YIELD"
		)
		target_label.text = "%s\nHOLD LMB  //  EXTRACT" % deposit
	elif not target_block.is_empty():
		var block: Dictionary = target_block["block"]
		var health := int(block.get("health", 0))
		var max_health := maxi(int(block.get("max_health", health)), 1)
		var integrity := health * 100 / max_health
		if build_mode:
			if health < max_health:
				if bool(block.get("construction_complete", false)):
					target_label.text = "%s DAMAGED // INTEGRITY %d%%\nHOLD LMB  //  REPAIR" % [
						String(block.get("kind", "block")).to_upper(), integrity
					]
				else:
					target_label.text = "%s FRAME // INTEGRITY %d%%\nHOLD LMB  //  CONTINUE WELD" % [
						String(block.get("kind", "block")).to_upper(), integrity
					]
			else:
				target_label.text = "%s // INTEGRITY 100%%\nHOLD LMB  //  PLACE %s  //  [ / ] ROTATE" % [
					String(block.get("kind", "block")).to_upper(),
					selected_block_kind.to_upper(),
				]
		else:
			var condition := _block_condition_label(block)
			var target_state := "" if condition.is_empty() else " " + condition
			target_label.text = "%s%s // INTEGRITY %d%%\nHOLD RMB  //  CUT AND SALVAGE" % [
				String(block.get("kind", "block")).to_upper(), target_state, integrity
			]
	else:
		target_label.text = (
			"CONSTRUCTION MODE // AIM AT A GRID BLOCK"
			if build_mode
			else "EVA NAVIGATION // AIM AT ROCK OR MACHINERY"
		)
	if grid_control_active:
		mode_label.text = "GRID CONTROL ACTIVE // RELEASE M OR PRESS X TO DAMPEN"
	elif action_charge <= 0.0:
		mode_label.text = (
			"CONSTRUCTION HOLOGRAM // %s // ROT %03d°" % [
				selected_block_kind.to_upper(), build_rotation_quarters * 90
			]
			if build_mode
			else "INDUSTRIAL HAND DRILL // READY"
		)
	action_progress.visible = action_charge > 0.0
	message_label.text = recent_message
	message_label.add_theme_color_override("font_color", recent_message_color)


func _update_inventory_terminal() -> void:
	var suit_inventory := _inventory(PLAYER_INVENTORY)
	var cargo_inventory := _inventory(_first_cargo_inventory())
	for side_data in [["suit", suit_inventory], ["cargo", cargo_inventory]]:
		var side := String(side_data[0])
		var inventory: Dictionary = side_data[1]
		var contents: Dictionary = inventory.get("contents", {})
		for resource in ["ore", "refined_material", "component"]:
			var quantity := _resource_amount(contents, resource)
			var quantity_label: Label = inventory_item_labels.get(
				"%s:%s:amount" % [side, resource], null
			)
			if quantity_label != null:
				quantity_label.text = str(quantity)
			var volume_data: Array = inventory_item_labels.get(
				"%s:%s:volume" % [side, resource], []
			)
			if volume_data.size() == 2:
				var volume_label: Label = volume_data[0]
				volume_label.text = "%d L" % (quantity * int(volume_data[1]))
			var mass_data: Array = inventory_item_labels.get(
				"%s:%s:mass" % [side, resource], []
			)
			if mass_data.size() == 2:
				var mass_label: Label = mass_data[0]
				mass_label.text = "%.1f kg" % (float(quantity) * float(mass_data[1]))
		var capacity := maxi(int(inventory.get("capacity_liters", 0)), 1)
		var used := int(inventory.get("used_liters", 0))
		var mass_kg := float(inventory.get("mass_grams", 0)) / 1000.0
		var capacity_label: Label = inventory_capacity_labels.get(side, null)
		if capacity_label != null:
			capacity_label.text = "%d / %d L  //  %.1f kg" % [used, capacity, mass_kg]
		var capacity_bar: ProgressBar = inventory_capacity_bars.get(side, null)
		if capacity_bar != null:
			capacity_bar.value = clampf(float(used) / float(capacity), 0.0, 1.0)


func _mission_text(career: Dictionary) -> String:
	var mined := int(career.get("voxels_mined", 0))
	var refined := int(career.get("refining_batches", 0))
	var crafted := int(career.get("components_crafted", 0))
	var built := int(career.get("blocks_built", 0))
	var anchored := int(career.get("anchors_engaged", 0))
	if mined < 3:
		return (
			"01 // CUT A PATH\n"
			+ "The relay rig is awake, but its stores are dry.\n\n"
			+ "Extract asteroid voxels  %d / 3\n"
			+ "Hold LMB on highlighted rock."
		) % mined
	if refined < 1:
		return (
			"02 // SMELT FEEDSTOCK\n"
			+ "Turn raw ore into registered alloy.\n\n"
			+ "Refining batches  0 / 1\n"
			+ "Press R when carrying at least 2 ore."
		)
	if crafted < 1:
		return (
			"03 // FABRICATE A PART\n"
			+ "Prove the production chain can sustain the rig.\n\n"
			+ "Components fabricated  0 / 1\n"
			+ "Press T with refined alloy."
		)
	if built < 2:
		return (
			"04 // EXPAND THE RELAY\n"
			+ "Add a frame, then an anchor toward the rock.\n\n"
			+ "Blocks constructed  %d / 2\n"
			+ "Press B, select 1 or 2, rotate with [ or ], then hold LMB to place and weld."
		) % built
	if anchored < 1:
		return (
			"05 // LOCK THE RIG\n"
			+ "Seat an anchor against the asteroid and energize it.\n\n"
			+ "Anchor engagements  0 / 1\n"
			+ "Press F while targeting the rig."
		)
	return (
		"CONTRACT COMPLETE // RELAY ONLINE\n"
		+ "Khepri Station recognizes your salvage license.\n\n"
		+ "Continue mining, design a larger grid, or cut the test frame apart."
	)


func _inventory(inventory_id: String) -> Dictionary:
	for inventory in snapshot.get("inventories", []):
		if inventory.get("inventory_id", "") == inventory_id:
			return inventory
	return {}


func _set_message(message: String, is_error := false) -> void:
	recent_message = message
	recent_message_color = Color(1.0, 0.40, 0.28) if is_error else Color(0.56, 0.87, 1.0)


func _dominant_axis(direction: Vector3) -> Vector3i:
	var absolute := direction.abs()
	if absolute.x >= absolute.y and absolute.x >= absolute.z:
		return Vector3i(1 if direction.x >= 0.0 else -1, 0, 0)
	if absolute.y >= absolute.x and absolute.y >= absolute.z:
		return Vector3i(0, 1 if direction.y >= 0.0 else -1, 0)
	return Vector3i(0, 0, 1 if direction.z >= 0.0 else -1)


func _coord_key(coordinate: Vector3i) -> String:
	return "%d,%d,%d" % [coordinate.x, coordinate.y, coordinate.z]


func _coord_i(value: Dictionary) -> Vector3i:
	return Vector3i(
		int(value.get("x", 0)),
		int(value.get("y", 0)),
		int(value.get("z", 0))
	)


func _coord_vector(value: Dictionary) -> Vector3:
	return Vector3(
		float(value.get("x", 0)),
		float(value.get("y", 0)),
		float(value.get("z", 0))
	)


func _vec3(value: Dictionary) -> Vector3:
	return Vector3(
		float(value.get("x", 0.0)),
		float(value.get("y", 0.0)),
		float(value.get("z", 0.0))
	)


func _grid_quaternion(grid: Dictionary) -> Quaternion:
	var value: Dictionary = grid.get("orientation", {})
	var rotation := Quaternion(
		float(value.get("x", 0.0)),
		float(value.get("y", 0.0)),
		float(value.get("z", 0.0)),
		float(value.get("w", 1.0))
	)
	return rotation.normalized() if rotation.length_squared() > 0.000001 else Quaternion.IDENTITY


func _grid_basis(grid: Dictionary) -> Basis:
	return Basis(_grid_quaternion(grid))


func _protocol_vec3(value: Vector3) -> Dictionary:
	return {"x": value.x, "y": value.y, "z": value.z}


func _protocol_ivec3(value: Vector3i) -> Dictionary:
	return {"x": value.x, "y": value.y, "z": value.z}
