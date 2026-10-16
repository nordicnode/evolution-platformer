//! The window's own look: one palette, one set of shapes, applied once.
//!
//! The window used to be egui's stock dark theme with a handful of local colour
//! constants in each module — a game drawn in one palette with its furniture drawn in
//! three. This module is the furniture's palette, and it is deliberately the *same*
//! palette as the art: the cream is the sun's face from `tools/art/common.py`, the
//! vermilion is the runner, the greens and golds are the panel's own. Nothing here
//! invents a colour; it names the ones the game already uses and says where they go.
//!
//! Three decisions are worth stating, because they are what turns a stock egui window
//! into something that reads as *this* game.
//!
//! * **Square-ish widgets.** Corners are three pixels, not eight: the game is drawn from
//!   sixteen-pixel tiles and a rounded pill of a button belongs to a different program.
//!   The rounding is one number ([`RADIUS`]) and every shape in the window reads it.
//! * **Four surfaces, never a gradient.** [`SHELL`], [`PANEL`], [`CARD`] and [`SUNKEN`]
//!   are the four depths the window has: the frame it is set in, the side panels, a card
//!   or button resting on one, and a well (the chart's plot, the status bar) cut into it.
//!   A widget that needs to stand out moves up a step or takes an accent; nothing
//!   invents a fifth shade.
//! * **The type is a HUD, not a document.** Headings and small labels are monospace so
//!   the numbers under them line up in columns, which is what a run's readings are,
//!   and body text stays proportional because sentences are sentences. The sizes are
//!   named ([`heading_size`]) rather than sprinkled, so a panel and the canvas can
//!   agree about how big a title is.
//!
//! [`apply`] is the whole of the installation: call it once with the egui context and
//! every widget after that draws itself in this palette. It is idempotent, so a test or
//! a second window can call it again without harm.

use egui::{
    Color32, Context, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Theme, ThemePreference,
    Visuals,
};

/// The *display* surfaces: the darkest colour in the window, and what the two things you
/// *look at* are set on — the level canvas' field and the fitness chart's plot.
///
/// The window is deliberately two materials. Its **chrome** is light — cream panels with a
/// gold lip, the way a game's own menus are drawn — and its **screens** are dark, so the art
/// and the readings on them are the brightest things on the screen. A single dark surface
/// everywhere was what made the window read as a tool: an editor's panels and a game's menu
/// are the same *shape* but not the same material, and saying so is most of the look.
pub const SHELL: Color32 = Color32::from_rgb(16, 19, 24);
/// A panel of chrome — one step up from the frame, and the surface most of the window is made of.
pub const PANEL: Color32 = Color32::from_rgb(238, 233, 219);
/// A card, a resting button, an input: a thing lying *on* a panel.
pub const CARD: Color32 = Color32::from_rgb(252, 250, 243);
/// A raised card: what the pointer is over, or a selected entry.
pub const CARD_HOVER: Color32 = Color32::from_rgb(255, 255, 255);
/// A card that cannot be used: quieter than a resting one, and *paler* rather than darker —
/// a disabled control on a light panel should recede into it, not punch a hole in it.
pub const CARD_OFF: Color32 = Color32::from_rgb(226, 221, 206);
/// A well cut into a panel: the switch's socket, the status bar. Recessed, not dark — the
/// dark surfaces in this window are the two screens ([`SHELL`]) and nothing else.
pub const SUNKEN: Color32 = Color32::from_rgb(216, 209, 191);
/// A well cut into a *screen*: the chart's plot inside its frame. The one place where the dark
/// material needs a second depth of its own, so a plot reads as data set into a display rather
/// than as the display itself.
pub const SCREEN_WELL: Color32 = Color32::from_rgb(26, 31, 39);
/// The hairline that separates two surfaces.
pub const EDGE: Color32 = Color32::from_rgb(198, 190, 170);

