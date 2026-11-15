//! Launch paths depend on the installation, never on the build machine's sources.
use crate::Options;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub(crate) const USAGE: &str =
    "Usage: physics-demo [--game-dir PATH] [--editor|--damage-demo|--destruction-demo]

Open the original main menu, Exhibition and TEAM HQ by default.
Game files are found beside this executable or in the current directory.
--game-dir PATH / RA2_GAME_DIR selects an installed Robot Arena 2.
--config FILE overrides embedded game settings; --settings-file FILE overrides preferences.
--editor-file BOT opens a robot in the workshop.
--help, --version require no assets.
Developer checks: --workshop-check, --exhibition-check, --multiplayer-check, --editor-smoke-test,
--damage-check, --destruction-check, --smoke-test.
";

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
        let mut o = Self {
            smoke_test: false,
            config: "@embedded".into(),
            game_dir: PathBuf::new(),
            settings_file: PathBuf::new(),
            editor: false,
            editor_file: None,
            editor_smoke_test: false,
            workshop_check: false,
            exhibition_check: false,
            multiplayer_check: false,
            destruction: false,
            destruction_check: false,
            damage_demo: false,
            damage_check: false,
        };
        let mut game = None;
        let mut settings = None;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.to_str().ok_or("Invalid option name")? {
                "--game-dir" => {
                    game = Some(PathBuf::from(args.next().ok_or("--game-dir needs a path")?))
                }
                "--config" => o.config = args.next().ok_or("--config needs a path")?.into(),
                "--settings-file" => {
                    settings = Some(PathBuf::from(
                        args.next().ok_or("--settings-file needs a path")?,
                    ))
                }
                "--editor" => o.editor = true,
                "--editor-file" => {
                    o.editor = true;
                    o.editor_file =
                        Some(args.next().ok_or("--editor-file needs a BOT path")?.into());
                }
                "--menu" => {}
                "--workshop-check" => o.workshop_check = true,
                "--multiplayer-check" => o.multiplayer_check = true,
                "--exhibition-check" => o.exhibition_check = true,
                "--editor-smoke-test" => {
                    o.editor = true;
                    o.editor_smoke_test = true;
                }
                "--smoke-test" => o.smoke_test = true,
                "--damage-demo" => o.damage_demo = true,
                "--damage-check" => {
                    o.damage_demo = true;
                    o.damage_check = true;
                }
                "--destruction-demo" => o.destruction = true,
                "--destruction-check" => {
                    o.destruction = true;
                    o.destruction_check = true;
                }
                _ => return Err(format!("Unknown argument: {}", arg.to_string_lossy())),
            }
        }
        o.game_dir = locations.game_root(game, true)?;
        o.settings_file = settings.unwrap_or_else(|| o.game_dir.join(".open-arena/settings.ron"));
        Ok(o)
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
        assert!(!o.editor && !o.damage_demo);
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
    fn obsolete_synthetic_demo_switches_are_rejected() {
        let s = Sandbox::new();
        for flag in [
            "--demo",
            "--headless-check",
            "--all-bots",
            "--objects",
            "--arena",
        ] {
            assert!(parse(&[flag], &s.locations()).is_err());
        }
    }
    #[test]
    fn original_workshop_and_damage_lab_entry_points() {
        let s = Sandbox::new();
        let mut paths = s.locations();
        paths.environment_game = Some(s.game("game"));
        assert!(parse(&["--editor"], &paths).unwrap().editor);
        assert!(
            parse(&["--editor-file", "robot name.bot"], &paths)
                .unwrap()
                .editor
        );
        assert!(parse(&["--damage-demo"], &paths).unwrap().damage_demo);
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
