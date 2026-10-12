//! What the view shows, derived from the core's state.

use std::time::Duration;

use prism_gpu::fan_curve::FanCurveSupport;
use prism_gpu::{ComputeApi, GpuDevice, GpuPerformanceDevice};
use prism_protocol::{ProfileSetting, ProfileSettingKey};

use crate::activity::ActivityAction;
use crate::cooling::{
    FanArrangement, FanMode, Fans, cooling_read, fixed_speed_of, fixed_speed_range, mode_of,
    setting_of, speed_now,
};
use crate::core::{
    Core, Discovery, Gpu, Latest, PrismdConnection, Section, Selection, TelemetrySource,
    TuningControl,
};
use crate::daemon::Fact;
use crate::drafts::{
    DialogState, ReviewChange, Staged, control_of, not_applied, setting_name, setting_readout,
    staged_text,
};
use crate::format::{Readout, UNKNOWN, pending, short_bus_id, short_gpu_name, signed};
use crate::palette::Command;
use crate::profiles::correction_change;
use crate::telemetry::{Chart, ChartPoint, LimitKind, LimitLine, Metric, Readings, TelemetryRange};

/// The readings the sidebar shows for each GPU.
const VITALS: [Metric; 4] = [
    Metric::Temperature,
    Metric::Utilization,
    Metric::PowerDraw,
    Metric::FanSpeed,
];

const WAITING: &str = "Waiting for the first reading";
const NOT_REPORTED: &str = "Not reported by this GPU";

/// The sidebar: a block per GPU, then the section rows.
#[derive(Debug, Clone, PartialEq)]
pub struct Sidebar {
    pub gpus: Vec<SidebarGpu>,
    /// Why there are no GPU blocks, once the inventory has been read and has none.
    pub no_gpus: Option<ExplainedState>,
    pub section: Option<Section>,
}

/// A GPU's block in the sidebar, with its live vitals.
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarGpu {
    pub id: String,
    /// The GPU's position in the inventory, such as "GPU 0".
    pub index: String,
    pub name: String,
    /// What tells this GPU apart from another of the same model: its bus id.
    pub identity: String,
    /// Index, name and identity together, such as "GPU 0 · RTX 5090 · 01:00.0".
    pub label: String,
    pub temperature: Readout,
    pub utilization: Readout,
    pub power_draw: Readout,
    pub fan_speed: Readout,
    /// Why some of the vitals are unknown, when they are.
    pub unknown_reason: Option<String>,
    /// What's in a failure pause on the GPU, such as "Fan curve paused" after a cooling fault or
    /// "Automatic switching failed", shown together as PAUSED.
    pub pauses: Vec<String>,
    pub selected: bool,
}

impl SidebarGpu {
    /// Whether anything is in a failure pause on the GPU, which the block shows as PAUSED.
    pub fn paused(&self) -> bool {
        !self.pauses.is_empty()
    }

    /// The vitals as a screen reader says them, with why any are unknown.
    pub fn vitals(&self) -> String {
        let vitals = format!(
            "temperature {}, utilization {}, power draw {}, fan speed {}",
            self.temperature.spoken(),
            self.utilization.spoken(),
            self.power_draw.spoken(),
            self.fan_speed.spoken()
        );
        [Some(vitals), self.unknown_reason.clone()]
            .into_iter()
            .flatten()
            .chain(self.pauses.iter().cloned())
            .collect::<Vec<_>>()
            .join(". ")
    }
}

/// What fills the content area.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    /// Prism hasn't read the GPU inventory yet.
    Searching,
    /// The inventory has no GPUs, for the reason given.
    NoGpus(ExplainedState),
    Gpu(Box<GpuWorkspace>),
    Section(Section),
}

/// The selected GPU's workspace.
#[derive(Debug, Clone, PartialEq)]
pub struct GpuWorkspace {
    pub id: String,
    pub title: String,
    /// Index, bus id and architecture, such as "GPU 0 · 01:00.0 · Blackwell".
    pub subtitle: String,
    /// The header's profile buttons.
    pub profiles: ProfileSwitcher,
    /// The fixed strip under the header with the GPU's unapplied tuning changes.
    pub changes: ChangeStrip,
    /// Why there's no tuning access, for the inline "Tuning needs prismd" line; none with it.
    pub no_tuning_access: Option<String>,
    /// The GPU's automatic switching status, while there is a session.
    pub switching: Option<SwitchingStatus>,
    /// The range the range picker has picked, which every track shows.
    pub range: TelemetryRange,
    /// One track per metric, top to bottom in [`TrackKind::ALL`]'s order.
    pub tracks: Vec<Track>,
    pub hardware: HardwareDetails,
    /// Changes each time the workspace should scroll its Fans track into view, such as when a
    /// notification about the GPU's failure pause was clicked.
    pub reveal_fans: u64,
}

impl GpuWorkspace {
    /// The track of this kind.
    pub fn track(&self, kind: TrackKind) -> &Track {
        self.tracks
            .iter()
            .find(|track| track.kind == kind)
            .expect("the workspace has every track")
    }

    /// The stepper that tunes `setting`, in whichever track's details it is.
    pub fn stepper(&self, setting: ProfileSettingKey) -> Option<&Stepper> {
        self.stepper_of(Tunable::Setting(setting))
    }

    /// The stepper of every fan's fixed speed, while the Fans track shows one.
    pub fn fixed_fan_speed(&self) -> Option<&Stepper> {
        self.stepper_of(Tunable::FixedFanSpeed)
    }

    fn stepper_of(&self, tunable: Tunable) -> Option<&Stepper> {
        self.tracks
            .iter()
            .flat_map(|track| &track.details)
            .filter_map(|detail| detail.stepper.as_ref())
            .find(|stepper| stepper.setting == tunable)
    }

    /// The Fans track's fan control mode.
    pub fn fan_mode(&self) -> Option<&FanModeControl> {
        self.tracks.iter().find_map(|track| track.fan_mode.as_ref())
    }

    /// The Fans track's fan curve editor, while it shows a curve.
    pub fn fan_curve(&self) -> Option<&FanCurveEditor> {
        self.tracks
            .iter()
            .find_map(|track| track.fan_curve.as_ref())
    }
}

/// The GPU's tuning profiles as the workspace header's buttons, one click applying one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileSwitcher {
    /// The caller's profiles of the GPU, in the order prismd lists them.
    pub choices: Vec<ProfileChoice>,
    /// The GPU's current profile's name, whoever owns it, if it has one.
    pub current: Option<String>,
    /// Why no profile can be applied, when there's no tuning access.
    pub read_only: Option<String>,
    /// Whether there are changes not saved to profile: the values in effect or the explicit
    /// defaults applied differ from the current profile.
    pub not_saved_to_profile: bool,
}

/// The workspace header's open `⋯` menu of profile actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileMenu {
    pub entries: Vec<MenuEntry>,
}

/// An action in a menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuEntry {
    /// Such as "Save to current profile".
    pub title: String,
    /// Why it can't run now, when it can't. It stays listed, dimmed.
    pub unavailable: Option<String>,
}

/// One of the GPU's profiles in the workspace header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileChoice {
    /// The profile's id, which applying it sends.
    pub id: String,
    /// The profile's name, as its button shows it.
    pub name: String,
    /// Whether it's the GPU's current profile, which shows as selected.
    pub current: bool,
    /// Whether it's the GPU's startup profile, which shows a dot.
    pub startup: bool,
    /// Why it can't be applied now, when it can't.
    pub unavailable: Option<String>,
}

/// The fixed strip under the workspace's header: "No unapplied changes", or how many there are,
/// with Discard and Apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeStrip {
    /// Such as "No unapplied changes" or "2 unapplied changes".
    pub summary: String,
    /// Whether the GPU has unapplied tuning changes.
    pub pending: bool,
    /// What the last Apply or profile application put into effect and what it left unapplied, and
    /// why, until the next edit, such as "Power limit applied · Graphics clock offset not applied:
    /// …" or "Quiet applied".
    pub outcome: Option<String>,
    /// Why Apply can't run now, while there are changes it can't apply.
    pub apply_unavailable: Option<String>,
    /// Why Discard can't run now, while there are changes it can't discard.
    pub discard_unavailable: Option<String>,
    /// What changed under the changes that the user must review before they replace it, such as
    /// cooling changed in another session; Apply opens the review.
    pub review: Option<String>,
}

/// A full-width track of the workspace, one per metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Temperature,
    Utilization,
    /// Power draw, with the configured and enforced power limits.
    Power,
    /// The measured graphics and memory clocks, with the clock offsets.
    Clocks,
    /// Fan speed, with each fan's own speed.
    Fans,
}

