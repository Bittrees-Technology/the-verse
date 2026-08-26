# SPDX-License-Identifier: AGPL-3.0-or-later
extends Node3D

const PROTOCOL_VERSION := 1
const DEFAULT_SERVER := "ws://127.0.0.1:7777/ws"
const PLAYER_INVENTORY := "inventory-player-local"
const STARTER_GRID := "grid-starter"
const MOVE_SPEED := 6.0
const BOOST_MULTIPLIER := 3.0
const MOUSE_SENSITIVITY := 0.0022
const MOVE_SEND_INTERVAL := 0.10
const TARGET_RANGE := 9.0

var socket := WebSocketPeer.new()
var server_url := DEFAULT_SERVER
var connected := false
var handshake_sent := false
var operation_counter := 0
var move_send_elapsed := 0.0
var first_snapshot := true
var snapshot: Dictionary = {}
var voxel_lookup: Dictionary = {}
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

var camera: Camera3D
var asteroid_root: Node3D
var grids_root: Node3D
var stars_root: Node3D
var target_highlight: MeshInstance3D
var status_label: Label
var inventory_label: Label
var target_label: Label
var message_label: Label
var connection_label: Label
var selected_label: Label

var rock_material: StandardMaterial3D
var ore_material: StandardMaterial3D
var block_materials: Dictionary = {}


func _ready() -> void:
	_parse_command_line()
	_register_inputs()
	_build_environment()
	_build_interface()
	Input.mouse_mode = Input.MOUSE_MODE_CAPTURED
	_connect_to_server()


func _exit_tree() -> void:
	socket.close()


func _process(delta: float) -> void:
	_poll_socket()
	_update_movement(delta)
	_update_target()
	_update_interface()


func _unhandled_input(event: InputEvent) -> void:
	if event is InputEventMouseMotion and Input.mouse_mode == Input.MOUSE_MODE_CAPTURED:
		camera.rotation.y -= event.relative.x * MOUSE_SENSITIVITY
		camera.rotation.x = clamp(
			camera.rotation.x - event.relative.y * MOUSE_SENSITIVITY,
			-deg_to_rad(89.0),
			deg_to_rad(89.0)
		)
		return

	if event is InputEventKey and event.pressed and not event.echo:
		match event.keycode:
			KEY_ESCAPE:
				Input.mouse_mode = (
					Input.MOUSE_MODE_VISIBLE
					if Input.mouse_mode == Input.MOUSE_MODE_CAPTURED
					else Input.MOUSE_MODE_CAPTURED
				)
			KEY_1:
				selected_block_kind = "structural"
			KEY_2:
				selected_block_kind = "anchor"
			KEY_3:
				selected_block_kind = "cargo"
			KEY_4:
				selected_block_kind = "power_source"
			KEY_5:
				selected_block_kind = "damage_test"
			KEY_B:
				_build_selected_block()
			KEY_F:
				_toggle_anchor()
			KEY_M:
				_move_target_grid()
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
			KEY_F5:
				_connect_to_server()

	if event is InputEventMouseButton and event.pressed:
		if Input.mouse_mode != Input.MOUSE_MODE_CAPTURED:
			Input.mouse_mode = Input.MOUSE_MODE_CAPTURED
			return
		if event.button_index == MOUSE_BUTTON_LEFT:
			_mine_target_voxel()
		elif event.button_index == MOUSE_BUTTON_RIGHT:
			_damage_target_block()


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
	environment.background_color = Color(0.004, 0.007, 0.015)
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.ambient_light_color = Color(0.20, 0.28, 0.42)
	environment.ambient_light_energy = 0.52
	environment.tonemap_mode = Environment.TONE_MAPPER_FILMIC
	world_environment.environment = environment
	add_child(world_environment)

	var key_light := DirectionalLight3D.new()
	key_light.rotation_degrees = Vector3(-38.0, -52.0, 0.0)
	key_light.light_color = Color(0.72, 0.84, 1.0)
	key_light.light_energy = 1.35
	key_light.shadow_enabled = true
	add_child(key_light)

	var rim_light := DirectionalLight3D.new()
	rim_light.rotation_degrees = Vector3(28.0, 132.0, 0.0)
	rim_light.light_color = Color(1.0, 0.36, 0.16)
	rim_light.light_energy = 0.55
	add_child(rim_light)

	camera = Camera3D.new()
	camera.current = true
	camera.fov = 76.0
	camera.near = 0.05
	camera.far = 1500.0
	camera.position = Vector3(10.0, 2.0, 4.0)
	add_child(camera)
	camera.look_at(Vector3.ZERO, Vector3.UP)

	asteroid_root = Node3D.new()
	asteroid_root.name = "AuthoritativeAsteroid"
	add_child(asteroid_root)
	grids_root = Node3D.new()
	grids_root.name = "AuthoritativeGrids"
	add_child(grids_root)
	stars_root = Node3D.new()
	stars_root.name = "Starfield"
	add_child(stars_root)

	rock_material = _material(Color(0.18, 0.23, 0.29), 0.92, 0.04)
	ore_material = _material(Color(0.66, 0.26, 0.12), 0.72, 0.36)
	block_materials = {
		"structural": _material(Color(0.33, 0.40, 0.47), 0.58, 0.48),
		"control_core": _material(Color(0.12, 0.58, 0.75), 0.42, 0.44),
		"power_source": _material(Color(0.93, 0.50, 0.10), 0.36, 0.36),
		"battery": _material(Color(0.83, 0.70, 0.18), 0.42, 0.32),
		"cargo": _material(Color(0.22, 0.48, 0.31), 0.65, 0.18),
		"drill": _material(Color(0.72, 0.22, 0.16), 0.48, 0.55),
		"anchor": _material(Color(0.55, 0.30, 0.75), 0.50, 0.45),
		"damage_test": _material(Color(0.72, 0.12, 0.16), 0.64, 0.22),
	}
	_build_starfield()
	_build_target_highlight()


