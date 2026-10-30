use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

pub const FILE: &str = "crucible.toml";
pub const DEFAULT_PLACE: &str = "start";
pub const DEFAULT_PROJECT: &str = "default.project.json";

#[derive(Debug, PartialEq)]
pub struct Manifest {
    pub places: BTreeMap<String, Place>,
    pub envs: BTreeMap<String, Env>,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Place {
    pub project: PathBuf,
}

#[derive(Debug, PartialEq)]
pub struct Env {
    pub universe: NonZeroU64,
    pub profile: Option<String>,
    pub version_type: VersionType,
    pub places: BTreeMap<String, NonZeroU64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum VersionType {
    #[default]
    Saved,
    Published,
}

#[derive(Debug)]
pub enum ManifestError {
    Read(io::Error),
    Parse(toml::de::Error),
    NoPlaces,
    InvalidName { kind: &'static str, name: String },
    PlaceMapping { env: String },
    PlaceShorthand { env: String },
    UnknownPlace { env: String, place: String },
    UnmappedPlace { env: String, place: String },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(f, "cannot read {FILE}: {error}"),
            Self::Parse(error) => write!(f, "{FILE}: {error}"),
            Self::NoPlaces => write!(f, "{FILE}: [places] declares no places"),
            Self::InvalidName { kind, name } => {
                write!(f, "{FILE}: {kind} name {name:?} must use only letters, digits, '-' and '_'")
            }
            Self::PlaceMapping { env } => write!(f, "{FILE}: env {env} must set exactly one of place or places"),
            Self::PlaceShorthand { env } => write!(
                f,
                "{FILE}: env {env} sets place = <id>, which works only when {DEFAULT_PLACE} is the only place. Map each place with places = {{ <name> = <id>, ... }}"
            ),
            Self::UnknownPlace { env, place } => write!(f, "{FILE}: env {env} maps undeclared place {place}"),
            Self::UnmappedPlace { env, place } => write!(f, "{FILE}: env {env} does not map place {place}"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    places: Option<BTreeMap<String, Place>>,
    #[serde(default)]
    env: BTreeMap<String, RawEnv>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEnv {
    universe: NonZeroU64,
    profile: Option<String>,
    #[serde(default)]
    version_type: VersionType,
    place: Option<NonZeroU64>,
    places: Option<BTreeMap<String, NonZeroU64>>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            places: BTreeMap::from([(DEFAULT_PLACE.into(), Place { project: DEFAULT_PROJECT.into() })]),
            envs: BTreeMap::new(),
        }
    }
}

impl Manifest {
    pub fn read(root: &Path) -> Result<Option<Self>, ManifestError> {
        match std::fs::read_to_string(root.join(FILE)) {
            Ok(text) => Self::parse(&text).map(Some),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ManifestError::Read(error)),
        }
    }

    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let raw: RawManifest = toml::from_str(text).map_err(ManifestError::Parse)?;
        let places = match raw.places {
            None => Self::default().places,
            Some(places) if places.is_empty() => return Err(ManifestError::NoPlaces),
            Some(places) => places,
        };
        check_names("place", places.keys())?;
        check_names("env", raw.env.keys())?;
        let mut envs = BTreeMap::new();
        for (name, env) in raw.env {
            let mapping = match (env.place, env.places) {
                (Some(id), None) if places.len() == 1 && places.contains_key(DEFAULT_PLACE) => {
                    BTreeMap::from([(DEFAULT_PLACE.to_owned(), id)])
                }
                (Some(_), None) => return Err(ManifestError::PlaceShorthand { env: name }),
                (None, Some(mapping)) => mapping,
                _ => return Err(ManifestError::PlaceMapping { env: name }),
            };
            if let Some(place) = mapping.keys().find(|place| !places.contains_key(*place)) {
                return Err(ManifestError::UnknownPlace { env: name, place: place.clone() });
            }
            if let Some(place) = places.keys().find(|place| !mapping.contains_key(*place)) {
                return Err(ManifestError::UnmappedPlace { env: name, place: place.clone() });
            }
            let env =
                Env { universe: env.universe, profile: env.profile, version_type: env.version_type, places: mapping };
            envs.insert(name, env);
        }
        Ok(Self { places, envs })
    }
}

fn check_names<'a>(kind: &'static str, mut names: impl Iterator<Item = &'a String>) -> Result<(), ManifestError> {
    match names.find(|name| name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    {
        Some(name) => Err(ManifestError::InvalidName { kind, name: name.clone() }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: u64) -> NonZeroU64 {
        NonZeroU64::new(value).unwrap()
    }

    fn error(text: &str) -> ManifestError {
        Manifest::parse(text).expect_err("manifest must be rejected")
    }

    #[test]
    fn parses_places_and_envs() {
        let manifest = Manifest::parse(
            r#"
            [places.start]
            project = "default.project.json"

            [places.arena]
            project = "arena.project.json"

            [env.staging]
            universe = 1234
            profile = "publish"
            places = { start = 5678, arena = 5679 }

            [env.production]
            universe = 9999
            version_type = "Published"
            places = { start = 4321, arena = 4322 }
            "#,
        )
        .unwrap();
        assert_eq!(manifest.places["arena"].project, PathBuf::from("arena.project.json"));
        assert_eq!(
            manifest.envs["staging"],
            Env {
                universe: id(1234),
                profile: Some("publish".into()),
                version_type: VersionType::Saved,
                places: BTreeMap::from([("start".into(), id(5678)), ("arena".into(), id(5679))]),
            }
        );
        assert_eq!(manifest.envs["production"].version_type, VersionType::Published);
        assert_eq!(manifest.envs["production"].profile, None);
    }

    #[test]
    fn cli_guide_example_parses() {
        let guide = include_str!("../../../../docs/cli.md").replace("\r\n", "\n");
        let (_, rest) = guide.split_once("```toml\n[places.start]").expect("CLI guide shows a crucible.toml");
        let (body, _) = rest.split_once("```").unwrap();
        let manifest = Manifest::parse(&format!("[places.start]{body}")).unwrap();
        assert_eq!(manifest.envs.len(), 2);
    }

    #[test]
    fn missing_places_means_one_start_place() {
        assert_eq!(Manifest::parse("").unwrap(), Manifest::default());
        assert_eq!(Manifest::default().places["start"].project, PathBuf::from("default.project.json"));
        let manifest = Manifest::parse("[env.production]\nuniverse = 9999\nplace = 4321\n").unwrap();
        assert_eq!(manifest.places, Manifest::default().places);
        assert_eq!(manifest.envs["production"].places, BTreeMap::from([("start".into(), id(4321))]));
    }

    #[test]
    fn absent_file_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Manifest::read(dir.path()).unwrap(), None);
        std::fs::write(dir.path().join(FILE), "[places.arena]\nproject = \"arena.project.json\"\n").unwrap();
        assert!(Manifest::read(dir.path()).unwrap().unwrap().places.contains_key("arena"));
    }

