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
    Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke,
    TextStyle, Theme, ThemePreference, Visuals,
};

/// The *display* surfaces: the darkest colour in the window, and what the two things you
/// *look at* are set on — the level canvas' field and the fitness chart's plot.
///
/// The window is deliberately two materials. Its **chrome** is light — cream panels with a
/// gold lip, the way a game's own menus are drawn — and its **screens** are dark, so the art
/// and the readings on them are the brightest things on the screen. A single dark surface
/// everywhere was what made the window read as a tool: an editor's panels and a game's menu
/// are the same *shape* but not the same material, and saying so is most of the look.
pub const SHELL: Color32 = Color32::from_rgb(20, 26, 38);
/// A panel of chrome — one step up from the frame, and the surface most of the window is made of.
pub const PANEL: Color32 = Color32::from_rgb(255, 249, 235);
/// A card, a resting button, an input: a thing lying *on* a panel.
pub const CARD: Color32 = Color32::from_rgb(255, 255, 250);
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
pub const SCREEN_WELL: Color32 = Color32::from_rgb(24, 32, 48);
/// The hairline that separates two surfaces.
///
/// Decorative: it divides a panel from a card, and 1.5:1 is the right weight for a line whose job
/// is to be *seen* rather than to be followed. A thing you can press is outlined with
/// [`CONTROL_EDGE`] instead, because "where does this control begin" is a question a boundary has
/// to answer at a glance.
pub const EDGE: Color32 = Color32::from_rgb(198, 190, 170);

/// The boundary of something you can **press**: a control's own outline, at 3:1 on every chrome
/// surface it sits on.
///
/// The second edge exists because the first one is a *hairline* and a control's edge is not the
/// same job. Measured against the palette: [`EDGE`] is 1.53:1 on a panel and 1.77:1 on a card,
/// which is right for a rule between two panels and too quiet for the outline of a button — a
/// resting plate then reads as a word with a shadow rather than as a thing to press. This one is
/// 3.0:1 on the sunken well and 4.4:1 on a card, which is the WCAG floor for a non-text boundary
/// (1.4.11), and it is deliberately *not* used for decorative rules: the window has two kinds of
/// line and now says which is which.
pub const CONTROL_EDGE: Color32 = Color32::from_rgb(123, 117, 105);

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
pub const DOCK: Color32 = Color32::from_rgb(255, 244, 214);
/// The soft drop shadow a raised surface casts on what is behind it.
///
/// It is also, since the plates lost their bevel lines, the *only* thing that says a control is
/// raised rather than printed: a resting plate casts it, a pressed one does not.
pub const SHADOW: Color32 = Color32::from_rgba_premultiplied(60, 50, 30, 60);
/// The plate a label sits on when it has to read as a *sign*: the level's name in the dock,
/// a banner over the canvas. The brightest surface in the window, so the type on it leads.
pub const PLATE: Color32 = Color32::from_rgb(255, 253, 245);

/// Body text, on chrome: dark ink, because the panels are cream.
pub const TEXT: Color32 = Color32::from_rgb(48, 45, 38);
/// Secondary text: units, hints, the small print under a heading.
///
/// **Measured against every chrome surface, not chosen by eye.** The window's panels are cream and
/// its status bar is a sunken well a shade darker, and a secondary ink has to clear 4.5:1 on the
/// darkest of them or the small print of the window is decoration: this one is 5.82:1 on [`PANEL`],
/// 6.76:1 on [`CARD`] and 4.64:1 on [`SUNKEN`]. The previous value (112, 105, 89) measured 4.49 on
/// a panel and 3.58 on the well — a hair under the line on one surface and well under it on the
/// other, which is exactly the kind of miss that a palette review catches and a screenshot does
/// not.
pub const TEXT_DIM: Color32 = Color32::from_rgb(94, 88, 75);
/// The quietest *active* text: metadata, a material list, the key hint in the status bar.
///
/// 5.07:1 on a panel and 5.89:1 on a card, so it is still readable small — a caption nobody can
/// read is not a caption. It is deliberately **not** used on [`SUNKEN`] (4.04:1 would be under the
/// floor there): the status bar's own readouts take [`TEXT_DIM`]. A *disabled* label is the one
/// place the contrast rule relaxes, and it is still drawn in this ink on [`CARD_OFF`] (4.53:1)
/// rather than in a grey that disappears.
pub const TEXT_FAINT: Color32 = Color32::from_rgb(103, 97, 84);

