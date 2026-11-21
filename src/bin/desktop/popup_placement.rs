use eframe::egui;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct PopupPlacement {
    pub(super) above: bool,
    pub(super) extra_height: f32,
}

impl PopupPlacement {
    pub(super) fn fit(toolbar: egui::Rect, work_area: Option<egui::Rect>, requested: f32) -> Self {
        let requested = requested.ceil().max(0.0);
        let Some(work_area) = work_area.filter(|_| requested > 0.0) else {
            return Self {
                above: false,
                extra_height: requested,
            };
        };
        let below = (work_area.bottom() - toolbar.bottom()).floor().max(0.0);
        let above = (toolbar.top() - work_area.top()).floor().max(0.0);
        let open_above = below < requested && above > below;
        Self {
            above: open_above,
            extra_height: requested.min(if open_above { above } else { below }),
        }
    }

    pub(super) fn toolbar_offset(self) -> f32 {
        if self.above { self.extra_height } else { 0.0 }
    }
}
