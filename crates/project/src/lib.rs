use experience::{Badge, BadgeOverlay, Declared, Located, Overlays, Priced, PricedOverlay, Settings, Tables};
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use toml::Spanned;

pub mod experience;
pub mod rojo;

pub const FILE: &str = "crucible.toml";
pub const DEFAULT_PLACE: &str = "start";
pub const DEFAULT_PROJECT: &str = "default.project.json";
pub const DEFAULT_TSCONFIG: &str = "tsconfig.json";
pub const DEFAULT_INCLUDE: &str = "include";

#[derive(Debug, PartialEq)]
pub struct Manifest {
    pub places: BTreeMap<String, Place>,
    pub envs: BTreeMap<String, Env>,
    pub experience: Declared,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Place {
    /// The Rojo project that lays out the place.
    pub project: PathBuf,
    #[serde(default = "Place::default_tsconfig")]
    pub tsconfig: PathBuf,
    /// The folder the project maps for the runtime library.
    #[serde(default = "Place::default_include")]
    pub include: PathBuf,
}

impl Place {
    fn default_tsconfig() -> PathBuf {
        DEFAULT_TSCONFIG.into()
    }

    fn default_include() -> PathBuf {
        DEFAULT_INCLUDE.into()
    }
}

#[derive(Debug, PartialEq)]
pub struct Env {
    pub universe: NonZeroU64,
    pub version_type: VersionType,
    /// Each place's ID, by place name, or `None` for an environment that sets neither `place` nor `places`, which
    /// `crucible deploy` refuses.
    pub places: Option<BTreeMap<String, NonZeroU64>>,
    pub overlays: Overlays,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
pub enum VersionType {
    #[default]
    Saved,
    Published,
}

#[derive(Debug)]
pub enum ManifestError {
    Read(io::Error),
    Parse { line: Option<usize>, message: String },
    NeedsVersion { needed: semver::Version, running: semver::Version },
    InvalidVersion { line: usize, value: String },
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
            Self::Parse { line: Some(line), message } => write!(f, "{FILE}:{line}: {message}"),
            Self::Parse { line: None, message } => write!(f, "{FILE}: {message}"),
            Self::NeedsVersion { needed, running } => write!(
                f,
                "{FILE}: this project needs crucible {needed} or later, and this is crucible {running}. Run `npx crucible upgrade {needed}`"
            ),
            Self::InvalidVersion { line, value } => {
                write!(f, "{FILE}:{line}: crucible: {value:?} is not a version such as \"0.6\" or \"0.6.1\"")
            }
            Self::NoPlaces => write!(f, "{FILE}: [places] declares no places"),
            Self::InvalidName { kind, name } => {
                write!(f, "{FILE}: {kind} name {name:?} must use only letters, digits, '-' and '_'")
            }
            Self::PlaceMapping { env } => write!(f, "{FILE}: env {env} sets both place and places. Set one of them"),
            Self::PlaceShorthand { env } => write!(
                f,
                "{FILE}: env {env} sets place = <id>, which works only when {DEFAULT_PLACE} is the only place. Map each place with places = {{ <name> = <id>, ... }}"
            ),
            Self::UnknownPlace { env, place } => write!(f, "{FILE}: env {env} maps undeclared place {place}"),
            Self::UnmappedPlace { env, place } => write!(f, "{FILE}: env {env} does not map place {place}"),
        }
    }
}

/// A Crucible game's places, environments and experience.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "crucible.toml")]
struct RawManifest {
    /// The oldest crucible that can read this file, such as "0.6".
    #[schemars(with = "Option<String>", regex(pattern = r"^[0-9]+(\.[0-9]+){0,2}$"))]
    crucible: Option<Spanned<String>>,
    #[schemars(extend("minProperties" = 1, "propertyNames" = { "pattern": "^[A-Za-z0-9_-]+$" }))]
    places: Option<BTreeMap<String, Place>>,
    #[serde(default)]
    #[schemars(extend("propertyNames" = { "pattern": "^[A-Za-z0-9_-]+$" }))]
    env: BTreeMap<String, RawEnv>,
    /// Settings of the universe.
    #[schemars(with = "Option<Settings>")]
    experience: Option<Spanned<Settings>>,
    /// Developer products, by key. A key uses lowercase letters, digits and '-', and starts with a letter.
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, Priced>", extend("propertyNames" = { "pattern": "^[a-z][a-z0-9-]*$" }))]
    products: BTreeMap<Spanned<String>, Priced>,
    /// Game passes, by key.
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, Priced>", extend("propertyNames" = { "pattern": "^[a-z][a-z0-9-]*$" }))]
    passes: BTreeMap<Spanned<String>, Priced>,
    /// Badges, by key.
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, Badge>", extend("propertyNames" = { "pattern": "^[a-z][a-z0-9-]*$" }))]
    badges: BTreeMap<Spanned<String>, Badge>,
}

