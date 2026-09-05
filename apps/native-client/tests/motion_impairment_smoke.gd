# SPDX-License-Identifier: AGPL-3.0-or-later
extends SceneTree

const CLIENT_SCRIPT: Script = preload("res://src/main.gd")
const FIXED_DELTA := 1.0 / 60.0
const HISTORY_LIMIT := 180

var failures: Array[String] = []


class HeadlessToolClient extends "res://src/main.gd":
	# The headless display cannot capture a pointer. Substitute only that device
	# input; connection, actor, life, frontier, and mutation gates remain real.
	func _tool_pointer_captured() -> bool:
		return true


class OutOfRangePresentationVerifier extends RefCounted:
	var commits := 0
	var discards := 0

	func stage_server_message(_packet: PackedByteArray) -> Dictionary:
		return {
			"ok": true,
			"token": 7,
			"kind": "interest_baseline",
			"sanitized_frame": JSON.stringify({
				"type": "interest_baseline",
				"baseline": {
					"event_sequence": "__VERSE_LOSSLESS_INTEGER__:18446744073709551616",
				},
			}),
		}

	func discard(token: int) -> Dictionary:
		if token == 7:
			discards += 1
		return {"ok": true}

	func commit(_token: int) -> Dictionary:
		commits += 1
		return {
			"ok": true,
			"acknowledgement": '{"type":"acknowledge_interest"}'.to_utf8_buffer(),
		}


func _initialize() -> void:
	call_deferred("_run")


func _run() -> void:
	_test_received_vs_processed_reconciliation()
	_test_ordering_corrections_and_motion_only_updates()
	_test_render_interpolation_and_grounded_pitch_prediction()
	_test_grounded_prediction_honors_grid_obstacles()
	_test_resting_floor_contact_allows_tangential_walk()
	_test_remote_player_presentation_smoothing()
	_test_menu_dead_disconnect_and_bounds()
	_test_life_state_reset()
	_test_bound_player_roster_selection()
	_test_short_roll_taps_and_idle_silence()
	_test_exact_tool_targeting()
	_test_starter_tool_kit()
	_test_private_projection_lifecycle()
	_test_actor_owned_industry_selection()
	_test_physical_production_client()
	_test_mutation_frontier_reconciliation()
	_test_verified_ack_order_and_presentation_failure()
	_test_reconnect_policy_and_generation_reset()
	_test_binary_json_frame_rejected()
	_test_lossless_verified_presentation()
	if not failures.is_empty():
		for failure in failures:
			printerr("VERSE_NATIVE_IMPAIRMENT_FAILED %s" % failure)
		quit(1)
		return
	print(
		"VERSE_NATIVE_IMPAIRMENT_OK queued_ack=ordered motion=monotonic corrections=bounded presentation=interpolated grounded_obstacles=blocked remote=smooth pitch=predicted menu=neutral_prediction lifecycle=reset buffers=bounded roll_tap=durable idle=silent rebuild=none targeting=closest_hit ownership=filtered privacy=projected operations=serialized production=physical"
	)
	quit(0)


func _check(condition: bool, label: String) -> void:
	if not condition:
		failures.append(label)


