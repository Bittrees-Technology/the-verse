# SPDX-License-Identifier: AGPL-3.0-or-later
extends SceneTree


func _initialize() -> void:
	if ClassDB.class_exists("VerseInterestVerifier"):
		printerr("VERSE_NATIVE_VERIFIER_MISSING_FAILED extension unexpectedly loaded")
		quit(1)
		return
	print("VERSE_NATIVE_VERIFIER_MISSING_OK startup=fail_closed")
	quit(0)
