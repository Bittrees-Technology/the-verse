# SPDX-License-Identifier: AGPL-3.0-or-later
extends SceneTree

const CLIENT_SCRIPT: Script = preload("res://src/main.gd")

var failures: Array[String] = []


func _initialize() -> void:
	call_deferred("_run")


func _run() -> void:
	_check(ClassDB.class_exists("VerseInterestVerifier"), "native verifier class is registered")
	if failures.is_empty():
		_test_stage_commit_discard_reset()
	if failures.is_empty():
		_test_portable_player_vector()
	if failures.is_empty():
		_test_frozen_invalid_corpus()
	if not failures.is_empty():
		for failure in failures:
			printerr("VERSE_NATIVE_VERIFIER_FAILED %s" % failure)
		quit(1)
		return
	print(
		"VERSE_NATIVE_VERIFIER_OK raw=staged sanitized=exposed token=one_use "
		+ "tamper=no_ack reset=invalidates unsafe_integer=exact invalid_corpus=16"
	)
	quit(0)


func _test_stage_commit_discard_reset() -> void:
	var verifier: Object = ClassDB.instantiate("VerseInterestVerifier")
	_check(verifier != null, "native verifier can be instantiated")
	if verifier == null:
		return
	_check(bool(_reset(verifier).get("ok", false)), "player verifier resets with protocol tuple")

	var malformed: Dictionary = verifier.call(
		"stage_server_message", PackedByteArray([123, 34, 116])
	)
	_check(
		not bool(malformed.get("ok", true))
		and String(malformed.get("error_code", "")) == "invalid_json",
		"malformed raw UTF-8 is rejected before Godot JSON",
	)

	var tampered_welcome := _welcome()
	tampered_welcome["world_schema_version"] = 21
	var tampered: Dictionary = verifier.call(
		"stage_server_message", JSON.stringify(tampered_welcome).to_utf8_buffer()
	)
	_check(
		not bool(tampered.get("ok", true))
		and String(tampered.get("error_code", "")) == "incompatible_welcome",
		"tampered protocol tuple is rejected",
	)

	var staged: Dictionary = _stage_welcome(verifier)
	_check(bool(staged.get("ok", false)), "valid welcome still stages after rejected tamper")
	_check(String(staged.get("kind", "")) == "welcome", "stage kind is sanitized")
	var sanitized: Variant = JSON.parse_string(String(staged.get("sanitized_frame", "")))
	_check(
		sanitized is Dictionary
		and String(sanitized.get("type", "")) == "welcome"
		and int(sanitized.get("protocol_version", -1)) == 18
		and sanitized.get("session_role", {}) is Dictionary
		and String(sanitized.get("session_role", {}).get("player_id", "")) == "player-local",
		"only typed sanitized JSON is exposed",
	)
	var token := int(staged.get("token", -1))
	var pending: Dictionary = verifier.call(
		"stage_server_message", JSON.stringify(_welcome()).to_utf8_buffer()
	)
	_check(
		not bool(pending.get("ok", true))
		and String(pending.get("error_code", "")) == "pending_stage",
		"only one stage can be pending",
	)
	var discarded: Dictionary = verifier.call("discard", token)
	_check(bool(discarded.get("ok", false)), "current token discards without commit")

	staged = _stage_welcome(verifier)
	token = int(staged.get("token", -1))
	var committed: Dictionary = verifier.call("commit", token)
	_check(
		bool(committed.get("ok", false))
		and not committed.has("acknowledgement"),
		"non-state commit produces no acknowledgement",
	)
	var reused: Dictionary = verifier.call("commit", token)
	_check(
		not bool(reused.get("ok", true))
		and String(reused.get("error_code", "")) == "invalid_stage_token",
		"stage token is one-use",
	)

	_check(bool(_reset(verifier).get("ok", false)), "connection reset succeeds")
	staged = _stage_welcome(verifier)
	token = int(staged.get("token", -1))
	_check(bool(_reset(verifier).get("ok", false)), "reset clears a pending stage")
	var invalidated: Dictionary = verifier.call("commit", token)
	_check(
		not bool(invalidated.get("ok", true))
		and String(invalidated.get("error_code", "")) == "invalid_stage_token",
		"reset invalidates prior tokens",
	)
	staged = _stage_welcome(verifier)
	_check(bool(staged.get("ok", false)), "fresh handshake is required after reset")


