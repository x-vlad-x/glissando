//! Colours and metrics. The palettes are our own, tuned to sit well next to
//! Phosh/libadwaita while following Material-style tonal contrast.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba(pub f64, pub f64, pub f64, pub f64);

impl Rgba {
    pub const fn hex(v: u32) -> Rgba {
        Rgba(((v >> 16) & 0xff) as f64 / 255.0, ((v >> 8) & 0xff) as f64 / 255.0, (v & 0xff) as f64 / 255.0, 1.0)
    }
    pub const fn alpha(self, a: f64) -> Rgba {
        Rgba(self.0, self.1, self.2, a)
    }
    pub fn set(&self, cr: &cairo::Context) {
        cr.set_source_rgba(self.0, self.1, self.2, self.3);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub dark: bool,
    pub background: Rgba,
    pub key: Rgba,
    pub key_pressed: Rgba,
    pub key_function: Rgba,
    pub key_function_pressed: Rgba,
    pub accent: Rgba,
    pub accent_pressed: Rgba,
    pub on_accent: Rgba,
    pub label: Rgba,
    pub hint: Rgba,
    pub strip_text: Rgba,
    pub strip_divider: Rgba,
    pub popup: Rgba,
    pub popup_selected: Rgba,
    pub on_popup_selected: Rgba,
    pub shadow: Rgba,
    pub trail: Rgba,
    pub delete_highlight: Rgba,
    /// Borders around keys (high contrast mode).
    pub key_border: Option<Rgba>,
}

impl Theme {
    pub fn light() -> Theme {
        Theme {
            dark: false,
            background: Rgba::hex(0xe8ebf1),
            key: Rgba::hex(0xffffff),
            key_pressed: Rgba::hex(0xc9d1de),
            key_function: Rgba::hex(0xc8d2e3),
            key_function_pressed: Rgba::hex(0xa9b6cc),
            accent: Rgba::hex(0x3461c7),
            accent_pressed: Rgba::hex(0x274c9e),
            on_accent: Rgba::hex(0xffffff),
            label: Rgba::hex(0x1a1c20),
            hint: Rgba::hex(0x6a707c),
            strip_text: Rgba::hex(0x22252b),
            strip_divider: Rgba::hex(0xc4c9d2),
            popup: Rgba::hex(0xffffff),
            popup_selected: Rgba::hex(0x3461c7),
            on_popup_selected: Rgba::hex(0xffffff),
            shadow: Rgba(0.0, 0.0, 0.0, 0.18),
            trail: Rgba::hex(0x3461c7),
            delete_highlight: Rgba::hex(0xd93025),
            key_border: None,
        }
    }

    pub fn dark() -> Theme {
        Theme {
            dark: true,
            background: Rgba::hex(0x1b1d21),
            key: Rgba::hex(0x3a3d44),
            key_pressed: Rgba::hex(0x575b64),
            key_function: Rgba::hex(0x2c2f35),
            key_function_pressed: Rgba::hex(0x474b53),
            accent: Rgba::hex(0xa7c3f5),
            accent_pressed: Rgba::hex(0x86a6dd),
            on_accent: Rgba::hex(0x0d2a5c),
            label: Rgba::hex(0xe8eaee),
            hint: Rgba::hex(0xa3a8b3),
            strip_text: Rgba::hex(0xe1e3e8),
            strip_divider: Rgba::hex(0x40444c),
            popup: Rgba::hex(0x464a52),
            popup_selected: Rgba::hex(0xa7c3f5),
            on_popup_selected: Rgba::hex(0x0d2a5c),
            shadow: Rgba(0.0, 0.0, 0.0, 0.45),
            trail: Rgba::hex(0xa7c3f5),
            delete_highlight: Rgba::hex(0xf28b82),
            key_border: None,
        }
    }

    pub fn with_borders(mut self) -> Theme {
        self.key_border = Some(if self.dark { Rgba::hex(0x6b707a) } else { Rgba::hex(0x9aa1ad) });
        self
    }
}

/// Size related settings in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub strip_height: f32,
    pub row_height: f32,
    pub radius: f32,
    pub label_size: f32,
    pub small_label_size: f32,
    pub hint_size: f32,
    pub strip_font_size: f32,
    pub trail_width: f32,
}

impl Metrics {
    /// `height_scale` is the user's keyboard height preference (0.8..1.3),
    /// `font_scale` the label size preference.
    pub fn new(landscape: bool, height_scale: f32, font_scale: f32) -> Metrics {
        let row = if landscape { 37.0 } else { 52.0 } * height_scale;
        Metrics {
            strip_height: if landscape { 36.0 } else { 44.0 },
            row_height: row,
            radius: 7.0,
            label_size: (row * 0.42).clamp(15.0, 25.0) * font_scale,
            small_label_size: 14.0 * font_scale,
            hint_size: 10.0 * font_scale,
            strip_font_size: 16.0 * font_scale,
            trail_width: 5.0,
        }
    }
}
