# Tervy 0.1.10

Tervy can now remind you to record a Microsoft Teams meeting. On macOS 26 or
later, sustained audio activity from desktop Teams brings Tervy forward with a
**Teams meeting detected** prompt, even when Tervy is hidden.

Choose **Record…** to open the recording and notes options, or **Dismiss**
(Escape) to skip that call. Reminders are enabled by default and can be turned
off in **Settings → Teams meeting reminders**. Your preference is remembered.
The detector does not capture audio, and nothing is recorded until you choose
a recording option. Existing recordings suppress reminders.

## Help test Teams detection

This first version uses Teams microphone or speaker activity rather than a
meeting API. Expect a reminder roughly 6–9 seconds after sustained activity
begins. Muted meetings can be detected through speaker activity. Prejoin audio
checks and other sustained Teams audio may also trigger a reminder. Teams in a
browser is not detected. Use **Stop** to finish recording.

Please try joining, muting, leaving, and rejoining a desktop Teams call, including
with Tervy hidden. Report missed or repeated prompts through
[GitHub Issues](https://github.com/jringeisen/tervy/issues), including your macOS
and Teams versions. Avoid attaching private meeting audio or transcripts.

## Install or update

Open **Settings** in Tervy and check for updates, then choose **Install update**.
Tervy asks before stopping running sessions.

For a new installation, download the Apple Silicon DMG from this release, open it,
and drag Tervy to Applications. Tervy supports Apple Silicon Macs running macOS
15 or later. Experimental meeting recording and Teams reminders require macOS
26 or later.
