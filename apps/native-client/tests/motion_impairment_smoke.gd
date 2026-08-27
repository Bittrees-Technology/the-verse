# SPDX-License-Identifier: AGPL-3.0-or-later
extends SceneTree

const CLIENT_SCRIPT: Script = preload("res://src/main.gd")
const FIXED_DELTA := 1.0 / 60.0
const HISTORY_LIMIT := 180

var failures: Array[String] = []


func _initialize() -> void:
	call_deferred("_run")


func _run() -> void:
	_test_received_vs_processed_reconciliation()
	_test_ordering_corrections_and_motion_only_updates()
	_test_menu_dead_disconnect_and_bounds()
	_test_life_state_reset()
	_test_bound_player_roster_selection()
	_test_short_roll_taps_and_idle_silence()
	_test_exact_tool_targeting()
	_test_private_projection_lifecycle()
	_test_actor_owned_industry_selection()
	if not failures.is_empty():
		for failure in failures:
			printerr("VERSE_NATIVE_IMPAIRMENT_FAILED %s" % failure)
		quit(1)
		return
	print(
		"VERSE_NATIVE_IMPAIRMENT_OK queued_ack=ordered motion=monotonic corrections=bounded menu=neutral_prediction lifecycle=reset buffers=bounded roll_tap=durable idle=silent rebuild=none targeting=closest_hit ownership=filtered privacy=projected"
	)
	quit(0)


func _check(condition: bool, label: String) -> void:
	if not condition:
		failures.append(label)


func _new_client(add_to_tree := false) -> Node3D:
	var client := Node3D.new()
	if add_to_tree:
		root.add_child(client)
	client.set_script(CLIENT_SCRIPT)
	var camera := Camera3D.new()
	client.add_child(camera)
	client.set("camera", camera)
	var player := _base_player()
	client.set("snapshot", {
		"projection_schema_version": 1,
		"event_sequence": 0,
		"simulation_tick": 0,
		"world_hash": "impairment-0",
		"players": [_public_player(player)],
		"environment": {
			"planet_center": _protocol_vec3(Vector3.ZERO),
			"surface_radius_m": 1200.0,
			"gravity": _protocol_vec3(Vector3.ZERO),
		},
		"voxels": [],
		"grids": [],
		"conservation_valid": true,
	})
	client.set("requested_player_id", "impairment-player")
	client.set("bound_player_id", "impairment-player")
	client.set("session_role_kind", "player")
	client.set("actor_private_snapshot", _private_snapshot(player))
	client.set("authoritative_player_ready", true)
	client.set("awaiting_reconnect_baseline", false)
	client.set("last_player_id", "impairment-player")
	client.set("last_player_life_state", "alive")
	client.set("movement_epoch", 1)
	client.set("last_authoritative_event_sequence", 0)
	client.set("last_authoritative_simulation_tick", 0)
	client.set("predicted_simulation_tick", 0)
	client.set("predicted_position", Vector3.ZERO)
	client.set("predicted_orientation", Quaternion.IDENTITY)
	client.set("predicted_linear_velocity", Vector3.ZERO)
	client.set("predicted_angular_velocity", Vector3.ZERO)
	client.set("predicted_surface_contact", false)
	camera.position = Vector3.ZERO
	camera.quaternion = Quaternion.IDENTITY
	return client


func _base_player() -> Dictionary:
	return {
		"player_id": "impairment-player",
		"inventory_id": "inventory-impairment-player",
		"position": _protocol_vec3(Vector3.ZERO),
		"orientation": _protocol_quat(Quaternion.IDENTITY),
		"linear_velocity": _protocol_vec3(Vector3.ZERO),
		"angular_velocity": _protocol_vec3(Vector3.ZERO),
		"surface_contact": false,
		"movement_epoch": 1,
		"last_received_input_sequence": 0,
		"last_processed_input_sequence": 0,
		"control_linear_input": _protocol_vec3(Vector3.ZERO),
		"control_angular_input": _protocol_vec3(Vector3.ZERO),
		"boost": false,
		"dampeners": true,
		"control_expires_at_simulation_tick": 0,
		"jetpack_enabled": true,
		"life_state": {"kind": "alive"},
	}


func _public_player(player: Dictionary) -> Dictionary:
	return {
		"player_id": player.get("player_id", ""),
		"position": player.get("position", _protocol_vec3(Vector3.ZERO)),
		"orientation": player.get("orientation", _protocol_quat(Quaternion.IDENTITY)),
		"linear_velocity": player.get("linear_velocity", _protocol_vec3(Vector3.ZERO)),
		"angular_velocity": player.get("angular_velocity", _protocol_vec3(Vector3.ZERO)),
		"surface_contact": player.get("surface_contact", false),
		"locomotion_kind": player.get("locomotion", {}).get("kind", "eva"),
		"life_state": String(player.get("life_state", {}).get("kind", "alive")),
	}


func _private_snapshot(
	player: Dictionary, inventories: Array = [], death_drops: Array = []
) -> Dictionary:
	var private_inventories := inventories
	if private_inventories.is_empty():
		private_inventories = [
			_inventory_snapshot(
				String(player.get("inventory_id", "")),
				"player",
				String(player.get("player_id", ""))
			),
		]
	return {
		"player": player,
		"inventories": private_inventories,
		"death_drops": death_drops,
		"owned_grid_masses": [],
	}


func _control(angular_input: Vector3, dampeners := true) -> Dictionary:
	return {
		"linear_input": Vector3.ZERO,
		"angular_input": angular_input,
		"boost": false,
		"dampeners": dampeners,
	}


func _player_from_motion(
	motion: Dictionary,
	received: int,
	processed: int,
	epoch := 1,
	life_kind := "alive"
) -> Dictionary:
	var player := _base_player()
	player["position"] = _protocol_vec3(motion.get("position", Vector3.ZERO))
	player["orientation"] = _protocol_quat(
		motion.get("orientation", Quaternion.IDENTITY)
	)
	player["linear_velocity"] = _protocol_vec3(
		motion.get("linear_velocity", Vector3.ZERO)
	)
	player["angular_velocity"] = _protocol_vec3(
		motion.get("angular_velocity", Vector3.ZERO)
	)
	player["movement_epoch"] = epoch
	player["last_received_input_sequence"] = received
	player["last_processed_input_sequence"] = processed
	player["life_state"] = {"kind": life_kind}
	return player