func _reset(verifier: Object) -> Dictionary:
	return verifier.call(
		"reset_player",
		"player-local",
		20,
		16,
		11,
		"p1.5.0",
		"fc61c05b335fb951868010ecf2942a92ec4f03d00d0a75d3acba8c6f5162b6bd",
		"the-verse-local",
		"4c367bbfa04218ece14104f0a3a7ec2c7e9fefcc37d4cf78a265df2d711a59da",
		"ce89422bd5d0c4a2ddc50f22883439a7ee1ecd7dd14165a46bb500623fd0b7eb",
	)


func _test_portable_player_vector() -> void:
	var verifier: Object = ClassDB.instantiate("VerseInterestVerifier")
	_check(verifier != null, "portable vector verifier can be instantiated")
	if verifier == null:
		return
	var reset: Dictionary = verifier.call(
		"reset_player",
		"player-vector",
		20,
		16,
		11,
		"p1.5.0",
		"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		"universe-vector",
		"f00517b0fbef09d7924fde2cb11f2c74066627992ab900a6a9e0bd3ac3dc7311",
		"a3d5eb718f859d6010854f231a0e2cb4518c9618580020762311b4c3e43e3e06",
	)
	_check(bool(reset.get("ok", false)), "portable player verifier resets")
	for name in ["welcome.json", "registry.json"]:
		var staged := _stage_vector(verifier, name)
		_check(bool(staged.get("ok", false)), "%s stages" % name)
		if not bool(staged.get("ok", false)):
			return
		var committed: Dictionary = verifier.call("commit", int(staged.get("token", -1)))
		_check(
			bool(committed.get("ok", false)) and not committed.has("acknowledgement"),
			"%s commits without state ACK" % name,
		)

	var baseline_bytes := _vector_bytes("baseline.json")
	var tampered_bytes := baseline_bytes.get_string_from_utf8().replace(
		'"altitude_m":-5e-7', '"altitude_m":-1.5e-6'
	).to_utf8_buffer()
	var tampered: Dictionary = verifier.call("stage_server_message", tampered_bytes)
	_check(
		not bool(tampered.get("ok", true))
		and String(tampered.get("error_code", "")) == "hash_mismatch"
		and not tampered.has("acknowledgement"),
		"portable hash tamper cannot stage or emit an ACK",
	)

	var client := Node3D.new()
	client.set_script(CLIENT_SCRIPT)
	for vector in [["baseline.json", "baseline.ack.json"], ["delta.json", "delta.ack.json"]]:
		var staged := _stage_vector(verifier, vector[0])
		_check(bool(staged.get("ok", false)), "%s stages after tamper" % vector[0])
		if not bool(staged.get("ok", false)):
			continue
		var presentation := String(staged.get("sanitized_frame", ""))
		_check(
			presentation.contains("__VERSE_LOSSLESS_INTEGER__:9007199254740993"),
			"%s marks the unsafe interest epoch before Godot JSON" % vector[0],
		)
		var parsed: Variant = JSON.parse_string(presentation)
		_check(
			parsed is Dictionary
			and bool(client.call("_decode_lossless_protocol_integers", parsed)),
			"%s lossless presentation decodes" % vector[0],
		)
		if parsed is Dictionary:
			var authoritative: Dictionary = parsed.get(
				"baseline" if vector[0] == "baseline.json" else "delta", {}
			)
			_check(
				typeof(authoritative.get("event_sequence", null)) == TYPE_INT
				and int(authoritative.get("event_sequence", 0))
				== (9007199254740995 if vector[0] == "baseline.json" else 9007199254740996)
				and int(authoritative.get("interest", {}).get("interest_epoch", 0))
				== 9007199254740993,
				"%s installs exact >2^53 view frontiers" % vector[0],
			)
			if vector[0] == "baseline.json":
				var private_state: Dictionary = authoritative.get("actor_private", {})
				_check(
					int(private_state.get("committed_operation_sequence", 0))
					== 9007199254741011
					and int(private_state.get("inventories", [])[0].get("mass_grams", 0))
					== 15000,
					"portable private frontier and inventory remain exact",
				)
		var committed: Dictionary = verifier.call("commit", int(staged.get("token", -1)))
		_check(
			bool(committed.get("ok", false))
			and committed.get("acknowledgement", PackedByteArray()) == _vector_bytes(vector[1]),
			"%s emits the published exact ACK only after presentation acceptance" % vector[0],
		)
	client.free()

	var integrated_verifier: Object = ClassDB.instantiate("VerseInterestVerifier")
	var integrated_reset: Dictionary = integrated_verifier.call(
		"reset_player",
		"player-vector",
		20,
		16,
		11,
		"p1.5.0",
		"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		"universe-vector",
		"f00517b0fbef09d7924fde2cb11f2c74066627992ab900a6a9e0bd3ac3dc7311",
		"a3d5eb718f859d6010854f231a0e2cb4518c9618580020762311b4c3e43e3e06",
	)
	_check(bool(integrated_reset.get("ok", false)), "integrated portable verifier resets")
	var integrated_client := _vector_client(integrated_verifier)
	for name in ["welcome.json", "registry.json", "baseline.json", "delta.json"]:
		integrated_client.call("_verify_and_handle_packet", _vector_bytes(name))
		_check(
			String(integrated_client.get("replication_state")) != "fatal",
			"integrated %s remains valid (%s)"
			% [name, String(integrated_client.get("replication_detail"))],
		)
	var installed_private: Dictionary = integrated_client.get("actor_private_snapshot")
	var installed_inventories: Array = installed_private.get("inventories", [])
	var installed_inventory: Dictionary = (
		installed_inventories[0] if not installed_inventories.is_empty() else {}
	)
	_check(
		String(integrated_client.get("replication_state")) == "ready"
		and int(integrated_client.get("interest_epoch")) == 9007199254740993
		and int(integrated_client.get("interest_delta_sequence")) == 1
		and int(integrated_client.get("snapshot").get("event_sequence", 0))
		== 9007199254740996
		and int(integrated_client.get("committed_operation_sequence"))
		== 9007199254741011
		and int(installed_inventory.get("mass_grams", 0)) == 15000
		and (integrated_client.get("test_outbound_trace") as Array)
		== ["interest_ack", "interest_ack"],
		"portable >2^53 baseline and delta pass the complete verified model install before ACK",
	)
	var handoff := {
		"type": "handoff",
		"handoff": {
			"transfer_id": "transfer-native-adapter",
			"phase": "preparing",
			"destination_cell_key": {
				"schema_version": 1,
				"universe_id": "universe-vector",
				"sector": {"x": "0", "y": "0", "z": "0"},
				"cell": {"x": 2, "y": 2, "z": 3},
			},
			"placement_generation": 2,
		},
	}
	for phase in ["preparing", "importing", "verifying_destination"]:
		handoff["handoff"]["phase"] = phase
		integrated_client.call(
			"_verify_and_handle_packet", JSON.stringify(handoff).to_utf8_buffer()
		)
		_check(
			String(integrated_client.get("replication_state")) != "fatal"
			and String(integrated_client.get("handoff_phase")) == phase,
			"native adapter commits %s handoff presentation" % phase,
		)
	_check(
		(integrated_client.get("snapshot") as Dictionary).is_empty()
		and not bool(integrated_client.get("authoritative_player_ready"))
		and (integrated_client.get("test_outbound_trace") as Array)
		== ["interest_ack", "interest_ack"],
		"handoff phases discard source presentation without emitting a state ACK",
	)
	integrated_client.free()