/// One Roblox universe the game deploys to. It sets one of `place` and `places`, or neither when `crucible deploy` never
/// publishes it, such as for a game that publishes its places with its own scripts.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "Environment", extend("not" = { "required": ["place", "places"] }))]
struct RawEnv {
    universe: NonZeroU64,
    #[serde(default)]
    version_type: VersionType,
    /// The place ID when `start` is the only place.
    place: Option<NonZeroU64>,
    /// Each place's ID, by place name.
    #[schemars(extend("minProperties" = 1, "propertyNames" = { "pattern": "^[A-Za-z0-9_-]+$" }))]
    places: Option<BTreeMap<String, NonZeroU64>>,
    /// This environment's changes to declared products.
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, PricedOverlay>", extend("propertyNames" = { "pattern": "^[a-z][a-z0-9-]*$" }))]
    products: BTreeMap<Spanned<String>, PricedOverlay>,
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, PricedOverlay>", extend("propertyNames" = { "pattern": "^[a-z][a-z0-9-]*$" }))]
    passes: BTreeMap<Spanned<String>, PricedOverlay>,
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, BadgeOverlay>", extend("propertyNames" = { "pattern": "^[a-z][a-z0-9-]*$" }))]
    badges: BTreeMap<Spanned<String>, BadgeOverlay>,
}

/// The JSON Schema of `crucible.toml`, for editors such as Taplo.
pub fn schema() -> String {
    let generator = schemars::generate::SchemaSettings::draft2020_12()
        .with_transform(schemars::transform::RecursiveTransform(without_null))
        .into_generator();
    serde_json::to_string_pretty(&generator.into_root_schema_for::<RawManifest>()).expect("a schema serializes") + "\n"
}

/// TOML has no null, so an optional key is one that may be absent, not one that may be null.
fn without_null(schema: &mut schemars::Schema) {
    let Some(object) = schema.as_object_mut() else { return };
    if let Some(serde_json::Value::Array(types)) = object.get_mut("type") {
        types.retain(|kind| kind != "null");
        if let [kind] = types.as_slice() {
            let kind = kind.clone();
            object.insert("type".into(), kind);
        }
    }
    if let Some(serde_json::Value::Array(choices)) = object.get("anyOf") {
        let kept: Vec<_> =
            choices.iter().filter(|choice| **choice != serde_json::json!({ "type": "null" })).cloned().collect();
        if let [serde_json::Value::Object(only)] = kept.as_slice() {
            let only = only.clone();
            object.remove("anyOf");
            object.extend(only);
        }
    }
}

