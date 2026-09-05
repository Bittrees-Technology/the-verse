# SPDX-License-Identifier: AGPL-3.0-or-later
extends SceneTree

var failures: Array[String] = []
var output_directory := "user://starter-kit-review"


func _initialize() -> void:
	call_deferred("_run")


func _check(condition: bool, label: String) -> void:
	if not condition:
		failures.append(label)


func _click(control: Control) -> void:
	var point := root.get_final_transform() * control.get_global_rect().get_center()
	var motion := InputEventMouseMotion.new()
	motion.position = point
	motion.global_position = point
	Input.parse_input_event(motion)
	await process_frame
	for pressed in [true, false]:
		var button := InputEventMouseButton.new()
		button.button_index = MOUSE_BUTTON_LEFT
		button.pressed = pressed
		button.position = point
		button.global_position = point
		Input.parse_input_event(button)
		await process_frame


func _save(name: String) -> void:
	await RenderingServer.frame_post_draw
	var picture := root.get_texture().get_image()
	_check(picture != null and not picture.is_empty(), "nonempty rendered %s" % name)
	if picture != null and not picture.is_empty():
		_check(picture.save_png(output_directory.path_join(name + ".png")) == OK, "save %s" % name)


func _run() -> void:
	if DisplayServer.get_name() == "headless":
		printerr("VERSE_STARTER_KIT_UI_FAILED requires a real display")
		quit(1)
		return
	for argument in OS.get_cmdline_user_args():
		if argument.begins_with("--output-directory="):
			output_directory = argument.trim_prefix("--output-directory=")
	DirAccess.make_dir_recursive_absolute(output_directory)
	var client: Node3D = load("res://main.tscn").instantiate()
	root.add_child(client)
	var deadline := Time.get_ticks_msec() + 20_000
	while not client.get("authoritative_player_ready") and Time.get_ticks_msec() < deadline:
		await process_frame
	_check(client.get("authoritative_player_ready"), "verified live player baseline")
	if not client.get("authoritative_player_ready"):
		printerr("VERSE_STARTER_KIT_UI_FAILED no verified baseline")
		quit(1)
		return
	client.call("_set_inventory_open", true)
	await process_frame
	await _click(client.get("inventory_tab_buttons")["tools"])
	_check(client.get("active_inventory_tab") == "tools", "Tools tab responds to real mouse input")
	_check(client.get("tools_content_root").visible, "equipment panel visible")
	for tool_id in ["drill", "grinder", "welder", "pulse"]:
		await _click(client.get("tool_equip_buttons")[tool_id])
		_check(client.get("equipped_tool") == tool_id, "Equip button selects %s" % tool_id)
		var visible_models := 0
		for model in client.get("tool_viewmodels").values():
			if model["root"].visible:
				visible_models += 1
		_check(visible_models == 1, "exactly one viewmodel after %s" % tool_id)
		_check(client.get("inventory_open"), "equipment click does not capture the pointer")
	await _save("suit-tools")
	await _click(client.get("inventory_tab_buttons")["production"])
	_check(client.get("active_inventory_tab") == "production", "Production tab responds to real mouse input")
	await _save("production")
	client.call("_set_inventory_open", false)
	for tool_id in ["drill", "grinder", "welder", "pulse"]:
		client.call("_equip_tool", tool_id)
		for frame in range(4):
			await process_frame
		await _save(tool_id)
	client.queue_free()
	await process_frame
	if not failures.is_empty():
		for failure in failures:
			printerr("VERSE_STARTER_KIT_UI_FAILED %s" % failure)
		quit(1)
		return
	print("VERSE_STARTER_KIT_UI_OK tabs=clickable tools=4 viewmodels=exclusive baseline=verified")
	quit(0)
