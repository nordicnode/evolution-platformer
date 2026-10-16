//! The window's own controls: chunky, bevelled, and the same wherever they appear.
//!
//! What the window was is egui's stock buttons in a row, and that is not a *look* — it is what a
//! tool that has no opinion about its own furniture renders as. What it should be is a game's
//! tool bar, where a control is a thing you press: a plate of plastic in a socket, lit along its
//! top edge, shaded along its bottom, that moves when you push it.
//!
//! Three ideas carry that, and they are the whole module.
//!
//! * **A control is a plate in a socket, not a rectangle.** [`bevel`] resolves one
//!   [`PlateState`] and one rectangle into every colour and every sub-rectangle the control
//!   needs — face, lit edge, shaded edge, and where its content sits. It is a pure function with
//!   no `Ui`, no painter and no input, which is what lets the look be *asserted*: that a pressed
//!   plate's content sits below a resting one's, that a selected plate wears the accent and the
//!   text goes dark, that a disabled one is quiet rather than merely dimmer.
//! * **Feedback is geometry, not decoration.** A hovered plate lifts its face colour, a pressed
//!   one sinks its content [`theme::PRESS_SINK`] pixels and *inverts* its edges (the dark edge
//!   moves to the top, which is what an inset face looks like), and a selected one fills with
//!   the palette's gold. Nothing animates: the state is a fact about the pointer, and a
//!   spring-damped bounce at forty points reads as a web page rather than as a game.
//! * **The game's own art is the icons, and every control says its own name.** A control draws a
//!   sprite out of the shipped atlas — the ground tile for the brush, the runner for play, a
//!   walker for the population — so the tool bar is made of the same sixteen-pixel pictures as
//!   the level behind it. The picture is half the job; the word under it is the other half. A bar
//!   of bare pictures is a puzzle rather than a tool bar, and the state a person most needs to
//!   read is the one where they have pressed nothing yet — so a caption is not something a
//!   control earns by being pressed, it is what the control *is*. The tooltip still carries the
//!   sentence the word cannot ("step back through the stroke history — Ctrl+Z").
//!
//! [`theme::RADIUS_LG`] is the corner these draw with and [`theme::RADIUS`] is what every small
//! widget in the window keeps: things you press are rounded, things you read are square-ish.

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Response, Sense, Ui,
    Vec2,
};

use crate::atlas::Atlas;
use crate::theme;

/// The one plate: a document tool, a mode cell, *Fit*, *Restart*, *Details*.
///
/// A **minimum**, not a size. Every plate is as wide as the caption it has to show
/// ([`caption_width`]) and never narrower than this, so a long word *widens* its plate instead of
/// being clipped by it — which is the whole of what stops a row of captioned controls from looking
/// broken, and it is a construction rather than a hope: nothing is allocated until the text has been
/// measured. The height is this number always, so a row of plates still lines up along one edge.
pub const TOOL_SIZE: Vec2 = Vec2::new(52.0, 52.0);
/// The square a glyph is drawn in inside a plate that also carries a caption — above the word in the
/// dock, beside it in a menu row.
///
/// Public because the palette draws its own cells rather than going through [`icon_button`] — a
/// brush cell is a drag source and needs gestures a button does not — and its pictures have to be
/// the same size as the dock's or the two rows read as furniture from two different windows.
pub const ICON_BOX: f32 = 20.0;
/// The shape of a menu row: a wide plate in a list, the glyph at the left and the word after it.
///
/// Shorter than a dock plate on purpose. A *column* of 52-point squares in a 300-point panel is a
/// wall of plates with a third of each one empty; the row shape is what makes a list of controls
/// read as a menu, which is the shape a run's own controls should have — it is the shape a game
/// pauses into.
pub const MENU_ROW: Vec2 = Vec2::new(210.0, 34.0);
/// How far a caption sits from the glyph it belongs to.
const LABEL_GAP: f32 = 3.0;
/// A control's content inset: how far its face is kept clear of its own lip.
const CELL_PAD: f32 = 5.0;
/// The type size of every caption in the window.
///
/// Proportional, because a caption is a word rather than a number: the monospace face is for
/// readings, which have to line up in columns, and a tool bar is not a column — at this size it also
/// sets about a fifth more of the word per point than monospace does, which is width the dock does
/// not have to spend.
const CAPTION_FONT: f32 = 12.5;

