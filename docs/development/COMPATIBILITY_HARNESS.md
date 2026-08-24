# Differential Compatibility Harness

This is development-only infrastructure for comparing normalized map findings from
HOI4 Map Editor with a `hoi4modutilities` reference snapshot. It is not a claim
that either tool proves Hearts of Iron IV engine behavior.

## Why snapshots

The inspected `hoi4modutilities` 0.16.0 world-map loader is an activated VS Code
extension surface: it uses VS Code configuration, loader sessions, and webview
world-map plumbing. It does not expose a stable standalone CLI. The editor
therefore does not execute it automatically, alter its checkout, add Node as a
dependency, or require it for normal builds, tests, release builds, or CI.

An optional external adapter may invoke a locally installed reference checkout,
then write only normalized TOML snapshot metadata. The Rust harness compares
those snapshots repeatedly without Node. `reference_tool` and
`reference_version` must remain `hoi4modutilities` and `0.16.0` unless the
normalization contract is intentionally reviewed and updated.

## Developer entry point

Run from the repository root:

```powershell
./scripts/check-compatibility.ps1
./scripts/check-compatibility.ps1 -ReferenceRoot C:\path\to\hoi4modutilities-master
```

This verifies deterministic normalization, matching, policy approvals, stale
snapshots, version drift, and unsupported-domain behavior. It deliberately does
not launch VS Code. A missing reference checkout is a clear optional condition,
not a test failure.

Use `tools/compatibility_cases.example.toml` as the committed schema example and
copy it to the ignored `tools/compatibility_cases.local.toml` for Vanilla, Azarya,
or another local mod. Do not commit absolute paths, HOI4 assets, or raw source
contents.

## Snapshot contract

Snapshots are TOML and contain only:

- snapshot schema and normalization versions;
- reference tool/version/source identifier;
- deterministic SHA-256 fingerprint of selected input paths and bytes;
- normalized findings: rule, severity class, province/state IDs, coordinates,
  logical source, and an optional structured detail key.

The comparison rejects schema/tool/version mismatch and reports
`StaleReferenceSnapshot` when the selected inputs no longer match. Physical
paths are relativized to the chosen root and normalized with `/` separators.
JSON is available for machine-readable comparison reports; the human report is
small, sorted, and grouped by comparison category.

## Rules and review

Rule IDs describe semantics rather than human message text, for example
`province.x_crossing`, `state.vp_outside_state`, `adjacency.missing_through`,
`river.no_source`, `railway.non_adjacent`, and `supply.missing_province`.
The external adapter must emit a rule from structured loader context, not infer
one from localized warning text.

Differences are `match`, `editor_only`, `reference_only`, `different_details`,
`unsupported_by_us`, `unsupported_by_reference`, `policy_difference`, or
`needs_review`. Review labels are separate: `our_bug`, `reference_limitation`,
`different_policy`, `false_positive`, `needs_hoi4_engine_test`,
`expected_difference`, and `unreviewed`.

Expected differences are narrow records keyed by case, rule, province IDs,
state IDs, and coordinate. They must never use broad rule-family wildcards.
Normal report mode does not fail on an unreviewed difference; strict baseline
workflows should require every non-match to have a reviewed narrow record.

Current intentional policy differences include custom/small dimensions, sparse
province IDs, no arbitrary province/state/strategic-region maximum, different
Save blocking semantics, conservative river-loop reporting, and stricter
malformed logistics parsing in some cases.

## Coverage and limits

Synthetic cases should cover valid tiny maps, X crossings, disconnected and
one-pixel provinces, definition identity errors, terrain/continent errors, bad
adjacencies/rivers/railways/supply nodes, and sparse IDs. Real projects are
optional local cases only.

The parser corpus should also exercise harmless whitespace and comments, decimal
and signed coordinates, quoted source values, repeated fields, malformed
records, and missing referenced files. Keep each fixture small and give it a
single declared expectation so a difference is reviewable rather than a broad
tool-level verdict.

Strategic Region rules now participate in normalized comparison for the
read-only domain: duplicate region IDs, missing/duplicate/unassigned Province
membership, State splits, and undefined naval terrain where supplied by a
reference adapter. The internal `ProvinceReferenceIndex` remains a derived
editor structure rather than a direct comparison target.

Multiple DLC precedence, river marker direction/loop legality, duplicate supply
nodes, repeated railway provinces, and special railway adjacency rules remain
`needs_hoi4_engine_test`. Agreement with Herbix is reference evidence, not an
engine claim.