/// Text *on a dark screen*: the chart's labels and the canvas' readouts, which are drawn on
/// [`SHELL`] rather than on chrome. Two ink scales rather than one, because a colour that is
/// legible on cream is invisible on the dark, and the window has both materials in it.
pub const SCREEN_INK: Color32 = Color32::from_rgb(226, 231, 238);
/// Secondary ink on a dark screen.
pub const SCREEN_INK_DIM: Color32 = Color32::from_rgb(158, 168, 180);
/// The hairline between two surfaces *on* a dark screen: the chart's axes.
///
/// 3.0:1 on the plot's own well and 3.38:1 on the shell around it, which is the floor a chart's
/// frame and axes have to clear to be *readable* rather than decorative. The old value (74, 84, 98)
/// measured 2.16:1 on the plot — a gridline that vanished on a bright monitor and a baseline that
/// was not there at all on a dim one.
pub const SCREEN_EDGE: Color32 = Color32::from_rgb(104, 118, 136);

/// The *interior* rule of a chart: a gridline, deliberately quieter than [`SCREEN_EDGE`].
///
/// Two weights of line on a dark screen for the same reason there are two on chrome: the frame is
/// the edge of the display and has to be found, a gridline is a scale a reader consults after the
/// data has been found. 2.2:1 on the plot is the value the frame had before it was brightened —
/// right for a rule *inside* a chart, too quiet for the chart's own outline.
pub const SCREEN_GRID: Color32 = Color32::from_rgb(74, 84, 98);

/// The sun's face: the window's accent, and the colour a thing worth looking at gets.
///
/// The bright half of the pair whose other half is [`INK_ON_ACCENT`] and the `*_INK` colours
/// below: this one *fills* something (a selected tool, a lamp on a banner, a curve on the dark
/// chart), and the inks are for type. Gold type on a cream panel is a smudge, which is why the
/// window does not have any.
pub const ACCENT: Color32 = Color32::from_rgb(251, 197, 49);
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
///
/// The five `*_INK` colours are the palette's *type* half, and each is measured at 4.5:1 or better
/// on all four chrome surfaces ([`PANEL`], [`CARD`], [`CARD_OFF`], [`SUNKEN`]) — 5.81:1 on a panel
/// and 4.63:1 on the status bar for this one. They are the same hue as the fills above them, taken
/// down until they are readable: a gold on cream is a smudge, and an amber dark enough to read is
/// still the accent's own colour. `theme::tests::every_type_colour_clears_the_contrast_floor` holds
/// the whole set to that number, so a future retune of the cream cannot quietly make the window's
/// coloured type unreadable.
pub const ACCENT_INK: Color32 = Color32::from_rgb(118, 82, 6);
/// The runner's vermilion as *type* on chrome: a playthrough's own line.
pub const RUN_INK: Color32 = Color32::from_rgb(162, 48, 30);
/// The win colour as type on chrome: a verdict, a "solved".
pub const GOOD_INK: Color32 = Color32::from_rgb(23, 102, 43);
/// The warning colour as type on chrome.
///
/// This is the ink a warning *label* takes. [`WARN`] is a fill (a lamp, a marker on the canvas) and
/// is 1.5:1 on the status bar's well — using it as type there was the one place the window drew a
/// warning nobody could read.
pub const WARN_INK: Color32 = Color32::from_rgb(134, 73, 14);
/// The failure colour as type on chrome: an unreachable level.
pub const BAD_INK: Color32 = Color32::from_rgb(166, 43, 40);