/// The version of the crucible that reads the file. The runtime, compiler types and CLI release in lockstep.
pub fn running_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("the package version is semver")
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            places: BTreeMap::from([(
                DEFAULT_PLACE.into(),
                Place {
                    project: DEFAULT_PROJECT.into(),
                    tsconfig: Place::default_tsconfig(),
                    include: Place::default_include(),
                },
            )]),
            envs: BTreeMap::new(),
            experience: Declared::default(),
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
        Self::parse_for(text, &running_version())
    }

    /// Parses the file for crucible `running`. A file that needs a newer crucible fails with that version before the
    /// strict parse, so a key or a syntax that version added never reads as an error.
    pub fn parse_for(text: &str, running: &semver::Version) -> Result<Self, ManifestError> {
        #[derive(Deserialize)]
        struct Minimum {
            crucible: Option<toml::Value>,
        }
        let lenient = || match toml::from_str(text) {
            Ok(Minimum { crucible: Some(toml::Value::String(value)) }) => Some(value),
            _ => None,
        };
        if let Some(needed) = minimum_line(text).or_else(lenient).as_deref().and_then(minimum)
            && needed > *running
        {
            return Err(ManifestError::NeedsVersion { needed, running: running.clone() });
        }
        let raw: RawManifest = toml::from_str(text).map_err(|error| ManifestError::Parse {
            line: error.span().map(|span| line_of(text, span.start)),
            message: error.message().trim_end().to_owned(),
        })?;
        if let Some(value) = &raw.crucible
            && minimum(value.get_ref()).is_none()
        {
            let line = line_of(text, value.span().start);
            return Err(ManifestError::InvalidVersion { line, value: value.get_ref().clone() });
        }
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
                    Some(BTreeMap::from([(DEFAULT_PLACE.to_owned(), id)]))
                }
                (Some(_), None) => return Err(ManifestError::PlaceShorthand { env: name }),
                (None, mapping) => mapping,
                (Some(_), Some(_)) => return Err(ManifestError::PlaceMapping { env: name }),
            };
            if let Some(mapping) = &mapping {
                if let Some(place) = mapping.keys().find(|place| !places.contains_key(*place)) {
                    return Err(ManifestError::UnknownPlace { env: name, place: place.clone() });
                }
                if let Some(place) = places.keys().find(|place| !mapping.contains_key(*place)) {
                    return Err(ManifestError::UnmappedPlace { env: name, place: place.clone() });
                }
            }
            let overlays = Overlays {
                products: located(text, env.products),
                passes: located(text, env.passes),
                badges: located(text, env.badges),
            };
            let env = Env { universe: env.universe, version_type: env.version_type, places: mapping, overlays };
            envs.insert(name, env);
        }
        let experience = Declared {
            settings: raw
                .experience
                .map(|settings| Located { line: line_of(text, settings.span().start), value: settings.into_inner() }),
            products: located(text, raw.products),
            passes: located(text, raw.passes),
            badges: located(text, raw.badges),
        };
        Ok(Self { places, envs, experience })
    }
}

/// The value of a top-level `crucible = "<version>"` before the first table header, read without parsing the file, so a
/// file whose syntax this crucible cannot parse still names the version it needs. A line inside a multi-line string is
/// text, never a key.
fn minimum_line(text: &str) -> Option<String> {
    let mut open = None;
    for line in text.strip_prefix('\u{feff}').unwrap_or(text).lines() {
        if open.is_none() {
            let line = line.trim();
            if line.starts_with('[') {
                return None;
            }
            let key = ["crucible", "\"crucible\"", "'crucible'"].into_iter().find_map(|key| line.strip_prefix(key));
            if let Some(value) = key.and_then(|rest| rest.trim_start().strip_prefix('=')).map(str::trim_start) {
                let quote = value.chars().next().filter(|c| matches!(c, '"' | '\''))?;
                let (value, rest) = value[1..].split_once(quote)?;
                let rest = rest.trim_start();
                return (rest.is_empty() || rest.starts_with('#')).then(|| value.to_owned());
            }
        }
        open = string_left_open(line, open);
    }
    None
}

/// A multi-line string, opened by `"""` or `'''`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Multiline {
    Basic,
    Literal,
}

/// The multi-line string still open after `line`, which starts inside `open`. Comments and one-line strings are
/// skipped, so a delimiter inside them opens nothing.
fn string_left_open(line: &str, mut open: Option<Multiline>) -> Option<Multiline> {
    let bytes = line.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let rest = &bytes[at..];
        match open {
            Some(Multiline::Basic) if rest[0] == b'\\' => at += 2,
            Some(Multiline::Basic) if rest.starts_with(b"\"\"\"") => (open, at) = (None, at + 3),
            Some(Multiline::Literal) if rest.starts_with(b"'''") => (open, at) = (None, at + 3),
            Some(_) => at += 1,
            None if rest[0] == b'#' => break,
            None if rest.starts_with(b"\"\"\"") => (open, at) = (Some(Multiline::Basic), at + 3),
            None if rest.starts_with(b"'''") => (open, at) = (Some(Multiline::Literal), at + 3),
            None if rest[0] == b'"' || rest[0] == b'\'' => {
                let quote = rest[0];
                at += 1;
                while at < bytes.len() && bytes[at] != quote {
                    at += if quote == b'"' && bytes[at] == b'\\' { 2 } else { 1 };
                }
                at += 1;
            }
            None => at += 1,
        }
    }
    open
}

