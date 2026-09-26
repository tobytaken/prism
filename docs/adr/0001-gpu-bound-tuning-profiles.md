# GPU-bound tuning profiles

For [issue #3](https://github.com/tobytaken/prism-electron/issues/3), v1 GPU tuning profiles belong to one individual GPU, identified by its stable identity, and each application targets only that GPU. Model names do not establish ownership: profiles remain stored but unavailable to apply while their GPU is absent, and cross-GPU reuse is deferred to avoid introducing compatibility and value-conversion rules in v1.

Saving a profile captures all savable settings within its tuning scope, rather than a user-selected subset. Any omitted settings leave current tuning values unchanged, while included settings request either a specific value or the default at application time; an explicit power default follows the GPU's reported default when applied, while the offset default is zero.

Saving includes every supported setting in scope and omits genuinely unsupported settings. Unknown support or an unavailable value blocks saving rather than inventing a value or silently creating an incomplete profile; saving is also unavailable if no settings are supported. A numeric value equal to the current default remains numeric unless the user explicitly chooses **Use default**, whose meaning is preserved in the profile.

The user must apply profile-setting edits before saving a profile; saving does not apply pending edits. With no pending edits, known current settings can be saved as a new profile without an unnecessary Apply operation. Distinguish **Unapplied changes** from **Changes not saved to profile**: the latter indicates that applied values or explicit default choices differ from the selected profile.

Provide **Apply all** as a separate action in the GPU tuning page, covering every outstanding tuning edit, including fan settings, rather than limiting it to profile settings. If application partly succeeds, refresh actual values, identify successful and failed settings, and preserve failed edits for retry or explicit discard. Unresolved profile-setting edits block saving, and partial application must not be reported as blanket success or silently saved as a mixture of intended and actual values.

Pending or failed fan edits remain visibly unapplied but do not block profile saving when all profile settings have been applied and are known, because cooling is outside v1 profile scope.

V1 includes power limit and graphics and memory clock offsets, matching existing editors and saved configuration support. Clock locks are deferred because they have no visible editor, and cooling controls are deferred until after a full-featured fan curve editor, which is outside this scope; fan controls in profiles remain a future intention. Applying a v1 profile leaves existing clock locks and cooling controls unchanged.

Before application, validate every included setting against the target GPU's current capabilities and ranges. Unsupported settings or unavailable requested defaults block application; an out-of-range numeric value prompts the user to update the profile to the new maximum or minimum, as appropriate, and apply it, or cancel. Range corrections require explicit agreement rather than silently changing the user's desired values; omitted settings do not require validation.

Show all proposed range corrections in one review with **Adjust profile and apply** and **Cancel** actions. Cancellation changes neither the profile nor the GPU; confirmation updates the saved desired values before attempting application. This explicit correction path differs from ordinary profile saving: if application fails, retain the corrected profile and report the failure, because a saved desired value does not establish that the GPU accepted it.
