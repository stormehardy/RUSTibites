//! The stomach in the Biology panel: the liquid and the food in the picture of the internals, and the "Stomach Details" pop-up that shows while the
//! pointer rests on the stomach (`BiologyPanel.OnPointerMove` reads the pixel under the pointer from the map of the organs).
//!
//! Numbers: the pop-up's geometry is the prefab's (`StomachDetailPopUp`, read from the native capture and the prefab's layout groups); the rows of
//! the contents are the `StomachContentElement` prefab. The pop-up hangs to the right of the panel's `Section` (anchor right, 9 away).
//! Not modelled: the physics that moves the pellets about, the slider scale that grows with the value (the scale is the one the prefab starts with),
//! and the pop-ups of the other organs.

use std::sync::Arc;

use bibites_core::math::Vec2;

use crate::app::App;
use crate::information_panel::{Face, Measure, ROBOTO_BOLD};
use crate::internals::{self, Params, StomachInfo, StomachShape};
use crate::text_sdf::SdfFonts;
use crate::ui::{FontRole, InspectorTab, Rgba, UiAssets, UiDraw, UiFrame};

/// The picture of the internals in the 1920 x 1080 reference: the `internals preview` node of the Biology panel.
pub const PICTURE: [f32; 4] = [25.0, 350.34, 305.0, 195.0];

/// The picture's rectangle for a living or a dead bibite (the dead one's panel is 7.84 shorter above the picture: the Dead Line in, the Energy row out).
pub fn picture(dead: bool) -> [f32; 4] {
    let shift = if dead {
        crate::biology_popups::DEAD_LINE_SHIFT - crate::biology_popups::ENERGY_ROW_PITCH
    } else {
        0.0
    };
    [PICTURE[0], PICTURE[1] + shift, PICTURE[2], PICTURE[3]]
}

/// The ink the white `LiberationSans SDF` text of these panels is drawn in (the material's face colour is black, so the white vertex colour gives dark).
const INK: Rgba = [33, 34, 33, 255];
const BLACK: Rgba = [0, 0, 0, 255];
const WHITE: Rgba = [255, 255, 255, 255];

/// `LiberationSans SDF` set bold, as the pop-up's labels are.
const LIBERATION_BOLD: Face = Face {
    name: "LiberationSans SDF",
    role: FontRole::Bold,
    line: 1.117_5,
};

/// The organ under a point of the picture (the ids of the map: 2 is the stomach).
pub fn organ_at(panel: &crate::app::SelectionPanel, reference: Vec2) -> Option<u8> {
    let params = panel.internals?;
    let picture = picture(panel.dead);
    let u = (reference.x - picture[0]) / picture[2];
    let v = 1.0 - (reference.y - picture[1]) / picture[3];
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return None;
    }
    let x = ((u * internals::WIDTH as f32) as usize).min(internals::WIDTH - 1);
    let y = ((v * internals::HEIGHT as f32) as usize).min(internals::HEIGHT - 1);
    let map = internals::sections_cached(&params);
    Some(map[x + y * internals::WIDTH])
}

/// Seconds since the first call (`Time.unscaledTime` for the acid's drift).
fn unscaled_time() -> f32 {
    // The tests compare frames: the acid stands still in them.
    if cfg!(test) {
        return 0.0;
    }
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f32()
}

/// The pellets and the acid over the preview (`StomachContainer`, then `StomachAcid`): `pos` and `size` are the picture's rectangle in pixels.
pub fn draw_overlays(
    frame: &mut UiFrame,
    ui: &UiAssets,
    pos: Vec2,
    size: Vec2,
    params: &Params,
    stomach: &StomachInfo,
    seed: u64,
    running: bool,
) {
    let k = size.x / PICTURE[2];
    let shape = StomachShape::of(params);
    for pellet in crate::stomach_sim::pellets_now(seed, stomach, &shape, running) {
        let Some(texture) = ui.texture_of_alias(internals::pellet_sprite(pellet.material, pellet.amount)) else {
            continue;
        };
        let half = pellet.size / 2.0;
        // `turned` holds the corners, so the turned square is bigger by the same factor.
        if let Some(art) = ui.texture_pixels(texture).and_then(|upright| {
            let turned = internals::turned_texture(ui, texture, pellet.rotation)?;
            Some((turned.width as f32 / (upright.width.max(1) * internals::TURN_SS) as f32, turned))
        }) {
            let side = pellet.size * art.0;
            frame.draws.push(UiDraw::Pixels {
                texture: art.1,
                pos: Vec2::new(pos.x + (pellet.pos[0] - side / 2.0) * k, pos.y + (pellet.pos[1] - side / 2.0) * k),
                size: Vec2::new(side * k, side * k),
            });
            continue;
        }
        frame.draws.push(UiDraw::Image {
            texture,
            pos: Vec2::new(pos.x + (pellet.pos[0] - half) * k, pos.y + (pellet.pos[1] - half) * k),
            size: Vec2::new(pellet.size * k, pellet.size * k),
            tint: [255, 255, 255, 255],
        });
    }
    let acid = internals::acid_picture(&shape, stomach.acid, unscaled_time());
    frame.draws.push(UiDraw::Pixels {
        texture: Arc::new(acid),
        pos,
        size,
    });
}