func _material(color: Color, roughness: float, metallic: float) -> StandardMaterial3D:
	var material := StandardMaterial3D.new()
	material.albedo_color = color
	material.roughness = roughness
	material.metallic = metallic
	return material


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


func _build_interface() -> void:
	var canvas := CanvasLayer.new()
	add_child(canvas)

	var top_bar := ColorRect.new()
	top_bar.color = Color(0.025, 0.042, 0.065, 0.94)
	top_bar.set_anchors_and_offsets_preset(Control.PRESET_TOP_WIDE)
	top_bar.custom_minimum_size.y = 54.0
	canvas.add_child(top_bar)

	var title := Label.new()
	title.text = "THE VERSE  //  P0 INDUSTRIAL PROOF"
	title.position = Vector2(22.0, 15.0)
	title.add_theme_font_size_override("font_size", 20)
	title.add_theme_color_override("font_color", Color(0.72, 0.90, 1.0))
	top_bar.add_child(title)

	connection_label = Label.new()
	connection_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	connection_label.position = Vector2(980.0, 17.0)
	connection_label.size = Vector2(430.0, 28.0)
	top_bar.add_child(connection_label)

	var left_panel := ColorRect.new()
	left_panel.color = Color(0.018, 0.029, 0.045, 0.86)
	left_panel.position = Vector2(18.0, 72.0)
	left_panel.size = Vector2(350.0, 228.0)
	canvas.add_child(left_panel)

	status_label = Label.new()
	status_label.position = Vector2(16.0, 14.0)
	status_label.size = Vector2(320.0, 78.0)
	status_label.add_theme_font_size_override("font_size", 14)
	left_panel.add_child(status_label)

	inventory_label = Label.new()
	inventory_label.position = Vector2(16.0, 92.0)
	inventory_label.size = Vector2(320.0, 62.0)
	inventory_label.add_theme_color_override("font_color", Color(0.95, 0.71, 0.27))
	left_panel.add_child(inventory_label)

	selected_label = Label.new()
	selected_label.position = Vector2(16.0, 160.0)
	selected_label.size = Vector2(320.0, 24.0)
	left_panel.add_child(selected_label)

	target_label = Label.new()
	target_label.position = Vector2(16.0, 188.0)
	target_label.size = Vector2(320.0, 28.0)
	target_label.add_theme_color_override("font_color", Color(0.48, 0.86, 1.0))
	left_panel.add_child(target_label)

	var help_panel := ColorRect.new()
	help_panel.color = Color(0.018, 0.029, 0.045, 0.82)
	help_panel.set_anchors_preset(Control.PRESET_TOP_RIGHT)
	help_panel.position = Vector2(-390.0, 72.0)
	help_panel.size = Vector2(370.0, 360.0)
	canvas.add_child(help_panel)
	var help := Label.new()
	help.position = Vector2(16.0, 14.0)
	help.size = Vector2(340.0, 340.0)
	help.text = (
		"FLIGHT\n"
		+ "WASD move  •  Space/C vertical  •  Shift boost\n"
		+ "Mouse look  •  Esc release cursor\n\n"
		+ "INDUSTRY\n"
		+ "Left click mine voxel  •  R refine 2 ore\n"
		+ "T craft component  •  V move 1 ore to cargo\n"
		+ "Shift+V move 1 ore back\n\n"
		+ "CONSTRUCTION\n"
		+ "1 structure  2 anchor  3 cargo  4 power  5 test\n"
		+ "Aim at grid + B build  •  Right click damage\n"
		+ "F anchor/release  •  M move grid  •  X stop\n\n"
		+ "SYSTEM\n"
		+ "P resync  •  F5 reconnect"
	)
	help.add_theme_font_size_override("font_size", 14)
	help.add_theme_color_override("font_color", Color(0.74, 0.81, 0.88))
	help_panel.add_child(help)

	var crosshair := Label.new()
	crosshair.text = "+"
	crosshair.set_anchors_and_offsets_preset(Control.PRESET_CENTER)
	crosshair.position -= Vector2(9.0, 18.0)
	crosshair.add_theme_font_size_override("font_size", 28)
	crosshair.add_theme_color_override("font_color", Color(0.65, 0.92, 1.0))
	canvas.add_child(crosshair)

	var bottom_bar := ColorRect.new()
	bottom_bar.color = Color(0.025, 0.042, 0.065, 0.92)
	bottom_bar.set_anchors_and_offsets_preset(Control.PRESET_BOTTOM_WIDE)
	bottom_bar.position.y -= 56.0
	bottom_bar.custom_minimum_size.y = 56.0
	canvas.add_child(bottom_bar)
	message_label = Label.new()
	message_label.position = Vector2(22.0, 17.0)
	message_label.size = Vector2(1350.0, 28.0)
	message_label.add_theme_font_size_override("font_size", 15)
	bottom_bar.add_child(message_label)


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
				"client_name": "godot-native-p0",
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
			camera.look_at(Vector3.ZERO, Vector3.UP)
	first_snapshot = false

	var voxels: Array = snapshot.get("voxels", [])
	if voxels.size() != rendered_voxel_count:
		rendered_voxel_count = voxels.size()
		_rebuild_voxels(voxels)
	_rebuild_grids(snapshot.get("grids", []))
	if smoke_test and smoke_operation.is_empty():
		smoke_operation = _operation_id("godot-smoke")
		_send({
			"type": "move_player",
			"operation_id": smoke_operation,
			"position": _protocol_vec3(position),
		})


