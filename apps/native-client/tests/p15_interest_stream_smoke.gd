# SPDX-License-Identifier: AGPL-3.0-or-later
extends SceneTree

const CLIENT_SCRIPT: Script = preload("res://src/main.gd")
const HASH_A := "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
const HASH_B := "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
const HASH_C := "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
const HASH_D := "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
const HASH_E := "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"

var failures: Array[String] = []


func _initialize() -> void:
	call_deferred("_run")


func _run() -> void:
	_test_exact_address_projection()
	_test_registry_and_contiguous_interest_stream()
	_test_legacy_family_fails_closed()
	if not failures.is_empty():
		for failure in failures:
			printerr("VERSE_NATIVE_P15_FAILED %s" % failure)
		quit(1)
		return
	print(
		"VERSE_NATIVE_P15_OK tuple=validated registry=bound address=exact "
		+ "baseline=atomic delta=contiguous removals=explicit stale=fail_closed legacy=rejected"
	)
	quit(0)


func _check(condition: bool, label: String) -> void:
	if not condition:
		failures.append(label)


func _new_client() -> Node3D:
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
	client.set("requested_player_id", "player-local")
	return client


func _test_exact_address_projection() -> void:
	var client := _new_client()
	client.set("universe_manifest", _manifest())
	var origin := _address(
		"170141183460469231731687303715884105726", 999, 9_999_999_999
	)
	var target := _address(
		"170141183460469231731687303715884105727", 0, -10_000_000_000
	)
	var offset: Variant = client.call("_address_relative_m", target, origin)
	_check(offset is Vector3, "huge signed sector coordinates retain a bounded renderer offset")
	if offset is Vector3:
		_check(is_equal_approx(offset.x, 0.000001), "sector/cell carry resolves to exactly one micrometre")
	var far := _address("1", 500, 0)
	_check(
		client.call("_address_relative_m", far, _address("0", 500, 0)) == null,
		"unbounded renderer coordinates are rejected"
	)
	client.free()