/// The five glyphs the window's own controls draw, by name.
///
/// A named list rather than five string literals at five call sites, because a glyph whose name
/// disagrees with the atlas is **invisible**: [`icon_button`] falls back to drawing the control's
/// name, so a typo is a plate with a word on it — no panic, no blank, nothing anyone would call a
/// bug. The art side has the same failure mode from the other end (a glyph renamed in
/// `tools/art/ui.py` and not here). One list means one place for both, and
/// `crate::atlas::tests::every_glyph_the_window_draws_is_in_the_atlas` is the thing that looks at
/// it.
pub const GLYPH_UNDO: &str = "ui_undo";
pub const GLYPH_REDO: &str = "ui_redo";
pub const GLYPH_FIT: &str = "ui_fit";
pub const GLYPH_INFO: &str = "ui_info";
pub const GLYPH_RESTART: &str = "ui_restart";
/// Two halves leaving a dashed axis, for the one tool that acts on the *whole level* rather than
/// on a tile: flipping the document left to right.
pub const GLYPH_MIRROR: &str = "ui_mirror";
/// The run's own transport, for the four things a run does that a document does not: pause, play,
/// one step, and the champion it can write out.
pub const GLYPH_PAUSE: &str = "ui_pause";
pub const GLYPH_PLAY: &str = "ui_play";
pub const GLYPH_STEP: &str = "ui_step";
pub const GLYPH_MEDAL: &str = "ui_medal";
/// Every one of them, in the order the atlas draws them.
///
/// Test-only, like `view::backdrop_colours`, and for the same reason: the app asks for the nine of
/// them by name at nine call sites, so a list of all nine is a fact *about* the app that only a
/// check needs — and a constant nothing in a running window reads would otherwise be a permanent
/// dead-code warning, which is exactly the kind of nag that gets an assertion deleted.
#[cfg(test)]
pub const GLYPHS: [&str; 10] = [
    GLYPH_MIRROR,
    GLYPH_UNDO,
    GLYPH_REDO,
    GLYPH_FIT,
    GLYPH_INFO,
    GLYPH_RESTART,
    GLYPH_PAUSE,
    GLYPH_PLAY,
    GLYPH_STEP,
    GLYPH_MEDAL,
];

/// The width a plate has to be to show `text` as a caption: the text's own width, the plate's own
/// padding, and its one-point lip on each side.
///
/// Called by both widgets *before* they allocate anything, so that "a caption is never clipped" is
/// true by construction. Measured in the font the caption will be drawn in, inside a begun frame: a
/// width estimated from a character count, or measured in some other face, would prove nothing about
/// what ends up on screen.
pub fn caption_width(ui: &Ui, text: &str) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        FontId::new(CAPTION_FONT, FontFamily::Proportional),
        theme::TEXT,
    );
    galley.size().x + CELL_PAD * 2.0 + 2.0
}

/// Everything a control's look depends on.
///
/// A struct rather than four booleans threaded through a drawing function, because the *combinations*
/// are the look: selected-and-hovered is gold with a lift, selected-and-pressed is gold and sunk,
/// and a disabled control ignores the pointer entirely rather than drawing hover feedback on
/// something that cannot be pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlateState {
    pub hovered: bool,
    pub pressed: bool,
    pub selected: bool,
    pub enabled: bool,
}

impl Default for PlateState {
    /// A resting, enabled plate: a default that drew a disabled control would make every call site
    /// that forgot a field *look* broken rather than behave broken, which is the harder bug to see.
    fn default() -> Self {
        PlateState {
            hovered: false,
            pressed: false,
            selected: false,
            enabled: true,
        }
    }
}

impl PlateState {
    /// The state of a widget that egui has already been asked about.
    pub fn of(response: &Response, selected: bool, enabled: bool) -> PlateState {
        PlateState {
            // A disabled control does not respond to the pointer at all: hover and press feedback
            // on something that cannot be pressed is a lie the pointer tells the eye.
            hovered: enabled && response.hovered(),
            pressed: enabled && response.is_pointer_button_down_on(),
            selected,
            enabled,
        }
    }
}

/// A plate's resolved geometry and colours: one control, ready to paint.
#[derive(Clone, Copy, Debug)]
pub struct Bevel {
    /// The whole control, including its socket lip.
    pub socket: Rect,
    /// The face: the socket inset by one pixel, which is the lip.
    pub face: Rect,
    /// Where the content (icon, label) goes. Sunk by [`theme::PRESS_SINK`] when pressed.
    pub content: Rect,
    /// The face's fill.
    pub fill: Color32,
    /// The lit edge: along the top when resting, along the bottom when pressed.
    pub light: Color32,
    /// The shaded edge: the other of the two.
    pub dark: Color32,
    /// The content's colour.
    pub text: Color32,
}

/// Resolve one control's look. Pure: no `Ui`, no painter, no input.
///
/// The five faces below are the whole vocabulary, in the order they are tested: a disabled control
/// is half a card and grey text; a selected one is the palette's gold with the frame's colour for
/// its text (so a gold plate reads as *pressed in* rather than as a lit button, which is what a
/// selected tool should read as); a pressed one inverts its edges and darkens; a hovered one lifts
/// a surface and brightens its type; and a resting one is a card.
pub fn bevel(state: PlateState, rect: Rect) -> Bevel {
    let pressed = state.pressed && state.enabled;
    let face = rect.shrink(1.0);
    // The content is inset far enough that an icon or a word never touches the lip, and the
    // pressed sink is applied *here* rather than to the face: a face that moved would take the
    // socket's lit edge with it, and the two edges are what say which way the plate went.
    let mut content = face.shrink(CELL_PAD);
    if pressed {
        content = content.translate(Vec2::new(0.0, theme::PRESS_SINK));
    }

    // The five faces, in the order they are tested. Two of them are worth a word, because on a
    // *light* chrome the depth has to be carried by the shaded edge rather than the lit one: a
    // white top edge on a near-white face is invisible, so a resting plate reads as raised
    // because of what is under it and because it casts a shadow, not because of a highlight.
    let (fill, light, dark, text) = if !state.enabled {
        (
            theme::CARD_OFF,
            theme::EDGE,
            theme::EDGE.gamma_multiply(0.92),
            theme::TEXT_FAINT,
        )
    } else if state.selected {
        (
            theme::ACCENT,
            theme::ACCENT.gamma_multiply(1.22),
            theme::GOLD_EDGE.gamma_multiply(0.72),
            theme::INK_ON_ACCENT,
        )
    } else if pressed {
        (
            // Pressed is darker *and* inset: the face has gone into its socket, so less light
            // reaches it.
            theme::CARD.gamma_multiply(0.9),
            // Inverted on purpose: the light is under the content and the shadow above it, which
            // is what a face pushed *into* its socket looks like from the front.
            theme::BEVEL_DARK,
            theme::BEVEL_LIGHT,
            theme::TEXT,
        )
    } else if state.hovered {
        (
            theme::CARD_HOVER,
            theme::BEVEL_LIGHT,
            theme::BEVEL_DARK,
            theme::TEXT,
        )
    } else {
        (
            theme::CARD,
            theme::BEVEL_LIGHT,
            theme::BEVEL_DARK,
            theme::TEXT,
        )
    };

    Bevel {
        socket: rect,
        face,
        content,
        fill,
        light,
        dark,
        text,
    }
}