func _motion_message(
	event_sequence: int,
	simulation_tick: int,
	player: Dictionary,
	grids: Array = []
) -> Dictionary:
	return {
		"projection_schema_version": 1,
		"event_sequence": event_sequence,
		"simulation_tick": simulation_tick,
		"world_hash": "impairment-%d" % event_sequence,
		"players": [_public_player(player)],
		"grids": grids,
		"actor_private": player,
	}


func _test_exact_tool_targeting() -> void:
	var client := _new_client()
	var face_normals: Array[Vector3] = [
		Vector3.RIGHT,
		Vector3.LEFT,
		Vector3.UP,
		Vector3.DOWN,
		Vector3.BACK,
		Vector3.FORWARD,
	]
	client.set("grid_lookup", {
		"grid-faces": _target_grid(
			"grid-faces", Vector3.ZERO, Quaternion.IDENTITY,
			[_target_block("block-faces", Vector3i.ZERO)]
		),
	})
	for expected_normal in face_normals:
		var origin := expected_normal * 2.0
		var hit: Dictionary = client.call(
			"_closest_tool_hit", origin, -expected_normal, 9.0
		)
		_check(String(hit.get("kind", "")) == "block", "six-face block selected")
		_check(
			is_equal_approx(float(hit.get("distance", -1.0)), 1.5),
			"six-face exact surface distance"
		)
		_check(
			(hit.get("local_normal", Vector3.ZERO) as Vector3).is_equal_approx(
				expected_normal
			),
			"six-face local entry normal"
		)
		_check(
			(hit.get("hit_position", Vector3.ZERO) as Vector3).is_equal_approx(
				expected_normal * 0.5
			),
			"six-face surface point"
		)
	client.set("grid_lookup", {
		"grid-corner": _target_grid(
			"grid-corner", Vector3.ZERO, Quaternion.IDENTITY,
			[_target_block("block-corner", Vector3i(2, 2, 0))]
		),
	})
	var corner_hit: Dictionary = client.call(
		"_closest_tool_hit", Vector3.ZERO, Vector3(1.0, 1.0, 0.0), 9.0
	)
	_check(
		(corner_hit.get("local_normal", Vector3.ZERO) as Vector3).is_equal_approx(
			Vector3.LEFT
		),
		"equal entry-axis tie resolves to X face"
	)

	var rotation := Quaternion(Vector3.UP, PI * 0.5)
	var rotation_basis := Basis(rotation)
	var rotated_center := rotation_basis * Vector3(2.0, 0.0, 0.0)
	var rotated_world_normal := (rotation_basis * Vector3.RIGHT).normalized()
	client.set("grid_lookup", {
		"grid-rotated": _target_grid(
			"grid-rotated", Vector3.ZERO, rotation,
			[_target_block("block-rotated", Vector3i(2, 0, 0))]
		),
	})
	var rotated_hit: Dictionary = client.call(
		"_closest_tool_hit",
		rotated_center + rotated_world_normal * 2.0,
		-rotated_world_normal,
		9.0
	)
	_check(
		(rotated_hit.get("local_normal", Vector3.ZERO) as Vector3).is_equal_approx(
			Vector3.RIGHT
		),
		"rotated grid keeps local face normal"
	)
	_check(
		(rotated_hit.get("world_normal", Vector3.ZERO) as Vector3).is_equal_approx(
			rotated_world_normal
		),
		"rotated grid emits world face normal"
	)
	client.set("target_block", rotated_hit)
	_check(
		(client.call("_build_coordinate") as Vector3i) == Vector3i(3, 0, 0),
		"rotated adjacency uses exact local hit face"
	)

	client.set("voxel_lookup", {"0,0,2": {"material": "rock"}})
	client.set("grid_lookup", {
		"grid-behind": _target_grid(
			"grid-behind", Vector3.ZERO, Quaternion.IDENTITY,
			[_target_block("block-behind", Vector3i(0, 0, 4))]
		),
	})
	var voxel_first: Dictionary = client.call(
		"_closest_tool_hit", Vector3.ZERO, Vector3.BACK, 9.0
	)
	_check(String(voxel_first.get("kind", "")) == "voxel", "voxel occludes farther block")

	client.set("voxel_lookup", {"0,0,4": {"material": "rock"}})
	client.set("grid_lookup", {
		"grid-front": _target_grid(
			"grid-front", Vector3.ZERO, Quaternion.IDENTITY,
			[_target_block("block-front", Vector3i(0, 0, 2))]
		),
	})
	var block_first: Dictionary = client.call(
		"_closest_tool_hit", Vector3.ZERO, Vector3.BACK, 9.0
	)
	_check(String(block_first.get("kind", "")) == "block", "block occludes farther voxel")

	client.set("voxel_lookup", {"0,0,2": {"material": "rock"}})
	var exact_tie: Dictionary = client.call(
		"_closest_tool_hit", Vector3.ZERO, Vector3.BACK, 9.0
	)
	_check(String(exact_tie.get("kind", "")) == "block", "block wins exact voxel tie")

	client.call("_set_tool_targets_from_hit", voxel_first)
	_check(client.get("target_voxel") == Vector3i(0, 0, 2), "voxel target derived")
	_check((client.get("target_block") as Dictionary).is_empty(), "voxel excludes block target")
	client.call("_set_tool_targets_from_hit", block_first)
	_check(client.get("target_voxel") == null, "block excludes voxel target")
	_check(not (client.get("target_block") as Dictionary).is_empty(), "block target derived")

	client.set("voxel_lookup", {})
	client.set("grid_lookup", {
		"grid-inside": _target_grid(
			"grid-inside", Vector3.ZERO, Quaternion.IDENTITY,
			[_target_block("block-inside", Vector3i.ZERO)]
		),
	})
	var inside_hit: Dictionary = client.call(
		"_closest_tool_hit", Vector3.ZERO, Vector3.BACK, 9.0
	)
	client.call("_set_tool_targets_from_hit", inside_hit)
	_check(is_zero_approx(float(inside_hit.get("distance", -1.0))), "inside geometry occludes at zero")
	_check(not bool(inside_hit.get("has_face", true)), "inside geometry has no entry face")
	_check(client.get("target_voxel") == null, "inside hit cannot mine")
	_check((client.get("target_block") as Dictionary).is_empty(), "inside hit cannot modify block")

	client.set("grid_lookup", {
		"grid-range": _target_grid(
			"grid-range", Vector3(0.0, 0.0, 0.5), Quaternion.IDENTITY,
			[_target_block("block-range", Vector3i(0, 0, 9))]
		),
	})
	var boundary_hit: Dictionary = client.call(
		"_closest_tool_hit", Vector3.ZERO, Vector3.BACK, 9.0
	)
	_check(
		String(boundary_hit.get("kind", "")) == "block"
		and is_equal_approx(float(boundary_hit.get("distance", -1.0)), 9.0),
		"surface exactly at range boundary is included"
	)
	client.set("grid_lookup", {
		"grid-beyond": _target_grid(
			"grid-beyond", Vector3(0.0, 0.0, 0.50001), Quaternion.IDENTITY,
			[_target_block("block-beyond", Vector3i(0, 0, 9))]
		),
	})
	var beyond_hit: Dictionary = client.call(
		"_closest_tool_hit", Vector3.ZERO, Vector3.BACK, 9.0
	)
	_check(beyond_hit.is_empty(), "surface beyond range boundary is excluded")

	client.set("grid_lookup", {
		"grid-z": _target_grid(
			"grid-z", Vector3.ZERO, Quaternion.IDENTITY,
			[_target_block("block-z", Vector3i(0, 0, 2))]
		),
		"grid-a": _target_grid(
			"grid-a", Vector3.ZERO, Quaternion.IDENTITY,
			[_target_block("block-a", Vector3i(0, 0, 2))]
		),
	})
	var stable_hit: Dictionary = client.call(
		"_closest_tool_hit", Vector3.ZERO, Vector3.BACK, 9.0
	)
	_check(String(stable_hit.get("grid_id", "")) == "grid-a", "equal hit uses stable grid identity")
	client.set("grid_lookup", {
		"grid-blocks": _target_grid(
			"grid-blocks", Vector3.ZERO, Quaternion.IDENTITY,
			[
				_target_block("block-z", Vector3i(0, 0, 2)),
				_target_block("block-a", Vector3i(0, 0, 2)),
			]
		),
	})
	var stable_block_hit: Dictionary = client.call(
		"_closest_tool_hit", Vector3.ZERO, Vector3.BACK, 9.0
	)
	_check(
		String(stable_block_hit.get("block", {}).get("block_id", "")) == "block-a",
		"equal hit uses stable block identity"
	)
	client.set("grid_lookup", {})
	client.set("voxel_lookup", {
		"1,0,0": {"material": "rock"},
		"0,0,0": {"material": "rock"},
	})
	var stable_voxel_hit: Dictionary = client.call(
		"_closest_tool_hit", Vector3(0.5, 0.0, 0.0), Vector3.BACK, 9.0
	)
	_check(
		stable_voxel_hit.get("coordinate", Vector3i(99, 99, 99)) == Vector3i.ZERO,
		"equal hit uses stable voxel coordinate identity"
	)
	client.set("voxel_lookup", {"0,0,0": {"material": "rock"}})
	var parallel_boundary_hit: Dictionary = client.call(
		"_closest_tool_hit", Vector3(0.5, 0.0, -4.0), Vector3.BACK, 9.0
	)
	_check(
		parallel_boundary_hit.get("coordinate", Vector3i(99, 99, 99)) == Vector3i.ZERO,
		"parallel boundary ray checks the voxel column on both sides"
	)
	_check(
		absf(float(parallel_boundary_hit.get("distance", 0.0)) - 3.5) <= 0.000000001,
		"parallel boundary voxel retains the exact surface distance"
	)
	client.free()


