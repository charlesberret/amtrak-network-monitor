//! The screen for a ride. One colored word, the rest quiet.

use crate::load::{Foot, LineTone, LoadView};
use crate::model::{Snapshot, Tone};
use chrono::Timelike;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

const INK: Color = Color::Rgb(214, 210, 200);
const DIM: Color = Color::Rgb(120, 116, 108);
const RULE: Color = Color::Rgb(68, 66, 62);
const KEY: Color = Color::Rgb(64, 132, 186);
const CABIN_GLOW: Color = Color::Rgb(245, 215, 130);
const CLEAR_GREEN: Color = Color::Rgb(40, 146, 82);
const CYAN: Color = Color::Rgb(80, 200, 220);
const WHITE: Color = Color::Rgb(255, 255, 255);

pub fn draw(frame: &mut Frame, snap: &Snapshot) {
    let area = frame.area();
    frame.render_widget(Paragraph::new(""), area);
    let width = area.width;

    let mut body = body_lines(snap, width);
    let mut hist = history_lines(snap, width);
    let mut foot = footer(snap, width);
    // Header, rule, then the words. Drop the history before dropping the footer.
    let chrome = 3u16;
    while chrome as usize + body.len() + hist.len() + foot.len() > area.height as usize {
        if !hist.is_empty() {
            hist.clear();
            continue;
        }
        if foot.len() > 1 {
            foot.pop();
            continue;
        }
        if body.len() > 4 {
            body.pop();
            continue;
        }
        break;
    }

    let mut constraints = vec![Constraint::Length(2), Constraint::Length(1)];
    constraints.push(Constraint::Length(body.len() as u16));
    if !hist.is_empty() {
        constraints.push(Constraint::Length(hist.len() as u16));
    }
    constraints.push(Constraint::Min(foot.len().max(1) as u16));
    let rects = Layout::vertical(constraints).split(area);

    render_line(frame, rects[0], header(snap, width));
    render_line(
        frame,
        rects[1],
        Line::from(Span::styled(rule(width), Style::new().fg(RULE))),
    );
    render_lines(frame, rects[2], body);
    if hist.is_empty() {
        render_lines(frame, rects[3], foot);
    } else {
        render_lines(frame, rects[3], hist);
        render_lines(frame, rects[4], foot);
    }
}

pub fn monitor_title(on_amtrak: bool, line: Option<&str>) -> String {
    if !on_amtrak {
        return "Network Monitor (Not on Amtrak)".to_string();
    }
    match line {
        Some(line) if !line.is_empty() => format!("Amtrak {line} Network Monitor"),
        _ => "Amtrak Network Monitor".to_string(),
    }
}

fn header(snap: &Snapshot, width: u16) -> Line<'static> {
    let title = monitor_title(snap.on_amtrak, snap.line.as_deref());
    let clock = format!("  {}", clock_label(snap));
    let room = (width as usize).saturating_sub(clock.chars().count() + 1);
    let title = clip(&format!("  {title}"), room);
    Line::from(vec![
        Span::styled(title, Style::new().fg(INK).add_modifier(Modifier::BOLD)),
        Span::styled(clock, Style::new().fg(DIM)),
    ])
}

fn body_lines(snap: &Snapshot, width: u16) -> Vec<Line<'static>> {
    let place = match snap.fix_age {
        Some(age) if age.as_secs() >= 120 => {
            format!("{}, {} min ago", snap.place, age.as_secs() / 60)
        }
        _ => snap.place.clone(),
    };
    let mut lines = vec![
        blank(),
        styled(
            clip(&format!("  {place}"), cols(width)),
            Style::new().fg(INK).add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some((lat, lon)) = snap.position {
        lines.push(dim(clip(&format!("  {lat:.3}, {lon:.3}"), cols(width))));
    }
    if let Some(motion) = &snap.motion {
        lines.push(dim(clip(&format!("  {motion}"), cols(width))));
    }
    lines.push(blank());
    lines.push(styled(
        clip(&format!("  {}", snap.tone.word()), cols(width)),
        Style::new()
            .fg(tone_color(snap.tone))
            .add_modifier(Modifier::BOLD),
    ));
    if !snap.pace.is_empty() {
        lines.push(ink(clip(&format!("  {}", snap.pace), cols(width))));
    }
    if !snap.page.is_empty() {
        lines.push(ink(clip(&format!("  {}", snap.page), cols(width))));
    }
    if !snap.path.is_empty() {
        lines.push(blank());
        lines.push(dim(clip(&format!("  {}", snap.path), cols(width))));
    }
    if snap.is_default {
        lines.push(dim(clip("  default route", cols(width))));
    }
    lines
}

fn history_lines(snap: &Snapshot, width: u16) -> Vec<Line<'static>> {
    let (lights, slot) = lights_that_fit(&snap.history, width);
    let minutes = lights.len().div_ceil(2).max(1);
    let title = if minutes == 1 {
        "  last minute".to_string()
    } else {
        format!("  last {minutes} minutes")
    };
    let title = clip(&title, cols(width));
    let mut dots = vec![Span::styled("  ", Style::new())];
    if lights.is_empty() {
        dots.push(Span::styled("·", Style::new().fg(DIM)));
    }
    for light in lights {
        // One block per 30 seconds, then one space, so two colors never share a block.
        dots.push(block(light.tone, light.label.is_some()));
        if slot > 1 {
            dots.push(Span::raw(" ".repeat(slot - 1)));
        }
    }
    let times = time_row(lights, slot, cols(width));
    vec![
        blank(),
        dim(title),
        Line::from(dots),
        Line::from(Span::styled(times, Style::new().fg(DIM))),
    ]
}

