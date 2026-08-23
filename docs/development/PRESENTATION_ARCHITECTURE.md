# Map presentation architecture

`Canvas` remains the frame and interaction orchestrator. It owns the map,
current `StateEditSession`, camera, selection/tools, dialogs, Save workflow, and
the UI command boundary. It supplies borrowed inputs to `PresentationRuntime`.

`PresentationRuntime` owns only derived, read-only presentation resources:

- `MapPresentationModel` and the Category, Manpower, and DMZ GPU textures;
- the validation-derived Problems overlay model; and
- generation/revision invalidation for those resources.

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

Dependency direction is `map/project/state -> presentation -> Canvas -> App/UI`.
Presentation does not import `Canvas` or `App`.
