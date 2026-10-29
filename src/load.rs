//! The questions asked once, when the monitor starts.
//! Enter on a y/N question is no. A self-reload does not ask again.

use crate::lines::{self, Service};

/// The connection is probed on this interval unless settings say otherwise.
pub const PROBE_SECS: u64 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    AmtrakWifi,
    PersonalWifi,
}

impl LinkKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::AmtrakWifi => "Free Amtrak WiFi",
            Self::PersonalWifi => "Personal WiFi (phone or hotspot)",
        }
    }

    pub fn stored(self) -> &'static str {
        match self {
            Self::AmtrakWifi => "amtrak-wifi",
            Self::PersonalWifi => "personal-wifi",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "amtrak-wifi" => Some(Self::AmtrakWifi),
            "personal-wifi" => Some(Self::PersonalWifi),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Board {
    pub on_amtrak: bool,
    pub line: Option<String>,
    pub link: Option<LinkKind>,
    pub save: bool,
    pub share: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    Riding,
    NotRiding,
    Line { query: String, cursor: usize },
    Link { cursor: usize },
    Store,
    Share,
}

#[derive(Clone, Debug)]
pub struct Wizard {
    phase: Phase,
    line: Option<String>,
    link: Option<LinkKind>,
    tick: usize,
}

impl Wizard {
    pub fn new() -> Self {
        Self {
            phase: Phase::Riding,
            line: None,
            link: None,
            tick: 0,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn tick_count(&self) -> usize {
        self.tick
    }

    pub fn view(&self, height: usize, storage: &str) -> LoadView {
        match &self.phase {
            Phase::Riding => LoadView {
                lines: vec![
                    prompt("Are you riding Amtrak right now? y/N"),
                    blank(),
                    dim("  enter is no"),
                ],
                pinned: None,
                foot: Foot::Quit,
                anim_tick: Some(self.tick),
            },
            Phase::NotRiding => LoadView {
                lines: vec![prompt(
                    "No? OK. It still watches the connection.",
                )],
                pinned: None,
                foot: Foot::EnterQuit,
                anim_tick: None,
            },
            Phase::Line { query, cursor } => line_view(query, *cursor, height),
            Phase::Link { cursor } => LoadView {
                lines: vec![
                    prompt("How are you connected?"),
                    blank(),
                    choice(0, *cursor, &format!("1  {}", LinkKind::AmtrakWifi.label())),
                    choice(1, *cursor, &format!("2  {}", LinkKind::PersonalWifi.label())),
                ],
                pinned: None,
                foot: Foot::Quit,
                anim_tick: None,
            },
            Phase::Store => LoadView {
                lines: vec![
                    prompt("Store trips in a local database on this computer? y/N"),
                    blank(),
                    text("A saved trip is kept so a later ride can warn you before a dead zone from an earlier trip, and so Amtrak Wi-Fi can be compared with a personal connection."),
                    blank(),
                    text(storage),
                    text("Nothing is uploaded."),
                ],
                pinned: None,
                foot: Foot::Quit,
                anim_tick: None,
            },
            Phase::Share => LoadView {
                lines: vec![
                    prompt("Share this trip anonymously? y/N"),
                    blank(),
                    text("Only the train line, the location to about 100 m, and the connection measurement. A public map of coverage, and of how the lines differ, would use those."),
                    blank(),
                    text("A yes keeps that anonymous copy on this computer. Nothing is sent until you run `share` after the ride."),
                ],
                pinned: None,
                foot: Foot::Quit,
                anim_tick: None,
            },
        }
    }

    pub fn push(&mut self, press: Press) -> Outcome {
        match &mut self.phase {
            Phase::Riding => match press {
                Press::Char('y' | 'Y') => {
                    self.phase = Phase::Line {
                        query: String::new(),
                        cursor: 0,
                    };
                    Outcome::Stay
                }
                Press::Char('n' | 'N') | Press::Enter => {
                    self.phase = Phase::NotRiding;
                    Outcome::Stay
                }
                Press::Char('q' | 'Q') | Press::Esc => Outcome::Quit,
                _ => Outcome::Stay,
            },
            Phase::NotRiding => match press {
                Press::Enter => Outcome::Done(Board {
                    on_amtrak: false,
                    line: None,
                    link: None,
                    save: false,
                    share: false,
                }),
                Press::Char('q' | 'Q') | Press::Esc => Outcome::Quit,
                _ => Outcome::Stay,
            },
            Phase::Line { query, cursor } => match press {
                Press::Char(ch) if !ch.is_control() => {
                    query.push(ch);
                    *cursor = 0;
                    Outcome::Stay
                }
                Press::Backspace => {
                    query.pop();
                    *cursor = 0;
                    Outcome::Stay
                }
                Press::Up => {
                    *cursor = cursor.saturating_sub(1);
                    Outcome::Stay
                }
                Press::Down => {
                    let n = lines::filter(query).len();
                    if *cursor < n {
                        *cursor += 1;
                    }
                    Outcome::Stay
                }
                Press::Enter => {
                    let found = lines::filter(query);
                    self.line = if *cursor < found.len() {
                        Some(found[*cursor].name.to_string())
                    } else {
                        None
                    };
                    self.phase = Phase::Link { cursor: 0 };
                    Outcome::Stay
                }
                Press::Esc => Outcome::Quit,
                _ => Outcome::Stay,
            },
            Phase::Link { cursor } => match press {
                Press::Char('1') => self.choose_link(LinkKind::AmtrakWifi),
                Press::Char('2') => self.choose_link(LinkKind::PersonalWifi),
                Press::Up => {
                    *cursor = 0;
                    Outcome::Stay
                }
                Press::Down => {
                    *cursor = 1;
                    Outcome::Stay
                }
                Press::Enter => {
                    let kind = if *cursor == 0 {
                        LinkKind::AmtrakWifi
                    } else {
                        LinkKind::PersonalWifi
                    };
                    self.choose_link(kind)
                }
                Press::Char('q' | 'Q') | Press::Esc => Outcome::Quit,
                _ => Outcome::Stay,
            },
            Phase::Store => match press {
                Press::Char('y' | 'Y') => {
                    self.phase = Phase::Share;
                    Outcome::Stay
                }
                Press::Char('n' | 'N') | Press::Enter => Outcome::Done(self.finish(false, false)),
                Press::Char('q' | 'Q') | Press::Esc => Outcome::Quit,
                _ => Outcome::Stay,
            },
            Phase::Share => match press {
                Press::Char('y' | 'Y') => Outcome::Done(self.finish(true, true)),
                Press::Char('n' | 'N') | Press::Enter => Outcome::Done(self.finish(true, false)),
                Press::Char('q' | 'Q') | Press::Esc => Outcome::Quit,
                _ => Outcome::Stay,
            },
        }
    }

    fn choose_link(&mut self, kind: LinkKind) -> Outcome {
        self.link = Some(kind);
        self.phase = Phase::Store;
        Outcome::Stay
    }

    fn finish(&self, save: bool, share: bool) -> Board {
        Board {
            on_amtrak: true,
            line: self.line.clone(),
            link: self.link,
            save,
            share: share && save,
        }
    }
}

fn line_view(query: &str, cursor: usize, height: usize) -> LoadView {
    let found = lines::filter(query);
    let mut lines = vec![
        prompt("Cool, which line are you riding?"),
        blank(),
        if query.is_empty() {
            dim("  type to search")
        } else {
            text(format!("  {query}"))
        },
        blank(),
    ];
    let room = height
        .saturating_sub(2 + 1)
        .saturating_sub(lines.len())
        .max(1);
    let (start, end) = window(found.len(), cursor, room);
    if found.is_empty() {
        lines.push(dim("  no line by that name"));
    }
    for (i, service) in found.iter().enumerate().take(end).skip(start) {
        lines.push(service_row(service, i == cursor));
    }
    let pinned = cursor >= found.len();
    LoadView {
        lines,
        pinned: Some(choice_text(
            if pinned {
                "  > Continue without selecting a line"
            } else {
                "    Continue without selecting a line"
            },
            pinned,
        )),
        foot: Foot::Esc,
        anim_tick: None,
    }
}

fn service_row(service: &Service, selected: bool) -> LoadLine {
    let mark = if selected { ">" } else { " " };
    choice_text(&format!("  {mark} {}", service.list_label()), selected)
}

fn window(len: usize, cursor: usize, room: usize) -> (usize, usize) {
    if len == 0 || room == 0 {
        return (0, 0);
    }
    let focus = cursor.min(len - 1);
    let mut start = focus.saturating_sub(room / 2);
    if start + room > len {
        start = len.saturating_sub(room);
    }
    (start, (start + room).min(len))
}

fn choice(index: usize, cursor: usize, label: &str) -> LoadLine {
    let selected = index == cursor;
    let mark = if selected { ">" } else { " " };
    choice_text(&format!("  {mark} {label}"), selected)
}

#[cfg(test)]
pub fn about_kb_an_hour(row_bytes: usize) -> String {
    about_kb(row_bytes, PROBE_SECS)
}

pub fn about_kb(row_bytes: usize, probe_secs: u64) -> String {
    let samples = (3_600 / probe_secs.max(1)) as usize;
    let per_hour = row_bytes.saturating_mul(samples);
    let kb = (per_hour + 512) / 1024;
    format!("About {kb} kB an hour.")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Char(char),
    Enter,
    Backspace,
    Up,
    Down,
    Esc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Quit,
    Done(Board),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Foot {
    Quit,
    Esc,
    EnterQuit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineTone {
    Prompt,
    Text,
    Dim,
    Selected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadLine {
    pub text: String,
    pub tone: LineTone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadView {
    pub lines: Vec<LoadLine>,
    /// Kept on the last row above the keys. The line list scrolls above it.
    pub pinned: Option<LoadLine>,
    pub foot: Foot,
    pub anim_tick: Option<usize>,
}

impl LoadView {
    #[cfg(test)]
    pub fn plain(&self) -> String {
        let mut parts: Vec<&str> = self.lines.iter().map(|line| line.text.as_str()).collect();
        if let Some(pinned) = &self.pinned {
            parts.push(pinned.text.as_str());
        }
        parts.join("\n")
    }
}

fn prompt(text: &str) -> LoadLine {
    LoadLine {
        text: format!("  {text}"),
        tone: LineTone::Prompt,
    }
}

fn text(text: impl Into<String>) -> LoadLine {
    LoadLine {
        text: format!("  {}", text.into()),
        tone: LineTone::Text,
    }
}

fn dim(text: &str) -> LoadLine {
    LoadLine {
        text: text.to_string(),
        tone: LineTone::Dim,
    }
}

fn blank() -> LoadLine {
    dim("")
}

fn choice_text(text: &str, selected: bool) -> LoadLine {
    LoadLine {
        text: text.to_string(),
        tone: if selected {
            LineTone::Selected
        } else {
            LineTone::Dim
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dump(wizard: &Wizard) -> String {
        wizard.view(30, "About 40 kB an hour.").plain()
    }

    fn ride_to_store(link: char) -> Wizard {
        let mut wizard = Wizard::new();
        assert_eq!(wizard.push(Press::Char('y')), Outcome::Stay);
        for ch in "pennsylvanian".chars() {
            wizard.push(Press::Char(ch));
        }
        wizard.push(Press::Enter);
        wizard.push(Press::Char(link));
        wizard
    }

    #[test]
    fn enter_is_no_and_the_ride_questions_are_not_asked() {
        let mut wizard = Wizard::new();
        assert!(dump(&wizard).contains("Are you riding Amtrak right now? y/N"));
        wizard.push(Press::Enter);
        let text = dump(&wizard);
        assert!(text.contains("No? OK. It still watches the connection."));
        assert!(!text.contains("How are you connected?"));
        assert!(!text.contains("Store trips"));
        match wizard.push(Press::Enter) {
            Outcome::Done(board) => {
                assert!(!board.on_amtrak);
                assert!(board.line.is_none());
                assert!(board.link.is_none());
                assert!(!board.save);
                assert!(!board.share);
            }
            other => panic!("expected the monitor, got {other:?}"),
        }
    }

    #[test]
    fn a_yes_picks_a_line_then_a_connection_then_storage_then_sharing() {
        let mut wizard = Wizard::new();
        wizard.push(Press::Char('Y'));
        let text = wizard.view(80, "About 40 kB an hour.").plain();
        assert!(text.contains("Cool, which line are you riding?"));
        assert!(text.contains("Continue without selecting a line"));
        assert!(text.contains("Pennsylvanian"));
        assert!(text.contains("Floridian"));
        assert!(!text.contains("Capitol Limited"));
        assert!(!text.contains("How are you connected?"));
        for ch in "penn".chars() {
            wizard.push(Press::Char(ch));
        }
        let text = dump(&wizard);
        assert!(text.contains("Pennsylvanian"));
        assert!(!text.contains("Palmetto"));
        wizard.push(Press::Enter);
        let text = dump(&wizard);
        assert!(text.contains("How are you connected?"));
        assert!(text.contains("1  Free Amtrak WiFi"));
        assert!(text.contains("2  Personal WiFi (phone or hotspot)"));
        wizard.push(Press::Char('2'));
        let text = dump(&wizard);
        assert!(text.contains("Store trips in a local database on this computer? y/N"));
        assert!(text.contains("dead zone"));
        assert!(text.contains("personal connection"));
        assert!(text.contains("About 40 kB an hour."));
        assert!(text.contains("Nothing is uploaded."));
        assert!(!text.contains("Share this trip"));
        wizard.push(Press::Char('y'));
        let text = dump(&wizard);
        assert!(text.contains("Share this trip anonymously? y/N"));
        assert!(text.contains("train line"));
        assert!(text.contains("location to about 100 m"));
        assert!(text.contains("connection measurement"));
        assert!(text.contains("Nothing is sent until you run `share`"));
        match wizard.push(Press::Char('y')) {
            Outcome::Done(board) => {
                assert!(board.on_amtrak);
                assert_eq!(board.line.as_deref(), Some("Pennsylvanian"));
                assert_eq!(board.link, Some(LinkKind::PersonalWifi));
                assert!(board.save);
                assert!(board.share);
            }
            other => panic!("expected a finished board, got {other:?}"),
        }
    }

    #[test]
    fn declining_storage_does_not_ask_to_share() {
        let mut wizard = ride_to_store('1');
        match wizard.push(Press::Enter) {
            Outcome::Done(board) => {
                assert_eq!(board.line.as_deref(), Some("Pennsylvanian"));
                assert_eq!(board.link, Some(LinkKind::AmtrakWifi));
                assert!(!board.save);
                assert!(!board.share);
            }
            other => panic!("expected to skip sharing, got {other:?}"),
        }
    }

    #[test]
    fn continue_without_a_line_still_asks_how_you_are_connected() {
        let mut wizard = Wizard::new();
        wizard.push(Press::Char('y'));
        for ch in "zzzz".chars() {
            wizard.push(Press::Char(ch));
        }
        assert!(dump(&wizard).contains("no line by that name"));
        wizard.push(Press::Enter);
        assert!(dump(&wizard).contains("How are you connected?"));
        wizard.push(Press::Char('1'));
        match wizard.push(Press::Char('n')) {
            Outcome::Done(board) => {
                assert!(board.on_amtrak);
                assert!(board.line.is_none());
                assert_eq!(board.link, Some(LinkKind::AmtrakWifi));
                assert!(!board.save);
            }
            other => panic!("expected no line, got {other:?}"),
        }
    }

    #[test]
    fn the_bottom_option_selects_no_line() {
        let mut wizard = Wizard::new();
        wizard.push(Press::Char('y'));
        for _ in 0..80 {
            wizard.push(Press::Down);
        }
        wizard.push(Press::Enter);
        wizard.push(Press::Enter);
        match wizard.push(Press::Enter) {
            Outcome::Done(board) => assert!(board.line.is_none()),
            other => panic!("expected the pinned option, got {other:?}"),
        }
    }

    #[test]
    fn backspace_clears_the_search() {
        let mut wizard = Wizard::new();
        wizard.push(Press::Char('y'));
        for _ in 0..4 {
            wizard.push(Press::Char('z'));
        }
        assert!(dump(&wizard).contains("no line by that name"));
        for _ in 0..4 {
            wizard.push(Press::Backspace);
        }
        assert!(dump(&wizard).contains("type to search"));
        assert!(dump(&wizard).contains("Acela"));
    }

    #[test]
    fn q_quits_except_while_typing_a_line() {
        let mut wizard = Wizard::new();
        assert_eq!(wizard.push(Press::Char('q')), Outcome::Quit);
        let mut wizard = Wizard::new();
        wizard.push(Press::Char('y'));
        wizard.push(Press::Char('q'));
        assert!(dump(&wizard).contains("Pere Marquette"));
        assert_eq!(wizard.push(Press::Esc), Outcome::Quit);
    }

    #[test]
    fn an_hour_of_rows_is_reported_in_kilobytes() {
        let line = about_kb_an_hour(250);
        assert_eq!(line, "About 44 kB an hour.");
    }

    #[test]
    fn wizard_ticks_increment_counter_and_provide_anim_tick() {
        let mut wizard = Wizard::new();
        assert_eq!(wizard.tick_count(), 0);
        let view = wizard.view(30, "About 40 kB an hour.");
        assert_eq!(view.anim_tick, Some(0));

        wizard.tick();
        assert_eq!(wizard.tick_count(), 1);
        let view = wizard.view(30, "About 40 kB an hour.");
        assert_eq!(view.anim_tick, Some(1));

        wizard.push(Press::Char('y'));
        let view = wizard.view(30, "About 40 kB an hour.");
        assert_eq!(view.anim_tick, None);
    }
}