fn block(tone: Tone, marked: bool) -> Span<'static> {
    let mut style = Style::new().fg(tone_color(tone));
    if marked {
        style = style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    }
    Span::styled("█", style)
}

/// How many 30-second blocks fit. One space between blocks when the row allows it.
fn lights_that_fit(
    history: &[crate::model::MinuteLight],
    width: u16,
) -> (&[crate::model::MinuteLight], usize) {
    let inner = cols(width).saturating_sub(2).max(1);
    let slot = if inner >= 4 { 2 } else { 1 };
    let count = (inner / slot).clamp(1, history.len().max(1));
    let count = count.min(history.len());
    if history.is_empty() {
        return (history, slot);
    }
    (&history[history.len() - count..], slot)
}

/// The colon of each time sits on that minute's square. Digits fall in the
/// gaps on either side, not on the neighboring squares.
fn time_row(lights: &[crate::model::MinuteLight], slot: usize, max: usize) -> String {
    if lights.is_empty() {
        return String::new();
    }
    let indent = 2;
    let mut cells = vec![' '; indent + lights.len() * slot];
    for (i, light) in lights.iter().enumerate() {
        let Some(text) = light.label.as_deref() else {
            continue;
        };
        let text: Vec<char> = text.chars().collect();
        if text.is_empty() {
            continue;
        }
        let anchor = indent + i * slot;
        // Odd lengths put the square on the middle character. Even lengths
        // ("2:20") put it on the colon, the left of the two middle characters.
        let half = text.len() / 2;
        let left = if text.len() % 2 == 0 {
            half.saturating_sub(1)
        } else {
            half
        };
        let mut start = anchor.saturating_sub(left);
        if start + text.len() > cells.len() {
            start = cells.len().saturating_sub(text.len());
        }
        let clear = (start..start + text.len()).all(|idx| cells.get(idx).copied() == Some(' '));
        if !clear {
            continue;
        }
        for (k, ch) in text.iter().enumerate() {
            cells[start + k] = *ch;
        }
    }
    clip(&cells.into_iter().collect::<String>(), max)
}

fn footer(snap: &Snapshot, width: u16) -> Vec<Line<'static>> {
    let age = match snap.sample_age {
        Some(d) if d.as_secs() < 5 => "updated just now".to_string(),
        Some(d) => format!("updated {}s ago", d.as_secs()),
        None => "no measurement yet".to_string(),
    };
    let saving = if snap.saving {
        "saving this trip"
    } else {
        "not saving"
    };
    vec![
        Line::from(Span::styled(rule(width), Style::new().fg(RULE))),
        dim(clip(&format!("  {age}"), cols(width))),
        dim(clip(&format!("  {saving}"), cols(width))),
        keys_line("quit", "settings"),
    ]
}

fn keys_line(q_word: &str, s_word: &str) -> Line<'static> {
    let word = Style::new().fg(DIM);
    let key = Style::new().fg(KEY);
    Line::from(vec![
        Span::styled("  ", word),
        Span::styled("q", key),
        Span::styled(format!("  {q_word}     "), word),
        Span::styled("s", key),
        Span::styled(format!("  {s_word}"), word),
    ])
}

fn settings_keys() -> Line<'static> {
    let word = Style::new().fg(DIM);
    let key = Style::new().fg(KEY);
    Line::from(vec![
        Span::styled("  ", word),
        Span::styled("s", key),
        Span::styled("  save     ", word),
        Span::styled("c", key),
        Span::styled("  cancel", word),
    ])
}