/// The one corner radius: six pixels, everywhere.
///
/// This window's furniture is a *toy*, not a terminal: a card, a cell and a chart's frame all
/// round off, because every panel in the games this one takes its look from is a soft-cornered
/// plate with a thick outline and a shadow under it. Six is enough to read as rounded at forty
/// points and not so much that a sixteen-pixel tile's grid starts looking like a pill.
pub const RADIUS: u8 = 6;

/// The *large* radius: the dock's plates, its tool buttons, the panels and the title screen's
/// menu box.
///
/// The same rule one size up: a panel, the title screen's menu box and a dock plate are all
/// fourteen points across the corner. The pair is the whole scale — nothing invents a third.
pub const RADIUS_LG: u8 = 14;

/// The outline every panel and control wears: a warm dark brown, never black.
///
/// The games this look comes from outline their menus in a deep chocolate rather than a neutral
/// black, and that one substitution is most of the difference between a cheerful sign and a
/// terminal. At 3:1 on cream it is a *boundary* (a control's edge), and because it is warm it
/// sits beside the gold and the red instead of cutting them.
pub const OUTLINE: Color32 = Color32::from_rgb(74, 52, 32);

/// The hard, tight shadow a plate casts: the outline again, held back so a plate reads as lying
/// *on* the surface rather than as floating over it.
pub fn drop_shadow() -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: [0, 3],
        blur: 0,
        spread: 0,
        color: Color32::from_rgba_premultiplied(74, 52, 32, 70),
    }
}

/// A ribbon: the bright blue a menu's header is printed on.
///
/// The second material of the look, and the one that makes the window read as *colourful* rather
/// than as a beige tool: cream is paper, gold is the accent, and blue is the board a heading is
/// chalked on — the colour every menu screen in the games this one follows puts its tabs and
/// titles on. White type goes on it, and nothing else does.
pub const RIBBON: Color32 = Color32::from_rgb(43, 133, 214);

/// The size of a press: how far a button's face sinks into its socket.
///
/// One pixel is invisible and three reads as a layout bug; two is the smallest offset that still
/// resolves as depth rather than as jitter at the sizes this window draws (a tool button is about
/// forty points).
pub const PRESS_SINK: f32 = 2.0;

/// The pixel face: Press Start 2P, the type the game's logo is set in.
///
/// The window's HUD type is egui's monospace, which is right for readings and wrong for a
/// *sign*: the title screen's menu is the one surface in the window that is a game's front
/// door rather than a tool's furniture, and a front door is lettered in the same face as the
/// name over it. The font is embedded (OFL, see `assets/OFL.txt`) rather than loaded from
/// the assets directory, for the logo's own reason: it is part of the game's identity, not
/// part of a tileset a person can swap with `--assets`.
pub const PIXEL_FONT_BYTES: &[u8] = include_bytes!("../assets/press-start-2p.ttf");

/// The family the pixel face is registered under (see [`apply`]).
pub fn pixel_family() -> FontFamily {
    FontFamily::Name("pixel".into())
}