/// The chunky gold outline a piece of chrome wears: the dock along its bottom edge, a panel
/// frame, the ring around the selected tool.
///
/// This is the one colour in the window that is *furniture* rather than content, and it is
/// what a game's menu has that an application's toolbar does not: a lip. [`ACCENT`] fills a
/// selected control with the same gold, so the window has one accent in two weights rather
/// than two accents.
pub const GOLD_EDGE: Color32 = Color32::from_rgb(238, 186, 58);
/// Ink for type *on* [`ACCENT`]: dark, warm, and the only colour that is readable on gold.
pub const INK_ON_ACCENT: Color32 = Color32::from_rgb(58, 42, 12);

/// The dock: the bar across the top, one step *up* from a panel and warmer than it.
///
/// A game's furniture is furniture: the tool bar is a plank carried in front of the frame,
/// the panels behind it are recessed, and the difference between them is what tells the eye
/// where the *controls* are. This is the plank's own colour — brighter than a panel, because
/// it is the surface the light reaches first.
pub const DOCK: Color32 = Color32::from_rgb(248, 245, 232);
/// The lit edge of a raised surface: its top and left sides, where the light comes from.
pub const BEVEL_LIGHT: Color32 = Color32::from_rgb(255, 255, 255);
/// The shaded edge of a raised surface: its bottom and right sides.
pub const BEVEL_DARK: Color32 = Color32::from_rgb(176, 167, 144);
/// The soft drop shadow a raised surface casts on what is behind it.
pub const SHADOW: Color32 = Color32::from_rgba_premultiplied(60, 50, 30, 60);
/// The plate a label sits on when it has to read as a *sign*: the level's name in the dock,
/// a banner over the canvas. The brightest surface in the window, so the type on it leads.
pub const PLATE: Color32 = Color32::from_rgb(255, 253, 245);

/// Body text, on chrome: dark ink, because the panels are cream.
pub const TEXT: Color32 = Color32::from_rgb(48, 45, 38);
/// Secondary text: units, hints, the small print under a heading.
pub const TEXT_DIM: Color32 = Color32::from_rgb(112, 105, 89);
/// The quietest text there is: a caption on a disabled control.
pub const TEXT_FAINT: Color32 = Color32::from_rgb(158, 150, 130);

/// Text *on a dark screen*: the chart's labels and the canvas' readouts, which are drawn on
/// [`SHELL`] rather than on chrome. Two ink scales rather than one, because a colour that is
/// legible on cream is invisible on the dark, and the window has both materials in it.
pub const SCREEN_INK: Color32 = Color32::from_rgb(226, 231, 238);
/// Secondary ink on a dark screen.
pub const SCREEN_INK_DIM: Color32 = Color32::from_rgb(158, 168, 180);
/// The hairline between two surfaces *on* a dark screen: the chart's axes.
pub const SCREEN_EDGE: Color32 = Color32::from_rgb(74, 84, 98);

/// The sun's face: the window's accent, and the colour a thing worth looking at gets.
///
/// The bright half of the pair whose other half is [`INK_ON_ACCENT`] and the `*_INK` colours
/// below: this one *fills* something (a selected tool, a lamp on a banner, a curve on the dark
/// chart), and the inks are for type. Gold type on a cream panel is a smudge, which is why the
/// window does not have any.
pub const ACCENT: Color32 = Color32::from_rgb(250, 214, 102);
/// The runner's vermilion: an action taken, or a thing currently being done.
pub const RUN: Color32 = Color32::from_rgb(228, 87, 61);
/// Growth, a win, a good reading.
pub const GOOD: Color32 = Color32::from_rgb(120, 216, 120);
/// A warning that is not yet a failure: a stall, a level that needs attention.
pub const WARN: Color32 = Color32::from_rgb(240, 150, 90);
/// A failure: an unreachable goal, a level that will not load.
pub const BAD: Color32 = Color32::from_rgb(226, 106, 106);
/// Distance, history, a second series: the cool end of the palette.
pub const COOL: Color32 = Color32::from_rgb(120, 170, 230);

