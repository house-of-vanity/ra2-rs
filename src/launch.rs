//! Launch paths depend on the installation, never on the build machine's sources.
use crate::{MAX_OBJECTS, Options};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub(crate) const USAGE: &str = "Usage: physics-demo [--game-dir PATH] [--menu|--demo|--editor|--damage-demo|--destruction-demo]

Without arguments, open the main menu (DEMO and TEAM HQ / robot workshop).
Find game files beside this executable or in the current directory.
Use --game-dir PATH or RA2_GAME_DIR to select another installation.
Settings: GAME/.open-arena/settings.ron; default configuration is embedded.

Options:
  --help, -h                 Show this help without loading the game
  --version                  Show the engine version
  --settings-file FILE       Override engine preferences location
  --config FILE              Override embedded scene/physics configuration
  --editor-file BOT          Open a robot in the workshop
  --arena combat|lab         Choose the arena
  --objects 0..10000          Set initial object count
  --all-bots                 Load all robots on the lab arena
  --bot-copies 1..16          Robot copies for the lab
  --bot-physics rigid|off     Lab collision mode
  --showcase                 Show component previews

Developer checks:
  --headless-check, --smoke-test, --menu-smoke-test, --editor-smoke-test,
  --workshop-check, --damage-check, --destruction-check";

struct Locations {
    cwd: PathBuf,
    executable: Option<PathBuf>,
    environment_game: Option<PathBuf>,
}
impl Locations {
    fn current() -> Result<Self, String> {
        Ok(Self {
            cwd: std::env::current_dir()
                .map_err(|e| format!("Cannot read current directory: {e}"))?,
            executable: std::env::current_exe().ok(),
            environment_game: std::env::var_os("RA2_GAME_DIR")
                .filter(|p| !p.is_empty())
                .map(PathBuf::from),
        })
    }
    fn absolute(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_owned()
        } else {
            self.cwd.join(path)
        }
    }
    fn game_root(&self, explicit: Option<PathBuf>, required: bool) -> Result<PathBuf, String> {
        if let Some(path) = explicit.or_else(|| self.environment_game.clone()) {
            let path = self.absolute(&path);
            return validate_game(&path);
        }
        let mut candidates = Vec::new();
        if let Some(parent) = self.executable.as_deref().and_then(Path::parent) {
            candidates.push(parent.to_owned());
            candidates.push(parent.join("RobotArena2"));
        }
        candidates.extend([self.cwd.clone(), self.cwd.join("RobotArena2")]);
        // Development builds in target/{debug,release} also work without a
        // path compiled into the executable. Installed binaries need no repo.
        if let Some(parent) = self.executable.as_deref().and_then(Path::parent) {
            candidates.extend(
                parent
                    .ancestors()
                    .skip(1)
                    .take(4)
                    .map(|p| p.join("RobotArena2")),
            );
        }
        candidates.extend(
            self.cwd
                .ancestors()
                .skip(1)
                .take(2)
                .map(|p| p.join("RobotArena2")),
        );
        if let Some(path) = candidates.into_iter().find(|p| is_game(p)) {
            return validate_game(&path);
        }
        if !required {
            return Ok(self.cwd.clone());
        }
        Err("Robot Arena 2 game files were not found. Put physics-demo beside Components.cfz and RA2.cfz, or run: physics-demo --game-dir /path/to/RobotArena2".into())
    }
}
fn is_game(path: &Path) -> bool {
    ["Components.cfz", "RA2.cfz"]
        .iter()
        .all(|n| path.join(n).is_file())
        && ["UI", "Components", "Arenas"]
            .iter()
            .all(|n| path.join(n).is_dir())
}
fn validate_game(path: &Path) -> Result<PathBuf, String> {
    if !is_game(path) {
        return Err(format!(
            "{} is not a Robot Arena 2 installation: expected Components.cfz, RA2.cfz and the UI, Components, Arenas directories. Use --game-dir PATH to select the installed game.",
            path.display()
        ));
    }
    std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))
}

impl Options {
    pub(crate) fn parse() -> Result<Self, String> {
        Self::parse_from(std::env::args_os().skip(1), &Locations::current()?)
    }