/// "0.6" means 0.6.0. A pre-release or build suffix is not a minimum anyone writes, so it is refused.
fn minimum(value: &str) -> Option<semver::Version> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.is_empty()
        || parts.len() > 3
        || parts.iter().any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let part = |at: usize| parts.get(at).map_or(Some(0), |part| part.parse().ok());
    Some(semver::Version::new(part(0)?, part(1)?, part(2)?))
}

fn line_of(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].bytes().filter(|&b| b == b'\n').count() + 1
}

fn located<T>(text: &str, tables: BTreeMap<Spanned<String>, T>) -> Tables<T> {
    tables
        .into_iter()
        .map(|(key, value)| {
            let line = line_of(text, key.span().start);
            (key.into_inner(), Located { line, value })
        })
        .collect()
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
                version_type: VersionType::Saved,
                places: Some(BTreeMap::from([("start".into(), id(5678)), ("arena".into(), id(5679))])),
                overlays: Overlays::default(),
            }
        );
        assert_eq!(manifest.envs["production"].version_type, VersionType::Published);
    }

    #[test]
    fn cli_guide_example_parses() {
        let guide = include_str!("../../../docs/cli.md").replace("\r\n", "\n");
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
        assert_eq!(manifest.envs["production"].places, Some(BTreeMap::from([("start".into(), id(4321))])));
    }

    #[test]
    fn a_place_names_its_tsconfig_and_include_or_takes_the_root_ones() {
        let manifest = Manifest::parse(
            "[places.job]\ntsconfig = \"tsconfig.job.json\"\nproject = \"job.generated.project.json\"\n\
             include = \"places/job/include\"\n[places.start]\nproject = \"default.project.json\"\n",
        )
        .unwrap();
        assert_eq!(
            manifest.places["job"],
            Place {
                project: "job.generated.project.json".into(),
                tsconfig: "tsconfig.job.json".into(),
                include: "places/job/include".into(),
            }
        );
        assert_eq!(manifest.places["start"], Manifest::default().places["start"]);
        let start = &Manifest::default().places["start"];
        assert_eq!((start.tsconfig.to_str(), start.include.to_str()), (Some("tsconfig.json"), Some("include")));
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
            assert!(message.starts_with("crucible.toml:") && message.contains("unknown field"), "{message}");
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
            assert!(matches!(error(text), ManifestError::Parse { .. }), "{text}");
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
    fn env_sets_at_most_one_place_form() {
        let failure = error("[env.staging]\nuniverse = 1\nplace = 2\nplaces = { start = 3 }\n");
        assert_eq!(failure.to_string(), "crucible.toml: env staging sets both place and places. Set one of them");
        assert!(matches!(failure, ManifestError::PlaceMapping { env } if env == "staging"));
    }

    #[test]
    fn an_env_may_map_no_place() {
        let text = "[places.start]\nproject = \"default.project.json\"\n[places.arena]\nproject = \"arena.project.json\"\n\
                    [env.staging]\nuniverse = 1\n";
        let manifest = Manifest::parse(text).unwrap();
        assert_eq!((manifest.envs["staging"].universe, manifest.envs["staging"].places.as_ref()), (id(1), None));
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
        assert_eq!(manifest.envs["staging"].places, Some(BTreeMap::from([("start".into(), id(2))])));
    }

    #[test]
    fn every_env_maps_every_place() {
        let text = "[places.start]\nproject = \"default.project.json\"\n[places.arena]\nproject = \"arena.project.json\"\n\
                    [env.staging]\nuniverse = 1\nplaces = { start = 2 }\n";
        let failure = error(text);
        assert_eq!(failure.to_string(), "crucible.toml: env staging does not map place arena");
        assert!(matches!(failure, ManifestError::UnmappedPlace { env, place } if env == "staging" && place == "arena"));
    }

    fn version(text: &str) -> semver::Version {
        semver::Version::parse(text).unwrap()
    }

    #[test]
    fn a_newer_minimum_is_reported_before_the_keys_it_added() {
        let text = "crucible = \"0.7\"\n[storefronts.main]\ntheme = \"dark\"\n";
        let failure = Manifest::parse_for(text, &version("0.6.2")).unwrap_err();
        assert_eq!(
            failure.to_string(),
            "crucible.toml: this project needs crucible 0.7.0 or later, and this is crucible 0.6.2. Run `npx crucible upgrade 0.7.0`"
        );
        assert!(Manifest::parse_for("crucible = \"0.6\"\n", &version("0.6.0")).is_ok());
        assert!(Manifest::parse_for("crucible = \"0.6.3\"\n", &version("0.6.2")).is_err());
        let failure = Manifest::parse_for(text, &version("0.7.0")).unwrap_err().to_string();
        assert!(failure.starts_with("crucible.toml:2: unknown field `storefronts`"), "{failure}");
    }

    #[test]
    fn a_newer_minimum_is_reported_before_a_syntax_this_crucible_cannot_parse() {
        for text in [
            "crucible = \"0.7\"\n[storefront\n",
            "# needs storefronts\ncrucible = '0.7' # the first release with them\n[places.start]\nproject = \"a\"\nproject = \"b\"\n",
            "crucible = \"0.7\"\n[storefront]\ntheme = { dark = true, }\n",
        ] {
            let failure = Manifest::parse_for(text, &version("0.6.2")).unwrap_err();
            assert!(matches!(failure, ManifestError::NeedsVersion { .. }), "{text}: {failure}");
        }
        for (text, line) in [
            ("crucible = \"0.6\"\n[storefront\n", 2),
            ("[places.start]\ncrucible = \"0.7\"\n[storefront\n", 3),
            ("crucible_min = \"0.7\"\n[storefront\n", 2),
        ] {
            let failure = Manifest::parse_for(text, &version("0.6.2")).unwrap_err();
            assert!(
                matches!(failure, ManifestError::Parse { line: Some(found), .. } if found == line),
                "{text}: {failure}"
            );
        }
    }

    #[test]
    fn a_crucible_line_inside_a_multi_line_string_is_not_the_minimum() {
        let needs = |text: &str| match Manifest::parse_for(text, &version("0.6.2")) {
            Err(ManifestError::NeedsVersion { needed, .. }) => Some(needed.to_string()),
            Ok(_) => None,
            Err(other) => panic!("{text}: {other}"),
        };
        for quote in ["\"\"\"", "'''"] {
            let holds =
                |version: &str| format!("products.gem.description = {quote}\ncrucible = \"{version}\"\n{quote}\n");
            let before = holds("0.1") + "crucible = \"9.0\"\n";
            assert_eq!(needs(&before), Some("9.0.0".into()), "{before}");
            assert_eq!(needs(&(before.clone() + "[storefront\n")), Some("9.0.0".into()), "{before}");
            assert_eq!(needs(&holds("9.0")), None, "only in a string");
            assert_eq!(needs(&(holds("9.0") + "crucible = \"0.1\"\n")), None, "a lower minimum after the string");
        }
        for text in [
            "products.gem.description = \"\"\"\n\\\"\"\" still open\ncrucible = \"0.1\"\n\"\"\"\ncrucible = \"9.0\"\n[storefront\n",
            "products.gem.description = \"a \\\" '''\"\ncrucible = \"9.0\"\n[storefront\n",
            "# a comment holds \"\"\"\ncrucible = \"9.0\"\n[storefront\n",
            "products.gem.description = '''one line'''\ncrucible = \"9.0\"\n[storefront\n",
            "\u{feff}crucible = \"9.0\"\n[storefront\n",
            "\"crucible\" = \"9.0\"\n[storefront\n",
        ] {
            assert_eq!(needs(text), Some("9.0.0".into()), "{text}");
        }
    }

    #[test]
    fn a_minimum_must_be_a_plain_version() {
        for value in ["latest", "^0.6", "0.6.0-beta", "0..6", "1.2.3.4", ""] {
            let failure = error(&format!("\n\ncrucible = {value:?}\n"));
            assert!(matches!(&failure, ManifestError::InvalidVersion { line: 3, .. }), "{value}: {failure:?}");
        }
        assert!(matches!(error("crucible = 6\n"), ManifestError::Parse { line: Some(1), .. }));
    }

    #[test]
    fn experience_tables_keep_the_line_of_their_key() {
        let manifest = Manifest::parse(
            "[experience]\nvoice_chat = true\n\n[products.coin-pack]\nname = \"Coin pack\"\nprice = 49\n\n\
             [badges]\nfirst-delivery = { name = \"First delivery\" }\n\n[passes.vip]\nname = \"VIP\"\nretired = true\n\
             [env.staging]\nuniverse = 1\nplace = 2\n[env.staging.products.coin-pack]\nprice = 1\n",
        )
        .unwrap();
        let declared = &manifest.experience;
        assert_eq!(declared.settings.as_ref().map(|s| (s.line, s.value.voice_chat)), Some((1, Some(true))));
        let coin = &declared.products["coin-pack"];
        assert_eq!((coin.line, coin.value.price, coin.value.name.as_deref()), (4, Some(49), Some("Coin pack")));
        assert_eq!(declared.badges["first-delivery"].line, 9);
        assert!(declared.passes["vip"].value.retired);
        let overlay = &manifest.envs["staging"].overlays.products["coin-pack"];
        assert_eq!((overlay.line, overlay.value.price), (17, Some(1)));
    }

    #[test]
    fn experience_tables_reject_unknown_and_unoverridable_fields_by_line() {
        for (text, start) in [
            ("[products.coin-pack]\nname = \"Coin\"\npricee = 49\n", "crucible.toml:3: unknown field `pricee`"),
            ("[badges.first]\nname = \"First\"\nprice = 1\n", "crucible.toml:3: unknown field `price`"),
            (
                "[env.staging]\nuniverse = 1\nplace = 2\n[env.staging.badges.first]\nname = \"x\"\n",
                "crucible.toml:5: unknown field `name`, expected `enabled`",
            ),
            ("[experience]\nname = \"Game\"\n", "crucible.toml:2: unknown field `name`"),
        ] {
            let message = error(text).to_string();
            assert!(message.starts_with(start), "{message}");
        }
    }

    #[test]
    fn the_schema_describes_every_section() {
        let schema: serde_json::Value = serde_json::from_str(&schema()).unwrap();
        let properties = schema["properties"].as_object().unwrap();
        for key in ["crucible", "places", "env", "experience", "products", "passes", "badges"] {
            assert!(properties.contains_key(key), "{key}");
        }
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn the_schema_checks_the_rules_one_table_holds() {
        let schema: serde_json::Value = serde_json::from_str(&schema()).unwrap();
        let (properties, defs) = (&schema["properties"], &schema["$defs"]);
        let key = serde_json::json!({ "pattern": "^[a-z][a-z0-9-]*$" });
        let name = serde_json::json!({ "pattern": "^[A-Za-z0-9_-]+$" });
        for table in ["products", "passes", "badges"] {
            assert_eq!(properties[table]["propertyNames"], key, "{table}");
            assert_eq!(defs["Environment"]["properties"][table]["propertyNames"], key, "env.<name>.{table}");
        }
        for (at, names) in [("places", &properties["places"]), ("env", &properties["env"])] {
            assert_eq!(names["propertyNames"], name, "{at}");
        }
        assert_eq!(properties["places"]["minProperties"], 1);
        assert_eq!(properties["crucible"]["pattern"], "^[0-9]+(\\.[0-9]+){0,2}$");
        assert_eq!(defs["Environment"]["not"], serde_json::json!({ "required": ["place", "places"] }));
        for (def, required) in
            [("Priced", serde_json::json!(["name", "price"])), ("Badge", serde_json::json!(["name"]))]
        {
            assert_eq!(defs[def]["then"]["required"], required, "{def}");
            assert_eq!(defs[def]["then"]["properties"]["name"]["minLength"], 1, "{def}");
            assert_eq!(defs[def]["not"]["required"], serde_json::json!(["retired", "untrack"]), "{def}");
            assert_eq!(defs[def]["allOf"][0]["then"]["required"], serde_json::json!(["name"]), "{def}");
            assert_eq!(defs[def]["properties"]["moved_from"]["pattern"], key["pattern"], "{def}");
            let blank = defs[def]["properties"]["name"]["not"]["pattern"].as_str().unwrap();
            assert!(blank.starts_with("^[\\t-\\r \\u0085") && blank.ends_with("\\ufeff]$"), "{def}: {blank}");
        }
        assert_eq!(defs["Priced"]["properties"]["price"]["minimum"], 1);
        assert_eq!(defs["PricedOverlay"]["properties"]["price"]["minimum"], 1);
    }
}