/// The accent as *type* on chrome: dark amber, for a count or a state that is worth looking at.
pub const ACCENT_INK: Color32 = Color32::from_rgb(146, 102, 8);
/// The action colour as type on chrome.
pub const RUN_INK: Color32 = Color32::from_rgb(176, 52, 32);
/// The win colour as type on chrome: a verdict, a "solved".
pub const GOOD_INK: Color32 = Color32::from_rgb(26, 118, 50);
/// The warning colour as type on chrome.
pub const WARN_INK: Color32 = Color32::from_rgb(168, 92, 18);
/// The failure colour as type on chrome: an unreachable level.
pub const BAD_INK: Color32 = Color32::from_rgb(176, 46, 42);

/// The one corner radius: three pixels, everywhere, on purpose.
///
/// Small widgets — a chip, a chart's frame, a palette cell — read this and stay square-ish, so
/// the window still belongs to a game drawn from sixteen-pixel tiles.
pub const RADIUS: u8 = 3;

/// The *chunky* radius: the dock's plates, its tool buttons and the mode switch.
///
/// Two radii rather than one, and the second exists because furniture is not a widget: a tool
/// button in a game's tool bar is a printed tile of plastic with a rounded lip, and the three-pixel
/// corner that keeps a chart's frame honest reads as a mistake at forty points across. The pair is
/// the whole rule — small things read [`RADIUS`], things you *press* read this — and nothing
/// invents a third.
pub const RADIUS_LG: u8 = 8;

/// The size of a press: how far a button's face sinks into its socket.
///
/// One pixel is invisible and three reads as a layout bug; two is the smallest offset that still
/// resolves as depth rather than as jitter at the sizes this window draws (a tool button is about
/// forty points).
pub const PRESS_SINK: f32 = 2.0;

/// The heading size. Named rather than inlined so the canvas and the panels agree.
pub const HEADING_SIZE: f32 = 19.0;
/// The small print's size: units, hints, the status bar.
pub const SMALL_SIZE: f32 = 11.0;
/// The monospace size: readings, counts, keys.
pub const MONO_SIZE: f32 = 13.0;

/// Install the theme. Call once per context; calling it twice is harmless.
pub fn apply(ctx: &Context) {
    // **The operating system does not get a vote on this window's look.** egui keeps *two* styles —
    // one per theme — and chooses between them from the system's preference, while `set_style` below
    // writes into whichever of the two happens to be selected at the time. Left alone the preference
    // is `System`, so on a machine with a dark desktop the theme installed here lands in the light
    // slot and is never used: the panels, the dock and the canvas would all still be this game's
    // (they paint their own fills) while every widget egui draws *itself* — a floating window's
    // frame and type, a button, a slider, a separator — came out in egui's stock dark colours inside
    // a cream window. That is not a hypothetical: it is what the level-details window did the first
    // time it was rendered, which is what turned this line up.
    ctx.set_theme(ThemePreference::Light);
    let mut style = (*ctx.style()).clone();

    style.visuals = visuals();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.menu_margin = egui::Margin::same(6);
    style.spacing.indent = 16.0;
    style.spacing.slider_width = 130.0;
    // A tool bar's controls are pressed, grabbed and dragged over. A scroll bar that appears on
    // hover is a control appearing under the pointer; the panels here are small enough to show
    // their bars always, which is also how a game's own option screens do it.
    style.spacing.scroll.floating = false;

    // A HUD's type scale: monospace where numbers have to line up, proportional for
    // sentences, and the same three sizes everywhere rather than a per-module guess.
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(HEADING_SIZE, FontFamily::Monospace),
    );
    style.text_styles.insert(
        TextStyle::Body,
        FontId::new(14.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(14.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(SMALL_SIZE, FontFamily::Monospace),
    );
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(MONO_SIZE, FontFamily::Monospace),
    );

    // Into **both** slots, so that nothing moving the preference can repaint the window: this game
    // has one palette and no dark mode, and the honest way to say so is to make the other slot the
    // same picture rather than to leave it behind as a trap for the next widget egui draws itself.
    ctx.set_style_of(Theme::Light, style.clone());
    ctx.set_style_of(Theme::Dark, style);
}