func _test_registry_and_contiguous_interest_stream() -> void:
	var client := _new_client()
	client.call("_handle_server_message", _wire(_welcome()))
	_check(bool(client.get("welcome_received")), "complete protocol tuple is accepted")
	client.call("_handle_server_message", _wire({
		"type": "registry", "registry": _registry(), "universe_manifest": _manifest(),
	}))
	_check(bool(client.get("registry_received")), "manifest-bound registry is accepted before state")

	var baseline := _baseline()
	_check(_install_presentation_frame(client, baseline, true), "valid baseline presentation stages")
	_check(client.get("replication_state") == "ready", "valid baseline becomes ready")
	_check(int(client.get("interest_delta_sequence")) == 0, "baseline installs delta frontier zero")
	var grids: Dictionary = client.get("interest_entities")["grid"]
	_check(grids.size() == 1 and grids.has("grid-a"), "baseline complete enter is installed once")
	_check(
		(client.get("grid_node_lookup")["grid-a"] as Node3D).position.is_equal_approx(Vector3(2.0, 0.0, 0.0)),
		"grid renderer position is derived from its exact address"
	)
	var baseline_grid_node: Node3D = client.get("grid_node_lookup")["grid-a"]
	var baseline_grid_instance := baseline_grid_node.get_instance_id()
	_check(
		(client.get("voxel_lookup") as Dictionary).has("0,0,0"),
		"baseline voxel chunk is rendered from its complete payload"
	)
	var baseline_voxel_fingerprint := String(
		(client.get("rendered_voxel_chunk_fingerprints") as Dictionary).get(
			"body-test:chunk:0:0:0", ""
		)
	)
	var registered_visuals: Dictionary = client.get("celestial_visuals")
	_check(
		registered_visuals.has("body-test")
		and registered_visuals["body-test"].get_meta("verse_visual_descriptor", "") == "neutral_proxy",
		"unknown registered visual descriptors use a neutral proxy"
	)
	var baseline_celestial_instance := (
		registered_visuals["body-test"] as Node3D
	).get_instance_id()

	var delta_one := _delta(1, HASH_C, HASH_D)
	var replacement := _grid(3_000_000)
	replacement["power"]["online"] = true
	delta_one["interest"]["replaced"] = [_projection("grid", "grid-a", replacement, 2)]
	delta_one["interest"]["replaced"].append(
		_projection("voxel_chunk", "body-test:chunk:0:0:0", _chunk(2, 1, "ferrite_ore"), 2)
	)
	delta_one["interest"]["entered"] = [
		_projection("death_drop", "drop-a", {
			"drop_id": "drop-a", "address": _address("0", 500, 4_000_000),
		}, 1),
	]
	_check(_install_presentation_frame(client, delta_one, false), "first delta presentation stages")
	_check(int(client.get("interest_delta_sequence")) == 1, "first contiguous delta advances frontier")
	_check(
		(client.get("grid_node_lookup")["grid-a"] as Node3D).position.is_equal_approx(Vector3(3.0, 0.0, 0.0)),
		"absolute replacement updates the existing grid without a motion patch"
	)
	var moved_grid_node: Node3D = client.get("grid_node_lookup")["grid-a"]
	_check(
		moved_grid_node.get_instance_id() == baseline_grid_instance,
		"motion and power replacement preserve stable grid node identity"
	)
	_check(
		moved_grid_node.get_node_or_null("VersePowerWorkLight") != null,
		"power state is updated in place without rebuilding block topology"
	)
	_check(
		not (client.get("voxel_lookup") as Dictionary).has("0,0,0")
		and (client.get("voxel_lookup") as Dictionary).has("1,0,0")
		and int(client.get("rendered_voxel_count")) == 1,
		"same-count voxel chunk replacement refreshes rendered voxel content"
	)
	_check(
		String(
			(client.get("rendered_voxel_chunk_fingerprints") as Dictionary).get(
				"body-test:chunk:0:0:0", ""
			)
		) != baseline_voxel_fingerprint,
		"chunk revision and payload replacement advances its render fingerprint"
	)
	_check(
		(client.get("celestial_visuals")["body-test"] as Node3D).get_instance_id()
		== baseline_celestial_instance,
		"ordinary interest deltas preserve registered celestial node identity"
	)

	var delta_two := _delta(2, HASH_D, HASH_E)
	delta_two["interest"]["removed"] = [
		{"entity_id": "grid-a", "kind": "grid", "reason": "out_of_interest"},
		{"entity_id": "drop-a", "kind": "death_drop", "reason": "destroyed"},
		{
			"entity_id": "body-test:chunk:0:0:0", "kind": "voxel_chunk",
			"reason": "out_of_interest",
		},
	]
	_check(_install_presentation_frame(client, delta_two, false), "removal delta presentation stages")
	_check(int(client.get("interest_delta_sequence")) == 2, "second contiguous delta advances frontier")
	_check(client.get("interest_entities")["grid"].is_empty(), "explicit removal leaves no grid ghost")
	_check(client.get("grid_node_lookup").is_empty(), "removed grid has no renderer node")
	_check(
		(client.get("voxel_lookup") as Dictionary).is_empty()
		and (client.get("rendered_voxel_chunk_fingerprints") as Dictionary).is_empty(),
		"removed voxel chunk clears its rendered voxels and fingerprint"
	)

	var delta_three := _delta(3, HASH_E, HASH_A)
	delta_three["interest"]["entered"] = [_projection("grid", "grid-a", _grid(5_000_000), 3)]
	_check(_install_presentation_frame(client, delta_three, false), "re-entry delta presentation stages")
	_check(int(client.get("interest_delta_sequence")) == 3, "removed identity may enter again freshly")
	_check(
		(client.get("grid_node_lookup")["grid-a"] as Node3D).position.is_equal_approx(Vector3(5.0, 0.0, 0.0)),
		"re-entry installs a complete fresh value without a ghost revision"
	)
	var reentered_grid_instance := (
		client.get("grid_node_lookup")["grid-a"] as Node3D
	).get_instance_id()
	_check(
		reentered_grid_instance != baseline_grid_instance,
		"grid removal and re-entry allocate a fresh renderer identity"
	)

	var delta_four := _delta(4, HASH_A, HASH_B)
	var topology_replacement := _grid(6_000_000)
	topology_replacement["blocks"] = [_block("block-a", 0)]
	delta_four["interest"]["replaced"] = [
		_projection("grid", "grid-a", topology_replacement, 4),
	]
	_check(_install_presentation_frame(client, delta_four, false), "topology delta presentation stages")
	_check(int(client.get("interest_delta_sequence")) == 4, "topology delta advances frontier")
	var rebuilt_grid: Node3D = client.get("grid_node_lookup")["grid-a"]
	_check(
		rebuilt_grid.get_instance_id() != reentered_grid_instance
		and rebuilt_grid.get_node_or_null("block-a") != null,
		"block topology replacement rebuilds only the changed grid renderer"
	)
	_check(
		(client.get("celestial_visuals")["body-test"] as Node3D).get_instance_id()
		== baseline_celestial_instance,
		"topology deltas do not rebuild fixed celestial visuals"
	)
	client.set("interest_local_origin", _address("0", 500, 1_000_000))
	client.call("_rebuild_registered_celestials")
	_check(
		(client.get("celestial_visuals")["body-test"] as Node3D).get_instance_id()
		!= baseline_celestial_instance,
		"a changed exact local origin deliberately rebuilds celestial relative positions"
	)

	var committed_entities: Dictionary = client.get("interest_entities").duplicate(true)
	var gap := _delta(6, HASH_B, HASH_C)
	gap["interest"]["entered"] = [_projection("grid", "grid-gap", _grid(9_000_000), 1)]
	if not _install_presentation_frame(client, gap, false):
		client.call("_request_fresh_interest_baseline", "INTEREST FRONTIER MISMATCH")
	_check(client.get("replication_state") == "stale", "delta gap marks the client stale")
	_check(bool(client.get("baseline_request_pending")), "delta gap requests a replacement baseline")
	_check(client.get("interest_entities") == committed_entities, "invalid staged delta is discarded")
	_check(not bool(client.get("authoritative_player_ready")), "stale frontier freezes authored controls")
	client.free()