    #[test]
    fn unreadable_file_is_a_read_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(FILE)).unwrap();
        assert!(matches!(Manifest::read(dir.path()), Err(ManifestError::Read(_))));
    }

    #[test]
    fn unknown_keys_are_rejected_everywhere() {
        for text in [
            "name = \"game\"\n",
            "[places.start]\nproject = \"a.project.json\"\npath = \"x\"\n",
            "[env.staging]\nuniverse = 1\nplace = 2\nkey = \"secret\"\n",
        ] {
            let message = error(text).to_string();
            assert!(message.starts_with("crucible.toml: ") && message.contains("unknown field"), "{message}");
        }
    }

    #[test]
    fn wrong_types_and_zero_ids_are_parse_errors() {
        for text in [
            "[places.start]\n",
            "[env.staging]\nplace = 2\n",
            "[env.staging]\nuniverse = 0\nplace = 2\n",
            "[env.staging]\nuniverse = -1\nplace = 2\n",
            "[env.staging]\nuniverse = 1\nplaces = { start = 0 }\n",
            "[env.staging]\nuniverse = 1\nplace = 2\nversion_type = \"Draft\"\n",
        ] {
            assert!(matches!(error(text), ManifestError::Parse(_)), "{text}");
        }
    }

    #[test]
    fn empty_places_table_is_rejected() {
        assert!(matches!(error("[places]\n"), ManifestError::NoPlaces));
    }

    #[test]
    fn names_must_be_file_and_flag_safe() {
        for (text, kind, name) in [
            ("[places.\"a b\"]\nproject = \"a.project.json\"\n", "place", "a b"),
            ("[places.\"\"]\nproject = \"a.project.json\"\n", "place", ""),
            ("[env.\"../prod\"]\nuniverse = 1\nplace = 2\n", "env", "../prod"),
        ] {
            match error(text) {
                ManifestError::InvalidName { kind: k, name: n } => assert_eq!((k, n.as_str()), (kind, name)),
                other => panic!("{text}: {other:?}"),
            }
        }
    }

    #[test]
    fn env_sets_exactly_one_place_form() {
        for text in
            ["[env.staging]\nuniverse = 1\n", "[env.staging]\nuniverse = 1\nplace = 2\nplaces = { start = 3 }\n"]
        {
            match error(text) {
                ManifestError::PlaceMapping { env } => assert_eq!(env, "staging"),
                other => panic!("{text}: {other:?}"),
            }
        }
    }

    #[test]
    fn env_cannot_map_an_undeclared_place() {
        match error("[env.staging]\nuniverse = 1\nplaces = { start = 2, arena = 3 }\n") {
            ManifestError::UnknownPlace { env, place } => {
                assert_eq!((env.as_str(), place.as_str()), ("staging", "arena"))
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn place_shorthand_needs_start_as_the_only_place() {
        let arena = "[places.arena]\nproject = \"arena.project.json\"\n";
        let start = "[places.start]\nproject = \"default.project.json\"\n";
        for places in [arena.to_owned(), format!("{start}{arena}")] {
            let failure = error(&format!("{places}[env.staging]\nuniverse = 1\nplace = 2\n"));
            assert_eq!(
                failure.to_string(),
                "crucible.toml: env staging sets place = <id>, which works only when start is the only place. \
                 Map each place with places = { <name> = <id>, ... }"
            );
        }
        let manifest = Manifest::parse(&format!("{start}[env.staging]\nuniverse = 1\nplace = 2\n")).unwrap();
        assert_eq!(manifest.envs["staging"].places, BTreeMap::from([("start".into(), id(2))]));
    }

    #[test]
    fn every_env_maps_every_place() {
        let text = "[places.start]\nproject = \"default.project.json\"\n[places.arena]\nproject = \"arena.project.json\"\n\
                    [env.staging]\nuniverse = 1\nplaces = { start = 2 }\n";
        let failure = error(text);
        assert_eq!(failure.to_string(), "crucible.toml: env staging does not map place arena");
        assert!(matches!(failure, ManifestError::UnmappedPlace { env, place } if env == "staging" && place == "arena"));
    }
}