func _test_actor_owned_industry_selection() -> void:
	var client := _new_client()
	var local := _base_player()
	local["player_id"] = "player-local"
	local["inventory_id"] = "inventory-player-local"
	var remote := _base_player()
	remote["player_id"] = "player-remote"
	remote["inventory_id"] = "inventory-player-remote"
	client.set("bound_player_id", "player-remote")

	var local_cargo := _target_block(
		"block-cargo-local", Vector3i.ZERO, "cargo", "inventory-cargo-local"
	)
	var remote_cargo_a := _target_block(
		"block-cargo-a", Vector3i.ZERO, "cargo", "inventory-cargo-a"
	)
	var remote_cargo_z := _target_block(
		"block-cargo-z", Vector3i.ZERO, "cargo", "inventory-cargo-z"
	)
	var unfinished_cargo := _target_block(
		"block-cargo-frame", Vector3i.ZERO, "cargo", "inventory-cargo-frame", false
	)
	var ambiguous_cargo_first := _target_block(
		"block-cargo-ambiguous", Vector3i.ZERO, "cargo", "inventory-cargo-ambiguous"
	)
	var ambiguous_cargo_second := ambiguous_cargo_first.duplicate(true)
	var grid_local := _target_grid(
		"grid-local", Vector3.ZERO, Quaternion.IDENTITY, [local_cargo], "player-local"
	)
	var grid_remote_a := _target_grid(
		"grid-remote-a", Vector3.ZERO, Quaternion.IDENTITY, [remote_cargo_a], "player-remote"
	)
	var grid_remote_z := _target_grid(
		"grid-remote-z", Vector3.ZERO, Quaternion.IDENTITY, [remote_cargo_z], "player-remote"
	)
	var grid_remote_frame := _target_grid(
		"grid-remote-frame", Vector3.ZERO, Quaternion.IDENTITY,
		[unfinished_cargo], "player-remote"
	)
	var grid_ambiguous_a := _target_grid(
		"grid-ambiguous-a", Vector3.ZERO, Quaternion.IDENTITY,
		[ambiguous_cargo_first], "player-remote"
	)
	var grid_ambiguous_b := _target_grid(
		"grid-ambiguous-b", Vector3.ZERO, Quaternion.IDENTITY,
		[ambiguous_cargo_second], "player-remote"
	)
	var grids := [
		grid_remote_z,
		grid_local,
		grid_remote_frame,
		grid_ambiguous_b,
		grid_remote_a,
		grid_ambiguous_a,
	]
	client.set("grid_lookup", {
		"grid-remote-z": grid_remote_z,
		"grid-local": grid_local,
		"grid-remote-frame": grid_remote_frame,
		"grid-ambiguous-b": grid_ambiguous_b,
		"grid-remote-a": grid_remote_a,
		"grid-ambiguous-a": grid_ambiguous_a,
	})
	var authoritative: Dictionary = client.get("snapshot")
	authoritative["players"] = [_public_player(local), _public_player(remote)]
	authoritative["grids"] = grids
	client.set("actor_private_snapshot", _private_snapshot(remote, [
		_inventory_snapshot("inventory-cargo-z", "cargo", "block-cargo-z"),
		_inventory_snapshot("inventory-player-remote", "player", "player-remote"),
		_inventory_snapshot("inventory-cargo-frame", "cargo", "block-cargo-frame"),
		_inventory_snapshot("inventory-cargo-a", "cargo", "block-cargo-a"),
		_inventory_snapshot("inventory-cargo-ambiguous", "cargo", "block-cargo-ambiguous"),
	]))
	client.set("snapshot", authoritative)

	var candidates: Array = client.call("_owned_cargo_candidates")
	_check(candidates.size() == 2, "only completed uniquely bound owned cargo is selectable")
	_check(
		candidates.size() == 2
		and String(candidates[0].get("inventory_id", "")) == "inventory-cargo-a"
		and String(candidates[1].get("inventory_id", "")) == "inventory-cargo-z",
		"owned cargo selection has stable grid and block order"
	)
	client.set("selected_cargo_inventory_id", "inventory-cargo-z")
	client.set("target_block", {})
	client.call("_refresh_owned_cargo_selection")
	_check(
		String(client.get("selected_cargo_inventory_id")) == "inventory-cargo-z",
		"valid explicit cargo selection is preserved"
	)
	client.set("target_block", {
		"grid_id": "grid-remote-a", "grid": grid_remote_a, "block": remote_cargo_a,
	})
	client.call("_refresh_owned_cargo_selection")
	_check(
		String(client.get("selected_cargo_inventory_id")) == "inventory-cargo-a",
		"targeted owned grid cargo is preferred"
	)
	client.set("target_block", {
		"grid_id": "grid-local", "grid": grid_local, "block": local_cargo,
	})
	_check(not bool(client.call("_target_grid_owned_by_local")), "foreign target is locked")
	_check(
		String(client.call("_owned_grid_for_command", false)).is_empty(),
		"foreign target never falls back to an owned grid"
	)
	client.call("_refresh_owned_cargo_selection")
	_check(
		String(client.get("selected_cargo_inventory_id")) == "inventory-cargo-a",
		"foreign target cannot select its cargo or displace owned cargo"
	)
	client.set("target_block", {})
	_check(
		String(client.call("_owned_grid_for_command", false)) == "grid-ambiguous-a",
		"no target uses the first stable owned grid"
	)

	client.set("active_grid_control_id", "grid-remote-z")
	client.set("target_block", {
		"grid_id": "grid-local", "grid": grid_local, "block": local_cargo,
	})
	_check(
		String(client.call("_take_active_grid_control_id")) == "grid-remote-z",
		"grid release uses the exact press-latched grid despite target changes"
	)
	_check(String(client.get("active_grid_control_id")).is_empty(), "grid release clears latch")

	var transfer_button := Button.new()
	var cargo_selector := OptionButton.new()
	var cargo_title := Label.new()
	var cargo_subtitle := Label.new()
	client.add_child(transfer_button)
	client.add_child(cargo_selector)
	client.add_child(cargo_title)
	client.add_child(cargo_subtitle)
	var transfer_buttons: Array = client.get("inventory_transfer_buttons")
	transfer_buttons.append(transfer_button)
	client.set("inventory_selectors", {"cargo": cargo_selector})
	client.set("inventory_title_labels", {"cargo": cargo_title})
	client.set("inventory_subtitle_labels", {"cargo": cargo_subtitle})
	var available_candidates: Array = client.call("_refresh_owned_cargo_selection")
	client.call("_update_cargo_inventory_selector", available_candidates)
	_check(not transfer_button.disabled, "authorized cargo enables transfer controls")
	_check(not cargo_selector.disabled, "authorized cargo enables cargo selector")
	_check(cargo_selector.item_count == 2, "selector exposes all authorized cargo links")
	_check(
		String(cargo_selector.get_item_metadata(cargo_selector.selected))
		== "inventory-cargo-a",
		"selector identifies the preserved authorized cargo link"
	)
	_check(cargo_title.text == "GRID-REMOTE-A", "authorized cargo identifies its owned grid")
	_check(
		cargo_subtitle.text == "AUTHORIZED CARGO // block-cargo-a",
		"authorized cargo identifies its containing block"
	)
	client.set("bound_player_id", "player-without-assets")
	var empty_candidates: Array = client.call("_refresh_owned_cargo_selection")
	client.call("_update_cargo_inventory_selector", empty_candidates)
	_check(empty_candidates.is_empty(), "player without assets has no cargo candidates")
	_check(transfer_button.disabled, "no cargo disables transfer controls")
	_check(cargo_selector.disabled, "no cargo disables cargo selector")
	_check(
		cargo_title.text == "NO AUTHORIZED CARGO LINK",
		"no cargo presents the explicit authorization state"
	)
	client.free()