func _rebuild_voxels(voxels: Array) -> void:
	for child in asteroid_root.get_children():
		child.queue_free()
	voxel_lookup.clear()
	var rock_positions: Array[Vector3] = []
	var ore_positions: Array[Vector3] = []
	for voxel in voxels:
		var coordinate: Dictionary = voxel.get("coordinate", {})
		var grid_position := Vector3i(
			int(coordinate.get("x", 0)),
			int(coordinate.get("y", 0)),
			int(coordinate.get("z", 0))
		)
		voxel_lookup[_coord_key(grid_position)] = voxel
		if voxel.get("material", "rock") == "ferrite_ore":
			ore_positions.append(Vector3(grid_position))
		else:
			rock_positions.append(Vector3(grid_position))
	_add_voxel_multimesh(rock_positions, rock_material, "RockVoxels")
	_add_voxel_multimesh(ore_positions, ore_material, "FerriteVoxels")


func _add_voxel_multimesh(
	positions: Array[Vector3],
	material: Material,
	node_name: String
) -> void:
	if positions.is_empty():
		return
	var mesh := BoxMesh.new()
	mesh.size = Vector3.ONE * 0.96
	mesh.material = material
	var multimesh := MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_3D
	multimesh.mesh = mesh
	multimesh.instance_count = positions.size()
	for index in positions.size():
		multimesh.set_instance_transform(index, Transform3D(Basis(), positions[index]))
	var instance := MultiMeshInstance3D.new()
	instance.name = node_name
	instance.multimesh = multimesh
	asteroid_root.add_child(instance)


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
		grid_node.rotation.y = float(grid.get("yaw_radians", 0.0))
		for block in grid.get("blocks", []):
			var mesh_instance := MeshInstance3D.new()
			mesh_instance.name = block.get("block_id", "block")
			var mesh := BoxMesh.new()
			mesh.size = Vector3.ONE * 0.94
			mesh.material = block_materials.get(
				block.get("kind", "structural"),
				block_materials["structural"]
			)
			mesh_instance.mesh = mesh
			var coordinate: Dictionary = block.get("coordinate", {})
			mesh_instance.position = Vector3(
				float(coordinate.get("x", 0)),
				float(coordinate.get("y", 0)),
				float(coordinate.get("z", 0))
			)
			grid_node.add_child(mesh_instance)
		grids_root.add_child(grid_node)