pub fn draw_settings(frame: &mut Frame, snap: &Snapshot, draft: &crate::settings::SettingsDraft) {
    let area = frame.area();
    frame.render_widget(Paragraph::new(""), area);
    let width = area.width;
    let mut lines = vec![
        blank(),
        styled(
            clip("  Settings", cols(width)),
            Style::new().fg(INK).add_modifier(Modifier::BOLD),
        ),
        blank(),
    ];
    for row in draft.rows() {
        let style = if row.selected {
            Style::new().fg(INK).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(INK)
        };
        lines.push(styled(clip(&row.text, cols(width)), style));
    }
    lines.push(blank());
    let rate_note = if draft.mac_helper {
        "  Mac helper still writes about every 15 s"
    } else {
        "  Location rate applies the next time you start"
    };
    lines.extend([
        dim(clip(rate_note, cols(width))),
        dim(clip("  A saved position is about 100 m", cols(width))),
        dim(clip(
            "  Town                    kept when known",
            cols(width),
        )),
        dim(clip("  Street address          not saved", cols(width))),
        dim(clip("  Wi-Fi name              not saved", cols(width))),
        dim(clip("  IP address              not saved", cols(width))),
        blank(),
        dim(clip(
            "  up and down move. space changes the row.",
            cols(width),
        )),
        settings_keys(),
    ]);
    let rects = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(lines.len() as u16),
    ])
    .split(area);
    render_line(frame, rects[0], header(snap, width));
    render_line(
        frame,
        rects[1],
        Line::from(Span::styled(rule(width), Style::new().fg(RULE))),
    );
    render_lines(frame, rects[2], lines);
}

fn tone_color(tone: Tone) -> Color {
    // Explicit RGB, so a terminal theme does not tint these.
    // Between the first pastel set and the bright set that followed.
    match tone {
        Tone::Clear => Color::Rgb(40, 146, 82),
        Tone::Uneven => Color::Rgb(184, 138, 40),
        Tone::Poor => Color::Rgb(176, 96, 48),
        Tone::Down => Color::Rgb(168, 68, 64),
        Tone::NoSample => Color::Rgb(120, 116, 108),
    }
}

fn clock_label(snap: &Snapshot) -> String {
    let hour = snap.at.hour();
    let h = match hour % 12 {
        0 => 12,
        n => n,
    };
    let suffix = if hour < 12 { "AM" } else { "PM" };
    format!("{h}:{:02} {suffix}", snap.at.minute())
}

fn rule(width: u16) -> String {
    let n = (width as usize).saturating_sub(3);
    if n <= 2 {
        return String::new();
    }
    format!("  {}", "─".repeat(n - 2))
}

fn cols(width: u16) -> usize {
    (width as usize).saturating_sub(1)
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = text.chars().take(keep).collect();
    out.push('…');
    out
}

fn blank() -> Line<'static> {
    Line::from("")
}

fn ink(text: String) -> Line<'static> {
    styled(text, Style::new().fg(INK))
}

fn dim(text: String) -> Line<'static> {
    styled(text, Style::new().fg(DIM))
}

fn styled(text: String, style: Style) -> Line<'static> {
    Line::from(Span::styled(text, style))
}

pub fn draw_load(frame: &mut Frame, view: &LoadView) {
    let area = frame.area();
    frame.render_widget(Paragraph::new(""), area);
    let width = area.width;
    let foot = load_foot(view.foot, width);
    let body: Vec<Line> = view.lines.iter().map(styled_load).collect();

    let anim_lines = if let Some(tick) = view.anim_tick {
        if area.height >= 16 && area.width >= TRAIN_MIN_WIDTH {
            Some(animation_frame(tick, width))
        } else if area.height >= 9 && area.width >= 30 {
            Some(bars_frame(tick, width, area.height))
        } else {
            None
        }
    } else {
        None
    };

    let mut constraints = Vec::new();
    if let Some(ref anim) = anim_lines {
        constraints.push(Constraint::Length(anim.len() as u16));
    }
    constraints.push(Constraint::Min(1));
    if view.pinned.is_some() {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(foot.len() as u16));
    let rects = Layout::vertical(constraints).split(area);

    let mut idx = 0;
    if let Some(anim) = anim_lines {
        render_lines(frame, rects[idx], anim);
        idx += 1;
    }
    render_lines(frame, rects[idx], body);
    idx += 1;
    if let Some(pinned) = &view.pinned {
        render_line(frame, rects[idx], styled_load(pinned));
        idx += 1;
    }
    render_lines(frame, rects[idx], foot);
}

/// Eighths of a bar, bottom up. Index 0 is an empty cell.
const BAR_GLYPHS: [&str; 8] = [" ", "⣀", "⣠", "⣤", "⣴", "⣶", "⣾", "⣿"];

/// One wheel truck, in four frames so the spokes turn.
const TRUCKS: [&str; 4] = ["(⠒⣿⠒)⠤(⠒⣿⠒)", "(⠦⣿⠴)⠤(⠦⣿⠴)", "(⠯⣿⠽)⠤(⠯⣿⠽)", "(⠭⣿⠮)⠤(⠭⣿⠮)"];
const TRUCK_W: usize = 11;

/// Columns left of the train, columns of lit rail past the nose, and the
/// locomotive's width. A train needs 54 columns: these, plus one 12-column
/// coach and its coupler.
const LEFT: usize = 2;
const AHEAD: usize = 4;
const LOCO_W: usize = 34;
pub const TRAIN_MIN_WIDTH: u16 = 54;

