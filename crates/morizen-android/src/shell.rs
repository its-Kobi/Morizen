//! The Android app's own screen: it opens before anything is downloaded, stays open, fetches the
//! player's Minecraft files on request, and edits the touch layout. The game itself is a separate
//! launch that only happens once prepared resources exist.
//!
//! Only `run` is Android-specific: the rest also compiles on desktop CI, where the workspace
//! clippy and test jobs check it.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use crate::gfx2d::{Gfx, Painter, Rect};
use crate::manifest::{self, Phase, Progress};
use horizon_client::touch::{Action, Layout};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use winit::application::ApplicationHandler;
use winit::event::{Ime, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::{Window, WindowId};

const BACKGROUND: [f32; 4] = [0.043, 0.047, 0.063, 1.0];
const PANEL: [f32; 4] = [0.086, 0.094, 0.121, 1.0];
const LINE: [f32; 4] = [0.24, 0.26, 0.33, 1.0];
const TEXT: [f32; 4] = [0.88, 0.90, 0.94, 1.0];
const MUTED: [f32; 4] = [0.55, 0.58, 0.66, 1.0];
const ACCENT: [f32; 4] = [0.83, 0.63, 0.30, 1.0];
const BUTTON: [f32; 4] = [0.14, 0.16, 0.21, 1.0];
const DISABLED: [f32; 4] = [0.10, 0.11, 0.14, 1.0];

const VERSION: &str = "26.3";

#[derive(Clone, Debug)]
struct Paths {
    /// Saves, options.txt and the touch layout: the game's working directory.
    game: PathBuf,
    /// The prepared resource root: `assets/`, `data/`, the Rust edition's tables and the world
    /// generator, with `horizon-resources.json` once the set-up is complete.
    prepared: PathBuf,
    /// Where a computer-prepared folder can be dropped on Android (`adb push`), for a build or a
    /// device that cannot run the set-up itself.
    imported: Option<PathBuf>,
    /// Every folder worth looking through for a prepared `26.3` root the player already has:
    /// shared storage, Download, this app's old data folders and the previous package's.
    search: Vec<PathBuf>,
    /// The build's bundled tables (`tables.zip`), copied out of the app's assets at startup.
    tables: Option<PathBuf>,
}

/// The file that makes a folder a prepared game root (`horizon-prepare`'s output).
const MANIFEST: &str = "horizon-resources.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Home,
    Controls,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    Download,
    Sounds,
    Play,
    Search,
    Controls,
    Back,
    Name,
    SizeDown,
    SizeUp,
    AlphaDown,
    AlphaUp,
    SensDown,
    SensUp,
    Reset,
    AddKey,
    RemoveKey,
}

/// What the background thread is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JobKind {
    /// Download Mojang's files and install the Rust edition's tables.
    Setup,
    /// The 360 MB of sound effects and music, after the game itself is installed.
    Sounds,
}

enum Msg {
    /// A step the player can read while the work in progress has no byte count.
    Stage(String),
    Progress(Progress),
    /// The line to show when the thread finishes, or why it stopped.
    Done(Result<String, String>),
}

/// The work a background job does: stage lines for the player, byte progress for downloads.
type JobWork =
    Box<dyn FnOnce(&dyn Fn(&str), &(dyn Fn(Progress) + Sync)) -> Result<String, String> + Send>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Name,
    Key,
}

pub struct Shell {
    window: Option<Arc<Window>>,
    gfx: Option<Gfx>,
    paths: Paths,
    page: Page,
    status: String,
    layout: Layout,
    layout_path: PathBuf,
    name: String,
    name_path: PathBuf,
    editing: Option<Field>,
    job: Option<(JobKind, Receiver<Msg>)>,
    progress: Option<Progress>,
    hits: Vec<(Rect, Act)>,
    drag: Option<(u64, String)>,
    selected: Option<String>,
    needs_redraw: bool,
    launch: bool,
    /// Whether the resumed handler already looked for a download left by an earlier launch.
    auto_started: bool,
}

impl Shell {
    fn new(paths: Paths) -> Self {
        let layout_path = paths.game.join("touch-controls.json");
        let name_path = paths.game.join("player-name.txt");
        let name = std::fs::read_to_string(&name_path)
            .ok()
            .map(|name| name.trim().to_owned())
            .filter(|name| (3..=16).contains(&name.len()))
            .unwrap_or_else(|| "Player".to_owned());
        let mut shell = Self {
            window: None,
            gfx: None,
            paths,
            page: Page::Home,
            status: String::new(),
            layout: Layout::load(&layout_path),
            layout_path,
            name,
            name_path,
            editing: None,
            job: None,
            progress: None,
            hits: Vec::new(),
            drag: None,
            selected: None,
            needs_redraw: true,
            launch: false,
            auto_started: false,
        };
        shell.discover(false);
        shell
    }