func _update_movement(delta: float) -> void:
	if Input.mouse_mode != Input.MOUSE_MODE_CAPTURED:
		return
	var input := Vector3(
		Input.get_action_strength("move_right") - Input.get_action_strength("move_left"),
		Input.get_action_strength("move_up") - Input.get_action_strength("move_down"),
		Input.get_action_strength("move_backward") - Input.get_action_strength("move_forward")
	)
	if input.length_squared() > 0.0:
		input = input.normalized()
		var speed := MOVE_SPEED
		if Input.is_action_pressed("move_boost"):
			speed *= BOOST_MULTIPLIER
		camera.position += camera.basis * input * speed * delta

	move_send_elapsed += delta
	if connected and move_send_elapsed >= MOVE_SEND_INTERVAL:
		move_send_elapsed = 0.0
		_send({
			"type": "move_player",
			"operation_id": _operation_id("move"),
			"position": _protocol_vec3(camera.position),
		})


func _update_target() -> void:
	target_voxel = _raymarch_voxel()
	target_block = _ray_target_block()
	if target_voxel != null:
		target_highlight.visible = true
		target_highlight.global_position = Vector3(target_voxel)
	elif not target_block.is_empty():
		target_highlight.visible = true
		target_highlight.global_position = target_block.get("world_position", Vector3.ZERO)
	else:
		target_highlight.visible = false


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
		var grid_basis := Basis(Vector3.UP, float(grid.get("yaw_radians", 0.0)))
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