impl TrackKind {
    /// Every track, top to bottom.
    pub const ALL: [Self; 5] = [
        Self::Temperature,
        Self::Utilization,
        Self::Power,
        Self::Clocks,
        Self::Fans,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Temperature => "Temperature",
            Self::Utilization => "Utilization",
            Self::Power => "Power",
            Self::Clocks => "Clocks",
            Self::Fans => "Fans",
        }
    }

    /// The metric the track's readout and statistics show.
    pub fn metric(self) -> Metric {
        match self {
            Self::Temperature => Metric::Temperature,
            Self::Utilization => Metric::Utilization,
            Self::Power => Metric::PowerDraw,
            Self::Clocks => Metric::GraphicsClock,
            Self::Fans => Metric::FanSpeed,
        }
    }
}

/// A track: its current reading and details on the left, its charts in the middle, and the
/// max/avg/min of its metric over the range on the right.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub kind: TrackKind,
    pub readout: Readout,
    /// Why the reading is unknown, when it is.
    pub unknown_reason: Option<String>,
    /// Further current values under the readout, such as the power limits.
    pub details: Vec<Detail>,
    /// The range the charts and statistics cover, such as "5 m".
    pub range: &'static str,
    pub max: Readout,
    pub avg: Readout,
    pub min: Readout,
    /// Side by side, left to right.
    pub charts: Vec<TrackChart>,
    /// The fan control mode, in the Fans track.
    pub fan_mode: Option<FanModeControl>,
    /// The fan curve editor, in the Fans track while the fans follow a curve or one is pending.
    pub fan_curve: Option<FanCurveEditor>,
}

/// The fan control mode of the fans the GPU's cooling controls set, every fan or the one picked:
/// the GPU's default automatic cooling, a fixed speed or a fan curve, with the mode picked and any
/// pending one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FanModeControl {
    /// Every mode, left to right.
    pub modes: Vec<FanModeChoice>,
    /// The mode staged for those fans, as shown next to the control, such as "→ fixed", while
    /// it's pending.
    pub pending: Option<String>,
    /// That the last Apply didn't put the pending cooling into effect, and why if prismd said,
    /// while it's still pending.
    pub failed: Option<String>,
    /// Why it can't be changed, when it can't.
    pub read_only: Option<String>,
    /// What the user should know about the GPU's cooling, such as why fan curves are unavailable.
    pub note: Option<String>,
    /// Why Prism stopped following the GPU's fan curve, with Retry or Resume, while it has.
    pub problem: Option<CoolingProblem>,
    /// Which fans the controls set, while the GPU has more than one fan.
    pub arrangement: Option<FanArrangementControl>,
    /// Whether the remembered fan curve can be recalled into the draft now.
    pub can_recall: bool,
}

/// Why Prism stopped following a GPU's fan curve: a cooling fault, after which its fans fell back
/// to automatic cooling, or another tool's change to fan control. The curve was kept, and only
/// Retry, or Resume, brings it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoolingProblem {
    /// Such as "Fan curve paused".
    pub title: String,
    /// Why it stopped, how each fan fell back, and what brings the curve back.
    pub detail: Vec<String>,
    /// The button's title: "Retry" after a fault, "Resume" after external control.
    pub action: String,
    /// Why the button can't be pressed now, when it can't.
    pub unavailable: Option<String>,
    /// Why prismd didn't apply the latest Retry or Resume, until the next one.
    pub outcome: Option<String>,
}

/// What drives automatic switching on a GPU, or why it's waiting, paused, overridden, blocked or
/// failed, with Pause or Resume, and Retry on a failure pause. It doesn't establish which tuning
/// values are in effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchingStatus {
    /// Such as "Cyberpunk 2077 → Quiet", "Paused" or "Failed".
    pub summary: String,
    /// Why, and what happens next.
    pub detail: Vec<String>,
    /// Whether automatic switching is in a failure pause on the GPU, which Retry ends.
    pub failed: bool,
    /// Whether Resume ends what holds it, an automatic switching pause or a manual override, so
    /// the button is Resume, not Pause.
    pub resumable: bool,
    /// Why Pause, or Resume, can't be pressed now, when it can't.
    pub pause_unavailable: Option<String>,
    /// Why Retry can't be pressed now, when it can't.
    pub retry_unavailable: Option<String>,
    /// Why prismd refused the latest Pause, Resume or Retry, until the next one or the status
    /// changes.
    pub outcome: Option<String>,
}

/// Which of a GPU's fans the cooling controls set: every fan together, or one fan on its own, with
/// whether the fans share one configuration and any pending change to that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FanArrangementControl {
    /// Every fan together, then each fan, left to right.
    pub choices: Vec<FanChoice>,
    /// Whether the fans share one configuration or each has its own settings, as shown: the
    /// pending arrangement, or else the one in effect.
    pub arrangement: FanArrangement,
    /// The arrangement staged, as shown next to the control, such as "→ per fan", while it's
    /// pending.
    pub pending: Option<String>,
}

/// One of the fan picker's choices: every fan together, or one fan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FanChoice {
    /// The fan it picks, by index, or none for every fan.
    pub fan: Option<u32>,
    /// As shown, such as "all" or "1".
    pub text: String,
    /// As a screen reader says it, such as "Shared" or "Fan 1".
    pub name: String,
    /// Whether the controls set the fans it picks.
    pub selected: bool,
    /// Why it can't be picked, when it can't.
    pub unavailable: Option<String>,
}

/// One of the fan control mode's choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FanModeChoice {
    /// The mode it picks.
    pub mode: FanMode,
    /// As shown, such as "Fixed speed".
    pub name: &'static str,
    /// Whether it's the mode pending, or else the one in effect.
    pub selected: bool,
    /// Why it can't be picked, when it can't.
    pub unavailable: Option<String>,
}

/// The fan curve editor: the curve the GPU's fans follow, or the one pending in its place, drawn
/// with temperature across and requested fan speed up, its points moved by dragging or with the
/// keyboard.
#[derive(Debug, Clone, PartialEq)]
pub struct FanCurveEditor {
    /// Whether it's open; collapsed, the fan speed history shows in its place.
    pub open: bool,
    /// The points it edits: the pending curve's, or else the curve in effect's.
    pub points: Vec<CurvePoint>,
    /// Whether the points are a pending curve's.
    pub pending: bool,
    /// The curve in effect as a line across the editor, if the fans follow one.
    pub in_effect_line: Option<Vec<ChartPoint>>,
    /// The pending curve as a line across the editor, drawn over the one in effect.
    pub pending_line: Option<Vec<ChartPoint>>,
    /// The temperatures at its left and right edges, in °C; its speeds run from 0% at the bottom
    /// to 100% at the top.
    pub temperature_axis: (i64, i64),
    /// Those temperatures as shown, such as "20 °C" and "100 °C".
    pub temperature_labels: [String; 2],
    /// The speeds a point can request, from the lowest every fan supports to the highest, in %;
    /// 0–100% while they're unknown.
    pub speeds: (i64, i64),
    /// Where across it a curve must reach full speed by, 5 °C below the GPU's maximum operating
    /// temperature, when that's known.
    pub full_speed_x: Option<f32>,
    /// Where the GPU's temperature and fan speed are now, while both are known.
    pub marker: Option<CurveMarker>,
    /// What keeps the pending curve from being applied, such as "Point 2: speed can't be lower
    /// than point 1's 40%."
    pub problems: Vec<String>,
    /// Why its points can't be moved, when they can't.
    pub read_only: Option<String>,
}

impl FanCurveEditor {
    /// What keeps the pending curve from being applied, in one paragraph.
    pub fn problems_text(&self) -> String {
        self.problems.join(" ")
    }
}

/// One of the fan curve's points, where it's drawn in the editor.
#[derive(Debug, Clone, PartialEq)]
pub struct CurvePoint {
    pub temperature_c: i64,
    pub speed_percent: i64,
    pub at: ChartPoint,
    /// What a screen reader calls it, such as "Fan curve point 2".
    pub name: String,
    /// Its speed and temperature as a screen reader says them, such as "65% at 65 °C".
    pub value_text: String,
    /// Whether one of the pending curve's problems is with it.
    pub faulty: bool,
}

/// The GPU's temperature and fan speed now, on the fan curve editor.
#[derive(Debug, Clone, PartialEq)]
pub struct CurveMarker {
    pub at: ChartPoint,
    /// Such as "Now 60 °C, fans at 55%".
    pub text: String,
}

impl Track {
    /// What a screen reader calls the readout, such as "Power draw reading".
    pub fn readout_label(&self) -> String {
        format!("{} reading", capitalized(self.kind.metric().name()))
    }