/// Paint a resolved plate: the socket, the face, then its two edges.
pub fn paint(painter: &Painter, b: &Bevel) {
    let radius = CornerRadius::same(theme::RADIUS_LG);
    // The socket first, filled with the plate's *shaded* colour: that ring is the lip the face
    // sits in, and it is why a resting button already has an outline to press into.
    painter.rect_filled(b.socket, radius, b.dark);
    painter.rect_filled(b.face, radius, b.fill);
    // The two edges, drawn inside the face and inset from its corners so they do not fight the
    // rounded lip at the ends.
    let inset = 3.0;
    let w = (b.face.width() - inset * 2.0).max(0.0);
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(b.face.min.x + inset, b.face.min.y + 1.5),
            Vec2::new(w, 1.0),
        ),
        CornerRadius::ZERO,
        b.light,
    );
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(b.face.min.x + inset, b.face.max.y - 2.5),
            Vec2::new(w, 1.0),
        ),
        CornerRadius::ZERO,
        b.dark,
    );
}

/// One glyph, centred in `rect` and clamped to the smaller of its two sides.
///
/// Shared by every control that carries a picture, so the icon on a dock plate, in a switch cell, in
/// a menu row and in a palette cell are all placed by one rule — the caller hands in the box the
/// picture may occupy and this fits the picture to it, which is what lets the same glyph be a
/// 20-point square in the dock and a smaller mark beside a word.
///
/// A disabled plate keeps its glyph and **fades** it. That is the only way a dark sprite can say
/// "not now": the tint multiplies, so it can carry a picture towards the plate behind it and can
/// never take it away from it.
fn draw_glyph(
    painter: &Painter,
    rect: Rect,
    atlas: Option<&Atlas>,
    icon: Option<&str>,
    enabled: bool,
) -> bool {
    let Some((texture, uv)) = atlas
        .zip(icon)
        .and_then(|(atlas, name)| atlas.uv(name).ok().map(|uv| (atlas.texture_id(), uv)))
    else {
        return false;
    };
    let side = rect.width().min(rect.height()).max(0.0);
    let box_rect = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let tint = if enabled {
        Color32::WHITE
    } else {
        Color32::from_white_alpha(80)
    };
    painter.image(texture, box_rect, uv, tint);
    true
}