/// The eggs of the womb in the picture (`EggContainer`), over the acid.
pub fn draw_eggs(frame: &mut UiFrame, pos: Vec2, size: Vec2, params: &Params, organs: &crate::biology_popups::OrganInfo, seed: u64) {
    let k = size.x / PICTURE[2];
    let shape = StomachShape::of(params);
    let color = [params.color[0], params.color[1], params.color[2]];
    let eggs = internals::eggs_in_womb(organs.egg_total_progress, organs.egg_progress, organs.egg_size, organs.womb_area, &shape, seed);
    let last = eggs.len().saturating_sub(1);
    for (index, egg) in eggs.into_iter().enumerate() {
        let (sheet, width) = internals::egg_sheet(egg.amount);
        // The egg that is still growing glows while the brain asks for eggs (`activeEgg.SetGrowing`).
        let glowing = index == last && organs.egg_glow != 0.0;
        let texture = if glowing {
            internals::growing_egg_texture(sheet, color, egg.rotation, organs.egg_glow, unscaled_time())
        } else {
            internals::egg_texture(sheet, color, egg.rotation)
        };
        let Some(texture) = texture else {
            continue;
        };
        // The turned picture is bigger than the upright one (it holds the corners): its pixels keep their size.
        let side = texture.width as f32 / internals::TURN_SS as f32 * egg.scale;
        let _ = width;
        frame.draws.push(UiDraw::Pixels {
            texture,
            pos: Vec2::new(pos.x + (egg.pos[0] - side / 2.0) * k, pos.y + (egg.pos[1] - side / 2.0) * k),
            size: Vec2::new(side * k, side * k),
        });
    }
}

pub(crate) struct Painter<'a> {
    pub(crate) seed: u64,
    pub(crate) frame: &'a mut UiFrame,
    pub(crate) ui: &'a UiAssets,
    pub(crate) s: f32,
    pub(crate) measure: Measure<'a>,
}

impl<'a> Painter<'a> {
    pub(crate) fn new(frame: &'a mut UiFrame, ui: &'a UiAssets, s: f32, measure: Measure<'a>, seed: u64) -> Self {
        Self { seed, frame, ui, s, measure }
    }
}