    /// What a screen reader calls the max, avg and min, such as "Maximum power draw".
    pub fn statistic_labels(&self) -> [String; 3] {
        let metric = self.kind.metric().name();
        ["Maximum", "Average", "Minimum"].map(|statistic| format!("{statistic} {metric}"))
    }
}

/// What a stepper stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tunable {
    Setting(ProfileSettingKey),
    /// The fixed speed every fan shares, in percent.
    FixedFanSpeed,
}

/// A tuning value stepped through its allowed range, such as the power limit, with the value in
/// effect and any pending one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stepper {
    /// What it stages.
    pub setting: Tunable,
    /// What a screen reader calls it, such as "Power limit".
    pub name: String,
    /// The value it steps from: the pending one, or else the one in effect.
    pub value: i64,
    /// That value as a screen reader says it, such as "450 W".
    pub value_text: String,
    pub min: i64,
    pub max: i64,
    pub step: i64,
    /// The value in effect, as shown.
    pub readout: Readout,
    /// The pending value as shown next to the control, such as "→ 450 W" or "→ default", while
    /// one is staged.
    pub pending: Option<String>,
    /// Whether Use default can stage the setting's default, which it can while it's editable and
    /// the default isn't staged already.
    pub can_use_default: bool,
    /// That the last Apply didn't put the pending value into effect, and why if prismd said, such
    /// as "Not applied: Insufficient permissions", while it's still pending.
    pub failed: Option<String>,
    /// Why it can't be changed, when it can't.
    pub read_only: Option<String>,
}

/// A current value a track shows under its readout, such as the power limit.
#[derive(Debug, Clone, PartialEq)]
pub struct Detail {
    /// The label as shown, such as "limit".
    pub label: String,
    /// What a screen reader calls it, such as "Power limit".
    pub name: String,
    pub readout: Readout,
    /// Why the value is unknown, when it is.
    pub unknown_reason: Option<String>,
    /// The limit it is, which is drawn across the chart, when it's one.
    pub limit: Option<LimitKind>,
    /// The stepper that tunes it in place, when it's a tuning value.
    pub stepper: Option<Stepper>,
}

/// One of a track's charts: a metric's readings over the range, and any limits across it.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackChart {
    pub metric: Metric,
    /// What tells it apart from the track's other chart, such as "MEMORY".
    pub label: Option<&'static str>,
    /// Whether the area under the series is filled, as utilization's is.
    pub area: bool,
    pub chart: Chart,
    pub limits: Vec<LimitLine>,
}

/// The workspace's hardware details: the GPU inventory's entry for the GPU and its GPU control
/// capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardwareDetails {
    /// Whether they're open; they start collapsed.
    pub open: bool,
    pub inventory: Vec<Fact>,
    pub capabilities: Vec<Fact>,
}

/// A state with nothing to show, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainedState {
    pub title: String,
    pub reason: String,
    pub hint: String,
}

/// A dialog over the whole window, which waits for an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dialog {
    /// prismd refused an Apply because the GPU's tuning state changed since its draft was made.
    Review(Review),
    /// Closing Prism with unapplied tuning changes: "Discard and close" or "Keep editing".
    ConfirmClose(ConfirmClose),
    /// Choosing a profile over the GPU's unapplied tuning changes: "Discard and apply" or "Keep
    /// editing".
    DiscardForProfile(DiscardForProfile),
    /// A profile's values out of the GPU's range: "Adjust profile and apply" or "Cancel".
    ProfileCorrections(ProfileCorrections),
    /// Naming a new profile, or a profile's new name.
    ProfileName(ProfileNameDialog),
    /// Deleting a profile: "Delete" or "Cancel".
    DeleteProfile(ConfirmDelete),
    /// The GPU's profiles, each to rename or delete.
    ManageProfiles(ManageProfiles),
    /// The caller's Steam apps, each to add as a game.
    AddSteamGame(AddSteamGame),
    /// The caller's running programs, each to add as a game, or a program file to browse for.
    AddProgram(AddProgram),
    /// Running the recommended driver plan: "Run" or "Cancel".
    RunDriverPlan(DriverConfirmation),
    /// Restarting the host: "Reboot" or "Cancel".
    Reboot(DriverConfirmation),
}

/// The caller's installed Steam apps, searched, to add one as a game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddSteamGame {
    pub title: String,
    pub message: String,
    /// The search as typed.
    pub filter: String,
    /// The apps that match the search, ordered by name.
    pub apps: Vec<SteamAppChoice>,
    /// What the list waits for, or why it's empty, such as "Reading your Steam libraries…"; or
    /// which app is being added.
    pub status: Option<String>,
    /// Why prismd refused to add the last app chosen.
    pub error: Option<String>,
}

/// One of the caller's Steam apps in Add Steam game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamAppChoice {
    /// The app's id, which adding it sends.
    pub app_id: u32,
    pub name: String,
    /// Why it can't be added now, such as it being added already.
    pub unavailable: Option<String>,
}

/// The caller's running programs, searched, to add one as a game, or a program file to browse
/// for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddProgram {
    pub title: String,
    pub message: String,
    /// The search as typed.
    pub filter: String,
    /// The running programs that match the search, as prismd orders them, each program once.
    pub programs: Vec<ProgramChoice>,
    /// What the list waits for, or why it's empty, such as "Reading your running programs…"; or
    /// which program is being added, or that the file chooser is open.
    pub status: Option<String>,
    /// Whether Browse can open the file chooser now: not while it's open or a program is being
    /// added.
    pub can_browse: bool,
    /// Why the last program chosen couldn't be added.
    pub error: Option<String>,
}

/// One of the caller's running programs in Add program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramChoice {
    /// Its host location, which adding it sends; none when prismd couldn't recover it.
    pub path: Option<String>,
    pub name: String,
    /// Its host location, or that it's unknown.
    pub detail: String,
    /// Why prismd may not recognize it by this location, such as for a script or an emulator.
    pub warning: Option<String>,
    /// Why it can't be added now, such as it being added already.
    pub unavailable: Option<String>,
}

/// The Games section: the automatic switching toggle, then the caller's games.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GamesSection {
    /// Whether the caller turned automatic switching on; unknown without a session.
    pub automatic_switching: Option<bool>,
    /// Why nothing here can be changed, such as "Needs tuning access · not running"; none with
    /// tuning access.
    pub read_only: Option<String>,
    /// Why prismd can't recognize running games, so automatic switching can't follow them.
    pub recognition_problem: Option<String>,
    /// Why prismd refused the latest change, until the next one.
    pub outcome: Option<String>,
    /// The caller's games, in the order prismd lists them.
    pub games: Vec<GameRow>,
    /// What the list says while it has no games, such as that there are none yet.
    pub empty: String,
}

/// One of the caller's games, with its link on each GPU here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameRow {
    /// What identifies the game, which changing it sends.
    pub game: prism_protocol::GameId,
    pub name: String,
    /// What it is, such as "Steam app 570" or the program's location, and whether it's missing,
    /// such as "Steam app 620 · Not installed".
    pub detail: String,
    /// The game's link on each GPU here, in sidebar order.
    pub links: Vec<GameLink>,
}

/// A game's link to at most one of the caller's profiles of a GPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameLink {
    pub gpu_id: String,
    /// Such as "GPU 0 · RTX 5090 · 01:00.0".
    pub gpu: String,
    /// The linked profile's name, "None", or "Unknown profile" for a link to none of the caller's
    /// profiles.
    pub linked: String,
    /// None, then the caller's profiles of the GPU, in the order prismd lists them.
    pub choices: Vec<LinkChoice>,
}

/// None, or one of the GPU's profiles, as a game's link can be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkChoice {
    /// The profile's id, which linking it sends; none for None.
    pub profile_id: Option<String>,
    pub name: String,
    /// Whether the game is linked to it on the GPU.
    pub linked: bool,
}

/// Drivers: the host's NVIDIA driver, the recommended driver plan and the latest driver job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriversSection {
    /// Why nothing here can run, such as "Needs tuning access · bob isn't in group wheel"; none
    /// with tuning access, or without a session, which `status` explains.
    pub read_only: Option<String>,
    /// What the section says in place of the host's driver status, such as that prismd is
    /// checking it, or why it can't.
    pub status: Option<String>,
    /// The host's distribution, kernel, package manager, Secure Boot, NVIDIA driver and GPUs.
    pub host: Vec<Fact>,
    /// What to know about the host before changing its driver, such as an NVIDIA runfile
    /// installation.
    pub warnings: Vec<String>,
    pub plan: Option<DriverPlanView>,
    /// The latest driver job of this host boot, if any.
    pub job: Option<DriverJobView>,
    /// Why prismd refused the latest request, until the next one.
    pub outcome: Option<String>,
}

