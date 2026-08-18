# Sparse Province IDs: characterization and migration design

## 1. Final external-ID invariant

With `preserve_ids`, positive `definition.csv` IDs are stable external identities. IDs may
have gaps: `1, 7, 42, 500` is valid. ID `0`, duplicate IDs, and duplicate colors remain
invalid. The compatibility scanner retains `SparseProvinceIds` as a non-blocking warning
with count/minimum/maximum/contiguity metadata; it does not repair or normalize IDs.

## 2. Why it exists

The original bridge uses a dense vector indexed by external province ID. It therefore
conflates province count, maximum ID, and the external identity.

## 3. Assumption inventory

| Location | Assumption | Risk |
| --- | --- | --- |
| `map/bridge.rs:construct_map_data` | public compatibility gate rejects a non-contiguous definition table | intentional final load blocker |
| same constructor | rejects `id == 0` or `id > preserved_id_count` | critical load blocker |
| `deconstruct_map_data_preserve_ids` | formerly used `Vec<Option<Definition>>` sized by live province count, accessed at `id - 1` | resolved in Phase 2 |
| same deconstructor | formerly filled holes by reassigning outlier IDs | resolved in Phase 2 |
| same deconstructor | formerly reported created/deleted ranges from counts | resolved in Phase 2 |
| `deconstruct_map_data_no_preserve_ids` | deliberately writes `1..=count` | expected when ID preservation is disabled |
| `CompatibilityReport::ids_contiguous` | compares IDs to `1..=definitions.len()` | compatibility gate |

Canvas selection, `StateEditSession`, State indexes, State Fill, and boundary lookup use
`u32` values in maps/sets and do not themselves index vectors by province ID.

## 4. Public load behavior

`construct_map_data` builds a `ProvinceIdIndex` for every definition-table lookup and no
longer requires contiguity. Thus a normal public project open accepts `1, 7, 42, 500` and
preserves exactly those IDs in the mutable map. Map colors still must match definitions and
the normal parser/bitmap validation remains unchanged.

## 5. Pre-Phase 2 save/deconstruction behavior

`deconstruct_map_data_preserve_ids` sizes `sparse_definitions_table` by active province
count and writes at `id - 1`. IDs above count become outliers; it fills holes from the
highest slot down and changes the outlier ID. The characterization test creates the
otherwise impossible in-memory IDs `1, 7, 42, 500`; current output is `1, 2, 3, 4`
with reassignment from an external ID above four into that dense range. The exact
ID-to-slot mapping is not a stable API because it originates from a hash map.

## 5a. Phase 2 implementation status

Preserved-ID deconstruction is now sparse-safe. It emits existing entries from
`ProvinceIdIndex` in ascending ID order, so `1, 7, 42, 500` remains exactly that sequence
without fake gap rows or a dense allocation. Colors not yet present in the index are sorted
lexicographically and receive checked IDs above the current maximum. Thus new colors are
deterministic, do not reuse gaps, and fail explicitly after `u32::MAX`.

`id_changes.txt` is now emitted only for those genuinely new assignments, in that same stable
order. Existing sparse IDs and deletions do not produce a false renumber entry. Candidate
validation rejects ID 0 and duplicate IDs/colors while continuing to validate BMP/CSV color
consistency. The ordinary Save Project pipeline now reloads sparse candidates through the
public loader and verifies their stable identities.

## 6. Deletion behavior

Canvas first calls `StateEditSession::remove_province_references` to drop or transfer
membership, victory points, and province buildings, then merges source pixels into the
target. Both map and State actions are undoable. Phase 2 serialization preserves every
remaining external ID, so deleting a non-tail ID creates a gap rather than shifting an
unrelated identity. Phase 3 additionally blocks removal when an `adjacencies.csv` record
references the province, so a read-only adjacency cannot be orphaned.

## 7. State-reference implications

State syntax stores province numbers as values. `StateIndexes` uses `HashMap<u32, u32>`,
`BTreeMap`, and `BTreeSet`; valid IDs are supplied as a set. Victory points and province
buildings are keyed by province ID in `StateWorkingSet`. Owner/controller/core/claim
data is State-level. Characterization tests prove State indexing accepts `1, 7, 42, 500`
when the map supplies that same valid-ID set.

## 8. Adjacency implications