/// A font id in the pixel face at `size`.
pub fn pixel_font(size: f32) -> FontId {
    FontId::new(size, pixel_family())
}

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

    // The pixel face, registered alongside egui's own families and otherwise unused by the
    // theme: only the title screen asks for it, by name, through [`pixel_font`]. Installed
    // here rather than there because a font is a property of the context, and `apply` is
    // the one place this window installs properties of the context.
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "press-start-2p".to_owned(),
        FontData::from_static(PIXEL_FONT_BYTES).into(),
    );
    fonts
        .families
        .insert(pixel_family(), vec!["press-start-2p".to_owned()]);
    ctx.set_fonts(fonts);

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
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
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
    v.window_stroke = Stroke::new(3.0_f32, OUTLINE);
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
    v.menu_corner_radius = CornerRadius::same(RADIUS);
    v.extreme_bg_color = CARD;
    v.faint_bg_color = SUNKEN;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT_INK;
    v.warn_fg_color = WARN_INK;
    v.error_fg_color = BAD_INK;

    // A selection is the accent itself with the ink staying dark, which is what the game's own
    // menus do: the highlighted cell is *filled*, not tinted, so it reads from across the room.
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0_f32, OUTLINE);
    v.text_cursor.stroke = Stroke::new(2.0_f32, ACCENT_INK);

    let widgets = &mut v.widgets;
    widgets.noninteractive.bg_fill = PANEL;
    widgets.noninteractive.weak_bg_fill = PANEL;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, CONTROL_EDGE);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    widgets.noninteractive.corner_radius = CornerRadius::same(RADIUS);

    widgets.inactive.bg_fill = CARD;
    widgets.inactive.weak_bg_fill = CARD;
    widgets.inactive.bg_stroke = Stroke::new(1.0_f32, CONTROL_EDGE);
    widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    widgets.inactive.corner_radius = CornerRadius::same(RADIUS);
    widgets.inactive.expansion = 0.0;

    widgets.hovered.bg_fill = CARD_HOVER;
    widgets.hovered.weak_bg_fill = CARD_HOVER;
    // Hovered is the accent's *ink* rather than its fill: a gold rim on a cream card is 1.5:1 and
    // therefore invisible at the size these controls are drawn, while this one is 5.8:1 and is the
    // same hue the gold is. A hover a person cannot see is not feedback.
    widgets.hovered.bg_stroke = Stroke::new(1.5_f32, ACCENT_INK);
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

/// A surface to put a panel's contents on: a warm white card with the brown outline.
///
/// Used for the level shelf's entries and the canvas' floating readouts, so a card looks like a
/// card everywhere rather than like whatever frame each call site chose.
pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        // A card is often *clickable* — the shelf's entries and the canvas' floating readouts — so
        // its outline is the control boundary's weight rather than the hairline's: a plate you
        // cannot find is a plate you cannot press.
        .stroke(Stroke::new(2.0_f32, OUTLINE))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .corner_radius(CornerRadius::same(RADIUS))
}

/// A panel of chrome: the cream surface inside a thick brown outline, with the gold lip inside
/// that.
///
/// Used by every side panel in the window. Three rings rather than one — brown outline, gold
/// hairline, cream face — because that is what a menu plate in a Nintendo-style game is made of,
/// and because the gold is what makes a region read as a *menu* rather than as a region of an
/// application window.
pub fn panel_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(3.0_f32, OUTLINE))
        .shadow(drop_shadow())
        .inner_margin(egui::Margin::symmetric(10, 10))
        .corner_radius(CornerRadius::same(RADIUS_LG))
}

/// The bar the window's tools are carried in.
///
/// A raised plank rather than a panel: warmer than a panel, a thick brown outline all round, and
/// room around the controls so they read as objects resting on something rather than as a row of
/// text.
pub fn dock_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(DOCK)
        .stroke(Stroke::new(3.0_f32, OUTLINE))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .corner_radius(CornerRadius::same(RADIUS_LG))
}

/// The strip along the bottom of the window: the HUD.
///
/// Warm white and outlined like everything else, because this look has no black bars in it: the
/// status line is a *sign*, not a machine readout, and it is allowed to be as cheerful as the
/// menu above it. The strip runs edge to edge, so its corners are square where it meets the
/// window's frame.
pub fn hud_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(DOCK)
        .stroke(Stroke::new(3.0_f32, OUTLINE))
        .inner_margin(egui::Margin::symmetric(10, 5))
        .corner_radius(CornerRadius::ZERO)
}

