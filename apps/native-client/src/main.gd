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
const PROTOCOL_VERSION := 16
const PROJECTION_SCHEMA_VERSION := 3
const WORLD_SCHEMA_VERSION := 18
const EVENT_SCHEMA_VERSION := 14
const CONTENT_SCHEMA_VERSION := 11
const CONTENT_MANIFEST_VERSION := "p1.5.0"
const CELESTIAL_REGISTRY_SCHEMA_VERSION := 1
const UNIVERSE_MANIFEST_SCHEMA_VERSION := 2
const INTEREST_SCHEMA_VERSION := 1
const DEFAULT_SERVER := "ws://127.0.0.1:7777/ws"
const DEFAULT_PLAYER_ID := "player-local"
const STARTER_GRID := "grid-starter"
# Clean-room prediction mirrors the protocol-fenced p0.10.0 character content.
# The server remains authoritative for elapsed time, contacts, and final motion.
const CHARACTER_FIXED_DELTA := 1.0 / 60.0
const CONTROL_SEND_INTERVAL := 0.10
# Leave enough headroom for Godot's float32 Vector3 components to survive
# JSON's float64 reconstruction without crossing the authoritative unit sphere.
const CONTROL_INPUT_SAFE_LIMIT := 0.999999
const PREDICTION_HISTORY_LIMIT := 180
const MUTATION_QUEUE_LIMIT := 32
const MUTATION_RETRY_INTERVAL := 1.5
const MUTATION_RETRY_LIMIT := 3
const JSON_SAFE_INTEGER_MAX := 9007199254740991
const POSITION_SNAP_DISTANCE := 2.0
const ORIENTATION_SNAP_ANGLE := PI / 3.0
const CHARACTER_COLLISION_RADIUS := 0.34
const CHARACTER_STANDING_HEIGHT := 1.8
const CHARACTER_EYE_HEIGHT := 1.62
const CHARACTER_CAPSULE_HALF_HEIGHT := (CHARACTER_STANDING_HEIGHT - 2.0 * CHARACTER_COLLISION_RADIUS) * 0.5
const CHARACTER_EYE_OFFSET := CHARACTER_EYE_HEIGHT - CHARACTER_STANDING_HEIGHT * 0.5
const CHARACTER_THRUST_ACCELERATION := 10.0
const CHARACTER_BOOST_ACCELERATION := 20.0
const CHARACTER_LINEAR_DAMPENER_ACCELERATION := 14.0
const CHARACTER_ANGULAR_ACCELERATION := 5.0
const CHARACTER_ANGULAR_DAMPENER_ACCELERATION := 7.0
const CHARACTER_MAXIMUM_SPEED := 12.0
const CHARACTER_BOOST_MAXIMUM_SPEED := 24.0
const CHARACTER_MAXIMUM_ANGULAR_SPEED := 2.5
const CHARACTER_UPRIGHT_ALIGNMENT_ACCELERATION := 28.0
const CHARACTER_WALK_SPEED := 4.5
const CHARACTER_SPRINT_SPEED := 7.5
const CHARACTER_GROUND_ACCELERATION := 18.0
const CHARACTER_GROUND_BRAKING := 24.0
const CHARACTER_JUMP_SPEED := 5.0
const CHARACTER_MAGNETIC_ADHESION_ACCELERATION := 24.0
const PHYSICS_MAXIMUM_LINEAR_SPEED := 32.0
const PHYSICS_MAXIMUM_ANGULAR_SPEED := 8.0
const MOUSE_ANGULAR_INPUT_PER_PIXEL := 0.12
const TARGET_RANGE := 9.0
const TOOL_HIT_EPSILON := 0.000000001
const TOOL_DIRECTION_EPSILON := 0.000000000001
const TOOL_DDA_MAX_STEPS := 512
const MINE_DURATION := 0.72
const WELD_DURATION := 0.52
const DAMAGE_DURATION := 0.46
const RENDER_DISTANCE_LIMIT_M := 12_000.0
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
var requested_player_id := DEFAULT_PLAYER_ID
var bound_player_id := ""
var connected := false
var handshake_sent := false
var welcome_received := false
var registry_received := false
var replication_state := "loading"
var replication_detail := "WAITING FOR PROTOCOL HANDSHAKE"
var stream_family := ""
var registry_snapshot: Dictionary = {}
var universe_manifest: Dictionary = {}
var interest_entities: Dictionary = {}
var interest_session_epoch := ""
var interest_epoch := -1
var interest_baseline_id := ""
var interest_delta_sequence := -1
var interest_view_hash := ""
var interest_local_origin: Dictionary = {}
var baseline_request_pending := false
var operation_counter := 0
var committed_operation_sequence := 0
var committed_operation_actor_id := ""
var operation_frontier_observed := false
var observed_operation_frontier := -1
var operation_frontier_ready := false
var mutation_queue: Array[Dictionary] = []
var mutation_queue_actor_id := ""
var in_flight_mutation: Dictionary = {}
var in_flight_mutation_text := ""
var in_flight_mutation_actor_id := ""
var mutation_retry_elapsed := 0.0
var mutation_retry_count := 0
var mutation_resync_required := false
var authoritative_player_ready := false
var awaiting_reconnect_baseline := true
var control_send_elapsed := 0.0
var movement_epoch := 0
var next_input_sequence := 1
var last_acked_input_sequence := 0
var last_authoritative_event_sequence := -1
var last_authoritative_simulation_tick := 0
var predicted_simulation_tick := 0
var predicted_position := Vector3.ZERO
var predicted_orientation := Quaternion.IDENTITY
var predicted_linear_velocity := Vector3.ZERO
var predicted_angular_velocity := Vector3.ZERO
var predicted_surface_contact := false
var predicted_jump_held := false
var prediction_planet_center := Vector3.ZERO
var prediction_surface_radius := 0.0
var prediction_gravitational_parameter := 0.0
var prediction_gravity_fallback := Vector3.ZERO
var prediction_gravity_model_ready := false
var desired_dampeners := true
var desired_magnetic_boots := false
var last_sent_control: Dictionary = {}
var current_prediction_input_sequence := 0
var prediction_history: Array[Dictionary] = []
var pending_controls: Array[Dictionary] = []
var prediction_history_invalid := false
var mouse_delta_accumulator := Vector2.ZERO
var roll_left_held := false
var roll_right_held := false
var pending_roll_transitions: Array[float] = []
var presentation_position_offset := Vector3.ZERO
var presentation_orientation_offset := Quaternion.IDENTITY
var require_neutral_baseline := true
var last_player_id := ""
var last_player_life_state := ""
var snapshot: Dictionary = {}
var actor_private_snapshot: Dictionary = {}
var session_role_kind := ""
var voxel_lookup: Dictionary = {}
var voxel_coordinate_lookup: Dictionary = {}
var voxel_chunk_nodes: Dictionary = {}
var rendered_voxel_chunk_fingerprints: Dictionary = {}
var grid_lookup: Dictionary = {}
var grid_node_lookup: Dictionary = {}
var grid_topology_fingerprints: Dictionary = {}
var remote_player_nodes: Dictionary = {}
var rendered_voxel_count := -1
var selected_block_kind := "structural"
var target_hit: Dictionary = {}
var target_voxel: Variant = null
var target_block: Dictionary = {}
var recent_message := "Starting local universe connection…"
var recent_message_color := Color(0.56, 0.87, 1.0)
var smoke_test := false
var smoke_operation := ""
var smoke_input_sequence := 0
var smoke_receipt_received := false
var smoke_visual_ready := false
var recovery_operation := ""
var last_socket_state := -1
var closed_reported := false
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
var active_grid_control_id := ""
var selected_cargo_inventory_id := ""
var last_targeted_owned_grid_id := ""
var inventory_item_labels: Dictionary = {}
var inventory_capacity_labels: Dictionary = {}
var inventory_capacity_bars: Dictionary = {}
var inventory_rows: Dictionary = {}
var inventory_title_labels: Dictionary = {}
var inventory_subtitle_labels: Dictionary = {}
var inventory_selectors: Dictionary = {}
var inventory_transfer_buttons: Array[Button] = []
var inventory_tab_buttons: Dictionary = {}
var inventory_content_root: Control
var production_content_root: Control
var production_machine_label: Label
var production_queue_label: Label
var production_route_label: Label
var production_buttons: Dictionary = {}
var active_inventory_tab := "inventory"
var inventory_selected_resource := "component"
var inventory_selected_side := "suit"
var inventory_filters := {"suit": "all", "cargo": "all"}
var inventory_search_queries := {"suit": "", "cargo": ""}

var camera: Camera3D
var asteroid_root: Node3D
var grids_root: Node3D
var players_root: Node3D
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
var celestial_visuals: Dictionary = {}
var rendered_celestial_registry_hash := ""
var rendered_celestial_origin: Dictionary = {}
var critical_oxygen_panel: ColorRect
var critical_oxygen_label: Label
var incapacitated_overlay: Control
var incapacitated_detail_label: Label
var recovery_button: Button

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
	_advance_mutation_transport(delta)
	_update_player_presentation(delta)
	_update_target()
	_update_tool_action(delta)
	_update_viewmodel(delta)
	_update_interface()


func _physics_process(delta: float) -> void:
	if (
		not connected
		or not authoritative_player_ready
		or not operation_frontier_ready
		or mutation_resync_required
	):
		return
	if _local_player_incapacitated():
		return
	control_send_elapsed += delta
	var control := _neutral_player_control()
	var sampled_roll_transition := false
	if require_neutral_baseline:
		if _send_player_control(control, true):
			require_neutral_baseline = false
	else:
		sampled_roll_transition = not pending_roll_transitions.is_empty()
		control = _sample_player_control()
		if _should_send_player_control(control) and _send_player_control(control, false):
			if sampled_roll_transition and not pending_roll_transitions.is_empty():
				pending_roll_transitions.pop_front()
	_predict_player_step(control, CHARACTER_FIXED_DELTA, true)


func _input(event: InputEvent) -> void:
	if event is InputEventKey and _reconnect_shortcut(event):
		_connect_to_server()
		get_viewport().set_input_as_handled()
		return

	if _local_player_incapacitated():
		if event is InputEventKey and event.pressed and not event.echo:
			if event.keycode in [KEY_ENTER, KEY_KP_ENTER]:
				_request_recovery()
				get_viewport().set_input_as_handled()
		return

	if event is InputEventMouseMotion and Input.mouse_mode == Input.MOUSE_MODE_CAPTURED:
		mouse_delta_accumulator += event.relative
		return

	# Inventory text entry owns keyboard input. A held grid-control key still gets
	# its release so opening the terminal cannot leave thrust latched.
	if inventory_open and event is InputEventKey:
		if event.keycode == KEY_M and not event.pressed and not active_grid_control_id.is_empty():
			_stop_target_grid()
		var focus_owner := get_viewport().gui_get_focus_owner()
		var text_entry_focused := focus_owner is LineEdit or focus_owner is TextEdit
		if _inventory_close_shortcut(event, text_entry_focused):
			_set_inventory_open(false)
		return

	if (
		event is InputEventKey
		and not event.echo
		and event.keycode in [KEY_Q, KEY_E]
		and Input.mouse_mode == Input.MOUSE_MODE_CAPTURED
	):
		_capture_roll_key_transition(event)

	if event is InputEventKey and event.keycode == KEY_M and not event.echo:
		if event.pressed and Input.mouse_mode == Input.MOUSE_MODE_CAPTURED:
			_move_target_grid()
		elif not active_grid_control_id.is_empty():
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
					_clear_transient_character_input()
			KEY_I:
				_set_inventory_open(not inventory_open)
			KEY_J:
				_toggle_jetpack()
			KEY_K:
				_toggle_magnetic_boots()
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
			KEY_6:
				selected_block_kind = "conveyor"
				build_mode = true
			KEY_7:
				selected_block_kind = "refinery"
				build_mode = true
			KEY_8:
				selected_block_kind = "assembler"
				build_mode = true
			KEY_B:
				build_mode = not build_mode
				action_charge = 0.0
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
				_send_transport({"type": "request_snapshot"})
			KEY_Z:
				desired_dampeners = not desired_dampeners
				_set_message(
					"Inertial dampeners %s" % ("online" if desired_dampeners else "offline")
				)
			KEY_L:
				suit_light_enabled = not suit_light_enabled
				suit_light.visible = suit_light_enabled
				_set_message("Helmet light %s" % ("online" if suit_light_enabled else "offline"))
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


func _reconnect_shortcut(event: InputEventKey) -> bool:
	return event.pressed and not event.echo and event.keycode == KEY_F5


func _player_life_state(player: Dictionary) -> String:
	var life_state: Variant = player.get("life_state", {})
	if life_state is Dictionary:
		return String(life_state.get("kind", "alive"))
	if life_state is String:
		return String(life_state)
	return "alive"


func _player_is_incapacitated(player: Dictionary) -> bool:
	return _player_life_state(player) == "incapacitated"


func _local_player_incapacitated() -> bool:
	var player := _local_player()
	return not _player_controls_enabled(player)


func _controlled_player_id() -> String:
	return bound_player_id if not bound_player_id.is_empty() else requested_player_id


func _player_from_roster(players: Array, player_id: String) -> Dictionary:
	for candidate in players:
		if candidate is Dictionary and String(candidate.get("player_id", "")) == player_id:
			return candidate
	return {}


func _local_player() -> Dictionary:
	var player: Variant = actor_private_snapshot.get("player", {})
	if not player is Dictionary:
		return {}
	if String(player.get("player_id", "")) != bound_player_id:
		return {}
	return player


func _local_inventory_id() -> String:
	return String(_local_player().get("inventory_id", ""))


func _local_environment() -> Dictionary:
	var player_environment: Variant = _local_player().get("environment", {})
	if player_environment is Dictionary and not player_environment.is_empty():
		return player_environment
	return snapshot.get("environment", {})


func _private_inventory_ready() -> bool:
	return not _local_inventory_id().is_empty()


func _clear_actor_private_state() -> void:
	actor_private_snapshot = {}
	operation_frontier_ready = false
	selected_cargo_inventory_id = ""
	last_targeted_owned_grid_id = ""
	for button in inventory_transfer_buttons:
		if is_instance_valid(button):
			button.disabled = true
	var selector: OptionButton = inventory_selectors.get("cargo", null)
	if selector != null and is_instance_valid(selector):
		selector.clear()
		selector.add_item("NO AUTHORIZED CARGO LINK")
		selector.set_item_metadata(0, "")
		selector.disabled = true


func _protocol_nonnegative_integer(value: Variant) -> int:
	var value_type := typeof(value)
	if value_type == TYPE_INT:
		var integer_value := int(value)
		if integer_value >= 0 and integer_value <= JSON_SAFE_INTEGER_MAX:
			return integer_value
		return -1
	if value_type != TYPE_FLOAT:
		return -1
	var float_value := float(value)
	if (
		not is_finite(float_value)
		or float_value < 0.0
		or float_value > float(JSON_SAFE_INTEGER_MAX)
		or floor(float_value) != float_value
	):
		return -1
	return int(float_value)


func _protocol_signed_integer(value: Variant) -> Variant:
	if typeof(value) == TYPE_INT:
		return int(value)
	if typeof(value) != TYPE_FLOAT:
		return null
	var float_value := float(value)
	if (
		not is_finite(float_value)
		or absf(float_value) > float(JSON_SAFE_INTEGER_MAX)
		or floor(float_value) != float_value
	):
		return null
	return int(float_value)


func _actor_private_matches(candidate: Variant, event_sequence: int) -> bool:
	if not candidate is Dictionary or candidate.is_empty() or bound_player_id.is_empty():
		return false
	var player: Variant = candidate.get("player", {})
	if not player is Dictionary or String(player.get("player_id", "")) != bound_player_id:
		return false
	var carried_inventory_id := String(player.get("inventory_id", ""))
	var carried_matches := 0
	for inventory_value in candidate.get("inventories", []):
		if not inventory_value is Dictionary:
			continue
		var domain: Dictionary = inventory_value.get("domain", {})
		if (
			String(inventory_value.get("inventory_id", "")) == carried_inventory_id
			and String(domain.get("kind", "")) == "player"
			and String(domain.get("player_id", "")) == bound_player_id
		):
			carried_matches += 1
	if carried_inventory_id.is_empty() or carried_matches != 1:
		return false
	if (
		not candidate.has("committed_operation_sequence")
		or _protocol_nonnegative_integer(
			candidate.get("committed_operation_sequence", null)
		) < 0
	):
		return false
	# Protocol 16 nests the overlay in the outer interest frame, making the
	# outer sequence authoritative. Honor a future explicit sequence only when
	# it agrees, so malformed extensions still fail closed.
	return (
		not candidate.has("event_sequence")
		or _protocol_nonnegative_integer(candidate.get("event_sequence", null))
		== event_sequence
	)


func _install_actor_private(candidate: Variant, event_sequence: int) -> bool:
	_clear_actor_private_state()
	if not _actor_private_matches(candidate, event_sequence):
		return false
	actor_private_snapshot = (candidate as Dictionary).duplicate(true)
	if not actor_private_snapshot.get("production_queues", []) is Array:
		actor_private_snapshot["production_queues"] = []
	return _reconcile_operation_frontier(
		_protocol_nonnegative_integer(
			actor_private_snapshot.get("committed_operation_sequence", null)
		)
	)


func _production_queues() -> Array:
	var queues: Variant = actor_private_snapshot.get("production_queues", [])
	return queues if queues is Array else []


func _life_support_display_state(player: Dictionary) -> String:
	if _player_is_incapacitated(player):
		return "incapacitated"
	var critical_threshold := int(player.get("critical_oxygen_milli", 0))
	var oxygen := int(player.get("suit_oxygen_milli", 0))
	if critical_threshold > 0 and oxygen < critical_threshold:
		return "critical"
	return "normal"


func _player_controls_enabled(player: Dictionary) -> bool:
	return not player.is_empty() and not _player_is_incapacitated(player)


func _parse_command_line() -> void:
	for argument in OS.get_cmdline_user_args():
		if argument.begins_with("--server="):
			server_url = argument.trim_prefix("--server=")
		elif argument.begins_with("--player-id="):
			requested_player_id = argument.trim_prefix("--player-id=")
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
	players_root = Node3D.new()
	players_root.name = "AuthoritativeRemotePlayers"
	add_child(players_root)
	stars_root = Node3D.new()
	stars_root.name = "Starfield"
	add_child(stars_root)
	planet_root = Node3D.new()
	planet_root.name = "RegisteredCelestialBodies"
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
		"conveyor": _armored_material(Color(0.24, 0.63, 0.72), 0.62, 0.56),
		"refinery": _armored_material(Color(0.56, 0.31, 0.18), 0.71, 0.66),
		"assembler": _armored_material(Color(0.28, 0.58, 0.48), 0.58, 0.52),
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


func _rebuild_registered_celestials() -> void:
	if planet_root == null or not registry_received or interest_local_origin.is_empty():
		return
	var registry_hash := String(registry_snapshot.get("registry_hash", ""))
	if (
		registry_hash == rendered_celestial_registry_hash
		and interest_local_origin == rendered_celestial_origin
	):
		return
	for child in planet_root.get_children():
		planet_root.remove_child(child)
		child.queue_free()
	celestial_visuals.clear()
	planet_cloud_layer = null
	if asteroid_root != null:
		var voxel_body := _registered_body(String(snapshot.get("voxel_body_id", "")))
		var voxel_center: Variant = _address_relative_m(voxel_body.get("center", {}), interest_local_origin)
		if voxel_center is Vector3:
			asteroid_root.position = voxel_center
	for body_value in registry_snapshot.get("bodies", []):
		if not body_value is Dictionary:
			continue
		var body: Dictionary = body_value
		var descriptor := String(body.get("visual_descriptor_id", ""))
		# The registered procedural origin asteroid is rendered from authoritative
		# voxel chunks. A second sphere would conceal mining changes.
		if descriptor == "origin-regolith-v1" and body.has("voxel_field_id"):
			continue
		var center_value: Variant = _address_relative_m(body.get("center", {}), interest_local_origin)
		if not center_value is Vector3:
			continue
		var center: Vector3 = center_value
		if center.length() > RENDER_DISTANCE_LIMIT_M * 2.0:
			continue
		var radius := float(body.get("surface_radius_um", 0)) / 1_000_000.0
		if not is_finite(radius) or radius <= 0.0:
			continue
		var visual := MeshInstance3D.new()
		visual.name = "Celestial_%s" % String(body.get("body_id", "unknown"))
		var mesh := SphereMesh.new()
		mesh.radius = radius
		mesh.height = radius * 2.0
		mesh.radial_segments = 192 if body.get("kind", "") == "planet" else 64
		mesh.rings = 96 if body.get("kind", "") == "planet" else 32
		if descriptor == "khepri-prime-terrestrial-v1":
			var surface_material := ShaderMaterial.new()
			surface_material.shader = PLANET_SHADER
			surface_material.set_shader_parameter("planet_albedo", PLANET_TEXTURE)
			mesh.material = surface_material
		elif descriptor == "sable-airless-v1":
			mesh.material = _material(Color(0.22, 0.18, 0.16), 0.98, 0.01)
		else:
			# Unknown descriptors remain visible but never borrow a misleading
			# authored appearance from a different registered body.
			mesh.material = _material(Color(0.34, 0.38, 0.42), 0.92, 0.08)
			visual.set_meta("verse_visual_descriptor", "neutral_proxy")
		visual.mesh = mesh
		visual.position = center
		var orientation: Dictionary = body.get("fixed_orientation_microradians", {})
		visual.rotation = Vector3(
			float(orientation.get("x", 0)) / 1_000_000.0,
			float(orientation.get("y", 0)) / 1_000_000.0,
			float(orientation.get("z", 0)) / 1_000_000.0
		)
		planet_root.add_child(visual)
		celestial_visuals[String(body.get("body_id", ""))] = visual
		var atmosphere_height := float(body.get("atmosphere_height_um", 0)) / 1_000_000.0
		if atmosphere_height <= 0.0:
			continue
		var atmosphere := MeshInstance3D.new()
		atmosphere.name = "%s_Atmosphere" % visual.name
		var atmosphere_mesh := SphereMesh.new()
		atmosphere_mesh.radius = radius + atmosphere_height
		atmosphere_mesh.height = atmosphere_mesh.radius * 2.0
		atmosphere_mesh.radial_segments = 128
		atmosphere_mesh.rings = 64
		var atmosphere_material := ShaderMaterial.new()
		atmosphere_material.shader = ATMOSPHERE_SHADER
		atmosphere_mesh.material = atmosphere_material
		atmosphere.mesh = atmosphere_mesh
		atmosphere.position = center
		planet_root.add_child(atmosphere)
		if descriptor == "khepri-prime-terrestrial-v1":
			planet_cloud_layer = MeshInstance3D.new()
			planet_cloud_layer.name = "%s_Clouds" % visual.name
			var cloud_mesh := SphereMesh.new()
			cloud_mesh.radius = radius + minf(12.0, atmosphere_height * 0.2)
			cloud_mesh.height = cloud_mesh.radius * 2.0
			cloud_mesh.radial_segments = 160
			cloud_mesh.rings = 80
			var cloud_material := ShaderMaterial.new()
			cloud_material.shader = CLOUD_SHADER
			cloud_mesh.material = cloud_material
			planet_cloud_layer.mesh = cloud_mesh
			planet_cloud_layer.position = center
			planet_root.add_child(planet_cloud_layer)
	rendered_celestial_registry_hash = registry_hash
	rendered_celestial_origin = interest_local_origin.duplicate(true)


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
		"WASD  MOVE    SPACE  JUMP/ASCEND    SHIFT  SPRINT/BOOST    Q/E  EVA ROLL    J  JETPACK    K  MAG BOOTS    I  INVENTORY",
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
	_build_life_support_interface(canvas)