/// The recommended driver plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverPlanView {
    /// Such as "Install recommended NVIDIA driver".
    pub title: String,
    /// What keeps it from running.
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    /// What's left to do once its package operations finish, such as restarting.
    pub manual_steps: Vec<String>,
    /// Why Run can't open its confirmation now, when it can't.
    pub run_unavailable: Option<String>,
}

/// A driver job, with its progress, log and outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverJobView {
    /// The driver action it runs, such as "Install recommended NVIDIA driver".
    pub title: String,
    /// Such as "Running" or "Reboot required".
    pub state: String,
    /// Whether it's queued or running.
    pub active: bool,
    /// Whether it failed.
    pub failed: bool,
    /// The step running now, such as "Installing NVIDIA driver packages", while it's active.
    pub step: Option<String>,
    /// What its state means, such as that the host needs a restart to load the driver.
    pub explanation: Vec<String>,
    /// Its log's last lines, oldest first.
    pub log: Vec<String>,
    /// Why it can't be canceled now, when it can't.
    pub cancel_unavailable: Option<String>,
    /// Whether it left the host needing a restart, which Reboot offers.
    pub reboot: bool,
    /// Why Reboot can't open its confirmation now, when it can't.
    pub reboot_unavailable: Option<String>,
}

/// The question running the recommended driver plan, or restarting the host, asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverConfirmation {
    /// Such as "Install recommended NVIDIA driver?".
    pub title: String,
    /// What confirming does.
    pub message: String,
    /// The plan's warnings and what's left to do after it, one per line.
    pub details: Vec<String>,
    /// "Run" or "Reboot".
    pub confirm_text: &'static str,
    /// Why it can't be confirmed now, when it can't.
    pub confirm_unavailable: Option<String>,
    /// Why prismd refused it.
    pub error: Option<String>,
}

/// The Activity panel: current problems first, then this boot's startup outcomes not yet
/// dismissed, then recent events from prismd and Prism itself, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityPanel {
    pub problems: Vec<ActivityItem>,
    /// Startup outcomes that applied in full; those that didn't are problems.
    pub startup: Vec<ActivityItem>,
    pub recent: Vec<ActivityItem>,
    /// Why prismd refused the latest action, until the next one.
    pub outcome: Option<String>,
    /// What the panel says while it lists nothing.
    pub empty: Option<String>,
}

/// A problem or an event in Activity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityItem {
    /// Such as "Quiet applied in part".
    pub title: String,
    /// Its lines, such as the GPU it happened to and each setting's outcome.
    pub detail: Vec<String>,
    /// How long ago it happened, such as "5 min ago"; none for a current problem.
    pub when: Option<String>,
    pub action: Option<ActivityButton>,
}

/// An Activity item's action, such as Dismiss or Open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityButton {
    pub title: String,
    pub action: ActivityAction,
    /// Why it can't run now, when it can't.
    pub unavailable: Option<String>,
}

/// The name of a new profile, or a profile's new name, in a text field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileNameDialog {
    /// Such as "Save as new profile" or "Rename Quiet".
    pub title: String,
    pub message: String,
    /// The name as typed.
    pub name: String,
    /// What's wrong with the name, such as another profile of the GPU having it, or why prismd
    /// refused it.
    pub error: Option<String>,
    /// Such as "Save" or "Rename".
    pub confirm_text: &'static str,
    /// Why the name can't be sent now, when it can't.
    pub confirm_unavailable: Option<String>,
}

/// The question deleting a profile asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmDelete {
    /// Such as "Delete Quiet?".
    pub title: String,
    /// What deleting it does.
    pub message: String,
    /// Why prismd refused to delete it.
    pub error: Option<String>,
    /// Why it can't be deleted now, when it can't.
    pub confirm_unavailable: Option<String>,
}

/// The GPU's profiles, to rename or delete any of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManageProfiles {
    pub title: String,
    pub message: String,
    /// The caller's profiles of the GPU, in the order prismd lists them.
    pub profiles: Vec<ManagedProfile>,
    /// Why no profile can be renamed or deleted now, when none can.
    pub actions_unavailable: Option<String>,
}

/// One of the GPU's profiles in Manage profiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedProfile {
    /// The profile's id, which renaming or deleting it sends.
    pub id: String,
    pub name: String,
    /// Whether it's the current or the startup profile, such as "Current profile · Startup
    /// profile", or empty.
    pub marks: String,
}

/// The question choosing a profile over unapplied tuning changes asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscardForProfile {
    /// Such as "Discard 2 unapplied changes and apply Quiet?".
    pub title: String,
    /// What applying the profile does to the unapplied changes.
    pub message: String,
}

/// The one review of every range correction applying a profile needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileCorrections {
    /// Such as "Adjust Quiet to apply it?".
    pub title: String,
    /// What adjusting does, and that nothing has changed yet.
    pub message: String,
    /// Each out-of-range setting, from its saved value to the one it would become.
    pub corrections: Vec<ReviewChange>,
}

/// What changed under a GPU's draft, with Apply to apply it again over the new state and Discard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    /// Such as "GPU 0 · RTX 5090 · 01:00.0 changed since these changes were made".
    pub title: String,
    /// What happened, and what the user can do about it.
    pub message: String,
    /// The current tuning values that changed, if any of them did.
    pub changes: Vec<ReviewChange>,
    /// Apply's text: "Apply" over a change the draft hasn't been applied over yet, or else
    /// "Apply again".
    pub apply_text: &'static str,
    /// Why Apply can't run now, when it can't.
    pub apply_unavailable: Option<String>,
    /// Why Discard can't run now, when it can't.
    pub discard_unavailable: Option<String>,
}

/// The question closing with unapplied tuning changes asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmClose {
    /// How many unapplied changes every GPU has, such as "3 unapplied changes".
    pub summary: String,
    /// What closing does to them.
    pub message: String,
}

/// The status bar's items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusBar {
    pub connection: String,
    /// Whether there is a session with prismd.
    pub connected: bool,
    pub telemetry: String,
    /// Whether any GPU has unapplied tuning changes, which hold automatic changes: EDIT HOLD.
    pub edit_hold: bool,
    /// What drives automatic switching on the selected GPU, or why it doesn't, such as
    /// "AUTO · CYBERPUNK 2077 → QUIET"; none without a session.
    pub switching: Option<String>,
    /// How many current problems Activity lists.
    pub problems: usize,
    /// The problem indicator, which opens Activity, such as "2 PROBLEMS".
    pub problem_indicator: String,
}

impl Core {
    /// Whether a dialog is over the window.
    pub fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// The dialog over the window, while one is open.
    pub fn dialog(&self) -> Option<Dialog> {
        let state = self.dialog.as_ref()?;
        if let Some(dialog) = self.profile_change_dialog(state) {
            return dialog;
        }
        if let Some(dialog) = self.add_steam_game_dialog(state) {
            return Some(Dialog::AddSteamGame(dialog));
        }
        if let Some(dialog) = self.add_program_dialog(state) {
            return Some(Dialog::AddProgram(dialog));
        }
        if let Some(dialog) = self.driver_dialog(state) {
            return Some(dialog);
        }
        Some(match state {
            DialogState::Review { gpu_id, changes } => {
                let gpu = self
                    .gpus()
                    .iter()
                    .enumerate()
                    .find(|(_, gpu)| &gpu.id == gpu_id)
                    .map_or_else(|| gpu_id.clone(), |(index, gpu)| gpu_label(index, gpu));
                Dialog::Review(Review {
                    title: format!("{gpu} changed since these changes were made"),
                    message: if changes.is_empty() {
                        "Its tuning state changed, but not the values these changes set. Apply \
                         them again, or discard them."
                    } else {
                        "Another Prism, automatic switching or a change outside Prism changed \
                         these values. Apply your changes over them, or discard them."
                    }
                    .into(),
                    changes: changes.clone(),
                    apply_text: if self.cooling_changed_under(gpu_id) {
                        "Apply"
                    } else {
                        "Apply again"
                    },
                    apply_unavailable: self.changes_unavailable(&Command::ApplyChanges),
                    discard_unavailable: self.changes_unavailable(&Command::DiscardChanges),
                })
            }
            DialogState::ConfirmClose => Dialog::ConfirmClose(ConfirmClose {
                summary: unapplied(self.unapplied_count()),
                message: "Closing Prism discards them and ends their edit hold.".into(),
            }),
            DialogState::DiscardForProfile { profile_id } => {
                let profile = self.profile(profile_id)?;
                let count = self
                    .drafts
                    .by_gpu
                    .get(&profile.gpu_id)
                    .map_or(0, |draft| draft.len());
                Dialog::DiscardForProfile(DiscardForProfile {
                    title: format!(
                        "Discard {} and apply {}?",
                        unapplied(count).to_lowercase(),
                        profile.name
                    ),
                    message: "The profile replaces the GPU's tuning values, so your unapplied \
                              changes would be lost."
                        .into(),
                })
            }
            DialogState::ProfileCorrections {
                profile_id,
                corrections,
            } => Dialog::ProfileCorrections(ProfileCorrections {
                title: format!("Adjust {} to apply it?", self.profile(profile_id)?.name),
                message: "These values are outside what the GPU allows now. Adjusting saves the \
                          nearest allowed values to the profile, then applies it. Nothing has \
                          changed yet."
                    .into(),
                corrections: corrections.iter().map(correction_change).collect(),
            }),
            DialogState::ProfileName { .. }
            | DialogState::DeleteProfile { .. }
            | DialogState::ManageProfiles { .. }
            | DialogState::AddSteamGame { .. }
            | DialogState::AddProgram { .. }
            | DialogState::RunDriverPlan { .. }
            | DialogState::Reboot { .. } => return None,
        })
    }