/// Bar heights for this tick, in rows. Two slow waves out of phase, so the
/// band rolls instead of pulsing.
fn bar_heights(tick: usize, bars: usize, rows: usize) -> Vec<f64> {
    let t = tick as f64;
    (0..bars)
        .map(|i| {
            let x = i as f64;
            let wave = (t * 0.38 + x * 0.45).sin() * 1.5 + (t * 0.24 + x * 0.75).cos() * 1.2 + 2.0;
            (wave * rows as f64 / 3.8).clamp(0.0, rows as f64)
        })
        .collect()
}

/// A band of bars, top row first. Each bar is one glyph and one space.
/// `color` is asked for each row, top row 0.
fn bar_rows(
    heights: &[f64],
    rows: usize,
    left: usize,
    color: impl Fn(usize) -> Color,
) -> Vec<Line<'static>> {
    (0..rows)
        .map(|row| {
            let floor = (rows - 1 - row) as f64;
            let style = Style::new().fg(color(row));
            let mut spans = vec![Span::raw(" ".repeat(left))];
            for &h in heights {
                let cell = if h >= floor + 1.0 {
                    "⣿ ".to_string()
                } else if h > floor {
                    let eighths = (((h - floor) * 8.0) as usize).clamp(1, 7);
                    format!("{} ", BAR_GLYPHS[eighths])
                } else {
                    "  ".to_string()
                };
                spans.push(Span::styled(cell, style));
            }
            Line::from(spans)
        })
        .collect()
}

/// The signal band on its own, centered, for a screen too small for the train.
pub fn bars_frame(tick: usize, width: u16, height: u16) -> Vec<Line<'static>> {
    let rows = (height as usize).saturating_sub(7).clamp(2, 5);
    let bars = ((width as usize).saturating_sub(6) / 2).clamp(8, 36);
    let left = (width as usize).saturating_sub(bars * 2) / 2;
    let heights = bar_heights(tick, bars, rows);
    bar_rows(&heights, rows, left, |row| {
        if row == 0 {
            CLEAR_GREEN
        } else if row <= rows / 2 {
            CYAN
        } else {
            KEY
        }
    })
}

/// Split the room left of the locomotive into coaches of 12 to 28 columns,
/// each followed by a two-column coupler.
fn coach_lengths(room: usize) -> Vec<usize> {
    let count = room.div_ceil(30).max(1);
    let inner = room.saturating_sub(2 * count);
    (0..count)
        .map(|i| inner / count + usize::from(i < inner % count))
        .collect()
}

/// Split `slack` columns around something centered, the odd column on the right.
fn centered(slack: usize) -> (usize, usize) {
    (slack / 2, slack - slack / 2)
}

/// A ground row that scrolls left one column a tick, so the train reads as
/// moving right. `pattern` repeats for the whole length.
fn ground_row(pattern: &str, tick: usize, len: usize, color: Color) -> Line<'static> {
    let period = pattern.chars().count();
    let text: String = pattern
        .chars()
        .cycle()
        .skip(tick % period)
        .take(len)
        .collect();
    Line::from(vec![
        Span::raw(" ".repeat(LEFT)),
        Span::styled(text, Style::new().fg(color)),
    ])
}

/// The five rows of the train, built left to right.
struct Train {
    roof: Vec<Span<'static>>,
    windows: Vec<Span<'static>>,
    body: Vec<Span<'static>>,
    sill: Vec<Span<'static>>,
    wheels: Vec<Span<'static>>,
}

impl Train {
    fn new() -> Self {
        let margin = || vec![Span::raw(" ".repeat(LEFT))];
        Train {
            roof: margin(),
            windows: margin(),
            body: margin(),
            sill: margin(),
            wheels: margin(),
        }
    }

    /// A coach `len` columns wide (at least 12), then its coupler.
    fn coach(&mut self, len: usize, truck: &'static str) {
        let ink = Style::new().fg(INK);
        let rule = Style::new().fg(RULE);

        self.roof
            .push(Span::styled(format!("⢀⣠{}⣄⡀", "⠤".repeat(len - 4)), ink));
        self.roof.push(Span::raw("  "));

        let panes = ((len - 4) / 4).max(1);
        let glass = vec!["⢺ ⡗"; panes].join(" ");
        let (left, right) = centered(len - 2 - (panes * 4 - 1));
        self.windows.push(Span::styled("⢸", ink));
        self.windows.push(Span::raw(" ".repeat(left)));
        self.windows
            .push(Span::styled(glass, Style::new().fg(CABIN_GLOW)));
        self.windows.push(Span::raw(" ".repeat(right)));
        self.windows.push(Span::styled("⢸", ink));
        self.windows.push(Span::styled("⠶⠶", rule));

        self.body
            .push(Span::styled(format!("⢸{}⡇", "⣿".repeat(len - 2)), ink));
        self.body.push(Span::raw("  "));

        let gear = if len >= 16 {
            "⠛⠛⠛⣾⣿⡷⠛⠛"
        } else {
            "⣾⣿⡷"
        };
        let (left, right) = centered(len - 2 - gear.chars().count());
        self.sill
            .push(Span::styled(format!("⠘{}", "⠛".repeat(left)), rule));
        self.sill.push(Span::styled(gear, ink));
        self.sill
            .push(Span::styled(format!("{}⠃", "⠛".repeat(right)), rule));
        self.sill.push(Span::raw("  "));

        let (left, right) = centered(len - TRUCK_W);
        self.wheels.push(Span::raw(" ".repeat(left)));
        self.wheels.push(Span::styled(truck, Style::new().fg(DIM)));
        self.wheels.push(Span::raw(" ".repeat(right + 2)));
    }