/// A picture **and its name** on a plate you can press: the window's control.
///
/// Both, always. Every glyph is either the thing the control acts on — the signpost for the pack of
/// levels, the crate a save writes, the flag a replay shows — or, for the few the world has no
/// picture of, a `ui_*` glyph drawn for exactly this row (`tools/art/ui.py`); and every one of them
/// carries its caption underneath. A bar of bare pictures is a puzzle: this window is a *tool* as
/// well as a game, and the one thing a person cannot do with a picture is look it up. The tooltip
/// explains what the control does; the caption says what it is.
///
/// The plate is **as wide as its caption** (or `min_size`, whichever is larger), so nothing is ever
/// clipped — the same construction the switch uses, and the reason a long word is a wider plate
/// rather than a cut-off one. If the plate is too short to stack a glyph over a word, the glyph goes
/// and the caption stays: a legible name is the control, and the picture is the decoration.
///
/// The atlas is optional and so is the sprite: a tileset drawn before the `ui_*` glyphs existed still
/// runs with captions on its plates and no pictures. A tool that vanished when an asset went missing
/// would be worse than one that is merely plain — the rule the palette's brush cells follow too.
pub fn icon_button(
    ui: &mut Ui,
    atlas: Option<&Atlas>,
    icon: Option<&str>,
    name: &str,
    min_size: Vec2,
    selected: bool,
    enabled: bool,
) -> Response {
    let size = Vec2::new(min_size.x.max(caption_width(ui, name)), min_size.y);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let state = PlateState::of(&response, selected, enabled);
    let b = bevel(state, rect);
    let painter = ui.painter_at(rect);
    paint(&painter, &b);
    stack(&painter, &b, atlas, icon, name, enabled);
    // A plate that can be pressed says so with the pointer, the way every control on the machine
    // does (AUDIT.md F5). The rule is *enabled* rather than *hovered alone*, because a disabled
    // plate that offers a hand is worse than one that offers nothing: the icon is the same and
    // the click does not happen.
    if enabled && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// A glyph over its caption, both centred on the plate's own axis.
///
/// The stacked layout the dock's plates and the switch's cells both wear, in one place so the two
/// cannot drift apart — a tool bar and the switch beside it are the same object as far as the eye is
/// concerned. The glyph takes whatever height the caption leaves and *shrinks* to it, and a plate too
/// short for both keeps the word and drops the picture: a legible name is the control, the picture is
/// the decoration.
fn stack(
    painter: &Painter,
    b: &Bevel,
    atlas: Option<&Atlas>,
    icon: Option<&str>,
    name: &str,
    enabled: bool,
) {
    let galley = painter.layout_no_wrap(
        name.to_string(),
        FontId::new(CAPTION_FONT, FontFamily::Proportional),
        b.text,
    );
    let caption_x = b.content.center().x - galley.size().x * 0.5;
    let room = b.content.height() - LABEL_GAP - galley.size().y;
    let side = ICON_BOX.min(room).min(b.content.width());
    if side >= 8.0 && draw_glyph(painter, glyph_box(b, side), atlas, icon, enabled) {
        painter.galley(
            Pos2::new(caption_x, b.content.max.y - galley.size().y),
            galley,
            b.text,
        );
    } else {
        painter.galley(
            Pos2::new(caption_x, b.content.center().y - galley.size().y * 0.5),
            galley,
            b.text,
        );
    }
}

/// The box a glyph occupies on a stacked plate: centred horizontally, hung from the top of the
/// content so the caption below it has the same baseline on every plate in the row, whatever the
/// caption's own height.
fn glyph_box(b: &Bevel, side: f32) -> Rect {
    Rect::from_min_size(
        Pos2::new(b.content.center().x - side * 0.5, b.content.min.y),
        Vec2::splat(side),
    )
}

/// A named row in a vertical menu: the glyph at the left, the caption after it, and the whole row a
/// plate you can press.
///
/// The other half of the same vocabulary. The dock is a horizontal bar, so its plates stack a glyph
/// over a word; a panel is a column, so a row puts the glyph at the left and the word beside it —
/// which is what makes a list of controls look like a menu rather than like a rack of square buttons
/// with the words in a footnote. `width` is the row's own: a menu is a *column of rows*, and a row
/// that shrank to fit its caption would leave the column ragged.
pub fn menu_row(
    ui: &mut Ui,
    atlas: Option<&Atlas>,
    icon: Option<&str>,
    name: &str,
    width: f32,
    selected: bool,
    enabled: bool,
) -> Response {
    let size = Vec2::new(width.max(caption_width(ui, name)), MENU_ROW.y);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let state = PlateState::of(&response, selected, enabled);
    let b = bevel(state, rect);
    let painter = ui.painter_at(rect);
    paint(&painter, &b);

    // The glyph first: it is what the eye lands on, and the caption starts after it so that a
    // column of rows has its words on one vertical line whatever pictures they carry.
    let side = ICON_BOX.min(b.content.height()).min(b.content.width());
    let mut text_x = b.content.min.x;
    if side >= 8.0
        && draw_glyph(
            &painter,
            Rect::from_min_size(b.content.left_center() - Vec2::new(0.0, side * 0.5), Vec2::splat(side)),
            atlas,
            icon,
            enabled,
        )
    {
        text_x += side + LABEL_GAP + 2.0;
    }
    let galley = painter.layout_no_wrap(
        name.to_string(),
        FontId::new(CAPTION_FONT, FontFamily::Proportional),
        b.text,
    );
    painter.galley(
        Pos2::new(text_x, b.content.center().y - galley.size().y * 0.5),
        galley,
        b.text,
    );
    response
}

/// The well a segmented switch's cells sit in: one long inset socket, edged, so the group reads as
/// one object rather than as a row of plates that happen to be near each other.
fn paint_well(painter: &Painter, outer: Rect) {
    painter.rect_filled(outer, CornerRadius::same(theme::RADIUS_LG), theme::SUNKEN);
    painter.rect_stroke(
        outer,
        CornerRadius::same(theme::RADIUS_LG),
        egui::Stroke::new(1.0_f32, theme::EDGE),
        egui::StrokeKind::Inside,
    );
}

/// A segmented switch: a socket holding several plates, one of them pressed in.
///
/// The mode switch, and anything else that is a *choice between exclusive things* rather than a row
/// of independent buttons. Each cell is a plate inside the well, so the chosen one reads as the cell
/// standing out of the group rather than as one of four unrelated buttons that happens to be gold.
///
/// **Every cell carries its own name**, in the same stacked shape as the dock's plates — always, not
/// only once it is the chosen one. A switch whose cells are bare pictures until you pick them makes
/// the window unreadable in exactly the state a person is in when they need to read it: the one
/// where they have not picked yet.
///
/// The cells are all **one size**, taken from the widest name in the set (or [`TOOL_SIZE`],
/// whichever is larger), so no name is clipped and the switch does not change shape when the choice
/// does: a group that widened for *Evolve* and narrowed for *Play* would shove every control to its
/// right sideways on every mode change.
///
/// Returns the index that was clicked this frame, if any — the caller decides what a choice *does*;
/// the widget only says which one the pointer picked — together with one response for the group,
/// which is what a caller hangs its own hover sentence on.
pub fn segmented(
    ui: &mut Ui,
    atlas: Option<&Atlas>,
    options: &[(Option<&str>, &str)],
    selected: usize,
    enabled: &[bool],
) -> (Option<usize>, Response) {
    // Every cell is measured before anything is allocated: a plate clips its content, so a cell
    // sized by a guess is a clipped word — the kind of defect no assertion about colours sees and
    // only a rendered frame ever did. The width is the *widest* name in the set rather than each
    // cell's own, so the switch keeps one size whatever is chosen: a group that grew for `Evolve`
    // and shrank for `Play` would move every control to its right on every mode change.
    let width = options
        .iter()
        .map(|(_, label)| caption_width(ui, label))
        .fold(TOOL_SIZE.x, f32::max);
    let widths: Vec<f32> = options.iter().map(|_| width).collect();
    let height = TOOL_SIZE.y;
    let total = widths.iter().sum::<f32>() + 8.0;
    let (outer, _) = ui.allocate_exact_size(Vec2::new(total, height + 8.0), Sense::hover());
    let painter = ui.painter_at(outer);
    paint_well(&painter, outer);

    let mut clicked = None;
    let mut hit = ui.interact(outer, ui.id().with("segments"), Sense::hover());
    let mut x = outer.min.x + 4.0;
    for (i, (icon, label)) in options.iter().enumerate() {
        let cell = Rect::from_min_size(
            Pos2::new(x, outer.min.y + 4.0),
            Vec2::new(widths[i], height),
        );
        x += widths[i];
        let response = ui.interact(cell, ui.id().with(("segment", i)), Sense::click());
        let allowed = enabled.get(i).copied().unwrap_or(true);
        let state = PlateState::of(&response, i == selected, allowed);
        let b = bevel(state, cell);
        paint(&painter, &b);
        stack(&painter, &b, atlas, *icon, label, allowed);
        if response.clicked() && allowed {
            clicked = Some(i);
        }
        // The cells' own pointer feedback is the *look*; the whole switch answers with one
        // response, so a caller can hang a sentence on the group ("the shape of the stroke")
        // without eleven tooltips on eleven cells.
        if i == selected || (clicked.is_none() && response.hovered()) {
            hit = response;
        }
    }
    (clicked, hit)
}

/// A switch whose cells are all a caller's size and all carry their own word.
///
/// The palette's stroke row is three words in a 190-point panel; the dock's switch is four pictures
/// on a bar. Both are the same control — one well, one chosen cell — and should look the same in
/// both places, which is what this exists for. It is the *labelled* mode: every cell says what it
/// is, because the thing it chooses between has no picture (one tile at a time, a box, a line).
pub fn segmented_sized(
    ui: &mut Ui,
    atlas: Option<&Atlas>,
    options: &[(Option<&str>, &str)],
    selected: usize,
    enabled: &[bool],
    cell_size: Vec2,
) -> (Option<usize>, Response) {
    let cells = options.len() as f32;
    let size = Vec2::new(cell_size.x * cells + 8.0, cell_size.y + 8.0);
    let (outer, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(outer);
    paint_well(&painter, outer);

    let mut clicked = None;
    let mut hit = ui.interact(outer, ui.id().with("segments"), Sense::hover());
    for (i, (icon, label)) in options.iter().enumerate() {
        let cell = Rect::from_min_size(
            Pos2::new(outer.min.x + 4.0 + i as f32 * cell_size.x, outer.min.y + 4.0),
            cell_size,
        );
        let id = ui.id().with(("segment", i));
        let response = ui.interact(cell, id, Sense::click());
        let allowed = enabled.get(i).copied().unwrap_or(true);
        let state = PlateState::of(&response, i == selected, allowed);
        let b = bevel(state, cell);
        paint(&painter, &b);
        // An icon and a word share a cell here, so the glyph is the small box and the word goes
        // after it; with no icon the word is centred on the cell's own axis. Either way it is
        // centred on the cell's *height*, which is where an eye expects a label in a control it
        // presses.
        let side = ICON_BOX.min(b.content.height()).min(b.content.width());
        let with_icon = side >= 8.0
            && draw_glyph(
                &painter,
                Rect::from_min_size(
                    b.content.left_center() - Vec2::new(0.0, side * 0.5),
                    Vec2::splat(side),
                ),
                atlas,
                *icon,
                allowed,
            );
        let galley = painter.layout_no_wrap(
            (*label).to_string(),
            FontId::new(CAPTION_FONT, FontFamily::Proportional),
            b.text,
        );
        let text_x = if with_icon {
            b.content.min.x + side + LABEL_GAP
        } else {
            b.content.center().x - galley.size().x * 0.5
        };
        painter.galley(
            Pos2::new(text_x, b.content.center().y - galley.size().y * 0.5),
            galley,
            b.text,
        );
        if response.clicked() && allowed {
            clicked = Some(i);
        }
        if i == selected || (clicked.is_none() && response.hovered()) {
            hit = response;
        }
    }
    (clicked, hit)
}

/// A sign over the canvas or in the dock: a plate, a coloured lamp, and one line of type.
///
/// Used for the live reading of a run and for messages that have to be *seen* rather than read out
/// of a status bar. The lamp is the colour's job (green for a win, amber for a stall, the accent
/// for progress) and the type is monospace, because a banner's content is usually numbers.
/// Returns the width it drew at, or `None` when the sign did not fit in `room` — the measurement is
/// the galley's own, so a caller that gates on it cannot disagree with what was painted. That is
/// what lets the dock keep its reading while a window is dragged narrow: the reading disappears
/// whole, rather than being cut in half by a clip rectangle.
pub fn banner(ui: &mut Ui, text: &str, lamp: Color32, room: f32) -> Option<f32> {
    let font = FontId::new(theme::MONO_SIZE, FontFamily::Monospace);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font.clone(), theme::TEXT);
    let size = Vec2::new(galley.size().x + 34.0, 26.0);
    if size.x > room {
        return None;
    }
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect.translate(Vec2::new(0.0, 2.0)),
        CornerRadius::same(theme::RADIUS_LG),
        theme::SHADOW,
    );
    painter.rect_filled(rect, CornerRadius::same(theme::RADIUS_LG), theme::PLATE);
    painter.rect_stroke(
        rect,
        CornerRadius::same(theme::RADIUS_LG),
        egui::Stroke::new(1.0_f32, theme::EDGE),
        egui::StrokeKind::Inside,
    );
    // The lamp: a filled disc with a dimmer ring, so it reads as lit rather than as a bullet point.
    let lamp_centre = Pos2::new(rect.min.x + 14.0, rect.center().y);
    painter.circle_filled(lamp_centre, 5.0, lamp);
    painter.circle_stroke(
        lamp_centre,
        5.0,
        egui::Stroke::new(1.0_f32, lamp.gamma_multiply(0.5)),
    );
    painter.galley(
        Pos2::new(rect.min.x + 26.0, rect.center().y - galley.size().y * 0.5),
        galley,
        theme::TEXT,
    );
    Some(size.x)
}