func _new_client(add_to_tree := false, client_script: Script = CLIENT_SCRIPT) -> Node3D:
	var client := Node3D.new()
	if add_to_tree:
		root.add_child(client)
	client.set_script(client_script)
	var camera := Camera3D.new()
	client.add_child(camera)
	client.set("camera", camera)
	var player := _base_player()
	client.set("snapshot", {
		"projection_schema_version": 4,
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
	client.set("committed_operation_sequence", 0)
	client.set("committed_operation_actor_id", "impairment-player")
	client.set("operation_frontier_observed", true)
	client.set("operation_frontier_ready", true)
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


func _test_lossless_verified_presentation() -> void:
	var client := _new_client()
	var presentation: Variant = JSON.parse_string(JSON.stringify({
		"event_sequence": "__VERSE_LOSSLESS_INTEGER__:9007199254740995",
		"interest": {"interest_epoch": "__VERSE_LOSSLESS_INTEGER__:9007199254740993"},
		"inventory": {
			"quantity": "__VERSE_LOSSLESS_INTEGER__:9007199254741011",
			"mass_grams": "__VERSE_LOSSLESS_INTEGER__:9007199254741013",
			"capacity_liters": "__VERSE_LOSSLESS_INTEGER__:9007199254741015",
		},
		"job": {
			"progress_ticks": "__VERSE_LOSSLESS_INTEGER__:9007199254741017",
			"duration_ticks": "__VERSE_LOSSLESS_INTEGER__:9007199254741019",
		},
		"literal": "__VERSE_LOSSLESS_STRING__:__VERSE_LOSSLESS_INTEGER__:7",
	}))
	_check(
		presentation is Dictionary
		and bool(client.call("_decode_lossless_protocol_integers", presentation)),
		"verified presentation lossless integers decode",
	)
	if presentation is Dictionary:
		_check(
			typeof(presentation.get("event_sequence", null)) == TYPE_INT
			and int(presentation.get("event_sequence", 0)) == 9007199254740995
			and int(presentation.get("interest", {}).get("interest_epoch", 0))
			== 9007199254740993
			and int(presentation.get("inventory", {}).get("quantity", 0))
			== 9007199254741011
			and int(presentation.get("inventory", {}).get("mass_grams", 0))
			== 9007199254741013
			and int(presentation.get("inventory", {}).get("capacity_liters", 0))
			== 9007199254741015
			and int(presentation.get("job", {}).get("progress_ticks", 0))
			== 9007199254741017
			and int(presentation.get("job", {}).get("duration_ticks", 0))
			== 9007199254741019,
			"verified frontiers, inventory, mass, capacity, and progress stay exact",
		)
		_check(
			String(presentation.get("literal", "")) == "__VERSE_LOSSLESS_INTEGER__:7",
			"reserved-prefix protocol strings round-trip without reinterpretation",
		)
		var exact_player := _base_player()
		var exact_private := _private_snapshot(exact_player)
		exact_private["committed_operation_sequence"] = presentation["inventory"]["quantity"]
		var exact_inventory: Dictionary = exact_private["inventories"][0]
		exact_inventory["contents"]["ore"] = presentation["inventory"]["quantity"]
		exact_inventory["capacity_liters"] = presentation["inventory"]["capacity_liters"]
		exact_inventory["used_liters"] = presentation["inventory"]["quantity"]
		exact_inventory["mass_grams"] = presentation["inventory"]["mass_grams"]
		exact_private["production_queues"] = [{
			"machine_block_id": "machine-exact",
			"jobs": [{
				"progress_ticks": presentation["job"]["progress_ticks"],
				"duration_ticks": presentation["job"]["duration_ticks"],
			}],
		}]
		var exact_world: Dictionary = client.get("snapshot").duplicate(true)
		exact_world["event_sequence"] = presentation["event_sequence"]
		exact_world["simulation_tick"] = presentation["event_sequence"]
		exact_world["actor_private"] = exact_private
		client.call("_install_verified_interest_model", {
			"world": exact_world,
			"entities": {"player": {}, "grid": {}, "voxel_chunk": {}, "death_drop": {}},
			"origin": {},
			"session_epoch": "lossless-install",
			"interest_epoch": presentation["interest"]["interest_epoch"],
			"baseline_id": "lossless-install",
			"delta_sequence": 0,
			"view_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		})
		var installed_private: Dictionary = client.get("actor_private_snapshot")
		var installed_inventory: Dictionary = installed_private.get("inventories", [])[0]
		var installed_job: Dictionary = installed_private.get("production_queues", [])[0].get(
			"jobs", []
		)[0]
		_check(
			int(client.get("interest_epoch")) == 9007199254740993
			and int(client.get("committed_operation_sequence")) == 9007199254741011
			and int(installed_inventory.get("contents", {}).get("ore", 0))
			== 9007199254741011
			and int(installed_inventory.get("mass_grams", 0)) == 9007199254741013
			and int(installed_inventory.get("capacity_liters", 0)) == 9007199254741015
			and int(installed_job.get("progress_ticks", 0)) == 9007199254741017,
			"full verified candidate install preserves unsafe frontier, quantity, mass, capacity, and progress",
		)
	var unsigned_revision := {
		"revision": "__VERSE_LOSSLESS_INTEGER__:18446744073709551615"
	}
	_check(
		bool(client.call("_decode_lossless_protocol_integers", unsigned_revision))
		and typeof(unsigned_revision.get("revision", null)) == TYPE_STRING
		and String(unsigned_revision.get("revision", "")) == "18446744073709551615"
		and String(client.call(
			"_protocol_exact_unsigned_text", unsigned_revision.get("revision", null)
		)) == "18446744073709551615",
		"verified u64 identity values above i64 remain exact decimal strings",
	)
	var overflow := {"event_sequence": "__VERSE_LOSSLESS_INTEGER__:18446744073709551616"}
	_check(
		not bool(client.call("_decode_lossless_protocol_integers", overflow)),
		"native presentation rejects verified integers outside the protocol u64 range",
	)
	var verifier := OutOfRangePresentationVerifier.new()
	client.set("interest_verifier", verifier)
	client.set("test_capture_transport", true)
	client.call("_verify_and_handle_packet", PackedByteArray([123, 125]))
	_check(
		client.get("replication_state") == "fatal"
		and verifier.discards == 1
		and verifier.commits == 0
		and (client.get("test_outbound_trace") as Array).is_empty(),
		"out-of-u64 presentation is discarded before commit and cannot emit an ACK",
	)
	client.free()


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
		"committed_operation_sequence": 0,
		"inventories": private_inventories,
		"death_drops": death_drops,
		"owned_grid_masses": [],
		"production_queues": [],
	}


func _test_verified_ack_order_and_presentation_failure() -> void:
	var client := _new_client()
	client.set("connected", true)
	client.set("test_capture_transport", true)
	client.set("test_fail_interest_presentation", true)
	client.set("mutation_queue_actor_id", "impairment-player")
	var queued_mutations: Array[Dictionary] = client.get("mutation_queue")
	queued_mutations.append({
		"type": "set_suit_mode",
		"operation_id": "ordered-after-ack",
		"helmet_closed": true,
		"jetpack_enabled": true,
		"dampeners": true,
	})
	client.set("authoritative_player_ready", false)
	client.set("operation_frontier_ready", false)
	client.set("mutation_resync_required", true)
	var world: Dictionary = client.get("snapshot").duplicate(true)
	world["event_sequence"] = 1
	world["simulation_tick"] = 1
	world["world_hash"] = "verified-order-1"
	world["actor_private"] = client.get("actor_private_snapshot").duplicate(true)
	var candidate := {
		"world": world,
		"entities": {"player": {}, "grid": {}, "voxel_chunk": {}, "death_drop": {}},
		"origin": {},
		"session_epoch": "ordered-session",
		"interest_epoch": 1,
		"baseline_id": "ordered-baseline",
		"delta_sequence": 0,
		"view_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
	}
	var finalized := bool(client.call(
		"_finalize_committed_interest", candidate, "{\"type\":\"acknowledge_interest\"}".to_utf8_buffer()
	))
	var trace: Array[String] = client.get("test_outbound_trace")
	_check(
		finalized
		and trace == ["interest_ack", "gameplay:set_suit_mode"],
		"verified ACK is outbound before queued gameplay intent: trace=%s queue=%s inflight=%s connected=%s auth=%s frontier=%s resync=%s actor=%s capture=%s"
		% [
			trace, client.get("mutation_queue"), client.get("in_flight_mutation"), client.get("connected"),
			client.get("authoritative_player_ready"), client.get("operation_frontier_ready"),
			client.get("mutation_resync_required"), client.call("_mutation_actor_matches_session"),
			client.get("test_capture_transport"),
		],
	)
	_check(
		int(client.get("interest_delta_sequence")) == 0
		and String(client.get("interest_view_hash")) == candidate["view_hash"]
		and int((client.get("snapshot") as Dictionary).get("event_sequence", -1)) == 1
		and bool(client.get("authoritative_player_ready"))
		and String(client.get("replication_state")) == "ready"
		and String(client.get("replication_detail")).contains("PRESENTATION DEGRADED"),
		"presentation failure preserves the verified model and ready authority",
	)
	client.free()


func _test_reconnect_policy_and_generation_reset() -> void:
	var client := _new_client()
	var expected := [0.5, 1.0, 2.0, 4.0, 8.0]
	for index in expected.size():
		client.set("auto_reconnect_scheduled", false)
		_check(bool(client.call("_schedule_auto_reconnect")), "bounded reconnect attempt schedules")
		_check(
			is_equal_approx(float(client.get("auto_reconnect_elapsed")), expected[index]),
			"reconnect exponential backoff attempt %d" % (index + 1),
		)
	client.set("auto_reconnect_scheduled", false)
	_check(
		not bool(client.call("_schedule_auto_reconnect"))
		and int(client.get("auto_reconnect_attempts")) == 5,
		"automatic reconnect stops at its attempt bound",
	)
	client.set("replication_state", "fatal")
	client.set("auto_reconnect_attempts", 0)
	_check(
		not bool(client.call("_schedule_auto_reconnect")),
		"fatal replication state never schedules reconnect",
	)

	client.set("replication_state", "loading")
	var verifier: Object = ClassDB.instantiate("VerseInterestVerifier")
	client.set("interest_verifier", verifier)
	_check(bool(client.call("_reset_interest_verifier")), "generation verifier initializes")
	var welcome := {
		"type": "welcome", "protocol_version": 18, "projection_schema_version": 4,
		"world_schema_version": 20, "event_schema_version": 16,
		"content_schema_version": 11, "content_manifest_version": "p1.5.0",
		"celestial_registry_schema_version": 1, "universe_manifest_schema_version": 4,
		"interest_schema_version": 2, "server_name": "generation-test",
		"session_role": {"kind": "player", "player_id": "impairment-player"},
	}
	var staged: Dictionary = verifier.call(
		"stage_server_message", JSON.stringify(welcome).to_utf8_buffer()
	)
	var old_token := int(staged.get("token", -1))
	client.set("interest_delta_sequence", 9)
	client.set("baseline_request_pending", true)
	client.set("operation_frontier_ready", true)
	var old_generation := int(client.get("connection_generation"))
	_check(bool(client.call("_begin_connection_generation")), "new connection generation resets")
	var stale_commit: Dictionary = verifier.call("commit", old_token)
	_check(
		int(client.get("connection_generation")) == old_generation + 1
		and int(client.get("interest_delta_sequence")) == -1
		and not bool(client.get("baseline_request_pending"))
		and not bool(client.get("operation_frontier_ready"))
		and not bool(stale_commit.get("ok", true))
		and String(stale_commit.get("error_code", "")) == "invalid_stage_token",
		"new generation invalidates old stage and every interest frontier",
	)
	client.free()


func _test_binary_json_frame_rejected() -> void:
	var client := _new_client()
	var valid_json_bytes := JSON.stringify({"type": "fatal", "code": "x", "message": "x"}).to_utf8_buffer()
	_check(
		not bool(client.call("_accept_server_packet", valid_json_bytes, false))
		and String(client.get("replication_state")) == "fatal"
		and String(client.get("replication_detail")) == "BINARY SERVER FRAME REJECTED",
		"binary WebSocket frame is rejected even when payload is valid JSON",
	)
	client.free()


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
		"projection_schema_version": 4,
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
	var decoded_private_state: Variant = JSON.parse_string(JSON.stringify(private_state))
	_check(
		decoded_private_state is Dictionary
		and typeof(decoded_private_state.get("committed_operation_sequence")) == TYPE_FLOAT
		and bool(client.call("_install_actor_private", decoded_private_state, 0)),
		"JSON-decoded safe integer frontier installs exactly"
	)
	var fractional_frontier := private_state.duplicate(true)
	fractional_frontier["committed_operation_sequence"] = 0.5
	_check(
		not bool(client.call("_install_actor_private", fractional_frontier, 0)),
		"fractional JSON frontier fails closed"
	)
	var unsafe_frontier := private_state.duplicate(true)
	unsafe_frontier["committed_operation_sequence"] = 9007199254740992.0
	_check(
		not bool(client.call("_install_actor_private", unsafe_frontier, 0)),
		"unsafe JSON frontier fails closed"
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
		"projection_schema_version": 4,
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
		"projection_schema_version": 4,
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
		"projection_schema_version": 4,
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


func _test_physical_production_client() -> void:
	var client := _new_client()
	var player := _base_player()
	var cargo_local := _target_block("cargo-local", Vector3i.ZERO, "cargo")
	var refinery := _target_block("refinery-local", Vector3i(1, 0, 0), "refinery")
	var assembler := _target_block("assembler-local", Vector3i(2, 0, 0), "assembler")
	var unfinished := _target_block("refinery-frame", Vector3i(3, 0, 0), "refinery", "", false)
	var foreign_refinery := _target_block("refinery-foreign", Vector3i.ZERO, "refinery")
	var local_grid := _target_grid(
		"grid-production", Vector3.ZERO, Quaternion.IDENTITY,
		[cargo_local, refinery, assembler, unfinished], "impairment-player"
	)
	local_grid["power"] = {"online": true}
	var foreign_grid := _target_grid(
		"grid-foreign-production", Vector3.ZERO, Quaternion.IDENTITY,
		[foreign_refinery], "player-foreign"
	)
	foreign_grid["power"] = {"online": true}
	client.set("snapshot", {"grids": [foreign_grid, local_grid]})
	var private_state := _private_snapshot(player, [
		_inventory_snapshot("inventory-impairment-player", "player", "impairment-player"),
		_inventory_snapshot("cargo-production", "cargo", "cargo-local"),
	])
	private_state["production_queues"] = [{
		"machine_block_id": "refinery-local",
		"jobs": [{
			"job_id": "job-valid",
			"owner_player_id": "impairment-player",
			"machine_block_id": "refinery-local",
			"recipe": "refining",
			"batches": 1,
			"source_inventory_id": "cargo-production",
			"destination_inventory_id": "cargo-production",
			"progress_ticks": 25,
			"duration_ticks": 100,
			"status": "output_blocked",
			"reserved_inputs": {"ore": 2, "refined_material": 0, "components": 0},
			"pending_outputs": {"ore": 0, "refined_material": 1, "components": 0},
		}, {
			"job_id": "job-foreign",
			"owner_player_id": "player-foreign",
			"machine_block_id": "refinery-local",
			"recipe": "refining",
			"batches": 9,
			"progress_ticks": 99,
			"duration_ticks": 100,
			"status": "running",
		}],
	}]
	client.set("actor_private_snapshot", private_state)

	var refinery_route: Dictionary = client.call("_production_route", "refining")
	var assembler_route: Dictionary = client.call("_production_route", "component")
	_check(
		String(refinery_route.get("machine_block_id", "")) == "refinery-local"
		and String(refinery_route.get("inventory_id", "")) == "cargo-production",
		"refining selects an owned completed refinery and same-grid owned cargo"
	)
	_check(
		String(assembler_route.get("machine_block_id", "")) == "assembler-local",
		"component production selects the owned completed assembler"
	)
	_check(
		(client.call("_owned_machine_candidates", "refinery") as Array).size() == 1,
		"foreign and unfinished production machines are filtered"
	)

	client.set("connected", true)
	client.set("in_flight_mutation", {"type": "test-blocker", "operation_sequence": 1})
	client.set("in_flight_mutation_actor_id", "impairment-player")
	client.call("_queue_physical_production", "refining")
	var queued: Array = client.get("mutation_queue")
	var command: Dictionary = queued.back() if not queued.is_empty() else {}
	_check(
		String(command.get("type", "")) == "queue_production"
		and String(command.get("machine_block_id", "")) == "refinery-local"
		and String(command.get("recipe", "")) == "refining"
		and int(command.get("batches", 0)) == 1
		and String(command.get("source_inventory_id", "")) == "cargo-production"
		and String(command.get("destination_inventory_id", "")) == "cargo-production"
		and not command.has("operation_sequence"),
		"physical production command has the exact typed shape before pipeline sequencing"
	)
	_check(
		String(command.get("source_inventory_id", "")) != "inventory-impairment-player",
		"physical production never routes through the suit inventory"
	)

	var machine_label := Label.new()
	var queue_label := Label.new()
	var route_label := Label.new()
	var inventory_root := Control.new()
	var production_root := Control.new()
	var refine_button := Button.new()
	var component_button := Button.new()
	client.add_child(machine_label)
	client.add_child(queue_label)
	client.add_child(route_label)
	client.add_child(inventory_root)
	client.add_child(production_root)
	client.add_child(refine_button)
	client.add_child(component_button)
	client.set("production_machine_label", machine_label)
	client.set("production_queue_label", queue_label)
	client.set("production_route_label", route_label)
	client.set("inventory_content_root", inventory_root)
	client.set("production_content_root", production_root)
	client.set("production_buttons", {
		"refining": refine_button, "component": component_button,
	})
	client.call("_update_production_terminal")
	_check(
		queue_label.text.contains("25%")
		and queue_label.text.contains("OUTPUT BLOCKED")
		and not queue_label.text.contains("×9"),
		"production presentation is authoritative, includes blocked progress, and hides foreign jobs"
	)
	_check(machine_label.text.contains("POWER ONLINE"), "machine presentation exposes power state")
	client.call("_set_inventory_tab", "production")
	_check(
		String(client.get("active_inventory_tab")) == "production"
		and production_root.visible and not inventory_root.visible,
		"production tab switches without replacing the inventory pane"
	)
	client.call("_set_inventory_tab", "inventory")
	_check(inventory_root.visible and not production_root.visible, "inventory tab restores two-pane UX")

	private_state["production_queues"] = {"malformed": true}
	client.set("actor_private_snapshot", private_state)
	client.call("_update_production_terminal")
	_check(queue_label.text.contains("NO CANONICAL JOBS"), "malformed queue state fails safely")
	client.set("snapshot", {"grids": [foreign_grid]})
	client.call("_queue_physical_production", "component")
	_check(
		(client.get("mutation_queue") as Array).size() == 1
		and String(client.get("recent_message")).contains("NO AUTHORIZED ASSEMBLER ROUTE"),
		"missing authorized route is visible and queues no mutation"
	)
	client.free()


func _test_mutation_frontier_reconciliation() -> void:
	var client := _new_client()
	client.set("committed_operation_sequence", 4)
	client.set("committed_operation_actor_id", "impairment-player")
	client.set("operation_frontier_observed", true)
	client.set("operation_frontier_ready", true)
	client.set("mutation_queue_actor_id", "impairment-player")
	client.set("in_flight_mutation_actor_id", "impairment-player")
	var exact_message := {
		"type": "mine_voxel",
		"operation_sequence": 5,
		"operation_id": "frontier-five",
		"coordinate": {"x": 1, "y": 2, "z": 3},
	}
	var exact_text := JSON.stringify(exact_message)
	client.set("in_flight_mutation", exact_message.duplicate(true))
	client.set("in_flight_mutation_text", exact_text)
	client.set("mutation_resync_required", true)
	_check(
		bool(client.call("_reconcile_operation_frontier", 4)),
		"uncommitted reconnect frontier permits exact retry"
	)
	_check(
		String(client.get("in_flight_mutation_text")) == exact_text
		and int(client.get("committed_operation_sequence")) == 4
		and not bool(client.get("mutation_resync_required")),
		"reconnect retains byte-exact pending payload and reusable sequence"
	)

	_check(
		bool(client.call("_reconcile_operation_frontier", 5)),
		"committed reconnect frontier requests exact receipt recovery"
	)
	_check(
		not (client.get("in_flight_mutation") as Dictionary).is_empty()
		and String(client.get("in_flight_mutation_text")) == exact_text
		and int(client.get("committed_operation_sequence")) == 4
		and int(client.get("observed_operation_frontier")) == 5,
		"frontier completion retains exact bytes until authority confirms identity"
	)
	var decoded_receipt: Variant = JSON.parse_string(JSON.stringify({
		"operation_sequence": 5,
		"operation_id": "frontier-five",
		"code": "voxel_mined",
	}))
	_check(
		decoded_receipt is Dictionary
		and bool(client.call("_handle_intent_accepted", decoded_receipt)),
		"JSON-decoded accepted receipt confirms the retained command"
	)
	_check(
		(client.get("in_flight_mutation") as Dictionary).is_empty()
		and int(client.get("committed_operation_sequence")) == 5,
		"exact receipt advances and clears only the confirmed command"
	)
	client.call("_clear_actor_private_state")
	_check(
		not bool(client.call("_reconcile_operation_frontier", 4))
		and bool(client.get("mutation_resync_required")),
		"private overlay clearing cannot make an observed frontier regress"
	)
	client.set("mutation_resync_required", false)
	client.set("operation_frontier_ready", true)

	client.set("mutation_queue_actor_id", "impairment-player")
	client.set("in_flight_mutation_actor_id", "impairment-player")
	client.set("in_flight_mutation", {
		"type": "craft_component",
		"operation_sequence": 6,
		"operation_id": "rejected-six",
	})
	client.set("in_flight_mutation_text", "exact-rejected-six")
	var decoded_rejection: Variant = JSON.parse_string(JSON.stringify({
		"type": "intent_rejected",
		"operation_sequence": 6,
		"operation_id": "rejected-six",
		"code": "insufficient_refined_material",
		"message": "not enough material",
	}))
	client.call("_handle_intent_rejected", decoded_rejection)
	_check(
		(client.get("in_flight_mutation") as Dictionary).is_empty()
		and int(client.get("committed_operation_sequence")) == 5,
		"JSON-decoded gameplay rejection leaves the operation sequence reusable"
	)

	client.set("in_flight_mutation_actor_id", "impairment-player")
	client.set("in_flight_mutation", {
		"type": "set_player_control",
		"operation_sequence": 6,
		"operation_id": "rejected-control-six",
		"input_sequence": 7,
	})
	client.set("in_flight_mutation_text", "exact-rejected-control-six")
	(client.get("mutation_queue") as Array).append({
		"type": "craft_component",
		"operation_id": "deferred-after-control",
	})
	client.set("authoritative_player_ready", true)
	client.call("_handle_intent_rejected", {
		"type": "intent_rejected",
		"operation_sequence": 6,
		"operation_id": "rejected-control-six",
		"code": "input_sequence_stale",
		"message": "control input is stale",
	})
	_check(
		not bool(client.get("authoritative_player_ready"))
		and (client.get("in_flight_mutation") as Dictionary).is_empty()
		and (client.get("mutation_queue") as Array).size() == 1,
		"rejected control defers later commands until a fresh prediction snapshot"
	)

	client.set("in_flight_mutation_actor_id", "impairment-player")
	client.set("in_flight_mutation", {
		"type": "craft_component",
		"operation_sequence": 6,
		"operation_id": "conflicted-six",
	})
	client.set("in_flight_mutation_text", "exact-conflicted-six")
	(client.get("mutation_queue") as Array).clear()
	client.call("_handle_intent_rejected", {
		"type": "intent_rejected",
		"operation_sequence": 6,
		"operation_id": "conflicted-six",
		"code": "operation_conflict",
		"message": "sequence already bound",
	})
	_check(
		bool(client.get("mutation_resync_required"))
		and not bool(client.get("operation_frontier_ready"))
		and not (client.get("in_flight_mutation") as Dictionary).is_empty(),
		"operation conflict pauses authority while preserving exact pending payload"
	)

	client.set("mutation_resync_required", false)
	client.set("operation_frontier_ready", true)
	client.set("authoritative_player_ready", true)
	client.set("connected", true)
	client.set("in_flight_mutation", {
		"type": "mine_voxel",
		"operation_sequence": 6,
		"operation_id": "blocking-six",
	})
	client.set("in_flight_mutation_actor_id", "impairment-player")
	var first_control := {
		"type": "set_player_control",
		"operation_id": "queued-control-one",
		"movement_epoch": 1,
		"input_sequence": 7,
	}
	var latest_control := first_control.duplicate(true)
	latest_control["operation_id"] = "queued-control-two"
	latest_control["input_sequence"] = 8
	_check(bool(client.call("_queue_mutation", first_control)), "first queued control accepted")
	_check(bool(client.call("_queue_mutation", latest_control)), "latest queued control coalesced")
	var queued: Array = client.get("mutation_queue")
	_check(
		queued.size() == 1
		and int((queued[0] as Dictionary).get("input_sequence", 0)) == 8
		and not (queued[0] as Dictionary).has("operation_sequence"),
		"queued controls stay bounded and receive no sequence before dispatch"
	)
	for index in range(1, 32):
		_check(bool(client.call("_queue_mutation", {
			"type": "craft_component",
			"operation_id": "bounded-command-%d" % index,
		})), "bounded command %d queued" % index)
	_check(
		not bool(client.call("_queue_mutation", {
			"type": "craft_component",
			"operation_id": "overflow-command",
		}))
		and (client.get("mutation_queue") as Array).size() == 32,
		"mutation queue rejects work beyond its deterministic bound"
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
		"projection_schema_version": 4,
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
	camera.position = Vector3(0.5, 0.72, 0)
	var before_gap_camera := camera.position
	correction_client.call("_apply_authoritative_player", epoch_player, 5, 5, "gap", "motion_state")
	_check(camera.position.is_equal_approx(before_gap_camera), "bounded history reset preserves rendered camera pose")
	_check((correction_client.get("prediction_history") as Array).is_empty(), "history gap hard reset")
	_check((correction_client.get("pending_controls") as Array).is_empty(), "history gap cleared controls")

	(correction_client.get("prediction_history") as Array).append({"simulation_tick": 5})
	(correction_client.get("pending_controls") as Array).append({"input_sequence": 4})
	correction_client.call("_apply_authoritative_player", epoch_player, 6, 6, "reconnect", "reconnect")
	_check((correction_client.get("prediction_history") as Array).is_empty(), "reconnect cleared history")
	_check((correction_client.get("pending_controls") as Array).is_empty(), "reconnect cleared controls")
	correction_client.free()


func _test_render_interpolation_and_grounded_pitch_prediction() -> void:
	var client := _new_client()
	client.set("previous_predicted_position", Vector3.ZERO)
	client.set("predicted_position", Vector3(2.0, 0.0, 0.0))
	client.set("previous_predicted_orientation", Quaternion.IDENTITY)
	client.set("predicted_orientation", Quaternion(Vector3.UP, PI * 0.5))
	client.set("previous_predicted_view_pitch_radians", 0.0)
	client.set("predicted_view_pitch_radians", 0.4)
	client.set("prediction_presentation_ready", true)
	_check(
		(client.call("_interpolated_prediction_position", 0.5) as Vector3).is_equal_approx(
			Vector3(1.0, 0.0, 0.0)
		),
		"render position interpolates fixed steps"
	)
	var halfway_orientation: Quaternion = client.call(
		"_interpolated_prediction_orientation", 0.5
	)
	_check(
		absf(halfway_orientation.get_angle() - PI * 0.25) < 0.0001,
		"render orientation interpolates fixed steps"
	)
	_check(
		is_equal_approx(float(client.call("_interpolated_prediction_view_pitch", 0.5)), 0.2),
		"grounded view pitch interpolates fixed steps"
	)

	var replay_client := _new_client()
	var forward_control := {
		"linear_input": Vector3(0.0, 0.0, -1.0),
		"angular_input": Vector3.ZERO,
		"boost": false,
		"jump": false,
		"dampeners": true,
	}
	replay_client.set("current_prediction_input_sequence", 1)
	replay_client.call("_capture_prediction_presentation_step")
	replay_client.call("_predict_player_step", forward_control, FIXED_DELTA, true)
	var first_replayed_position: Vector3 = replay_client.get("predicted_position")
	replay_client.call("_capture_prediction_presentation_step")
	replay_client.call("_predict_player_step", forward_control, FIXED_DELTA, true)
	var second_replayed_position: Vector3 = replay_client.get("predicted_position")
	var replay_fraction := clampf(Engine.get_physics_interpolation_fraction(), 0.0, 1.0)
	var rendered_position := first_replayed_position.lerp(
		second_replayed_position, replay_fraction
	)
	var replay_camera: Camera3D = replay_client.get("camera")
	var eye_offset: Vector3 = replay_client.call(
		"_prediction_camera_eye_offset", Quaternion.IDENTITY
	)
	replay_camera.position = rendered_position + eye_offset
	var replay_authority := _player_from_motion({}, 1, 0)
	replay_client.call(
		"_apply_authoritative_player", replay_authority, 0, 1, "irregular", "motion_state"
	)
	_check(
		(replay_client.get("previous_predicted_position") as Vector3).is_equal_approx(
			first_replayed_position
		)
		and (replay_client.get("predicted_position") as Vector3).is_equal_approx(
			second_replayed_position
		),
		"irregular authority preserves the final replay interpolation span",
	)
	_check(
		(replay_client.get("presentation_position_offset") as Vector3).length() < 0.0001,
		"ordinary fixed-step interpolation is not accumulated as reconciliation error",
	)
	replay_client.free()

	var grounded := _base_player()
	grounded["jetpack_enabled"] = false
	grounded["locomotion"] = {
		"kind": "grounded",
		"up": _protocol_vec3(Vector3.UP),
		"view_pitch_radians": 0.0,
		"jump_held": false,
	}
	client.set("actor_private_snapshot", _private_snapshot(grounded))
	client.set("predicted_view_pitch_radians", 0.0)
	client.call("_predict_player_step", {
		"linear_input": Vector3.ZERO,
		"angular_input": Vector3(1.0, 0.0, 0.0),
		"boost": false,
		"jump": false,
		"dampeners": true,
	}, FIXED_DELTA, false)
	_check(
		is_equal_approx(
			float(client.get("predicted_view_pitch_radians")), 2.5 * FIXED_DELTA
		),
		"grounded pitch predicts before authoritative motion arrives"
	)

	var spherical_player := grounded.duplicate(true)
	spherical_player["position"] = _protocol_vec3(Vector3(0.0, 1200.901, 0.0))
	spherical_player["locomotion"] = {
		"kind": "grounded",
		"up": _protocol_vec3(Vector3.UP),
		"view_pitch_radians": 0.0,
		"jump_held": false,
		"support": {"body_id": "planet-body-khepri-prime"},
	}
	client.set("actor_private_snapshot", _private_snapshot(spherical_player))
	client.set("snapshot", {
		"environment": {
			"celestial_body_id": "planet-body-khepri-prime",
			"planet_center": _protocol_vec3(Vector3.ZERO),
			"surface_radius_m": 1200.0,
			"gravity": _protocol_vec3(Vector3(0.0, -9.81, 0.0)),
		},
		"players": [_public_player(spherical_player)],
		"grids": [],
		"voxels": [],
	})
	client.set("predicted_position", Vector3(0.0, 1200.901, 0.0))
	client.set("predicted_orientation", Quaternion.IDENTITY)
	client.set("predicted_linear_velocity", Vector3.ZERO)
	client.set("predicted_angular_velocity", Vector3.ZERO)
	client.set("prediction_planet_center", Vector3.ZERO)
	client.set("prediction_surface_radius", 1200.0)
	client.set("prediction_gravitational_parameter", 9.81 * 1200.901 * 1200.901)
	client.set("prediction_gravity_model_ready", true)
	var starting_radius := (client.get("predicted_position") as Vector3).length()
	for _step in 600:
		client.call("_predict_player_step", {
			"linear_input": Vector3(0.0, 0.0, -1.0),
			"angular_input": Vector3.ZERO,
			"boost": false,
			"jump": false,
			"dampeners": true,
		}, FIXED_DELTA, false)
	var curved_position: Vector3 = client.get("predicted_position")
	_check(
		absf(curved_position.length() - starting_radius) < 0.001,
		"grounded planet prediction follows the curved support radius",
	)
	_check(
		absf(curved_position.z) > 30.0,
		"grounded planet prediction still advances tangentially",
	)

	var micro_position_blend := float(client.call(
		"_presentation_position_correction_blend", Vector3(0.01, 0.0, 0.0), 0.016
	))
	var material_position_blend := float(client.call(
		"_presentation_position_correction_blend", Vector3(0.5, 0.0, 0.0), 0.016
	))
	_check(
		micro_position_blend < material_position_blend,
		"sub-visual position reconciliation decays more gently",
	)
	var micro_orientation_blend := float(client.call(
		"_presentation_orientation_correction_blend",
		Quaternion(Vector3.UP, 0.002),
		0.016,
	))
	var material_orientation_blend := float(client.call(
		"_presentation_orientation_correction_blend",
		Quaternion(Vector3.UP, 0.2),
		0.016,
	))
	_check(
		micro_orientation_blend < material_orientation_blend,
		"sub-visual orientation reconciliation decays more gently",
	)
	var camera: Camera3D = client.get("camera")
	camera.fov = 74.0
	client.set("predicted_linear_velocity", Vector3(0.0, 0.0, -7.5))
	client.call("_update_player_presentation", FIXED_DELTA)
	_check(
		is_equal_approx(camera.fov, 74.0),
		"ordinary grounded sprint does not pulse the field of view",
	)
	client.set("predicted_linear_velocity", Vector3(0.0, 0.0, -24.0))
	client.call("_update_player_presentation", FIXED_DELTA)
	_check(camera.fov > 74.0, "true boost retains readable field-of-view feedback")
	client.free()


func _test_grounded_prediction_honors_grid_obstacles() -> void:
	var client := _new_client()
	var start := Vector3(0.0, 1200.901, 0.0)
	var grounded := _base_player()
	grounded["jetpack_enabled"] = false
	grounded["position"] = _protocol_vec3(start)
	grounded["surface_contact"] = true
	grounded["locomotion"] = {
		"kind": "grounded",
		"up": _protocol_vec3(Vector3.UP),
		"view_pitch_radians": 0.0,
		"jump_held": false,
		"support": {"body_id": "planet-body-khepri-prime"},
	}
	client.set("actor_private_snapshot", _private_snapshot(grounded))
	client.set("snapshot", {
		"environment": {
			"celestial_body_id": "planet-body-khepri-prime",
			"planet_center": _protocol_vec3(Vector3.ZERO),
			"surface_radius_m": 1200.0,
			"gravity": _protocol_vec3(Vector3(0.0, -9.81, 0.0)),
		},
		"players": [_public_player(grounded)],
		"grids": [],
		"voxels": [],
	})
	client.set("grid_lookup", {
		"grounded-wall": _target_grid(
			"grounded-wall",
			start + Vector3(0.0, 0.0, -0.9),
			Quaternion.IDENTITY,
			[_target_block("grounded-wall-block", Vector3i.ZERO)],
		),
	})
	client.set("predicted_position", start)
	client.set("predicted_orientation", Quaternion.IDENTITY)
	client.set("predicted_linear_velocity", Vector3.ZERO)
	client.set("predicted_angular_velocity", Vector3.ZERO)
	client.set("prediction_planet_center", Vector3.ZERO)
	client.set("prediction_surface_radius", 1200.0)
	client.set("prediction_gravitational_parameter", 9.81 * 1200.901 * 1200.901)
	client.set("prediction_gravity_model_ready", true)
	var forward_control := {
		"linear_input": Vector3(0.0, 0.0, -1.0),
		"angular_input": Vector3.ZERO,
		"boost": false,
		"jump": false,
		"dampeners": true,
	}
	for _step in 120:
		client.call("_predict_player_step", forward_control, FIXED_DELTA, false)
	var blocked_position: Vector3 = client.get("predicted_position")
	_check(
		blocked_position.z > -0.2,
		"grounded prediction stops at a grid obstacle instead of reconciling through it",
	)
	_check(
		bool(client.get("predicted_surface_contact")),
		"grounded obstacle prediction preserves support contact",
	)
	client.free()


func _test_resting_floor_contact_allows_tangential_walk() -> void:
	var client := _new_client()
	client.set("snapshot", {"environment": {}})
	var floor_blocks: Array = []
	for x in range(-2, 4):
		floor_blocks.append(_target_block("floor-%d" % x, Vector3i(x, 0, 0)))
	client.set("grid_lookup", {"floor": _target_grid("floor", Vector3.ZERO, Quaternion.IDENTITY, floor_blocks)})
	# Five millimetres of resting solver penetration must not freeze each step.
	var start := Vector3(0, 1.395, 0)
	var finish := start + Vector3(1, 0, 0)
	var sweep: Dictionary = client.call("_sweep_player_position", start, finish)
	_check(Vector3(sweep.get("position")).is_equal_approx(finish), "resting contact permits continuous tangential walking")
	_check(not client.call("_player_position_is_clear", Vector3(0, 1.37, 0)), "contact skin still rejects deep floor penetration")
	var player := _base_player()
	player["jetpack_enabled"] = false
	player["locomotion"] = {"kind": "grounded", "up": _protocol_vec3(Vector3(0.02, 1, 0).normalized()), "support": {"body_id": "floor", "local_normal": _protocol_vec3(Vector3.UP)}}
	client.set("actor_private_snapshot", _private_snapshot(player))
	client.set("predicted_position", start)
	client.set("predicted_orientation", Quaternion.IDENTITY)
	client.set("predicted_linear_velocity", Vector3.ZERO)
	for _step in 60:
		client.call("_predict_player_step", {"linear_input": Vector3.RIGHT, "angular_input": Vector3.ZERO}, FIXED_DELTA, false)
	var walked: Vector3 = client.get("predicted_position")
	_check(walked.x > 3.0 and absf(walked.y - start.y) < 0.001, "gravity tilt does not drive prediction into flat capital floor")
	client.free()


func _test_remote_player_presentation_smoothing() -> void:
	var client := _new_client()
	var players_root := Node3D.new()
	client.add_child(players_root)
	client.set("players_root", players_root)
	var remote_node := Node3D.new()
	var pilot_label := Node3D.new()
	pilot_label.name = "PilotLabel"
	remote_node.add_child(pilot_label)
	players_root.add_child(remote_node)
	client.set("remote_player_nodes", {"player-remote": remote_node})
	var remote := _base_player()
	remote["player_id"] = "player-remote"
	remote["position"] = _protocol_vec3(Vector3.ZERO)
	remote["orientation"] = _protocol_quat(Quaternion.IDENTITY)
	client.call("_sync_remote_players", [remote])
	var node: Node3D = (client.get("remote_player_nodes") as Dictionary).get(
		"player-remote", null
	)
	_check(node != null, "remote presentation creates an avatar")
	if node == null:
		client.free()
		return

	var target_orientation := Quaternion(Vector3.UP, 0.5)
	remote["position"] = _protocol_vec3(Vector3(1.0, 0.0, 0.0))
	remote["orientation"] = _protocol_quat(target_orientation)
	client.call("_sync_remote_players", [remote])
	_check(node.position.is_zero_approx(), "ordinary remote motion does not snap")
	_check(
		node.quaternion.is_equal_approx(Quaternion.IDENTITY),
		"ordinary remote rotation does not snap"
	)
	_check(
		bool(client.call("_remote_player_visuals_match", [remote])),
		"remote presentation tracks the newest authoritative target"
	)

	client.call("_update_remote_player_presentation", FIXED_DELTA)
	_check(
		node.position.x > 0.0 and node.position.x < 1.0,
		"remote position advances smoothly between authoritative samples"
	)
	_check(
		node.quaternion.get_angle() > 0.0 and node.quaternion.get_angle() < 0.5,
		"remote orientation advances smoothly between authoritative samples"
	)
	for _step in 120:
		client.call("_update_remote_player_presentation", FIXED_DELTA)
	_check(
		node.position.is_equal_approx(Vector3(1.0, 0.0, 0.0)),
		"remote position converges to authority"
	)
	_check(
		node.quaternion.is_equal_approx(target_orientation),
		"remote orientation converges to authority"
	)

	remote["position"] = _protocol_vec3(Vector3(10.0, 0.0, 0.0))
	client.call("_sync_remote_players", [remote])
	_check(
		node.position.is_equal_approx(Vector3(10.0, 0.0, 0.0)),
		"remote teleport snaps without crossing intervening geometry"
	)
	client.free()


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


func _test_starter_tool_kit() -> void:
	var client := _new_client(true, HeadlessToolClient)
	var progress := ProgressBar.new()
	var mode := Label.new()
	client.add_child(progress)
	client.add_child(mode)
	client.set("action_progress", progress)
	client.set("mode_label", mode)
	client.set("connected", true)
	client.set("test_capture_transport", true)
	client.set("handoff_phase", "live")
	Input.mouse_mode = Input.MOUSE_MODE_CAPTURED
	var inventory_before: Dictionary = client.get("actor_private_snapshot").duplicate(true)
	var block := _target_block("kit-block", Vector3i.ZERO)
	block["health"] = 50
	block["max_health"] = 100
	var grid := _target_grid("kit-grid", Vector3.ZERO, Quaternion.IDENTITY, [block], "impairment-player")
	var target := {"grid_id": "kit-grid", "grid": grid, "block": block, "local_normal": Vector3.RIGHT}
	client.set("target_block", target)
	client.set("target_voxel", null)
	for pair in [["drill", ""], ["grinder", "damage"], ["welder", "weld"], ["pulse", ""]]:
		client.call("_equip_tool", pair[0])
		var plan: Dictionary = client.call("_tool_action_plan")
		_check(plan.get("kind", "") == pair[1], "tool block action is isolated: %s" % pair[0])
		_check(client.get("primary_needs_release"), "tool switch requires primary release")
	client.set("target_block", {})
	client.set("target_voxel", Vector3i.ZERO)
	for id in ["drill", "grinder", "welder", "pulse"]:
		client.call("_equip_tool", id)
		var plan: Dictionary = client.call("_tool_action_plan")
		_check(plan.get("kind", "") == ("mine" if id == "drill" else ""), "only drill mines: %s" % id)
	client.set("target_voxel", null)
	client.set("target_block", target)
	client.call("_equip_tool", "welder")
	block["health"] = 100
	target["block"] = block
	client.set("target_block", target)
	_check(client.call("_tool_action_plan").is_empty(), "welder on completed block cannot implicitly build")
	client.set("build_mode", true)
	_check(client.call("_tool_action_plan").get("kind", "") == "build", "explicit construction allows frame placement")
	client.call("_select_number_slot", 6)
	_check(client.get("selected_block_kind") == "refinery", "construction number selects a machine")
	_check(client.get("equipped_tool") == "welder", "construction retains welder")
	client.set("build_mode", false)
	client.call("_select_number_slot", 1)
	_check(client.get("equipped_tool") == "grinder", "normal number selects tool")
	client.set("action_charge", 0.8)
	client.call("_equip_tool", "pulse")
	_check(is_zero_approx(client.get("action_charge")), "switch cannot carry partial grinder charge into a shot")
	_check(not client.call("_fire_pulse"), "equipping while held cannot fire")
	client.call("_advance_tool_action", 0.01, false)
	_check(client.call("_fire_pulse"), "fresh pulse press fires")
	var sent: Dictionary = client.get("in_flight_mutation")
	_check(sent.get("type", "") == "damage_block" and sent.get("block_id", "") == "kit-block", "pulse uses server block-damage intent")
	_check(not sent.has("damage") and not sent.has("ray") and not sent.has("ammo"), "pulse never authors damage, ray or ammo")
	_check(String(sent.get("operation_id", "")).begins_with("pulse-"), "pulse retains diagnostic operation identity")
	_check(not client.call("_fire_pulse"), "held pulse does not repeat")
	client.call("_advance_tool_action", 2.0, true)
	_check(client.get("mutation_queue").is_empty(), "held pulse never queues automatic shots")
	client.call("_advance_tool_action", 0.01, false)
	_check(not client.call("_fire_pulse"), "pending receipt blocks another shot")
	client.set("in_flight_mutation", {})
	client.set("action_cooldown", 0.0)
	client.call("_advance_tool_action", 0.01, false)
	client.set("inventory_open", true)
	_check(not client.call("_fire_pulse"), "inventory prevents discharge")
	client.call("_advance_tool_action", 0.01, true)
	client.set("inventory_open", false)
	_check(not client.call("_fire_pulse"), "held menu click cannot discharge after closing")
	client.call("_advance_tool_action", 0.01, false)
	for flag in ["connected", "authoritative_player_ready", "operation_frontier_ready"]:
		client.set(flag, false)
		_check(not client.call("_fire_pulse"), "unavailable authority prevents discharge: %s" % flag)
		client.set(flag, true)
	client.set("handoff_phase", "importing")
	_check(not client.call("_fire_pulse"), "handoff prevents discharge")
	client.set("handoff_phase", "live")
	client.set("target_block", {})
	client.set("target_hit", {})
	_check(client.call("_fire_pulse"), "empty-space pulse is cosmetic")
	_check(client.get("in_flight_mutation").is_empty(), "empty-space pulse cannot mutate world")
	_check(client.get("actor_private_snapshot") == inventory_before, "tool selection and shots do not invent inventory")
	client.call("_equip_tool", "grinder")
	client.set("target_block", target)
	client.set("action_cooldown", 0.0)
	client.call("_advance_tool_action", 0.01, false)
	client.call("_advance_tool_action", 0.2, true)
	_check(float(client.get("action_charge")) > 0.0, "grinder charges while held")
	client.set("connected", false)
	client.call("_advance_tool_action", 1.0, true)
	_check(is_zero_approx(client.get("action_charge")), "disconnect cancels charged tool")
	_check(client.get("in_flight_mutation").is_empty(), "disconnect never completes tool work")
	_check(String(client.call("_mission_text", {"voxels_mined": 3})).contains("transfer at least 2 ore"), "guide teaches cargo before refining")
	_check(String(client.call("_mission_text", {"voxels_mined": 3, "refining_batches": 1})).contains("alloy in industrial cargo"), "guide teaches machine component production")
	client.free()
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
