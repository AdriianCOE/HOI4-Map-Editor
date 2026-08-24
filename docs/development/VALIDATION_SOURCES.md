# Validation Sources and Certainty

| Rule family | Editor implementation | Reference use | Current policy | Engine certainty |
| --- | --- | --- | --- | --- |
| `province.x_crossing`, geometry | Independent Rust map analysis | Herbix world-map province BMP warnings | Compatibility diagnostic, non-blocking | Reference-supported |
| Definition identity/bitmap consistency | Independent Rust CSV/BMP checks | Herbix province map loader | Sparse IDs and small maps are valid editor inputs | Mixed; sparse policy is deliberate |
| State/VP/building membership | Parsed State documents and cross-domain validator | Herbix States loader | Current-map state data only | Reference-supported, no bookmark policy |
| Adjacency | Parsed `adjacencies.csv` validator | Herbix adjacency loader | Explicit endpoint/through diagnostics | Reference-supported |
| River topology | Independent indexed bitmap analysis | Herbix river loader | Conservative loop diagnostics | Partial; marker semantics need engine tests |
| Railway/supply | Parsed read-only logistics models | Herbix railway loader | Stricter malformed-input handling may differ | Partial; route constraints need engine tests |
| Strategic Regions | Source-aware `map/strategicregions/*.txt` parser and cross-domain checks | Herbix/world-map format guidance | Sparse IDs, partial-load cascade suppression and non-blocking Save diagnostics | Format/reference-supported; dense sequential-ID engine behavior remains unclaimed |

Open `needs_hoi4_engine_test` questions: multiple DLC collision precedence;
river marker `1`/`2` direction semantics; real-map loop legality and marker
constraints; duplicate supply nodes; repeated railway provinces; and special
railway adjacency rules. Do not mark these engine-confirmed solely from a
differential match.

Strategic Region duplicate Province IDs within one region and any engine
requirement that IDs be sequential remain `needs_hoi4_engine_test`. The editor
retains raw membership and accepts sparse IDs for safe inspection; this is not
an engine compatibility claim.
