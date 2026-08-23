# Map presentation architecture

`Canvas` remains the frame and interaction orchestrator. It owns the map,
current `StateEditSession`, camera, selection/tools, dialogs, Save workflow, and
the UI command boundary. It supplies borrowed inputs to `PresentationRuntime`.

`PresentationRuntime` owns only derived, read-only presentation resources:

- `MapPresentationModel` and the Category, Manpower, and DMZ GPU textures;
- the validation-derived Problems overlay model; and
- generation/revision invalidation for those resources.

`problems_ui` owns the Project Problems view state and its read-only
interpretation of validation diagnostics: filtering, ordering, row summaries,
technical details, selected action sequencing, and conversion to request
models. `Canvas` renders those models and is the sole executor of requests: it
keeps camera focus, State-session selection, modal transitions, alerts, and
platform open/reveal/copy handling at the application boundary.

`save_ui` is intentionally separate from `PresentationRuntime`. It derives a
read-only `SaveReviewModel` and dialog presentation from the existing project
save plan, combined validation, and transaction report. Its controller emits
only commit, Problems, and integrity-review requests. Candidate construction,
Auto Coastal, source and revision freshness checks, validation, backup/journal,
atomic commit, rollback, and recovery remain owned by `project` save modules;
Canvas/App execute the UI requests and retain task lifecycle ownership. Save UI
state is reset on project generation replacement and its commit request is
one-shot until the save engine either starts or rejects it.

`project_lifecycle` owns the synchronous project/base-game picker state and
the request/effect transition from a selected location to isolated candidate
discovery. `ProjectOpenCandidate` identifies a normal mod, the existing
province-only degradation path, or a legacy map; it does not load or activate
live Canvas state. App executes native dialogs and candidate loading, while
`Canvas` remains the accepted, fully loaded candidate and owns the coherent
live-state replacement. All fallible discovery and loading completes before
`replace_project_canvas` advances the generation. That hook binds the new
Canvas generation, then resets its PresentationRuntime, Problems controller,
Save UI controller, round-trip snapshot, and navigation marker through their
existing public lifecycle APIs before the old Canvas is dropped. A failed or
canceled candidate restores the prior lifecycle generation without touching
the live Canvas, its source graph, or its caches.

Political and Resources retain their focused domain parsers and existing Canvas
draw paths. Their source-aware catalog/icon caches are owned by the runtime and
remain generation-bound; a State revision does not parse either source again.

## Invalidation matrix

| Event | Map presentation | Category/Manpower/DMZ GPU | Problems | Political/Resources |
| --- | --- | --- | --- | --- |
| Project replacement | drop | drop | drop | drop with Canvas |
| State revision | drop | drop | retained until validation refresh | existing view-specific refresh only |
| Validation refresh | retain | retain | rebuild | retain |
| View switch | lazy build if needed | lazy build if needed | retain | existing lazy behavior |

The runtime never owns authoritative editable State data. A presentation model
is rebuilt from the active session revision and cannot survive a generation
replacement. PNG composition uses the same immutable model and overlay order
(DMZ, Resources, Victory Points, Problems), but retains its CPU renderer.

Map presentation direction is `map/project/state -> presentation/problems_ui -> Canvas -> App/UI`.
Project Save review direction is `project save engine -> SaveReviewModel -> SaveUiController -> Canvas/App`.
Project opening direction is `project discovery -> ProjectOpenCandidate -> ProjectLifecycleController -> App dialogs/loading -> Canvas activation`.
Neither presentation resource, Problems UI, Save UI, nor project lifecycle imports `Canvas` or `App`.
