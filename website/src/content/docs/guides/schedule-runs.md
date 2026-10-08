---
title: Schedule recipe runs
description: Run a recipe list on a schedule with launchd, cron, or Task Scheduler, and check the result.
---

This page shows you how to run a recipe list on a schedule on macOS, Linux,
and Windows, and how to check whether a scheduled run succeeded.

## Before you begin

- [Create overrides](/guides/create-overrides/) for the recipes that you
  plan to run, and confirm that they run by hand.
- Create a folder for the recipe list, report, and log of scheduled runs,
  such as `~/russet-work`.
- Save a recipe list in that folder, such as `recipes.txt`. For the
  format, see [Run recipes](/guides/run-recipes/#run-a-list-of-recipes).

## Schedule runs on macOS

On macOS, use a launchd agent. An agent runs as your user while you're logged
in, so it uses your preferences, recipe repositories, and overrides.

To schedule a daily run, follow these steps:

1. Create the file `~/Library/LaunchAgents/com.example.russet.plist` with the
   following contents:

   ```xml
   <?xml version="1.0" encoding="UTF-8"?>
   <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
   <plist version="1.0">
   <dict>
       <key>Label</key>
       <string>com.example.russet</string>
       <key>ProgramArguments</key>
       <array>
           <string>/usr/local/bin/russet</string>
           <string>run</string>
           <string>--recipe-list</string>
           <string>recipes.txt</string>
           <string>--report-plist</string>
           <string>report.plist</string>
       </array>
       <key>WorkingDirectory</key>
       <string>/Users/USERNAME/russet-work</string>
       <key>StartCalendarInterval</key>
       <dict>
           <key>Hour</key>
           <integer>6</integer>
           <key>Minute</key>
           <integer>0</integer>
       </dict>
       <key>StandardOutPath</key>
       <string>/Users/USERNAME/russet-work/russet.log</string>
       <key>StandardErrorPath</key>
       <string>/Users/USERNAME/russet-work/russet.log</string>
   </dict>
   </plist>
   ```

   Replace `USERNAME` with your user name. To use your organization's naming,
   replace `com.example.russet` in the filename and the `Label` value.

   The `WorkingDirectory` key sets the folder for the relative paths
   `recipes.txt` and `report.plist`.

1. Load the agent:

   ```sh
   launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.example.russet.plist
   ```

1. To test the agent without waiting for the scheduled time, start it now:

   ```sh
   launchctl kickstart gui/$(id -u)/com.example.russet
   ```

   When the run finishes, read `~/russet-work/russet.log`.

The agent runs at 6:00 AM in the computer's local time zone each day. If the
computer is asleep at that time, launchd runs the job when it wakes.

## Schedule runs on Linux

On Linux, use cron. To schedule a daily run, follow these steps:

1. Open your crontab for editing:

   ```sh
   crontab -e
   ```

1. Add the following line:

   ```text
   0 6 * * * cd "$HOME/russet-work" && /usr/local/bin/russet run --recipe-list recipes.txt --report-plist report.plist >> russet.log 2>&1
   ```

   The `cd` command sets the folder for the relative paths in the command.

1. Save the file and close the editor.

## Schedule runs on Windows

On Windows, use Task Scheduler. To schedule a daily run, run the following
commands in PowerShell:

```powershell
$action = New-ScheduledTaskAction -Execute 'C:\Tools\Russet\russet.exe' `
    -Argument 'run --recipe-list recipes.txt --report-plist report.plist' `
    -WorkingDirectory 'C:\russet-work'
$trigger = New-ScheduledTaskTrigger -Daily -At 6am
Register-ScheduledTask -TaskName 'Russet' -Action $action -Trigger $trigger
```

If you installed Russet in a different folder, or you use a different working
folder, change the paths in the first command.

## Check the result of a scheduled run

The exit status of `russet run` shows how the run went:

| Status | Meaning |
| --- | --- |
| `0` | Every recipe ran without an error. |
| `70` | At least one recipe failed while it ran. The other recipes ran. |
| `1` | Russet stopped before it ran any recipe, for example because a recipe wasn't found or failed trust verification. Russet doesn't write a report in this case. |

The report file lists each failed recipe and its error in the `failures`
array, and what the run downloaded, packaged, or imported in the
`summary_results` dictionary. Treat any status other than `0` as a run that
needs attention.

## What's next

- [Run recipes](/guides/run-recipes/)
- [Exit codes](/reference/exit-codes/)
- [Troubleshooting](/resources/troubleshooting/)
