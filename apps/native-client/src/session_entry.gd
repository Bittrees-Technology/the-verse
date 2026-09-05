# SPDX-License-Identifier: AGPL-3.0-or-later
extends CanvasLayer

var entered := false
var client: Node
var panel: Control
var heading: Label
var detail: Label
var enter_button: Button
var worker_pid := 0
var attempts := 0
var timer := 0.0
var explicit_server := false
var skip_entry := false
var probing := false
var probed := false
var external_server := false
var worker_script := ""
var data_directory := ""
var last_problem := ""


func _ready() -> void:
	client = get_parent()
	layer = 50
	for argument in OS.get_cmdline_user_args():
		explicit_server = explicit_server or argument.begins_with("--server=")
		skip_entry = skip_entry or argument in ["--smoke-test", "--skip-entry"]
	var configured_port := OS.get_environment("VERSE_LOCAL_PORT")
	if not explicit_server and configured_port.is_valid_int() and int(configured_port) >= 1024 and int(configured_port) <= 65535:
		client.set("server_url", "ws://127.0.0.1:%d/ws" % int(configured_port))
	var executable_directory := OS.get_executable_path().get_base_dir()
	var runtime_directory := executable_directory.path_join("../Resources/verse-runtime").simplify_path()
	if OS.get_name() != "macOS":
		runtime_directory = executable_directory.path_join("verse-runtime")
	worker_script = runtime_directory.path_join("start-owned-worker.sh")
	if OS.get_name() == "macOS":
		data_directory = OS.get_environment("HOME").path_join("Library/Application Support/The Verse Capital Playtest/universe")
	else:
		data_directory = OS.get_environment("HOME").path_join(".local/share/the-verse-capital/universe")
	if not OS.get_environment("VERSE_DATA_DIR").is_empty():
		data_directory = OS.get_environment("VERSE_DATA_DIR")
	_build_panel()
	if skip_entry:
		entered = true
		panel.hide()


func _build_panel() -> void:
	panel = ColorRect.new()
	panel.color = Color(0.015, 0.025, 0.04, 0.96)
	panel.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	add_child(panel)
	var center := CenterContainer.new()
	center.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	panel.add_child(center)
	var column := VBoxContainer.new()
	column.custom_minimum_size = Vector2(610, 0)
	column.add_theme_constant_override("separation", 20)
	center.add_child(column)
	heading = Label.new()
	heading.add_theme_font_size_override("font_size", 34)
	column.add_child(heading)
	detail = Label.new()
	detail.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	detail.custom_minimum_size = Vector2(610, 130)
	detail.add_theme_font_size_override("font_size", 19)
	column.add_child(detail)
	enter_button = Button.new()
	enter_button.text = "Enter the Verse"
	enter_button.custom_minimum_size.y = 58
	enter_button.pressed.connect(enter_world)
	column.add_child(enter_button)
	var retry := Button.new()
	retry.text = "Retry connection"
	retry.pressed.connect(_retry)
	column.add_child(retry)
	var quit_button := Button.new()
	quit_button.text = "Quit"
	quit_button.pressed.connect(func(): get_tree().quit())
	column.add_child(quit_button)


func enter_world() -> void:
	if not client.get("authoritative_player_ready"):
		return
	entered = true
	panel.hide()
	client.call("_clear_transient_character_input")
	client.set("primary_needs_release", true)
	client.call("_set_message", "WORLD READY // WASD move · Mouse look · 1–4 tools · I inventory · Esc menu")
	Input.mouse_mode = Input.MOUSE_MODE_CAPTURED


func pause_entry() -> void:
	entered = false
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	client.call("_clear_transient_character_input")
	client.call("_cancel_tool_charge")


func _retry() -> void:
	attempts = 0
	last_problem = ""
	probed = false
	external_server = false
	client.call("_connect_to_server", true)


func _process(delta: float) -> void:
	timer += delta
	var ready := bool(client.get("authoritative_player_ready")) and bool(client.get("connected")) and Time.get_ticks_msec() - int(client.get("last_verified_packet_msec")) < 5000
	if not explicit_server and FileAccess.file_exists(worker_script):
		if worker_pid > 0 and not OS.is_process_running(worker_pid):
			worker_pid = 0
			last_problem = "The local server stopped. Recovering your saved world…"
			probed = false
			timer = 0.0
		if not ready and worker_pid == 0 and not external_server and not probing and not probed and timer > 2.0 and attempts < 3:
			_probe_server()
	if skip_entry:
		return
	if not ready:
		entered = false
	panel.visible = not entered
	if entered:
		return
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	enter_button.disabled = not ready
	heading.text = "THE VERSE // READY" if ready else "CONNECTING TO YOUR WORLD"
	if ready:
		detail.text = "Your world is ready. Enter to walk and look around.\n\nWASD — move    Mouse — look    Space — jump\n1–4 — tools    I — inventory / production    B — build\nEsc — return to this menu"
	elif attempts >= 3 and worker_pid == 0:
		heading.text = "LOCAL SERVER COULD NOT START"
		detail.text = "Your save has been kept. Retry the connection or check:\n%s/server.log\n\n%s" % [data_directory, last_problem]
	elif explicit_server or not FileAccess.file_exists(worker_script):
		detail.text = "Waiting for a playable world at %s.\n\nIf you opened an older client alone, use its workshop launcher.\n%s" % [client.get("server_url"), client.get("recent_message")]
	else:
		detail.text = "Starting the capital and loading your saved progress.\nYou can enter when the server has verified your player.\n\n%s" % last_problem


func _probe_server() -> void:
	probing = true
	probed = true
	var request := HTTPRequest.new()
	request.timeout = 3.0
	add_child(request)
	request.request_completed.connect(func(_result: int, code: int, _headers: PackedStringArray, _body: PackedByteArray):
		probing = false
		request.queue_free()
		if code != 0:
			external_server = true
			last_problem = "An existing server is responding. Waiting for its gameplay connection."
			return
		_start_worker()
	)
	if request.request(String(client.get("server_url")).replace("ws://", "http://").trim_suffix("/ws") + "/healthz") != OK:
		probing = false
		request.queue_free()
		last_problem = "Unable to check the local server."


func _start_worker() -> void:
	attempts += 1
	worker_pid = OS.create_process("/bin/bash", PackedStringArray([worker_script, data_directory]))
	if worker_pid <= 0:
		worker_pid = 0
		probed = false
		last_problem = "The bundled server process could not be started."
	client.call("_connect_to_server", true)
	timer = 0.0


func _exit_tree() -> void:
	if worker_pid > 0 and OS.is_process_running(worker_pid):
		# Signal only the worker this app created; let it drain its journal normally.
		OS.execute("/bin/kill", PackedStringArray(["-TERM", str(worker_pid)]))
