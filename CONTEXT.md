# Prism

Prism is a Linux GPU control center for inspecting hardware, monitoring performance, tuning GPU behavior, and managing NVIDIA drivers.

## Language

### Hardware and observation

**Host**:
The Linux system whose GPUs and driver installation Prism manages, including when Prism runs inside a sandbox.
_Avoid_: Container when referring to the managed system

**GPU**:
An individual graphics device on the host, with its own identity, capabilities, readings, and tuning values; multiple GPUs can share the same model name.
_Avoid_: GPU model when referring to an individual device

**GPU inventory**:
A snapshot of GPUs Prism can inspect, including their hardware details and available controls; an empty inventory alone does not establish that the host has no GPU.
_Avoid_: Telemetry, hardware scan when referring to this snapshot

**GPU telemetry**:
Measurements of a GPU's operating state over time, such as utilization, temperature, power draw, clock speeds, and memory usage; an unavailable reading is unknown, rather than zero.
_Avoid_: GPU configuration, benchmark results

**GPU control capability**:
Support for a particular tuning control on a GPU and its driver, including any allowed value range; support alone does not establish permission or current availability to change it.
_Avoid_: Supported GPU when only one control has been checked

### Tuning, profiles, and startup

**GPU tuning profile**:
A named, reusable set of desired GPU tuning values owned by one individual GPU, with a name unique among that GPU's profiles regardless of letter case, capturing all supported settings within the profile's tuning scope after edits have been applied. Each application targets that GPU; any omitted settings leave the corresponding current tuning values unchanged.
_Avoid_: Saved GPU configuration, preset

**Profile application**:
An attempt to put a GPU tuning profile's settings into effect on its owning GPU; choosing a profile requests its application, and a saved profile alone does not establish that its values are in effect.
_Avoid_: Loading a profile, profile selection when the profile has not been applied

**Current profile**:
The GPU tuning profile used as the reference for a GPU's applied or saved tuning state, including an automatically applied game profile; an individual GPU has at most one, and its saved values can differ from the current tuning values.
_Avoid_: Selected profile, loaded profile, active profile

**Startup behavior**:
The host-wide choice of what Prism puts into effect on GPUs when the host starts: nothing, each GPU's startup profile, or the last applied tuning values; nothing by default.
_Avoid_: Saved configuration reapplication, autostart

**Startup profile**:
The GPU tuning profile deliberately designated, host-wide, for an individual GPU to be applied when the host starts; a GPU has at most one, none by default, and its latest saved values are the ones applied.
_Avoid_: Default profile, current profile

**Last applied tuning values**:
The most recent successfully applied manual value of each setting within the profile tuning scope for an individual GPU, whether it came from a profile or a live tuning change; temporary automatic applications and their restorations are excluded.
_Avoid_: Last applied profile, current profile

**Startup outcome**:
The per-GPU result of Prism's startup behavior on the most recent host start, including settings applied, failed, or not attempted and why, such as a skip after an unclean shutdown.
_Avoid_: Profile application result when no user requested it

**Game**:
Something a user plays that Prism can recognize while it runs: a Steam app, identified by its app ID whether it runs natively or through Proton, or a manually identified program, identified by its executable's location.
_Avoid_: Process, executable when referring to a Steam game

**Game launch**:
The period between a user starting a game and its own processes existing, including launcher-side preparation such as shader compilation or compatibility setup.
_Avoid_: Running game

**Launcher**:
A program used to browse, install, and start games, such as the Steam client; a running launcher alone is not a running game.
_Avoid_: Game when referring to the store or client application

**Running game**:
A game whose own processes currently exist on the host, regardless of window focus.
_Avoid_: Game launch, active game when referring to the focused window

**Game association**:
A user's link from a game to at most one of their GPU tuning profiles per GPU, used for automatic switching only while that user's game is running; GPUs without a linked profile are unaffected, and many games can share one profile.
_Avoid_: Game profile, per-game profile

**Automatic switching**:
An opt-in, per-user behavior that applies game-associated GPU tuning profiles while games run, with the most recently started associated game taking precedence on each GPU.
_Avoid_: Auto profile, game mode

**Automatic switching status**:
The per-GPU explanation of which game and profile drive automatic switching, or why switching is waiting, paused, overridden, blocked, or failed; it does not establish which tuning values are actually in effect.
_Avoid_: Active profile, profile application result

**Pre-game tuning values**:
The actual tuning values within a GPU's profile tuning scope held as its restoration baseline before automatic changes, including changes not saved to a profile.
_Avoid_: Startup settings, previous profile

**Edit hold**:
The deferral of automatic changes to a GPU while an open Prism session has unapplied tuning changes within its profile tuning scope; it ends when those edits are applied or discarded, or when that session closes.
_Avoid_: Automatic switching pause, lock