func _install_presentation_frame(client: Node3D, authoritative: Dictionary, baseline: bool) -> bool:
	var candidate: Variant = client.call(
		"_prepare_interest_baseline" if baseline else "_prepare_interest_delta",
		_wire(authoritative),
	)
	if not candidate is Dictionary or candidate.is_empty():
		return false
	client.set("test_capture_transport", true)
	return bool(client.call(
		"_finalize_committed_interest",
		candidate,
		"{\"type\":\"acknowledge_interest\"}".to_utf8_buffer(),
	))


func _test_legacy_family_fails_closed() -> void:
	var client := _new_client()
	client.call("_handle_server_message", {"type": "snapshot", "snapshot": {}})
	_check(client.get("replication_state") == "fatal", "legacy snapshot family is a fatal protocol error")
	client.free()


func _wire(value: Dictionary) -> Dictionary:
	var parsed: Variant = JSON.parse_string(JSON.stringify(value))
	return parsed if parsed is Dictionary else {}


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
		"server_name": "test",
		"session_role": {"kind": "player", "player_id": "player-local"},
	}


func _manifest() -> Dictionary:
	return {
		"schema_version": 4,
		"manifest_hash": HASH_A,
		"universe_id": "the-verse-local",
		"world_seed": "test",
		"address_schema_version": 1,
		"sector_edge_um": 20_000_000_000_000,
		"cell_edge_um": 20_000_000_000,
		"cells_per_sector_axis": 1000,
		"generation_rule_version": "p1.5-proof-1",
		"frontier_policy_version": "proof-1",
		"celestial_registry_schema_version": 1,
		"celestial_registry_hash": HASH_B,
		"content_schema_version": 11,
		"content_manifest_version": "p1.5.0",
		"content_hash": HASH_C,
		"world_schema_version": 20,
		"event_schema_version": 16,
		"lifecycle_control_schema_version": 1,
		"production_schedule_occurrence_schema_version": 1,
		"lifecycle_policy_hash": "5bc077cc8a2eb101fcaecdce5513c13aa243e1f68a5af839a602dd689859ff3a",
	}