    fn is_prepared(root: &Path) -> bool {
        root.join(MANIFEST).is_file()
    }

    /// Whether the sound effects and music are installed (the game plays silently without them).
    fn has_sounds(root: &Path) -> bool {
        root.join("assets/minecraft/sounds.json").is_file()
    }

    /// Whether this root holds Mojang's download but not the tables the Rust game needs, which
    /// happens when a previous launch stopped after downloading: finish that set-up without a
    /// tap, since the player already agreed to the download.
    fn has_raw_download(root: &Path) -> bool {
        root.join("versions")
            .join(VERSION)
            .join(format!("{VERSION}.jar"))
            .is_file()
            || root.join("assets/indexes").is_dir()
    }

    fn finish_existing_download(&mut self) {
        if self.paths.tables.is_none() || Self::is_prepared(&self.paths.prepared) {
            return;
        }
        let started = Self::has_raw_download(&self.paths.prepared)
            || self
                .paths
                .imported
                .as_deref()
                .is_some_and(Self::has_raw_download);
        if started {
            self.setup();
        }
    }

    /// Takes `folder` as the resource root and remembers it for the next launch.
    fn adopt(&mut self, folder: &Path) {
        self.paths.prepared = folder.to_path_buf();
        let record = self.paths.game.join("prepared-path.txt");
        let _ = std::fs::write(record, folder.to_string_lossy().as_bytes());
    }

    /// Looks for a prepared root the player already has. `thorough` also opens subfolders (a
    /// desktop checkout's `.local/horizon-client/26.3`, for instance); the quick pass keeps
    /// startup short.
    fn scan(&self, depth: u32, budget: usize) -> Option<PathBuf> {
        let mut seen = HashSet::new();
        let mut left = budget;
        for root in std::iter::once(&self.paths.prepared).chain(self.paths.search.iter()) {
            let mut found = None;
            Self::walk(root, depth, &mut left, &mut seen, &mut found);
            if found.is_some() {
                return found;
            }
            if left == 0 {
                break;
            }
        }
        None
    }