    pub fn sidebar(&self) -> Sidebar {
        Sidebar {
            gpus: self
                .gpus()
                .iter()
                .enumerate()
                .map(|(index, gpu)| self.sidebar_gpu(index, gpu))
                .collect(),
            no_gpus: self.no_gpus(),
            section: match &self.selection {
                Some(Selection::Section(section)) => Some(*section),
                _ => None,
            },
        }
    }

    pub fn content(&self) -> Content {
        match &self.selection {
            Some(Selection::Section(section)) => Content::Section(*section),
            Some(Selection::Gpu(id)) => match self.gpus().iter().position(|gpu| &gpu.id == id) {
                Some(index) => Content::Gpu(Box::new(self.workspace(index, &self.gpus()[index]))),
                // The GPU left the inventory.
                None => self.no_gpus().map_or(Content::Searching, Content::NoGpus),
            },
            None => Content::Searching,
        }
    }

    pub fn status_bar(&self) -> StatusBar {
        let problems = self.problems().len();
        StatusBar {
            connection: self.connection_item(),
            connected: self.connected(),
            telemetry: if self.no_gpus().is_some() {
                "NO GPU TELEMETRY"
            } else {
                match self.telemetry.as_ref().map(|telemetry| telemetry.source) {
                    None => "WAITING FOR TELEMETRY",
                    Some(TelemetrySource::InProcess) => "MONITORING IN-PROCESS",
                    Some(TelemetrySource::Prismd) => "MONITORING THROUGH PRISMD",
                }
            }
            .into(),
            edit_hold: self.edit_hold(),
            switching: self.switching_item(),
            problems,
            problem_indicator: match problems {
                0 => "NO PROBLEMS".into(),
                1 => "1 PROBLEM".into(),
                count => format!("{count} PROBLEMS"),
            },
        }
    }

    /// The status bar's prismd item: "PRISMD · " and the reason, or that it's connected.
    fn connection_item(&self) -> String {
        let state = match self.prismd() {
            PrismdConnection::Connected(welcome) if !welcome.can_tune => "not allowed",
            PrismdConnection::Connected(_) => "connected",
            other => other.reason().unwrap_or_default(),
        };
        format!("PRISMD · {}", state.to_uppercase())
    }

    pub(crate) fn gpus(&self) -> &[Gpu] {
        match &self.discovery {
            Discovery::Found { gpus, .. } => gpus,
            Discovery::Searching => &[],
        }
    }

    fn no_gpus(&self) -> Option<ExplainedState> {
        let Discovery::Found { gpus, warnings } = &self.discovery else {
            return None;
        };
        if !gpus.is_empty() {
            return None;
        }
        Some(if warnings.is_empty() {
            ExplainedState {
                title: "No GPUs found".into(),
                reason: "NVML reports no NVIDIA GPUs on this host.".into(),
                hint: "An empty list alone doesn't mean the host has no GPU. Check that the \
                       NVIDIA driver is loaded, then restart Prism."
                    .into(),
            }
        } else {
            ExplainedState {
                title: "No GPU telemetry".into(),
                reason: warnings.join(" "),
                hint: "Prism reads NVIDIA GPUs through NVML, which comes with the NVIDIA driver. \
                       Check that the driver is installed and loaded, then restart Prism."
                    .into(),
            }
        })
    }

    fn sidebar_gpu(&self, index: usize, gpu: &Gpu) -> SidebarGpu {
        let latest = latest_readings(gpu);
        SidebarGpu {
            id: gpu.id.clone(),
            label: gpu_label(index, gpu),
            index: index_label(index),
            name: short_gpu_name(&gpu.device.name).to_owned(),
            identity: gpu
                .device
                .bus_id
                .as_deref()
                .map(short_bus_id)
                .unwrap_or_default()
                .to_owned(),
            temperature: Readout::celsius(latest.temperature_c),
            utilization: Readout::percent(latest.utilization_percent),
            power_draw: Readout::watts(latest.power_draw_watts),
            fan_speed: Readout::percent(latest.fan_speed_percent),
            unknown_reason: unknown_reason(gpu, &VITALS),
            pauses: [
                self.cooling_paused(&gpu.id).then_some("Fan curve paused"),
                self.switching_failure(&gpu.id),
            ]
            .into_iter()
            .flatten()
            .map(str::to_owned)
            .collect(),
            selected: self.selection == Some(Selection::Gpu(gpu.id.clone())),
        }
    }

