# SPDX-License-Identifier: AGPL-3.0-or-later
extends SceneTree

func _initialize() -> void:
	var builder := Node3D.new()
	builder.set_script(load("res://src/main.gd"))
	for offset in [Vector3.ZERO, Vector3(1000, -3000, 4000)]:
		var surface := SurfaceTool.new()
		surface.begin(Mesh.PRIMITIVE_TRIANGLES)
		var samples: Array[Vector3] = [offset + Vector3(0, -1, 0)]
		builder.call("_add_surface_triangle", surface, offset + Vector3(-1, 0, -1), offset + Vector3(1, 0, -1), offset + Vector3(0, 0, 1), samples)
		surface.generate_normals()
		var mesh := surface.commit()
		for normal in mesh.surface_get_arrays(0)[Mesh.ARRAY_NORMAL]:
			if normal.dot(Vector3.UP) < 0.99:
				printerr("VERSE_VOXEL_NORMALS_FAILED outward normal at %s: %s" % [offset, normal])
				builder.free()
				quit(1)
				return
	builder.free()
	print("VERSE_VOXEL_NORMALS_OK origin_and_planet=outward")
	quit(0)