    /// The locomotive, 34 columns, facing right: a long body, the cab,
    /// a rounded nose with the headlight, and a plow.
    fn locomotive(&mut self, truck: &'static str) {
        let ink = Style::new().fg(INK);
        let rule = Style::new().fg(RULE);
        let lamp = Style::new().fg(WHITE).add_modifier(Modifier::BOLD);

        self.roof
            .push(Span::styled("⢀⣠⠤⠶⠶⠶⠶⠶⠶⠶⠶⠶⠶⠶⠶⠶⠶⠶⠶⠶⠶⠶⠶⠶⠤⣄", ink));

        self.windows.push(Span::styled("⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿", ink));
        self.windows
            .push(Span::styled("⢹ ⡇ ⢹ ⡇ ", Style::new().fg(KEY)));
        self.windows.push(Span::styled("⠙⢦", ink));

        self.body
            .push(Span::styled("⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿", ink));
        self.body.push(Span::styled("(●)", lamp));
        self.body.push(Span::styled(" ⠸⣆", ink));

        self.sill.push(Span::styled("⠘⠛⠛⠛⠛⠛⠛⠛⠛", rule));
        self.sill.push(Span::styled("⣾⣿⣿⡷", ink));
        self.sill.push(Span::styled("⠛⠛⠛⠛⠛⠛⠛⠛⠛⠛⠛⠛⠛⠛⠛⠛⠲⢤⣀", rule));

        self.wheels.push(Span::raw(" "));
        self.wheels.push(Span::styled(truck, Style::new().fg(DIM)));
        self.wheels.push(Span::raw("  "));
        self.wheels.push(Span::styled(truck, Style::new().fg(DIM)));
    }

    fn lines(self) -> [Line<'static>; 5] {
        [
            Line::from(self.roof),
            Line::from(self.windows),
            Line::from(self.body),
            Line::from(self.sill),
            Line::from(self.wheels),
        ]
    }
}

/// The load screen's train: three rows of signal band, five of train, and
/// three of rails and ground. The rail runs to the last column, lit past
/// the nose. No line is wider than `width` when `width` is at least
/// `TRAIN_MIN_WIDTH`; `draw_load` keeps narrower screens on `bars_frame`.
pub fn animation_frame(tick: usize, width: u16) -> Vec<Line<'static>> {
    let cols = (width as usize).max(TRAIN_MIN_WIDTH as usize);
    let room = cols - LEFT - AHEAD - LOCO_W;
    let ground = cols - LEFT;

    let heights = bar_heights(tick, (ground - 2) / 2, 3);
    let mut lines = bar_rows(&heights, 3, LEFT, |row| match row {
        0 => CLEAR_GREEN,
        1 => CYAN,
        _ => KEY,
    });

    let truck = TRUCKS[tick % TRUCKS.len()];
    let mut train = Train::new();
    for len in coach_lengths(room) {
        train.coach(len, truck);
    }
    train.locomotive(truck);
    lines.extend(train.lines());

    lines.push(Line::from(vec![
        Span::raw(" ".repeat(LEFT)),
        Span::styled("⠶".repeat(ground - AHEAD), Style::new().fg(RULE)),
        Span::styled(
            "⠶".repeat(AHEAD),
            Style::new().fg(WHITE).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(ground_row("⠓⠒⠚", tick, ground, RULE));
    lines.push(ground_row(" ⠂ ⠂ ⠄", tick, ground, DIM));
    lines
}

fn styled_load(line: &crate::load::LoadLine) -> Line<'static> {
    let style = match line.tone {
        LineTone::Prompt | LineTone::Selected => Style::new().fg(INK).add_modifier(Modifier::BOLD),
        LineTone::Text => Style::new().fg(INK),
        LineTone::Dim => Style::new().fg(DIM),
    };
    Line::from(Span::styled(line.text.clone(), style))
}

fn load_foot(foot: Foot, width: u16) -> Vec<Line<'static>> {
    let keys = match foot {
        Foot::Quit => one_key("q", "quit"),
        Foot::Esc => one_key("esc", "quit"),
        Foot::EnterQuit => Line::from(vec![
            Span::styled("  ", Style::new().fg(DIM)),
            Span::styled("enter", Style::new().fg(KEY)),
            Span::styled("  continue     ", Style::new().fg(DIM)),
            Span::styled("q", Style::new().fg(KEY)),
            Span::styled("  quit", Style::new().fg(DIM)),
        ]),
    };
    vec![
        Line::from(Span::styled(rule(width), Style::new().fg(RULE))),
        keys,
    ]
}

fn one_key(key: &str, word: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("  ", Style::new().fg(DIM)),
        Span::styled(key.to_string(), Style::new().fg(KEY)),
        Span::styled(format!("  {word}"), Style::new().fg(DIM)),
    ])
}

