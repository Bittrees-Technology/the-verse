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
	if not failures.is_empty():
		for failure in failures:
			printerr("VERSE_NATIVE_IMPAIRMENT_FAILED %s" % failure)
		quit(1)
		return
	print(
		"VERSE_NATIVE_IMPAIRMENT_OK queued_ack=ordered motion=monotonic corrections=bounded menu=neutral_prediction lifecycle=reset buffers=bounded rebuild=none"
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
		"event_sequence": 0,
		"simulation_tick": 0,
		"world_hash": "impairment-0",
		"player": player,
		"players": [player],
		"environment": {
			"planet_center": _protocol_vec3(Vector3.ZERO),
			"surface_radius_m": 1200.0,
			"gravity": _protocol_vec3(Vector3.ZERO),
		},
		"voxels": [],
		"grids": [],
	})
	client.set("requested_player_id", "impairment-player")
	client.set("bound_player_id", "impairment-player")
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
		"event_sequence": event_sequence,
		"simulation_tick": simulation_tick,
		"world_hash": "impairment-%d" % event_sequence,
		"player": player,
		"players": [player],
		"grids": grids,
	}


func _test_bound_player_roster_selection() -> void:
	var client := _new_client()
	var primary := _base_player()
	primary["player_id"] = "player-local"
	primary["inventory_id"] = "inventory-player-local"
	var remote := _base_player()
	remote["player_id"] = "player-remote"
	remote["inventory_id"] = "inventory-player-remote"
	remote["position"] = _protocol_vec3(Vector3(4.0, 0.0, 0.0))
	client.set("bound_player_id", "player-remote")
	var roster_snapshot: Dictionary = client.get("snapshot")
	roster_snapshot["player"] = primary
	roster_snapshot["players"] = [primary, remote]
	client.set("snapshot", roster_snapshot)
	var selected: Dictionary = client.call("_local_player")
	_check(String(selected.get("player_id", "")) == "player-remote", "bound actor selected")
	_check(
		String(client.call("_local_inventory_id")) == "inventory-player-remote",
		"bound actor inventory selected"
	)
	remote["position"] = _protocol_vec3(Vector3(9.0, 1.0, -2.0))
	client.call("_apply_motion_state", {
		"event_sequence": 1,
		"simulation_tick": 1,
		"world_hash": "impairment-roster-1",
		"player": primary,
		"players": [primary, remote],
		"grids": [],
	})
	var merged: Dictionary = client.call("_local_player")
	_check(String(merged.get("player_id", "")) == "player-remote", "motion keeps bound actor")
	_check(
		client.call("_vec3", merged.get("position", {})).is_equal_approx(Vector3(9.0, 1.0, -2.0)),
		"bound actor motion merged"
	)


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
	var dead_snapshot: Dictionary = (client.get("snapshot") as Dictionary).duplicate(true)
	var dead_player: Dictionary = dead_snapshot.get("player", {}).duplicate(true)
	dead_player["life_state"] = {"kind": "incapacitated"}
	dead_snapshot["player"] = dead_player
	dead_snapshot["players"] = [dead_player]
	client.set("snapshot", dead_snapshot)
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


func _protocol_vec3(value: Vector3) -> Dictionary:
	return {"x": value.x, "y": value.y, "z": value.z}


func _protocol_quat(value: Quaternion) -> Dictionary:
	return {"x": value.x, "y": value.y, "z": value.z, "w": value.w}
