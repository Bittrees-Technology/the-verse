# SPDX-License-Identifier: AGPL-3.0-or-later
extends Node

var client: Node3D

func _ready() -> void:
	call_deferred("_run")

func _wait_ready(seconds: float) -> bool:
	var deadline := Time.get_ticks_msec() + int(seconds * 1000.0)
	while Time.get_ticks_msec() < deadline:
		var entry: CanvasLayer = client.get("session_entry")
		if client.get("authoritative_player_ready") and client.get("connected") and entry.worker_pid > 0 and not entry.enter_button.disabled:
			return true
		await get_tree().create_timer(0.1).timeout
	return false

func _run() -> void:
	client = get_parent()
	print("VERSE_OWNED_WORKER_START")
	if not await _wait_ready(45):
		printerr("VERSE_OWNED_WORKER_FAILED initial app launch")
		get_tree().quit(1)
		return
	var entry: CanvasLayer = client.get("session_entry")
	var first_pid: int = entry.worker_pid
	entry.enter_world()
	await get_tree().create_timer(2.0).timeout
	var sequence := int(client.get("snapshot").get("event_sequence", 0))
	# Reproduce laptop suspension: let the real writer lease expire, then resume.
	OS.execute("/bin/kill", PackedStringArray(["-STOP", str(first_pid)]))
	await get_tree().create_timer(18.0).timeout
	var blocked: bool = not entry.entered and entry.enter_button.disabled
	OS.execute("/bin/kill", PackedStringArray(["-CONT", str(first_pid)]))
	var deadline := Time.get_ticks_msec() + 45_000
	while entry.worker_pid == first_pid and Time.get_ticks_msec() < deadline:
		await get_tree().create_timer(0.1).timeout
	if not blocked or not await _wait_ready(45) or entry.worker_pid == first_pid:
		printerr("VERSE_OWNED_WORKER_FAILED lease recovery blocked=%s pid=%d" % [blocked, entry.worker_pid])
		get_tree().quit(1)
		return
	if int(client.get("snapshot").get("event_sequence", 0)) < sequence:
		printerr("VERSE_OWNED_WORKER_FAILED world history regressed")
		get_tree().quit(1)
		return
	print("VERSE_OWNED_WORKER_OK launch=app stale=blocked lease=expired restart=fresh_process save=retained")
	get_tree().quit(0)
