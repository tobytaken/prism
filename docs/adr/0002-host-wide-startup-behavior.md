# Host-wide startup behavior replaces saved configuration reapplication

For [Define manual profile use and startup behavior](https://github.com/tobytaken/prism/issues/4), what Prism puts into effect when the host starts is a single host-wide **startup behavior** setting: nothing (the default for new installs), each GPU's deliberately designated **startup profile**, or the **last applied tuning values**. The daemon applies it at boot, before any user logs in and without the app running. This replaces the daemon's reapplication of the one saved GPU configuration per GPU, so there is only one boot-time source of tuning values.

Profiles belong to individual users, but startup happens before login, so the startup behavior and each GPU's startup profile are host-wide: anyone who can tune GPUs through Prism can change them, and the latest change wins. A startup profile is a link, not a snapshot: overwriting, correcting, or renaming the profile changes what the next boot applies, and deleting it leaves that GPU with no startup profile. The daemon therefore needs its own copy of the startup profile's latest values; the mechanism belongs to [Define profile persistence, compatibility, and failure contracts](https://github.com/tobytaken/prism/issues/8).

Restoring the last applied tuning values restores, for each setting in the profile tuning scope, the most recent value that was applied successfully by a manual profile application or live tuning change, including changes never saved to a profile and explicit **Use default** requests. [Define automatic switching and restoration rules](https://github.com/tobytaken/prism/issues/6) refines this record to exclude temporary automatic applications and their restorations, so game tuning does not become boot tuning merely because the host restarted during a game. Clock locks and cooling are not restored. Those manual values were never chosen deliberately for startup, so an unstable value could crash the host on every boot. To prevent that, Settings warns about the risk, and Prism skips the restore once after an unclean shutdown that followed a restore. Startup profiles were chosen deliberately, so they skip this guard.

Boot-time application follows ADR-0001's validation rules, but nobody is present to review range corrections. If the GPU is absent, a setting is unsupported, or a value is out of range, nothing is applied to that GPU; out-of-range values are never clamped. A partial application failure is reported per setting. Each GPU's **startup outcome** is shown the next time Prism opens.

Existing saved GPU configurations are migrated once. Each one's power limit and offsets become a profile named "Previous startup settings", designated as that GPU's startup profile, with startup behavior set to startup profiles. The user is told once that saved clock locks are no longer restored. The saved configuration, its reapplication, and its reset are then retired.

_Amended 2026-10-10:_ [#121](https://github.com/tobytaken/prism/issues/121) removed this one-time migration. Prism was never released, so there are no saved configurations to carry over.

## Considered options

- **Keep saved configuration reapplication alongside profiles**: rejected because two boot-time sources could conflict on the same GPU, and nothing in the current UI writes saved configurations.
- **Apply a startup profile only when the app launches**: rejected because tuning should hold without opening Prism.
- **Offer only startup profiles, or only restoring last applied values**: both are offered, because users differ between wanting a deliberate baseline and wanting "what I had".