func _test_private_projection_lifecycle() -> void:
	var client := _new_client()
	var player := _base_player()
	var private_state := _private_snapshot(player)
	client.set("selected_cargo_inventory_id", "stale-cargo")
	_check(
		bool(client.call("_install_actor_private", private_state, 0)),
		"matching actor-private projection installs"
	)
	_check(
		String(client.call("_local_inventory_id")) == "inventory-impairment-player",
		"installed projection exposes only the bound carried inventory"
	)

	client.set("selected_cargo_inventory_id", "stale-cargo")
	_check(
		not bool(client.call("_install_actor_private", {}, 1)),
		"missing actor-private projection fails closed"
	)
	_check(
		(client.get("actor_private_snapshot") as Dictionary).is_empty()
		and String(client.get("selected_cargo_inventory_id")).is_empty(),
		"missing projection clears cached inventories and cargo selection"
	)

	var wrong_actor := private_state.duplicate(true)
	wrong_actor["player"]["player_id"] = "player-foreign"
	wrong_actor["player"]["inventory_id"] = "inventory-player-foreign"
	wrong_actor["inventories"] = [
		_inventory_snapshot("inventory-player-foreign", "player", "player-foreign"),
	]
	_check(
		not bool(client.call("_install_actor_private", wrong_actor, 2)),
		"mismatched actor-private projection fails closed"
	)
	var wrong_sequence := private_state.duplicate(true)
	wrong_sequence["event_sequence"] = 8
	_check(
		not bool(client.call("_install_actor_private", wrong_sequence, 7)),
		"explicit mismatched private sequence fails closed"
	)

	_check(bool(client.call("_install_actor_private", private_state, 0)), "private state restored")
	var private_before: Dictionary = client.get("actor_private_snapshot")
	var spoofed_public_motion := _public_player(player)
	spoofed_public_motion["position"] = _protocol_vec3(Vector3(1.0, 0.0, 0.0))
	spoofed_public_motion["last_processed_input_sequence"] = 99
	client.call("_apply_motion_state", {
		"projection_schema_version": 1,
		"event_sequence": 1,
		"simulation_tick": 1,
		"world_hash": "public-only-motion",
		"players": [spoofed_public_motion],
		"grids": [],
	})
	_check(
		(client.get("actor_private_snapshot") as Dictionary).get("inventories", [])
		== private_before.get("inventories", []),
		"public-only motion preserves the valid private inventory overlay"
	)
	_check(
		int(client.get("last_acked_input_sequence")) == 0,
		"public motion cannot forge the bound actor acknowledgement"
	)
	var private_motion := player.duplicate(true)
	private_motion["position"] = _protocol_vec3(Vector3(2.0, 0.0, 0.0))
	private_motion["last_received_input_sequence"] = 2
	private_motion["last_processed_input_sequence"] = 2
	client.call("_apply_motion_state", {
		"projection_schema_version": 1,
		"event_sequence": 2,
		"simulation_tick": 2,
		"world_hash": "private-motion",
		"players": [_public_player(private_motion)],
		"grids": [],
		"actor_private": private_motion,
	})
	_check(
		int(client.get("last_acked_input_sequence")) == 2,
		"actor-private motion advances the bound acknowledgement"
	)
	var public_view := _public_player(player)
	_check(
		public_view.get("life_state") is String,
		"public player life state uses the redacted string enum"
	)
	_check(
		not public_view.has("inventory_id")
		and not public_view.has("last_processed_input_sequence")
		and not public_view.has("control_linear_input"),
		"public players require no exact remote inventory or control fields"
	)

	client.set("selected_cargo_inventory_id", "stale-after-private-motion")
	var wrong_private_motion := private_motion.duplicate(true)
	wrong_private_motion["player_id"] = "player-foreign"
	client.set("authoritative_player_ready", true)
	client.call("_apply_motion_state", {
		"projection_schema_version": 1,
		"event_sequence": 3,
		"simulation_tick": 3,
		"world_hash": "wrong-private-actor-motion",
		"players": [_public_player(player)],
		"grids": [],
		"actor_private": wrong_private_motion,
	})
	_check(
		(client.get("actor_private_snapshot") as Dictionary).is_empty()
		and String(client.get("selected_cargo_inventory_id")).is_empty(),
		"present motion for the wrong private actor clears all cached private state"
	)
	_check(
		not bool(client.get("authoritative_player_ready")),
		"wrong-actor private motion fails closed pending resnapshot"
	)

	_check(
		bool(client.call("_install_actor_private", private_state, 3)),
		"private state can be restored after invalid motion"
	)
	client.set("authoritative_player_ready", true)
	client.call("_apply_motion_state", {
		"projection_schema_version": 99,
		"event_sequence": 4,
		"simulation_tick": 4,
		"world_hash": "wrong-projection-motion",
		"players": [_public_player(player)],
		"grids": [],
	})
	_check(
		(client.get("actor_private_snapshot") as Dictionary).is_empty(),
		"wrong-schema motion clears the restored private projection"
	)
	_check(
		not bool(client.get("authoritative_player_ready")),
		"wrong-schema motion fails closed pending resnapshot"
	)
	client.free()


