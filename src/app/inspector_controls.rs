#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width: width.max(0.0),
            height: height.max(0.0),
        }
    }

    pub fn right(self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(self) -> f64 {
        self.y + self.height
    }

    pub fn center(self) -> [f64; 2] {
        [self.x + self.width / 2.0, self.y + self.height / 2.0]
    }

    pub fn contains(self, [x, y]: [f64; 2]) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InspectorControlId {
    Collapse,
    OpenSource,
    CopyPath,
    Search,
    SearchResult(usize),
    Section(usize),
    FooterLeft,
    FooterRight,
    Field(usize),
    Toggle(usize),
    Decrement(InspectorValueTarget),
    Increment(InspectorValueTarget),
    Select(InspectorPickTarget),
    Remove(InspectorPickTarget),
    RemoveValue(InspectorValueTarget),
    Add(InspectorPickTarget),
    MapPick(MapTagPickTarget),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InspectorValueTarget {
    Manpower,
    BuildingsMaxLevelFactor,
    LocalSupplies,
    Resource(usize),
    StateBuilding(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InspectorPickTarget {
    StateCategory,
    Owner,
    Controller,
    Core,
    Claim,
    Resource,
    StateBuilding,
    ProvinceBuilding,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InspectorControlRect {
    pub id: InspectorControlId,
    pub draw: Rect,
    pub hit: Rect,
}

impl InspectorControlRect {
    pub fn hit_test(self, point: [f64; 2]) -> Option<InspectorControlId> {
        self.hit.contains(point).then_some(self.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InspectorControlLayout {
    pub origin: [f64; 2],
    pub width: f64,
    pub body_y: f64,
    pub row_height: f64,
    pub scroll_y: f64,
    pub scale: f64,
    pub collapsed: bool,
}

impl InspectorControlLayout {
    pub fn new(origin: [f64; 2], width: f64, body_y: f64, scroll_y: f64, scale: f64) -> Self {
        Self {
            origin,
            width: width.max(0.0),
            body_y,
            row_height: 19.0,
            scroll_y: scroll_y.max(0.0),
            scale: finite_positive(scale),
            collapsed: false,
        }
    }

    pub fn collapsed(origin: [f64; 2], width: f64, scale: f64) -> Self {
        Self {
            collapsed: true,
            ..Self::new(origin, width, 0.0, 0.0, scale)
        }
    }

    pub fn control(self, id: InspectorControlId, rect: Rect) -> InspectorControlRect {
        let draw = self.scale_rect(rect);
        InspectorControlRect {
            id,
            draw,
            hit: draw,
        }
    }

    pub fn body_control(
        self,
        id: InspectorControlId,
        row: usize,
        x: f64,
        width: f64,
        height: f64,
    ) -> InspectorControlRect {
        self.control(
            id,
            Rect::new(
                x,
                self.body_y + row as f64 * self.row_height - self.scroll_y,
                width,
                height,
            ),
        )
    }

    pub fn numeric_stepper(
        self,
        row: usize,
        target: InspectorValueTarget,
    ) -> [InspectorControlRect; 2] {
        let button_width = 40.0;
        let gap = 6.0;
        let height = (self.row_height - 3.0).max(0.0);
        let plus_x = (self.width - button_width).max(0.0);
        let minus_x = (plus_x - gap - button_width).max(0.0);
        [
            self.body_control(
                InspectorControlId::Decrement(target),
                row,
                minus_x,
                button_width,
                height,
            ),
            self.body_control(
                InspectorControlId::Increment(target),
                row,
                plus_x,
                button_width,
                height,
            ),
        ]
    }

    pub fn hit_test(
        self,
        point: [f64; 2],
        controls: &[InspectorControlRect],
    ) -> Option<InspectorControlId> {
        if self.collapsed {
            return controls.iter().find_map(|control| {
                (control.id == InspectorControlId::Collapse)
                    .then(|| control.hit_test(point))
                    .flatten()
            });
        }
        controls.iter().find_map(|control| control.hit_test(point))
    }

    fn scale_rect(self, rect: Rect) -> Rect {
        Rect::new(
            self.origin[0] + rect.x * self.scale,
            self.origin[1] + rect.y * self.scale,
            rect.width * self.scale,
            rect.height * self.scale,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchablePicker<T> {
    items: Vec<T>,
    open: bool,
    query: String,
    highlighted: usize,
    scroll: usize,
    visible_rows: usize,
}

impl<T> SearchablePicker<T> {
    pub fn new(items: impl IntoIterator<Item = T>) -> Self {
        Self {
            items: items.into_iter().collect(),
            open: false,
            query: String::new(),
            highlighted: 0,
            scroll: 0,
            visible_rows: 8,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn highlighted(&self) -> usize {
        self.highlighted
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }

    pub fn set_visible_rows(&mut self, visible_rows: usize) {
        self.visible_rows = visible_rows.max(1);
        self.keep_highlight_visible();
    }

    pub fn open(&mut self) {
        self.open = true;
        self.highlighted = 0;
        self.scroll = 0;
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.highlighted = 0;
        self.scroll = 0;
    }

    pub fn cancel(&mut self) {
        self.open = false;
        self.query.clear();
        self.highlighted = 0;
        self.scroll = 0;
    }

    pub fn filtered_indices<F>(&self, text: F) -> Vec<usize>
    where
        F: for<'a> Fn(&'a T) -> &'a str,
    {
        let query = self.query.trim().to_ascii_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                (query.is_empty() || text(item).to_ascii_lowercase().contains(&query))
                    .then_some(index)
            })
            .collect()
    }

    pub fn next<F>(&mut self, text: F)
    where
        F: for<'a> Fn(&'a T) -> &'a str,
    {
        let count = self.filtered_indices(text).len();
        if count == 0 {
            self.highlighted = 0;
        } else {
            self.highlighted = (self.highlighted + 1).min(count - 1);
        }
        self.keep_highlight_visible();
    }

    pub fn previous<F>(&mut self, text: F)
    where
        F: for<'a> Fn(&'a T) -> &'a str,
    {
        let count = self.filtered_indices(text).len();
        if count == 0 {
            self.highlighted = 0;
        } else {
            self.highlighted = self.highlighted.saturating_sub(1).min(count - 1);
        }
        self.keep_highlight_visible();
    }

    pub fn page<F>(&mut self, next: bool, text: F)
    where
        F: for<'a> Fn(&'a T) -> &'a str,
    {
        let count = self.filtered_indices(text).len();
        if count == 0 {
            self.highlighted = 0;
        } else if next {
            self.highlighted = (self.highlighted + self.visible_rows).min(count - 1);
        } else {
            self.highlighted = self.highlighted.saturating_sub(self.visible_rows);
        }
        self.keep_highlight_visible();
    }

    pub fn home<F>(&mut self, text: F)
    where
        F: for<'a> Fn(&'a T) -> &'a str,
    {
        if !self.filtered_indices(text).is_empty() {
            self.highlighted = 0;
            self.keep_highlight_visible();
        }
    }

    pub fn end<F>(&mut self, text: F)
    where
        F: for<'a> Fn(&'a T) -> &'a str,
    {
        let count = self.filtered_indices(text).len();
        if count != 0 {
            self.highlighted = count - 1;
            self.keep_highlight_visible();
        }
    }

    pub fn scroll_by<F>(&mut self, delta: isize, text: F)
    where
        F: for<'a> Fn(&'a T) -> &'a str,
    {
        let count = self.filtered_indices(text).len();
        if count == 0 {
            self.scroll = 0;
            self.highlighted = 0;
            return;
        }
        let max_scroll = count.saturating_sub(self.visible_rows);
        self.scroll = self.scroll.saturating_add_signed(delta).min(max_scroll);
        self.highlighted = self.highlighted.clamp(
            self.scroll,
            (self.scroll + self.visible_rows - 1).min(count - 1),
        );
    }

    pub fn confirm<F>(&mut self, text: F) -> Option<&T>
    where
        F: for<'a> Fn(&'a T) -> &'a str,
    {
        let index = *self.filtered_indices(text).get(self.highlighted)?;
        self.open = false;
        self.items.get(index)
    }

    pub fn click<F>(&mut self, visible_row: usize, text: F) -> Option<&T>
    where
        F: for<'a> Fn(&'a T) -> &'a str,
    {
        let highlighted = self.scroll + visible_row;
        let index = *self.filtered_indices(text).get(highlighted)?;
        self.highlighted = highlighted;
        self.open = false;
        self.items.get(index)
    }

    fn keep_highlight_visible(&mut self) {
        if self.highlighted < self.scroll {
            self.scroll = self.highlighted;
        } else if self.highlighted >= self.scroll + self.visible_rows {
            self.scroll = self.highlighted + 1 - self.visible_rows;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MapTagPickTarget {
    Owner,
    Controller,
    Core,
    Claim,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MapTagPicker {
    target: Option<MapTagPickTarget>,
}

impl MapTagPicker {
    pub fn begin(&mut self, target: MapTagPickTarget) {
        self.target = Some(target);
    }

    pub fn cancel(&mut self) {
        self.target = None;
    }

    pub fn active_target(&self) -> Option<MapTagPickTarget> {
        self.target
    }

    pub fn pick(&mut self, tag: Option<impl Into<String>>) -> Option<(MapTagPickTarget, String)> {
        let target = self.target.take()?;
        Some((target, tag?.into()))
    }
}

fn finite_positive(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepper_centers_and_boundaries_are_shared_between_draw_and_hit() {
        let layout = InspectorControlLayout::new([100.0, 20.0], 300.0, 80.0, 0.0, 1.0);
        let [minus, plus] = layout.numeric_stepper(2, InspectorValueTarget::Manpower);

        assert_eq!(minus.draw, minus.hit);
        assert_eq!(plus.draw, plus.hit);
        assert_eq!(minus.draw.center(), [334.0, 146.0]);
        assert_eq!(plus.draw.center(), [380.0, 146.0]);
        assert_eq!(
            minus.hit_test(minus.draw.center()),
            Some(InspectorControlId::Decrement(
                InspectorValueTarget::Manpower
            ))
        );
        assert_eq!(
            plus.hit_test(plus.draw.center()),
            Some(InspectorControlId::Increment(
                InspectorValueTarget::Manpower
            ))
        );
        assert_eq!(minus.hit_test([minus.draw.x - 0.1, minus.draw.y]), None);
        assert_eq!(plus.hit_test([plus.draw.right(), plus.draw.y + 1.0]), None);
    }

    #[test]
    fn body_controls_apply_scroll_and_scale() {
        let layout = InspectorControlLayout::new([10.0, 5.0], 200.0, 100.0, 19.0, 2.0);
        let control = layout.body_control(InspectorControlId::Field(3), 3, 4.0, 50.0, 10.0);

        assert_eq!(control.draw, Rect::new(18.0, 281.0, 100.0, 20.0));
        assert_eq!(
            control.hit_test([18.0, 281.0]),
            Some(InspectorControlId::Field(3))
        );
        assert_eq!(control.hit_test([118.0, 281.0]), None);
    }

    #[test]
    fn row_controls_stay_inside_the_inspector_and_share_draw_hit_rects() {
        let layout = InspectorControlLayout::new([8.0, 196.0], 404.0, 0.0, 0.0, 1.25);
        let controls = [
            layout.numeric_stepper(3, InspectorValueTarget::Resource(0))[0],
            layout.numeric_stepper(3, InspectorValueTarget::Resource(0))[1],
            layout.body_control(
                InspectorControlId::RemoveValue(InspectorValueTarget::Resource(0)),
                3,
                231.0,
                75.0,
                16.0,
            ),
        ];

        for control in controls {
            assert_eq!(control.draw, control.hit);
            assert!(control.draw.x >= layout.origin[0]);
            assert!(control.draw.right() <= layout.origin[0] + layout.width * layout.scale);
        }
    }

    #[test]
    fn collapsed_layout_only_hits_collapse_control() {
        let layout = InspectorControlLayout::collapsed([500.0, 20.0], 48.0, 1.5);
        let collapse = layout.control(
            InspectorControlId::Collapse,
            Rect::new(0.0, 0.0, 48.0, 32.0),
        );
        let field = layout.control(
            InspectorControlId::Field(0),
            Rect::new(0.0, 40.0, 48.0, 20.0),
        );

        assert_eq!(
            layout.hit_test(collapse.draw.center(), &[collapse, field]),
            Some(InspectorControlId::Collapse)
        );
        assert_eq!(
            layout.hit_test(field.draw.center(), &[collapse, field]),
            None
        );
    }

    #[test]
    fn picker_opens_searches_selects_and_cancels_without_mutating_items() {
        let mut picker = SearchablePicker::new([
            "ABC".to_owned(),
            "custom_value".to_owned(),
            "DEF".to_owned(),
        ]);

        picker.open();
        picker.set_query("CUSTOM");
        assert_eq!(picker.filtered_indices(String::as_str), vec![1]);
        assert_eq!(
            picker.confirm(String::as_str),
            Some(&"custom_value".to_owned())
        );
        assert_eq!(picker.items()[1], "custom_value");
        assert!(!picker.is_open());

        picker.open();
        picker.set_query("abc");
        picker.cancel();
        assert!(!picker.is_open());
        assert_eq!(picker.query(), "");
        assert_eq!(picker.items()[0], "ABC");
    }

    #[test]
    fn picker_keyboard_and_click_selection_respect_scroll() {
        let mut picker = SearchablePicker::new(["alpha", "bravo", "charlie", "delta", "echo"]);
        picker.set_visible_rows(2);
        picker.open();

        picker.next(|value| *value);
        picker.next(|value| *value);
        assert_eq!(picker.highlighted(), 2);
        assert_eq!(picker.scroll(), 1);
        picker.previous(|value| *value);
        assert_eq!(picker.highlighted(), 1);
        picker.scroll_by(2, |value| *value);
        assert_eq!(picker.scroll(), 3);
        assert_eq!(picker.click(1, |value| *value), Some(&"echo"));
        assert!(!picker.is_open());
    }

    #[test]
    fn picker_page_home_end_keep_keyboard_selection_visible() {
        let mut picker = SearchablePicker::new(0..20);
        picker.set_visible_rows(4);
        picker.open();
        picker.page(true, |_| "");
        assert_eq!((picker.highlighted(), picker.scroll()), (4, 1));
        picker.end(|_| "");
        assert_eq!((picker.highlighted(), picker.scroll()), (19, 16));
        picker.home(|_| "");
        assert_eq!((picker.highlighted(), picker.scroll()), (0, 0));
    }

    #[test]
    fn map_tag_picker_preserves_selected_custom_tag() {
        let mut picker = MapTagPicker::default();
        picker.begin(MapTagPickTarget::Owner);

        assert_eq!(picker.active_target(), Some(MapTagPickTarget::Owner));
        assert_eq!(
            picker.pick(Some("XYZ")),
            Some((MapTagPickTarget::Owner, "XYZ".to_owned()))
        );
        assert_eq!(picker.active_target(), None);

        picker.begin(MapTagPickTarget::Controller);
        picker.cancel();
        assert_eq!(picker.pick(Some("ABC")), None);
    }
}