**Manual override**:
A user's attempted profile application or live change within the profile tuning scope that pauses automatic switching for the affected GPU until all its associated running games end or the user explicitly resumes switching, preserving the resulting values afterward.
_Avoid_: Profile override, automatic switching off

**Automatic switching pause**:
An explicit, persistent hold on automatic changes to one GPU until the user resumes switching, retaining its current tuning values; it is distinct from disabling a user's automatic switching and from a temporary manual override.
_Avoid_: Automatic switching off, manual override

**Failure pause**:
A suspension of automatic writes to one GPU following an unsuccessful automatic application or recovery, retaining recovery state until the user requests a retry.
_Avoid_: Manual override, automatic switching off

**Profile setting**:
A desired tuning instruction in a GPU tuning profile, expressed as a specific value or an explicit request for the default at application time; omission means leave the current tuning value unchanged.
_Avoid_: Current tuning value, captured default when referring to an explicit default request

**GPU tuning**:
Adjustment of a GPU's power, clock, or cooling controls.
_Avoid_: Overclocking as an umbrella term for all tuning

**Current tuning values**:
The control values currently reported for a GPU, such as its power limit or clock offsets; these can differ from saved values and from telemetry measurements.
_Avoid_: Saved configuration, desired values

**Unapplied tuning changes**:
Edits to desired GPU tuning settings that have not yet been successfully applied to the GPU.
_Avoid_: Unsaved profile changes when the edits have not reached the GPU

**Changes not saved to profile**:
Applied tuning values or explicit default choices within a profile's scope that differ from the current profile.
_Avoid_: Unapplied changes, fan changes when cooling is outside the profile's scope

**Live tuning change**:
A change to current GPU controls made outside a profile application, which updates no GPU tuning profile.
_Avoid_: Save, persistent configuration change

**Power draw**:
The GPU's measured power consumption in watts at a given time.
_Avoid_: Power limit, power usage percentage

**Power limit**:
A configured ceiling on GPU power consumption in watts, with a default value and an allowed range.
_Avoid_: Power draw, target consumption

**Enforced power limit**:
The effective power ceiling reported for the GPU, which can differ from the configured power limit.
_Avoid_: Configured power limit, power draw

**Clock lock**:
A requested minimum and maximum frequency range for a GPU's graphics or memory clock; equal bounds express a single requested frequency.
_Avoid_: Clock offset, measured clock speed

**Clock offset**:
A signed frequency adjustment for the graphics or memory clock, expressed in MHz; zero represents no offset.
_Avoid_: Clock lock, absolute clock speed

**Fan control mode**:
The choice between automatic cooling control and a manual speed target for an individual GPU fan.
_Avoid_: Fan curve when referring to a fixed manual target

### Driver management

**NVIDIA driver**:
The host software that enables use and management of NVIDIA GPUs; installed driver packages and the currently loaded driver are distinct aspects of its state.
_Avoid_: Firmware, GPU configuration

**Driver action**:
One of Prism's driver operations: installing the recommended NVIDIA driver or updating installed NVIDIA driver packages on the host.
_Avoid_: GPU tuning action, system upgrade

**Driver plan**:
A proposed driver action evaluated for the host, including its recommendation, blockers, warnings, and any manual completion steps.
_Avoid_: Driver job, installation result

**Recommended driver plan**:
The driver plan Prism prefers for the detected driver state; a recommendation can still have blockers that prevent execution.
_Avoid_: Runnable plan, latest driver version

**Driver plan blocker**:
A condition that prevents Prism from starting a driver plan.
_Avoid_: Warning, manual completion step

**Driver plan warning**:
Information relevant to assessing or completing a driver plan that does not by itself prevent the action from starting.
_Avoid_: Blocker

**Driver job**:
A particular execution of a driver action, with its own progress, log, and outcome; finishing package operations can leave a reboot or manual step outstanding.
_Avoid_: Driver plan, installed driver

**Active driver job**:
A driver job that is queued or running; a job awaiting a reboot or manual completion is no longer active.
_Avoid_: Most recent job, unfinished driver setup

**Reboot required**:
A driver job outcome indicating that package operations finished and the host needs to restart to load the driver.
_Avoid_: Driver loaded, verified installation

**Manual step required**:
A driver job outcome indicating that package operations finished with user completion work outstanding, such as firmware key enrollment, in addition to a reboot.
_Avoid_: Failed job, fully completed installation

### Appearance

**Accent preference**:
The user's choice of an AMD, NVIDIA, or Intel color theme, or automatic selection from detected GPU vendors; choosing a vendor theme does not imply hardware support for that vendor.
_Avoid_: GPU selection, tuning profile

**Effective accent**:
The color theme currently resolved from the accent preference and available GPU inventory; automatic selection can remain unresolved and use Prism's default appearance.
_Avoid_: Accent preference, detected GPU vendor
