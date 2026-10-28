//! The screen for a ride. One colored word, the rest quiet.

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

fn header(snap: &Snapshot, width: u16) -> Line<'static> {
    let title = match snap.line.as_deref() {
        Some(line) if !line.is_empty() => format!("Amtrak {line} Network Monitor"),
        _ => "Amtrak Network Monitor".to_string(),
    };
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
        lines.push(dim(clip(
            &format!("  {lat:.3}, {lon:.3}"),
            cols(width),
        )));
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
        dim(clip("  s  settings     q  quit", cols(width))),
    ]
}

pub fn draw_settings(frame: &mut Frame, snap: &Snapshot) {
    let area = frame.area();
    frame.render_widget(Paragraph::new(""), area);
    let width = area.width;
    let saving = if snap.saving { "On" } else { "Off" };
    let lines = vec![
        blank(),
        styled(
            clip("  Settings", cols(width)),
            Style::new().fg(INK).add_modifier(Modifier::BOLD),
        ),
        blank(),
        ink(clip(
            &format!("  Save this trip          {saving}"),
            cols(width),
        )),
        blank(),
        dim(clip("  Position                about 100 m", cols(width))),
        dim(clip("  Town                    kept", cols(width))),
        dim(clip("  Street address          not saved", cols(width))),
        dim(clip("  Wi-Fi name              not saved", cols(width))),
        dim(clip("  IP address              not saved", cols(width))),
        blank(),
        dim(clip("  space  save on or off", cols(width))),
        dim(clip("  s  close", cols(width))),
        dim(clip("  q  quit", cols(width))),
    ];
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

    fn snap(history: Vec<MinuteLight>) -> Snapshot {
        Snapshot {
            at: Local.with_ymd_and_hms(2026, 10, 21, 14, 23, 40).unwrap(),
            line: Some("Pennsylvanian".into()),
            place: "Millerstown, PA".into(),
            position: Some((40.536, -77.150)),
            motion: None,
            tone: Tone::Clear,
            pace: "54 ms, no loss".into(),
            page: "test page loaded in 0.22 s".into(),
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
            .draw(|frame| draw_settings(frame, &view))
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
        assert!(screen.contains("Street address"));
        assert!(screen.contains("not saved"));
        assert!(screen.contains("Wi-Fi name"));
        assert!(screen.contains("IP address"));
        assert!(!screen.contains("sampler"));
    }
}
