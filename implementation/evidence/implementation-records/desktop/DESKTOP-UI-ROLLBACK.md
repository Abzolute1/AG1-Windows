# Desktop UI: review and rollback

> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status](../../../README.md). Original bytes and old paths are preserved in the [relocation manifest](../manifest.json) and recovery archive.

The current interface is installed at `/opt/ag1/releases/16e04b2ec9e945bcb3706672df73f4b1`. Close and relaunch AG1 to load it. Settings provides appearance, typography, layout, workflow, models, permissions and GitHub. AG1 Graphite follows the supplied background palette; Tokyo adapts the published [Tokyo Theme palette](https://plugins.jetbrains.com/plugin/29646-tokyo-theme).

To restore the immediately preceding installed application, first close AG1, then run:

```sh
sudo ln -sfn releases/f416da9cbb77441c9e5ab6e69c80462e /opt/ag1/current
```

To return to the application from before the whole UI redesign:

```sh
sudo ln -sfn releases/4428d311dd3a40a29986e7794c422b96 /opt/ag1/current
```

All retained release directories remain available. Switching the application release leaves chats, project data, settings and model tuning in their current locations. Older releases may ignore newer preferences.

The exact source before the redesign is saved in `.local/p14-ui-redesign-baseline-1/source.tar.gz`, with original hashes, Git diff and installed-release identity. This includes earlier uncommitted work. The source restore covers 156 changed/new/removed files and does not reset the working tree to Git HEAD.

Review without modifying source:

```sh
python tools/revert_desktop_redesign.py
```

Restore after checking every current file against the recorded version:

```sh
python tools/revert_desktop_redesign.py --apply
```

The restore refuses if later edits exist. It leaves user data and installed releases alone. After restoring, run `npm ci` in `product/apps/desktop` before rebuilding to match the restored lockfile.

Validation receipt: [desktop-ui-polish.json](../../P14/desktop-ui-polish.json). Current source identity: `1e890f943ffc99332071aa7b501abfc649bead7fc09b7cc5d417352334254460`. Screenshots: [appearance](../../P14/desktop-ui-polish/appearance.png), [typography](../../P14/desktop-ui-polish/typography.png), [compact tools](../../P14/desktop-ui-polish/compact-tools.png), [earliest messages](../../P14/desktop-ui-polish/first-message.png), [GitHub](../../P14/desktop-ui-polish/github.png). Logs and private visual drivers: `.local/p14-ui-polish/`.

GitHub requires the official GitHub CLI, installed here as version 2.97.0. Choose Settings → GitHub → Connect to link your account. Testing used a fake CLI and made no real external publication. Chat opening no longer hashes all model weights; a cold request still verifies and loads its selected model. An owned local Stop may unload the model server. Neither duration workload was started.

Startup fix receipt: [desktop-startup-fix.json](../../P14/desktop-startup-fix.json).

Current settings delivery receipt: [desktop-settings-delivery.json](../../P14/desktop-settings-delivery.json). New screenshots include [Appearance](../../P14/desktop-settings-delivery/appearance.png), [Midnight](../../P14/desktop-settings-delivery/midnight.png), [Plugins](../../P14/desktop-settings-delivery/plugins.png) and [reasoning controls](../../P14/desktop-settings-delivery/reasoning-slider.png). Older builds may ignore Trash metadata and show those retained conversations again; software rollback does not erase chat data.

Current correction receipt: [desktop-model-admission-fix.json](../../P14/desktop-model-admission-fix.json). This corrects catalog-wide model verification, theme hit testing and adds explicit bypass confirmation. Returning to the preceding build also returns those bugs.

## Prompt, image input and rendering correction

Current receipt: [desktop-prompt-images.json](../../P14/desktop-prompt-images.json). The immediately preceding f416 release includes prompt and image changes but lacks the latest image progress and display pacing correction. The release before this entire follow-up is `1c8c5f165dad4ac7946d1087075711fb`.

The next-launch native host configuration now points to a separate sealed capture with the existing verified 35B vision projector. To restore the prior configuration, close AG1 and run from this repository:

```sh
python .local/p14-prompt-refinement/restore-model-config.py
```

This checks the current hash and refuses to overwrite later edits. It restores the backed-up host pointer/model input metadata; it does not delete either capture or alter the model profiles. Application rollback and configuration rollback are separate operations. Real vision inference with the restored projector has not yet been live-tested.


## Readable activity and independent conversations

Current receipt: [desktop-activity-conversations.json](../../P14/desktop-activity-conversations.json). Installed release `cc3a4e969a1847218089fe1936d4be63`; the immediately preceding application remains at `/opt/ag1/releases/16e04b2ec9e945bcb3706672df73f4b1`. Close AG1 before restoring the application pointer using the procedure above.

Source rollback now checks 173 files, including generated protocol fixtures. The durable per-conversation selection namespace is additive; no database schema migration or journal deletion occurred. Older native builds retain their old navigation restrictions and ignore the separate selection records. New runtime configuration from the prior vision pass is unchanged by this update; its rollback remains separate.


## Native edit reliability correction

Current application: `/opt/ag1/releases/2ebdc491fb814b959b79378f24c62e01`. [Investigation](../../P14/desktop-edit-reliability.md) and [receipt](../../P14/desktop-edit-reliability.json). Source rollback now checks 187 files, preserving the original pre-redesign source and earlier uncommitted work.

To restore the immediately preceding application, close AG1 and run:

```sh
sudo ln -sfn releases/cc3a4e969a1847218089fe1936d4be63 /opt/ag1/current
```

This restores the earlier edit-policy behavior, failure reporting and working-state insertion. The new context suffix is transient and is not journaled; this correction introduces no database migration. Model profiles and runtime configuration were not changed. Chats and project files remain in place when the application pointer is restored.