impl Painter<'_> {
    fn at(&self, x: f32, y: f32) -> Vec2 {
        Vec2::new(x * self.s, y * self.s)
    }

    pub(crate) fn slice(&mut self, sprite: &str, border: [f32; 4], scale: f32, rect: [f32; 4], tint: Rgba) {
        let (pos, size) = (self.at(rect[0], rect[1]), self.at(rect[2], rect[3]));
        match self.ui.texture_of_alias(sprite) {
            Some(texture) => self.frame.draws.push(UiDraw::NineSlice {
                texture,
                border,
                dest_scale: scale,
                pos,
                size,
                tint,
            }),
            None => self.frame.rect(pos, size, tint),
        }
    }

    pub(crate) fn icon(&mut self, sprite: &str, rect: [f32; 4]) {
        if let Some(texture) = self.ui.texture_of_alias(sprite) {
            let (pos, size) = (self.at(rect[0], rect[1]), self.at(rect[2], rect[3]));
            self.frame.draws.push(UiDraw::Image {
                texture,
                pos,
                size,
                tint: [255, 255, 255, 255],
            });
        }
    }

    /// A text in a box, middle-aligned vertically; `align` is -1 left, 0 centre, 1 right.
    pub(crate) fn text(&mut self, face: Face, text: &str, rect: [f32; 4], align: i8, px: f32, color: Rgba) {
        if text.is_empty() {
            return;
        }
        let width = self.measure.width(face, text, px);
        let line = self.measure.height(face, px);
        let x = match align {
            -1 => rect[0],
            0 => rect[0] + (rect[2] - width) / 2.0,
            _ => rect[0] + rect[2] - width,
        };
        let y = rect[1] + (rect[3] - line) / 2.0;
        self.frame.draws.push(UiDraw::Text {
            font: Some(face.name.to_owned()),
            text: text.to_owned(),
            pos: self.at(x, y),
            px: px * self.s,
            color,
            role: face.role,
        });
    }

    /// `InfoSliderHandle`: the track (`tiled_bars_1`) and the fill (`tiled_bars_0`), 2 pixels per sprite pixel; the fill is `ScaledX(value)` long, which is
    /// two caps of 8 and the rest in proportion to `value / scale`.
    fn slider(&mut self, rect: [f32; 4], ratio: f32, color: Rgba) {
        self.slice("tiled_bars_1", [4.0, 5.0, 4.0, 5.0], 2.0, rect, [255, 255, 255, 255]);
        let fill = 16.0 + ratio.clamp(0.0, 1.0) * (rect[2] - 16.0);
        self.slice("tiled_bars_0", [4.0, 5.0, 4.0, 5.0], 2.0, [rect[0], rect[1], fill, rect[3]], color);
    }

    /// `RateSliderHandle` on a track of the stomach's elements.
    fn rate_slider(&mut self, rect: [f32; 4], value: f32, scale: f32, text: &str) {
        let pl = 1.0 / 0.5_f32.max(8.0 / rect[2].max(1.0)).max(12.0 / rect[3].max(1.0));
        self.rate_slider_pl(rect, value, scale, pl, Some(text), 14.3, 11.9);
    }

    /// `RateSliderHandle`: a track with its centre line; the fill goes right (green) or left (red) from the centre by half of `ScaledX`, and the value is
    /// written on the empty side of the centre (the texts are children of the zero-width `CenterPoint`, 5 pixel lengths in from it).
    pub(crate) fn rate_slider_pl(&mut self, rect: [f32; 4], value: f32, scale: f32, pl: f32, text: Option<&str>, positive_px: f32, negative_px: f32) {
        let border = [4.0, 5.0, 4.0, 5.0];
        self.slice("tiled_bars_1", border, pl, rect, [255, 255, 255, 255]);
        let centre = rect[0] + rect[2] / 2.0;
        let ratio = if scale > 0.0 { (value / scale).abs().min(1.0) } else { 0.0 };
        let reach = (8.0 * pl + ratio * (rect[2] - 8.0 * pl)) / 2.0;
        if value > 0.0 {
            self.slice("tiled_bars_2", [0.0, 5.0, 4.0, 5.0], pl, [centre, rect[1], reach, rect[3]], [0, 255, 30, 255]);
        } else if value < 0.0 {
            self.slice("tiled_bars_3", [4.0, 5.0, 0.0, 5.0], pl, [centre - reach, rect[1], reach, rect[3]], [255, 6, 0, 255]);
        }
        self.frame.rect(self.at(centre - pl, rect[1]), self.at(2.0 * pl, rect[3]), [38, 38, 38, 255]);
        if let Some(text) = text {
            if value > 0.0 {
                self.text(ROBOTO_BOLD, text, [centre - 65.0, rect[1], 65.0 - 5.0 * pl, rect[3]], 1, positive_px, BLACK);
            } else if value < 0.0 {
                self.text(ROBOTO_BOLD, text, [centre + 5.0 * pl, rect[1], 65.0 - 5.0 * pl, rect[3]], -1, negative_px, BLACK);
            }
        }
    }

    /// A flat rectangle (reference pixels).
    pub(crate) fn frame_rect(&mut self, rect: [f32; 4], color: Rgba) {
        self.frame.rect(self.at(rect[0], rect[1]), self.at(rect[2], rect[3]), color);
    }
}

