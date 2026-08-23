/// Identity assigned to a fully loaded project context.
///
/// It deliberately does not track edit revisions: it distinguishes one loaded
/// map/mod from another and is the common cache boundary for presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ProjectGeneration(pub(crate) u64);
