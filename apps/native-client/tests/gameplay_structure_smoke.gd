# SPDX-License-Identifier: AGPL-3.0-or-later
extends SceneTree

var failures := 0
func check(ok: bool, message: String) -> void:
	if not ok:
		failures += 1
		printerr(message)

func _initialize() -> void:
	var client = load("res://src/main.gd").new()
	client.camera = Camera3D.new()
	client.add_child(client.camera)
	client.block_materials = {"structural": StandardMaterial3D.new()}
	for key in ["dark", "steel", "construction", "amber"]:
		client.detail_materials[key] = StandardMaterial3D.new()
	var blocks: Array[Dictionary] = []
	for i in range(4):
		blocks.append({"block_id": "b%d" % i, "kind": "structural", "coordinate": {"x": i * 2, "y": 0, "z": 0}, "orientation": i, "health": 100, "max_health": 100, "construction_complete": true})
	var grid := {"grid_id": "test", "blocks": blocks}
	var batched: Node3D = client._create_grid_node(grid)
	var prototype: Node3D = client._build_block_visual(blocks[0])
	check(batched.get_child_count() == prototype.get_child_count(), "one batch per mesh part")
	for part_index in range(prototype.get_child_count()):
		var batch: MultiMeshInstance3D = batched.get_child(part_index)
		var part: MeshInstance3D = prototype.get_child(part_index)
		check(batch.multimesh.instance_count == 4, "all healthy blocks batched")
		check(batch.material_override == part.material_override, "material preserved")
		for i in range(4):
			var expected := Transform3D(Basis(Vector3.UP, deg_to_rad(i * 90)), Vector3(i * 2, 0, 0)) * part.transform
			if DisplayServer.get_name() != "headless":
				check(batch.multimesh.get_instance_transform(i).is_equal_approx(expected), "orientation and part placement preserved")
	var fingerprint: String = client._grid_topology_fingerprint(grid)
	var reordered := blocks.duplicate(true)
	reordered.reverse()
	check(client._grid_topology_fingerprint({"blocks": reordered}) == fingerprint, "fingerprint independent of order")
	for complete in [true, false]:
		var damaged := blocks[0].duplicate(true)
		damaged.health = 50
		damaged.construction_complete = complete
		check(not client._can_instance_structure(damaged), "damage and construction retain individual visuals")
		check(client._grid_topology_fingerprint({"blocks": [damaged]}) != client._grid_topology_fingerprint({"blocks": [blocks[0]]}), "damage changes visual fingerprint")
	client.grid_lookup = {"test": grid}
	var rng := RandomNumberGenerator.new()
	rng.seed = 8107
	for sample in range(500):
		var position := Vector3(rng.randf_range(-2, 8), rng.randf_range(-2, 2), rng.randf_range(-2, 2))
		var rotation := Quaternion(Vector3(1, 2, 3).normalized(), 0.63) if sample % 2 else Quaternion.IDENTITY
		var translation := Vector3(8, -3, 5) if sample % 2 else Vector3.ZERO
		grid["orientation"] = {"x": rotation.x, "y": rotation.y, "z": rotation.z, "w": rotation.w}
		grid["position"] = client._protocol_vec3(translation)
		position = Basis(rotation) * position + translation
		var expected_clear := true
		for fraction in [-1.0, -0.5, 0.0, 0.5, 1.0]:
			var center: Vector3 = position + client._camera_up() * client.CHARACTER_CAPSULE_HALF_HEIGHT * fraction
			for block in blocks:
				var delta: Vector3 = Basis(rotation).inverse() * (center - translation) - client._coord_vector(block.coordinate)
				var closest := delta.clamp(Vector3.ONE * -0.5, Vector3.ONE * 0.5)
				if (delta - closest).length_squared() < client.CHARACTER_COLLISION_RADIUS * client.CHARACTER_COLLISION_RADIUS:
					expected_clear = false
		check(client._player_position_is_clear(position) == expected_clear, "spatial collision matches exhaustive capsule test")
	client.grid_lookup = {"test": {"blocks": []}}
	check(client._player_position_is_clear(Vector3.ZERO), "replaced projection invalidates collision cache")
	var grand_blocks := []
	for coordinate in [Vector3i.ZERO, Vector3i(12, 0, 11), Vector3i(12, 8, 11)]:
		var block := blocks[0].duplicate(true)
		block["coordinate"] = {"x": coordinate.x, "y": coordinate.y, "z": coordinate.z}
		block["block_id"] = "block-capital-%s-%d-%d" % ["floor" if coordinate.y == 0 else "roof", coordinate.x, coordinate.z]
		grand_blocks.append(block)
	var grand: Node3D = client._create_grid_node({"grid_id": "grand", "blocks": grand_blocks})
	check(grand.get_node_or_null("GrandCapitalDecor") != null, "expanded capital has concourse ornament")
	check(grand.get_node("Capital_inlay_0").multimesh.instance_count == 1, "central arrival inlay stays instanced")
	check(grand.get_node("Capital_stone_0").multimesh.instance_count == 2, "stone architecture stays instanced")
	for ornament in grand.get_node("GrandCapitalDecor").get_children():
		if ornament is MeshInstance3D:
			check(ornament.position.y >= 5.0, "non-colliding decoration stays overhead")
	grand.free()
	batched.free()
	prototype.free()
	client.free()
	print("VERSE_STRUCTURE_OK" if failures == 0 else "VERSE_STRUCTURE_FAILED %d" % failures)
	quit(0 if failures == 0 else 1)