fn render_line(frame: &mut Frame, area: Rect, line: Line<'_>) {
    frame.render_widget(Paragraph::new(line), area);
}

fn render_lines(frame: &mut Frame, area: Rect, lines: Vec<Line<'_>>) {
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MinuteLight;
    use chrono::{Local, TimeZone};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::time::Duration;

    fn settings_draft() -> crate::settings::SettingsDraft {
        crate::settings::SettingsDraft::from_saved(&crate::settings::Gather::default(), true)
    }

    fn snap(history: Vec<MinuteLight>) -> Snapshot {
        Snapshot {
            at: Local.with_ymd_and_hms(2026, 10, 21, 14, 23, 40).unwrap(),
            line: Some("Pennsylvanian".into()),
            on_amtrak: true,
            place: "Millerstown, PA".into(),
            position: Some((40.536, -77.150)),
            motion: None,
            tone: Tone::Clear,
            pace: "54 ms, no loss".into(),
            page: "test page: 0.22 s".into(),
            path: "iPhone hotspot over Wi-Fi".into(),
            is_default: true,
            history,
            saving: true,
            sample_age: Some(Duration::from_secs(3)),
            fix_age: Some(Duration::from_secs(20)),
        }
    }

    #[test]
    fn ten_minute_time_sits_under_its_square() {
        let mut history = vec![
            MinuteLight {
                tone: Tone::Clear,
                label: None,
            };
            10
        ];
        history[6].label = Some("2:20".into());
        history[6].tone = Tone::Down;
        let backend = TestBackend::new(40, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let view = snap(history);
        terminal.draw(|frame| draw(frame, &view)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut time_at = None;
        let mut dot_at = None;
        for y in 0..buf.area.height {
            let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            if let Some(col) = row.find("2:20") {
                time_at = Some((col as u16, y));
            }
            let mark = &buf[(14, y)];
            if mark.symbol() == "█" && mark.style().add_modifier.contains(Modifier::UNDERLINED) {
                dot_at = Some(y);
            }
        }
        let screen: String = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !screen.contains("colored word"),
            "the status caption should not be on the screen"
        );
        let (col, time_y) = time_at.expect("time under the ten-minute square");
        let dot_y = dot_at.expect("underlined square");
        assert_eq!(col, 13, "2:20 should start one column left of its square");
        assert_eq!(time_y, dot_y + 1);
        assert_eq!(buf[(14, time_y)].symbol(), ":");
    }

    #[test]
    fn times_sit_on_their_own_squares() {
        let mut lights = vec![
            MinuteLight {
                tone: Tone::Clear,
                label: None,
            };
            8
        ];
        lights[3].label = Some("2:20".into());
        lights[6].label = Some("12:30".into());
        let row: Vec<char> = time_row(&lights, 2, 80).chars().collect();
        let square = |i: usize| 2 + i * 2;
        assert_eq!(row[square(3)], ':');
        assert_eq!(row[square(3) - 1], '2');
        assert_eq!(row[square(2)], ' ');
        assert_eq!(row[square(6)], ':');
    }

    #[test]
    fn a_stale_town_carries_its_age() {
        let mut old = snap(vec![MinuteLight {
            tone: Tone::Clear,
            label: None,
        }]);
        old.fix_age = Some(Duration::from_secs(48 * 60));
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &old)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let screen: String = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("Amtrak Pennsylvanian Network Monitor"));
        assert!(screen.contains("Millerstown, PA, 48 min ago"));
        assert!(!screen.contains("location 48 min ago"));
        assert!(!screen.contains("sampler"));
        assert!(!screen.contains("Measuring the connection"));
    }

    #[test]
    fn settings_states_what_is_saved_and_what_is_left_out() {
        let view = snap(vec![]);
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw_settings(frame, &view, &settings_draft()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let screen: String = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("Save this trip"));
        assert!(screen.contains("On"));
        assert!(screen.contains("save"));
        assert!(screen.contains("cancel"));
        assert!(!screen.contains("quit"));
        assert!(screen.contains("Street address"));
        assert!(screen.contains("not saved"));
        assert!(screen.contains("Wi-Fi name"));
        assert!(screen.contains("IP address"));
        assert!(!screen.contains("sampler"));
    }

    #[test]
    fn quit_and_settings_keys_are_blue_and_side_by_side() {
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &snap(vec![]))).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut found = None;
        for y in 0..buf.area.height {
            let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            if let Some(quit_at) = row.find("quit") {
                let q = quit_at - 3;
                let s = row.find("settings").expect("settings") - 3;
                found = Some((q, s, y));
            }
        }
        let (q, s, y) = found.expect("quit key");
        let blue = Some(Color::Rgb(64, 132, 186));
        assert_eq!(buf[(q as u16, y)].symbol(), "q");
        assert_eq!(buf[(s as u16, y)].symbol(), "s");
        assert_eq!(buf[(q as u16, y)].style().fg, blue);
        assert_eq!(buf[(s as u16, y)].style().fg, blue);
        assert!(s > q);
        assert!(s - q < 16, "s should sit next to q");
    }

    #[test]
    fn settings_offers_save_and_cancel_instead_of_quit() {
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw_settings(frame, &snap(vec![]), &settings_draft()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut found = None;
        for y in 0..buf.area.height {
            let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            if let Some(cancel_at) = row.find("cancel") {
                let save_at = row.find("save").expect("save");
                found = Some((save_at - 3, cancel_at - 3, y, row));
            }
        }
        let (s, c, y, row) = found.expect("save and cancel");
        let blue = Some(Color::Rgb(64, 132, 186));
        assert!(!row.contains("quit"));
        assert_eq!(buf[(s as u16, y)].symbol(), "s");
        assert_eq!(buf[(c as u16, y)].symbol(), "c");
        assert_eq!(buf[(s as u16, y)].style().fg, blue);
        assert_eq!(buf[(c as u16, y)].style().fg, blue);
        assert!(c > s);
        assert!(c - s < 16, "c should sit next to s");
    }

    #[test]
    fn off_the_train_the_title_drops_the_product_name() {
        let mut view = snap(vec![]);
        view.on_amtrak = false;
        view.line = None;
        let backend = TestBackend::new(70, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &view)).unwrap();
        let screen = screen_of(&terminal);
        assert!(screen.contains("Network Monitor (Not on Amtrak)"));
        assert!(!screen.contains("Amtrak Network Monitor"));
        assert_eq!(
            monitor_title(false, None),
            "Network Monitor (Not on Amtrak)"
        );
        assert_eq!(
            monitor_title(true, Some("Pennsylvanian")),
            "Amtrak Pennsylvanian Network Monitor"
        );
        assert_eq!(monitor_title(true, None), "Amtrak Network Monitor");
    }

    #[test]
    fn the_load_screen_shows_the_question_and_a_blue_quit_key() {
        use crate::load::Wizard;
        let view = Wizard::new().view(24, "About 40 kB an hour.");
        let backend = TestBackend::new(70, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw_load(frame, &view)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let screen = screen_text(&buf);
        assert!(screen.contains("Are you riding Amtrak right now? y/N"));
        let mut quit = None;
        for y in 0..buf.area.height {
            let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            if let Some(at) = row.find("quit") {
                quit = Some((at - 3, y));
            }
        }
        let (q, y) = quit.expect("quit key");
        assert_eq!(buf[(q as u16, y)].symbol(), "q");
        assert_eq!(
            buf[(q as u16, y)].style().fg,
            Some(Color::Rgb(64, 132, 186))
        );
    }

    #[test]
    fn continue_without_a_line_stays_on_the_bottom_row() {
        use crate::load::{Press, Wizard};
        let mut wizard = Wizard::new();
        wizard.push(Press::Char('y'));
        let view = wizard.view(16, "About 40 kB an hour.");
        let backend = TestBackend::new(70, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw_load(frame, &view)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let rule_y = (0..buf.area.height)
            .rev()
            .find(|y| {
                let row: String = (0..buf.area.width).map(|x| buf[(x, *y)].symbol()).collect();
                row.contains('─')
            })
            .expect("rule");
        let pinned: String = (0..buf.area.width)
            .map(|x| buf[(x, rule_y - 1)].symbol())
            .collect();
        assert!(
            pinned.contains("Continue without selecting a line"),
            "{pinned}"
        );
    }

    #[test]
    fn the_store_question_wraps_the_reason_and_the_size() {
        use crate::load::{Press, Wizard};
        let mut wizard = Wizard::new();
        wizard.push(Press::Char('y'));
        for ch in "penn".chars() {
            wizard.push(Press::Char(ch));
        }
        wizard.push(Press::Enter);
        wizard.push(Press::Char('1'));
        let view = wizard.view(24, "About 40 kB an hour.");
        let backend = TestBackend::new(60, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw_load(frame, &view)).unwrap();
        let screen = screen_text(terminal.backend().buffer());
        let flat = screen.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains("only you can read"));
        assert!(flat.contains("which connection you picked"));
        assert!(flat.contains("About 40 kB an hour."));
        assert!(flat.contains("Nothing is uploaded."));
        assert!(!flat.contains("Keep an anonymous copy"));
    }

    #[test]
    fn the_train_fits_every_width_and_the_rail_reaches_the_edge() {
        for width in TRAIN_MIN_WIDTH..=260 {
            for tick in 0..12 {
                let lines = animation_frame(tick, width);
                assert_eq!(lines.len(), 11, "w={width}");
                for (i, line) in lines.iter().enumerate() {
                    assert!(
                        line.width() <= width as usize,
                        "line {i} is {} wide at w={width} tick={tick}",
                        line.width()
                    );
                }
                assert_eq!(lines[8].width(), width as usize, "rail at w={width}");
                assert_eq!(lines[9].width(), width as usize, "ties at w={width}");
                assert_eq!(lines[10].width(), width as usize, "ballast at w={width}");
            }
        }
    }

    #[test]
    fn every_coach_is_wide_enough_for_its_wheels() {
        for room in 14..=300 {
            let lengths = coach_lengths(room);
            assert_eq!(lengths.iter().sum::<usize>() + 2 * lengths.len(), room);
            for len in lengths {
                assert!((12..=28).contains(&len), "room={room} len={len}");
            }
        }
        assert_eq!(coach_lengths(14), vec![12]);
        assert_eq!(coach_lengths(31), vec![14, 13]);
    }

    #[test]
    fn the_ground_scrolls_one_column_a_tick() {
        let a = ground_row("⠓⠒⠚", 0, 9, RULE).to_string();
        let b = ground_row("⠓⠒⠚", 1, 9, RULE).to_string();
        assert_eq!(a.trim_start(), "⠓⠒⠚⠓⠒⠚⠓⠒⠚");
        assert_eq!(b.trim_start(), "⠒⠚⠓⠒⠚⠓⠒⠚⠓");
        assert_eq!(ground_row("⠓⠒⠚", 3, 9, RULE).to_string(), a);
    }

    #[test]
    fn the_bars_alone_fit_every_small_screen() {
        for width in 30..=120u16 {
            for height in 9..=15u16 {
                let lines = bars_frame(0, width, height);
                assert_eq!(lines.len(), (height as usize - 7).clamp(2, 5));
                for line in &lines {
                    assert!(line.width() <= width as usize, "w={width} h={height}");
                }
            }
        }
    }

    #[test]
    fn the_load_screen_shows_animated_train_when_tall_enough() {
        use crate::load::Wizard;
        let mut wizard = Wizard::new();
        wizard.tick();
        wizard.tick();
        let view = wizard.view(24, "About 40 kB an hour.");
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw_load(frame, &view)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let screen = screen_text(&buf);
        assert!(screen.contains("Are you riding Amtrak right now? y/N"));
        assert!(screen.contains("⢺ ⡗"));
        assert!(!screen.contains("⢺⣿⡗"));
        assert!(!screen.contains("⢹⣿⡇"));
        assert!(!screen.contains("⠹⣿⠏"));
        assert!(screen.contains("(●)"));
        assert!(screen.contains("⠶⠶"));
    }

    #[test]
    fn the_load_screen_adapts_to_wide_and_narrow_screens() {
        use crate::load::Wizard;
        for (w, h) in [(56u16, 18usize), (70, 20), (100, 24), (120, 30)] {
            let mut wizard = Wizard::new();
            wizard.tick();
            let view = wizard.view(h, "About 40 kB an hour.");
            let backend = TestBackend::new(w, h as u16);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| draw_load(frame, &view)).unwrap();
            let buf = terminal.backend().buffer().clone();
            let screen = screen_text(&buf);
            assert!(screen.contains("⢺ ⡗"), "failed at w={} h={}", w, h);
            assert!(!screen.contains("⢺⣿⡗"));
            assert!(!screen.contains("⢹⣿⡇"));
            assert!(screen.contains("(●)"), "failed at w={} h={}", w, h);
        }
    }

    #[test]
    fn the_load_screen_shows_moving_bars_alone_when_short() {
        use crate::load::Wizard;
        let mut wizard = Wizard::new();
        wizard.tick();
        let view = wizard.view(12, "About 40 kB an hour.");
        let backend = TestBackend::new(50, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw_load(frame, &view)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let screen = screen_text(&buf);
        assert!(screen.contains("Are you riding Amtrak right now? y/N"));
        assert!(screen.contains('⣿') || screen.contains('⣤') || screen.contains('⣶'));
        assert!(!screen.contains("(●)"));
    }

    fn screen_of(terminal: &Terminal<TestBackend>) -> String {
        screen_text(terminal.backend().buffer())
    }

    fn screen_text(buf: &ratatui::buffer::Buffer) -> String {
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