    fn workspace(&self, index: usize, gpu: &Gpu) -> GpuWorkspace {
        let index_label = index_label(index);
        let subtitle = [
            Some(index_label.as_str()),
            gpu.device.bus_id.as_deref().map(short_bus_id),
            gpu.device.architecture.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        GpuWorkspace {
            id: gpu.id.clone(),
            title: gpu.device.name.clone(),
            subtitle,
            profiles: self.profile_switcher(&gpu.id),
            changes: self.change_strip(),
            no_tuning_access: self.tuning_access().reason().map(str::to_owned),
            switching: self.switching_status(&gpu.id),
            range: self.range,
            tracks: TrackKind::ALL
                .into_iter()
                .map(|kind| self.track(gpu, kind))
                .collect(),
            hardware: HardwareDetails {
                open: self.hardware_open,
                inventory: inventory_facts(&gpu.device),
                capabilities: capability_facts(gpu),
            },
            reveal_fans: self.fans_revealed,
        }
    }

    /// When the latest sample arrived, which the ranges look back from.
    fn now(&self) -> Duration {
        self.telemetry
            .as_ref()
            .map(|telemetry| telemetry.received_at)
            .unwrap_or_default()
    }

    fn track(&self, gpu: &Gpu, kind: TrackKind) -> Track {
        let metric = kind.metric();
        let now = self.now();
        let chart = |metric, label, also_fit: &[Option<u32>]| TrackChart {
            metric,
            label,
            area: metric == Metric::Utilization,
            chart: gpu.history.chart(
                metric,
                self.range,
                now,
                &also_fit.iter().flatten().copied().collect::<Vec<_>>(),
            ),
            limits: Vec::new(),
        };
        let (details, charts) = match kind {
            TrackKind::Temperature | TrackKind::Utilization => {
                (Vec::new(), vec![chart(metric, None, &[])])
            }
            TrackKind::Power => {
                let power = latest_device(gpu).map(|device| &device.power);
                let configured = power.and_then(|power| power.current_limit_watts);
                let enforced = power.and_then(|power| power.enforced_limit_watts);
                let staged = self.staged_power_limit(gpu);
                let mut draw = TrackChart {
                    metric,
                    label: None,
                    area: false,
                    chart: self.power_chart(gpu),
                    limits: Vec::new(),
                };
                draw.limits = [
                    (LimitKind::Configured, configured),
                    (LimitKind::Enforced, enforced),
                    (LimitKind::Pending, staged),
                ]
                .into_iter()
                .filter_map(|(kind, watts)| {
                    watts.map(|watts| LimitLine {
                        kind,
                        y: draw.chart.y(watts),
                    })
                })
                .collect();
                let said = power.and_then(|power| {
                    gpu_said(
                        power.current_value_error.as_deref(),
                        power.reason.as_deref(),
                    )
                });
                let limit = |kind, label: &str, name: &str, watts| Detail {
                    label: label.into(),
                    name: name.into(),
                    readout: Readout::watts(watts),
                    unknown_reason: value_reason(gpu, watts.is_some(), said),
                    limit: Some(kind),
                    stepper: None,
                };
                (
                    vec![
                        Detail {
                            stepper: Some(self.stepper(gpu, ProfileSettingKey::PowerLimitWatts)),
                            ..limit(LimitKind::Configured, "limit", "Power limit", configured)
                        },
                        limit(
                            LimitKind::Enforced,
                            "enforced",
                            "Enforced power limit",
                            enforced,
                        ),
                    ],
                    vec![draw],
                )
            }
            TrackKind::Clocks => {
                let memory = latest_readings(gpu).memory_clock_mhz;
                let clocks = latest_device(gpu).map(|device| &device.clocks);
                let offset =
                    |label: &str,
                     setting: ProfileSettingKey,
                     offset: Option<&prism_gpu::ClockOffsetInfo>| {
                        let mhz = offset.and_then(|offset| offset.current_offset_mhz);
                        let said = offset.and_then(|offset| {
                            gpu_said(
                                offset.current_value_error.as_deref(),
                                offset.reason.as_deref(),
                            )
                        });
                        Detail {
                            label: label.into(),
                            name: setting_name(setting).into(),
                            readout: Readout::offset_megahertz(mhz),
                            unknown_reason: value_reason(gpu, mhz.is_some(), said),
                            limit: None,
                            stepper: Some(self.stepper(gpu, setting)),
                        }
                    };
                (
                    vec![
                        Detail {
                            label: "memory".into(),
                            name: "Memory clock".into(),
                            readout: Readout::megahertz(memory),
                            unknown_reason: unknown_reason(gpu, &[Metric::MemoryClock]),
                            limit: None,
                            stepper: None,
                        },
                        offset(
                            "graphics offset",
                            ProfileSettingKey::GraphicsOffsetMhz,
                            clocks.map(|clocks| &clocks.graphics),
                        ),
                        offset(
                            "memory offset",
                            ProfileSettingKey::MemoryOffsetMhz,
                            clocks.map(|clocks| &clocks.memory),
                        ),
                    ],
                    vec![
                        chart(
                            Metric::GraphicsClock,
                            Some("GRAPHICS"),
                            &[gpu.device.clocks.graphics_max_mhz],
                        ),
                        chart(
                            Metric::MemoryClock,
                            Some("MEMORY"),
                            &[gpu.device.clocks.memory_max_mhz],
                        ),
                    ],
                )
            }
            TrackKind::Fans => {
                let fans = latest_device(gpu)
                    .map(|device| device.cooling.fans.as_slice())
                    .unwrap_or_default();
                let fixed_speed = self.fixed_fan_speed(gpu).map(|stepper| Detail {
                    label: "fixed speed".into(),
                    name: stepper.name.clone(),
                    readout: stepper.readout.clone(),
                    unknown_reason: None,
                    limit: None,
                    stepper: Some(stepper),
                });
                let details = fixed_speed
                    .into_iter()
                    .chain(fans.iter().map(|fan| Detail {
                        label: format!("fan {}", fan.index),
                        name: format!("Fan {} speed", fan.index),
                        readout: Readout::percent(fan.current_speed_percent),
                        unknown_reason: value_reason(
                            gpu,
                            fan.current_speed_percent.is_some(),
                            None,
                        ),
                        limit: None,
                        stepper: None,
                    }))
                    .collect();
                (details, vec![chart(metric, None, &[])])
            }
        };
        let stats = gpu.history.stats(metric, self.range, now);
        let value = |value: Option<u32>| readout(metric, value);
        Track {
            kind,
            readout: value(latest_readings(gpu).get(metric)),
            unknown_reason: unknown_reason(gpu, &[metric]),
            details,
            range: self.range.label(),
            max: value(stats.map(|stats| stats.max)),
            avg: value(stats.map(|stats| stats.avg.round() as u32)),
            min: value(stats.map(|stats| stats.min)),
            charts,
            fan_mode: (kind == TrackKind::Fans).then(|| self.fan_mode(gpu)),
            fan_curve: (kind == TrackKind::Fans)
                .then(|| self.fan_curve(gpu))
                .flatten(),
        }
    }
}

impl Core {
    /// The power chart's scale fits the readings, the limits in effect and the whole allowed
    /// range, but not a pending limit, so the scale stays put while the limit's line is dragged.
    pub(crate) fn power_chart(&self, gpu: &Gpu) -> Chart {
        let power = latest_device(gpu).map(|device| &device.power);
        let control = &gpu.device.control.power_limit;
        let fit: Vec<_> = [
            power.and_then(|power| power.current_limit_watts),
            power.and_then(|power| power.enforced_limit_watts),
            power
                .and_then(|power| power.max_limit_watts)
                .or(control.max_watts),
            power
                .and_then(|power| power.min_limit_watts)
                .or(control.min_watts),
        ]
        .into_iter()
        .flatten()
        .collect();
        gpu.history
            .chart(Metric::PowerDraw, self.range, self.now(), &fit)
    }

    /// The power limit the GPU's draft stages, if it stages one, with a default as the GPU
    /// reports it.
    fn staged_power_limit(&self, gpu: &Gpu) -> Option<u32> {
        let key = ProfileSettingKey::PowerLimitWatts;
        match self.drafts.by_gpu.get(&gpu.id)?.settings.get(key)? {
            ProfileSetting::Value { value } => u32::try_from(value).ok(),
            ProfileSetting::Default {} => gpu.default_of(key)?.try_into().ok(),
        }
    }

    /// The stepper for the GPU's setting: the value in effect, any pending one, and why it's
    /// read-only when it is.
    fn stepper(&self, gpu: &Gpu, setting: ProfileSettingKey) -> Stepper {
        let in_effect = self.in_effect(gpu, setting);
        let draft = self.drafts.by_gpu.get(&gpu.id);
        let staged = draft.and_then(|draft| draft.settings.get(setting));
        let range = gpu.range(setting);
        let name = setting_name(setting);
        let said = latest_device(gpu).and_then(|device| match setting {
            ProfileSettingKey::PowerLimitWatts => device.power.reason.clone(),
            ProfileSettingKey::GraphicsOffsetMhz => device.clocks.graphics.reason.clone(),
            ProfileSettingKey::MemoryOffsetMhz => device.clocks.memory.reason.clone(),
        });
        let unsupported = said
            .unwrap_or_else(|| format!("The {} isn't supported by this GPU", name.to_lowercase()));
        let read_only = self
            .read_only(gpu, control_of(setting), unsupported)
            .or_else(|| {
                range.is_none().then(|| {
                    format!(
                        "The allowed {}s aren't reported by this GPU",
                        name.to_lowercase()
                    )
                })
            });
        let (min, max) = range.unwrap_or_default();
        let value = match staged {
            Some(ProfileSetting::Value { value }) => Some(value),
            Some(ProfileSetting::Default {}) => gpu.default_of(setting).or(in_effect),
            None => in_effect,
        };
        Stepper {
            setting: Tunable::Setting(setting),
            name: name.into(),
            value: value.unwrap_or(min),
            value_text: setting_readout(setting, value).spoken(),
            min,
            max,
            step: match setting {
                ProfileSettingKey::PowerLimitWatts => 1,
                ProfileSettingKey::GraphicsOffsetMhz | ProfileSettingKey::MemoryOffsetMhz => 10,
            },
            readout: setting_readout(setting, in_effect),
            pending: staged.map(|staged| pending(&staged_text(setting, staged))),
            can_use_default: read_only.is_none()
                && staged != Some(ProfileSetting::Default {})
                && !self.applying(&gpu.id),
            failed: draft
                .and_then(|draft| draft.failed.get(&Staged::Setting(setting)))
                .map(|why| not_applied("Not applied", why.as_deref())),
            read_only,
        }
    }

    /// Why the GPU's control can't be changed, if it can't: no tuning access, no reading yet, or
    /// what the GPU says about the control.
    fn read_only(&self, gpu: &Gpu, control: TuningControl, unsupported: String) -> Option<String> {
        match (self.tuning_access().reason(), &gpu.latest) {
            (Some(reason), _) => Some(reason.to_owned()),
            (None, Latest::Waiting) => Some(WAITING.into()),
            (None, Latest::Missing { reason }) => Some(reason.clone()),
            (None, Latest::Read(_)) => (!self.editable(&gpu.id, control)).then_some(unsupported),
        }
    }

    /// Why the GPU's cooling can't be changed, if it can't.
    pub(crate) fn cooling_read_only(&self, gpu: &Gpu) -> Option<String> {
        let unsupported = cooling_read(gpu)
            .and_then(|cooling| cooling.unsupported_reason.clone())
            .unwrap_or_else(|| "Fan control isn't supported by this GPU".into());
        self.read_only(gpu, TuningControl::Cooling, unsupported)
    }