    fn walk(
        dir: &Path,
        depth: u32,
        left: &mut usize,
        seen: &mut HashSet<PathBuf>,
        found: &mut Option<PathBuf>,
    ) {
        if found.is_some() || *left == 0 {
            return;
        }
        if !seen.insert(dir.to_path_buf()) {
            return;
        }
        *left -= 1;
        if Self::is_prepared(dir) {
            *found = Some(dir.to_path_buf());
            return;
        }
        if depth == 0 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if found.is_some() || *left == 0 {
                return;
            }
            let path = entry.path();
            if path.is_dir() {
                Self::walk(&path, depth - 1, left, seen, found);
            }
        }
    }

    /// Finds a prepared root on this device, if there is one, and updates the status for it.
    /// The quick pass runs at startup; Search runs the thorough one.
    fn discover(&mut self, thorough: bool) -> bool {
        let remembered = std::fs::read_to_string(self.paths.game.join("prepared-path.txt"))
            .ok()
            .map(|saved| PathBuf::from(saved.trim()))
            .filter(|folder| Self::is_prepared(folder));
        let found = remembered.or_else(|| {
            if thorough {
                self.scan(4, 3000)
            } else {
                self.scan(4, 800)
            }
        });
        match found {
            Some(folder) => {
                self.adopt(&folder);
                self.status = format!(
                    "Found Minecraft {VERSION} (Rust edition) at {}. Tap PLAY.",
                    folder.display()
                );
                true
            }
            None => {
                self.status = self.missing_status();
                false
            }
        }
    }

    fn missing_status(&self) -> String {
        if self.paths.tables.is_none() {
            // A build without the bundled tables, or a device that refused the asset: the only
            // way in is a folder prepared on a computer.
            let mut text =
                format!("Minecraft {VERSION} (Rust edition) was not found on this device.");
            if let Some(imported) = &self.paths.imported {
                text.push_str(&format!(
                    " Copy a prepared {VERSION} folder (the one with horizon-resources.json) to {}",
                    imported.display()
                ));
            } else {
                text.push_str(" Copy a prepared 26.3 folder (the one with horizon-resources.json) to this app's files");
            }
            text.push_str(", or anywhere under Download, then tap SEARCH DEVICE.");
            return text;
        }
        if Self::has_raw_download(&self.paths.prepared) {
            format!(
                "Mojang's Minecraft {VERSION} files are on this device. Tap DOWNLOAD & SET UP to \
                 finish installing the Rust edition."
            )
        } else {
            format!(
                "Tap DOWNLOAD & SET UP: Morizen downloads Minecraft {VERSION} from Mojang (about \
                 135 MB), checks every file against Mojang's SHA-1, and builds the Rust edition's \
                 files on this device. Nothing of Mojang's ships in the app."
            )
        }
    }

    fn poll_job(&mut self) {
        let Some((kind, receiver)) = self.job.take() else {
            return;
        };
        let mut finished = false;
        loop {
            match receiver.try_recv() {
                Ok(Msg::Stage(text)) => {
                    self.progress = None;
                    self.status = text;
                    self.needs_redraw = true;
                }
                Ok(Msg::Progress(progress)) => {
                    self.progress = Some(progress);
                    self.needs_redraw = true;
                }
                Ok(Msg::Done(Ok(text))) => {
                    self.progress = None;
                    self.status = text;
                    finished = true;
                }
                Ok(Msg::Done(Err(error))) => {
                    self.progress = None;
                    self.status = format!(
                        "{} stopped: {error}",
                        match kind {
                            JobKind::Setup => "The set-up",
                            JobKind::Sounds => "The sound download",
                        }
                    );
                    finished = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if !finished {
                        self.progress = None;
                        self.status = "The set-up stopped unexpectedly.".to_owned();
                        finished = true;
                    }
                    break;
                }
            }
        }
        if finished {
            // A finished set-up may have made this folder playable: remember it.
            if Self::is_prepared(&self.paths.prepared) {
                let folder = self.paths.prepared.clone();
                self.adopt(&folder);
            }
            self.needs_redraw = true;
        } else {
            self.job = Some((kind, receiver));
        }
    }

    /// Starts a background job: the status line starts with `status`, then the thread reports.
    fn start_job(&mut self, kind: JobKind, status: String, work: JobWork) {
        let (sender, receiver) = channel();
        let sender = Arc::new(Mutex::new(sender));
        self.job = Some((kind, receiver));
        self.progress = None;
        self.status = status;
        self.needs_redraw = true;
        std::thread::spawn(move || {
            let stage = {
                let sender = sender.clone();
                move |line: &str| {
                    let _ = sender.lock().unwrap().send(Msg::Stage(line.to_owned()));
                }
            };
            let progress = {
                let sender = sender.clone();
                move |progress: Progress| {
                    let _ = sender.lock().unwrap().send(Msg::Progress(progress));
                }
            };
            let result = work(&stage, &progress);
            let _ = sender.lock().unwrap().send(Msg::Done(result));
        });
    }

    /// The one tap: Mojang's files, the Rust edition's tables, and the world generator built from
    /// the client JAR. Every step keeps files that already match, so tapping again resumes.
    fn setup(&mut self) {
        if self.job.is_some() {
            return;
        }
        let Some(tables_zip) = self.paths.tables.clone() else {
            self.status =
                "This build has no bundled tables: tap SEARCH DEVICE for a prepared 26.3 \
                           folder."
                    .to_owned();
            self.needs_redraw = true;
            return;
        };
        let paths = self.paths.clone();
        self.start_job(
            JobKind::Setup,
            format!("Setting up Minecraft {VERSION}…"),
            Box::new(
                move |stage: &dyn Fn(&str), progress: &(dyn Fn(Progress) + Sync)| {
                    setup_files(&paths, &tables_zip, stage, progress)
                },
            ),
        );
    }

    /// The sound effects and music, once the game itself plays.
    fn download_sounds(&mut self) {
        if self.job.is_some() {
            return;
        }
        let root = self.paths.prepared.clone();
        self.start_job(
            JobKind::Sounds,
            "Downloading sound effects and music…".to_owned(),
            Box::new(
                move |stage: &dyn Fn(&str), progress: &(dyn Fn(Progress) + Sync)| {
                    sounds_files(&root, stage, progress)
                },
            ),
        );
    }

    fn activate(&mut self, act: Act) {
        match act {
            Act::Download => self.setup(),
            Act::Sounds => self.download_sounds(),
            Act::Controls => {
                self.page = Page::Controls;
                self.status.clear();
            }
            Act::Back => {
                self.page = Page::Home;
                self.save_layout();
            }
            Act::Play => {
                if Self::is_prepared(&self.paths.prepared) {
                    self.launch = true;
                    self.status = "Starting Morizen 26.3 (Rust edition)...".to_owned();
                    self.needs_redraw = true;
                } else if self.discover(true) {
                    self.launch = true;
                    self.needs_redraw = true;
                } else {
                    self.needs_redraw = true;
                }
            }
            Act::Search => {
                self.status =
                    format!("Searching this device for a prepared Minecraft {VERSION}...");
                self.needs_redraw = true;
                self.discover(true);
            }
            Act::Name => self.begin_edit(Field::Name),
            Act::SizeDown => self.resize_layout(-1.0),
            Act::SizeUp => self.resize_layout(1.0),
            Act::AlphaDown => self.alpha_layout(-0.1),
            Act::AlphaUp => self.alpha_layout(0.1),
            Act::SensDown => self.sensitivity_layout(-0.02),
            Act::SensUp => self.sensitivity_layout(0.02),
            Act::Reset => {
                self.layout = Layout::default();
                self.selected = None;
                self.save_layout();
            }
            Act::AddKey => self.begin_edit(Field::Key),
            Act::RemoveKey => {
                let id = self
                    .selected
                    .clone()
                    .filter(|id| id.starts_with("custom-"))
                    .or_else(|| {
                        self.layout
                            .buttons
                            .iter()
                            .rev()
                            .find(|button| button.id.starts_with("custom-"))
                            .map(|button| button.id.clone())
                    });
                if let Some(id) = id {
                    self.layout.remove_custom(&id);
                    self.selected = None;
                    self.save_layout();
                }
            }
        }
        self.needs_redraw = true;
    }

    fn begin_edit(&mut self, field: Field) {
        self.editing = Some(field);
        if let Some(window) = &self.window {
            window.set_ime_allowed(true);
        }
    }

    fn commit_edit(&mut self, text: &str) {
        let Some(field) = self.editing.take() else {
            return;
        };
        if let Some(window) = &self.window {
            window.set_ime_allowed(false);
        }
        match field {
            Field::Name => {
                let name: String = text
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .take(16)
                    .collect();
                if (3..=16).contains(&name.len()) {
                    self.name = name;
                    let _ = std::fs::write(&self.name_path, &self.name);
                    self.status = format!("Player name set to {}.", self.name);
                } else {
                    self.status = "A name is 3 to 16 letters, digits or underscores.".to_owned();
                }
            }
            Field::Key => {
                let key = text.trim().to_ascii_lowercase();
                let action = if key.starts_with("key.mouse.") {
                    Action::Mouse(key)
                } else if key.starts_with("key.") {
                    Action::Key(key)
                } else {
                    Action::Key(format!("key.keyboard.{key}"))
                };
                if !action.valid() {
                    self.status = format!("{text:?} is not a Minecraft key name.");
                } else {
                    let label = text.trim().to_ascii_uppercase();
                    let added = self
                        .layout
                        .add_custom(&label, action)
                        .map(|button| button.id.clone());
                    match added {
                        Ok(id) => {
                            self.selected = Some(id);
                            self.save_layout();
                            self.status =
                                format!("Added a {label} button; drag it where you want.");
                        }
                        Err(error) => self.status = error,
                    }
                }
            }
        }
        self.needs_redraw = true;
    }

    fn save_layout(&mut self) {
        if let Err(error) = self.layout.save(&self.layout_path) {
            self.status = format!("Could not save the touch layout: {error}");
        }
    }

    fn alpha_layout(&mut self, delta: f32) {
        self.layout.opacity = (self.layout.opacity + delta).clamp(0.15, 1.0);
        self.save_layout();
    }

    fn sensitivity_layout(&mut self, delta: f32) {
        self.layout.look_sensitivity = (self.layout.look_sensitivity + delta).clamp(0.03, 1.0);
        self.save_layout();
    }

    /// `SIZE -`/`SIZE +`: the picked button's own size, else every button's at once.
    fn resize_layout(&mut self, step: f32) {
        let index = self.selected.as_deref().and_then(|id| {
            self.layout
                .buttons
                .iter()
                .position(|button| button.id == id)
        });
        if let Some(index) = index {
            let size = (self.layout.buttons[index].size + step * 0.01).clamp(0.04, 0.25);
            let id = self.layout.buttons[index].id.clone();
            self.layout.resize_button(&id, size);
        } else {
            self.layout.scale = (self.layout.scale + step * 0.1).clamp(0.5, 2.0);
        }
        self.save_layout();
    }

    #[allow(clippy::too_many_arguments)]
    fn button(
        &mut self,
        painter: &mut Painter,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        label: &str,
        act: Act,
        enabled: bool,
    ) {
        let area = Rect::new(x, y, w, h);
        painter.frame(
            area,
            if enabled { BUTTON } else { DISABLED },
            if enabled { LINE } else { PANEL },
        );
        let size = Painter::fitted(label, 3.0 * painter.ui, w - 8.0 * painter.ui);
        let color = if enabled { TEXT } else { MUTED };
        painter.text_centered(x + w / 2.0, y + (h - size) / 2.0, label, size, color);
        if enabled {
            self.hits.push((area, act));
        }
    }

    fn paint_home(&mut self, painter: &mut Painter) {
        let (w, h, ui) = (painter.width, painter.height, painter.ui);
        painter.text_centered(w / 2.0, h * 0.04, "MORIZEN", 10.0 * ui, TEXT);
        painter.text_centered(
            w / 2.0,
            h * 0.04 + 12.0 * ui,
            "MINECRAFT 26.3 RUST EDITION",
            2.4 * ui,
            ACCENT,
        );
        let top = (h * 0.19).max(h * 0.04 + 21.0 * ui);
        let panel = Rect::new(w * 0.05, top, w * 0.90, h * 0.30);
        painter.frame(panel, PANEL, LINE);
        let size = 2.4 * ui;
        let mut y = panel.y + 4.0 * ui;
        for line in Painter::wrap(&self.status, size, panel.w - 8.0 * ui)
            .into_iter()
            .take(6)
        {
            painter.text(panel.x + 4.0 * ui, y, &line, size, TEXT);
            y += size * 1.4;
        }
        if let Some(progress) = &self.progress {
            y += 2.0 * ui;
            let bar = Rect::new(panel.x + 4.0 * ui, y, panel.w - 8.0 * ui, 3.0 * ui);
            painter.frame(bar, BACKGROUND, LINE);
            let fraction = if progress.total == 0 {
                0.0
            } else {
                (progress.done as f32 / progress.total as f32).clamp(0.0, 1.0)
            };
            painter.rect(
                bar.x + 1.0,
                bar.y + 1.0,
                (bar.w - 2.0) * fraction,
                bar.h - 2.0,
                ACCENT,
            );
            y += 5.0 * ui;
            let label = format!(
                "{} {}/{}",
                match progress.phase {
                    Phase::Manifest => "MANIFEST",
                    Phase::Version => "VERSION",
                    Phase::Client => "CLIENT",
                    Phase::AssetIndex => "INDEX",
                    Phase::Assets => "ASSETS",
                },
                progress.done,
                progress.total
            );
            painter.text(panel.x + 4.0 * ui, y, &label, 2.2 * ui, MUTED);
        }

        let bw = w * 0.66;
        let bh = 9.0 * ui;
        let bx = (w - bw) / 2.0;
        let mut y = h * 0.55;
        let gap = 2.5 * ui;
        let working = self.job.is_some();
        let prepared = Self::is_prepared(&self.paths.prepared);
        let (label, act, enabled) = if !prepared {
            (
                "DOWNLOAD & SET UP",
                Act::Download,
                self.paths.tables.is_some(),
            )
        } else if !Self::has_sounds(&self.paths.prepared) {
            ("DOWNLOAD SOUNDS", Act::Sounds, true)
        } else {
            ("SET-UP COMPLETE", Act::Download, false)
        };
        self.button(painter, bx, y, bw, bh, label, act, enabled && !working);
        y += bh + gap;
        self.button(
            painter,
            bx,
            y,
            bw,
            bh,
            "TOUCH CONTROLS",
            Act::Controls,
            !working,
        );
        y += bh + gap;
        if prepared {
            self.button(painter, bx, y, bw, bh, "PLAY", Act::Play, true);
        } else {
            self.button(
                painter,
                bx,
                y,
                bw,
                bh,
                "SEARCH DEVICE FOR 26.3",
                Act::Search,
                !working,
            );
        }
        y += bh + gap;
        let name = format!("PLAYER: {}", self.name.to_ascii_uppercase());
        self.button(painter, bx, y, bw, 7.0 * ui, &name, Act::Name, !working);
    }

    fn paint_controls(&mut self, painter: &mut Painter) {
        let (w, h, ui) = (painter.width, painter.height, painter.ui);
        let rows: [[(&str, Act); 5]; 2] = [
            [
                ("BACK", Act::Back),
                ("SIZE -", Act::SizeDown),
                ("SIZE +", Act::SizeUp),
                ("ALPHA -", Act::AlphaDown),
                ("ALPHA +", Act::AlphaUp),
            ],
            [
                ("LOOK -", Act::SensDown),
                ("LOOK +", Act::SensUp),
                ("ADD KEY", Act::AddKey),
                ("REMOVE", Act::RemoveKey),
                ("RESET", Act::Reset),
            ],
        ];
        let bh = 8.0 * ui;
        let gap = 1.2 * ui;
        let bar = Rect::new(0.0, 0.0, w, 2.0 * bh + 3.0 * gap);
        painter.rect(bar.x, bar.y, bar.w, bar.h, PANEL);
        for (row, entries) in rows.into_iter().enumerate() {
            let n = entries.len() as f32;
            let bw = (w - 2.0 * ui - gap * (n - 1.0)) / n;
            let mut x = ui;
            let y = gap + row as f32 * (bh + gap);
            for (label, act) in entries {
                self.button(painter, x, y, bw, bh, label, act, true);
                x += bw + gap;
            }
        }

        let selected = self
            .selected
            .as_deref()
            .and_then(|id| self.layout.buttons.iter().find(|button| button.id == id));
        let hint = match selected {
            Some(button) => format!(
                "{} SELECTED  SIZE {:.2}  DRAG TO MOVE",
                button.label, button.size
            ),
            None => format!(
                "DRAG A BUTTON TO MOVE IT.  WHOLE LAYOUT {:.2}  ALPHA {:.2}  LOOK {:.2}",
                self.layout.scale, self.layout.opacity, self.layout.look_sensitivity
            ),
        };
        painter.text(ui, bar.h + gap, &hint, 2.2 * ui, MUTED);

        let screen = [w, h];
        let bounds = self.layout.buttons.iter().map(|button| {
            (
                button.id.clone(),
                button.label.clone(),
                button.bounds(screen, self.layout.scale),
            )
        });
        for (id, label, [bx, by, bw, bh]) in bounds.collect::<Vec<_>>() {
            let selected = self.selected.as_deref() == Some(id.as_str());
            painter.frame(
                Rect::new(bx, by, bw, bh),
                if selected {
                    [0.33, 0.27, 0.15, 0.85]
                } else {
                    [0.08, 0.09, 0.12, 0.75]
                },
                if selected { ACCENT } else { LINE },
            );
            let size = Painter::fitted(&label, 2.4 * ui, bw - 4.0);
            painter.text_centered(bx + bw / 2.0, by + (bh - size) / 2.0, &label, size, TEXT);
        }
    }

    fn draw(&mut self) {
        let (width, height) = match self.gfx.as_ref() {
            Some(gfx) => (gfx.width(), gfx.height()),
            None => return,
        };
        let mut painter = Painter::new(width as f32, height as f32);
        painter.clear(BACKGROUND);
        self.hits.clear();
        match self.page {
            Page::Home => self.paint_home(&mut painter),
            Page::Controls => self.paint_controls(&mut painter),
        }
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.draw(&painter);
        }
    }

    fn touch(&mut self, id: u64, phase: TouchPhase, at: [f32; 2]) {
        match phase {
            TouchPhase::Started => {
                let hit = self
                    .hits
                    .iter()
                    .rev()
                    .find(|(rect, _)| rect.contains(at))
                    .map(|(_, act)| *act);
                if let Some(act) = hit {
                    self.activate(act);
                    return;
                }
                let layout_hit = if self.page == Page::Controls {
                    self.layout.hit(at, self.screen()).map(|b| b.id.clone())
                } else {
                    None
                };
                if let Some(button) = layout_hit {
                    self.selected = Some(button.clone());
                    self.drag = Some((id, button));
                    self.needs_redraw = true;
                }
            }
            TouchPhase::Moved => {
                if let Some((finger, button)) = self.drag.clone()
                    && finger == id
                {
                    let screen = self.screen();
                    let x = at[0] / screen[0];
                    let y = at[1] / screen[1];
                    if self.layout.move_button(&button, x, y) {
                        self.needs_redraw = true;
                    }
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                if self.drag.take().is_some() {
                    self.save_layout();
                }
            }
        }
    }

    fn screen(&self) -> [f32; 2] {
        self.gfx
            .as_ref()
            .map(|gfx| [gfx.width() as f32, gfx.height() as f32])
            .unwrap_or([1.0, 1.0])
    }
}