thread_local! {
    static SCALES: std::cell::RefCell<std::collections::HashMap<&'static str, (u64, f32)>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// The scale of a slider that raises its own scale (`InfoSliderHandle.OnValueChange`: the prefabs' buffer factor and value are 0, so the scale becomes the
/// value as soon as the value reaches it, and stays). It starts at 1 for every bibite that is selected (`InitSliderHandle(1)`). The digestion and energy
/// sliders of every material share one scale (`OnScaleRaised` updates all of them).
fn remember_scale(key: &'static str, seed: u64, value: f32) -> f32 {
    remember_scale_from(key, seed, value, 1.0)
}

pub(crate) fn remember_scale_pub(key: &'static str, seed: u64, value: f32, base: f32) -> f32 {
    remember_scale_from(key, seed, value, base)
}

fn remember_scale_from(key: &'static str, seed: u64, value: f32, base: f32) -> f32 {
    SCALES.with(|cell| {
        let mut map = cell.borrow_mut();
        let entry = map.entry(key).or_insert((seed, base));
        if entry.0 != seed {
            *entry = (seed, base);
        }
        entry.1 = entry.1.max(value.abs());
        entry.1
    })
}

/// The height of the content part of the pop-up for `n` materials.
fn contents_height(n: usize) -> f32 {
    if n == 0 {
        0.0
    } else {
        130.0 * n as f32 + 5.0 * (n - 1) as f32
    }
}

/// The organ whose pop-up shows: the Biology panel is open on a bibite and the pointer is on a cell of that organ.
pub fn hovered_organ(app: &App, reference: Vec2) -> Option<crate::biology_popups::Organ> {
    if !(app.inspector_open && app.inspector_tab == InspectorTab::Biology) {
        return None;
    }
    let panel = app.selection.as_ref()?;
    panel.stomach_info.as_ref()?;
    crate::biology_popups::organ_of_id(organ_at(panel, reference)?)
}

/// Whether the stomach's pop-up shows.
pub fn popup_shows(app: &App, reference: Vec2) -> bool {
    hovered_organ(app, reference) == Some(crate::biology_popups::Organ::Stomach)
}

/// `StomachDetailPopUp`.
pub fn draw_popup(frame: &mut UiFrame, app: &App, ui: &UiAssets, metrics: Option<&SdfFonts>, width: f32, height: f32) {
    let s = UiFrame::scaler(width, height);
    let reference = Vec2::new(app.cursor.x / s, app.cursor.y / s);
    match hovered_organ(app, reference) {
        Some(crate::biology_popups::Organ::Stomach) => {}
        Some(organ) => {
            crate::biology_popups::draw(frame, app, ui, metrics, width, height, organ);
            return;
        }
        None => {
            // The pointer on a bar's row opens that bar's pop-up (`OnHoverShow`).
            if app.inspector_open
                && app.inspector_tab == InspectorTab::Biology
                && let Some(panel) = app.selection.as_ref()
                && panel.organs.is_some()
            {
                match app.bar_hover.shown {
                    Some((crate::biology_popups::HoverKey::Bar(bar), center)) => {
                        crate::biology_popups::draw_bar(frame, app, ui, metrics, width, height, bar, center);
                    }
                    Some((crate::biology_popups::HoverKey::Flow, center)) => {
                        crate::biology_popups::draw_flow_popup(frame, app, ui, metrics, width, height, center);
                    }
                    None => {}
                }
            }
            return;
        }
    }
    let Some(info) = app.selection.as_ref().and_then(|panel| panel.stomach_info.as_ref()) else {
        return;
    };
    let mut p = Painter {
        seed: selection_seed(app),
        frame,
        ui,
        s,
        measure: Measure { fonts: metrics },
    };
    let n = info.items.len();
    let (w, h) = (471.3, 221.7 + contents_height(n));
    // Right of the `Section` (its width 374 from the picture's left), 9 away, centred on it.
    let picture = picture(app.selection.as_ref().is_some_and(|panel| panel.dead));
    let x = picture[0] + 374.0 + 9.0;
    let y = picture[1] + picture[3] / 2.0 - 0.91 - h / 2.0;
    p.slice("base_icon", [5.0, 5.0, 5.0, 5.0], 1.0, [x, y, w, h], [255, 255, 255, 255]);
    let at = |dx: f32, dy: f32, bw: f32, bh: f32| [x + dx, y + dy, bw, bh];

    // The title line: the title takes what is left (flexible width), the capacity (at least 65 wide) and the portion are as wide as their text, 10 apart.
    p.text(LIBERATION_BOLD, "Stomach Details:", at(10.0, 10.0, 314.9, 24.0), -1, 24.0, INK);
    let portion = format!("({:.1}%)", info.portion * 100.0);
    let capacity = format!("{:.2} u²", info.area);
    let portion_w = p.measure.width(ROBOTO_BOLD, &portion, 14.0);
    let capacity_w = p.measure.width(ROBOTO_BOLD, &capacity, 18.0).max(65.0);
    let portion_x = 461.3 - portion_w;
    let capacity_x = portion_x - 10.0 - capacity_w;
    p.text(ROBOTO_BOLD, &capacity, at(capacity_x, 10.0, capacity_w, 24.0), -1, 18.0, BLACK);
    p.text(ROBOTO_BOLD, &portion, at(portion_x, 10.0, portion_w, 24.0), 1, 14.0, [56, 56, 56, 255]);

    // Fullness.
    p.text(LIBERATION_BOLD, "Fullness:", at(10.0, 39.8, 120.0, 22.4), -1, 20.0, INK);
    let track = at(135.0, 39.0, 256.3, 24.0);
    p.slider(track, if info.area > 0.0 { info.total / info.area } else { 0.0 }, [174, 255, 0, 255]);
    p.text(ROBOTO_BOLD, &format!("{:.2}", info.total), at(135.0, 41.0, 116.3, 20.0), 1, 18.0, BLACK);
    p.text(ROBOTO_BOLD, "/", at(253.3, 41.0, 18.2, 20.0), 0, 18.0, BLACK);
    p.text(ROBOTO_BOLD, &format!("{:.2} u²", info.area), at(273.5, 41.0, 117.8, 20.0), -1, 18.0, BLACK);
    p.text(ROBOTO_BOLD, &format!("{:.1}%", info.fullness() * 100.0), at(396.3, 39.0, 65.0, 24.0), 1, 18.0, BLACK);

    // Acid level.
    p.text(LIBERATION_BOLD, "Acid Level:", at(10.0, 68.8, 120.0, 22.4), -1, 20.0, INK);
    p.slider(at(135.0, 68.0, 256.3, 24.0), info.acid.abs().min(1.0), [255, 245, 33, 255]);
    p.text(ROBOTO_BOLD, &format!("{:.1}%", info.acid * 100.0), at(396.3, 68.0, 65.0, 24.0), 1, 18.0, BLACK);

    // The bonus and the malus.
    p.text(LIBERATION_BOLD, "Digestion Bonus:", at(100.0, 97.5, 200.0, 22.4), -1, 20.0, INK);
    let green: Rgba = [36, 138, 0, 255];
    let red: Rgba = [196, 23, 0, 255];
    p.text(ROBOTO_BOLD, "+", at(300.0, 97.0, 12.0, 23.4), 0, 20.0, green);
    p.text(ROBOTO_BOLD, &format!("{:.1}%", info.bonus * 100.0), at(317.0, 97.0, 144.0, 23.4), -1, 17.0, green);
    p.text(LIBERATION_BOLD, "Efficiency Malus:", at(100.0, 123.0, 200.0, 22.4), -1, 20.0, INK);
    p.text(ROBOTO_BOLD, "-", at(300.0, 122.4, 12.0, 23.4), 0, 20.0, red);
    p.text(ROBOTO_BOLD, &format!("{:.1}%", info.malus * 100.0), at(317.0, 122.4, 144.0, 23.4), -1, 17.0, red);

    // Potential.
    p.text(LIBERATION_BOLD, "Potential:", at(10.0, 151.7, 120.0, 22.4), -1, 20.0, INK);
    p.slider(at(135.0, 150.9, 256.3, 24.0), info.effectiveness / remember_scale("potential", p.seed, info.effectiveness), [255, 163, 0, 255]);
    p.text(ROBOTO_BOLD, &format!("{:.2}x", info.effectiveness), at(396.3, 150.9, 65.0, 24.0), 1, 18.0, BLACK);

    // The contents.
    p.text(LIBERATION_BOLD, "Content :", at(10.0, 179.9, 111.1, 26.8), -1, 24.0, INK);
    if n == 0 {
        p.text(ROBOTO_BOLD, "Empty", at(131.1, 181.6, 56.9, 23.4), -1, 20.0, [97, 5, 5, 255]);
    }
    for item in &info.items {
        remember_scale("digest", p.seed, item.digestion_rate);
        remember_scale("energy", p.seed, item.energy_gain());
    }
    let mut top = 211.7;
    for item in &info.items {
        content_element(&mut p, x + 10.0, y + top, item, info.area);
        top += 135.0;
    }
}

/// `StomachContentElement`: a `gray_panel` with the material's amount and the three rates (15 inside, 5 apart). The panel is as wide as the widest of its
/// rows asks (a vertical layout group that does not expand): the header is the icon (25), the name, a colon (10), the amount and a 150 slider; a rate line is
/// a 150 label and a slider of at least 100.
fn content_element(p: &mut Painter<'_>, x: f32, y: f32, item: &internals::StomachItem, area: f32) {
    let name_w = p.measure.width(LIBERATION_BOLD, item.name(), 22.0);
    let amount = format!("{:.1} u²", item.amount);
    let amount_w = p.measure.width(LIBERATION_BOLD, &amount, 20.0);
    let header_w = 25.0 + 5.0 + name_w + 5.0 + 10.0 + 5.0 + amount_w + 5.0 + 150.0;
    let inner = header_w.max(255.0);
    let width = inner + 30.0;
    p.slice("gray_panel", [10.0, 10.0, 10.0, 10.0], 1.0, [x, y, width, 130.0], [255, 255, 255, 255]);
    let icon = match item.material {
        bibites_core::organs::StomachMaterial::Plant => "tiny_plant",
        bibites_core::organs::StomachMaterial::Meat => "tiny_meat",
    };
    // The header: the colon takes what the lines make the panel wider than the header.
    let extra = inner - header_w;
    let mut at_x = x + 15.0;
    p.icon(icon, [at_x, y + 15.0, 25.0, 25.0]);
    at_x += 30.0;
    p.text(LIBERATION_BOLD, item.name(), [at_x, y + 15.0, name_w, 25.0], -1, 22.0, WHITE);
    at_x += name_w + 5.0;
    p.text(LIBERATION_BOLD, ":", [at_x, y + 15.0, 10.0 + extra, 25.0], -1, 20.0, WHITE);
    at_x += 10.0 + extra + 5.0;
    p.text(LIBERATION_BOLD, &amount, [at_x, y + 15.0, amount_w, 25.0], 1, 20.0, WHITE);
    at_x += amount_w + 5.0;
    let share = if area > 0.0 { item.amount / area } else { 0.0 };
    let bar = [at_x, y + 21.5, 150.0, 12.0];
    p.slider(bar, share, [174, 255, 0, 255]);
    p.text(ROBOTO_BOLD, &format!("{:.0}%", share * 100.0), bar, 0, 18.0, BLACK);
    // The three lines: a 150 label and a slider that takes the rest.
    let slider_w = width - 30.0 - 150.0 - 5.0;
    let track = |line: f32| [x + 170.0, y + 45.0 + line * 25.0, slider_w, 20.0];
    p.text(LIBERATION_BOLD, "Digestion Rate :", [x + 15.0, y + 45.0, 150.0, 20.0], -1, 18.0, WHITE);
    p.slider(track(0.0), item.digestion_rate / remember_scale("digest", p.seed, 0.0), [255, 199, 0, 255]);
    p.text(ROBOTO_BOLD, &format!("{:.2} u²/s", item.digestion_rate), track(0.0), 0, 16.0, BLACK);
    p.text(LIBERATION_BOLD, "Efficiency :", [x + 15.0, y + 70.0, 150.0, 20.0], -1, 18.0, WHITE);
    p.rate_slider(track(1.0), item.efficiency, 1.0, &format!("{:.1} %", item.efficiency * 100.0));
    p.text(LIBERATION_BOLD, "Energy Gain Rate:", [x + 15.0, y + 95.0, 150.0, 20.0], -1, 18.0, WHITE);
    let gain = item.energy_gain();
    p.rate_slider(track(2.0), gain, remember_scale("energy", p.seed, 0.0), &format!("{:.1} E/s", gain));
}

thread_local! {
    static FLOW_SCALE: std::cell::Cell<(u64, f32)> = const { std::cell::Cell::new((0, 1.0)) };
}

/// The scale of the Flow bar (`energyRateSlider`, `InitSliderHandle(1)` when a bibite is selected, then raised to every value that reaches it).
pub fn flow_scale(seed: u64, flow: f32) -> f32 {
    FLOW_SCALE.with(|cell| {
        let (owner, scale) = cell.get();
        let scale = if owner == seed { scale.max(flow.abs()) } else { flow.abs().max(1.0) };
        cell.set((seed, scale));
        scale
    })
}

/// How far the Flow bar's fill reaches from its centre (`RateSliderHandle.OnValueChange`: half of `ScaledX`, two caps of 8 and the rest by `value / scale`
/// over the 250 wide track).
pub fn flow_reach(flow: f32, scale: f32) -> f32 {
    (16.0 + (flow / scale).abs().min(1.0) * (250.0 - 16.0)) / 2.0
}

/// A number that stands for the selected bibite (the stomach's pellets and the Flow bar's scale are kept per bibite).
pub fn selection_seed(app: &App) -> u64 {
    app.selected.map_or(0, |id| {
        format!("{id:?}")
            .bytes()
            .fold(7u64, |hash, byte| hash.wrapping_mul(131).wrapping_add(u64::from(byte)))
    })
}

/// The Flow bar of the Biology panel (`EnergyRateSliderHolder/EnergyFlowInfo`): which of its parts a node is, when it is one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FlowPart {
    /// `PositiveFill`.
    PositiveFill,
    /// `NegativeFill`.
    NegativeFill,
    /// `PositiveRateText`.
    PositiveText,
    /// `NegativeRateText`.
    NegativeText,
}