func _test_frozen_invalid_corpus() -> void:
	var corpus_value: Variant = JSON.parse_string(
		_vector_bytes("invalid-corpus.json").get_string_from_utf8()
	)
	_check(corpus_value is Dictionary, "frozen invalid corpus manifest parses")
	if not (corpus_value is Dictionary):
		return
	var cases: Array = corpus_value.get("cases", [])
	_check(cases.size() == 16, "frozen invalid corpus size is exact")
	var presentation_client := Node3D.new()
	presentation_client.set_script(CLIENT_SCRIPT)
	for case_value in cases:
		var case: Dictionary = case_value
		var case_name := String(case.get("name", "unnamed"))
		var verifier: Object = ClassDB.instantiate("VerseInterestVerifier")
		_check(verifier != null, "%s verifier instantiates" % case_name)
		if verifier == null:
			continue
		_check(
			bool(_reset_vector(verifier).get("ok", false)),
			"%s verifier resets" % case_name,
		)
		for prerequisite_value in case.get("prerequisites", []):
			var prerequisite := String(prerequisite_value)
			var staged := _stage_vector(verifier, prerequisite)
			_check(bool(staged.get("ok", false)), "%s prerequisite %s stages" % [case_name, prerequisite])
			if bool(staged.get("ok", false)):
				var committed: Dictionary = verifier.call("commit", int(staged.get("token", -1)))
				_check(bool(committed.get("ok", false)), "%s prerequisite %s commits" % [case_name, prerequisite])

		var rejected := _stage_vector(verifier, String(case.get("frame", "")))
		_check(
			not bool(rejected.get("ok", true))
			and String(rejected.get("error_code", "")) == String(case.get("expected_code", ""))
			and not rejected.has("acknowledgement"),
			"%s rejects without stage or ACK" % case_name,
		)
		if bool(rejected.get("ok", false)):
			verifier.call("discard", int(rejected.get("token", -1)))

		var recovery := _stage_vector(verifier, String(case.get("recovery_frame", "")))
		_check(bool(recovery.get("ok", false)), "%s exact recovery stages" % case_name)
		if bool(recovery.get("ok", false)):
			var target := String(case.get("target", ""))
			if target in ["baseline", "delta"]:
				var presentation := String(recovery.get("sanitized_frame", ""))
				var parsed: Variant = JSON.parse_string(presentation)
				_check(
					presentation.contains("__VERSE_LOSSLESS_INTEGER__:9007199254740993")
					and parsed is Dictionary
					and bool(presentation_client.call("_decode_lossless_protocol_integers", parsed)),
					"%s recovery presentation preserves lossless integers" % case_name,
				)
			var committed: Dictionary = verifier.call("commit", int(recovery.get("token", -1)))
			_check(
				bool(committed.get("ok", false))
				and committed.has("acknowledgement") == (target in ["baseline", "delta"]),
				"%s exact recovery commit has expected ACK shape" % case_name,
			)
	presentation_client.free()