func _inventory_snapshot(
	inventory_id: String, domain_kind: String, owner_id: String
) -> Dictionary:
	var domain := {"kind": domain_kind}
	if domain_kind == "cargo":
		domain["block_id"] = owner_id
	else:
		domain["player_id"] = owner_id
	return {
		"inventory_id": inventory_id,
		"domain": domain,
		"contents": {"ore": 0, "refined_material": 0, "components": 0},
		"capacity_liters": 100,
		"used_liters": 0,
		"mass_grams": 0,
	}


func _target_grid(
	_grid_id: String,
	position: Vector3,
	orientation: Quaternion,
	blocks: Array,
	owner_player_id := ""
) -> Dictionary:
	return {
		"grid_id": _grid_id,
		"owner_player_id": owner_player_id,
		"position": _protocol_vec3(position),
		"orientation": _protocol_quat(orientation),
		"blocks": blocks,
	}


func _target_block(
	block_id: String,
	coordinate: Vector3i,
	kind := "structural",
	_inventory_id := "",
	construction_complete := true
) -> Dictionary:
	return {
		"block_id": block_id,
		"coordinate": {
			"x": coordinate.x, "y": coordinate.y, "z": coordinate.z,
		},
		"kind": kind,
		"health": 100,
		"max_health": 100,
		"construction_complete": construction_complete,
	}


