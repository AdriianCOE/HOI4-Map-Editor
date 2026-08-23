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

Dependency direction is `map/project/state -> presentation/problems_ui -> Canvas -> App/UI`.
Neither presentation resource nor Problems UI code imports `Canvas` or `App`.