func _reset_vector(verifier: Object) -> Dictionary:
	return verifier.call(
		"reset_player",
		"player-vector",
		20,
		16,
		11,
		"p1.5.0",
		"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		"universe-vector",
		"f00517b0fbef09d7924fde2cb11f2c74066627992ab900a6a9e0bd3ac3dc7311",
		"a3d5eb718f859d6010854f231a0e2cb4518c9618580020762311b4c3e43e3e06",
	)


func _vector_client(verifier: Object) -> Node3D:
	var client := Node3D.new()
	client.set_script(CLIENT_SCRIPT)
	var camera := Camera3D.new()
	client.add_child(camera)
	client.set("camera", camera)
	for field in ["asteroid_root", "grids_root", "players_root", "planet_root"]:
		var node := Node3D.new()
		client.add_child(node)
		client.set(field, node)
	var material := StandardMaterial3D.new()
	client.set("rock_material", material)
	client.set("block_materials", {"structural": material})
	client.set("detail_materials", {
		"amber": material,
		"construction": material,
		"dark": material,
		"steel": material,
	})
	client.set("requested_player_id", "player-vector")
	client.set("interest_verifier", verifier)
	client.set("test_capture_transport", true)
	return client


func _stage_vector(verifier: Object, name: String) -> Dictionary:
	return verifier.call("stage_server_message", _vector_bytes(name))


func _vector_bytes(name: String) -> PackedByteArray:
	var native_root := ProjectSettings.globalize_path("res://").simplify_path()
	var path := native_root.path_join(
		"../../crates/verse-interest-verifier/test-vectors/v1/%s" % name
	).simplify_path()
	var bytes := FileAccess.get_file_as_bytes(path)
	if bytes.size() > 0 and bytes[bytes.size() - 1] == 10:
		bytes.resize(bytes.size() - 1)
	return bytes


func _stage_welcome(verifier: Object) -> Dictionary:
	return verifier.call("stage_server_message", JSON.stringify(_welcome()).to_utf8_buffer())


func _welcome() -> Dictionary:
	return {
		"type": "welcome",
		"protocol_version": 18,
		"projection_schema_version": 4,
		"world_schema_version": 20,
		"event_schema_version": 16,
		"content_schema_version": 11,
		"content_manifest_version": "p1.5.0",
		"celestial_registry_schema_version": 1,
		"universe_manifest_schema_version": 4,
		"interest_schema_version": 2,
		"server_name": "native-verifier-smoke",
		"session_role": {"kind": "player", "player_id": "player-local"},
	}


func _check(condition: bool, label: String) -> void:
	if not condition:
		failures.append(label)