/// One row of a board: what is being read on the left, the reading itself on the right, and both on
/// the panel's own margins.
///
/// This is what turns a column of prose into a *board*. Six lines of `ui.small(format!(...))` are six
/// sentences, and the eye has to parse each one to find the number in it; the same six as label and
/// value line up down the page as a column of numbers, which is the thing a person opens a details
/// window to compare. It is one function rather than four call sites for the reason `panel::section`
/// is one: the next row added gets the same treatment by existing, and a board cannot drift
/// half-aligned.
///
/// The value is right-aligned and set in the monospace the readings use, so a column of them has one
/// right edge whatever their widths — and a long label wraps nothing, because the two are laid out
/// in one line rather than as a stack.
pub fn stat_row(ui: &mut Ui, label: &str, value: impl Into<String>) {
    // The label gets exactly half of the row's width and the value the rest, rather than the label
    // sizing itself and the value taking what is left. A row laid out the other way round has no
    // floor under it: a label long enough to reach the value's own right edge makes the value's
    // rectangle zero or negative-width, and egui then draws the two over each other — a legibility
    // failure that only appears at the one label length nobody measured, which is why it is a fixed
    // column instead of a hope. `LABEL_COLUMN` is a floor as well as a half, so a *narrow* panel
    // (a board in a window dragged small) keeps the label readable and truncates instead of
    // overprinting.
    let half = (ui.available_width() * 0.5).max(72.0);
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(half, ROW_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(label)
                            .small()
                            .color(theme::TEXT_DIM),
                    )
                    .truncate(),
                );
            },
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(value.into())
                        .monospace()
                        .color(theme::TEXT),
                )
                .truncate(),
            );
        });
    });
}