/// See [`FlowPart`].
pub fn flow_part(path: &str) -> Option<FlowPart> {
    let rest = path.split("/BodyInfoHolder/EnergyRateSliderHolder/EnergyFlowInfo/CenterPoint/").nth(1)?;
    match rest {
        "PositiveFill" => Some(FlowPart::PositiveFill),
        "NegativeFill" => Some(FlowPart::NegativeFill),
        "PositiveRateText" => Some(FlowPart::PositiveText),
        "NegativeRateText" => Some(FlowPart::NegativeText),
        _ => None,
    }
}

/// The text of the Flow bar (`FloatValueTextHandle`: two decimals and the unit).
pub fn flow_text(flow: f32) -> String {
    format!("{flow:.2}E/S")
}

/// What a bar of the Biology panel's body section changes in its captured node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BarEdit {
    /// The fill's width (reference pixels).
    Width(f32),
    /// The left edge of a progress slider's target marker.
    MarkerX(f32),
}

/// `ValueSliderHandle` / `ProgressSliderHandle`: a fill is `ScaledX(value)` long, two caps of 8 and the rest in proportion to `value / scale`, over the
/// 250 wide track that starts at x 120.
fn scaled_x(ratio: f32) -> f32 {
    16.0 + ratio.clamp(0.0, 1.0) * (250.0 - 16.0)
}