    fn parse_from(
        args: impl IntoIterator<Item = OsString>,
        locations: &Locations,
    ) -> Result<Self, String> {
        let mut options = Self {
            objects: 0,
            headless: false,
            smoke_test: false,
            arena: true,
            showcase: false,
            config: "@embedded".into(),
            game_dir: PathBuf::new(),
            all_bots: false,
            bot_copies: 1,
            bot_physics: true,
            menu: true,
            menu_smoke_test: false,
            settings_file: PathBuf::new(),
            editor: false,
            editor_file: None,
            editor_smoke_test: false,
            workshop_check: false,
            destruction: false,
            destruction_check: false,
            damage_demo: false,
            damage_check: false,
        };
        let mut explicit_objects = false;
        let mut explicit_arena = false;
        let mut explicit_scene = false;
        let mut explicit_menu = None;
        let mut explicit_settings = false;
        let mut explicit_game = false;
        let mut arguments = args.into_iter();
        while let Some(argument) = arguments.next() {
            match argument
                .to_str()
                .ok_or("Option names must be valid UTF-8")?
            {
                "--destruction-demo" => options.destruction = true,
                "--damage-demo" => options.damage_demo = true,
                "--damage-check" => {
                    options.damage_demo = true;
                    options.damage_check = true;
                }
                "--destruction-check" => {
                    options.destruction = true;
                    options.destruction_check = true;
                }
                "--workshop-check" => options.workshop_check = true,
                "--menu" => explicit_menu = Some(true),
                "--editor" => options.editor = true,
                "--editor-smoke-test" => {
                    options.editor = true;
                    options.editor_smoke_test = true;
                }
                "--editor-file" => {
                    options.editor = true;
                    options.editor_file = Some(
                        arguments
                            .next()
                            .ok_or("--editor-file needs a BOT path")?
                            .into(),
                    );
                }
                "--demo" => explicit_menu = Some(false),
                "--menu-smoke-test" => options.menu_smoke_test = true,
                "--settings-file" => {
                    options.settings_file = arguments
                        .next()
                        .ok_or("--settings-file needs a path")?
                        .into();
                    explicit_settings = true;
                }
                "--showcase" => options.showcase = true,
                "--all-bots" => options.all_bots = true,
                "--bot-copies" => {
                    options.all_bots = true;
                    options.bot_copies = arguments
                        .next()
                        .ok_or("--bot-copies needs a number")?
                        .to_str()
                        .ok_or("Expected a UTF-8 number")?
                        .parse()
                        .map_err(|_| "Invalid copy count")?;
                    if !(1..=16).contains(&options.bot_copies) {
                        return Err("--bot-copies expects 1..16".into());
                    }
                }
                "--bot-physics" => {
                    options.bot_physics = match arguments
                        .next()
                        .as_deref()
                        .and_then(std::ffi::OsStr::to_str)
                    {
                        Some("rigid") => true,
                        Some("off") => false,
                        _ => return Err("--bot-physics expects rigid or off".into()),
                    }
                }
                "--config" => {
                    options.config = arguments.next().ok_or("--config needs a path")?.into()
                }
                "--headless-check" => options.headless = true,
                "--smoke-test" => options.smoke_test = true,
                "--arena" => {
                    explicit_arena = true;
                    options.arena = match arguments
                        .next()
                        .as_deref()
                        .and_then(std::ffi::OsStr::to_str)
                    {
                        Some("combat") => true,
                        Some("lab") => false,
                        _ => return Err("--arena expects combat or lab".into()),
                    };
                }
                "--game-dir" => {
                    options.game_dir = arguments.next().ok_or("--game-dir needs a path")?.into();
                    explicit_game = true;
                }
                "--objects" => {
                    explicit_objects = true;
                    options.objects = arguments
                        .next()
                        .ok_or("--objects needs a number")?
                        .to_str()
                        .ok_or("Expected a UTF-8 number")?
                        .parse()
                        .map_err(|_| "Invalid object count")?;
                    if options.objects > MAX_OBJECTS {
                        return Err(format!("Maximum object count is {MAX_OBJECTS}"));
                    }
                }
                _ => return Err(format!("Unknown argument: {}", argument.to_string_lossy())),
            }
        }
        if options.all_bots {
            if explicit_arena && options.arena {
                return Err("--all-bots needs the large --arena lab".into());
            }
            options.arena = false;
        }
        if !options.arena && !explicit_objects && !options.all_bots {
            options.objects = 256;
        }
        explicit_scene |=
            options.all_bots || explicit_arena || explicit_objects || options.showcase;
        options.menu = options.menu_smoke_test
            || explicit_menu.unwrap_or(!explicit_scene && !options.smoke_test && !options.headless);
        let needs_game = options.arena
            || options.menu
            || options.editor
            || options.showcase
            || options.all_bots
            || options.damage_demo
            || options.destruction;
        options.game_dir =
            locations.game_root(explicit_game.then_some(options.game_dir), needs_game)?;
        if !explicit_settings {
            options.settings_file = options.game_dir.join(if options.menu_smoke_test {
                ".open-arena/menu-smoke-settings.ron"
            } else {
                ".open-arena/settings.ron"
            });
        }
        Ok(options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "ra2-launch-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn game(&self, name: impl AsRef<Path>) -> PathBuf {
            let p = self.0.join(name);
            for d in ["UI", "Components", "Arenas"] {
                std::fs::create_dir_all(p.join(d)).unwrap();
            }
            for f in ["Components.cfz", "RA2.cfz"] {
                std::fs::write(p.join(f), []).unwrap();
            }
            p
        }
        fn locations(&self) -> Locations {
            Locations {
                cwd: self.0.clone(),
                executable: Some(self.0.join("engine/physics-demo")),
                environment_game: None,
            }
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn parse(args: &[&str], paths: &Locations) -> Result<Options, String> {
        Options::parse_from(args.iter().map(OsString::from), paths)
    }
    #[test]
    fn next_to_game_defaults_to_menu_and_embedded_configuration() {
        let s = Sandbox::new();
        let game = s.game("Игра друга");
        let mut paths = s.locations();
        paths.executable = Some(game.join("physics-demo"));
        let o = parse(&[], &paths).unwrap();
        assert!(o.menu && o.arena && !o.editor);
        assert_eq!(o.game_dir, game);
        assert_eq!(o.config, Path::new("@embedded"));
        assert_eq!(o.settings_file, game.join(".open-arena/settings.ron"));
    }
    #[test]
    fn game_argument_beats_environment_and_executable_and_relocates_settings() {
        let s = Sandbox::new();
        let exe_game = s.game("exe");
        let env_game = s.game("env");
        let chosen = s.game("other game");
        let mut paths = s.locations();
        paths.executable = Some(exe_game.join("physics-demo"));
        paths.environment_game = Some(env_game);
        let o = parse(&["--game-dir", "other game"], &paths).unwrap();
        assert!(o.menu);
        assert_eq!(o.game_dir, chosen);
        assert_eq!(o.settings_file, chosen.join(".open-arena/settings.ron"));
        assert_eq!(o.config, Path::new("@embedded"));
        let o = parse(
            &[
                "--settings-file",
                "custom settings.ron",
                "--game-dir",
                "other game",
                "--config",
                "custom.ron",
            ],
            &paths,
        )
        .unwrap();
        assert_eq!(o.settings_file, Path::new("custom settings.ron"));
        assert_eq!(o.config, Path::new("custom.ron"));
    }
    #[test]
    fn environment_game_and_current_directory_are_supported() {
        let s = Sandbox::new();
        let game = s.game("game");
        let mut paths = s.locations();
        paths.environment_game = Some(game.clone());
        assert_eq!(parse(&[], &paths).unwrap().game_dir, game);
        paths.environment_game = None;
        paths.cwd = game.clone();
        assert_eq!(parse(&[], &paths).unwrap().game_dir, game);
    }
    #[test]
    fn development_build_finds_sibling_installation_without_build_time_paths() {
        let s = Sandbox::new();
        let game = s.game("RobotArena2");
        let mut paths = s.locations();
        paths.executable = Some(s.0.join("physics-demo/target/release/physics-demo"));
        assert_eq!(parse(&[], &paths).unwrap().game_dir, game);
    }
    #[test]
    fn missing_and_invalid_games_fail_before_engine_startup() {
        let s = Sandbox::new();
        let mut paths = s.locations();
        assert!(parse(&[], &paths).err().unwrap().contains("--game-dir"));
        paths.environment_game = Some(s.game("valid"));
        assert!(
            parse(&["--game-dir", "missing"], &paths)
                .err()
                .unwrap()
                .contains("not a Robot Arena 2 installation")
        );
        paths.environment_game = Some(s.0.join("missing"));
        assert!(parse(&[], &paths).is_err());
        assert!(parse(&["--game-dir", "valid"], &paths).is_ok());
    }
    #[test]
    fn synthetic_headless_lab_needs_no_original_files() {
        let s = Sandbox::new();
        let o = parse(
            &["--headless-check", "--arena", "lab", "--objects", "0"],
            &s.locations(),
        )
        .unwrap();
        assert!(!o.arena && !o.menu && o.headless);
        assert_eq!(o.objects, 0);
        assert_eq!(o.config, Path::new("@embedded"));
    }
    #[test]
    fn scene_switches_keep_workshop_and_demo_entry_points() {
        let s = Sandbox::new();
        let mut paths = s.locations();
        paths.environment_game = Some(s.game("game"));
        assert!(!parse(&["--demo"], &paths).unwrap().menu);
        assert!(parse(&["--editor"], &paths).unwrap().editor);
        assert!(
            parse(&["--editor-file", "robot name.bot"], &paths)
                .unwrap()
                .editor
        );
        let o = parse(
            &["--all-bots", "--bot-copies", "2", "--bot-physics", "off"],
            &paths,
        )
        .unwrap();
        assert!(o.all_bots && !o.arena && !o.menu && !o.bot_physics);
        assert_eq!(o.bot_copies, 2);
        assert!(parse(&["--objects", "10001"], &paths).is_err());
        assert!(parse(&["--game-dir"], &paths).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn linux_game_paths_do_not_require_utf8() {
        use std::os::unix::ffi::OsStringExt;
        let s = Sandbox::new();
        let game = s.game(OsString::from_vec(b"game-\xff".to_vec()));
        let o = Options::parse_from(
            [OsString::from("--game-dir"), game.clone().into_os_string()],
            &s.locations(),
        )
        .unwrap();
        assert_eq!(o.game_dir, game);
    }
}