func _test_bound_player_roster_selection() -> void:
	var client := _new_client()
	var primary := _base_player()
	primary["player_id"] = "player-local"
	primary["inventory_id"] = "inventory-player-local"
	primary["environment"] = {
		"celestial_body_name": "Khepri Prime",
		"planet_center": _protocol_vec3(Vector3.ZERO),
		"surface_radius_m": 1200.0,
		"gravity": _protocol_vec3(Vector3(0.0, -0.5, 0.0)),
	}
	var remote := _base_player()
	remote["player_id"] = "player-remote"
	remote["inventory_id"] = "inventory-player-remote"
	remote["position"] = _protocol_vec3(Vector3(4.0, 0.0, 0.0))
	remote["environment"] = {
		"celestial_body_name": "Remote Frontier",
		"planet_center": _protocol_vec3(Vector3(100.0, 0.0, 0.0)),
		"surface_radius_m": 1200.0,
		"gravity": _protocol_vec3(Vector3(-2.0, 0.0, 0.0)),
	}
	client.set("bound_player_id", "player-remote")
	client.set("actor_private_snapshot", _private_snapshot(remote))
	var roster_snapshot: Dictionary = client.get("snapshot")
	roster_snapshot["players"] = [_public_player(primary), _public_player(remote)]
	client.set("snapshot", roster_snapshot)
	var selected: Dictionary = client.call("_local_player")
	_check(String(selected.get("player_id", "")) == "player-remote", "bound actor selected")
	_check(
		String(client.call("_local_inventory_id")) == "inventory-player-remote",
		"bound actor inventory selected"
	)
	remote["position"] = _protocol_vec3(Vector3(9.0, 1.0, -2.0))
	client.call("_apply_motion_state", {
		"projection_schema_version": 1,
		"event_sequence": 1,
		"simulation_tick": 1,
		"world_hash": "impairment-roster-1",
		"players": [_public_player(primary), _public_player(remote)],
		"grids": [],
		"actor_private": remote,
	})
	var merged: Dictionary = client.call("_local_player")
	_check(String(merged.get("player_id", "")) == "player-remote", "motion keeps bound actor")
	_check(
		client.call("_vec3", merged.get("position", {})).is_equal_approx(Vector3(9.0, 1.0, -2.0)),
		"bound actor motion merged"
	)
	_check(
		String((client.call("_local_environment") as Dictionary).get(
			"celestial_body_name", ""
		)) == "Remote Frontier",
		"bound actor environment selected"
	)
	_check(
		(client.get("prediction_gravity_fallback") as Vector3).is_equal_approx(
			Vector3(-2.0, 0.0, 0.0)
		),
		"bound actor gravity selected"
	)
	remote["life_state"] = {
		"kind": "incapacitated",
		"death_id": "remote-death",
		"cause": {"kind": "oxygen_depleted"},
	}
	client.set("actor_private_snapshot", _private_snapshot(remote))
	_check(bool(client.call("_local_player_incapacitated")), "bound remote death gates controls")
	_check(
		String((client.call("_local_player") as Dictionary).get("player_id", ""))
		== "player-remote",
		"primary alive state cannot replace dead bound actor"
	)
	client.set("bound_player_id", "player-absent")
	_check((client.call("_local_player") as Dictionary).is_empty(), "missing bound actor fails closed")
	_check(
		not bool(client.call("_player_controls_enabled", {})),
		"empty actor state cannot enable controls"
	)
	_check(String(client.call("_local_inventory_id")).is_empty(), "missing actor has no inventory")


func _test_received_vs_processed_reconciliation() -> void:
	var client := _new_client()
	var press := _control(Vector3(0.0, 0.0, 1.0))
	var release := _control(Vector3.ZERO)
	var one_step: Dictionary = client.call(
		"_integrate_player_motion",
		Vector3.ZERO,
		Quaternion.IDENTITY,
		Vector3.ZERO,
		Vector3.ZERO,
		press,
		Vector3.ZERO,
		true,
		FIXED_DELTA
	)
	var two_steps: Dictionary = client.call(
		"_integrate_player_motion",
		one_step.get("position", Vector3.ZERO),
		one_step.get("orientation", Quaternion.IDENTITY),
		one_step.get("linear_velocity", Vector3.ZERO),
		one_step.get("angular_velocity", Vector3.ZERO),
		release,
		Vector3.ZERO,
		true,
		FIXED_DELTA
	)

	var pending: Array = client.get("pending_controls")
	pending.append({"movement_epoch": 1, "input_sequence": 1, "control": press})
	pending.append({"movement_epoch": 1, "input_sequence": 2, "control": release})
	client.set("current_prediction_input_sequence", 1)
	client.call("_predict_player_step", press, FIXED_DELTA, true)
	client.set("current_prediction_input_sequence", 2)
	client.call("_predict_player_step", release, FIXED_DELTA, true)

	client.call(
		"_apply_motion_state",
		_motion_message(1, 0, _player_from_motion({}, 2, 0))
	)
	_check((client.get("pending_controls") as Array).size() == 2, "received-only kept controls")
	_check((client.get("prediction_history") as Array).size() == 2, "received-only replayed frames")

	client.call(
		"_apply_motion_state",
		_motion_message(2, 1, _player_from_motion(one_step, 2, 1))
	)
	var after_press: Array = client.get("pending_controls")
	_check(after_press.size() == 1, "processed press dropped one control")
	_check(
		after_press.size() == 1 and int(after_press[0].get("input_sequence", 0)) == 2,
		"processed press retained release"
	)
	_check((client.get("prediction_history") as Array).size() == 1, "release replayed once")

	client.call(
		"_apply_motion_state",
		_motion_message(3, 2, _player_from_motion(two_steps, 2, 2))
	)
	_check((client.get("pending_controls") as Array).is_empty(), "processed neutral drained controls")
	_check((client.get("prediction_history") as Array).is_empty(), "processed neutral drained history")
	_check(
		(client.get("predicted_orientation") as Quaternion).is_equal_approx(
			two_steps.get("orientation", Quaternion.IDENTITY)
		),
		"processed neutral converged orientation"
	)
	client.free()