func _registry() -> Dictionary:
	return {
		"schema_version": 1,
		"registry_hash": HASH_B,
		"license": "CC-BY-SA-4.0",
		"universe_id": "the-verse-local",
		"generation_rule_version": "p1.5-proof-1",
		"minimum_fixed_body_surface_gap_um": 3_000_000_000,
		"bodies": [{
			"body_id": "body-test",
			"display_name": "Test Body",
			"kind": "asteroid",
			"center": _address("0", 500, 0),
			"surface_radius_um": 1_000_000,
			"exclusion_radius_um": 1_000_000,
			"fixed_orientation_microradians": {"x": 0, "y": 0, "z": 0},
			"surface_gravity_millimetres_per_second_squared": 0,
			"atmosphere_height_um": 0,
			"oxygen_parts_per_million": 0,
			"geometry_definition_id": "sphere-heightfield-v1",
			"material_definition_id": "test",
			"gravity_definition_id": "none-v1",
			"atmosphere_definition_id": "none-v1",
			"resource_definition_id": "none-v1",
			"visual_descriptor_id": "unknown-test-v1",
			"scale_class": "proof",
			"generation_seed": "test",
			"generation_rule_version": "p1.5-proof-1",
			"materialized_registry_version": 1,
			"content_manifest_version": "p1.5.0",
			"content_hash": HASH_C,
		}],
	}


func _baseline() -> Dictionary:
	var public_player := _public_player()
	var grid := _grid(2_000_000)
	var chunk := _chunk(1, 0, "rock")
	return {
		"projection_schema_version": 4,
		"schema_version": 20,
		"content_manifest_version": "p1.5.0",
		"universe_id": "the-verse-local",
		"cell_id": "origin",
		"universe_manifest_hash": HASH_A,
		"celestial_registry_hash": HASH_B,
		"cell_address": _address("0", 500, 0),
		"gravity_body_id": "body-test",
		"voxel_body_id": "body-test",
		"event_sequence": 0,
		"simulation_tick": 0,
		"fencing_token": 1,
		"world_hash": HASH_A,
		"players": [public_player],
		"environment": _environment(),
		"voxel_chunks": [chunk],
		"grids": [grid],
		"death_drops": [],
		"conservation_valid": true,
		"interest": _interest("baseline", 0, "", HASH_C),
		"actor_private": _actor_private(),
	}


func _delta(sequence: int, previous_hash: String, view_hash: String) -> Dictionary:
	return {
		"projection_schema_version": 4,
		"schema_version": 20,
		"content_manifest_version": "p1.5.0",
		"universe_id": "the-verse-local",
		"cell_id": "origin",
		"universe_manifest_hash": HASH_A,
		"celestial_registry_hash": HASH_B,
		"cell_address": _address("0", 500, 0),
		"gravity_body_id": "body-test",
		"voxel_body_id": "body-test",
		"event_sequence": sequence,
		"simulation_tick": sequence,
		"world_hash": HASH_A,
		"environment": _environment(),
		"conservation_valid": true,
		"interest": _interest("delta", sequence, previous_hash, view_hash),
		"actor_private": _actor_private(),
	}


func _interest(kind: String, sequence: int, previous_hash: String, view_hash: String) -> Dictionary:
	var interest := {
		"schema_version": 2,
		"frame_kind": kind,
		"session_epoch": "session-a",
		"interest_epoch": 1,
		"baseline_id": "baseline-a",
		"delta_sequence": sequence,
		"observer_class": "bound_player",
		"cell_address": _address("0", 500, 0),
		"local_origin_address": _address("0", 500, 0),
		"registry_hash": HASH_B,
		"universe_manifest_hash": HASH_A,
		"canonical_event_sequence": sequence,
		"canonical_tick": sequence,
		"canonical_world_hash": HASH_A,
		"view_hash": view_hash,
		"entered": [],
		"replaced": [],
		"removed": [],
	}
	if kind == "baseline":
		interest["previous_view_hash"] = null
		interest["entered"] = [
			_projection("player", "player-local", _public_player(), 1),
			_projection("grid", "grid-a", _grid(2_000_000), 1),
			_projection(
				"voxel_chunk", "body-test:chunk:0:0:0", _chunk(1, 0, "rock"), 1
			),
		]
	else:
		interest["previous_view_hash"] = previous_hash
	return interest


