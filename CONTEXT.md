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

### Tuning and saved configuration

**GPU tuning profile**:
A named, reusable set of desired GPU tuning values owned by one individual GPU, capturing all supported settings within the profile's tuning scope after edits have been applied. Each application targets that GPU; any omitted settings leave the corresponding current tuning values unchanged.
_Avoid_: Saved GPU configuration when referring to a named profile

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
Applied tuning values or explicit default choices within a profile's scope that differ from the selected GPU tuning profile.
_Avoid_: Unapplied changes, fan changes when cooling is outside the profile's scope

**Saved GPU configuration**:
The retained desired power limit, clock locks, and clock offsets for an individual GPU, available for later reapplication; its existence does not establish that those values are currently in effect.
_Avoid_: Profile, preset, current tuning values

**Live tuning change**:
A change to current GPU controls that does not update the saved GPU configuration.
_Avoid_: Save, persistent configuration change

**Saved configuration reapplication**:
An attempt to apply retained desired values to their corresponding GPUs again, with a separate outcome for each GPU.
_Avoid_: Reset, restore defaults

**GPU configuration reset**:
Restoration of supported power and clock controls to their defaults, with the saved configuration cleared on success; fan mode is outside this reset's scope.
_Avoid_: Factory reset, reset all settings

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