/// The theme as egui's own widget state table.
///
/// Public because the tests read it back rather than trusting the constants: a theme is a
/// picture, and the only thing worth asserting about it is what a widget actually draws.
pub fn visuals() -> Visuals {
    // **Light**, not dark: the chrome is cream and every default egui derives from this flag —
    // the base text colour, the widget fills, the icon tints — has to be derived from the light
    // end or half the window's defaults fight the palette. The two dark surfaces in the window
    // are its *screens*, and they paint themselves; nothing in egui's own widgets is drawn on them.
    let mut v = Visuals::light();

    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(2.0_f32, GOLD_EDGE);
    v.window_corner_radius = CornerRadius::same(RADIUS_LG);
    // A window and a menu float *over* the canvas, and a floating thing with no shadow is a
    // rectangle of paint: the level's shelf and the Open menu are the only surfaces in the
    // window that are not part of its frame, so they are the only ones that cast. Small and
    // hard-edged rather than a modern big soft blur, because everything else here is chunky.
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 1,
        color: SHADOW,
    };
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 3],
        blur: 8,
        spread: 1,
        color: SHADOW,
    };
    v.menu_corner_radius = CornerRadius::same(RADIUS_LG);
    v.extreme_bg_color = CARD;
    v.faint_bg_color = SUNKEN;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT_INK;
    v.warn_fg_color = WARN_INK;
    v.error_fg_color = BAD_INK;

    // A selection is the accent itself with the ink staying dark, which is what the game's own
    // menus do: the highlighted cell is *filled*, not tinted, so it reads from across the room.
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT_INK);
    v.text_cursor.stroke = Stroke::new(2.0_f32, ACCENT_INK);

    let widgets = &mut v.widgets;
    widgets.noninteractive.bg_fill = PANEL;
    widgets.noninteractive.weak_bg_fill = PANEL;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, EDGE);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    widgets.noninteractive.corner_radius = CornerRadius::same(RADIUS);

    widgets.inactive.bg_fill = CARD;
    widgets.inactive.weak_bg_fill = CARD;
    widgets.inactive.bg_stroke = Stroke::new(1.0_f32, EDGE);
    widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    widgets.inactive.corner_radius = CornerRadius::same(RADIUS);
    widgets.inactive.expansion = 0.0;

    widgets.hovered.bg_fill = CARD_HOVER;
    widgets.hovered.weak_bg_fill = CARD_HOVER;
    widgets.hovered.bg_stroke = Stroke::new(1.5_f32, GOLD_EDGE);
    widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    widgets.hovered.corner_radius = CornerRadius::same(RADIUS);
    widgets.hovered.expansion = 0.0;

    widgets.active.bg_fill = ACCENT;
    widgets.active.weak_bg_fill = ACCENT;
    widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT_INK);
    widgets.active.fg_stroke = Stroke::new(1.0_f32, INK_ON_ACCENT);
    widgets.active.corner_radius = CornerRadius::same(RADIUS);
    widgets.active.expansion = 0.0;

    widgets.open.bg_fill = CARD;
    widgets.open.weak_bg_fill = CARD;
    widgets.open.bg_stroke = Stroke::new(1.0_f32, EDGE);
    widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT);
    widgets.open.corner_radius = CornerRadius::same(RADIUS);

    v
}

/// A surface to put a panel's contents on: the card colour, edged, with the theme's key.
///
/// Used for the level shelf's entries and the canvas' floating readouts, so a card looks
/// like a card everywhere rather than like whatever frame each call site chose.
pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, EDGE))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .corner_radius(CornerRadius::same(RADIUS))
}