func _build_life_support_interface(canvas: CanvasLayer) -> void:
	critical_oxygen_panel = ColorRect.new()
	critical_oxygen_panel.name = "CriticalOxygenWarning"
	critical_oxygen_panel.anchor_left = 0.5
	critical_oxygen_panel.anchor_right = 0.5
	critical_oxygen_panel.offset_left = -260.0
	critical_oxygen_panel.offset_top = 48.0
	critical_oxygen_panel.offset_right = 260.0
	critical_oxygen_panel.offset_bottom = 92.0
	critical_oxygen_panel.color = Color(0.38, 0.025, 0.018, 0.94)
	critical_oxygen_panel.mouse_filter = Control.MOUSE_FILTER_IGNORE
	critical_oxygen_panel.visible = false
	canvas.add_child(critical_oxygen_panel)

	critical_oxygen_label = Label.new()
	critical_oxygen_label.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	critical_oxygen_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	critical_oxygen_label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	critical_oxygen_label.add_theme_font_size_override("font_size", 15)
	critical_oxygen_label.add_theme_color_override("font_color", Color(1.0, 0.73, 0.48))
	critical_oxygen_panel.add_child(critical_oxygen_label)

	incapacitated_overlay = Control.new()
	incapacitated_overlay.name = "IncapacitatedOverlay"
	incapacitated_overlay.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	incapacitated_overlay.mouse_filter = Control.MOUSE_FILTER_STOP
	incapacitated_overlay.visible = false
	canvas.add_child(incapacitated_overlay)

	var blackout := ColorRect.new()
	blackout.color = Color(0.015, 0.002, 0.002, 0.91)
	blackout.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	blackout.mouse_filter = Control.MOUSE_FILTER_STOP
	incapacitated_overlay.add_child(blackout)

	var failure_panel := ColorRect.new()
	failure_panel.anchor_left = 0.5
	failure_panel.anchor_top = 0.5
	failure_panel.anchor_right = 0.5
	failure_panel.anchor_bottom = 0.5
	failure_panel.offset_left = -310.0
	failure_panel.offset_top = -170.0
	failure_panel.offset_right = 310.0
	failure_panel.offset_bottom = 170.0
	failure_panel.color = Color(0.055, 0.012, 0.014, 0.98)
	incapacitated_overlay.add_child(failure_panel)

	var failure_accent := ColorRect.new()
	failure_accent.color = Color(1.0, 0.16, 0.07)
	failure_accent.anchor_right = 1.0
	failure_accent.offset_bottom = 3.0
	failure_panel.add_child(failure_accent)

	var failure_title := Label.new()
	failure_title.text = "EVA LIFE SUPPORT FAILURE"
	failure_title.position = Vector2(24.0, 32.0)
	failure_title.size = Vector2(572.0, 42.0)
	failure_title.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	failure_title.add_theme_font_size_override("font_size", 27)
	failure_title.add_theme_color_override("font_color", Color(1.0, 0.34, 0.20))
	failure_panel.add_child(failure_title)

	incapacitated_detail_label = Label.new()
	incapacitated_detail_label.position = Vector2(34.0, 94.0)
	incapacitated_detail_label.size = Vector2(552.0, 100.0)
	incapacitated_detail_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	incapacitated_detail_label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	incapacitated_detail_label.add_theme_font_size_override("font_size", 14)
	incapacitated_detail_label.add_theme_color_override("font_color", Color(0.82, 0.84, 0.85))
	failure_panel.add_child(incapacitated_detail_label)

	recovery_button = Button.new()
	recovery_button.text = "[ENTER]  REQUEST RECOVERY"
	recovery_button.position = Vector2(155.0, 230.0)
	recovery_button.size = Vector2(310.0, 56.0)
	recovery_button.add_theme_font_size_override("font_size", 16)
	recovery_button.add_theme_color_override("font_color", Color(0.88, 0.96, 1.0))
	recovery_button.add_theme_stylebox_override(
		"normal", _terminal_style(Color(0.08, 0.20, 0.24), Color(0.34, 0.76, 0.88), 2)
	)
	recovery_button.pressed.connect(_request_recovery)
	failure_panel.add_child(recovery_button)


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
		tab.disabled = not tab_index in [0, 2]
		if tab_index == 0:
			tab.pressed.connect(_set_inventory_tab.bind("inventory"))
			inventory_tab_buttons["inventory"] = tab
		elif tab_index == 2:
			tab.pressed.connect(_set_inventory_tab.bind("production"))
			inventory_tab_buttons["production"] = tab
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

	inventory_content_root = Control.new()
	inventory_content_root.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	inventory_content_root.mouse_filter = Control.MOUSE_FILTER_PASS
	terminal.add_child(inventory_content_root)

	var suit_panel := _inventory_column(
		inventory_content_root, 0.018, 0.462, "ORPHEUS-7", "SUIT INVENTORY", "suit"
	)
	var cargo_panel := _inventory_column(
		inventory_content_root, 0.538, 0.982, "KHEPRI RELAY", "LARGE CARGO CONTAINER", "cargo"
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
	_add_transfer_controls(inventory_content_root)
	_update_inventory_row_styles()

	var hint := _hud_label(
		"SELECT AN ITEM, THEN USE THE CENTER ARROWS  //  SINGLE = ONE UNIT  //  DOUBLE = FULL STACK  //  V = QUICK TRANSFER",
		Vector2(24.0, -34.0),
		11
	)
	hint.set_anchors_preset(Control.PRESET_BOTTOM_LEFT)
	hint.add_theme_color_override("font_color", Color(0.45, 0.66, 0.73))
	inventory_content_root.add_child(hint)

	_build_production_terminal(terminal)
	_set_inventory_tab("inventory")


func _build_production_terminal(terminal: Control) -> void:
	production_content_root = Control.new()
	production_content_root.name = "ProductionTerminal"
	production_content_root.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	production_content_root.mouse_filter = Control.MOUSE_FILTER_PASS
	terminal.add_child(production_content_root)

	var panel := ColorRect.new()
	panel.color = Color(0.052, 0.063, 0.069, 0.98)
	panel.anchor_left = 0.018
	panel.anchor_top = 0.12
	panel.anchor_right = 0.982
	panel.anchor_bottom = 0.92
	production_content_root.add_child(panel)
	var title := _hud_label("PHYSICAL PRODUCTION NETWORK", Vector2(22.0, 18.0), 19)
	title.add_theme_color_override("font_color", Color(0.80, 0.91, 0.94))
	panel.add_child(title)
	var subtitle := _hud_label(
		"OWNED MACHINES  //  CARGO-TO-CARGO ROUTING  //  AUTHORITATIVE QUEUES",
		Vector2(22.0, 48.0), 11
	)
	subtitle.add_theme_color_override("font_color", Color(0.38, 0.67, 0.74))
	panel.add_child(subtitle)

	production_machine_label = _hud_label("NO AUTHORIZED MACHINES", Vector2(24.0, 92.0), 13)
	production_machine_label.size = Vector2(420.0, 330.0)
	production_machine_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	panel.add_child(production_machine_label)
	production_queue_label = _hud_label("NO CANONICAL JOBS", Vector2(468.0, 92.0), 13)
	production_queue_label.size = Vector2(470.0, 330.0)
	production_queue_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	panel.add_child(production_queue_label)
	production_route_label = _hud_label("SELECT AN OWNED MACHINE ROUTE", Vector2(24.0, -88.0), 12)
	production_route_label.set_anchors_preset(Control.PRESET_BOTTOM_LEFT)
	production_route_label.size = Vector2(900.0, 28.0)
	production_route_label.add_theme_color_override("font_color", Color(0.53, 0.76, 0.80))
	panel.add_child(production_route_label)
	for data in [
		["refining", "QUEUE 1 REFINING BATCH", Vector2(24.0, -52.0)],
		["component", "QUEUE 1 COMPONENT BATCH", Vector2(292.0, -52.0)],
	]:
		var button := Button.new()
		button.text = String(data[1])
		button.set_anchors_preset(Control.PRESET_BOTTOM_LEFT)
		button.position = data[2]
		button.size = Vector2(252.0, 34.0)
		button.pressed.connect(_queue_physical_production.bind(String(data[0])))
		panel.add_child(button)
		production_buttons[String(data[0])] = button


func _set_inventory_tab(tab_name: String) -> void:
	if not tab_name in ["inventory", "production"]:
		return
	active_inventory_tab = tab_name
	if is_instance_valid(inventory_content_root):
		inventory_content_root.visible = tab_name == "inventory"
	if is_instance_valid(production_content_root):
		production_content_root.visible = tab_name == "production"
	for name in inventory_tab_buttons:
		var button: Button = inventory_tab_buttons[name]
		button.button_pressed = String(name) == tab_name
	if tab_name == "production":
		_update_production_terminal()


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
	inventory_title_labels[side] = title_label
	var subtitle_label := _hud_label(subtitle, Vector2(16.0, 36.0), 10)
	subtitle_label.add_theme_color_override("font_color", Color(0.38, 0.67, 0.74))
	panel.add_child(subtitle_label)
	inventory_subtitle_labels[side] = subtitle_label

	var selector := OptionButton.new()
	selector.position = Vector2(16.0, 60.0)
	selector.size = Vector2(438.0, 32.0)
	selector.add_item("%s  /  %s" % [title, subtitle])
	if side == "cargo":
		selector.item_selected.connect(_cargo_inventory_selected)
	panel.add_child(selector)
	inventory_selectors[side] = selector
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
		inventory_transfer_buttons.append(button)


func _transfer_selected_inventory(reverse: bool, all: bool) -> void:
	_transfer_inventory_resource(inventory_selected_resource, reverse, all)


func _cargo_inventory_selected(index: int) -> void:
	var selector: OptionButton = inventory_selectors.get("cargo", null)
	if selector == null or index < 0 or index >= selector.item_count:
		return
	var inventory_id := String(selector.get_item_metadata(index))
	for candidate in _owned_cargo_candidates():
		if String(candidate.get("inventory_id", "")) == inventory_id:
			selected_cargo_inventory_id = inventory_id
			return


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
	_begin_player_resync()
	recovery_operation = ""
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
			_send_transport({
				"type": "hello",
				"protocol_version": PROTOCOL_VERSION,
				"client_name": "godot-native-p1.5",
				"authentication": {
					"kind": "local_development",
					"player_id": requested_player_id,
				},
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
		if replication_state != "fatal":
			_begin_player_resync()
		recovery_operation = ""
		handshake_sent = false


func _handle_server_message(message: Dictionary) -> void:
	match message.get("type", ""):
		"welcome":
			if welcome_received or not _welcome_tuple_valid(message):
				_client_fatal("PROTOCOL TUPLE MISMATCH")
				return
			welcome_received = true
			replication_state = "loading"
			replication_detail = "VALIDATING CELESTIAL REGISTRY"
			var session_role: Dictionary = message.get("session_role", {})
			session_role_kind = String(session_role.get("kind", ""))
			_clear_actor_private_state()
			if session_role_kind == "player":
				bound_player_id = String(session_role.get("player_id", requested_player_id))
			else:
				bound_player_id = ""
			if not _mutation_actor_matches_session():
				_clear_mutation_pipeline()
				mutation_resync_required = true
				_set_message("PENDING COMMANDS DISCARDED // SESSION ACTOR CHANGED", true)
			if (
				not committed_operation_actor_id.is_empty()
				and committed_operation_actor_id != bound_player_id
			):
				committed_operation_sequence = 0
				committed_operation_actor_id = ""
				operation_frontier_observed = false
				observed_operation_frontier = -1
			_set_message(
				"Protocol 16 linked to %s // %s // loading registry"
				% [message.get("server_name", "The Verse"), _controlled_player_id()]
			)
		"registry":
			if not welcome_received or registry_received:
				_client_fatal("REGISTRY ORDER VIOLATION")
				return
			if not _install_registry(message):
				_client_fatal("REGISTRY OR MANIFEST BINDING INVALID")
				return
			registry_received = true
			replication_detail = "WAITING FOR INTEREST BASELINE"
			_set_message("CELESTIAL REGISTRY VERIFIED // WAITING FOR LOCAL VIEW")
		"interest_baseline":
			if not _apply_interest_baseline(message.get("baseline", {})):
				_request_fresh_interest_baseline("BASELINE VALIDATION FAILED")
		"interest_delta":
			if not _apply_interest_delta(message.get("delta", {})):
				_request_fresh_interest_baseline("INTEREST FRONTIER MISMATCH")
		"snapshot", "motion_state":
			_client_fatal("LEGACY REPLICATION FAMILY REJECTED")
		"intent_accepted":
			var receipt: Dictionary = message.get("receipt", {})
			if _handle_intent_accepted(receipt):
				if receipt.get("code", "") != "player_control_set":
					_set_message(receipt.get("message", "Intent accepted"))
				if receipt.get("operation_id", "") == recovery_operation:
					_set_message("Recovery authorized // awaiting authoritative snapshot")
				if smoke_test and receipt.get("operation_id", "") == smoke_operation:
					smoke_receipt_received = true
					_check_smoke_control_ack(_local_player())
		"intent_rejected":
			pending_mine_position = null
			if _handle_intent_rejected(message):
				if message.get("operation_id", "") == recovery_operation:
					recovery_operation = ""
				_set_message(
					"%s — %s"
					% [message.get("code", "rejected"), message.get("message", "")],
					true
				)
		"fatal":
			mutation_resync_required = true
			operation_frontier_ready = false
			authoritative_player_ready = false
			replication_state = "fatal"
			replication_detail = "%s // %s" % [message.get("code", ""), message.get("message", "")]
			_set_message(
				"FATAL %s — %s" % [message.get("code", ""), message.get("message", "")],
				true
			)
		_:
			_client_fatal("UNKNOWN SERVER MESSAGE TYPE")


func _welcome_tuple_valid(message: Dictionary) -> bool:
	var role: Variant = message.get("session_role", {})
	if not role is Dictionary or not String(role.get("kind", "")) in ["player", "spectator"]:
		return false
	if String(role.get("kind", "")) == "player" and (
		String(role.get("player_id", "")).is_empty()
		or String(role.get("player_id", "")) != requested_player_id
	):
		return false
	return (
		int(message.get("protocol_version", -1)) == PROTOCOL_VERSION
		and int(message.get("projection_schema_version", -1)) == PROJECTION_SCHEMA_VERSION
		and int(message.get("world_schema_version", -1)) == WORLD_SCHEMA_VERSION
		and int(message.get("event_schema_version", -1)) == EVENT_SCHEMA_VERSION
		and int(message.get("content_schema_version", -1)) == CONTENT_SCHEMA_VERSION
		and String(message.get("content_manifest_version", "")) == CONTENT_MANIFEST_VERSION
		and int(message.get("celestial_registry_schema_version", -1))
		== CELESTIAL_REGISTRY_SCHEMA_VERSION
		and int(message.get("universe_manifest_schema_version", -1))
		== UNIVERSE_MANIFEST_SCHEMA_VERSION
		and int(message.get("interest_schema_version", -1)) == INTEREST_SCHEMA_VERSION
	)


func _install_registry(message: Dictionary) -> bool:
	var registry_value: Variant = message.get("registry", {})
	var manifest_value: Variant = message.get("universe_manifest", {})
	if not registry_value is Dictionary or not manifest_value is Dictionary:
		return false
	var registry: Dictionary = registry_value
	var manifest: Dictionary = manifest_value
	var registry_hash := String(registry.get("registry_hash", ""))
	var manifest_hash := String(manifest.get("manifest_hash", ""))
	var universe_id := String(registry.get("universe_id", ""))
	if (
		int(registry.get("schema_version", -1)) != CELESTIAL_REGISTRY_SCHEMA_VERSION
		or int(manifest.get("schema_version", -1)) != UNIVERSE_MANIFEST_SCHEMA_VERSION
		or not _valid_hash(registry_hash)
		or not _valid_hash(manifest_hash)
		or universe_id.is_empty()
		or String(manifest.get("universe_id", "")) != universe_id
		or String(manifest.get("celestial_registry_hash", "")) != registry_hash
		or int(manifest.get("celestial_registry_schema_version", -1))
		!= CELESTIAL_REGISTRY_SCHEMA_VERSION
		or int(manifest.get("content_schema_version", -1)) != CONTENT_SCHEMA_VERSION
		or String(manifest.get("content_manifest_version", "")) != CONTENT_MANIFEST_VERSION
		or int(manifest.get("world_schema_version", -1)) != WORLD_SCHEMA_VERSION
		or int(manifest.get("event_schema_version", -1)) != EVENT_SCHEMA_VERSION
		or String(manifest.get("generation_rule_version", ""))
		!= String(registry.get("generation_rule_version", ""))
		or int(manifest.get("address_schema_version", -1)) != 1
		or int(manifest.get("cells_per_sector_axis", -1)) <= 0
		or int(manifest.get("cell_edge_um", 0)) <= 0
		or int(manifest.get("sector_edge_um", 0)) <= 0
		or not registry.get("bodies", []) is Array
		or registry.get("bodies", []).is_empty()
	):
		if smoke_test:
			printerr(
				"VERSE_SMOKE_REGISTRY_HEADER_INVALID registry_schema=%s manifest_schema=%s "
				+ "registry_hash=%s manifest_hash=%s sector_edge=%s cell_edge=%s cells=%s"
				% [
					registry.get("schema_version", null), manifest.get("schema_version", null),
					registry_hash, manifest_hash, manifest.get("sector_edge_um", null),
					manifest.get("cell_edge_um", null), manifest.get("cells_per_sector_axis", null),
				]
			)
		return false
	var ids: Dictionary = {}
	for body_value in registry.get("bodies", []):
		if not body_value is Dictionary:
			return false
		var body: Dictionary = body_value
		var body_id := String(body.get("body_id", ""))
		if (
			body_id.is_empty()
			or ids.has(body_id)
			or not String(body.get("kind", "")) in ["planet", "moon", "asteroid", "asteroid_field"]
			or not _universe_address_valid(body.get("center", {}), manifest)
			or int(body.get("surface_radius_um", 0)) <= 0
			or int(body.get("exclusion_radius_um", 0)) < int(body.get("surface_radius_um", 0))
			or String(body.get("content_manifest_version", "")) != CONTENT_MANIFEST_VERSION
			or String(body.get("content_hash", "")) != String(manifest.get("content_hash", ""))
			or String(body.get("generation_rule_version", ""))
			!= String(registry.get("generation_rule_version", ""))
			or String(body.get("geometry_definition_id", "")).is_empty()
			or String(body.get("material_definition_id", "")).is_empty()
			or String(body.get("gravity_definition_id", "")).is_empty()
			or String(body.get("atmosphere_definition_id", "")).is_empty()
			or String(body.get("resource_definition_id", "")).is_empty()
			or String(body.get("visual_descriptor_id", "")).is_empty()
		):
			if smoke_test:
				printerr(
					"VERSE_SMOKE_REGISTRY_BODY_INVALID id=%s kind=%s address=%s radius=%s exclusion=%s version=%s content=%s"
					% [
						body_id, body.get("kind", null),
						_universe_address_valid(body.get("center", {}), manifest),
						body.get("surface_radius_um", null), body.get("exclusion_radius_um", null),
						body.get("content_manifest_version", null), body.get("content_hash", null),
					]
				)
			return false
		ids[body_id] = true
	var parents: Dictionary = {}
	for body_value in registry.get("bodies", []):
		var parent_id := String(body_value.get("parent_body_id", ""))
		if not parent_id.is_empty() and (not ids.has(parent_id) or parent_id == body_value.get("body_id", "")):
			return false
		parents[String(body_value.get("body_id", ""))] = parent_id
	for body_id in parents:
		var visited: Dictionary = {}
		var cursor := String(body_id)
		while not cursor.is_empty():
			if visited.has(cursor):
				return false
			visited[cursor] = true
			cursor = String(parents.get(cursor, ""))
	registry_snapshot = registry.duplicate(true)
	universe_manifest = manifest.duplicate(true)
	return true


func _apply_interest_baseline(authoritative: Dictionary) -> bool:
	if not welcome_received or not registry_received or authoritative.is_empty():
		return false
	if not stream_family.is_empty() and stream_family != "interest":
		return false
	var interest_value: Variant = authoritative.get("interest", {})
	if not interest_value is Dictionary:
		return false
	var interest: Dictionary = interest_value
	if (
		not _interest_outer_bindings_valid(authoritative, interest)
		or String(interest.get("frame_kind", "")) != "baseline"
		or int(interest.get("delta_sequence", -1)) != 0
		or interest.get("previous_view_hash", null) != null
		or String(interest.get("session_epoch", "")).is_empty()
		or String(interest.get("baseline_id", "")).is_empty()
		or not _valid_hash(String(interest.get("view_hash", "")))
		or not interest.get("replaced", []) is Array
		or not interest.get("removed", []) is Array
		or not interest.get("replaced", []).is_empty()
		or not interest.get("removed", []).is_empty()
	):
		return false
	var origin: Dictionary = interest.get("local_origin_address", {})
	var staged := _empty_interest_entities()
	if not _apply_interest_operations(staged, interest, origin, true):
		return false
	if not _baseline_arrays_match_interest(authoritative, staged):
		return false
	var committed := _world_from_interest(authoritative, staged, origin, true)
	if committed.is_empty() or not _private_projection_candidate_valid(committed):
		return false
	_apply_snapshot(committed)
	if session_role_kind == "player" and not authoritative_player_ready:
		return false
	interest_entities = staged
	interest_session_epoch = String(interest.get("session_epoch", ""))
	interest_epoch = int(interest.get("interest_epoch", -1))
	interest_baseline_id = String(interest.get("baseline_id", ""))
	interest_delta_sequence = 0
	interest_view_hash = String(interest.get("view_hash", ""))
	interest_local_origin = origin.duplicate(true)
	stream_family = "interest"
	baseline_request_pending = false
	_rebuild_registered_celestials()
	_acknowledge_interest()
	replication_state = "ready"
	replication_detail = "INTEREST VIEW CURRENT"
	return true


func _apply_interest_delta(authoritative: Dictionary) -> bool:
	if stream_family != "interest" or interest_delta_sequence < 0 or baseline_request_pending:
		return false
	if int(authoritative.get("event_sequence", -1)) < last_authoritative_event_sequence:
		return false
	var interest_value: Variant = authoritative.get("interest", {})
	if not interest_value is Dictionary:
		return false
	var interest: Dictionary = interest_value
	if (
		not _interest_outer_bindings_valid(authoritative, interest)
		or String(interest.get("frame_kind", "")) != "delta"
		or String(interest.get("session_epoch", "")) != interest_session_epoch
		or int(interest.get("interest_epoch", -1)) != interest_epoch
		or String(interest.get("baseline_id", "")) != interest_baseline_id
		or int(interest.get("delta_sequence", -1)) != interest_delta_sequence + 1
		or String(interest.get("previous_view_hash", "")) != interest_view_hash
		or not _valid_hash(String(interest.get("view_hash", "")))
	):
		return false
	var origin: Dictionary = interest.get("local_origin_address", {})
	var staged: Dictionary = interest_entities.duplicate(true)
	if not _apply_interest_operations(staged, interest, origin, false):
		return false
	if not _rehydrate_interest_entities(staged, origin):
		return false
	var committed := _world_from_interest(authoritative, staged, origin, false)
	if committed.is_empty() or not _private_projection_candidate_valid(committed):
		return false
	_apply_snapshot(committed)
	if session_role_kind == "player" and not authoritative_player_ready:
		return false
	interest_entities = staged
	interest_delta_sequence = int(interest.get("delta_sequence", -1))
	interest_view_hash = String(interest.get("view_hash", ""))
	interest_local_origin = origin.duplicate(true)
	_rebuild_registered_celestials()
	_acknowledge_interest()
	replication_state = "ready"
	replication_detail = "INTEREST VIEW CURRENT"
	return true


func _interest_outer_bindings_valid(authoritative: Dictionary, interest: Dictionary) -> bool:
	var registry_hash := String(registry_snapshot.get("registry_hash", ""))
	var manifest_hash := String(universe_manifest.get("manifest_hash", ""))
	var expected_observer := "bound_player" if session_role_kind == "player" else "public_origin_spectator"
	return (
		int(authoritative.get("projection_schema_version", -1)) == PROJECTION_SCHEMA_VERSION
		and int(authoritative.get("schema_version", -1)) == WORLD_SCHEMA_VERSION
		and String(authoritative.get("content_manifest_version", "")) == CONTENT_MANIFEST_VERSION
		and String(authoritative.get("universe_id", "")) == String(universe_manifest.get("universe_id", ""))
		and String(authoritative.get("universe_manifest_hash", "")) == manifest_hash
		and String(authoritative.get("celestial_registry_hash", "")) == registry_hash
		and int(interest.get("schema_version", -1)) == INTEREST_SCHEMA_VERSION
		and String(interest.get("registry_hash", "")) == registry_hash
		and String(interest.get("universe_manifest_hash", "")) == manifest_hash
		and String(interest.get("observer_class", "")) == expected_observer
		and int(interest.get("canonical_event_sequence", -1)) == int(authoritative.get("event_sequence", -2))
		and int(interest.get("canonical_tick", -1)) == int(authoritative.get("simulation_tick", -2))
		and String(interest.get("canonical_world_hash", "")) == String(authoritative.get("world_hash", ""))
		and _valid_hash(String(authoritative.get("world_hash", "")))
		and _universe_address_valid(authoritative.get("cell_address", {}), universe_manifest)
		and authoritative.get("cell_address", {}) == interest.get("cell_address", {})
		and _universe_address_valid(interest.get("local_origin_address", {}), universe_manifest)
		and interest.get("entered", []) is Array
		and interest.get("replaced", []) is Array
		and interest.get("removed", []) is Array
	)


func _empty_interest_entities() -> Dictionary:
	return {"player": {}, "grid": {}, "voxel_chunk": {}, "death_drop": {}}


func _apply_interest_operations(
	staged: Dictionary, interest: Dictionary, origin: Dictionary, baseline: bool
) -> bool:
	var seen: Dictionary = {}
	for operation_name in ["entered", "replaced"]:
		for projection_value in interest.get(operation_name, []):
			if not projection_value is Dictionary:
				return false
			var projection: Dictionary = projection_value
			var identity := _validated_interest_projection(projection, origin)
			if identity.is_empty():
				return false
			var key := "%s:%s" % [identity["kind"], identity["entity_id"]]
			if seen.has(key):
				return false
			seen[key] = true
			var collection: Dictionary = staged[identity["kind"]]
			var exists := collection.has(identity["entity_id"])
			if (operation_name == "entered" and exists) or (operation_name == "replaced" and not exists):
				return false
			collection[identity["entity_id"]] = identity["value"]
			staged[identity["kind"]] = collection
	for removal_value in interest.get("removed", []):
		if not removal_value is Dictionary:
			return false
		var kind := String(removal_value.get("kind", ""))
		var entity_id := String(removal_value.get("entity_id", ""))
		var key := "%s:%s" % [kind, entity_id]
		if (
			not staged.has(kind)
			or entity_id.is_empty()
			or seen.has(key)
			or not String(removal_value.get("reason", "")) in ["out_of_interest", "destroyed", "transferred"]
		):
			return false
		var collection: Dictionary = staged[kind]
		if not collection.has(entity_id):
			return false
		collection.erase(entity_id)
		staged[kind] = collection
		seen[key] = true
	return not baseline or interest.get("replaced", []).is_empty() and interest.get("removed", []).is_empty()


func _validated_interest_projection(projection: Dictionary, origin: Dictionary) -> Dictionary:
	var kind := String(projection.get("kind", ""))
	var entity_id := String(projection.get("entity_id", ""))
	var payload_value: Variant = projection.get("payload", {})
	if (
		not kind in ["player", "grid", "voxel_chunk", "death_drop"]
		or entity_id.is_empty()
		or int(projection.get("component_schema_version", -1)) != PROJECTION_SCHEMA_VERSION
		or not payload_value is Dictionary
		or String(payload_value.get("entity_kind", "")) != kind
		or not payload_value.get("value", {}) is Dictionary
	):
		return {}
	var value: Dictionary = payload_value.get("value", {}).duplicate(true)
	var natural_id_field: String = {
		"player": "player_id", "grid": "grid_id", "voxel_chunk": "chunk_id", "death_drop": "drop_id",
	}[kind]
	if String(value.get(natural_id_field, "")) != entity_id:
		return {}
	if kind in ["player", "grid", "death_drop"]:
		var position: Variant = _address_relative_m(value.get("address", {}), origin)
		if not position is Vector3:
			return {}
		value["position"] = _protocol_vec3(position)
	return {"kind": kind, "entity_id": entity_id, "value": value}


func _rehydrate_interest_entities(staged: Dictionary, origin: Dictionary) -> bool:
	for kind in ["player", "grid", "death_drop"]:
		var collection: Dictionary = staged[kind]
		for entity_id in collection:
			var value: Dictionary = collection[entity_id]
			var position: Variant = _address_relative_m(value.get("address", {}), origin)
			if not position is Vector3:
				return false
			value["position"] = _protocol_vec3(position)
			collection[entity_id] = value
		staged[kind] = collection
	return true


func _baseline_arrays_match_interest(authoritative: Dictionary, staged: Dictionary) -> bool:
	for mapping in [
		["players", "player", "player_id"],
		["grids", "grid", "grid_id"],
		["voxel_chunks", "voxel_chunk", "chunk_id"],
		["death_drops", "death_drop", "drop_id"],
	]:
		var values: Variant = authoritative.get(mapping[0], [])
		if not values is Array or values.size() != staged[mapping[1]].size():
			return false
		var seen: Dictionary = {}
		for value in values:
			if not value is Dictionary:
				return false
			var entity_id := String(value.get(mapping[2], ""))
			if entity_id.is_empty() or seen.has(entity_id) or not staged[mapping[1]].has(entity_id):
				return false
			seen[entity_id] = true
	return true


func _world_from_interest(
	authoritative: Dictionary, staged: Dictionary, origin: Dictionary, baseline: bool
) -> Dictionary:
	var world := authoritative.duplicate(true) if baseline else snapshot.duplicate(true)
	for field in [
		"projection_schema_version", "schema_version", "content_manifest_version", "universe_id",
		"cell_id", "universe_manifest_hash", "celestial_registry_hash", "cell_address",
		"gravity_body_id", "voxel_body_id", "event_sequence", "simulation_tick", "world_hash", "interest",
	]:
		world[field] = authoritative.get(field)
	if authoritative.has("environment"):
		world["environment"] = authoritative.get("environment", {}).duplicate(true)
		var environment: Dictionary = world["environment"]
		var gravity_body := _registered_body(String(environment.get("celestial_body_id", "")))
		var planet_center: Variant = _address_relative_m(gravity_body.get("center", {}), origin)
		if planet_center is Vector3:
			environment["planet_center"] = _protocol_vec3(planet_center)
		world["environment"] = environment
	if authoritative.has("conservation_valid"):
		world["conservation_valid"] = bool(authoritative.get("conservation_valid", false))
	world["players"] = _ordered_entity_values(staged["player"])
	world["grids"] = _ordered_entity_values(staged["grid"])
	world["voxel_chunks"] = _ordered_entity_values(staged["voxel_chunk"])
	world["death_drops"] = _ordered_entity_values(staged["death_drop"])
	var voxels: Array = []
	for chunk in world["voxel_chunks"]:
		voxels.append_array(chunk.get("voxels", []))
	world["voxels"] = voxels
	var private_value: Variant = authoritative.get("actor_private", null)
	if private_value == null and not baseline:
		private_value = actor_private_snapshot.duplicate(true)
	if private_value is Dictionary and not private_value.is_empty():
		var private_projection: Dictionary = private_value.duplicate(true)
		if not _hydrate_private_spatial_state(private_projection, origin):
			return {}
		world["actor_private"] = private_projection
	var private_motion: Variant = authoritative.get("actor_private_motion", null)
	if private_motion is Dictionary:
		if not world.get("actor_private", {}) is Dictionary:
			return {}
		var motion: Dictionary = private_motion.duplicate(true)
		var position: Variant = _address_relative_m(motion.get("address", {}), origin)
		if not position is Vector3:
			return {}
		motion["position"] = _protocol_vec3(position)
		var private_projection: Dictionary = world["actor_private"]
		var player: Dictionary = private_projection.get("player", {}).duplicate(true)
		if String(player.get("player_id", "")) != String(motion.get("player_id", "")):
			return {}
		for field in motion:
			player[field] = motion[field]
		private_projection["player"] = player
		world["actor_private"] = private_projection
	return world


func _ordered_entity_values(collection: Dictionary) -> Array:
	var ids: Array = collection.keys()
	ids.sort()
	var values: Array = []
	for entity_id in ids:
		values.append(collection[entity_id].duplicate(true))
	return values


func _registered_body(body_id: String) -> Dictionary:
	for body_value in registry_snapshot.get("bodies", []):
		if body_value is Dictionary and String(body_value.get("body_id", "")) == body_id:
			return body_value
	return {}


func _hydrate_private_spatial_state(private_projection: Dictionary, origin: Dictionary) -> bool:
	var player: Variant = private_projection.get("player", {})
	if not player is Dictionary:
		return false
	var player_position: Variant = _address_relative_m(player.get("address", {}), origin)
	if not player_position is Vector3:
		return false
	player["position"] = _protocol_vec3(player_position)
	var environment: Variant = player.get("environment", null)
	if environment is Dictionary:
		var gravity_body := _registered_body(String(environment.get("celestial_body_id", "")))
		var planet_center: Variant = _address_relative_m(gravity_body.get("center", {}), origin)
		if planet_center is Vector3:
			environment["planet_center"] = _protocol_vec3(planet_center)
		player["environment"] = environment
	private_projection["player"] = player
	for drop_value in private_projection.get("death_drops", []):
		if not drop_value is Dictionary:
			return false
		var drop_position: Variant = _address_relative_m(drop_value.get("address", {}), origin)
		if not drop_position is Vector3:
			return false
		drop_value["position"] = _protocol_vec3(drop_position)
	return true


func _private_projection_candidate_valid(world: Dictionary) -> bool:
	if session_role_kind == "spectator":
		return not world.has("actor_private") or world.get("actor_private", {}) == null
	var candidate: Variant = world.get("actor_private", {})
	if not _actor_private_matches(candidate, int(world.get("event_sequence", -1))):
		return false
	var frontier := _protocol_nonnegative_integer(candidate.get("committed_operation_sequence", null))
	var observed_floor := committed_operation_sequence
	if operation_frontier_observed:
		observed_floor = maxi(observed_floor, observed_operation_frontier)
	if operation_frontier_observed and committed_operation_actor_id == bound_player_id and frontier < observed_floor:
		return false
	if not in_flight_mutation.is_empty():
		var pending_sequence := int(in_flight_mutation.get("operation_sequence", 0))
		if pending_sequence <= 0 or frontier < pending_sequence - 1:
			return false
	return true


func _acknowledge_interest() -> void:
	_send_transport({
		"type": "acknowledge_interest",
		"session_epoch": interest_session_epoch,
		"interest_epoch": interest_epoch,
		"baseline_id": interest_baseline_id,
		"delta_sequence": interest_delta_sequence,
		"view_hash": interest_view_hash,
	})


func _request_fresh_interest_baseline(reason: String) -> void:
	if replication_state == "fatal":
		return
	replication_state = "stale"
	replication_detail = reason
	authoritative_player_ready = false
	operation_frontier_ready = false
	mutation_resync_required = true
	baseline_request_pending = true
	interest_delta_sequence = -1
	_set_message("%s // REQUESTING FRESH BASELINE" % reason, true)
	if socket.get_ready_state() == WebSocketPeer.STATE_OPEN:
		_send_transport({"type": "request_snapshot"})


func _client_fatal(reason: String) -> void:
	if smoke_test:
		printerr("VERSE_SMOKE_CLIENT_FATAL %s" % reason)
	replication_state = "fatal"
	replication_detail = reason
	authoritative_player_ready = false
	operation_frontier_ready = false
	mutation_resync_required = true
	_set_message("FATAL CLIENT PROTOCOL ERROR // %s" % reason, true)
	if socket.get_ready_state() == WebSocketPeer.STATE_OPEN:
		socket.close(1002, reason)


func _apply_snapshot(authoritative: Dictionary) -> void:
	if authoritative.is_empty():
		return
	var event_sequence := int(authoritative.get("event_sequence", 0))
	if not _full_snapshot_event_is_current(
		event_sequence, last_authoritative_event_sequence
	):
		return
	var private_candidate: Variant = authoritative.get("actor_private", {})
	snapshot = authoritative.duplicate(true)
	snapshot.erase("actor_private")
	var players: Array = snapshot.get("players", [])
	_sync_remote_players(players)

	var voxels: Array = snapshot.get("voxels", [])
	if _sync_voxel_projection(snapshot.get("voxel_chunks", []), voxels):
		if pending_mine_position != null:
			var mined_coordinate: Vector3i = pending_mine_position
			if not voxel_lookup.has(_coord_key(mined_coordinate)):
				_emit_mining_fragments(Vector3(mined_coordinate))
				pending_mine_position = null
	_rebuild_grids(snapshot.get("grids", []))
	if smoke_test:
		print("VERSE_SMOKE_STRUCTURAL_READY event=%d" % event_sequence)

	var public_player := _player_from_roster(players, bound_player_id)
	var projection_valid := (
		int(snapshot.get("projection_schema_version", 0)) == PROJECTION_SCHEMA_VERSION
		and not public_player.is_empty()
		and _install_actor_private(private_candidate, event_sequence)
	)
	if not projection_valid:
		if smoke_test:
			printerr(
				"VERSE_SMOKE_PRIVATE_PROJECTION_INVALID actor=%s event=%d"
				% [bound_player_id, event_sequence]
			)
		_clear_actor_private_state()
		authoritative_player_ready = false
		last_authoritative_event_sequence = event_sequence
		last_authoritative_simulation_tick = int(snapshot.get("simulation_tick", 0))
		_set_message(
			"PRIVATE INVENTORY LINK UNAVAILABLE // RESYNC REQUIRED",
			true
		)
		return

	var player := _local_player()
	_capture_prediction_gravity({"player": player, "environment": _local_environment()})
	var level := int(player.get("level", 1))
	if level > last_level:
		_set_message("CLEARANCE ADVANCED // SALVAGER LEVEL %d" % level)
		tool_kick = 1.0
	last_level = level
	_apply_authoritative_player(
		player,
		int(snapshot.get("simulation_tick", 0)),
		event_sequence,
		String(snapshot.get("world_hash", "")),
		"snapshot"
	)
	_dispatch_next_mutation()
	if smoke_test and not smoke_visual_ready:
		print("VERSE_SMOKE_VISUAL_ASSERTIONS_START event=%d" % event_sequence)
		if not _run_visual_smoke_assertions():
			get_tree().quit(1)
			return
		smoke_visual_ready = true
		print("VERSE_SMOKE_VISUAL_ASSERTIONS_COMPLETE event=%d" % event_sequence)


func _full_snapshot_event_is_current(incoming_sequence: int, current_sequence: int) -> bool:
	# An equal-sequence snapshot is an intentional complete refresh of the same
	# canonical event. Older snapshots must not roll structural state backward.
	return incoming_sequence >= current_sequence


func _capture_prediction_gravity(authoritative: Dictionary) -> void:
	var environment: Dictionary = authoritative.get("environment", {})
	var player: Dictionary = authoritative.get("player", {})
	var center := _vec3(environment.get("planet_center", {}))
	var surface_radius := float(environment.get("surface_radius_m", 0.0))
	var gravity_sample := _vec3(environment.get("gravity", {}))
	prediction_gravity_fallback = gravity_sample
	prediction_gravity_model_ready = false
	if player.is_empty() or surface_radius <= 0.0 or gravity_sample.length_squared() <= 0.0:
		return
	var sample_position := _vec3(player.get("position", {}))
	var gravitational_parameter := _gravitational_parameter_from_sample(
		gravity_sample, sample_position, center
	)
	if gravitational_parameter <= 0.0:
		return
	prediction_planet_center = center
	prediction_surface_radius = surface_radius
	prediction_gravitational_parameter = gravitational_parameter
	prediction_gravity_model_ready = true


func _gravitational_parameter_from_sample(
	gravity_sample: Vector3, sample_position: Vector3, center: Vector3
) -> float:
	var sample_distance := maxf(sample_position.distance_to(center), 1.0)
	return gravity_sample.length() * sample_distance * sample_distance


func _apply_motion_state(motion: Dictionary) -> void:
	if motion.is_empty():
		return
	var event_sequence := int(motion.get("event_sequence", -1))
	if event_sequence <= last_authoritative_event_sequence:
		return
	if int(motion.get("projection_schema_version", 0)) != PROJECTION_SCHEMA_VERSION:
		_invalidate_private_motion(event_sequence, int(motion.get("simulation_tick", 0)))
		return
	var has_private_motion := motion.has("actor_private")
	var private_motion: Variant = motion.get("actor_private", {})
	if has_private_motion and (
		not private_motion is Dictionary
		or String(private_motion.get("player_id", "")) != bound_player_id
	):
		_invalidate_private_motion(event_sequence, int(motion.get("simulation_tick", 0)))
		return
	var existing_players: Array = snapshot.get("players", []).duplicate(true)
	var motion_players: Array = motion.get("players", [])
	for player_motion in motion_players:
		if not player_motion is Dictionary:
			continue
		var player_id := String(player_motion.get("player_id", ""))
		var found := false
		for index in existing_players.size():
			if String(existing_players[index].get("player_id", "")) != player_id:
				continue
			var merged: Dictionary = existing_players[index].duplicate(true)
			for key in player_motion:
				merged[key] = player_motion[key]
			existing_players[index] = merged
			found = true
			break
		if not found:
			existing_players.append(player_motion.duplicate(true))
	snapshot["players"] = existing_players
	snapshot["event_sequence"] = event_sequence
	snapshot["simulation_tick"] = int(motion.get("simulation_tick", 0))
	snapshot["world_hash"] = String(motion.get("world_hash", ""))
	_update_grid_motion(motion.get("grids", []))
	_sync_remote_players(existing_players)

	var merged_player := _local_player().duplicate(true)
	if merged_player.is_empty():
		last_authoritative_event_sequence = event_sequence
		last_authoritative_simulation_tick = int(motion.get("simulation_tick", 0))
		return
	if has_private_motion:
		for key in private_motion:
			merged_player[key] = private_motion[key]
	else:
		var public_motion := _player_from_roster(motion_players, bound_player_id)
		merged_player = _merge_public_motion_into_private(merged_player, public_motion)
	actor_private_snapshot["player"] = merged_player
	_capture_prediction_gravity({"player": merged_player, "environment": _local_environment()})
	_apply_authoritative_player(
		merged_player,
		int(motion.get("simulation_tick", 0)),
		event_sequence,
		String(motion.get("world_hash", "")),
		"motion_state"
	)


func _invalidate_private_motion(event_sequence: int, simulation_tick: int) -> void:
	_clear_actor_private_state()
	authoritative_player_ready = false
	last_authoritative_event_sequence = event_sequence
	last_authoritative_simulation_tick = simulation_tick
	_set_message("PRIVATE MOTION LINK INVALID // REQUESTING RESYNC", true)
	if socket.get_ready_state() == WebSocketPeer.STATE_OPEN:
		_send_transport({"type": "request_snapshot"})


func _merge_public_motion_into_private(
	private_player: Dictionary, public_motion: Dictionary
) -> Dictionary:
	var merged := private_player.duplicate(true)
	for key in [
		"position", "orientation", "linear_velocity", "angular_velocity", "surface_contact",
	]:
		if public_motion.has(key):
			merged[key] = public_motion[key]
	if public_motion.has("locomotion_kind"):
		var locomotion: Dictionary = merged.get("locomotion", {}).duplicate(true)
		locomotion["kind"] = public_motion["locomotion_kind"]
		merged["locomotion"] = locomotion
	return merged


func _update_grid_motion(grids: Array) -> void:
	for motion_grid in grids:
		var grid_id := String(motion_grid.get("grid_id", ""))
		if grid_id.is_empty() or not grid_lookup.has(grid_id):
			continue
		var grid: Dictionary = grid_lookup[grid_id]
		for key in ["position", "orientation", "linear_velocity", "angular_velocity"]:
			grid[key] = motion_grid.get(key, grid.get(key, {}))
		grid_lookup[grid_id] = grid
		var grid_node: Node3D = grid_node_lookup.get(grid_id, null)
		if grid_node != null:
			grid_node.position = _vec3(grid.get("position", {}))
			grid_node.quaternion = _grid_quaternion(grid)


func _sync_remote_players(players: Array) -> void:
	if players_root == null:
		return
	var visible_ids: Dictionary = {}
	for player in players:
		if not player is Dictionary:
			continue
		var player_id := String(player.get("player_id", ""))
		if player_id.is_empty() or player_id == _controlled_player_id():
			continue
		visible_ids[player_id] = true
		var node: Node3D = remote_player_nodes.get(player_id, null)
		if node == null:
			node = _build_remote_player_visual(player_id)
			remote_player_nodes[player_id] = node
			players_root.add_child(node)
		node.position = _vec3(player.get("position", {}))
		node.quaternion = _quat(player.get("orientation", {}))
		node.visible = true
	for player_id in remote_player_nodes.keys().duplicate():
		if visible_ids.has(player_id):
			continue
		var stale: Node3D = remote_player_nodes[player_id]
		remote_player_nodes.erase(player_id)
		stale.queue_free()


func _remote_player_visuals_match(players: Array) -> bool:
	var expected: Dictionary = {}
	for player in players:
		if not player is Dictionary:
			continue
		var player_id := String(player.get("player_id", ""))
		if player_id.is_empty() or player_id == _controlled_player_id():
			continue
		expected[player_id] = player
	if remote_player_nodes.size() != expected.size():
		return false
	for player_id in expected:
		var node: Node3D = remote_player_nodes.get(player_id, null)
		if node == null or not is_instance_valid(node) or node.get_parent() != players_root:
			return false
		var player: Dictionary = expected[player_id]
		if node.position.distance_to(_vec3(player.get("position", {}))) > 0.001:
			return false
		if node.get_node_or_null("PilotLabel") == null:
			return false
	return true


func _build_remote_player_visual(player_id: String) -> Node3D:
	var root := Node3D.new()
	root.name = "RemotePilot_%s" % player_id
	var suit_material := _armored_material(Color(0.58, 0.66, 0.70), 0.68, 0.54)
	var joint_material := _material(Color(0.055, 0.075, 0.09), 0.52, 0.66)
	var visor_material := _glass_material()

	var torso := _box_visual(Vector3(0.58, 0.68, 0.32), suit_material)
	torso.position = Vector3(0.0, 0.08, 0.0)
	root.add_child(torso)
	var chest := _box_visual(Vector3(0.38, 0.20, 0.06), detail_materials["cyan"])
	chest.position = Vector3(0.0, 0.18, -0.19)
	root.add_child(chest)
	var pelvis := _box_visual(Vector3(0.48, 0.22, 0.30), joint_material)
	pelvis.position = Vector3(0.0, -0.34, 0.0)
	root.add_child(pelvis)
	var backpack := _box_visual(Vector3(0.44, 0.54, 0.20), detail_materials["steel"])
	backpack.position = Vector3(0.0, 0.10, 0.25)
	root.add_child(backpack)

	var helmet := MeshInstance3D.new()
	var helmet_mesh := SphereMesh.new()
	helmet_mesh.radius = 0.25
	helmet_mesh.height = 0.48
	helmet_mesh.radial_segments = 24
	helmet_mesh.rings = 12
	helmet_mesh.material = suit_material
	helmet.mesh = helmet_mesh
	helmet.position = Vector3(0.0, 0.61, 0.0)
	root.add_child(helmet)
	var visor := _box_visual(Vector3(0.34, 0.17, 0.035), visor_material)
	visor.position = Vector3(0.0, 0.63, -0.225)
	root.add_child(visor)

	for side in [-1.0, 1.0]:
		var arm := _cylinder_visual(0.105, 0.62, suit_material)
		arm.position = Vector3(side * 0.39, -0.02, 0.0)
		root.add_child(arm)
		var glove := _box_visual(Vector3(0.20, 0.18, 0.19), joint_material)
		glove.position = Vector3(side * 0.39, -0.39, -0.01)
		root.add_child(glove)
		var leg := _cylinder_visual(0.13, 0.66, suit_material)
		leg.position = Vector3(side * 0.17, -0.69, 0.0)
		root.add_child(leg)
		var boot := _box_visual(Vector3(0.25, 0.20, 0.37), joint_material)
		boot.position = Vector3(side * 0.17, -1.05, -0.07)
		root.add_child(boot)

	var label := Label3D.new()
	label.name = "PilotLabel"
	label.text = player_id
	label.position = Vector3(0.0, 1.06, 0.0)
	label.font_size = 28
	label.outline_size = 8
	label.modulate = Color(0.48, 0.88, 1.0)
	label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
	root.add_child(label)
	return root


func _apply_authoritative_player(
	player: Dictionary,
	simulation_tick: int,
	event_sequence: int,
	_world_hash: String,
	source: String
) -> void:
	if (
		player.is_empty()
		or event_sequence < last_authoritative_event_sequence
		or (event_sequence == last_authoritative_event_sequence and source != "snapshot")
	):
		return
	var incoming_player_id := String(player.get("player_id", ""))
	var incoming_life_state := _player_life_state(player)
	var incoming_epoch := int(player.get("movement_epoch", 0))
	var incoming_ack := int(player.get("last_processed_input_sequence", 0))
	var incoming_received := int(player.get("last_received_input_sequence", incoming_ack))
	var incoming_locomotion: Dictionary = player.get("locomotion", {})
	var lifecycle_reset := (
		not authoritative_player_ready
		or awaiting_reconnect_baseline
		or incoming_player_id != last_player_id
		or incoming_life_state != last_player_life_state
		or incoming_epoch != movement_epoch
		or source == "reconnect"
	)
	var old_present_position := camera.position - _camera_eye_offset()
	var old_present_orientation := camera.quaternion
	var old_history := prediction_history.duplicate(true)
	var old_predicted_simulation_tick := predicted_simulation_tick
	var history_reset := (
		not lifecycle_reset
		and _prediction_history_requires_reset(
			prediction_history_invalid,
			old_history,
			incoming_epoch,
			simulation_tick,
			old_predicted_simulation_tick
		)
	)

	predicted_position = _vec3(player.get("position", {}))
	predicted_orientation = _quat(player.get("orientation", {}))
	predicted_linear_velocity = _vec3(player.get("linear_velocity", {}))
	predicted_angular_velocity = _vec3(player.get("angular_velocity", {}))
	predicted_surface_contact = bool(player.get("surface_contact", false))
	predicted_jump_held = bool(incoming_locomotion.get("jump_held", false))
	predicted_simulation_tick = simulation_tick
	movement_epoch = incoming_epoch
	last_acked_input_sequence = incoming_ack
	next_input_sequence = maxi(next_input_sequence, incoming_received + 1)

	var remaining_controls: Array[Dictionary] = []
	for pending in pending_controls:
		if int(pending.get("movement_epoch", -1)) == incoming_epoch and int(
			pending.get("input_sequence", 0)
		) > incoming_ack:
			remaining_controls.append(pending)
	pending_controls = remaining_controls
	if lifecycle_reset:
		prediction_history.clear()
		pending_controls.clear()
		prediction_history_invalid = false
		next_input_sequence = incoming_received + 1
		current_prediction_input_sequence = incoming_ack
		desired_dampeners = bool(player.get("dampeners", true))
		desired_magnetic_boots = bool(incoming_locomotion.get("magnetic_boots_enabled", false))
		last_sent_control = {}
		control_send_elapsed = CONTROL_SEND_INTERVAL
		_clear_transient_character_input()
		require_neutral_baseline = incoming_life_state == "alive"
	elif history_reset:
		# The missing local timeline cannot be replayed safely. Preserve sequence
		# monotonicity, snap to this canonical state, and supersede any in-flight
		# control with a fresh neutral baseline on the next physics frame.
		prediction_history.clear()
		pending_controls.clear()
		prediction_history_invalid = false
		current_prediction_input_sequence = incoming_ack
		desired_dampeners = bool(player.get("dampeners", true))
		desired_magnetic_boots = bool(incoming_locomotion.get("magnetic_boots_enabled", false))
		last_sent_control = {}
		control_send_elapsed = CONTROL_SEND_INTERVAL
		_clear_transient_character_input()
		require_neutral_baseline = incoming_life_state == "alive"
	else:
		prediction_history.clear()
		var replay_frames: Array[Dictionary] = []
		for frame in old_history:
			if (
				int(frame.get("movement_epoch", -1)) == incoming_epoch
				and (
					int(frame.get("simulation_tick", 0)) > simulation_tick
					or int(frame.get("input_sequence", 0)) > incoming_ack
				)
			):
				replay_frames.append(frame)
		while replay_frames.size() > PREDICTION_HISTORY_LIMIT:
			replay_frames.pop_front()
		for frame in replay_frames:
			current_prediction_input_sequence = int(frame.get("input_sequence", incoming_ack))
			_predict_player_step(frame.get("control", _neutral_player_control()), CHARACTER_FIXED_DELTA, true)
		if pending_controls.is_empty():
			desired_dampeners = bool(player.get("dampeners", true))
		desired_magnetic_boots = bool(incoming_locomotion.get("magnetic_boots_enabled", false))

	var correction_distance := old_present_position.distance_to(predicted_position)
	var target_view_orientation := _player_view_orientation(
		predicted_orientation, incoming_locomotion
	)
	var correction_angle := _quaternion_angular_distance(
		old_present_orientation, target_view_orientation
	)
	if _correction_requires_snap(
		lifecycle_reset or history_reset, correction_distance, correction_angle
	):
		presentation_position_offset = Vector3.ZERO
		presentation_orientation_offset = Quaternion.IDENTITY
		camera.position = predicted_position + _camera_eye_offset()
		camera.quaternion = target_view_orientation
	else:
		presentation_position_offset = old_present_position - predicted_position
		presentation_orientation_offset = (
			old_present_orientation * target_view_orientation.inverse()
		).normalized()

	authoritative_player_ready = true
	awaiting_reconnect_baseline = false
	last_authoritative_event_sequence = event_sequence
	last_authoritative_simulation_tick = simulation_tick
	last_player_id = incoming_player_id
	var previous_life_state := last_player_life_state
	last_player_life_state = incoming_life_state
	if incoming_life_state == "incapacitated" and previous_life_state != "incapacitated":
		_enter_incapacitated_state()
	elif previous_life_state == "incapacitated" and incoming_life_state == "alive":
		recovery_operation = ""
		Input.mouse_mode = Input.MOUSE_MODE_CAPTURED
		_set_message("RECOVERY COMPLETE // EVA SUIT CONTROL RESTORED")
	_check_smoke_control_ack(player)


func _prediction_history_requires_reset(
	invalidated: bool,
	history: Array[Dictionary],
	epoch: int,
	authoritative_tick: int,
	local_predicted_tick: int
) -> bool:
	return invalidated or _prediction_history_has_gap(
		history,
		epoch,
		authoritative_tick,
		local_predicted_tick
	)


func _prediction_buffer_exceeds_limit(size: int) -> bool:
	return size > PREDICTION_HISTORY_LIMIT


func _prediction_history_has_gap(
	history: Array[Dictionary],
	epoch: int,
	authoritative_tick: int,
	local_predicted_tick: int
) -> bool:
	if local_predicted_tick <= authoritative_tick:
		return false
	var expected_tick := authoritative_tick + 1
	for frame in history:
		if int(frame.get("movement_epoch", -1)) != epoch:
			continue
		var frame_tick := int(frame.get("simulation_tick", 0))
		if frame_tick <= authoritative_tick:
			continue
		if frame_tick != expected_tick:
			return true
		expected_tick += 1
	return expected_tick - 1 != local_predicted_tick


func _quaternion_angular_distance(first: Quaternion, second: Quaternion) -> float:
	var normalized_first := first.normalized()
	var normalized_second := second.normalized()
	var magnitude := clampf(absf(normalized_first.dot(normalized_second)), 0.0, 1.0)
	return 2.0 * acos(magnitude)


func _correction_requires_snap(
	lifecycle_or_history_reset: bool,
	position_distance: float,
	orientation_angle: float
) -> bool:
	return (
		lifecycle_or_history_reset
		or position_distance > POSITION_SNAP_DISTANCE
		or orientation_angle > ORIENTATION_SNAP_ANGLE
	)


func _begin_player_resync() -> void:
	authoritative_player_ready = false
	welcome_received = false
	registry_received = false
	replication_state = "loading"
	replication_detail = "WAITING FOR PROTOCOL HANDSHAKE"
	stream_family = ""
	registry_snapshot = {}
	universe_manifest = {}
	interest_entities = _empty_interest_entities()
	interest_session_epoch = ""
	interest_epoch = -1
	interest_baseline_id = ""
	interest_delta_sequence = -1
	interest_view_hash = ""
	interest_local_origin = {}
	baseline_request_pending = false
	bound_player_id = ""
	session_role_kind = ""
	_clear_actor_private_state()
	active_grid_control_id = ""
	awaiting_reconnect_baseline = true
	prediction_history.clear()
	pending_controls.clear()
	prediction_history_invalid = false
	prediction_gravity_model_ready = false
	prediction_gravity_fallback = Vector3.ZERO
	last_sent_control = {}
	control_send_elapsed = 0.0
	_clear_transient_character_input()
	presentation_position_offset = Vector3.ZERO
	presentation_orientation_offset = Quaternion.IDENTITY
	require_neutral_baseline = true
	last_authoritative_event_sequence = -1
	_sync_remote_players([])


func _reset_control_prediction_after_rejection() -> void:
	authoritative_player_ready = false
	awaiting_reconnect_baseline = true
	prediction_history.clear()
	pending_controls.clear()
	prediction_history_invalid = false
	last_sent_control = {}
	control_send_elapsed = 0.0
	_clear_transient_character_input()
	require_neutral_baseline = true


func _sync_voxel_projection(chunks: Array, voxels: Array) -> bool:
	var next_fingerprints: Dictionary = {}
	for chunk_value in chunks:
		if not chunk_value is Dictionary:
			continue
		var chunk: Dictionary = chunk_value
		var chunk_id := String(chunk.get("chunk_id", ""))
		if chunk_id.is_empty():
			continue
		next_fingerprints[chunk_id] = "%s|%s|%s" % [
			String(chunk.get("body_id", "")),
			str(int(chunk.get("revision", -1))),
			JSON.stringify(chunk.get("voxels", [])),
		]
	# Structural smoke fixtures predating chunked interest snapshots still use
	# the flattened array. Keep that path exact without weakening the live
	# per-chunk revision and payload fingerprints.
	if chunks.is_empty() and not voxels.is_empty():
		next_fingerprints["__flattened__"] = JSON.stringify(voxels)
	if (
		next_fingerprints == rendered_voxel_chunk_fingerprints
		and rendered_voxel_count == voxels.size()
	):
		return false
	rendered_voxel_chunk_fingerprints = next_fingerprints
	rendered_voxel_count = voxels.size()
	_rebuild_voxels(voxels)
	return true


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
		if previous.get_parent() == asteroid_root:
			asteroid_root.remove_child(previous)
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
	var next_lookup: Dictionary = {}
	var next_nodes: Dictionary = {}
	var next_fingerprints: Dictionary = {}
	for grid_value in grids:
		if not grid_value is Dictionary:
			continue
		var grid: Dictionary = grid_value
		var grid_id := String(grid.get("grid_id", ""))
		if grid_id.is_empty():
			continue
		var topology_fingerprint := _grid_topology_fingerprint(grid)
		var grid_node: Node3D = grid_node_lookup.get(grid_id, null)
		if (
			grid_node == null
			or String(grid_topology_fingerprints.get(grid_id, "")) != topology_fingerprint
		):
			if grid_node != null:
				if grid_node.get_parent() == grids_root:
					grids_root.remove_child(grid_node)
				grid_node.queue_free()
			grid_node = _create_grid_node(grid)
			grids_root.add_child(grid_node)
		else:
			grid_node.position = _vec3(grid.get("position", {}))
			grid_node.quaternion = _grid_quaternion(grid)
			_sync_grid_power_visual(grid_node, bool(grid.get("power", {}).get("online", false)))
		next_lookup[grid_id] = grid
		next_nodes[grid_id] = grid_node
		next_fingerprints[grid_id] = topology_fingerprint
	for previous_id in grid_node_lookup.keys():
		if next_nodes.has(previous_id):
			continue
		var removed: Node3D = grid_node_lookup.get(previous_id, null)
		if removed == null:
			continue
		if removed.get_parent() == grids_root:
			grids_root.remove_child(removed)
		removed.queue_free()
	grid_lookup = next_lookup
	grid_node_lookup = next_nodes
	grid_topology_fingerprints = next_fingerprints


func _create_grid_node(grid: Dictionary) -> Node3D:
	var grid_node := Node3D.new()
	grid_node.name = String(grid.get("grid_id", "grid"))
	grid_node.position = _vec3(grid.get("position", {}))
	grid_node.quaternion = _grid_quaternion(grid)
	for block_value in grid.get("blocks", []):
		if not block_value is Dictionary:
			continue
		var block: Dictionary = block_value
		var block_visual := _build_block_visual(block)
		var coordinate: Dictionary = block.get("coordinate", {})
		block_visual.position = Vector3(
			float(coordinate.get("x", 0)),
			float(coordinate.get("y", 0)),
			float(coordinate.get("z", 0))
		)
		block_visual.rotation.y = deg_to_rad(float(int(block.get("orientation", 0)) * 90))
		grid_node.add_child(block_visual)
	_sync_grid_power_visual(grid_node, bool(grid.get("power", {}).get("online", false)))
	return grid_node


func _sync_grid_power_visual(grid_node: Node3D, online: bool) -> void:
	var existing: Node = grid_node.get_node_or_null("VersePowerWorkLight")
	if not online:
		if existing != null:
			grid_node.remove_child(existing)
			existing.queue_free()
		return
	if existing != null:
		return
	var work_light := OmniLight3D.new()
	work_light.name = "VersePowerWorkLight"
	work_light.light_color = Color(0.24, 0.72, 1.0)
	work_light.light_energy = 1.35
	work_light.omni_range = 10.0
	work_light.shadow_enabled = true
	work_light.position = Vector3(0.0, 2.2, 0.0)
	grid_node.add_child(work_light)


func _grid_topology_fingerprint(grid: Dictionary) -> String:
	var blocks: Array = grid.get("blocks", []).duplicate(true)
	blocks.sort_custom(func(first: Variant, second: Variant) -> bool:
		return _grid_block_sort_key(first) < _grid_block_sort_key(second)
	)
	var topology: Array = []
	for block_value in blocks:
		if not block_value is Dictionary:
			continue
		var block: Dictionary = block_value
		var coordinate: Dictionary = block.get("coordinate", {})
		var health := int(block.get("health", 1))
		var max_health := maxi(int(block.get("max_health", health)), 1)
		topology.append({
			"block_id": String(block.get("block_id", "")),
			"kind": String(block.get("kind", "structural")),
			"coordinate": {
				"x": int(coordinate.get("x", 0)),
				"y": int(coordinate.get("y", 0)),
				"z": int(coordinate.get("z", 0)),
			},
			"orientation": int(block.get("orientation", 0)),
			"health": health,
			"max_health": max_health,
			"construction_complete": bool(
				block.get("construction_complete", health >= max_health)
			),
		})
	return JSON.stringify(topology)


func _grid_block_sort_key(value: Variant) -> String:
	if not value is Dictionary:
		return ""
	var block: Dictionary = value
	var coordinate: Dictionary = block.get("coordinate", {})
	return "%s|%012d|%012d|%012d" % [
		String(block.get("block_id", "")),
		int(coordinate.get("x", 0)),
		int(coordinate.get("y", 0)),
		int(coordinate.get("z", 0)),
	]


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
		"conveyor":
			for axis in [-0.30, 0.0, 0.30]:
				var roller := _cylinder_visual(0.075, 0.72, detail_materials["steel"])
				roller.rotation_degrees.z = 90.0
				roller.position = Vector3(0.0, axis, -0.56)
				root.add_child(roller)
			for x in [-0.42, 0.42]:
				var belt_rail := _box_visual(Vector3(0.06, 0.78, 0.08), detail_materials["cyan"])
				belt_rail.position = Vector3(x, 0.0, -0.54)
				root.add_child(belt_rail)
			root.add_child(_block_face_label("CONVEYOR", Color(0.30, 0.88, 1.0)))
		"refinery":
			var furnace := _cylinder_visual(0.27, 0.56, detail_materials["amber"])
			furnace.rotation_degrees.x = 90.0
			furnace.position = Vector3(0.0, 0.03, -0.62)
			root.add_child(furnace)
			for x in [-0.34, 0.34]:
				var exhaust := _cylinder_visual(0.075, 0.58, detail_materials["dark"])
				exhaust.position = Vector3(x, 0.31, -0.34)
				root.add_child(exhaust)
			var refinery_glow := _box_visual(Vector3(0.48, 0.09, 0.055), detail_materials["amber"])
			refinery_glow.position = Vector3(0.0, -0.31, -0.58)
			root.add_child(refinery_glow)
			root.add_child(_block_face_label("REFINERY", Color(1.0, 0.52, 0.12)))
		"assembler":
			for x in [-0.25, 0.25]:
				var actuator := _cylinder_visual(0.09, 0.58, detail_materials["steel"])
				actuator.rotation_degrees.x = 90.0
				actuator.position = Vector3(x, 0.18, -0.60)
				root.add_child(actuator)
			var fabrication_bed := _box_visual(Vector3(0.66, 0.18, 0.10), detail_materials["dark"])
			fabrication_bed.position = Vector3(0.0, -0.24, -0.57)
			root.add_child(fabrication_bed)
			var assembler_screen := _box_visual(Vector3(0.42, 0.18, 0.045), detail_materials["green"])
			assembler_screen.position = Vector3(0.0, 0.34, -0.57)
			root.add_child(assembler_screen)
			root.add_child(_block_face_label("ASSEMBLER", Color(0.26, 1.0, 0.61)))
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


func _sample_player_control() -> Dictionary:
	if Input.mouse_mode != Input.MOUSE_MODE_CAPTURED or inventory_open:
		_clear_transient_character_input()
		return _neutral_player_control()
	var player := _local_player()
	var jetpack_enabled := bool(player.get("jetpack_enabled", true))
	var vertical_input := (
		Input.get_action_strength("move_up") - Input.get_action_strength("move_down")
		if jetpack_enabled
		else 0.0
	)
	var linear_input := Vector3(
		Input.get_action_strength("move_right") - Input.get_action_strength("move_left"),
		vertical_input,
		Input.get_action_strength("move_backward") - Input.get_action_strength("move_forward")
	).limit_length(CONTROL_INPUT_SAFE_LIMIT)
	var roll_input := (
		Input.get_action_strength("roll_right") - Input.get_action_strength("roll_left")
	)
	var roll_angular_input := -roll_input
	if not pending_roll_transitions.is_empty():
		roll_angular_input = pending_roll_transitions.front()
	var mouse_delta := mouse_delta_accumulator
	mouse_delta_accumulator = Vector2.ZERO
	var angular_input := _bounded_angular_input(mouse_delta, roll_angular_input)
	return {
		"linear_input": linear_input,
		"angular_input": angular_input,
		"boost": Input.is_action_pressed("move_boost"),
		"jump": not jetpack_enabled and Input.is_action_pressed("move_up"),
		"dampeners": desired_dampeners,
	}


func _bounded_angular_input(mouse_delta: Vector2, roll_angular_input: float) -> Vector3:
	return Vector3(
		-mouse_delta.y * MOUSE_ANGULAR_INPUT_PER_PIXEL,
		-mouse_delta.x * MOUSE_ANGULAR_INPUT_PER_PIXEL,
		roll_angular_input
	).limit_length(CONTROL_INPUT_SAFE_LIMIT)


func _neutral_player_control() -> Dictionary:
	return {
		"linear_input": Vector3.ZERO,
		"angular_input": Vector3.ZERO,
		"boost": false,
		"jump": false,
		"dampeners": desired_dampeners,
	}


func _controls_equal(first: Dictionary, second: Dictionary) -> bool:
	if first.is_empty() or second.is_empty():
		return false
	return (
		(first.get("linear_input", Vector3.ZERO) as Vector3).is_equal_approx(
			second.get("linear_input", Vector3.ZERO) as Vector3
		)
		and (first.get("angular_input", Vector3.ZERO) as Vector3).is_equal_approx(
			second.get("angular_input", Vector3.ZERO) as Vector3
		)
		and bool(first.get("boost", false)) == bool(second.get("boost", false))
		and bool(first.get("jump", false)) == bool(second.get("jump", false))
		and bool(first.get("dampeners", true)) == bool(second.get("dampeners", true))
	)


func _should_send_player_control(control: Dictionary) -> bool:
	return _control_send_due(control, last_sent_control, control_send_elapsed)


func _control_send_due(control: Dictionary, previous: Dictionary, elapsed: float) -> bool:
	return (
		previous.is_empty()
		or not _controls_equal(control, previous)
		or (
			_control_requires_lease_refresh(control)
			and elapsed >= CONTROL_SEND_INTERVAL
		)
	)


func _control_requires_lease_refresh(control: Dictionary) -> bool:
	return (
		(control.get("linear_input", Vector3.ZERO) as Vector3).length_squared() > 0.00000001
		or (control.get("angular_input", Vector3.ZERO) as Vector3).length_squared() > 0.00000001
		or bool(control.get("boost", false))
		or bool(control.get("jump", false))
		or not bool(control.get("dampeners", true))
	)


func _send_player_control(control: Dictionary, force: bool) -> bool:
	if (
		not connected
		or not authoritative_player_ready
		or _local_player_incapacitated()
		or (not force and not _should_send_player_control(control))
	):
		return false
	var bounded_control := {
		"linear_input": (control.get("linear_input", Vector3.ZERO) as Vector3).limit_length(
			CONTROL_INPUT_SAFE_LIMIT
		),
		"angular_input": (control.get("angular_input", Vector3.ZERO) as Vector3).limit_length(
			CONTROL_INPUT_SAFE_LIMIT
		),
		"boost": bool(control.get("boost", false)),
		"jump": bool(control.get("jump", false)),
		"dampeners": bool(control.get("dampeners", true)),
	}
	var sequence := next_input_sequence
	var operation_id := "player-control-%d-%d" % [movement_epoch, sequence]
	var message := _player_control_message(operation_id, movement_epoch, sequence, bounded_control)
	if not _queue_mutation(message):
		return false
	next_input_sequence += 1
	current_prediction_input_sequence = sequence
	last_sent_control = bounded_control.duplicate(true)
	control_send_elapsed = 0.0
	_record_pending_control(movement_epoch, sequence, bounded_control)
	if smoke_test and smoke_visual_ready and smoke_operation.is_empty():
		smoke_operation = operation_id
		smoke_input_sequence = sequence
	return true


func _capture_roll_key_transition(event: InputEventKey) -> void:
	var changed := false
	if event.keycode == KEY_Q and roll_left_held != event.pressed:
		roll_left_held = event.pressed
		changed = true
	elif event.keycode == KEY_E and roll_right_held != event.pressed:
		roll_right_held = event.pressed
		changed = true
	if not changed:
		return
	var roll_input := float(int(roll_right_held) - int(roll_left_held))
	var angular_roll := -roll_input * CONTROL_INPUT_SAFE_LIMIT
	if pending_roll_transitions.is_empty() or not is_equal_approx(
		pending_roll_transitions.back(), angular_roll
	):
		pending_roll_transitions.append(angular_roll)


func _clear_transient_character_input() -> void:
	mouse_delta_accumulator = Vector2.ZERO
	roll_left_held = false
	roll_right_held = false
	pending_roll_transitions.clear()


func _record_pending_control(epoch: int, sequence: int, control: Dictionary) -> void:
	_append_bounded_prediction_entry(pending_controls, {
		"movement_epoch": epoch,
		"input_sequence": sequence,
		"control": control.duplicate(true),
	})


func _append_bounded_prediction_entry(
	buffer: Array[Dictionary],
	entry: Dictionary
) -> void:
	buffer.append(entry)
	while _prediction_buffer_exceeds_limit(buffer.size()):
		prediction_history_invalid = true
		buffer.pop_front()


func _player_control_message(
	operation_id: String,
	epoch: int,
	sequence: int,
	control: Dictionary
) -> Dictionary:
	return {
		"type": "set_player_control",
		"operation_id": operation_id,
		"movement_epoch": epoch,
		"input_sequence": sequence,
		"linear_input": _protocol_vec3(control.get("linear_input", Vector3.ZERO)),
		"angular_input": _protocol_vec3(control.get("angular_input", Vector3.ZERO)),
		"boost": bool(control.get("boost", false)),
		"jump": bool(control.get("jump", false)),
		"dampeners": bool(control.get("dampeners", true)),
	}


func _predict_player_step(control: Dictionary, delta: float, record_history: bool) -> void:
	var player := _local_player()
	var locomotion: Dictionary = player.get("locomotion", {})
	var prediction_control := control.duplicate(true)
	var jump_held := bool(control.get("jump", false))
	prediction_control["jump"] = jump_held and not predicted_jump_held
	predicted_jump_held = jump_held
	var result := _integrate_player_motion(
		predicted_position,
		predicted_orientation,
		predicted_linear_velocity,
		predicted_angular_velocity,
		prediction_control,
		_prediction_gravity(predicted_position),
		bool(player.get("jetpack_enabled", true)),
		delta,
		locomotion
	)
	var proposed_position: Vector3 = result.get("position", predicted_position)
	var locomotion_kind := String(locomotion.get("kind", "eva"))
	var supported := locomotion_kind in ["grounded", "magnetic"] and not bool(
		prediction_control.get("jump", false)
	)
	var sweep := (
		{"position": proposed_position, "collided": true}
		if supported
		else _sweep_player_position(predicted_position, proposed_position)
	)
	predicted_position = sweep.get("position", proposed_position)
	predicted_orientation = result.get("orientation", predicted_orientation)
	predicted_linear_velocity = result.get("linear_velocity", predicted_linear_velocity)
	predicted_angular_velocity = result.get("angular_velocity", predicted_angular_velocity)
	predicted_surface_contact = bool(sweep.get("collided", false))
	if predicted_surface_contact and not supported:
		predicted_linear_velocity *= 0.05
		tool_kick = max(tool_kick, 0.22)
	predicted_simulation_tick += 1
	if record_history:
		_append_bounded_prediction_entry(prediction_history, {
			"movement_epoch": movement_epoch,
			"input_sequence": current_prediction_input_sequence,
			"simulation_tick": predicted_simulation_tick,
			"control": control.duplicate(true),
		})


func _integrate_player_motion(
	position: Vector3,
	orientation: Quaternion,
	linear_velocity: Vector3,
	angular_velocity: Vector3,
	control: Dictionary,
	gravity: Vector3,
	jetpack_enabled: bool,
	delta: float,
	locomotion: Dictionary = {}
) -> Dictionary:
	var linear_input := (control.get("linear_input", Vector3.ZERO) as Vector3).limit_length(1.0)
	var angular_input := (control.get("angular_input", Vector3.ZERO) as Vector3).limit_length(1.0)
	var boost := bool(control.get("boost", false))
	var dampeners := bool(control.get("dampeners", true))
	var body_basis := Basis(orientation)
	var world_input := body_basis * linear_input
	var acceleration := gravity
	if jetpack_enabled and not dampeners:
		var thrust := CHARACTER_BOOST_ACCELERATION if boost else CHARACTER_THRUST_ACCELERATION
		var selected_maximum_speed := (
			CHARACTER_BOOST_MAXIMUM_SPEED if boost else CHARACTER_MAXIMUM_SPEED
		)
		var gravity_velocity := linear_velocity + gravity * delta
		if world_input.length() > 0.00001:
			var speed_ceiling := maxf(gravity_velocity.length(), selected_maximum_speed)
			linear_velocity = (
				gravity_velocity + world_input * thrust * delta
			).limit_length(speed_ceiling)
		else:
			linear_velocity = gravity_velocity
	elif jetpack_enabled:
		var maximum_speed := (
			CHARACTER_BOOST_MAXIMUM_SPEED if boost else CHARACTER_MAXIMUM_SPEED
		)
		var target_velocity := world_input * maximum_speed
		var maximum_acceleration := (
			CHARACTER_BOOST_ACCELERATION
			if world_input.length() > 0.00001 and boost
			else CHARACTER_THRUST_ACCELERATION
			if world_input.length() > 0.00001
			else CHARACTER_LINEAR_DAMPENER_ACCELERATION
		)
		acceleration = ((target_velocity - linear_velocity) / delta).limit_length(
			maximum_acceleration
		)
		linear_velocity += acceleration * delta
	elif String(locomotion.get("kind", "airborne")) in ["grounded", "magnetic"]:
		var up := _vec3(locomotion.get("up", {"x": 0.0, "y": 1.0, "z": 0.0}))
		up = up.normalized() if up.length_squared() > 0.000001 else Vector3.UP
		var walk_input := Vector3(linear_input.x, 0.0, linear_input.z)
		var raw_direction := body_basis * walk_input
		var tangent_direction := raw_direction - up * raw_direction.dot(up)
		var tangent_input := (
			tangent_direction.normalized() * minf(walk_input.length(), 1.0)
			if tangent_direction.length_squared() > 0.000001
			else Vector3.ZERO
		)
		var support_velocity := _prediction_support_velocity(locomotion)
		var relative_velocity := linear_velocity - support_velocity
		var relative_tangent := relative_velocity - up * relative_velocity.dot(up)
		var selected_speed := CHARACTER_SPRINT_SPEED if boost else CHARACTER_WALK_SPEED
		var target_tangent := tangent_input * selected_speed
		var motor_acceleration := (
			CHARACTER_GROUND_ACCELERATION
			if tangent_input.length_squared() > 0.000001
			else CHARACTER_GROUND_BRAKING
		)
		relative_tangent = relative_tangent.move_toward(
			target_tangent, motor_acceleration * delta
		)
		linear_velocity = support_velocity + relative_tangent
		if bool(control.get("jump", false)):
			linear_velocity += up * CHARACTER_JUMP_SPEED
		elif String(locomotion.get("kind", "")) == "magnetic":
			linear_velocity -= up * CHARACTER_MAGNETIC_ADHESION_ACCELERATION * delta
	else:
		linear_velocity += gravity * delta
	linear_velocity = linear_velocity.limit_length(PHYSICS_MAXIMUM_LINEAR_SPEED)

	if jetpack_enabled:
		var world_angular_input := body_basis * angular_input
		if dampeners:
			var target_angular_velocity := (
				world_angular_input * CHARACTER_MAXIMUM_ANGULAR_SPEED
			)
			var angular_acceleration := (
				CHARACTER_ANGULAR_ACCELERATION
				if world_angular_input.length() > 0.00001
				else CHARACTER_ANGULAR_DAMPENER_ACCELERATION
			)
			angular_velocity = angular_velocity.move_toward(
				target_angular_velocity, angular_acceleration * delta
			)
		elif world_angular_input.length() > 0.00001:
			var angular_speed_ceiling := maxf(
				angular_velocity.length(), CHARACTER_MAXIMUM_ANGULAR_SPEED
			)
			angular_velocity = (
				angular_velocity + world_angular_input * CHARACTER_ANGULAR_ACCELERATION * delta
			).limit_length(angular_speed_ceiling)
	else:
		var desired_up := _vec3(
			locomotion.get("up", {"x": 0.0, "y": 1.0, "z": 0.0})
		)
		desired_up = desired_up.normalized() if desired_up.length_squared() > 0.000001 else Vector3.UP
		var current_up := (body_basis * Vector3.UP).normalized()
		var upright_axis := current_up.cross(desired_up)
		if upright_axis.length_squared() <= 0.00000001 and current_up.dot(desired_up) < 0.0:
			upright_axis = (body_basis * Vector3.RIGHT).normalized()
		var target_angular_velocity := (
			desired_up * angular_input.y * CHARACTER_MAXIMUM_ANGULAR_SPEED
			+ upright_axis * CHARACTER_MAXIMUM_ANGULAR_SPEED
		).limit_length(CHARACTER_MAXIMUM_ANGULAR_SPEED)
		angular_velocity = angular_velocity.move_toward(
			target_angular_velocity,
			CHARACTER_UPRIGHT_ALIGNMENT_ACCELERATION * delta
		)
	angular_velocity = angular_velocity.limit_length(PHYSICS_MAXIMUM_ANGULAR_SPEED)
	if angular_velocity.length_squared() > 0.00000001:
		var delta_rotation := Quaternion(
			angular_velocity.normalized(), angular_velocity.length() * delta
		)
		orientation = (delta_rotation * orientation).normalized()
	position += linear_velocity * delta
	return {
		"position": position,
		"orientation": orientation,
		"linear_velocity": linear_velocity,
		"angular_velocity": angular_velocity,
	}


func _prediction_support_velocity(locomotion: Dictionary) -> Vector3:
	var support: Dictionary = locomotion.get("support", {})
	var body_id := String(support.get("body_id", ""))
	if body_id.is_empty() or not grid_lookup.has(body_id):
		return Vector3.ZERO
	var grid: Dictionary = grid_lookup.get(body_id, {})
	var grid_position := _vec3(grid.get("position", {}))
	var grid_basis := _grid_basis(grid)
	var world_anchor := grid_position + grid_basis * _vec3(support.get("local_anchor", {}))
	var linear_velocity := _vec3(grid.get("linear_velocity", {}))
	var angular_velocity := _vec3(grid.get("angular_velocity", {}))
	return linear_velocity + angular_velocity.cross(world_anchor - grid_position)


func _prediction_gravity(position: Vector3) -> Vector3:
	var environment := _local_environment()
	var fallback := prediction_gravity_fallback
	if fallback.length_squared() <= 0.0:
		fallback = _vec3(environment.get("gravity", {}))
	if not prediction_gravity_model_ready:
		return fallback
	return _inverse_square_gravity(
		position,
		prediction_planet_center,
		prediction_surface_radius,
		prediction_gravitational_parameter,
		fallback
	)


func _inverse_square_gravity(
	position: Vector3,
	center: Vector3,
	surface_radius: float,
	gravitational_parameter: float,
	fallback: Vector3
) -> Vector3:
	var radial := position - center
	if (
		surface_radius <= 0.0
		or gravitational_parameter <= 0.0
		or radial.length_squared() <= 0.0001
	):
		return fallback
	var distance := maxf(radial.length(), 1.0)
	var surface_gravity := gravitational_parameter / (surface_radius * surface_radius)
	var gravity_magnitude := minf(
		gravitational_parameter / (distance * distance), surface_gravity * 1.25
	)
	return -radial.normalized() * gravity_magnitude


func _sweep_player_position(start: Vector3, finish: Vector3) -> Dictionary:
	var distance := start.distance_to(finish)
	var steps := maxi(1, ceili(distance / 0.18))
	var last_clear := start
	for index in range(1, steps + 1):
		var sample := start.lerp(finish, float(index) / float(steps))
		if not _player_position_is_clear(sample):
			return {"position": last_clear, "collided": true}
		last_clear = sample
	return {"position": finish, "collided": false}


func _player_position_is_clear(position: Vector3) -> bool:
	var environment := _local_environment()
	var capsule_up := _camera_up()
	var surface_radius := float(environment.get("surface_radius_m", 0.0))
	if surface_radius > 0.0:
		var planet_center := _vec3(environment.get("planet_center", {}))
		var radial := position - planet_center
		var radial_up := radial.normalized() if radial.length_squared() > 0.000001 else Vector3.UP
		var radial_extent := CHARACTER_COLLISION_RADIUS + CHARACTER_CAPSULE_HALF_HEIGHT * absf(
			capsule_up.dot(radial_up)
		)
		if radial.length() < surface_radius + radial_extent:
			return false
	var capsule_centers: Array[Vector3] = []
	for fraction in [-1.0, -0.5, 0.0, 0.5, 1.0]:
		capsule_centers.append(position + capsule_up * CHARACTER_CAPSULE_HALF_HEIGHT * fraction)
	var collision_offsets: Array[Vector3] = [
		Vector3.ZERO,
		Vector3(CHARACTER_COLLISION_RADIUS, 0.0, 0.0),
		Vector3(-CHARACTER_COLLISION_RADIUS, 0.0, 0.0),
		Vector3(0.0, CHARACTER_COLLISION_RADIUS, 0.0),
		Vector3(0.0, -CHARACTER_COLLISION_RADIUS, 0.0),
		Vector3(0.0, 0.0, CHARACTER_COLLISION_RADIUS),
		Vector3(0.0, 0.0, -CHARACTER_COLLISION_RADIUS),
	]
	for center in capsule_centers:
		for offset in collision_offsets:
			var sample: Vector3 = center + offset
			var coordinate := Vector3i(roundi(sample.x), roundi(sample.y), roundi(sample.z))
			if voxel_lookup.has(_coord_key(coordinate)):
				return false
	for grid_id in grid_lookup:
		var grid: Dictionary = grid_lookup[grid_id]
		var inverse_grid_basis := _grid_basis(grid).inverse()
		var grid_position := _vec3(grid.get("position", {}))
		for center in capsule_centers:
			var local_position := inverse_grid_basis * (center - grid_position)
			for block in grid.get("blocks", []):
				var delta := local_position - _coord_vector(block.get("coordinate", {}))
				var closest := Vector3(
					clampf(delta.x, -0.5, 0.5),
					clampf(delta.y, -0.5, 0.5),
					clampf(delta.z, -0.5, 0.5)
				)
				if (delta - closest).length_squared() < CHARACTER_COLLISION_RADIUS * CHARACTER_COLLISION_RADIUS:
					return false
	return true


func _update_player_presentation(delta: float) -> void:
	if not authoritative_player_ready:
		return
	var blend := clampf(delta * 12.0, 0.0, 1.0)
	presentation_position_offset = presentation_position_offset.lerp(Vector3.ZERO, blend)
	presentation_orientation_offset = _shortest_slerp(
		presentation_orientation_offset, Quaternion.IDENTITY, blend
	)
	var locomotion: Dictionary = _local_player().get("locomotion", {})
	var view_orientation := _player_view_orientation(predicted_orientation, locomotion)
	camera.position = predicted_position + presentation_position_offset + _camera_eye_offset()
	camera.quaternion = (presentation_orientation_offset * view_orientation).normalized()
	var boost_amount := clampf(
		predicted_linear_velocity.length() / CHARACTER_BOOST_MAXIMUM_SPEED, 0.0, 1.0
	)
	camera.fov = lerpf(camera.fov, 74.0 + boost_amount * 8.0, minf(delta * 5.0, 1.0))


func _camera_up() -> Vector3:
	var player := _local_player()
	var locomotion: Dictionary = player.get("locomotion", {})
	var kind := String(locomotion.get("kind", "eva"))
	if kind in ["grounded", "magnetic", "airborne"]:
		var authoritative_up := _vec3(locomotion.get("up", {}))
		if authoritative_up.length_squared() > 0.000001:
			return authoritative_up.normalized()
	return (Basis(predicted_orientation) * Vector3.UP).normalized()


func _camera_eye_offset() -> Vector3:
	return _camera_up() * CHARACTER_EYE_OFFSET


func _player_view_orientation(body_orientation: Quaternion, locomotion: Dictionary) -> Quaternion:
	if String(locomotion.get("kind", "eva")) == "eva":
		return body_orientation
	var pitch := float(locomotion.get("view_pitch_radians", 0.0))
	return (body_orientation * Quaternion(Vector3.RIGHT, pitch)).normalized()


func _shortest_slerp(first: Quaternion, second: Quaternion, weight: float) -> Quaternion:
	var adjusted_second := second
	if first.dot(second) < 0.0:
		adjusted_second = Quaternion(-second.x, -second.y, -second.z, -second.w)
	return first.slerp(adjusted_second, weight).normalized()


func _update_target() -> void:
	if _local_player_incapacitated():
		target_hit = {}
		target_voxel = null
		target_block = {}
		target_highlight.visible = false
		build_preview.visible = false
		return
	var origin := camera.global_position
	var direction := -camera.global_transform.basis.z.normalized()
	target_hit = _closest_tool_hit(origin, direction, TARGET_RANGE)
	_set_tool_targets_from_hit(target_hit)
	build_preview.visible = false
	if target_hit.is_empty() or not bool(target_hit.get("has_face", false)):
		target_highlight.visible = false
	elif target_voxel != null:
		target_highlight.visible = true
		target_highlight.global_transform = Transform3D(
			Basis.IDENTITY, Vector3(target_voxel)
		)
	elif not target_block.is_empty():
		target_highlight.visible = true
		target_highlight.global_transform = Transform3D(
			_grid_basis(target_block["grid"]),
			target_block.get("world_position", Vector3.ZERO)
		)
	else:
		target_highlight.visible = false
	if (
		build_mode
		and not target_block.is_empty()
		and _target_grid_owned_by_local()
		and not _block_needs_weld(target_block["block"])
	):
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
	var reconnect_key := InputEventKey.new()
	reconnect_key.keycode = KEY_F5
	reconnect_key.pressed = true
	var reconnect_echo := InputEventKey.new()
	reconnect_echo.keycode = KEY_F5
	reconnect_echo.pressed = true
	reconnect_echo.echo = true
	var contiguous_history: Array[Dictionary] = [
		{"movement_epoch": 7, "simulation_tick": 101},
		{"movement_epoch": 7, "simulation_tick": 102},
	]
	var gapped_history: Array[Dictionary] = [
		{"movement_epoch": 7, "simulation_tick": 101},
		{"movement_epoch": 7, "simulation_tick": 103},
	]
	var small_orientation := Quaternion(Vector3.UP, ORIENTATION_SNAP_ANGLE * 0.5)
	var large_orientation := Quaternion(Vector3.UP, ORIENTATION_SNAP_ANGLE + 0.1)
	var equivalent_identity := Quaternion(0.0, 0.0, 0.0, -1.0)
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
	var life_support_valid := _run_life_support_smoke_assertions()
	var motion_prediction_valid := _run_motion_prediction_smoke_assertions()
	var remote_roster_valid := _remote_player_visuals_match(
		snapshot.get("players", [])
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
		and _reconnect_shortcut(reconnect_key)
		and not _reconnect_shortcut(reconnect_echo)
		and _full_snapshot_event_is_current(42, 42)
		and _full_snapshot_event_is_current(43, 42)
		and not _full_snapshot_event_is_current(41, 42)
		and not _prediction_history_requires_reset(
			false, contiguous_history, 7, 100, 102
		)
		and _prediction_history_requires_reset(false, gapped_history, 7, 100, 103)
		and _prediction_history_requires_reset(true, contiguous_history, 7, 100, 102)
		and not _prediction_buffer_exceeds_limit(PREDICTION_HISTORY_LIMIT)
		and _prediction_buffer_exceeds_limit(PREDICTION_HISTORY_LIMIT + 1)
		and not _correction_requires_snap(
			false,
			POSITION_SNAP_DISTANCE * 0.5,
			_quaternion_angular_distance(Quaternion.IDENTITY, small_orientation)
		)
		and _correction_requires_snap(
			false,
			0.0,
			_quaternion_angular_distance(Quaternion.IDENTITY, large_orientation)
		)
		and _quaternion_angular_distance(
			Quaternion.IDENTITY, equivalent_identity
		) < 0.00001
		and life_support_valid
		and motion_prediction_valid
		and remote_roster_valid
	)
	frame_visual.free()
	damaged_visual.free()
	repaired_visual.free()
	if not valid:
		printerr("VERSE_VISUAL_STATE_FAILED")
		return false
	print(
		"VERSE_VISUAL_STATE_OK frame=frame damaged=armor_damaged repaired=armor_complete inventory_focus=owned reconnect=global reconciliation=bounded remote_roster=visible"
	)
	print("VERSE_EVA_GRAVITY_OK drift=gravity dampeners=compensating")
	return true


func _run_motion_prediction_smoke_assertions() -> bool:
	var drift_control := {
		"linear_input": Vector3.ZERO,
		"angular_input": Vector3.ZERO,
		"boost": false,
		"dampeners": false,
	}
	var dampened_control := drift_control.duplicate(true)
	dampened_control["dampeners"] = true
	var thrust_control := dampened_control.duplicate(true)
	thrust_control["linear_input"] = Vector3(0.0, 0.0, -1.0)
	var roll_control := dampened_control.duplicate(true)
	roll_control["angular_input"] = Vector3(0.0, 0.0, -1.0)
	var drift_thrust_control := drift_control.duplicate(true)
	drift_thrust_control["linear_input"] = Vector3.RIGHT
	var drift_roll_control := drift_control.duplicate(true)
	drift_roll_control["angular_input"] = Vector3(0.0, 0.0, 1.0)
	var gravity_probe := Vector3(0.0, -0.5, 0.0)
	var gravity_surface_radius := 1200.0
	var gravity_surface_strength := 6.2
	var gravity_parameter := gravity_surface_strength * gravity_surface_radius * gravity_surface_radius
	var gravity_reference := _inverse_square_gravity(
		Vector3(gravity_surface_radius * 2.0, 0.0, 0.0),
		Vector3.ZERO,
		gravity_surface_radius,
		gravity_parameter,
		gravity_probe
	)
	var gravity_inward := _inverse_square_gravity(
		Vector3(gravity_surface_radius * 1.5, 0.0, 0.0),
		Vector3.ZERO,
		gravity_surface_radius,
		gravity_parameter,
		gravity_probe
	)
	var gravity_outward := _inverse_square_gravity(
		Vector3(gravity_surface_radius * 3.0, 0.0, 0.0),
		Vector3.ZERO,
		gravity_surface_radius,
		gravity_parameter,
		gravity_probe
	)
	var gravity_capped := _inverse_square_gravity(
		Vector3(gravity_surface_radius * 0.5, 0.0, 0.0),
		Vector3.ZERO,
		gravity_surface_radius,
		gravity_parameter,
		gravity_probe
	)
	var drift := _integrate_player_motion(
		Vector3.ZERO,
		Quaternion.IDENTITY,
		Vector3.ZERO,
		Vector3.ZERO,
		drift_control,
		gravity_probe,
		true,
		CHARACTER_FIXED_DELTA
	)
	var dampened := _integrate_player_motion(
		Vector3.ZERO,
		Quaternion.IDENTITY,
		Vector3.ZERO,
		Vector3.ZERO,
		dampened_control,
		gravity_probe,
		true,
		CHARACTER_FIXED_DELTA
	)
	var rolled := _integrate_player_motion(
		Vector3.ZERO,
		Quaternion.IDENTITY,
		Vector3.ZERO,
		Vector3.ZERO,
		roll_control,
		Vector3.ZERO,
		true,
		CHARACTER_FIXED_DELTA
	)
	var inertial_velocity := Vector3(18.0, 0.0, 0.0)
	var inertial_drift := _integrate_player_motion(
		Vector3.ZERO,
		Quaternion.IDENTITY,
		inertial_velocity,
		Vector3.ZERO,
		drift_control,
		gravity_probe,
		true,
		CHARACTER_FIXED_DELTA
	)
	var normal_cap := _integrate_player_motion(
		Vector3.ZERO,
		Quaternion.IDENTITY,
		Vector3.RIGHT * CHARACTER_MAXIMUM_SPEED,
		Vector3.ZERO,
		drift_thrust_control,
		Vector3.ZERO,
		true,
		CHARACTER_FIXED_DELTA
	)
	var boost_release := _integrate_player_motion(
		Vector3.ZERO,
		Quaternion.IDENTITY,
		Vector3.RIGHT * CHARACTER_BOOST_MAXIMUM_SPEED,
		Vector3.ZERO,
		drift_thrust_control,
		Vector3.ZERO,
		true,
		CHARACTER_FIXED_DELTA
	)
	var angular_inertia := _integrate_player_motion(
		Vector3.ZERO,
		Quaternion.IDENTITY,
		Vector3.ZERO,
		Vector3(0.0, 0.0, 4.0),
		drift_control,
		Vector3.ZERO,
		true,
		CHARACTER_FIXED_DELTA
	)
	var angular_ceiling := _integrate_player_motion(
		Vector3.ZERO,
		Quaternion.IDENTITY,
		Vector3.ZERO,
		Vector3(0.0, 0.0, 4.0),
		drift_roll_control,
		Vector3.ZERO,
		true,
		CHARACTER_FIXED_DELTA
	)
	var grounded_locomotion := {
		"kind": "grounded",
		"up": {"x": 0.0, "y": 1.0, "z": 0.0},
		"support": {},
	}
	var ground_walk_control := dampened_control.duplicate(true)
	ground_walk_control["linear_input"] = Vector3.RIGHT
	ground_walk_control["jump"] = false
	var ground_jump_control := dampened_control.duplicate(true)
	ground_jump_control["jump"] = true
	var ground_roll_control := dampened_control.duplicate(true)
	ground_roll_control["angular_input"] = Vector3(0.0, 0.0, 1.0)
	var ground_walk := _integrate_player_motion(
		Vector3.ZERO, Quaternion.IDENTITY, Vector3.ZERO, Vector3.ZERO,
		ground_walk_control, gravity_probe, false, CHARACTER_FIXED_DELTA,
		grounded_locomotion
	)
	var ground_jump := _integrate_player_motion(
		Vector3.ZERO, Quaternion.IDENTITY, Vector3.ZERO, Vector3.ZERO,
		ground_jump_control, gravity_probe, false, CHARACTER_FIXED_DELTA,
		grounded_locomotion
	)
	var ground_roll := _integrate_player_motion(
		Vector3.ZERO, Quaternion.IDENTITY, Vector3.ZERO, Vector3.ZERO,
		ground_roll_control, gravity_probe, false, CHARACTER_FIXED_DELTA,
		grounded_locomotion
	)
	var radial_ground_locomotion := {
		"kind": "grounded",
		"up": {"x": 1.0, "y": 0.0, "z": 0.0},
		"support": {},
	}
	var radial_orientation := Quaternion(Vector3.BACK, deg_to_rad(-65.0))
	var radial_initial_alignment := (Basis(radial_orientation) * Vector3.UP).dot(Vector3.RIGHT)
	var radial_upright := {
		"position": Vector3.ZERO,
		"orientation": radial_orientation,
		"linear_velocity": Vector3.ZERO,
		"angular_velocity": Vector3.ZERO,
	}
	for _index in range(90):
		radial_upright = _integrate_player_motion(
			radial_upright.get("position", Vector3.ZERO),
			radial_upright.get("orientation", Quaternion.IDENTITY),
			radial_upright.get("linear_velocity", Vector3.ZERO),
			radial_upright.get("angular_velocity", Vector3.ZERO),
			dampened_control,
			Vector3.LEFT * 9.81,
			false,
			CHARACTER_FIXED_DELTA,
			radial_ground_locomotion
		)
	var radial_final_orientation: Quaternion = radial_upright.get(
		"orientation", Quaternion.IDENTITY
	)
	var radial_final_alignment := (
		Basis(radial_final_orientation) * Vector3.UP
	).dot(Vector3.RIGHT)
	var message := _player_control_message("player-control-4-9", 4, 9, roll_control)
	var forbidden_fields := [
		"position", "orientation", "linear_velocity", "angular_velocity",
		"surface_contact", "delta", "delta_seconds",
	]
	var contains_forbidden_field := false
	for field in forbidden_fields:
		contains_forbidden_field = contains_forbidden_field or message.has(field)
	var rolled_orientation: Quaternion = rolled.get("orientation", Quaternion.IDENTITY)
	var equivalent_orientation := Quaternion(
		-rolled_orientation.x,
		-rolled_orientation.y,
		-rolled_orientation.z,
		-rolled_orientation.w
	)
	var shortest_orientation := _shortest_slerp(
		rolled_orientation, equivalent_orientation, 0.5
	)
	var valid: bool = (
		message.get("type", "") == "set_player_control"
		and int(message.get("movement_epoch", 0)) == 4
		and int(message.get("input_sequence", 0)) == 9
		and message.has("jump")
		and not contains_forbidden_field
		and (drift.get("linear_velocity", Vector3.ZERO) as Vector3).is_equal_approx(
			gravity_probe * CHARACTER_FIXED_DELTA
		)
		and (dampened.get("linear_velocity", Vector3.ZERO) as Vector3).is_zero_approx()
		and gravity_inward.length() > gravity_reference.length()
		and gravity_outward.length() < gravity_reference.length()
		and gravity_inward.x < 0.0
		and gravity_outward.x < 0.0
		and is_equal_approx(gravity_capped.length(), gravity_surface_strength * 1.25)
		and _inverse_square_gravity(
			Vector3.ZERO,
			Vector3.ZERO,
			gravity_surface_radius,
			gravity_parameter,
			gravity_probe
		).is_equal_approx(gravity_probe)
		and is_equal_approx(
			_gravitational_parameter_from_sample(
				gravity_reference,
				Vector3(gravity_surface_radius * 2.0, 0.0, 0.0),
				Vector3.ZERO
			),
			gravity_parameter
		)
		and (inertial_drift.get("linear_velocity", Vector3.ZERO) as Vector3).is_equal_approx(
			inertial_velocity + gravity_probe * CHARACTER_FIXED_DELTA
		)
		and is_equal_approx(
			(normal_cap.get("linear_velocity", Vector3.ZERO) as Vector3).length(),
			CHARACTER_MAXIMUM_SPEED
		)
		and is_equal_approx(
			(boost_release.get("linear_velocity", Vector3.ZERO) as Vector3).length(),
			CHARACTER_BOOST_MAXIMUM_SPEED
		)
		and (angular_inertia.get("angular_velocity", Vector3.ZERO) as Vector3).is_equal_approx(
			Vector3(0.0, 0.0, 4.0)
		)
		and is_equal_approx(
			(angular_ceiling.get("angular_velocity", Vector3.ZERO) as Vector3).length(),
			4.0
		)
		and float(rolled_orientation.length_squared()) > 0.999
		and (rolled.get("angular_velocity", Vector3.ZERO) as Vector3).z < 0.0
		and (ground_walk.get("linear_velocity", Vector3.ZERO) as Vector3).x > 0.0
		and is_zero_approx((ground_walk.get("linear_velocity", Vector3.ZERO) as Vector3).y)
		and (ground_jump.get("linear_velocity", Vector3.ZERO) as Vector3).y >= CHARACTER_JUMP_SPEED
		and (ground_roll.get("angular_velocity", Vector3.ZERO) as Vector3).is_zero_approx()
		and radial_final_alignment > radial_initial_alignment + 0.07
		and radial_final_alignment > 0.98
		and _controls_equal(dampened_control, dampened_control.duplicate(true))
		and not _controls_equal(dampened_control, roll_control)
		and _control_send_due(dampened_control, thrust_control, 0.0)
		and not _control_send_due(dampened_control, dampened_control, 0.05)
		and not _control_send_due(
			dampened_control, dampened_control, CONTROL_SEND_INTERVAL
		)
		and _control_send_due(roll_control, roll_control, CONTROL_SEND_INTERVAL)
		and absf(shortest_orientation.dot(rolled_orientation)) > 0.999
	)
	if not valid:
		printerr("VERSE_CHARACTER_PREDICTION_FAILED")
		return false
	print(
		"VERSE_CHARACTER_PREDICTION_OK input_only=true roll=eva_only ground=walk_jump radial=upright fixed_step=60hz drift=inertial caps=preserved"
	)
	return true


func _run_life_support_smoke_assertions() -> bool:
	var threshold_player := {
		"player_id": "smoke-player",
		"life_state": {"kind": "alive"},
		"suit_oxygen_milli": 200,
		"critical_oxygen_milli": 200,
		"helmet_closed": true,
	}
	var critical_player := threshold_player.duplicate(true)
	critical_player["suit_oxygen_milli"] = 199
	var zero_but_alive_player := threshold_player.duplicate(true)
	zero_but_alive_player["suit_oxygen_milli"] = 0
	var incapacitated_player := threshold_player.duplicate(true)
	incapacitated_player["suit_oxygen_milli"] = 0
	incapacitated_player["life_state"] = {
		"kind": "incapacitated",
		"death_id": "death-smoke-oxygen",
		"cause": {"kind": "oxygen_depleted"},
	}

	_update_life_support_interface(critical_player)
	var critical_ui_visible := (
		critical_oxygen_panel.visible
		and not incapacitated_overlay.visible
		and critical_oxygen_label.text.contains("O₂ CRITICAL")
	)
	_update_life_support_interface(incapacitated_player)
	var incapacitated_ui_visible := (
		incapacitated_overlay.visible
		and not critical_oxygen_panel.visible
		and recovery_button.text.contains("[ENTER]")
		and incapacitated_detail_label.text.contains("OXYGEN RESERVE DEPLETED")
	)
	var authoritative_player := _local_player()
	_update_life_support_interface(authoritative_player)

	var valid := (
		_life_support_display_state(threshold_player) == "normal"
		and _life_support_display_state(critical_player) == "critical"
		and _life_support_display_state(zero_but_alive_player) == "critical"
		and _player_controls_enabled(zero_but_alive_player)
		and _life_support_display_state(incapacitated_player) == "incapacitated"
		and not _player_controls_enabled(incapacitated_player)
		and critical_ui_visible
		and incapacitated_ui_visible
	)
	if not valid:
		printerr("VERSE_LIFE_SUPPORT_STATE_FAILED")
		return false
	print("VERSE_LIFE_SUPPORT_UI_OK critical=visible incapacitated=canonical recovery=enter")
	return true


func _check_smoke_control_ack(player: Dictionary) -> void:
	if (
		not smoke_test
		or not smoke_visual_ready
		or smoke_operation.is_empty()
		or not smoke_receipt_received
		or int(player.get("last_processed_input_sequence", 0)) < smoke_input_sequence
	):
		return
	print(
		"VERSE_SMOKE_OK event=%d input_sequence=%d"
		% [last_authoritative_event_sequence, smoke_input_sequence]
	)
	get_tree().quit(0)


func _closest_tool_hit(
	origin: Vector3, direction: Vector3, maximum_distance := TARGET_RANGE
) -> Dictionary:
	if direction.length() <= TOOL_DIRECTION_EPSILON or maximum_distance < 0.0:
		return {}
	var ray_direction := direction.normalized()
	var best := _raymarch_voxel_hit(origin, ray_direction, maximum_distance)
	var grid_ids: Array = grid_lookup.keys()
	grid_ids.sort()
	for grid_id_value in grid_ids:
		var grid_id := String(grid_id_value)
		var grid: Dictionary = grid_lookup[grid_id_value]
		var grid_position := _vec3(grid.get("position", {}))
		var grid_basis := _grid_basis(grid)
		var inverse_basis := grid_basis.inverse()
		var local_origin := inverse_basis * (origin - grid_position)
		var local_direction := inverse_basis * ray_direction
		var blocks: Array = grid.get("blocks", []).duplicate()
		blocks.sort_custom(func(first: Dictionary, second: Dictionary) -> bool:
			return String(first.get("block_id", "")) < String(second.get("block_id", ""))
		)
		for block in blocks:
			var local_center := _coord_vector(block.get("coordinate", {}))
			var intersection := _ray_unit_box_hit(
				local_origin, local_direction, local_center, maximum_distance
			)
			if intersection.is_empty():
				continue
			var local_normal: Vector3 = intersection.get("normal", Vector3.ZERO)
			var distance := float(intersection.get("distance", 0.0))
			var candidate := {
				"kind": "block",
				"distance": distance,
				"has_face": not local_normal.is_zero_approx(),
				"hit_position": origin + ray_direction * distance,
				"local_normal": local_normal,
				"world_normal": (grid_basis * local_normal).normalized()
				if not local_normal.is_zero_approx()
				else Vector3.ZERO,
				"grid_id": grid_id,
				"grid": grid,
				"block": block,
				"world_position": grid_position + grid_basis * local_center,
			}
			if _tool_hit_is_better(candidate, best):
				best = candidate
	return best


func _raymarch_voxel_hit(
	origin: Vector3, direction: Vector3, maximum_distance: float
) -> Dictionary:
	var touching := _origin_touching_voxel_hit(origin)
	if not touching.is_empty():
		return touching
	var coordinate := _voxel_cell_at(origin)
	var step := Vector3i(
		_ray_axis_step(direction.x),
		_ray_axis_step(direction.y),
		_ray_axis_step(direction.z)
	)
	var next_crossing := Vector3(
		_first_voxel_boundary_distance(origin.x, direction.x, coordinate.x, step.x),
		_first_voxel_boundary_distance(origin.y, direction.y, coordinate.y, step.y),
		_first_voxel_boundary_distance(origin.z, direction.z, coordinate.z, step.z)
	)
	var crossing_delta := Vector3(
		_ray_crossing_delta(direction.x),
		_ray_crossing_delta(direction.y),
		_ray_crossing_delta(direction.z)
	)
	var best: Dictionary = {}
	for _index in TOOL_DDA_MAX_STEPS:
		for touched_coordinate in _parallel_boundary_voxel_cells(coordinate, origin, step):
			if voxel_lookup.has(_coord_key(touched_coordinate)):
				var intersection := _ray_unit_box_hit(
					origin, direction, Vector3(touched_coordinate), maximum_distance
				)
				if not intersection.is_empty():
					var normal: Vector3 = intersection.get("normal", Vector3.ZERO)
					var distance := float(intersection.get("distance", 0.0))
					var candidate := {
						"kind": "voxel",
						"distance": distance,
						"has_face": not normal.is_zero_approx(),
						"hit_position": origin + direction * distance,
						"local_normal": normal,
						"world_normal": normal,
						"coordinate": touched_coordinate,
					}
					if _tool_hit_is_better(candidate, best):
						best = candidate

		var axis := _first_crossing_axis(next_crossing)
		var next_distance := _vector_axis(next_crossing, axis)
		if next_distance > maximum_distance + TOOL_HIT_EPSILON:
			break
		if (
			not best.is_empty()
			and next_distance > float(best.get("distance", 0.0)) + TOOL_HIT_EPSILON
		):
			break
		coordinate = _step_coordinate_axis(coordinate, step, axis)
		next_crossing = _advance_vector_axis(next_crossing, crossing_delta, axis)
	return best


func _origin_touching_voxel_hit(origin: Vector3) -> Dictionary:
	var x_coordinates := _voxel_axis_origin_cells(origin.x)
	var y_coordinates := _voxel_axis_origin_cells(origin.y)
	var z_coordinates := _voxel_axis_origin_cells(origin.z)
	var candidates: Array[Vector3i] = []
	for x in x_coordinates:
		for y in y_coordinates:
			for z in z_coordinates:
				var coordinate := Vector3i(int(x), int(y), int(z))
				if voxel_lookup.has(_coord_key(coordinate)):
					candidates.append(coordinate)
	if candidates.is_empty():
		return {}
	candidates.sort_custom(func(first: Vector3i, second: Vector3i) -> bool:
		return _voxel_coordinate_less(first, second)
	)
	var coordinate: Vector3i = candidates.front()
	return {
		"kind": "voxel",
		"distance": 0.0,
		"has_face": false,
		"hit_position": origin,
		"local_normal": Vector3.ZERO,
		"world_normal": Vector3.ZERO,
		"coordinate": coordinate,
	}


func _ray_unit_box_hit(
	origin: Vector3, direction: Vector3, center: Vector3, maximum_distance: float
) -> Dictionary:
	var relative_origin := origin - center
	var near_distance := -INF
	var far_distance := INF
	var entry_normal := Vector3.ZERO
	for axis in 3:
		var axis_origin := _vector_axis(relative_origin, axis)
		var axis_direction := _vector_axis(direction, axis)
		if absf(axis_direction) <= TOOL_DIRECTION_EPSILON:
			if axis_origin < -0.5 or axis_origin > 0.5:
				return {}
			continue
		var first := (-0.5 - axis_origin) / axis_direction
		var second := (0.5 - axis_origin) / axis_direction
		var axis_near := minf(first, second)
		var axis_far := maxf(first, second)
		var axis_normal := _axis_vector(axis) * (-1.0 if axis_direction > 0.0 else 1.0)
		if axis_near > near_distance + TOOL_HIT_EPSILON:
			near_distance = axis_near
			entry_normal = axis_normal
		far_distance = minf(far_distance, axis_far)
		if near_distance > far_distance + TOOL_HIT_EPSILON:
			return {}

	if far_distance < -TOOL_HIT_EPSILON:
		return {}
	var distance := maxf(near_distance, 0.0)
	if distance > maximum_distance + TOOL_HIT_EPSILON:
		return {}
	return {
		"distance": distance,
		"normal": entry_normal if distance > TOOL_HIT_EPSILON else Vector3.ZERO,
	}


func _tool_hit_is_better(candidate: Dictionary, current: Dictionary) -> bool:
	if current.is_empty():
		return true
	var candidate_distance := float(candidate.get("distance", INF))
	var current_distance := float(current.get("distance", INF))
	if candidate_distance < current_distance - TOOL_HIT_EPSILON:
		return true
	if candidate_distance > current_distance + TOOL_HIT_EPSILON:
		return false
	var candidate_kind := String(candidate.get("kind", ""))
	var current_kind := String(current.get("kind", ""))
	if candidate_kind != current_kind:
		return candidate_kind == "block"
	if candidate_kind == "block":
		var candidate_grid := String(candidate.get("grid_id", ""))
		var current_grid := String(current.get("grid_id", ""))
		if candidate_grid != current_grid:
			return candidate_grid < current_grid
		return (
			String(candidate.get("block", {}).get("block_id", ""))
			< String(current.get("block", {}).get("block_id", ""))
		)
	return _voxel_coordinate_less(
		candidate.get("coordinate", Vector3i.ZERO),
		current.get("coordinate", Vector3i.ZERO)
	)


func _set_tool_targets_from_hit(hit: Dictionary) -> void:
	target_voxel = null
	target_block = {}
	if hit.is_empty() or not bool(hit.get("has_face", false)):
		return
	if String(hit.get("kind", "")) == "voxel":
		target_voxel = hit.get("coordinate", null)
	elif String(hit.get("kind", "")) == "block":
		target_block = hit


func _voxel_cell_at(point: Vector3) -> Vector3i:
	return Vector3i(
		floori(point.x + 0.5), floori(point.y + 0.5), floori(point.z + 0.5)
	)


func _voxel_axis_origin_cells(value: float) -> Array[int]:
	var primary := floori(value + 0.5)
	var coordinates: Array[int] = [primary]
	if absf(value - (float(primary) - 0.5)) <= TOOL_DIRECTION_EPSILON:
		coordinates.append(primary - 1)
	coordinates.sort()
	return coordinates


func _parallel_boundary_voxel_cells(
	coordinate: Vector3i, origin: Vector3, step: Vector3i
) -> Array[Vector3i]:
	var x_coordinates := _parallel_boundary_axis_cells(coordinate.x, origin.x, step.x)
	var y_coordinates := _parallel_boundary_axis_cells(coordinate.y, origin.y, step.y)
	var z_coordinates := _parallel_boundary_axis_cells(coordinate.z, origin.z, step.z)
	var coordinates: Array[Vector3i] = []
	for x in x_coordinates:
		for y in y_coordinates:
			for z in z_coordinates:
				coordinates.append(Vector3i(int(x), int(y), int(z)))
	coordinates.sort_custom(func(first: Vector3i, second: Vector3i) -> bool:
		return _voxel_coordinate_less(first, second)
	)
	return coordinates


func _parallel_boundary_axis_cells(
	coordinate: int, origin: float, step: int
) -> Array[int]:
	if (
		step == 0
		and absf((origin + 0.5) - roundf(origin + 0.5)) <= TOOL_DIRECTION_EPSILON
	):
		return [coordinate - 1, coordinate]
	return [coordinate]


func _ray_axis_step(value: float) -> int:
	if value > TOOL_DIRECTION_EPSILON:
		return 1
	if value < -TOOL_DIRECTION_EPSILON:
		return -1
	return 0


func _first_voxel_boundary_distance(
	origin: float, direction: float, coordinate: int, step: int
) -> float:
	if step == 0:
		return INF
	var boundary := float(coordinate) + (0.5 if step > 0 else -0.5)
	return (boundary - origin) / direction


func _ray_crossing_delta(direction: float) -> float:
	return INF if absf(direction) <= TOOL_DIRECTION_EPSILON else absf(1.0 / direction)


func _first_crossing_axis(crossing: Vector3) -> int:
	if crossing.x <= crossing.y and crossing.x <= crossing.z:
		return 0
	if crossing.y <= crossing.z:
		return 1
	return 2


func _vector_axis(value: Vector3, axis: int) -> float:
	if axis == 0:
		return value.x
	if axis == 1:
		return value.y
	return value.z


func _axis_vector(axis: int) -> Vector3:
	if axis == 0:
		return Vector3.RIGHT
	if axis == 1:
		return Vector3.UP
	return Vector3.BACK


func _step_coordinate_axis(coordinate: Vector3i, step: Vector3i, axis: int) -> Vector3i:
	if axis == 0:
		return coordinate + Vector3i(step.x, 0, 0)
	if axis == 1:
		return coordinate + Vector3i(0, step.y, 0)
	return coordinate + Vector3i(0, 0, step.z)


func _advance_vector_axis(value: Vector3, delta: Vector3, axis: int) -> Vector3:
	if axis == 0:
		return Vector3(value.x + delta.x, value.y, value.z)
	if axis == 1:
		return Vector3(value.x, value.y + delta.y, value.z)
	return Vector3(value.x, value.y, value.z + delta.z)


func _voxel_coordinate_less(first: Vector3i, second: Vector3i) -> bool:
	if first.x != second.x:
		return first.x < second.x
	if first.y != second.y:
		return first.y < second.y
	return first.z < second.z


func _update_tool_action(delta: float) -> void:
	if inventory_open or _local_player_incapacitated():
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
	elif (
		holding_primary
		and build_mode
		and not target_block.is_empty()
		and _target_grid_owned_by_local()
	):
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
	var motion := clampf(predicted_linear_velocity.length() / CHARACTER_MAXIMUM_SPEED, 0.0, 1.5)
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
	if not target_hit.is_empty():
		return target_hit.get(
			"hit_position", camera.global_position - camera.global_transform.basis.z * 2.0
		)
	return camera.global_position - camera.basis.z * 2.0


func _mine_target_voxel() -> void:
	if target_voxel == null:
		_set_message("Aim at an asteroid voxel within mining range", true)
		return
	pending_mine_position = target_voxel
	_queue_mutation({
		"type": "mine_voxel",
		"operation_id": _operation_id("mine"),
		"coordinate": _protocol_ivec3(target_voxel),
	})


func _damage_target_block() -> void:
	if target_block.is_empty():
		_set_message("Aim at a grid block to apply test damage", true)
		return
	var block: Dictionary = target_block["block"]
	_queue_mutation({
		"type": "damage_block",
		"operation_id": _operation_id("damage"),
		"grid_id": target_block["grid_id"],
		"block_id": block.get("block_id", ""),
	})


func _build_selected_block() -> void:
	if target_block.is_empty():
		_set_message("Aim at a grid block before building", true)
		return
	if not _target_grid_owned_by_local():
		_report_foreign_grid_access(target_block.get("grid", {}))
		return
	var coordinate := _build_coordinate()
	_queue_mutation({
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
	if not _target_grid_owned_by_local():
		_report_foreign_grid_access(target_block.get("grid", {}))
		return
	var block: Dictionary = target_block["block"]
	if not _block_needs_weld(block):
		_set_message("Target block is already at full integrity", true)
		return
	_queue_mutation({
		"type": "weld_block",
		"operation_id": _operation_id("weld"),
		"grid_id": target_block["grid_id"],
		"block_id": block.get("block_id", ""),
	})


func _build_coordinate() -> Vector3i:
	if target_block.is_empty():
		return Vector3i.ZERO
	var block: Dictionary = target_block["block"]
	var current := _coord_i(block.get("coordinate", {}))
	var local_normal: Vector3 = target_block.get("local_normal", Vector3.ZERO)
	if local_normal.is_zero_approx():
		return current
	var offset := Vector3i(
		roundi(local_normal.x), roundi(local_normal.y), roundi(local_normal.z)
	)
	return current + offset


func _toggle_anchor() -> void:
	var grid_id := _owned_grid_for_command()
	if grid_id.is_empty():
		return
	_queue_mutation({
		"type": "toggle_grid_anchor",
		"operation_id": _operation_id("anchor"),
		"grid_id": grid_id,
	})


func _move_target_grid() -> void:
	var grid_id := _owned_grid_for_command()
	if grid_id.is_empty():
		return
	var direction := -camera.global_transform.basis.z.normalized()
	var grid: Dictionary = grid_lookup.get(grid_id, {})
	var local_direction := (_grid_basis(grid).inverse() * direction).limit_length(0.999)
	if _queue_mutation({
		"type": "set_grid_control",
		"operation_id": _operation_id("grid-control"),
		"grid_id": grid_id,
		"linear_input": _protocol_vec3(local_direction),
		"angular_input": _protocol_vec3(Vector3(0.0, 0.24, 0.0)),
		"dampeners": true,
	}):
		active_grid_control_id = grid_id


func _stop_target_grid() -> void:
	var grid_id := _take_active_grid_control_id()
	if grid_id.is_empty():
		return
	_queue_mutation({
		"type": "set_grid_control",
		"operation_id": _operation_id("grid-stop"),
		"grid_id": grid_id,
		"linear_input": _protocol_vec3(Vector3.ZERO),
		"angular_input": _protocol_vec3(Vector3.ZERO),
		"dampeners": true,
	})


func _take_active_grid_control_id() -> String:
	var grid_id := active_grid_control_id
	active_grid_control_id = ""
	return grid_id


func _refine_ore() -> void:
	_queue_physical_production("refining")


func _craft_component() -> void:
	_queue_physical_production("component")


func _queue_physical_production(recipe: String) -> void:
	var route := _production_route(recipe)
	if route.is_empty():
		var machine_name := "REFINERY" if recipe == "refining" else "ASSEMBLER"
		_set_message(
			"NO AUTHORIZED %s ROUTE // OWNED COMPLETE MACHINE AND SAME-GRID CARGO REQUIRED"
			% machine_name,
			true
		)
		return
	_queue_mutation({
		"type": "queue_production",
		"operation_id": _operation_id("production-%s" % recipe),
		"machine_block_id": route.get("machine_block_id", ""),
		"recipe": recipe,
		"batches": 1,
		"source_inventory_id": route.get("inventory_id", ""),
		"destination_inventory_id": route.get("inventory_id", ""),
	})
	_set_message(
		"%s BATCH QUEUED // %s" % [recipe.to_upper(), route.get("machine_block_id", "")]
	)


func _production_route(recipe: String) -> Dictionary:
	var machine_kind := "refinery" if recipe == "refining" else "assembler" if recipe == "component" else ""
	if machine_kind.is_empty():
		return {}
	var machines := _owned_machine_candidates(machine_kind)
	var cargo := _owned_cargo_candidates()
	for machine in machines:
		for candidate in cargo:
			if String(candidate.get("grid_id", "")) == String(machine.get("grid_id", "")):
				return {
					"grid_id": machine.get("grid_id", ""),
					"machine_block_id": machine.get("block_id", ""),
					"inventory_id": candidate.get("inventory_id", ""),
				}
	return {}


func _owned_machine_candidates(kind: String) -> Array[Dictionary]:
	var candidates: Array[Dictionary] = []
	var targeted_grid_id := _targeted_owned_grid_id()
	for grid_value in snapshot.get("grids", []):
		if not grid_value is Dictionary:
			continue
		var grid: Dictionary = grid_value
		if not _grid_owned_by_local(grid):
			continue
		var grid_id := String(grid.get("grid_id", ""))
		for block_value in grid.get("blocks", []):
			if not block_value is Dictionary:
				continue
			var block: Dictionary = block_value
			if (
				String(block.get("kind", "")) != kind
				or not bool(block.get("construction_complete", false))
				or int(block.get("health", 0)) <= 0
			):
				continue
			candidates.append({
				"grid_id": grid_id,
				"block_id": String(block.get("block_id", "")),
				"powered": bool(grid.get("power", {}).get("online", false)),
				"targeted": grid_id == targeted_grid_id,
			})
	candidates.sort_custom(func(first: Dictionary, second: Dictionary) -> bool:
		if bool(first.get("targeted", false)) != bool(second.get("targeted", false)):
			return bool(first.get("targeted", false))
		var first_grid := String(first.get("grid_id", ""))
		var second_grid := String(second.get("grid_id", ""))
		if first_grid != second_grid:
			return first_grid < second_grid
		return String(first.get("block_id", "")) < String(second.get("block_id", ""))
	)
	return candidates


func _transfer_to_or_from_cargo(reverse: bool) -> void:
	var cargo_id := _selected_owned_cargo_inventory()
	if cargo_id.is_empty():
		_set_message("NO AUTHORIZED CARGO LINK", true)
		return
	var suit_id := _local_inventory_id()
	if suit_id.is_empty():
		_set_message("No authoritative suit inventory is available", true)
		return
	_queue_mutation({
		"type": "transfer_inventory",
		"operation_id": _operation_id("transfer"),
		"source_inventory_id": cargo_id if reverse else suit_id,
		"destination_inventory_id": suit_id if reverse else cargo_id,
		"resource": "ore",
		"quantity": 1,
	})


func _set_inventory_open(open: bool) -> void:
	if open and _local_player_incapacitated():
		return
	inventory_open = open
	inventory_overlay.visible = open
	build_mode = false if open else build_mode
	action_charge = 0.0
	_clear_transient_character_input()
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE if open else Input.MOUSE_MODE_CAPTURED
	_set_message(
		"Engineering inventory terminal online" if open else "Engineering terminal closed"
	)


func _enter_incapacitated_state() -> void:
	predicted_linear_velocity = Vector3.ZERO
	predicted_angular_velocity = Vector3.ZERO
	prediction_history.clear()
	pending_controls.clear()
	last_sent_control = {}
	_clear_transient_character_input()
	require_neutral_baseline = false
	action_charge = 0.0
	action_target_key = ""
	action_cooldown = 0.0
	build_mode = false
	active_grid_control_id = ""
	recovery_operation = ""
	if inventory_open:
		inventory_open = false
		inventory_overlay.visible = false
	get_viewport().gui_release_focus()
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	_set_message("LIFE SUPPORT FAILURE // AWAITING RECOVERY REQUEST", true)


func _request_recovery() -> void:
	if not _local_player_incapacitated():
		return
	if not recovery_operation.is_empty():
		_set_message("Recovery request pending authoritative response")
		return
	if socket.get_ready_state() != WebSocketPeer.STATE_OPEN:
		_set_message("Recovery unavailable while disconnected — press F5", true)
		return
	recovery_operation = _operation_id("respawn")
	if not _queue_mutation({
		"type": "respawn_player",
		"operation_id": recovery_operation,
	}):
		recovery_operation = ""
		return
	_set_message("Recovery requested // awaiting authoritative spawn")


func _toggle_jetpack() -> void:
	var player := _local_player()
	_queue_mutation({
		"type": "set_suit_mode",
		"operation_id": _operation_id("suit"),
		"helmet_closed": bool(player.get("helmet_closed", true)),
		"jetpack_enabled": not bool(player.get("jetpack_enabled", true)),
		"magnetic_boots_enabled": desired_magnetic_boots,
	})


func _toggle_magnetic_boots() -> void:
	var player := _local_player()
	desired_magnetic_boots = not desired_magnetic_boots
	_queue_mutation({
		"type": "set_suit_mode",
		"operation_id": _operation_id("suit"),
		"helmet_closed": bool(player.get("helmet_closed", true)),
		"jetpack_enabled": bool(player.get("jetpack_enabled", true)),
		"magnetic_boots_enabled": desired_magnetic_boots,
	})
	_set_message("Magnetic boots %s" % ("armed" if desired_magnetic_boots else "off"))


func _toggle_helmet() -> void:
	var player := _local_player()
	_queue_mutation({
		"type": "set_suit_mode",
		"operation_id": _operation_id("suit"),
		"helmet_closed": not bool(player.get("helmet_closed", true)),
		"jetpack_enabled": bool(player.get("jetpack_enabled", true)),
		"magnetic_boots_enabled": desired_magnetic_boots,
	})


func _transfer_inventory_resource(resource: String, reverse: bool, all: bool) -> void:
	var cargo_id := _selected_owned_cargo_inventory()
	if cargo_id.is_empty():
		_set_message("NO AUTHORIZED CARGO LINK", true)
		return
	var suit_id := _local_inventory_id()
	if suit_id.is_empty():
		_set_message("No authoritative suit inventory is available", true)
		return
	var source_id := cargo_id if reverse else suit_id
	var destination_id := suit_id if reverse else cargo_id
	var quantity := 1
	if all:
		quantity = _resource_amount(_inventory(source_id).get("contents", {}), resource)
	if quantity <= 0:
		_set_message("The selected source stack is empty", true)
		return
	_queue_mutation({
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


func _selected_owned_cargo_inventory() -> String:
	_refresh_owned_cargo_selection()
	return selected_cargo_inventory_id


func _refresh_owned_cargo_selection() -> Array[Dictionary]:
	var candidates := _owned_cargo_candidates()
	if candidates.is_empty():
		selected_cargo_inventory_id = ""
		last_targeted_owned_grid_id = _targeted_owned_grid_id()
		return candidates

	var candidate_ids: Array[String] = []
	for candidate in candidates:
		candidate_ids.append(String(candidate.get("inventory_id", "")))
	var targeted_grid_id := _targeted_owned_grid_id()
	if targeted_grid_id != last_targeted_owned_grid_id and not targeted_grid_id.is_empty():
		for candidate in candidates:
			if String(candidate.get("grid_id", "")) == targeted_grid_id:
				selected_cargo_inventory_id = String(candidate.get("inventory_id", ""))
				break
	last_targeted_owned_grid_id = targeted_grid_id
	if not selected_cargo_inventory_id in candidate_ids:
		selected_cargo_inventory_id = String(candidates.front().get("inventory_id", ""))
	return candidates


func _owned_cargo_candidates() -> Array[Dictionary]:
	var candidates: Array[Dictionary] = []
	for inventory_value in actor_private_snapshot.get("inventories", []):
		if not inventory_value is Dictionary:
			continue
		var inventory: Dictionary = inventory_value
		if String(inventory.get("domain", {}).get("kind", "")) != "cargo":
			continue
		var binding := _cargo_inventory_binding(inventory)
		if binding.is_empty():
			continue
		var grid: Dictionary = binding["grid"]
		var block: Dictionary = binding["block"]
		if (
			not _grid_owned_by_local(grid)
			or not bool(block.get("construction_complete", false))
			or int(block.get("health", 0)) <= 0
		):
			continue
		candidates.append({
			"inventory_id": String(inventory.get("inventory_id", "")),
			"grid_id": String(binding.get("grid_id", "")),
			"block_id": String(block.get("block_id", "")),
		})
	candidates.sort_custom(func(first: Dictionary, second: Dictionary) -> bool:
		var first_grid := String(first.get("grid_id", ""))
		var second_grid := String(second.get("grid_id", ""))
		if first_grid != second_grid:
			return first_grid < second_grid
		var first_block := String(first.get("block_id", ""))
		var second_block := String(second.get("block_id", ""))
		if first_block != second_block:
			return first_block < second_block
		return String(first.get("inventory_id", "")) < String(second.get("inventory_id", ""))
	)
	return candidates


func _cargo_inventory_binding(inventory: Dictionary) -> Dictionary:
	var inventory_id := String(inventory.get("inventory_id", ""))
	var domain: Dictionary = inventory.get("domain", {})
	var block_id := String(domain.get("block_id", ""))
	if inventory_id.is_empty() or block_id.is_empty():
		return {}
	var matches: Array[Dictionary] = []
	for grid_id in _all_grid_ids():
		var grid := _grid_record(grid_id)
		for block_value in grid.get("blocks", []):
			if not block_value is Dictionary:
				continue
			var block: Dictionary = block_value
			if (
				String(block.get("block_id", "")) == block_id
				and String(block.get("kind", "")) == "cargo"
			):
				matches.append({"grid_id": grid_id, "grid": grid, "block": block})
	return matches.front() if matches.size() == 1 else {}


func _all_grid_ids() -> Array[String]:
	var ids: Dictionary = {}
	for grid_value in snapshot.get("grids", []):
		if grid_value is Dictionary:
			var grid_id := String(grid_value.get("grid_id", ""))
			if not grid_id.is_empty():
				ids[grid_id] = true
	for grid_id in grid_lookup:
		ids[String(grid_id)] = true
	var ordered: Array[String] = []
	for grid_id in ids:
		ordered.append(String(grid_id))
	ordered.sort()
	return ordered


func _grid_record(grid_id: String) -> Dictionary:
	if grid_lookup.has(grid_id):
		return grid_lookup[grid_id]
	for grid_value in snapshot.get("grids", []):
		if grid_value is Dictionary and String(grid_value.get("grid_id", "")) == grid_id:
			return grid_value
	return {}


func _grid_owned_by_local(grid: Dictionary) -> bool:
	var owner_player_id := String(grid.get("owner_player_id", ""))
	return not owner_player_id.is_empty() and owner_player_id == _controlled_player_id()


func _target_grid_owned_by_local() -> bool:
	return not target_block.is_empty() and _grid_owned_by_local(target_block.get("grid", {}))


func _targeted_owned_grid_id() -> String:
	return String(target_block.get("grid_id", "")) if _target_grid_owned_by_local() else ""


func _owned_grid_ids() -> Array[String]:
	var owned: Array[String] = []
	for grid_id in _all_grid_ids():
		if _grid_owned_by_local(_grid_record(grid_id)):
			owned.append(grid_id)
	return owned


func _owned_grid_for_command(report_error := true) -> String:
	if not target_block.is_empty():
		if _target_grid_owned_by_local():
			return String(target_block.get("grid_id", ""))
		if report_error:
			_report_foreign_grid_access(target_block.get("grid", {}))
		return ""
	var owned := _owned_grid_ids()
	if not owned.is_empty():
		return owned.front()
	if report_error:
		_set_message("No owned grid is available for engineering control", true)
	return ""


func _report_foreign_grid_access(grid: Dictionary) -> void:
	var owner := String(grid.get("owner_player_id", "UNREGISTERED"))
	if owner.is_empty():
		owner = "UNREGISTERED"
	_set_message("ACCESS LOCKED // PROPERTY OF %s" % owner, true)


func _mutation_actor_matches_session() -> bool:
	if mutation_queue_actor_id.is_empty() and in_flight_mutation_actor_id.is_empty():
		return true
	if session_role_kind != "player" or bound_player_id.is_empty():
		return false
	return (
		(mutation_queue_actor_id.is_empty() or mutation_queue_actor_id == bound_player_id)
		and (
			in_flight_mutation_actor_id.is_empty()
			or in_flight_mutation_actor_id == bound_player_id
		)
	)


func _clear_mutation_pipeline() -> void:
	mutation_queue.clear()
	mutation_queue_actor_id = ""
	in_flight_mutation = {}
	in_flight_mutation_text = ""
	in_flight_mutation_actor_id = ""
	mutation_retry_elapsed = 0.0
	mutation_retry_count = 0


func _refresh_mutation_actor_binding() -> void:
	if mutation_queue.is_empty() and in_flight_mutation.is_empty():
		mutation_queue_actor_id = ""
		in_flight_mutation_actor_id = ""


func _queue_mutation(message: Dictionary) -> bool:
	if (
		not connected
		or not authoritative_player_ready
		or not operation_frontier_ready
		or mutation_resync_required
		or session_role_kind != "player"
		or bound_player_id.is_empty()
	):
		_set_message("AUTHORITATIVE COMMAND FRONTIER UNAVAILABLE // RESYNC REQUIRED", true)
		return false
	if message.has("operation_sequence"):
		_set_message("CLIENT COMMAND REJECTED // SEQUENCE IS SERVER-RECONCILED", true)
		return false
	if mutation_queue_actor_id.is_empty():
		mutation_queue_actor_id = bound_player_id
	if mutation_queue_actor_id != bound_player_id:
		_set_message("CLIENT COMMAND REJECTED // SESSION ACTOR CHANGED", true)
		return false

	var queued := message.duplicate(true)
	if String(queued.get("type", "")) == "set_player_control" and _coalesce_queued_control(queued):
		return true
	if mutation_queue.size() >= MUTATION_QUEUE_LIMIT:
		_set_message("COMMAND BUFFER FULL // WAITING FOR AUTHORITY", true)
		return false
	mutation_queue.append(queued)
	_dispatch_next_mutation()
	return true


func _coalesce_queued_control(message: Dictionary) -> bool:
	if mutation_queue.is_empty():
		return false
	var queued: Dictionary = mutation_queue.back()
	if String(queued.get("type", "")) != "set_player_control":
		return false
	var superseded_input_sequence := int(queued.get("input_sequence", 0))
	mutation_queue[mutation_queue.size() - 1] = message.duplicate(true)
	var retained_controls: Array[Dictionary] = []
	for pending in pending_controls:
		if int(pending.get("input_sequence", 0)) != superseded_input_sequence:
			retained_controls.append(pending)
	pending_controls = retained_controls
	prediction_history_invalid = true
	return true


func _dispatch_next_mutation() -> bool:
	if (
		not in_flight_mutation.is_empty()
		or mutation_queue.is_empty()
		or not connected
		or not authoritative_player_ready
		or not operation_frontier_ready
		or mutation_resync_required
		or not _mutation_actor_matches_session()
	):
		return false
	if committed_operation_sequence >= JSON_SAFE_INTEGER_MAX:
		mutation_resync_required = true
		_set_message("COMMAND SEQUENCE EXCEEDS SAFE CLIENT RANGE // AUTHORITY HALTED", true)
		return false

	var message: Dictionary = mutation_queue.pop_front()
	message["operation_sequence"] = committed_operation_sequence + 1
	in_flight_mutation = message.duplicate(true)
	in_flight_mutation_text = JSON.stringify(in_flight_mutation)
	in_flight_mutation_actor_id = bound_player_id
	mutation_retry_elapsed = 0.0
	mutation_retry_count = 0
	if not _send_text_transport(in_flight_mutation_text):
		operation_frontier_ready = false
		return false
	return true


func _advance_mutation_transport(delta: float) -> void:
	if (
		in_flight_mutation.is_empty()
		or not connected
		or not authoritative_player_ready
		or not operation_frontier_ready
		or mutation_resync_required
	):
		return
	mutation_retry_elapsed += delta
	if mutation_retry_elapsed < MUTATION_RETRY_INTERVAL:
		return
	if mutation_retry_count >= MUTATION_RETRY_LIMIT:
		_request_operation_resync("COMMAND RECEIPT TIMEOUT")
		return
	mutation_retry_elapsed = 0.0
	mutation_retry_count += 1
	if not _send_text_transport(in_flight_mutation_text):
		operation_frontier_ready = false


func _request_operation_resync(reason: String) -> void:
	mutation_resync_required = true
	operation_frontier_ready = false
	authoritative_player_ready = false
	_set_message("%s // REQUESTING AUTHORITATIVE FRONTIER" % reason, true)
	if socket.get_ready_state() == WebSocketPeer.STATE_OPEN:
		_send_transport({"type": "request_snapshot"})


func _reconcile_operation_frontier(frontier: int) -> bool:
	if frontier < 0 or not _mutation_actor_matches_session():
		mutation_resync_required = true
		return false
	var observed_floor := committed_operation_sequence
	if operation_frontier_observed:
		observed_floor = maxi(observed_floor, observed_operation_frontier)
	if (
		operation_frontier_observed
		and committed_operation_actor_id == bound_player_id
		and frontier < observed_floor
	):
		mutation_resync_required = true
		return false
	if (
		operation_frontier_observed
		and not committed_operation_actor_id.is_empty()
		and committed_operation_actor_id != bound_player_id
	):
		mutation_resync_required = true
		return false
	var resuming_after_resync := mutation_resync_required or not operation_frontier_ready
	committed_operation_actor_id = bound_player_id
	if in_flight_mutation.is_empty():
		committed_operation_sequence = frontier
		operation_frontier_observed = true
		observed_operation_frontier = frontier
		operation_frontier_ready = true
		mutation_resync_required = false
		_refresh_mutation_actor_binding()
		return true

	var pending_sequence := int(in_flight_mutation.get("operation_sequence", 0))
	if pending_sequence <= 0 or frontier < pending_sequence - 1:
		mutation_resync_required = true
		return false
	if frontier == pending_sequence - 1:
		committed_operation_sequence = frontier
		operation_frontier_observed = true
		observed_operation_frontier = frontier
		operation_frontier_ready = true
		mutation_resync_required = false
		mutation_retry_elapsed = MUTATION_RETRY_INTERVAL
		if resuming_after_resync:
			mutation_retry_count = 0
		return true
	if frontier == pending_sequence:
		# A frontier alone cannot prove that this exact payload won a race with
		# another session using the same actor. Retain and retry the byte-identical
		# command so authority returns either its original receipt or a conflict.
		operation_frontier_observed = true
		observed_operation_frontier = frontier
		operation_frontier_ready = true
		mutation_resync_required = false
		mutation_retry_elapsed = MUTATION_RETRY_INTERVAL
		if resuming_after_resync:
			mutation_retry_count = 0
		return true

	# Another writer advanced this actor while the local command outcome was
	# unknown. The pending command is committed, but later queued commands were
	# authored against an obsolete frontier and must never be guessed forward.
	committed_operation_sequence = frontier
	operation_frontier_observed = true
	observed_operation_frontier = frontier
	_clear_mutation_pipeline()
	operation_frontier_ready = true
	mutation_resync_required = false
	_set_message("COMMAND QUEUE DISCARDED // ACTOR FRONTIER ADVANCED ELSEWHERE", true)
	return true


func _handle_intent_accepted(receipt: Dictionary) -> bool:
	var sequence := _protocol_nonnegative_integer(
		receipt.get("operation_sequence", null)
	)
	if sequence < 0:
		_request_operation_resync("MALFORMED COMMAND RECEIPT")
		return false
	if sequence <= committed_operation_sequence:
		return false
	if in_flight_mutation.is_empty():
		_request_operation_resync("UNEXPECTED COMMAND RECEIPT")
		return false
	if (
		sequence != int(in_flight_mutation.get("operation_sequence", 0))
		or String(receipt.get("operation_id", ""))
		!= String(in_flight_mutation.get("operation_id", ""))
		or sequence != committed_operation_sequence + 1
	):
		_request_operation_resync("COMMAND RECEIPT CONFLICT")
		return false
	committed_operation_sequence = sequence
	committed_operation_actor_id = bound_player_id
	operation_frontier_observed = true
	observed_operation_frontier = maxi(observed_operation_frontier, sequence)
	in_flight_mutation = {}
	in_flight_mutation_text = ""
	in_flight_mutation_actor_id = ""
	mutation_retry_elapsed = 0.0
	mutation_retry_count = 0
	operation_frontier_ready = true
	mutation_resync_required = false
	_refresh_mutation_actor_binding()
	_dispatch_next_mutation()
	return true


func _handle_intent_rejected(message: Dictionary) -> bool:
	var sequence := _protocol_nonnegative_integer(
		message.get("operation_sequence", null)
	)
	if sequence < 0:
		_request_operation_resync("UNBOUND COMMAND REJECTION")
		return false
	if sequence <= committed_operation_sequence:
		return false
	if in_flight_mutation.is_empty():
		_request_operation_resync("UNEXPECTED COMMAND REJECTION")
		return false
	var response_operation_id := String(message.get("operation_id", ""))
	if (
		sequence != int(in_flight_mutation.get("operation_sequence", 0))
		or response_operation_id.is_empty()
		or response_operation_id != String(in_flight_mutation.get("operation_id", ""))
	):
		_request_operation_resync("COMMAND REJECTION CONFLICT")
		return false
	var code := String(message.get("code", ""))
	if code in [
		"operation_conflict",
		"operation_sequence_gap",
		"operation_already_committed",
		"operation_history_invalid",
		"operation_sequence_invalid",
		"operation_sequence_exhausted",
	]:
		_request_operation_resync("AUTHORITATIVE %s" % code.to_upper())
		return false

	# Gameplay validation did not consume the frontier. Drop only the rejected
	# command so the next queued command reuses this exact sequence.
	var rejected_player_control := (
		String(in_flight_mutation.get("type", "")) == "set_player_control"
	)
	in_flight_mutation = {}
	in_flight_mutation_text = ""
	in_flight_mutation_actor_id = ""
	mutation_retry_elapsed = 0.0
	mutation_retry_count = 0
	_refresh_mutation_actor_binding()
	if rejected_player_control:
		# Do not send anything authored from the stale prediction baseline. The
		# complete snapshot will reconcile the reusable operation frontier first.
		_reset_control_prediction_after_rejection()
		_request_operation_resync("PLAYER CONTROL REJECTED")
		return true
	_dispatch_next_mutation()
	return true


func _send_transport(message: Dictionary) -> bool:
	return _send_text_transport(JSON.stringify(message))


func _send_text_transport(encoded_message: String) -> bool:
	if socket.get_ready_state() != WebSocketPeer.STATE_OPEN:
		_set_message("No authoritative server connection — press F5", true)
		return false
	var error := socket.send_text(encoded_message)
	if error != OK:
		_set_message("Network send failed: %s" % error_string(error), true)
		return false
	return true


func _operation_id(prefix: String) -> String:
	operation_counter += 1
	return "%s-%d-%d" % [prefix, Time.get_ticks_usec(), operation_counter]


func _update_life_support_interface(player: Dictionary) -> void:
	var display_state := _life_support_display_state(player)
	critical_oxygen_panel.visible = display_state == "critical"
	incapacitated_overlay.visible = display_state == "incapacitated"
	if display_state == "critical":
		var oxygen_percent := int(player.get("suit_oxygen_milli", 0)) / 10
		var environment := _local_environment()
		var helmet_closed := bool(player.get("helmet_closed", true))
		var breathable := bool(environment.get("breathable", false))
		var remedy := "SEEK BREATHABLE ATMOSPHERE"
		if not helmet_closed and not breathable:
			remedy = "SEAL HELMET [H]"
		elif not helmet_closed and breathable:
			remedy = "SUIT RESERVE RECHARGING"
		critical_oxygen_label.text = "⚠ O₂ CRITICAL // %d%% // %s" % [oxygen_percent, remedy]
	if display_state == "incapacitated":
		var life_state: Dictionary = player.get("life_state", {})
		var cause: Dictionary = life_state.get("cause", {})
		var cause_text := (
			"OXYGEN RESERVE DEPLETED"
			if cause.get("kind", "") == "oxygen_depleted"
			else "SUIT FAILURE RECORDED"
		)
		var death_id := String(life_state.get("death_id", "unavailable"))
		var drop_text := (
			"CARRIED INVENTORY DROP RECORDED AT FAILURE SITE"
			if _owned_death_drop_recorded(player)
			else "NO CARRIED INVENTORY DROP RECORDED"
		)
		incapacitated_detail_label.text = "%s\n%s\nDEATH RECORD // %s" % [
			cause_text, drop_text, death_id
		]
		recovery_button.disabled = not connected or not recovery_operation.is_empty()
		recovery_button.text = (
			"RECOVERY REQUESTED…"
			if not recovery_operation.is_empty()
			else "[ENTER]  REQUEST RECOVERY"
		)


func _owned_death_drop_recorded(player: Dictionary) -> bool:
	var player_id := String(player.get("player_id", ""))
	var life_state: Dictionary = player.get("life_state", {})
	var death_id := String(life_state.get("death_id", ""))
	for drop in actor_private_snapshot.get("death_drops", []):
		if (
			drop is Dictionary
			and String(drop.get("owner_player_id", "")) == player_id
			and String(drop.get("death_id", "")) == death_id
		):
			return true
	return false


func _update_interface() -> void:
	var link_text := "○ RELAY OFFLINE // F5 TO RETRY"
	if connected:
		match replication_state:
			"ready":
				link_text = "● LINKED // INTEREST VIEW %d" % interest_delta_sequence
			"stale":
				link_text = "◐ LINK STALE // SAFE RESYNC IN PROGRESS"
			"fatal":
				link_text = "× PROTOCOL HALTED // %s" % replication_detail
			_:
				link_text = "◌ LINK LOADING // %s" % replication_detail
	connection_label.text = link_text
	connection_label.add_theme_color_override(
		"font_color",
		Color(0.35, 0.95, 0.62)
		if connected and replication_state == "ready"
		else Color(1.0, 0.72, 0.24)
		if connected and replication_state in ["loading", "stale"]
		else Color(1.0, 0.38, 0.25)
	)
	var player := _local_player()
	_update_life_support_interface(player)
	var level := int(player.get("level", 1))
	var experience := int(player.get("experience", 0))
	var next_level := int(player.get("next_level_experience", 100))
	level_label.text = "SALVAGER // LEVEL %d     REP %d / %d" % [level, experience, next_level]
	var suit_power := clampi(100 - roundi(predicted_linear_velocity.length() * 1.4), 72, 100)
	var oxygen_percent := int(player.get("suit_oxygen_milli", 1000)) / 10
	var helmet_state := "SEALED" if player.get("helmet_closed", true) else "OPEN"
	var locomotion: Dictionary = player.get("locomotion", {})
	var locomotion_kind := String(locomotion.get("kind", "eva"))
	var jetpack_state := (
		"JET"
		if player.get("jetpack_enabled", true)
		else "MAG-LOCK"
		if locomotion_kind == "magnetic"
		else "GROUND"
		if locomotion_kind == "grounded"
		else "FREEFALL"
	)
	var boots_state := "BOOTS ARMED" if desired_magnetic_boots else "BOOTS OFF"
	var dampener_state := "DAMP" if desired_dampeners else "DRIFT"
	var contact_state := "CONTACT" if predicted_surface_contact else "FREE"
	telemetry_label.text = "O₂ %03d%%   PWR %03d%%   %s   %s   %s   %s   %s" % [
		oxygen_percent, suit_power, helmet_state, jetpack_state, boots_state,
		dampener_state, contact_state
	]
	var life_support_state := _life_support_display_state(player)
	telemetry_label.add_theme_color_override(
		"font_color",
		Color(1.0, 0.32, 0.18)
		if life_support_state != "normal"
		else Color(0.64, 0.90, 0.94)
	)
	var environment := _local_environment()
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
			"CONSERVED" if snapshot.get("conservation_valid", false) else "FAULT",
			String(snapshot.get("world_hash", "—")).left(8),
		]
	)
	var player_inventory := _inventory(_local_inventory_id())
	var contents: Dictionary = player_inventory.get("contents", {})
	var conservation_valid := bool(snapshot.get("conservation_valid", false))
	if _private_inventory_ready():
		inventory_label.text = (
			"CARGO HARNESS  //  %d / %d L\nORE  %03d     ALLOY  %03d     PARTS  %03d\n[I] OPEN LOGISTICS TERMINAL"
		) % [
			int(player_inventory.get("used_liters", 0)),
			int(player_inventory.get("capacity_liters", 0)),
			int(contents.get("ore", 0)),
			int(contents.get("refined_material", 0)),
			int(contents.get("components", 0)),
		]
	else:
		inventory_label.text = (
			"PRIVATE INVENTORY LINK UNAVAILABLE\n"
			+ "REFINE / CRAFT / TRANSFER DISABLED\n[F5] REQUEST AUTHORITATIVE RESYNC"
		)
	inventory_label.add_theme_color_override(
		"font_color",
		Color(0.95, 0.71, 0.27)
		if conservation_valid
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
			["6", "CONVEYOR", "conveyor"],
			["7", "REFINERY", "refinery"],
			["8", "ASSEMBLER", "assembler"],
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
		hotbar_label.text = "HAND DRILL     [J] JETPACK     [K] MAG BOOTS     [B] CONSTRUCTION     [R] REFINE     [T] FABRICATE     [V] CARGO"
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
		if not _target_grid_owned_by_local():
			var owner := String(target_block.get("grid", {}).get(
				"owner_player_id", "UNREGISTERED"
			))
			if owner.is_empty():
				owner = "UNREGISTERED"
			target_label.text = (
				"%s // INTEGRITY %d%%\nACCESS LOCKED // PROPERTY OF %s\nHOLD RMB  //  CUT AND SALVAGE"
				% [String(block.get("kind", "block")).to_upper(), integrity, owner]
			)
		elif build_mode:
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
	if not active_grid_control_id.is_empty():
		mode_label.text = "GRID CONTROL ACTIVE // %s // RELEASE M OR PRESS X TO DAMPEN" % active_grid_control_id
	elif build_mode and not target_block.is_empty() and not _target_grid_owned_by_local():
		mode_label.text = "CONSTRUCTION ACCESS LOCKED // TARGET AN OWNED GRID"
	elif action_charge <= 0.0:
		mode_label.text = (
			"CONSTRUCTION HOLOGRAM // %s // ROT %03d°" % [
				selected_block_kind.to_upper(), build_rotation_quarters * 90
			]
			if build_mode
			else "INDUSTRIAL HAND DRILL // READY"
			)
	if _player_is_incapacitated(player):
		hotbar_label.text = "EVA CONTROL OFFLINE     [ENTER] REQUEST RECOVERY"
		target_label.text = "CANONICAL PLAYER STATE // INCAPACITATED"
		mode_label.text = "ALL MOVEMENT AND WORK CONTROLS LOCKED"
	action_progress.visible = action_charge > 0.0
	message_label.text = recent_message
	message_label.add_theme_color_override("font_color", recent_message_color)


func _update_inventory_terminal() -> void:
	var cargo_candidates := _refresh_owned_cargo_selection()
	_update_cargo_inventory_selector(cargo_candidates)
	_update_production_terminal()
	var suit_inventory := _inventory(_local_inventory_id())
	var cargo_inventory := _inventory(selected_cargo_inventory_id)
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
		var reported_capacity := int(inventory.get("capacity_liters", 0))
		var capacity := maxi(reported_capacity, 1)
		var used := int(inventory.get("used_liters", 0))
		var mass_kg := float(inventory.get("mass_grams", 0)) / 1000.0
		var capacity_label: Label = inventory_capacity_labels.get(side, null)
		if capacity_label != null:
			capacity_label.text = "%d / %d L  //  %.1f kg" % [
				used, reported_capacity, mass_kg
			]
		var capacity_bar: ProgressBar = inventory_capacity_bars.get(side, null)
		if capacity_bar != null:
			capacity_bar.value = clampf(float(used) / float(capacity), 0.0, 1.0)


func _update_production_terminal() -> void:
	var machine_lines: Array[String] = ["AUTHORIZED MACHINES"]
	for kind in ["refinery", "assembler"]:
		var machines := _owned_machine_candidates(kind)
		if machines.is_empty():
			machine_lines.append("%s  //  UNAVAILABLE" % kind.to_upper())
		for machine in machines:
			machine_lines.append(
				"%s  //  %s  //  GRID %s  //  POWER %s" % [
					kind.to_upper(),
					machine.get("block_id", ""),
					machine.get("grid_id", ""),
					"ONLINE" if machine.get("powered", false) else "OFFLINE",
				]
			)
	if is_instance_valid(production_machine_label):
		production_machine_label.text = "\n".join(machine_lines)

	var queue_lines: Array[String] = ["CANONICAL PRODUCTION QUEUES"]
	var valid_jobs := 0
	for queue_value in _production_queues():
		if not queue_value is Dictionary:
			continue
		var machine_id := String(queue_value.get("machine_block_id", ""))
		var jobs: Variant = queue_value.get("jobs", [])
		if machine_id.is_empty() or not jobs is Array:
			continue
		for job_value in jobs:
			if not job_value is Dictionary:
				continue
			var job: Dictionary = job_value
			if (
				String(job.get("owner_player_id", "")) != bound_player_id
				or String(job.get("machine_block_id", "")) != machine_id
			):
				continue
			var duration := maxi(int(job.get("duration_ticks", 0)), 0)
			var progress := clampi(int(job.get("progress_ticks", 0)), 0, duration)
			var percent := 0 if duration <= 0 else int(round(100.0 * float(progress) / float(duration)))
			var status := String(job.get("status", "queued")).to_upper().replace("_", " ")
			queue_lines.append(
				"%s  //  %s ×%d  //  %d%%  //  %s\nROUTE %s → %s  //  RESERVED %s  //  PENDING %s" % [
					machine_id,
					String(job.get("recipe", "unknown")).to_upper(),
					maxi(int(job.get("batches", 0)), 0),
					percent,
					status,
					String(job.get("source_inventory_id", "")),
					String(job.get("destination_inventory_id", "")),
					_production_contents_text(job.get("reserved_inputs", {})),
					_production_contents_text(job.get("pending_outputs", {})),
				]
			)
			valid_jobs += 1
	if valid_jobs == 0:
		queue_lines.append("NO CANONICAL JOBS")
	if is_instance_valid(production_queue_label):
		production_queue_label.text = "\n\n".join(queue_lines)

	var refining_route := _production_route("refining")
	var component_route := _production_route("component")
	if is_instance_valid(production_route_label):
		production_route_label.text = "REFINERY %s  //  ASSEMBLER %s" % [
			_production_route_text(refining_route), _production_route_text(component_route)
		]
	for recipe in production_buttons:
		var route := refining_route if recipe == "refining" else component_route
		var button: Button = production_buttons[recipe]
		button.disabled = route.is_empty() or not _private_inventory_ready()


func _production_contents_text(value: Variant) -> String:
	if not value is Dictionary:
		return "—"
	return "O%d A%d C%d" % [
		maxi(int(value.get("ore", 0)), 0),
		maxi(int(value.get("refined_material", 0)), 0),
		maxi(int(value.get("components", 0)), 0),
	]


func _production_route_text(route: Dictionary) -> String:
	if route.is_empty():
		return "NO AUTHORIZED ROUTE"
	return "%s ↔ %s" % [route.get("machine_block_id", ""), route.get("inventory_id", "")]


func _update_cargo_inventory_selector(candidates: Array[Dictionary]) -> void:
	var selector: OptionButton = inventory_selectors.get("cargo", null)
	var title_label: Label = inventory_title_labels.get("cargo", null)
	var subtitle_label: Label = inventory_subtitle_labels.get("cargo", null)
	var available := not selected_cargo_inventory_id.is_empty()
	for button in inventory_transfer_buttons:
		button.disabled = not available
	if selector == null:
		return
	selector.clear()
	if not available:
		selector.add_item("NO AUTHORIZED CARGO LINK")
		selector.set_item_metadata(0, "")
		selector.disabled = true
		if title_label != null:
			title_label.text = "NO AUTHORIZED CARGO LINK"
		if subtitle_label != null:
			subtitle_label.text = "OWNED COMPLETED CARGO REQUIRED"
		return

	selector.disabled = false
	var selected_index := 0
	var selected_candidate: Dictionary = {}
	for index in candidates.size():
		var candidate: Dictionary = candidates[index]
		var grid_id := String(candidate.get("grid_id", ""))
		var block_id := String(candidate.get("block_id", ""))
		var inventory_id := String(candidate.get("inventory_id", ""))
		selector.add_item("%s  /  %s" % [grid_id, block_id])
		selector.set_item_metadata(index, inventory_id)
		if inventory_id == selected_cargo_inventory_id:
			selected_index = index
			selected_candidate = candidate
	selector.select(selected_index)
	if selected_candidate.is_empty() and not candidates.is_empty():
		selected_candidate = candidates[selected_index]
	if title_label != null:
		title_label.text = String(selected_candidate.get("grid_id", "OWNED GRID")).to_upper()
	if subtitle_label != null:
		subtitle_label.text = "AUTHORIZED CARGO // %s" % String(
			selected_candidate.get("block_id", "")
		)


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
	for inventory in actor_private_snapshot.get("inventories", []):
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


func _valid_hash(value: String) -> bool:
	if value.length() != 64:
		return false
	for index in value.length():
		var code := value.unicode_at(index)
		if not (code >= 48 and code <= 57) and not (code >= 97 and code <= 102):
			return false
	return true


func _universe_address_valid(value: Variant, manifest: Dictionary) -> bool:
	if not value is Dictionary:
		return false
	var address: Dictionary = value
	var sector: Variant = address.get("sector", {})
	var cell: Variant = address.get("cell", {})
	var local: Variant = address.get("local_um", {})
	if not sector is Dictionary or not cell is Dictionary or not local is Dictionary:
		return false
	if String(address.get("universe_id", "")) != String(manifest.get("universe_id", "")):
		return false
	var cells_per_axis := int(manifest.get("cells_per_sector_axis", 0))
	var cell_edge := int(manifest.get("cell_edge_um", 0))
	var sector_edge := int(manifest.get("sector_edge_um", 0))
	if (
		cells_per_axis <= 0
		or cell_edge <= 0
		or sector_edge != cell_edge * cells_per_axis
		or cell_edge % 2 != 0
	):
		return false
	var half := cell_edge / 2
	for axis in ["x", "y", "z"]:
		if not _canonical_i128_decimal(String(sector.get(axis, ""))):
			return false
		var cell_component := _protocol_nonnegative_integer(cell.get(axis, null))
		if cell_component < 0 or cell_component >= cells_per_axis:
			return false
		var local_value: Variant = _protocol_signed_integer(local.get(axis, null))
		if local_value == null:
			return false
		var local_component := int(local_value)
		if local_component < -half or local_component >= half:
			return false
	return true


func _canonical_i128_decimal(value: String) -> bool:
	if value.is_empty() or value == "-0" or value.begins_with("+"):
		return false
	var negative := value.begins_with("-")
	var magnitude := value.substr(1) if negative else value
	if magnitude.is_empty() or (magnitude.length() > 1 and magnitude.begins_with("0")):
		return false
	for index in magnitude.length():
		var code := magnitude.unicode_at(index)
		if code < 48 or code > 57:
			return false
	var maximum := (
		"170141183460469231731687303715884105728"
		if negative
		else "170141183460469231731687303715884105727"
	)
	return _decimal_compare(magnitude, maximum) <= 0


func _address_relative_m(address_value: Variant, origin_value: Variant) -> Variant:
	if (
		not _universe_address_valid(address_value, universe_manifest)
		or not _universe_address_valid(origin_value, universe_manifest)
	):
		return null
	var address: Dictionary = address_value
	var origin: Dictionary = origin_value
	var sector_edge := int(universe_manifest.get("sector_edge_um", 0))
	var cell_edge := int(universe_manifest.get("cell_edge_um", 0))
	var components: Array[float] = []
	for axis in ["x", "y", "z"]:
		var sector_delta: Variant = _signed_decimal_difference_limited(
			String(address["sector"].get(axis, "0")),
			String(origin["sector"].get(axis, "0")),
			4
		)
		if sector_delta == null:
			return null
		var offset_um := (
			int(sector_delta) * sector_edge
			+ (int(address["cell"].get(axis, 0)) - int(origin["cell"].get(axis, 0))) * cell_edge
			+ int(address["local_um"].get(axis, 0))
			- int(origin["local_um"].get(axis, 0))
		)
		if absi(offset_um) > int(RENDER_DISTANCE_LIMIT_M * 2.0 * 1_000_000.0):
			return null
		components.append(float(offset_um) / 1_000_000.0)
	return Vector3(components[0], components[1], components[2])


func _signed_decimal_difference_limited(left: String, right: String, limit: int) -> Variant:
	var left_negative := left.begins_with("-")
	var right_negative := right.begins_with("-")
	var left_magnitude := left.substr(1) if left_negative else left
	var right_magnitude := right.substr(1) if right_negative else right
	var magnitude := "0"
	var negative := false
	if left_negative == right_negative:
		var comparison := _decimal_compare(left_magnitude, right_magnitude)
		if comparison >= 0:
			magnitude = _decimal_subtract(left_magnitude, right_magnitude)
			negative = left_negative and magnitude != "0"
		else:
			magnitude = _decimal_subtract(right_magnitude, left_magnitude)
			negative = not left_negative and magnitude != "0"
	else:
		magnitude = _decimal_add(left_magnitude, right_magnitude)
		negative = left_negative
	if _decimal_compare(magnitude, str(limit)) > 0:
		return null
	var value := int(magnitude)
	return -value if negative else value


func _decimal_compare(left: String, right: String) -> int:
	var normalized_left := left.lstrip("0")
	var normalized_right := right.lstrip("0")
	if normalized_left.is_empty():
		normalized_left = "0"
	if normalized_right.is_empty():
		normalized_right = "0"
	if normalized_left.length() != normalized_right.length():
		return -1 if normalized_left.length() < normalized_right.length() else 1
	if normalized_left == normalized_right:
		return 0
	return -1 if normalized_left < normalized_right else 1


func _decimal_add(left: String, right: String) -> String:
	var left_index := left.length() - 1
	var right_index := right.length() - 1
	var carry := 0
	var result := ""
	while left_index >= 0 or right_index >= 0 or carry > 0:
		var digit := carry
		if left_index >= 0:
			digit += int(left.substr(left_index, 1))
			left_index -= 1
		if right_index >= 0:
			digit += int(right.substr(right_index, 1))
			right_index -= 1
		result = str(digit % 10) + result
		carry = floori(float(digit) / 10.0)
	return result


func _decimal_subtract(larger: String, smaller: String) -> String:
	var larger_index := larger.length() - 1
	var smaller_index := smaller.length() - 1
	var borrow := 0
	var result := ""
	while larger_index >= 0:
		var digit := int(larger.substr(larger_index, 1)) - borrow
		if smaller_index >= 0:
			digit -= int(smaller.substr(smaller_index, 1))
		if digit < 0:
			digit += 10
			borrow = 1
		else:
			borrow = 0
		result = str(digit) + result
		larger_index -= 1
		smaller_index -= 1
	var normalized := result.lstrip("0")
	return "0" if normalized.is_empty() else normalized


func _vec3(value: Dictionary) -> Vector3:
	return Vector3(
		float(value.get("x", 0.0)),
		float(value.get("y", 0.0)),
		float(value.get("z", 0.0))
	)


func _quat(value: Dictionary) -> Quaternion:
	var rotation := Quaternion(
		float(value.get("x", 0.0)),
		float(value.get("y", 0.0)),
		float(value.get("z", 0.0)),
		float(value.get("w", 1.0))
	)
	return rotation.normalized() if rotation.length_squared() > 0.000001 else Quaternion.IDENTITY


func _grid_quaternion(grid: Dictionary) -> Quaternion:
	return _quat(grid.get("orientation", {}))


func _grid_basis(grid: Dictionary) -> Basis:
	return Basis(_grid_quaternion(grid))


func _protocol_vec3(value: Vector3) -> Dictionary:
	return {"x": value.x, "y": value.y, "z": value.z}


func _protocol_ivec3(value: Vector3i) -> Dictionary:
	return {"x": value.x, "y": value.y, "z": value.z}