Bridge loading resolves adjacency source, destination, and optional `through` IDs through
`ProvinceIdIndex`, never by indexing an ID-sized vector. The parser keeps `through = -1`
as `None`. A record with a missing endpoint or `through` is retained for deterministic
round-trip and emits a structured `cross.adjacency.province.unknown` diagnostic; valid
unsupported adjacency kinds remain preserved without a false missing-ID error. Save
reconstruction converts colors back through the same index. State Fill's
`ProvinceAdjacency` is already `BTreeMap<u32, BTreeSet<u32>>`, and canvas boundary setup
now obtains its IDs via the map index; its sparse traversal test retains `1 -> 7 -> 42 ->
500`. Province removal is rejected before State or map mutation if any adjacency record
uses the province as source, destination, or `through`.

## 9. Validation implications

Project validation treats IDs as values while checking unknown State references,
province buildings, victory points, and map/state consistency. Sparse support needs
identity checks for occupied IDs, ID-to-color mapping, State membership, adjacency,
victory points, and buildings after reload.

## 10. Save Project implications

`build_province_map_candidate` calls the deconstructor; unified Save Project then uses
the existing candidate, backup, journal, commit order, reload, semantic verification,
and recovery. Those transaction mechanisms are ID-agnostic and were not redesigned.
The candidate preserves sparse IDs and the normal public loader now completes the post-save
reload and semantic verification path without compaction.

## 11. Proposed target data model

Add a `ProvinceIdIndex` owned by the map model:

```text
occupied IDs: BTreeMap<ProvinceId, Color>
color lookup:  AHashMap<Color, ProvinceId>
province data: existing AHashMap<Color, ProvinceData>
```

The current implementation keeps `ProvinceId` as `u32` and exposes count, max ID, ordered
occupied iteration, both lookups, and checked allocation. `BTreeMap` provides stable
CSV ordering without memory proportional to the highest ID; the reverse hash map keeps
interactive color lookup fast. Existing color-keyed texture/boundary/history internals
can remain.

## 12. Stable Province ID invariants

- Existing positive IDs are external identities, not indexes.
- Count, maximum ID, and occupied IDs are distinct.
- Open/edit/save/reload never renumbers an existing ID.
- Deletion creates a gap; recolor preserves ID.
- State and adjacency references resolve through the same index.
- Any future renumber command is explicit and cross-domain.

## 13. New-ID allocation policy

Recommend checked `max(existing IDs) + 1`. It is deterministic, avoids rebinding a
deleted ID while stale references may exist, and is simple for undo/redo and external
mod tools. Smallest-unused-positive keeps ranges compact but reuses deletion gaps and
can turn stale references into a new province. Neither policy changes production here.

## 14. Migration phases

1. Add `ProvinceIdIndex` while retaining contiguous loading. **Complete.**
2. Migrate bridge/map/Canvas consumers from implicit indexes to it. **Complete.**
3. Preserve ordered sparse IDs in deconstruction and candidate validation; test gaps. **Complete.**
4. Migrate adjacency, State validation inputs, and deletion safety everywhere. **Complete.**
5. Remove the loader gate, change the scanner finding, and add full sparse E2E. **Complete.**

## 15. Test plan

Coverage includes public loading, compatibility metadata, `1, 7, 42, 500` ID/color
round trips, `1 -> 10000` internal bridge behavior, deterministic CSV and adjacency
serialization, State membership/VP/buildings, deletion guards, candidate validation,
combined Save Project reload, rollback, and interrupted-save recovery. Real-mod tests remain
controlled-copy opt-ins.

## 16. Rollback and recovery considerations

Keep journals, fingerprints, staged files, backup manifests, and commit ordering. The
candidate carries stable ID/color relationships through normal public reload verification.
Failure before or during commit leaves/restores source IDs intact; rollback and interrupted
save recovery restore map, State, and adjacency bytes exactly.

## 17. Performance considerations

The index is O(province count), not O(max ID). Color-to-ID is expected O(1); ordered
serialization is O(log n) per entry. Do not rebuild it in canvas draw paths.

## 18. Known risks

- **Invalid adjacency references block validation and deletion is rejected,** but adjacency
  editing is intentionally out of scope; users must repair a record externally for now.
- **Performance:** ID lookup is O(province count), not O(max ID); do not reintroduce
  max-ID-sized storage in future consumers.
- **Compatibility UI:** sparse IDs appear as a non-blocking structural warning so users can
  inspect the layout without being told it was repaired.

## 19. Explicit non-goals

This migration does not add adjacency editing, automatic renumbering, a sparse-ID repair
operation, or changes to State syntax/lossless editing. Sparse external IDs are now supported
by normal opening and the unified Save Project flow.