func _test_ordering_corrections_and_motion_only_updates() -> void:
	var client := _new_client()
	var grid_node := Node3D.new()
	client.add_child(grid_node)
	client.set("grid_lookup", {
		"grid-a": {
			"grid_id": "grid-a",
			"position": _protocol_vec3(Vector3.ZERO),
			"orientation": _protocol_quat(Quaternion.IDENTITY),
			"linear_velocity": _protocol_vec3(Vector3.ZERO),
			"angular_velocity": _protocol_vec3(Vector3.ZERO),
			"blocks": [{"block_id": "sentinel-block"}],
		}
	})
	client.set("grid_node_lookup", {"grid-a": grid_node})
	client.set("rendered_voxel_count", 77)
	client.set("voxel_lookup", {"sentinel": true})
	var canonical := _player_from_motion({}, 0, 0)
	client.call(
		"_apply_motion_state",
		_motion_message(5, 4, canonical, [{
			"grid_id": "grid-a",
			"position": _protocol_vec3(Vector3(2.0, 0.0, 0.0)),
			"orientation": _protocol_quat(Quaternion.IDENTITY),
			"linear_velocity": _protocol_vec3(Vector3.ZERO),
			"angular_velocity": _protocol_vec3(Vector3.ZERO),
		}])
	)
	_check(int(client.get("predicted_simulation_tick")) == 4, "skipped update chose newest tick")
	_check(grid_node.position.is_equal_approx(Vector3(2.0, 0.0, 0.0)), "motion updated grid in place")
	_check(int(client.get("rendered_voxel_count")) == 77, "motion skipped voxel rebuild")
	_check((client.get("voxel_lookup") as Dictionary).has("sentinel"), "motion preserved voxel lookup")
	var grid_after: Dictionary = (client.get("grid_lookup") as Dictionary).get("grid-a", {})
	_check((grid_after.get("blocks", []) as Array).size() == 1, "motion preserved grid structure")

	client.call(
		"_apply_motion_state",
		_motion_message(4, 1, _player_from_motion({"position": Vector3(99.0, 0.0, 0.0)}, 0, 0), [{
			"grid_id": "grid-a", "position": _protocol_vec3(Vector3(99.0, 0.0, 0.0)),
		}])
	)
	_check(int(client.get("last_authoritative_event_sequence")) == 5, "older motion rejected")
	_check(grid_node.position.is_equal_approx(Vector3(2.0, 0.0, 0.0)), "older grid motion rejected")
	client.call("_apply_motion_state", _motion_message(5, 9, canonical))
	_check(int(client.get("predicted_simulation_tick")) == 4, "equal motion rejected")
	client.free()

	var correction_client := _new_client()
	var camera: Camera3D = correction_client.get("camera")
	camera.position = Vector3(0.5, 0.0, 0.0)
	camera.quaternion = Quaternion(Vector3.UP, 0.1)
	correction_client.call("_apply_authoritative_player", canonical, 1, 1, "small", "motion_state")
	var first_position_offset: Vector3 = correction_client.get("presentation_position_offset")
	var first_orientation_offset: Quaternion = correction_client.get("presentation_orientation_offset")
	_check(first_position_offset.length() > 0.0, "small position correction smoothed")
	_check(
		absf(first_orientation_offset.w) < 0.999999,
		"small orientation correction smoothed"
	)
	correction_client.call("_update_player_presentation", 0.016)
	var second_position_offset: Vector3 = correction_client.get("presentation_position_offset")
	var second_orientation_offset: Quaternion = correction_client.get("presentation_orientation_offset")
	_check(second_position_offset.length() < first_position_offset.length(), "position offset decayed")
	_check(
		absf(second_orientation_offset.w) > absf(first_orientation_offset.w),
		"orientation offset decayed"
	)

	camera.position = Vector3(3.0, 0.0, 0.0)
	correction_client.call("_apply_authoritative_player", canonical, 2, 2, "large", "motion_state")
	_check(
		(correction_client.get("presentation_position_offset") as Vector3).is_zero_approx(),
		"large position correction snapped"
	)

	camera.position = Vector3.ZERO
	camera.quaternion = Quaternion(Vector3.UP, 1.3)
	correction_client.call("_apply_authoritative_player", canonical, 3, 3, "large-angle", "motion_state")
	_check(
		(correction_client.get("presentation_orientation_offset") as Quaternion).is_equal_approx(
			Quaternion.IDENTITY
		),
		"large orientation correction snapped"
	)

	(correction_client.get("pending_controls") as Array).append({"input_sequence": 1})
	(correction_client.get("prediction_history") as Array).append({"simulation_tick": 3})
	var epoch_player := canonical.duplicate(true)
	epoch_player["movement_epoch"] = 2
	correction_client.call("_apply_authoritative_player", epoch_player, 4, 4, "epoch", "motion_state")
	_check((correction_client.get("pending_controls") as Array).is_empty(), "epoch cleared controls")
	_check((correction_client.get("prediction_history") as Array).is_empty(), "epoch cleared history")

	correction_client.set("predicted_simulation_tick", 7)
	(correction_client.get("prediction_history") as Array).append({
		"movement_epoch": 2, "input_sequence": 3, "simulation_tick": 7,
	})
	(correction_client.get("pending_controls") as Array).append({
		"movement_epoch": 2, "input_sequence": 3,
	})
	correction_client.call("_apply_authoritative_player", epoch_player, 5, 5, "gap", "motion_state")
	_check((correction_client.get("prediction_history") as Array).is_empty(), "history gap hard reset")
	_check((correction_client.get("pending_controls") as Array).is_empty(), "history gap cleared controls")

	(correction_client.get("prediction_history") as Array).append({"simulation_tick": 5})
	(correction_client.get("pending_controls") as Array).append({"input_sequence": 4})
	correction_client.call("_apply_authoritative_player", epoch_player, 6, 6, "reconnect", "reconnect")
	_check((correction_client.get("prediction_history") as Array).is_empty(), "reconnect cleared history")
	_check((correction_client.get("pending_controls") as Array).is_empty(), "reconnect cleared controls")
	correction_client.free()