    /// The fan control mode of the fans the GPU's cooling controls set: the one pending, or else
    /// the one in effect.
    fn fan_mode(&self, gpu: &Gpu) -> FanModeControl {
        let fans = self.controlled_fans(gpu);
        let staged = self.staged_cooling(&gpu.id);
        let in_effect = self.cooling_in_effect(gpu);
        let shown = self
            .shown_cooling(gpu)
            .as_ref()
            .and_then(|shown| mode_of(shown, fans));
        let cooling = cooling_read(gpu);
        let curves_unavailable = match self.curve_support(gpu, fans) {
            Some(FanCurveSupport::Unavailable { reason }) => Some(reason),
            Some(FanCurveSupport::Available { .. }) => None,
            None => {
                Some(cooling_unknown(gpu).unwrap_or_else(|| "Fan curves aren't available".into()))
            }
        };
        let modes = FanMode::ALL
            .into_iter()
            .map(|mode| FanModeChoice {
                mode,
                name: mode.name(),
                selected: shown == Some(mode),
                unavailable: match mode {
                    FanMode::Automatic => None,
                    FanMode::Fixed => match cooling {
                        Some(cooling) => fixed_speed_range(cooling, fans).err().map(Into::into),
                        None => cooling_unknown(gpu),
                    },
                    FanMode::Curve => curves_unavailable.clone(),
                },
            })
            .collect();
        // Why curves are unavailable only where fans can be controlled at all; why a curve stopped
        // is the problem's.
        let note =
            curves_unavailable.filter(|_| gpu.capabilities.contains(&TuningControl::Cooling));
        // A mode is pending while those fans' setting differs from the one in effect.
        let staged_setting = staged.and_then(|staged| setting_of(staged, fans));
        let pending_mode = staged_setting
            .filter(|&setting| {
                in_effect
                    .as_ref()
                    .and_then(|in_effect| setting_of(in_effect, fans))
                    != Some(setting)
            })
            .map(|setting| pending(&FanMode::of(setting).short_name().to_lowercase()));
        FanModeControl {
            modes,
            pending: pending_mode,
            failed: self
                .drafts
                .by_gpu
                .get(&gpu.id)
                .and_then(|draft| draft.failed.get(&Staged::Cooling))
                .map(|why| not_applied("Not applied", why.as_deref())),
            read_only: self.cooling_read_only(gpu),
            note,
            problem: self.cooling_problem(&gpu.id),
            arrangement: self.fan_arrangement(gpu),
            can_recall: self.selected_gpu().as_ref() == Some(&gpu.id)
                && self.recall_unavailable().is_none(),
        }
    }

    /// Which of the GPU's fans the cooling controls set, while it has more than one fan. Read-only,
    /// fans that share one configuration have nothing to pick, and the fans can't be made to share
    /// one.
    fn fan_arrangement(&self, gpu: &Gpu) -> Option<FanArrangementControl> {
        let fans = &cooling_read(gpu)?.fans;
        let arrangement = self.shown_arrangement(gpu);
        let read_only = self.cooling_read_only(gpu);
        if fans.len() < 2 || (read_only.is_some() && arrangement == FanArrangement::Shared) {
            return None;
        }
        let controlled = self.controlled_fans(gpu);
        let in_effect = self.cooling_in_effect(gpu).as_ref().map(FanArrangement::of);
        let every = FanChoice {
            fan: None,
            text: "all".into(),
            name: FanArrangement::Shared.name().into(),
            selected: controlled == Fans::Every,
            unavailable: read_only,
        };
        let each = fans.iter().map(|fan| FanChoice {
            fan: Some(fan.index),
            text: fan.index.to_string(),
            name: format!("Fan {}", fan.index),
            selected: controlled == Fans::One(fan.index),
            unavailable: None,
        });
        Some(FanArrangementControl {
            choices: std::iter::once(every).chain(each).collect(),
            arrangement,
            pending: self
                .staged_cooling(&gpu.id)
                .map(FanArrangement::of)
                .filter(|&staged| Some(staged) != in_effect)
                .map(|staged| pending(&staged.name().to_lowercase())),
        })
    }

    /// The stepper of the fixed speed of the fans the GPU's cooling controls set, while fixed
    /// speed is their mode pending, or else the one in effect.
    fn fixed_fan_speed(&self, gpu: &Gpu) -> Option<Stepper> {
        let fans = self.controlled_fans(gpu);
        let shown = self.shown_cooling(gpu)?;
        if mode_of(&shown, fans) != Some(FanMode::Fixed) {
            return None;
        }
        let in_effect_speed = self
            .cooling_in_effect(gpu)
            .as_ref()
            .and_then(|in_effect| fixed_speed_of(in_effect, fans));
        let staged_speed = self
            .staged_cooling(&gpu.id)
            .and_then(|staged| fixed_speed_of(staged, fans))
            .filter(|&speed| Some(speed) != in_effect_speed);
        let (min, max) = fixed_speed_range(cooling_read(gpu)?, fans).ok()?;
        let value = staged_speed.or(in_effect_speed).unwrap_or(min);
        // Under another mode, the fans' speed now is what's in effect.
        let readout = in_effect_speed
            .or_else(|| speed_now(gpu, fans))
            .and_then(|speed| speed.try_into().ok());
        Some(Stepper {
            setting: Tunable::FixedFanSpeed,
            name: match fans {
                Fans::Every => "Fixed fan speed".into(),
                Fans::One(index) => format!("Fan {index} fixed speed"),
            },
            value,
            value_text: Readout::percent(value.try_into().ok()).spoken(),
            min,
            max,
            step: 1,
            readout: Readout::percent(readout),
            pending: staged_speed.map(|speed| pending(&format!("{speed}%"))),
            can_use_default: false,
            // The fan control mode says when the last Apply didn't put cooling into effect.
            failed: None,
            read_only: self.cooling_read_only(gpu),
        })
    }

    /// The header's buttons for the GPU's profiles.
    fn profile_switcher(&self, gpu_id: &str) -> ProfileSwitcher {
        let current = self.profiles.current_profiles.get(gpu_id);
        let startup = self.profiles.startup_profiles.get(gpu_id);
        let read_only = self.tuning_access().reason().map(str::to_owned);
        ProfileSwitcher {
            choices: self
                .profiles_of(gpu_id)
                .map(|profile| ProfileChoice {
                    id: profile.id.clone(),
                    name: profile.name.clone(),
                    current: current.is_some_and(|current| current.id == profile.id),
                    startup: startup.is_some_and(|startup| startup.id == profile.id),
                    unavailable: read_only
                        .is_none()
                        .then(|| self.profile_unavailable(&profile.id))
                        .flatten(),
                })
                .collect(),
            current: current.map(|current| current.name.clone()),
            read_only,
            not_saved_to_profile: self.changes_not_saved_to_profile(gpu_id),
        }
    }

