//! Rider-facing Amtrak services as of the 25 October 2026 timetable index.
//! A bundled PDF (Illinois Service, Michigan Services, the corridor file)
//! is split into the trains a rider would name. Floridian is the current
//! name of the old Capitol Limited and Silver Star. Thruway buses are not
//! trains. Capitol Limited and Silver Star are not current names.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Service {
    pub name: &'static str,
    pub seasonal: bool,
    pub also: &'static [&'static str],
}

impl Service {
    pub fn list_label(self) -> String {
        if self.seasonal {
            format!("{} (seasonal)", self.name)
        } else {
            self.name.to_string()
        }
    }
}

const LINES: &[Service] = &[
    Service {
        name: "Acela",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Adirondack",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Auto Train",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Berkshire Flyer",
        seasonal: true,
        also: &[],
    },
    Service {
        name: "Blue Water",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Borealis",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "California Zephyr",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Capitol Corridor",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Cardinal",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Carl Sandburg",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Carolinian",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Cascades",
        seasonal: false,
        also: &["amtrak cascades"],
    },
    Service {
        name: "City of New Orleans",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Coast Starlight",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Crescent",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Downeaster",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Empire Builder",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Empire Service",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Ethan Allen Express",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Floridian",
        seasonal: false,
        also: &["capitol limited", "silver star"],
    },
    Service {
        name: "Gold Runner",
        seasonal: false,
        also: &["san joaquin", "san joaquins"],
    },
    Service {
        name: "Hartford Line",
        seasonal: false,
        also: &["ctrail", "ct rail"],
    },
    Service {
        name: "Heartland Flyer",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Hiawatha",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Illini",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Illinois Zephyr",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Keystone Service",
        seasonal: false,
        also: &["keystone"],
    },
    Service {
        name: "Lake Shore Limited",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Lincoln Service",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Maple Leaf",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Mardi Gras",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Missouri River Runner",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Northeast Regional",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Pacific Surfliner",
        seasonal: false,
        also: &["surfliner"],
    },
    Service {
        name: "Palmetto",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Pennsylvanian",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Pere Marquette",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Piedmont",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Saluki",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Silver Meteor",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Southwest Chief",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Sunset Limited",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Texas Eagle",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Valley Flyer",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Vermonter",
        seasonal: false,
        also: &[],
    },
    Service {
        name: "Winter Park Express",
        seasonal: true,
        also: &[],
    },
    Service {
        name: "Wolverine",
        seasonal: false,
        also: &[],
    },
];

pub fn filter(query: &str) -> Vec<&'static Service> {
    let q = query.trim().to_ascii_lowercase();
    LINES
        .iter()
        .filter(|service| q.is_empty() || matches(service, &q))
        .collect()
}

fn matches(service: &Service, q: &str) -> bool {
    service.name.to_ascii_lowercase().contains(q)
        || service.also.iter().any(|alias| alias.contains(q))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(query: &str) -> Vec<&'static str> {
        filter(query)
            .into_iter()
            .map(|service| service.name)
            .collect()
    }

    #[test]
    fn names_are_unique_and_the_bundles_are_split() {
        let mut found: Vec<_> = LINES.iter().map(|service| service.name).collect();
        let count = found.len();
        found.sort();
        found.dedup();
        assert_eq!(found.len(), count);
        assert!(names("").contains(&"Wolverine"));
        assert!(names("").contains(&"Illini"));
        assert!(names("").contains(&"Piedmont"));
        assert!(names("").contains(&"Northeast Regional"));
        assert!(names("").contains(&"Acela"));
        assert!(!names("").contains(&"Illinois Service"));
        assert!(!names("").contains(&"Michigan Services"));
        assert!(!names("").contains(&"Northeast Corridor Mainline"));
        assert!(!names("").contains(&"Capitol Limited"));
        assert!(!names("").contains(&"Silver Star"));
    }

    #[test]
    fn search_finds_a_line_and_an_old_name_points_at_the_current_one() {
        assert_eq!(names("penn"), vec!["Pennsylvanian"]);
        assert_eq!(names("PENN"), vec!["Pennsylvanian"]);
        assert_eq!(names("capitol limited"), vec!["Floridian"]);
        assert_eq!(names("silver star"), vec!["Floridian"]);
        assert_eq!(names("san joaquin"), vec!["Gold Runner"]);
        assert!(names("capitol").contains(&"Capitol Corridor"));
        assert!(names("capitol").contains(&"Floridian"));
        let flyer = LINES
            .iter()
            .find(|service| service.name == "Berkshire Flyer")
            .unwrap();
        assert_eq!(flyer.list_label(), "Berkshire Flyer (seasonal)");
    }
}
