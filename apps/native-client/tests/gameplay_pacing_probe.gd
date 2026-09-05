# SPDX-License-Identifier: AGPL-3.0-or-later
extends SceneTree

class MeasuredClient:
	extends "res://src/main.gd"
	var timings: Dictionary = {}
	func note(label: String, started: int) -> void:
		timings[label] = float(timings.get(label, 0.0)) + float(Time.get_ticks_usec() - started) / 1000.0
	func _poll_socket() -> void:
		var started := Time.get_ticks_usec()
		super._poll_socket()
		note("network_ms", started)
	func _prepare_interest_delta(authoritative: Dictionary) -> Dictionary:
		var started := Time.get_ticks_usec()
		var result = super._prepare_interest_delta(authoritative)
		note("prepare_ms", started)
		return result
	func _present_verified_interest_model(authoritative: Dictionary) -> bool:
		var started := Time.get_ticks_usec()
		var result = super._present_verified_interest_model(authoritative)
		note("present_ms", started)
		return result
	func _player_position_is_clear(position: Vector3) -> bool:
		var started := Time.get_ticks_usec()
		var result = super._player_position_is_clear(position)
		note("collision_ms", started)
		return result
	func _grid_topology_fingerprint(grid: Dictionary) -> String:
		var started := Time.get_ticks_usec()
		var result = super._grid_topology_fingerprint(grid)
		note("fingerprint_ms", started)
		return result
	func _physics_process(delta: float) -> void:
		var started := Time.get_ticks_usec()
		super._physics_process(delta)
		note("physics_ms", started)
	func _update_target() -> void:
		var started := Time.get_ticks_usec()
		super._update_target()
		note("target_ms", started)
	func _update_interface() -> void:
		var started := Time.get_ticks_usec()
		super._update_interface()
		note("hud_ms", started)
	func _update_player_presentation(delta: float) -> void:
		var started := Time.get_ticks_usec()
		super._update_player_presentation(delta)
		note("camera_ms", started)

func _initialize() -> void:
	call_deferred("_run")

func _run() -> void:
	var client := MeasuredClient.new()
	root.add_child(client)
	var deadline := Time.get_ticks_msec() + 30_000
	while not client.authoritative_player_ready and Time.get_ticks_msec() < deadline:
		await process_frame
	if not client.authoritative_player_ready:
		printerr("VERSE_PACING_FAILED no baseline")
		quit(1)
		return
	client.session_entry.enter_world()
	await create_timer(1.0).timeout
	var phases := ["idle", "walking", "mouse"]
	if "--render-isolation" in OS.get_cmdline_user_args():
		phases.append_array(["no_planet", "no_shadows"])
	for phase in phases:
		if phase == "no_planet":
			client.planet_root.visible = false
		if phase == "no_shadows":
			for node in client.find_children("*", "Light3D", true, false):
				node.shadow_enabled = false
		client.timings.clear()
		var frames: Array[float] = []
		var previous := Time.get_ticks_usec()
		deadline = Time.get_ticks_msec() + 4000
		if phase == "walking":
			Input.action_press("move_right")
		while Time.get_ticks_msec() < deadline:
			await process_frame
			var now := Time.get_ticks_usec()
			var elapsed := float(now - previous) / 1000.0
			frames.append(elapsed)
			previous = now
			if phase == "mouse":
				var motion := InputEventMouseMotion.new()
				motion.relative = Vector2(elapsed * 0.12, 0)
				Input.parse_input_event(motion)
		Input.action_release("move_right")
		frames.sort()
		var metrics := client.timings.duplicate()
		metrics["connected"] = client.connected
		metrics["replication"] = client.replication_state
		metrics["authoritative_ready"] = client.authoritative_player_ready
		metrics["phase"] = phase
		metrics["frames"] = frames.size()
		metrics["frame_p50_ms"] = frames[frames.size() / 2]
		metrics["frame_p95_ms"] = frames[int(frames.size() * 0.95)]
		metrics["draw_calls"] = Performance.get_monitor(Performance.RENDER_TOTAL_DRAW_CALLS_IN_FRAME)
		metrics["nodes"] = Performance.get_monitor(Performance.OBJECT_NODE_COUNT)
		metrics["history"] = client.prediction_history.size()
		print("VERSE_PACING ", JSON.stringify(metrics))
		if not client.connected or not client.authoritative_player_ready or client.replication_state != "ready":
			printerr("VERSE_PACING_FAILED lost playable connection: ", client.session_entry.last_problem)
			client.queue_free()
			await process_frame
			quit(1)
			return
	client.queue_free()
	await process_frame
	quit(0)
