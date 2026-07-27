#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateBrushMode {
  AssignToTarget,
  Unassign,
}

impl StateBrushMode {
  pub fn label(self) -> &'static str {
    match self {
      Self::AssignToTarget => "Assign to target",
      Self::Unassign => "Unassign",
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrushProvinceClassification {
  Selectable,
  NoOp,
  IgnoredNonLand,
  BlockedAmbiguous,
  BlockedInvalidState,
  Unknown,
}

pub fn sample_segment(
  start: [f64; 2],
  end: [f64; 2],
  maximum_step: f64,
  dimensions: [u32; 2],
) -> Vec<[u32; 2]> {
  if dimensions.contains(&0) {
    return Vec::new();
  }
  let maximum_step = maximum_step.max(1.0);
  let max_x = dimensions[0].saturating_sub(1) as f64;
  let max_y = dimensions[1].saturating_sub(1) as f64;
  let start = [start[0].clamp(0.0, max_x), start[1].clamp(0.0, max_y)];
  let end = [end[0].clamp(0.0, max_x), end[1].clamp(0.0, max_y)];
  let dx = end[0] - start[0];
  let dy = end[1] - start[1];
  let distance = (dx * dx + dy * dy).sqrt();
  let steps = (distance / maximum_step).ceil().max(1.0) as u32;
  let mut points = Vec::with_capacity(steps as usize + 1);
  for step in 0..=steps {
    let t = step as f64 / steps as f64;
    let x = (start[0] + dx * t).round().clamp(0.0, max_x) as u32;
    let y = (start[1] + dy * t).round().clamp(0.0, max_y) as u32;
    if points.last() != Some(&[x, y]) {
      points.push([x, y]);
    }
  }
  points
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn segment_sampling_deduplicates_and_clamps() {
    assert_eq!(sample_segment([0.0, 0.0], [3.0, 0.0], 1.0, [10, 10]).len(), 4);
    assert_eq!(sample_segment([-5.0, 0.0], [20.0, 0.0], 10.0, [4, 4]), vec![[0, 0], [3, 0]]);
    assert_eq!(sample_segment([0.0, 0.0], [4096.0, 0.0], 1.0, [5000, 1]).len(), 4097);
  }
}