    /// The selected GPU's change strip.
    fn change_strip(&self) -> ChangeStrip {
        let draft = self.selected_draft();
        let count = draft.map_or(0, |draft| draft.len());
        ChangeStrip {
            summary: unapplied(count),
            pending: count > 0,
            outcome: match draft {
                Some(draft) => draft.outcome.clone(),
                None => self
                    .selected_gpu()
                    .and_then(|id| self.profile_outcomes.get(&id).cloned()),
            },
            apply_unavailable: draft.and_then(|_| self.changes_unavailable(&Command::ApplyChanges)),
            discard_unavailable: draft
                .and_then(|_| self.changes_unavailable(&Command::DiscardChanges)),
            review: self
                .selected_gpu()
                .filter(|id| self.cooling_changed_under(id))
                .map(|_| {
                    "The GPU's cooling changed since these changes were made. Review the change \
                     before applying over it."
                        .into()
                }),
        }
    }
}

/// How many unapplied changes there are, such as "No unapplied changes" or "2 unapplied changes".
fn unapplied(count: usize) -> String {
    match count {
        0 => "No unapplied changes".into(),
        1 => "1 unapplied change".into(),
        count => format!("{count} unapplied changes"),
    }
}

/// A reading of `metric` in its unit.
fn readout(metric: Metric, value: Option<u32>) -> Readout {
    match metric {
        Metric::Temperature => Readout::celsius(value),
        Metric::Utilization | Metric::FanSpeed => Readout::percent(value),
        Metric::PowerDraw => Readout::watts(value),
        Metric::GraphicsClock | Metric::MemoryClock => Readout::megahertz(value),
    }
}

/// What the GPU said about a current tuning value it didn't report: the error reading it, or why
/// the control is unavailable.
fn gpu_said<'a>(value_error: Option<&'a str>, control_reason: Option<&'a str>) -> Option<&'a str> {
    value_error.or(control_reason)
}

/// Why the GPU's cooling is unknown, while there's no reading of it.
fn cooling_unknown(gpu: &Gpu) -> Option<String> {
    match &gpu.latest {
        Latest::Waiting => Some(WAITING.into()),
        Latest::Missing { reason } => Some(reason.clone()),
        Latest::Read(_) => None,
    }
}

/// Why a value the latest reading carries is unknown: why there's no reading, or what the GPU
/// said about the value, if anything.
fn value_reason(gpu: &Gpu, known: bool, said: Option<&str>) -> Option<String> {
    match &gpu.latest {
        Latest::Waiting => Some(WAITING.into()),
        Latest::Missing { reason } => Some(reason.clone()),
        Latest::Read(_) if known => None,
        Latest::Read(_) => Some(said.unwrap_or(NOT_REPORTED).into()),
    }
}

fn fact(label: &str, value: impl Into<String>) -> Fact {
    Fact {
        label: label.into(),
        value: value.into(),
    }
}

/// The parts that are known, joined, such as "CUDA 13.2 · capability 12.0"; unknown with none.
fn joined(parts: impl IntoIterator<Item = Option<String>>) -> String {
    let parts: Vec<_> = parts.into_iter().flatten().collect();
    if parts.is_empty() {
        UNKNOWN.into()
    } else {
        parts.join(" · ")
    }
}

/// What the GPU inventory says about the GPU.
fn inventory_facts(device: &GpuDevice) -> Vec<Fact> {
    let known = |value: &Option<String>| value.clone().unwrap_or_else(|| UNKNOWN.into());
    let pcie = match (device.pcie.gen_max, device.pcie.width_max) {
        (Some(generation), Some(width)) => format!("Gen {generation} ×{width}"),
        (Some(generation), None) => format!("Gen {generation}"),
        (None, Some(width)) => format!("×{width}"),
        (None, None) => UNKNOWN.into(),
    };
    let compute = &device.compute;
    let api = compute.api.map(|api| {
        let name = match api {
            ComputeApi::Cuda => "CUDA",
            ComputeApi::Rocm => "ROCm",
        };
        match &compute.api_version {
            Some(version) => format!("{name} {version}"),
            None => name.into(),
        }
    });
    vec![
        fact("Model", device.name.clone()),
        fact("Architecture", known(&device.architecture)),
        fact("UUID", device.id.clone()),
        fact("Bus", known(&device.bus_id)),
        fact("PCIe", pcie),
        fact("Driver", known(&device.driver_version)),
        fact("VBIOS", known(&device.firmware_version)),
        fact(
            "Memory",
            device
                .memory
                .total_mib
                .map_or_else(|| UNKNOWN.into(), |mib| format!("{mib} MiB")),
        ),
        fact(
            "Compute",
            joined([
                api,
                compute
                    .capability
                    .as_ref()
                    .map(|capability| format!("capability {capability}")),
                compute.core_count.map(|cores| format!("{cores} cores")),
            ]),
        ),
        fact(
            "Maximum clocks",
            joined([
                device
                    .clocks
                    .graphics_max_mhz
                    .map(|mhz| format!("graphics {mhz} MHz")),
                device
                    .clocks
                    .memory_max_mhz
                    .map(|mhz| format!("memory {mhz} MHz")),
            ]),
        ),
    ]
}

const SUPPORTED: &str = "Supported";
const NOT_SUPPORTED: &str = "Not supported";
const RANGE_UNKNOWN: &str = "Supported · range unknown";

/// A supported control's allowed range, such as "400–600 W".
fn range(min: Option<u32>, max: Option<u32>, unit: &str) -> String {
    match (min, max) {
        (Some(min), Some(max)) => format!("{min}–{max} {unit}"),
        _ => RANGE_UNKNOWN.into(),
    }
}

/// A supported offset's allowed range, such as "-1000 to +1000 MHz".
fn offset_range(min: Option<i32>, max: Option<i32>) -> String {
    match (min, max) {
        (Some(min), Some(max)) => format!("{} to {} MHz", signed(min), signed(max)),
        _ => RANGE_UNKNOWN.into(),
    }
}

/// `allowed` when the GPU supports the control, or that it doesn't.
fn supported(supported: bool, allowed: impl FnOnce() -> String) -> String {
    if supported {
        allowed()
    } else {
        NOT_SUPPORTED.into()
    }
}

/// The GPU's control capabilities: what it lets Prism tune, and within which range.
fn capability_facts(gpu: &Gpu) -> Vec<Fact> {
    let control = &gpu.device.control;
    let features = &control.supported_features;
    let power = &control.power_limit;
    let offsets = &control.offsets;
    let lock = |lock: &prism_gpu::ClockLockControl| {
        supported(lock.supported, || {
            range(lock.min_allowed_mhz, lock.max_allowed_mhz, "MHz")
        })
    };
    // Cooling is known only from a reading.
    let cooling = match &gpu.latest {
        Latest::Waiting => Err(WAITING.to_owned()),
        Latest::Missing { reason } => Err(reason.clone()),
        Latest::Read(device) => Ok(&device.cooling),
    };
    let fan_control = match &cooling {
        Err(reason) => reason.clone(),
        Ok(cooling) if cooling.is_supported => {
            let count = match cooling.fan_count {
                1 => "1 fan".to_owned(),
                count => format!("{count} fans"),
            };
            let speeds = cooling.fans.first().and_then(|fan| {
                Some(format!(
                    "{}–{}%",
                    fan.min_speed_percent?, fan.max_speed_percent?
                ))
            });
            joined([Some(count), speeds])
        }
        Ok(cooling) => joined([
            Some(NOT_SUPPORTED.into()),
            cooling.unsupported_reason.clone(),
        ]),
    };
    let max_temperature = match &cooling {
        Err(reason) => reason.clone(),
        Ok(cooling) => match cooling.max_operating_temperature_c {
            Some(celsius) => Readout::celsius(Some(celsius)).text(),
            None => cooling
                .max_operating_temperature_reason
                .clone()
                .unwrap_or_else(|| UNKNOWN.into()),
        },
    };
    vec![
        fact(
            "Power limit",
            supported(features.power_limit, || {
                joined([
                    Some(range(power.min_watts, power.max_watts, "W")),
                    power
                        .default_watts
                        .map(|watts| format!("default {watts} W")),
                ])
            }),
        ),
        fact(
            "Graphics clock offset",
            supported(features.graphics_offset, || {
                offset_range(offsets.min_graphics_mhz, offsets.max_graphics_mhz)
            }),
        ),
        fact(
            "Memory clock offset",
            supported(features.memory_offset, || {
                offset_range(offsets.min_memory_mhz, offsets.max_memory_mhz)
            }),
        ),
        fact("Graphics clock lock", lock(&control.gpu_clock_lock)),
        fact("Memory clock lock", lock(&control.memory_clock_lock)),
        fact("Fan control", fan_control),
        fact("Maximum operating temperature", max_temperature),
        fact(
            "Reset to defaults",
            supported(features.reset, || SUPPORTED.into()),
        ),
    ]
}

/// Index, name and identity together, such as "GPU 0 · RTX 5090 · 01:00.0".
pub(crate) fn gpu_label(index: usize, gpu: &Gpu) -> String {
    let index = index_label(index);
    let identity = gpu
        .device
        .bus_id
        .as_deref()
        .map(short_bus_id)
        .unwrap_or_default();
    [index.as_str(), short_gpu_name(&gpu.device.name), identity]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A GPU's position in the inventory, such as "GPU 0".
fn index_label(index: usize) -> String {
    format!("GPU {index}")
}

/// Why any of `metrics` is unknown in the GPU's latest reading. A track names no metric, since it
/// shows one; the sidebar names the ones it lacks.
fn unknown_reason(gpu: &Gpu, metrics: &[Metric]) -> Option<String> {
    let readings = match &gpu.latest {
        Latest::Waiting => return Some(WAITING.into()),
        Latest::Missing { reason } => return Some(reason.clone()),
        Latest::Read(device) => Readings::from(&device.telemetry),
    };
    let unknown: Vec<_> = metrics
        .iter()
        .filter(|metric| readings.get(**metric).is_none())
        .map(|metric| metric.name())
        .collect();
    match unknown.as_slice() {
        [] => None,
        [_] if metrics.len() == 1 => Some(NOT_REPORTED.into()),
        names => Some(format!(
            "{}: not reported by this GPU",
            capitalized(&names.join(", "))
        )),
    }
}

/// `text` with its first letter in upper case, such as "Fan speed".
fn capitalized(text: &str) -> String {
    let mut letters = text.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}

fn latest_readings(gpu: &Gpu) -> Readings {
    latest_device(gpu)
        .map(|device| Readings::from(&device.telemetry))
        .unwrap_or_default()
}

/// The GPU as the latest sample read it, if it did.
fn latest_device(gpu: &Gpu) -> Option<&GpuPerformanceDevice> {
    match &gpu.latest {
        Latest::Read(device) => Some(device),
        Latest::Waiting | Latest::Missing { .. } => None,
    }
}