/// The fills of the health, energy, growth and age bars and the markers of the last two, from the selected bibite (`BiologyPanel.UpdateBodySection`:
/// health and energy take their maximum as the scale; growth is `InitProgressHandle(1.25, 1)`, age `(1.25 x threshold, threshold)`; both raise their
/// scale to a value that reaches it).
pub fn bar_edit(path: &str, panel: &crate::app::SelectionPanel, seed: u64) -> Option<BarEdit> {
    let ratio = |value: f32, scale: f32| if scale > 0.0 { value / scale } else { 0.0 };
    if path.ends_with("/HealthSliderHolder/ValueSlider/Fill") {
        return Some(BarEdit::Width(scaled_x(ratio(panel.health, panel.max_health))));
    }
    if path.ends_with("/EnergySliderHolder/ValueSlider/Fill") {
        return Some(BarEdit::Width(scaled_x(ratio(panel.energy, panel.max_energy))));
    }
    let growth_scale = remember_scale_from("growth", seed, panel.maturity, 1.25);
    let age_scale = remember_scale_from("age", seed, panel.age_hours, 1.25 * panel.aging_threshold);
    if path.ends_with("/MaturitySliderHolder/ProgressSlider/Fill") {
        return Some(BarEdit::Width(scaled_x(ratio(panel.maturity, growth_scale))));
    }
    if path.ends_with("/MaturitySliderHolder/ProgressSlider/Indicator") {
        return Some(BarEdit::MarkerX(120.0 + scaled_x(ratio(1.0, growth_scale)) - 8.0));
    }
    if path.ends_with("/AgeSliderHolder/ProgressSlider/Fill") {
        return Some(BarEdit::Width(scaled_x(ratio(panel.age_hours, age_scale))));
    }
    if path.ends_with("/AgeSliderHolder/ProgressSlider/Indicator") {
        return Some(BarEdit::MarkerX(120.0 + scaled_x(ratio(panel.aging_threshold, age_scale)) - 8.0));
    }
    None
}