/// The one tap, on a background thread: Mojang's files (each checked against the SHA-1 Mojang
/// publishes), then the Rust edition's tables, then the world generator built from the client
/// JAR. Every step keeps files that already match, so a second tap resumes rather than repeats.
fn setup_files(
    paths: &Paths,
    tables_zip: &Path,
    stage: &dyn Fn(&str),
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<String, String> {
    let root = &paths.prepared;
    stage(&format!("Downloading Minecraft {VERSION} from Mojang…"));
    let downloaded = manifest::download(VERSION, root, progress)?;
    stage("Unpacking Mojang's client…");
    let (files, assets) = manifest::extract_raw(&downloaded, root)?;
    stage("Installing the Rust edition's tables and world generator…");
    let tables = crate::tables::install(tables_zip, root, &downloaded.client_jar, &|line| {
        log::info!("Morizen: {line}");
    })?;
    Ok(format!(
        "Minecraft {VERSION} is ready ({files} client files, {assets} assets, {tables} Rust edition \
         tables). Tap PLAY.{}",
        if Shell::has_sounds(root) {
            ""
        } else {
            " Sound effects are not installed: tap DOWNLOAD SOUNDS."
        }
    ))
}

/// The sound effects and music (about 360 MB), installed into the prepared root's `assets/`.
fn sounds_files(
    root: &Path,
    stage: &dyn Fn(&str),
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<String, String> {
    stage("Downloading sound effects and music…");
    let files = manifest::download_sounds(VERSION, root, progress)?;
    stage("Installing the sounds…");
    let index = manifest::asset_index_path(root, VERSION)?;
    let installed = manifest::materialise_assets(root, &index, root)?;
    Ok(format!(
        "Sound effects and music are installed ({files} files, {installed} assets ready). Tap PLAY."
    ))
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("Morizen")
                .with_transparent(false),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                log::error!("could not open Morizen's window: {error}");
                event_loop.exit();
                return;
            }
        };
        match Gfx::new(window.clone()) {
            Ok(gfx) => self.gfx = Some(gfx),
            Err(error) => {
                log::error!("could not start Morizen's drawing: {error}");
                event_loop.exit();
                return;
            }
        }
        if let Some(size) = self.gfx.as_ref() {
            log::info!("Morizen shell {}x{}", size.width(), size.height());
        }
        self.window = Some(window);
        self.needs_redraw = true;
        if !self.auto_started {
            self.auto_started = true;
            self.finish_existing_download();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::Resized(size) => {
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.resize(size.width, size.height);
                }
                self.needs_redraw = true;
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::Touch(touch) => {
                let at = [touch.location.x as f32, touch.location.y as f32];
                self.touch(touch.id, touch.phase, at);
            }
            WindowEvent::Ime(Ime::Commit(text)) => self.commit_edit(&text),
            WindowEvent::Ime(Ime::Enabled) => {}
            WindowEvent::Ime(Ime::Disabled) => self.editing = None,
            WindowEvent::CloseRequested => {
                self.save_layout();
                event_loop.exit();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.poll_job();
        event_loop.set_control_flow(if self.job.is_some() {
            ControlFlow::Poll
        } else {
            ControlFlow::Wait
        });
        if self.needs_redraw {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            self.needs_redraw = false;
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.save_layout();
    }
}

/// The process's only winit handler: Morizen's own screen until the player taps Play, then the
/// game. Both live on the one event loop winit allows per process; the shell's window and surface
/// are dropped before the game creates its own on the same native window.
#[allow(clippy::large_enum_variant)]
pub enum Stage {
    Shell(Shell),
    Game(horizon_client::game::AndroidGame),
}

impl ApplicationHandler for Stage {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        match self {
            Stage::Shell(shell) => shell.resumed(event_loop),
            Stage::Game(game) => game.resumed(event_loop),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match self {
            Stage::Shell(shell) => shell.window_event(event_loop, id, event),
            Stage::Game(game) => game.window_event(event_loop, id, event),
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Stage::Game(game) = self {
            game.about_to_wait(event_loop);
            return;
        }
        let launch = match self {
            Stage::Shell(shell) => {
                shell.about_to_wait(event_loop);
                shell.launch
            }
            Stage::Game(_) => false,
        };
        if !launch {
            return;
        }
        let request = match self {
            Stage::Shell(shell) => Some((shell.paths.prepared.clone(), shell.name.clone())),
            Stage::Game(_) => None,
        };
        let Some((resources, name)) = request else {
            return;
        };
        // The game's own launch checks run here; the shell stays in charge if they refuse.
        match horizon_client::game::AndroidGame::start(resources, name) {
            Ok(game) => *self = Stage::Game(game),
            Err(error) => {
                if let Stage::Shell(shell) = self {
                    shell.launch = false;
                    shell.status = format!("Could not start the game: {error}");
                    shell.needs_redraw = true;
                }
                return;
            }
        }
        if let Stage::Game(game) = self {
            game.resumed(event_loop);
            game.about_to_wait(event_loop);
        }
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        match self {
            Stage::Shell(shell) => shell.exiting(event_loop),
            Stage::Game(game) => game.exiting(event_loop),
        }
    }
}

/// Reads the build's `tables.zip` out of the APK's assets into the app's data folder, once.
#[cfg(target_os = "android")]
fn bundled_tables(
    app: &winit::platform::android::activity::AndroidApp,
    data: &Path,
) -> Option<PathBuf> {
    use std::ffi::CString;
    // The build's workflow prepared tables with `horizon-prepare` and packed them beside the
    // libraries; they are read out of the APK once into the app's own data folder, where `zip`
    // and the game can seek in them.
    let name = CString::new("tables.zip").ok()?;
    let manager = app.asset_manager();
    let mut asset = manager.open(&name)?;
    let path = data.join("tables.zip");
    let length = asset.length() as u64;
    if std::fs::metadata(&path).map(|meta| meta.len()) == Ok(length) {
        return Some(path);
    }
    match std::fs::File::create(&path) {
        Ok(mut file) => match std::io::copy(&mut asset, &mut file) {
            Ok(_) => Some(path),
            Err(error) => {
                log::warn!("Morizen: could not read the bundled tables: {error}");
                None
            }
        },
        Err(error) => {
            log::warn!("Morizen: could not write {}: {error}", path.display());
            None
        }
    }
}

/// Opens Morizen's screen and runs the app until its window closes. The game, once started, takes
/// over the same event loop.
#[cfg(target_os = "android")]
pub fn run(app: winit::platform::android::activity::AndroidApp) -> Result<(), String> {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    let data = app
        .internal_data_path()
        .ok_or("Android did not provide an app data folder.")?;
    let downloaded = data.join("prepared").join(VERSION);
    // A folder prepared on a computer wins when it is complete; without one the app prepares its
    // own root here.
    let external = app.external_data_path();
    let imported = external
        .as_ref()
        .map(|dir| dir.join("prepared").join(VERSION));
    let prepared = match &imported {
        Some(folder) if folder.join(MANIFEST).is_file() => folder.clone(),
        _ => downloaded,
    };
    // Shared storage and app data folders a downloaded or copied 26.3 can plausibly live in. The
    // previous package name is here too: an older build's files may still be on the device.
    let mut search = Vec::new();
    if let Some(dir) = &external {
        search.push(dir.join("prepared"));
        search.push(dir.clone());
    }
    for base in ["/sdcard", "/storage/emulated/0"] {
        let base = PathBuf::from(base);
        for name in [
            "Download",
            "Documents",
            "Morizen",
            "morizen",
            "horizon-client",
            "games",
        ] {
            search.push(base.join(name));
        }
        for package in ["com.morizen.app", "dev.morizen.mobile"] {
            search.push(
                base.join("Android/data")
                    .join(package)
                    .join("files/prepared"),
            );
            search.push(base.join("Android/data").join(package).join("files"));
        }
    }
    for package in ["com.morizen.app", "dev.morizen.mobile"] {
        search.push(
            PathBuf::from("/data/user/0")
                .join(package)
                .join("files/prepared"),
        );
    }
    // Removable cards mount under /storage/<id>; look for the usual folders beside that mount.
    if let Ok(cards) = std::fs::read_dir("/storage") {
        for card in cards.flatten() {
            let base = card.path();
            for name in ["Download", "Documents", "Morizen", "horizon-client"] {
                search.push(base.join(name));
            }
        }
    }
    let paths = Paths {
        game: data.join("game"),
        prepared,
        imported,
        search,
        tables: bundled_tables(&app, &data),
    };
    for folder in [
        Some(&paths.game),
        Some(&paths.prepared),
        paths.imported.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    }
    std::env::set_current_dir(&paths.game).map_err(|e| format!("{}: {e}", paths.game.display()))?;
    let mut stage = Stage::Shell(Shell::new(paths));
    let event_loop = winit::event_loop::EventLoop::builder()
        .with_android_app(app)
        .build()
        .map_err(|e| e.to_string())?;
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop
        .run_app(&mut stage)
        .map_err(|e| format!("Morizen stopped: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A prepared folder copied in a desktop checkout's layout, under Download, is adopted on
    /// startup even though it is a few levels deep; the game then launches from that root.
    #[test]
    fn finds_a_prepared_folder_in_a_nested_search_root() {
        let dir = std::env::temp_dir().join(format!(
            "morizen-shell-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let root = dir
            .join("Download")
            .join("Morizen")
            .join(".local")
            .join("horizon-client")
            .join(VERSION);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(MANIFEST), b"{}").unwrap();
        let paths = Paths {
            game: dir.join("game"),
            prepared: dir.join("nothing-here"),
            imported: None,
            search: vec![dir.join("Download")],
            tables: None,
        };
        let shell = Shell::new(paths);
        assert_eq!(shell.paths.prepared, root);
        assert!(shell.status.contains("Tap PLAY"), "{}", shell.status);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Raw Mojang files are reported as downloaded but unprepared, not as absent.
    #[test]
    fn reports_a_raw_download_as_unprepared() {
        let dir = std::env::temp_dir().join(format!("morizen-shell-raw-{}", std::process::id()));
        let prepared = dir.join("prepared").join(VERSION);
        std::fs::create_dir_all(prepared.join("assets").join("indexes")).unwrap();
        let paths = Paths {
            game: dir.join("game"),
            prepared: prepared.clone(),
            imported: Some(dir.join("external").join(VERSION)),
            search: Vec::new(),
            tables: Some(dir.join("tables.zip")),
        };
        let shell = Shell::new(paths);
        assert_eq!(shell.paths.prepared, prepared);
        assert!(
            shell.status.contains("are on this device"),
            "{}",
            shell.status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