func _mine_target_voxel() -> void:
	if target_voxel == null:
		_set_message("Aim at an asteroid voxel within mining range", true)
		return
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
	var grid: Dictionary = target_block["grid"]
	var block: Dictionary = target_block["block"]
	var current := _coord_i(block.get("coordinate", {}))
	var offset: Vector3i
	if selected_block_kind == "anchor":
		var grid_position := _vec3(grid.get("position", {}))
		var basis := Basis(Vector3.UP, float(grid.get("yaw_radians", 0.0)))
		var toward_asteroid := basis.inverse() * (-grid_position)
		offset = _dominant_axis(toward_asteroid)
	else:
		var basis := Basis(Vector3.UP, float(grid.get("yaw_radians", 0.0)))
		var toward_camera: Vector3 = basis.inverse() * (
			camera.global_position - target_block.get("world_position", Vector3.ZERO)
		)
		offset = _dominant_axis(toward_camera)
	var coordinate := current + offset
	_send({
		"type": "build_block",
		"operation_id": _operation_id("build"),
		"grid_id": target_block["grid_id"],
		"coordinate": _protocol_ivec3(coordinate),
		"kind": selected_block_kind,
	})


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
	_send({
		"type": "set_grid_motion",
		"operation_id": _operation_id("grid-motion"),
		"grid_id": grid_id,
		"linear_velocity": _protocol_vec3(direction * 2.0),
		"angular_velocity": 0.24,
	})


func _stop_target_grid() -> void:
	_send({
		"type": "set_grid_motion",
		"operation_id": _operation_id("grid-stop"),
		"grid_id": _target_or_starter_grid(),
		"linear_velocity": _protocol_vec3(Vector3.ZERO),
		"angular_velocity": 0.0,
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
		"● CONNECTED  %s" % server_url
		if connected
		else "○ OFFLINE  %s" % server_url
	)
	connection_label.add_theme_color_override(
		"font_color",
		Color(0.35, 0.95, 0.62) if connected else Color(1.0, 0.38, 0.25)
	)
	status_label.text = (
		"Universe  %s\nCell  %s\nEvent  %d  •  Tick  %d\nFence  %d  •  Hash  %s"
		% [
			snapshot.get("universe_id", "awaiting state"),
			snapshot.get("cell_id", "—"),
			int(snapshot.get("event_sequence", 0)),
			int(snapshot.get("simulation_tick", 0)),
			int(snapshot.get("fencing_token", 0)),
			String(snapshot.get("world_hash", "—")).left(12),
		]
	)
	var player_inventory := _inventory(PLAYER_INVENTORY)
	var contents: Dictionary = player_inventory.get("contents", {})
	var conserved: Dictionary = snapshot.get("conservation", {})
	inventory_label.text = (
		"ORE  %d     REFINED  %d     COMPONENTS  %d\n"
		+ "CONSERVATION  %s"
	) % [
		int(contents.get("ore", 0)),
		int(contents.get("refined_material", 0)),
		int(contents.get("components", 0)),
		"VALID" if conserved.get("valid", false) else "INVALID",
	]
	inventory_label.add_theme_color_override(
		"font_color",
		Color(0.95, 0.71, 0.27)
		if conserved.get("valid", false)
		else Color(1.0, 0.18, 0.18)
	)
	selected_label.text = "BUILD SELECTION  %s" % selected_block_kind.to_upper()
	if target_voxel != null:
		var voxel: Dictionary = voxel_lookup.get(_coord_key(target_voxel), {})
		target_label.text = "TARGET  %s voxel  %s" % [
			voxel.get("material", "rock").to_upper(),
			str(target_voxel),
		]
	elif not target_block.is_empty():
		var block: Dictionary = target_block["block"]
		target_label.text = "TARGET  %s  HP %d  [%s]" % [
			String(block.get("kind", "block")).to_upper(),
			int(block.get("health", 0)),
			target_block.get("grid_id", ""),
		]
	else:
		target_label.text = "TARGET  none"
	message_label.text = recent_message
	message_label.add_theme_color_override("font_color", recent_message_color)


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


func _protocol_vec3(value: Vector3) -> Dictionary:
	return {"x": value.x, "y": value.y, "z": value.z}


func _protocol_ivec3(value: Vector3i) -> Dictionary:
	return {"x": value.x, "y": value.y, "z": value.z}
