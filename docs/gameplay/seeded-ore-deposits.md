# Seeded ore workshop (F-070 / UX-005)

The opt-in `ore-workshop` development genesis profile places clustered ferrite,
cuprite and cobaltite deposits. Targets are 13.2%, 5.5% and 3.3% of asteroid
voxels respectively; the rest is low-yield rock. Each mineral reserves three
surface samples. Integer noise and stable ranking make generation repeatable
for a world seed. Ore does not regenerate after mining.

These are geological varieties of the existing high-yield ore grade, not new
inventory currencies: each yields three shared ore units, versus one from rock.
All use the existing refinery and assembler recipes. Separate metal inventories
and recipes require a future coordinated economy/schema migration.

Generation runs only before the first event and persists the selected high-yield
coordinates in the existing snapshot. Established worlds are never reseeded.
Default orbital and Earth-start genesis remain unchanged. Roll back the client
or select an older launcher without changing existing saves; it will display
all rich deposits as ferrite. Native assay labels are a presentation-only,
seed-derived catalog and never determine mining yield or authority.

The engineering launcher uses a separate Ore Workshop save directory. Use a
fresh directory and `--genesis-profile ore-workshop` for a server test. The
native pinned connection contract still uses the packaged world seed.
