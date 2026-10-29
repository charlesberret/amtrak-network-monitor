//! What the ride keeps, and how often it looks. The street, the Wi-Fi name,
//! the IP address, and a precise position are not settings. They are left out.

use serde_json::json;

pub const PROBE_CHOICES: [u64; 4] = [10, 20, 30, 60];
pub const LOC_CHOICES: [u64; 3] = [15, 30, 60];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gather {
    pub save_trip: bool,
    pub probe_secs: u64,
    pub loc_secs: u64,
    pub ping: bool,
    pub http: bool,
    pub position: bool,
}

impl Default for Gather {
    fn default() -> Self {
        Self {
            save_trip: true,
            probe_secs: crate::load::PROBE_SECS,
            loc_secs: 15,
            ping: true,
            http: true,
            position: true,
        }
    }
}

impl Gather {
    pub fn from_json(text: &str) -> Self {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            return Self::default();
        };
        let mut gather = Self::default();
        if let Some(save) = value.get("save_trip").and_then(|v| v.as_bool()) {
            gather.save_trip = save;
        }
        if let Some(secs) = value.get("probe_secs").and_then(|v| v.as_u64()) {
            if PROBE_CHOICES.contains(&secs) {
                gather.probe_secs = secs;
            }
        }
        if let Some(secs) = value.get("loc_secs").and_then(|v| v.as_u64()) {
            if LOC_CHOICES.contains(&secs) {
                gather.loc_secs = secs;
            }
        }
        if let Some(ping) = value.get("ping").and_then(|v| v.as_bool()) {
            gather.ping = ping;
        }
        if let Some(http) = value.get("http").and_then(|v| v.as_bool()) {
            gather.http = http;
        }
        if let Some(position) = value.get("position").and_then(|v| v.as_bool()) {
            gather.position = position;
        }
        gather
    }

    pub fn to_json(self) -> String {
        json!({
            "save_trip": self.save_trip,
            "probe_secs": self.probe_secs,
            "loc_secs": self.loc_secs,
            "ping": self.ping,
            "http": self.http,
            "position": self.position,
        })
        .to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Save,
    Probe,
    Location,
    Ping,
    Http,
    Position,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nudge {
    Up,
    Down,
    Space,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsDraft {
    pub cursor: usize,
    pub save: bool,
    pub probe_secs: u64,
    pub loc_secs: u64,
    pub ping: bool,
    pub http: bool,
    pub position: bool,
    /// The Mac helper writes on its own clock. The location row is not a control there.
    pub mac_helper: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingRow {
    pub text: String,
    pub selected: bool,
}

impl SettingsDraft {
    pub fn from_saved(gather: &Gather, mac_helper: bool) -> Self {
        Self {
            cursor: 0,
            save: gather.save_trip,
            probe_secs: gather.probe_secs,
            loc_secs: gather.loc_secs,
            ping: gather.ping,
            http: gather.http,
            position: gather.position,
            mac_helper,
        }
    }

    pub fn apply_to(&self, gather: &mut Gather) {
        gather.save_trip = self.save;
        gather.probe_secs = self.probe_secs;
        gather.loc_secs = self.loc_secs;
        gather.ping = self.ping;
        gather.http = self.http;
        gather.position = self.position;
    }

    pub fn nudge(&mut self, key: Nudge) {
        let n = self.fields().len();
        if n == 0 {
            return;
        }
        match key {
            Nudge::Up => self.cursor = (self.cursor + n - 1) % n,
            Nudge::Down => self.cursor = (self.cursor + 1) % n,
            Nudge::Space => self.activate(),
        }
    }

    pub fn rows(&self) -> Vec<SettingRow> {
        let fields = self.fields();
        fields
            .iter()
            .enumerate()
            .map(|(i, field)| SettingRow {
                text: format!("  {} {}", mark(i == self.cursor), self.label(*field)),
                selected: i == self.cursor,
            })
            .collect()
    }

    fn fields(&self) -> Vec<Field> {
        let mut fields = vec![Field::Save, Field::Probe];
        if !self.mac_helper {
            fields.push(Field::Location);
        }
        fields.extend([Field::Ping, Field::Http, Field::Position]);
        fields
    }

    fn activate(&mut self) {
        let Some(field) = self.fields().get(self.cursor).copied() else {
            return;
        };
        match field {
            Field::Save => self.save = !self.save,
            Field::Probe => self.probe_secs = cycle(&PROBE_CHOICES, self.probe_secs),
            Field::Location => self.loc_secs = cycle(&LOC_CHOICES, self.loc_secs),
            Field::Ping => self.ping = !self.ping,
            Field::Http => self.http = !self.http,
            Field::Position => self.position = !self.position,
        }
    }

    fn label(&self, field: Field) -> String {
        match field {
            Field::Save => format!("Save this trip          {}", on_off(self.save)),
            Field::Probe => format!("Connection every        {} s", self.probe_secs),
            Field::Location => format!("Location every          {} s", self.loc_secs),
            Field::Ping => format!("Ping                    {}", on_off(self.ping)),
            Field::Http => format!("Test page               {}", on_off(self.http)),
            Field::Position => format!("Position                {}", on_off(self.position)),
        }
    }
}

fn mark(selected: bool) -> &'static str {
    if selected {
        ">"
    } else {
        " "
    }
}

fn on_off(on: bool) -> &'static str {
    if on {
        "On"
    } else {
        "Off"
    }
}

fn cycle(choices: &[u64], current: u64) -> u64 {
    let Some(at) = choices.iter().position(|secs| *secs == current) else {
        return choices[0];
    };
    choices[(at + 1) % choices.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_settings_file_keeps_its_save_bit_and_the_new_defaults() {
        let gather = Gather::from_json(r#"{"save_trip":false}"#);
        assert!(!gather.save_trip);
        assert_eq!(gather.probe_secs, 20);
        assert_eq!(gather.loc_secs, 15);
        assert!(gather.ping && gather.http && gather.position);
    }

    #[test]
    fn a_rate_outside_the_list_is_ignored() {
        let gather = Gather::from_json(r#"{"probe_secs":1,"loc_secs":5}"#);
        assert_eq!(gather.probe_secs, 20);
        assert_eq!(gather.loc_secs, 15);
    }

    #[test]
    fn space_cycles_the_probe_and_toggles_a_measurement() {
        let mut draft = SettingsDraft::from_saved(&Gather::default(), false);
        draft.nudge(Nudge::Down);
        draft.nudge(Nudge::Space);
        assert_eq!(draft.probe_secs, 30);
        draft.nudge(Nudge::Down);
        draft.nudge(Nudge::Space);
        assert_eq!(draft.loc_secs, 30);
        draft.nudge(Nudge::Down);
        draft.nudge(Nudge::Space);
        assert!(!draft.ping);
        let value: serde_json::Value = serde_json::from_str(&draft.to_json_preview()).unwrap();
        let keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        assert!(!keys
            .iter()
            .any(|key| key == "ip" || key == "ssid" || key == "street"));
    }

    #[test]
    fn the_mac_list_has_no_location_control() {
        let draft = SettingsDraft::from_saved(&Gather::default(), true);
        let rows = draft.rows();
        assert!(rows.iter().any(|row| row.text.contains("Connection every")));
        assert!(rows.iter().all(|row| !row.text.contains("Location every")));
        assert!(rows[0].text.contains("Save this trip"));
        assert!(rows[0].selected);
    }

    impl SettingsDraft {
        fn to_json_preview(&self) -> String {
            let mut gather = Gather::default();
            self.apply_to(&mut gather);
            gather.to_json()
        }
    }
}