func _test_menu_dead_disconnect_and_bounds() -> void:
	var client := _new_client()
	client.set("prediction_planet_center", Vector3.ZERO)
	client.set("prediction_surface_radius", 1200.0)
	client.set("prediction_gravitational_parameter", 6.2 * 1200.0 * 1200.0)
	client.set("prediction_gravity_model_ready", true)
	client.set("predicted_position", Vector3(2400.0, 0.0, 0.0))
	client.set("desired_dampeners", false)
	client.set("inventory_open", true)
	client.set("connected", true)
	client.set("require_neutral_baseline", false)
	var neutral: Dictionary = client.call("_neutral_player_control")
	client.set("last_sent_control", neutral.duplicate(true))
	client.set("control_send_elapsed", 0.0)
	client.set("mouse_delta_accumulator", Vector2(9.0, 7.0))
	client.call("_physics_process", FIXED_DELTA)
	_check(int(client.get("predicted_simulation_tick")) == 1, "menu continued prediction")
	_check((client.get("predicted_linear_velocity") as Vector3).x < 0.0, "menu accumulated gravity")
	_check((client.get("mouse_delta_accumulator") as Vector2).is_zero_approx(), "menu cleared mouse")
	_check(
		int(client.get("next_input_sequence")) == 1,
		"menu did not refresh unchanged neutral control before due"
	)

	client.set("connected", false)
	var disconnected_tick := int(client.get("predicted_simulation_tick"))
	client.call("_physics_process", FIXED_DELTA)
	_check(int(client.get("predicted_simulation_tick")) == disconnected_tick, "disconnected prediction stopped")
	client.set("connected", true)
	var dead_player := _base_player()
	dead_player["life_state"] = {"kind": "incapacitated"}
	client.set("actor_private_snapshot", _private_snapshot(dead_player))
	client.call("_physics_process", FIXED_DELTA)
	_check(int(client.get("predicted_simulation_tick")) == disconnected_tick, "dead prediction stopped")

	(client.get("pending_controls") as Array).append({"input_sequence": 1})
	(client.get("prediction_history") as Array).append({"simulation_tick": 2})
	client.set("mouse_delta_accumulator", Vector2.ONE)
	client.call("_begin_player_resync")
	_check((client.get("pending_controls") as Array).is_empty(), "resync cleared controls")
	_check((client.get("prediction_history") as Array).is_empty(), "resync cleared history")
	_check((client.get("mouse_delta_accumulator") as Vector2).is_zero_approx(), "resync cleared mouse")

	var bound_client := _new_client()
	var bound_control := _control(Vector3.ZERO)
	for index in HISTORY_LIMIT + 1:
		bound_client.set("current_prediction_input_sequence", index + 1)
		bound_client.call("_predict_player_step", bound_control, FIXED_DELTA, true)
	_check((bound_client.get("prediction_history") as Array).size() == HISTORY_LIMIT, "history bounded")
	_check(bool(bound_client.get("prediction_history_invalid")), "history overflow invalidated replay")
	bound_client.free()

	var pending_bound_client := _new_client()
	for index in HISTORY_LIMIT + 1:
		pending_bound_client.call("_record_pending_control", 1, index + 1, bound_control)
	var bounded_pending: Array = pending_bound_client.get("pending_controls")
	_check(bounded_pending.size() == HISTORY_LIMIT, "pending controls bounded")
	_check(bool(pending_bound_client.get("prediction_history_invalid")), "pending overflow invalidated replay")
	_check(
		bounded_pending.size() == HISTORY_LIMIT
		and int(bounded_pending.front().get("input_sequence", 0)) == 2
		and int(bounded_pending.back().get("input_sequence", 0)) == HISTORY_LIMIT + 1,
		"pending append path retained newest controls"
	)
	pending_bound_client.free()
	client.free()


func _test_life_state_reset() -> void:
	var client := _new_client(true)
	(client.get("pending_controls") as Array).append({"input_sequence": 1})
	(client.get("prediction_history") as Array).append({"simulation_tick": 1})
	client.set("mouse_delta_accumulator", Vector2.ONE)
	var dead_player := _base_player()
	dead_player["life_state"] = {
		"kind": "incapacitated",
		"death_id": "impairment-death",
		"cause": {"kind": "oxygen_depleted"},
	}
	client.call("_apply_authoritative_player", dead_player, 1, 1, "death", "motion_state")
	_check((client.get("pending_controls") as Array).is_empty(), "death cleared controls")
	_check((client.get("prediction_history") as Array).is_empty(), "death cleared history")
	_check((client.get("mouse_delta_accumulator") as Vector2).is_zero_approx(), "death cleared mouse")
	_check(not bool(client.get("require_neutral_baseline")), "death blocked baseline send")
	client.free()


func _test_short_roll_taps_and_idle_silence() -> void:
	var client := _new_client()
	client.call("_register_inputs")
	var previous_mouse_mode := Input.mouse_mode
	Input.mouse_mode = Input.MOUSE_MODE_CAPTURED

	# Both transitions happen before a physics sample. The press must remain at
	# the front of the queue and the release must follow it on the next sample.
	client.call("_capture_roll_key_transition", _key_event(KEY_Q, true))
	client.call("_capture_roll_key_transition", _key_event(KEY_Q, false))
	var q_transitions: Array = client.get("pending_roll_transitions")
	_check(q_transitions.size() == 2, "short Q tap retained press and release")
	_check(
		q_transitions.size() == 2 and float(q_transitions[0]) > 0.99,
		"Q tap rolls left with positive local Z"
	)
	_check(
		q_transitions.size() == 2 and is_zero_approx(float(q_transitions[1])),
		"Q release follows the queued press"
	)
	q_transitions.clear()

	client.call("_capture_roll_key_transition", _key_event(KEY_E, true))
	client.call("_capture_roll_key_transition", _key_event(KEY_E, false))
	var e_transitions: Array = client.get("pending_roll_transitions")
	_check(
		e_transitions.size() == 2 and float(e_transitions[0]) < -0.99,
		"E tap rolls right with negative local Z"
	)
	_check(
		e_transitions.size() == 2 and is_zero_approx(float(e_transitions[1])),
		"E release follows the queued press"
	)
	e_transitions.clear()

	var combined: Vector3 = client.call("_bounded_angular_input", Vector2(8.0, -6.0), 1.0)
	_check(
		combined.length() <= 0.9999991,
		"combined mouse and roll input keeps float32-safe headroom"
	)
	client.call("_capture_roll_key_transition", _key_event(KEY_Q, true))
	client.call("_clear_transient_character_input")
	_check(
		(client.get("pending_roll_transitions") as Array).is_empty(),
		"modal reset clears queued roll transitions"
	)

	var idle: Dictionary = client.call("_neutral_player_control")
	var held := idle.duplicate(true)
	held["angular_input"] = Vector3(0.0, 0.0, 0.5)
	var drift := idle.duplicate(true)
	drift["dampeners"] = false
	_check(
		not bool(client.call("_control_send_due", idle, idle.duplicate(true), 60.0)),
		"unchanged idle control does not create periodic traffic"
	)
	_check(
		bool(client.call("_control_send_due", held, held.duplicate(true), 0.10)),
		"held roll refreshes its authoritative lease"
	)
	_check(
		bool(client.call("_control_send_due", drift, drift.duplicate(true), 0.10)),
		"dampeners-off drift refreshes its authoritative lease"
	)
	_check(
		bool(client.call("_control_send_due", idle, held, 0.0)),
		"neutral release transition sends immediately"
	)

	Input.mouse_mode = previous_mouse_mode
	client.free()


func _key_event(keycode: Key, pressed: bool) -> InputEventKey:
	var event := InputEventKey.new()
	event.keycode = keycode
	event.physical_keycode = keycode
	event.pressed = pressed
	event.echo = false
	return event


func _protocol_vec3(value: Vector3) -> Dictionary:
	return {"x": value.x, "y": value.y, "z": value.z}


func _protocol_quat(value: Quaternion) -> Dictionary:
	return {"x": value.x, "y": value.y, "z": value.z, "w": value.w}