/// A panel of chrome: the cream surface with the gold lip, which is what makes a region read
/// as a *menu* rather than as a region of an application window.
///
/// Used by every side panel in the window. The lip is deliberately chunky (two points) and
/// deliberately the same gold as a selected tool, because that is the whole trick: a game's
/// chrome is outlined in its own accent, and an application's is outlined in grey.
pub fn panel_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(2.0_f32, GOLD_EDGE))
        .inner_margin(egui::Margin::symmetric(10, 10))
        .corner_radius(CornerRadius::same(RADIUS_LG))
}

/// The bar the window's tools are carried in.
///
/// A raised plank rather than a panel: no stroke (its own lit and shaded edges *are* its outline,
/// drawn by the widgets on it), the dock's warmer colour, and room around the controls so they
/// read as objects resting on something rather than as a row of text.
pub fn dock_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(DOCK)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .corner_radius(CornerRadius::same(RADIUS))
}

/// A well cut into a panel: the status bar and the chart's plot area.
pub fn sunk_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(SUNKEN)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .corner_radius(CornerRadius::same(RADIUS))
}

/// The colour a *material* is drawn in on a shelf thumbnail, from the tile itself.
///
/// The shelf has to say what a level is made of without the atlas (a level the window has
/// not opened has no loaded art behind it yet), so the five materials have five flat
/// colours: the greens and browns of soil, the pale gold of a beach, the greys of stone,
/// the terracotta of brick and the brown of timber. They are deliberately the *palette*
/// of the art rather than the art: a thumbnail is a map of a level, and a map is not a
/// picture.
pub fn material_colour(tile: sim::Tile) -> Option<Color32> {
    use sim::Tile;
    match tile {
        Tile::Empty | Tile::Goal => None,
        Tile::Solid => Some(Color32::from_rgb(96, 176, 96)),
        Tile::Sand => Some(Color32::from_rgb(230, 206, 140)),
        Tile::Brick => Some(Color32::from_rgb(196, 110, 84)),
        Tile::Plank => Some(Color32::from_rgb(176, 132, 84)),
        Tile::Spike => Some(Color32::from_rgb(226, 106, 106)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The theme replaces egui's stock dark visuals rather than tinting them.
    ///
    /// Three facts, each one a way a theme can be installed and not take: the style must
    /// be *replaced* (a context that keeps its default window fill is a window that still
    /// looks like egui), the four surfaces must be four distinct shades (a theme whose
    /// card and panel are the same colour has no depth), and the widget states must
    /// differ from each other (a button that does not change under the pointer is a
    /// button nobody knows is a button).
    /// A surface's brightness, which is the one property the two *materials* of the window are
    /// told apart by: chrome is light, a screen is dark.
    fn luma(c: Color32) -> f32 {
        0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32
    }

    #[test]
    fn apply_installs_a_theme_and_the_surfaces_are_four_distinct_depths() {
        let ctx = Context::default();
        apply(&ctx);
        let style = ctx.style();
        assert_eq!(style.visuals.window_fill, PANEL);
        assert_eq!(style.visuals.panel_fill, PANEL);
        assert_ne!(SHELL, PANEL);
        assert_ne!(PANEL, CARD);
        assert_ne!(CARD, SUNKEN);
        // **The chrome is light and the screens are dark**, which is the whole of the window's
        // look: a cream panel with the game's art on a dark field around it. Asserted as a
        // brightness *relation* rather than as hex codes, because that is the claim — a future
        // pass may retune the cream, but a pass that made the panels dark again would have
        // rebuilt the tool-looking window this theme exists to stop.
        assert!(
            luma(PANEL) > 180.0 && luma(SHELL) < 60.0,
            "chrome {PANEL:?} should be light and the display {SHELL:?} dark"
        );
        // And the ink follows the surface it is for: dark on chrome, light on a screen.
        assert!(luma(TEXT) < 90.0, "type on a cream panel has to be dark");
        assert!(luma(SCREEN_INK) > 190.0, "type on a dark plot has to be light");
        // The accent is a *fill*, so there is an ink for the type that goes on it, and it is
        // dark enough to read on the gold.
        assert!(luma(INK_ON_ACCENT) < 90.0);
        assert!(luma(ACCENT) > 190.0);
        // The padding the theme installs, read back rather than trusted: a control packed into
        // the text it carries reads as a label with a box drawn round it.
        assert_eq!(style.spacing.button_padding, egui::vec2(12.0, 7.0));

        let widgets = &style.visuals.widgets;
        assert_ne!(widgets.inactive.bg_fill, widgets.hovered.bg_fill);
        assert_ne!(widgets.hovered.bg_fill, widgets.active.bg_fill);
        assert_eq!(widgets.inactive.corner_radius, CornerRadius::same(RADIUS));

        // And the type scale is the HUD's: three named sizes, monospace where numbers are.
        let heading = style.text_styles.get(&TextStyle::Heading).unwrap();
        assert_eq!(heading.family, FontFamily::Monospace);
        assert_eq!(heading.size, HEADING_SIZE);
    }

    /// A theme change cannot repaint the window.
    ///
    /// The bug this pins was found by *looking at a frame*, not by a test: egui holds two styles and
    /// picks one from the system's theme preference, and `set_style` writes to whichever is selected —
    /// so on this machine (a dark desktop) the installed theme went into the light slot, the dark
    /// default stayed in force, and the level-details window drew its frame and its type in egui's
    /// stock colours inside a cream window. Asserting both slots is the only way to see it, because
    /// the slot that is wrong is the one nothing on screen is using yet.
    #[test]
    fn a_theme_switch_cannot_repaint_the_window() {
        let ctx = Context::default();
        apply(&ctx);
        let ours = visuals();
        for theme in [Theme::Light, Theme::Dark] {
            let style = ctx.style_of(theme);
            assert_eq!(style.visuals.window_fill, ours.window_fill, "{theme:?}");
            assert_eq!(style.visuals.panel_fill, ours.panel_fill, "{theme:?}");
            assert_eq!(style.visuals.override_text_color, Some(TEXT), "{theme:?}");
            assert_eq!(style.spacing.button_padding, egui::vec2(12.0, 7.0), "{theme:?}");
        }
        // And the preference is pinned, so egui never swaps the two under us in the first place.
        assert_eq!(
            ctx.options(|options| options.theme_preference),
            ThemePreference::Light
        );
    }

    /// Two calls leave the same theme: the app installs it at startup and a test may
    /// install it again, and the second install must not compound padding or stacking.
    #[test]
    fn applying_twice_changes_nothing() {
        let ctx = Context::default();
        apply(&ctx);
        let once = (*ctx.style()).clone();
        apply(&ctx);
        let twice = (*ctx.style()).clone();
        assert_eq!(once.spacing.item_spacing, twice.spacing.item_spacing);
        assert_eq!(once.text_styles.len(), twice.text_styles.len());
        assert_eq!(once.visuals.window_fill, twice.visuals.window_fill);
    }

    /// Every material a level can be made of has a thumbnail colour, and empty ground
    /// has none — the four materials and the hazard, and nothing drawn for air.
    #[test]
    fn every_material_has_a_thumbnail_colour() {
        use sim::Tile;
        assert!(material_colour(Tile::Empty).is_none());
        let solids = [
            Tile::Solid,
            Tile::Sand,
            Tile::Brick,
            Tile::Plank,
            Tile::Spike,
        ];
        let colours: Vec<Color32> = solids.iter().filter_map(|t| material_colour(*t)).collect();
        assert_eq!(colours.len(), solids.len(), "every material is drawn");
        for (i, a) in colours.iter().enumerate() {
            for b in &colours[i + 1..] {
                assert_ne!(a, b, "two materials share a thumbnail colour");
            }
        }
    }
}