/// The colour a *material* is drawn in on a shelf thumbnail, from the tile itself.
///
/// The shelf has to say what a level is made of without the atlas (a level the window has
/// not opened has no loaded art behind it yet), so the materials have flat colours: the
/// greens and browns of soil, the pale gold of a beach, the greys of stone, the terracotta
/// of brick and the brown of timber. They are deliberately the *palette* of the art rather
/// than the art: a thumbnail is a map of a level, and a map is not a picture.
///
/// **A brittle material is a darker shade of its material, not a colour of its own.** The
/// thumbnail's question is what a level is made of, and a cracked bridge is made of the same
/// earth as the ground it spans; the darker tone is the same sentence the crack is, said at
/// the size a thumbnail can say it, and it keeps a level's palette reading as four materials
/// rather than as eight. The spike and the pad are the two that *are* their own colours,
/// because they are the two tiles that are not a material: one ends an episode and one throws
/// a body across the level. The three elements joined them for the same reason — lava, water
/// and a vine are each a new *rule*, not a new shade of ground — and they are the only entries
/// here that are not ground at all.
pub fn material_colour(tile: sim::Tile) -> Option<Color32> {
    use sim::Tile;
    match tile {
        Tile::Empty | Tile::Goal => None,
        Tile::Solid => Some(Color32::from_rgb(96, 176, 96)),
        Tile::Cracked => Some(Color32::from_rgb(62, 118, 64)),
        Tile::Sand => Some(Color32::from_rgb(230, 206, 140)),
        Tile::CrackedSand => Some(Color32::from_rgb(178, 156, 98)),
        Tile::Brick => Some(Color32::from_rgb(196, 110, 84)),
        Tile::CrackedBrick => Some(Color32::from_rgb(146, 78, 60)),
        Tile::Plank => Some(Color32::from_rgb(176, 132, 84)),
        Tile::CrackedPlank => Some(Color32::from_rgb(126, 92, 58)),
        // A one-way shelf is *timber*, one shade lighter than the solid planks it is cut
        // from: a thumbnail should say "a thin deck of the same wood", not invent a sixth
        // material for the one tile whose distinction is vertical.
        Tile::OneWay => Some(Color32::from_rgb(206, 166, 118)),
        Tile::Spike => Some(Color32::from_rgb(226, 106, 106)),
        Tile::Spring => Some(Color32::from_rgb(110, 196, 214)),
        // The elements are their own colours for the spike's reason: none of them is a
        // material the level is made of. A map has to show a pool and a lava pit as what they
        // are, because a level's whole shape can be two pits of the same size with entirely
        // different answers.
        Tile::Water => Some(Color32::from_rgb(58, 132, 176)),
        Tile::Lava => Some(Color32::from_rgb(232, 112, 44)),
        Tile::Vine => Some(Color32::from_rgb(86, 156, 92)),
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

    /// One channel on the sRGB transfer curve, which is where "relative luminance" is defined.
    ///
    /// The plain 0..1 average above is right for "is this surface light or dark" and wrong for a
    /// contrast ratio: the ratio compares *light*, and light is linear in the physics rather than
    /// in the byte. Getting this wrong is how a palette review concludes that 3.5:1 is 4.6:1.
    fn linear(channel: u8) -> f32 {
        let c = channel as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    fn luminance(c: Color32) -> f32 {
        0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
    }

    /// The WCAG 2.x contrast ratio between two colours, 1.0..21.0.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let (x, y) = (luminance(a), luminance(b));
        let (hi, lo) = if x > y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// **Every active piece of type clears 4.5:1 on every surface it is drawn on.**
    ///
    /// The measurement the palette needed and did not have. The window has two materials and both
    /// put type on several depths: [`TEXT_DIM`] was 4.49:1 on a panel and 3.58:1 on the sunken
    /// status bar before this pass, and a caption nobody can read is a caption that may as well not
    /// be drawn. The list below is the *inventory* rather than a sample, so an ink added later
    /// without a line here is a test that has to be edited — which is the point: the floor is a
    /// claim about the window, not about one colour.
    #[test]
    fn every_type_colour_clears_the_contrast_floor() {
        let surfaces = [PANEL, CARD, CARD_HOVER, CARD_OFF, SUNKEN, DOCK, PLATE];
        for ink in [
            TEXT, TEXT_DIM, ACCENT_INK, RUN_INK, GOOD_INK, WARN_INK, BAD_INK,
        ] {
            for bg in surfaces {
                let ratio = contrast(ink, bg);
                assert!(
                    ratio >= 4.5,
                    "{ink:?} on {bg:?} is {ratio:.2}:1, under the 4.5:1 floor for small type"
                );
            }
        }
        // The tertiary ink is for panels and cards; on the sunken well its readouts step up to
        // `TEXT_DIM`, so the well is deliberately not in this list.
        for bg in [PANEL, CARD, CARD_HOVER, CARD_OFF, DOCK, PLATE] {
            let ratio = contrast(TEXT_FAINT, bg);
            assert!(
                ratio >= 4.5,
                "TEXT_FAINT on {bg:?} is {ratio:.2}:1, under the 4.5:1 floor"
            );
        }
        // Ink on the accent: it is dark on gold, and it is the one pair where a fill is behind
        // type. The window's selected control is filled, so this is type on that fill.
        assert!(contrast(INK_ON_ACCENT, ACCENT) >= 4.5);
        assert!(contrast(INK_ON_ACCENT, GOLD_EDGE) >= 4.5);
        // Type on the dark screens, and the chart's own marks. The inks are held to the text floor;
        // the series colours are *data* rather than words, so they are held to the 3:1 non-text
        // floor — a curve has to be visible, not readable.
        for bg in [SHELL, SCREEN_WELL] {
            for ink in [SCREEN_INK, SCREEN_INK_DIM] {
                assert!(contrast(ink, bg) >= 4.5, "{ink:?} on {bg:?}");
            }
            for series in [ACCENT, RUN, GOOD, WARN, BAD, COOL] {
                assert!(contrast(series, bg) >= 3.0, "{series:?} on {bg:?}");
            }
        }
    }

    /// **A boundary you press is not a hairline, and a hairline is not a boundary.**
    ///
    /// The second measurement of the pass: [`EDGE`] is a rule between surfaces and is *meant* to
    /// sit at 1.5:1, while the outline of a control has to clear 3:1 (WCAG 1.4.11) or the control
    /// reads as a word on a shadow. The chart's axes are held to the same floor against the plot
    /// they are drawn on, because a gridline nobody can see is a chart with no scale.
    #[test]
    fn a_control_boundary_is_not_a_hairline() {
        for bg in [PANEL, CARD, CARD_HOVER, CARD_OFF, SUNKEN, DOCK] {
            let ratio = contrast(CONTROL_EDGE, bg);
            assert!(
                ratio >= 3.0,
                "CONTROL_EDGE on {bg:?} is {ratio:.2}:1, under the 3:1 control floor"
            );
        }
        assert!(
            contrast(SCREEN_EDGE, SCREEN_WELL) >= 3.0,
            "the chart's axes have to be visible on the plot they are drawn on"
        );
        assert!(
            contrast(EDGE, PANEL) < 2.0,
            "EDGE is the decorative rule, and a decorative rule that shouts is furniture"
        );
        // The hover state is a *state*, so it is held to the control floor too: it used to be a
        // gold rim, which is 1.5:1 on a card and therefore no feedback at all.
        let hover = visuals().widgets.hovered.bg_stroke.color;
        assert!(contrast(hover, CARD) >= 3.0, "hover on a card: {hover:?}");
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
        assert!(
            luma(SCREEN_INK) > 190.0,
            "type on a dark plot has to be light"
        );
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
            assert_eq!(
                style.spacing.button_padding,
                egui::vec2(12.0, 7.0),
                "{theme:?}"
            );
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
    /// has none — the four materials, their four brittle selves, the hazard and the pad, and
    /// nothing drawn for air.
    #[test]
    fn every_material_has_a_thumbnail_colour() {
        use sim::Tile;
        assert!(material_colour(Tile::Empty).is_none());
        assert!(material_colour(Tile::Goal).is_none());
        let solids = [
            Tile::Solid,
            Tile::Cracked,
            Tile::Sand,
            Tile::CrackedSand,
            Tile::Brick,
            Tile::CrackedBrick,
            Tile::Plank,
            Tile::CrackedPlank,
            Tile::Spike,
            Tile::Spring,
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