/// The height a [`stat_row`] reserves for its label column. One line and no more: a board is a
/// column of readings, and a label that wrapped to two lines would break the column's spacing at
/// exactly one row in ten.
const ROW_HEIGHT: f32 = 14.0;

/// The window's title: what the document is, and one line about it.
///
/// A plaque rather than a heading, because the name of the level is the one piece of type in the
/// window that is *about* the thing on screen — it reads as a sign hanging on the tool bar, which
/// is where the eye goes first. Its [`Response`] is returned so the sign can carry a tooltip about
/// the file behind it without wrapping the whole thing in a second layout.
pub fn plaque(ui: &mut Ui, title: &str, subtitle: &str) -> Response {
    egui::Frame::new()
        .fill(theme::PLATE)
        .corner_radius(CornerRadius::same(theme::RADIUS_LG))
        .inner_margin(egui::Margin::symmetric(12, 5))
        .show(ui, |ui| {
            // A *frame* inherits its parent's layout, and a plaque lives on a horizontal bar:
            // without this the name and the file would be laid out beside each other as one long
            // line, which is a caption rather than a sign and takes a hundred points the dock needs.
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(
                    egui::RichText::new(title)
                        .font(FontId::new(theme::HEADING_SIZE, FontFamily::Proportional))
                        .color(theme::TEXT),
                );
                if !subtitle.is_empty() {
                    ui.label(
                        egui::RichText::new(subtitle)
                            .font(FontId::new(theme::SMALL_SIZE, FontFamily::Monospace))
                            .color(theme::TEXT_DIM),
                    );
                }
            });
        })
        .response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_min_size(Pos2::new(10.0, 20.0), TOOL_SIZE)
    }

    /// The pressed plate is the *lower* one, and the resting plate is where the content stays:
    /// this is the whole of what "tactile" means, and it is the one claim a look can make that a
    /// screenshot cannot check.
    #[test]
    fn a_pressed_plate_sinks_its_content_and_a_resting_one_does_not() {
        let rest = bevel(PlateState::default(), rect());
        let down = bevel(
            PlateState {
                pressed: true,
                ..PlateState::default()
            },
            rect(),
        );
        assert_eq!(
            down.content.min.y - rest.content.min.y,
            theme::PRESS_SINK,
            "a pressed plate's content must sit exactly the press depth below a resting one's"
        );
        assert_eq!(down.content.min.x, rest.content.min.x, "and not move sideways");
        // The *face* does not move: the socket's edges are what say which way the plate went.
        assert_eq!(down.face, rest.face);

        // A disabled plate ignores a press rather than sinking, which is the difference between
        // "not now" and "you are pushing a dead control".
        let off = bevel(
            PlateState {
                pressed: true,
                enabled: false,
                ..PlateState::default()
            },
            rect(),
        );
        assert_eq!(off.content.min.y, rest.content.min.y);
    }

    /// A pressed plate is inset — its shaded edge above its lit one — and a resting plate is raised.
    /// Reversing the pair is how a control says "pushed in", and it is invisible to anything that
    /// only checks that the colours differ.
    #[test]
    fn a_pressed_plate_inverts_its_edges() {
        fn luma(c: Color32) -> f32 {
            0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32
        }
        let rest = bevel(PlateState::default(), rect());
        let down = bevel(
            PlateState {
                pressed: true,
                ..PlateState::default()
            },
            rect(),
        );
        // Asserted as the *relation* rather than as two colour expressions, because the relation
        // is the claim and it has to survive a retune of either surface — this pass retuned both.
        // A raised face is lit along its top edge and shaded along its bottom one; an inset face
        // is the other way up.
        assert!(
            luma(rest.light) > luma(rest.dark),
            "a resting plate should be lit above and shaded below"
        );
        assert!(
            luma(down.light) < luma(down.dark),
            "a pressed plate should be shaded above and lit below"
        );
        assert_ne!(down.fill, rest.fill, "and its face darkens as well");
        assert!(
            luma(down.fill) < luma(rest.fill),
            "and darkens towards the socket rather than away from it"
        );
    }

    /// The five states are five different faces. A theme that draws a selected tool the same as a
    /// hovered one has no way to say which tool is *on*, which is the one question a tool bar
    /// exists to answer.
    #[test]
    fn every_state_has_its_own_face() {
        let off = bevel(
            PlateState {
                enabled: false,
                ..PlateState::default()
            },
            rect(),
        );
        let rest = bevel(PlateState::default(), rect());
        let hover = bevel(
            PlateState {
                hovered: true,
                ..PlateState::default()
            },
            rect(),
        );
        let down = bevel(
            PlateState {
                pressed: true,
                ..PlateState::default()
            },
            rect(),
        );
        let chosen = bevel(
            PlateState {
                selected: true,
                ..PlateState::default()
            },
            rect(),
        );
        let fills = [off.fill, rest.fill, hover.fill, down.fill, chosen.fill];
        for (i, a) in fills.iter().enumerate() {
            for b in &fills[i + 1..] {
                assert_ne!(a, b, "two states share a face colour: {fills:?}");
            }
        }
        // And the selected one is the palette's gold, with the ink that is readable on it: a gold
        // plate with light type on top is unreadable, and it is the mistake this assertion exists
        // to catch.
        assert_eq!(chosen.fill, theme::ACCENT);
        assert_eq!(chosen.text, theme::INK_ON_ACCENT);
        // A disabled plate is quiet: grey type rather than white, and it refuses the pointer
        // rather than merely drawing dimmer — a pressed, hovered state on a control that cannot
        // be pressed is the pointer lying to the eye.
        assert_eq!(off.text, theme::TEXT_FAINT);
        let shouting = PlateState {
            hovered: true,
            pressed: true,
            selected: false,
            enabled: false,
        };
        let ignored = bevel(shouting, rect());
        assert_eq!(ignored.fill, off.fill, "a dead control does not light up");
        assert_eq!(ignored.content, off.content, "and does not sink");
    }

    /// The widgets really draw: a button, a switch and a banner each put geometry on the screen in
    /// a headless context, and the switch reports the cell the pointer is over.
    ///
    /// Counted in *vertices*, because that is the only thing that separates "drew a chunky plate"
    /// from "allocated space and painted nothing", and a look that does not reach the screen is not
    /// a look.
    #[test]
    fn the_controls_paint_and_the_switch_reports_the_cell_under_the_pointer() {
        use egui::{Pos2, RawInput, Rect, Vec2};
        let size = Vec2::new(600.0, 200.0);
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);

        let mut drawn = 0usize;
        let output = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default())
                    .show(ctx, |ui| {
                        let r = icon_button(
                            ui,
                            None,
                            None,
                            "Save",
                            TOOL_SIZE,
                            false,
                            true,
                        );
                        assert!(!r.clicked(), "a button nobody clicked is not a click");
                        icon_button(ui, None, Some("ground_1"), "Ground", TOOL_SIZE, true, true);
                        // A disabled control answers no pointer: the state resolver gates hover and
                        // press on `enabled`, which is the difference between "not now" and "you are
                        // pushing a dead control".
                        let stuck = icon_button(ui, None, None, "Stuck", TOOL_SIZE, false, false);
                        let dead = PlateState::of(&stuck, false, false);
                        assert!(!dead.hovered && !dead.pressed);
                        let (picked, row) = segmented(
                            ui,
                            None,
                            &[(None, "Edit"), (None, "Play"), (None, "Evolve")],
                            0,
                            &[true, true, false],
                        );
                        assert_eq!(picked, None, "no pointer, no choice");
                        // The chosen cell names itself, so it is wider than the pictures beside it —
                        // which is the whole of what makes the switch's own width follow the choice.
                        // Measured on the cell rather than on the group because the response this
                        // returns *is* a cell's: it is the one a caller hangs a sentence on.
                        assert!(
                            row.rect.width() >= TOOL_SIZE.x,
                            "a switch cell is narrower than a plate: {} points",
                            row.rect.width()
                        );
                        // The group answers as a whole too, which is what a caller hangs its
                        // own sentence on rather than tooltipping every cell.
                        assert!(!row.hovered(), "no pointer, no hover either");
                        let sign = banner(ui, "gen 43/300  best 218.2  wins 2/200", theme::GOOD, 400.0);
                        assert!(
                            sign.is_some_and(|w| w > 200.0),
                            "a reading that fits is drawn, and reported as drawn"
                        );
                        // The same reading into a dock too narrow to hold it draws nothing at all:
                        // a caption cut in half reads as a layout bug, not as a smaller window.
                        assert_eq!(
                            banner(ui, "gen 43/300  best 218.2  wins 2/200", theme::GOOD, 120.0),
                            None
                        );
                        plaque(ui, "L5-tightrope", "saved · levels/l5-tightrope.json");
                    });
            },
        );
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        for p in &primitives {
            if let egui::epaint::Primitive::Mesh(mesh) = &p.primitive {
                drawn += mesh.vertices.len();
            }
        }
        // A plate is at least a socket, a face and two edges: three plates alone are far past this
        // floor, and a regression that stopped painting would fall through it.
        assert!(
            drawn > 200,
            "the controls drew only {drawn} vertices, which is not a drawn button: \
             an empty paint has no vertices at all"
        );
    }

    /// The switch's cells are one size, and that size has room for every name in the set, measured
    /// in the font the switch draws with.
    ///
    /// A plate clips its own content, so a cell sized by a guess loses the tail of its label rather
    /// than wrapping — and *Replay* is the longest word the switch has. Rather than assert a
    /// clearance the widget happens to satisfy, this measures the four real mode names in the real
    /// font and asks the width rule for each: the room a cell gives a caption has to be at least
    /// that caption's own width. The two formulas have to *agree* for that to hold — the one that
    /// sizes the plate and the one that insets its content — which is exactly the sort of drift a
    /// retune of the padding causes and no assertion about colour sees.
    #[test]
    fn every_switch_cell_has_room_for_the_widest_name_in_the_set() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        // Fonts exist only once a frame has been begun, so the measuring happens inside one — the
        // same context the switch is drawn in, which is the point: an assertion measured in some
        // other font would prove nothing about this one.
        let mut measured: Vec<(&str, f32, f32)> = Vec::new();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 200.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default())
                    .show(ctx, |ui| {
                        // The rule `segmented` sizes its cells with, applied to the whole set at
                        // once: one width for the widest name, never narrower than a plate.
                        let cell = ["Edit", "Play", "Evolve", "Replay"]
                            .iter()
                            .map(|word| caption_width(ui, word))
                            .fold(TOOL_SIZE.x, f32::max);
                        for word in ["Edit", "Play", "Evolve", "Replay"] {
                            let width = ui
                                .painter()
                                .layout_no_wrap(
                                    word.to_string(),
                                    FontId::new(CAPTION_FONT, FontFamily::Proportional),
                                    theme::TEXT,
                                )
                                .size()
                                .x;
                            let room = cell - CELL_PAD * 2.0 - 2.0;
                            measured.push((word, width, room));
                        }
                    });
            },
        );
        assert_eq!(measured.len(), 4);
        for (word, width, room) in &measured {
            assert!(
                room >= width,
                "`{word}` is {width:.1} points wide and its cell gives it {room:.1}"
            );
        }
    }

    /// The atlas icon is *optional everywhere*, including when the atlas is present but does not
    /// carry the sprite: a tool bar that panicked or blanked on a missing name would take the whole
    /// window with it over one misnamed picture.
    #[test]
    fn a_button_without_a_working_icon_still_draws_its_label() {
        let ctx = egui::Context::default();
        let mut drew = 0usize;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 120.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default())
                    .show(ctx, |ui| {
                        icon_button(ui, None, Some("not_a_sprite"), "New", TOOL_SIZE, false, true);
                    });
            },
        );
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        for p in &primitives {
            if let egui::epaint::Primitive::Mesh(mesh) = &p.primitive {
                drew += mesh.vertices.len();
            }
        }
        assert!(drew > 0, "a button with no icon must still be a button");
    }
}