func _projection(kind: String, entity_id: String, value: Dictionary, revision: int) -> Dictionary:
	return {
		"entity_id": entity_id,
		"kind": kind,
		"projected_revision": revision,
		"component_schema_version": 4,
		"payload": {"entity_kind": kind, "value": value},
	}


func _public_player() -> Dictionary:
	return {
		"player_id": "player-local",
		"address": _address("0", 500, 0),
		"orientation": _quat(),
		"linear_velocity": _vec(),
		"angular_velocity": _vec(),
		"surface_contact": false,
		"locomotion_kind": "eva",
		"life_state": "alive",
		"helmet_closed": true,
		"jetpack_enabled": true,
	}


func _grid(local_x_um: int) -> Dictionary:
	return {
		"grid_id": "grid-a",
		"owner_player_id": "player-local",
		"address": _address("0", 500, local_x_um),
		"orientation": _quat(),
		"linear_velocity": _vec(),
		"angular_velocity": _vec(),
		"anchored": false,
		"power": {"online": false, "generation": 0, "consumption": 0},
		"blocks": [],
	}


func _chunk(revision: int, coordinate_x: int, material: String) -> Dictionary:
	return {
		"chunk_id": "body-test:chunk:0:0:0",
		"body_id": "body-test",
		"revision": revision,
		"voxels": [{
			"coordinate": {"x": coordinate_x, "y": 0, "z": 0},
			"material": material,
		}],
	}


func _block(block_id: String, coordinate_x: int) -> Dictionary:
	return {
		"block_id": block_id,
		"kind": "structural",
		"coordinate": {"x": coordinate_x, "y": 0, "z": 0},
		"orientation": 0,
		"health": 100,
		"max_health": 100,
		"construction_complete": true,
	}


func _actor_private() -> Dictionary:
	var player := {
		"player_id": "player-local",
		"inventory_id": "inventory-player-local",
		"address": _address("0", 500, 0),
		"orientation": _quat(),
		"linear_velocity": _vec(),
		"angular_velocity": _vec(),
		"surface_contact": false,
		"locomotion": {"kind": "eva", "jump_held": false, "magnetic_boots_enabled": false},
		"movement_epoch": 1,
		"last_received_input_sequence": 0,
		"last_processed_input_sequence": 0,
		"dampeners": true,
		"jetpack_enabled": true,
		"life_state": {"kind": "alive"},
		"level": 1,
		"environment": _environment(),
	}
	return {
		"player": player,
		"committed_operation_sequence": 0,
		"inventories": [{
			"inventory_id": "inventory-player-local",
			"domain": {"kind": "player", "player_id": "player-local"},
			"contents": {"ore": 0, "refined_material": 0, "components": 0},
		}],
		"death_drops": [],
		"owned_grid_masses": [],
		"production_queues": [],
	}


func _environment() -> Dictionary:
	return {
		"celestial_body_id": "body-test",
		"celestial_body_name": "Test Body",
		"celestial_scale_class": "proof",
		"nearest_body_id": "body-test",
		"nearest_body_name": "Test Body",
		"planet_center": _vec(),
		"surface_radius_m": 1.0,
		"distance_to_center_m": 2.0,
		"distance_to_surface_m": 1.0,
		"altitude_m": 1.0,
		"gravity": _vec(),
		"gravity_m_s2": 0.0,
		"atmosphere_density": 0.0,
		"oxygen_fraction": 0.0,
		"breathable": false,
	}


func _address(sector_x: String, cell_x: int, local_x_um: int) -> Dictionary:
	return {
		"universe_id": "the-verse-local",
		"sector": {"x": sector_x, "y": "0", "z": "0"},
		"cell": {"x": cell_x, "y": 500, "z": 500},
		"local_um": {"x": local_x_um, "y": 0, "z": 0},
	}


func _vec() -> Dictionary:
	return {"x": 0.0, "y": 0.0, "z": 0.0}


func _quat() -> Dictionary:
	return {"x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0}
